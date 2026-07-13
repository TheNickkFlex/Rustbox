//! Notification daemon (dunst reimplementation) integrated into the WM.
//!
//! Implements the freedesktop.org Desktop Notifications spec
//! (`org.freedesktop.Notifications`) by owning that well-known name on the
//! session bus. The D-Bus connection runs on the WM's main thread: each event
//! loop iteration drains pending messages via a non-blocking `incoming(0)`
//! call, so no extra thread or wake pipe is required.
//!
//! Rendering is intentionally lightweight: popups are override-redirect X11
//! windows drawn with the WM's bitmap font and X core primitives (no cairo).

pub mod render;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::OnceLock;
use std::time::Instant;

use dbus::arg::Variant;
use dbus::ffidisp::Connection as DbusConn;
use dbus::message::Message;
use dbus::strings::ErrorName;
use dbus::MessageType;
use regex::Regex;

use crate::render::image::ImageControl;

/// Commands delivered to the notification daemon through the in-process Rust
/// channel, which is an alternative to the D-Bus `org.freedesktop.Notifications`
/// service. This lets the WM (and other Rust code) raise complex notifications —
/// with actions, icons, progress bars, markup, stack tags — without requiring a
/// running session bus.
#[derive(Debug)]
pub enum NotifyCommand {
    /// Show a notification built in Rust (no D-Bus involved).
    Notify(RawNotification),
    /// Close a notification by its id.
    Close(u32),
    /// Close every currently displayed notification.
    CloseAll,
}

/// Global sender side of the in-process notification channel. Initialized once
/// when the daemon is created; any Rust code can grab it via
/// [`NotifyDaemon::channel`] to raise notifications without D-Bus.
static NOTIFY_SENDER: OnceLock<Sender<NotifyCommand>> = OnceLock::new();
use crate::x11::X11Connection;

/// Where on the screen notifications stack from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

impl Urgency {
    fn from_u32(v: u32) -> Urgency {
        match v {
            0 => Urgency::Low,
            2 => Urgency::Critical,
            _ => Urgency::Normal,
        }
    }
}

/// A notification as received over D-Bus, already assigned a stable id.
#[derive(Debug, Clone)]
pub struct RawNotification {
    pub id: u32,
    pub app_name: String,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    /// (action_key, label) pairs from the `actions` array.
    pub actions: Vec<(String, String)>,
    pub urgency: Urgency,
    /// `true` => do not keep in history when closed.
    pub transient: bool,
    /// -1 => use configured default; 0 => sticky; >0 => milliseconds.
    pub expire_timeout: i32,
    pub created: Instant,
    /// Raw icon bytes from the `image-data` hint (takes precedence over the
    /// file path in `app_icon` when present).
    pub icon_data: Option<Vec<u8>>,
    /// `x-canonical-private-synchronous` value, used to replace an existing
    /// notification with the same key (like `replaces_id`).
    pub sync_key: Option<String>,
    /// `x-dunst-stack-tag` hint: groups/merges notifications with the same tag.
    pub stack_tag: Option<String>,
    /// `category` hint (used by rules).
    pub category: Option<String>,
    /// `desktop-entry` hint (used by rules / theme icon lookup).
    pub desktop_entry: Option<String>,
    /// `value` hint (0..=100) rendered as a progress bar; `None` => no bar.
    pub progress: Option<u8>,
    /// If true, drop the notification from history when closed.
    pub history_ignore: bool,
}

/// Reason codes reported in the `NotificationClosed` signal (spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    Expired = 1,
    User = 2,
    ClosedByCaller = 3,
    Undefined = 4,
}

/// Everything we care about from the `hints` dict of a `Notify` call.
#[derive(Debug, Clone, Default)]
struct ParsedHints {
    urgency: Urgency,
    transient: bool,
    /// `image-path` hint (a file path or theme icon name).
    image_path: Option<String>,
    /// `image-data` hint, decoded to RGBA8 pixels.
    image_data: Option<Vec<u8>>,
    /// `desktop-entry` hint (icon name to look up in the theme).
    desktop_entry: Option<String>,
    /// `x-canonical-private-synchronous` value.
    sync_key: Option<String>,
    /// `x-dunst-stack-tag` value (group/merge by tag).
    stack_tag: Option<String>,
    /// `category` hint.
    category: Option<String>,
    /// `value` hint (0..=100), rendered as a progress bar.
    progress: Option<u8>,
}

/// A matching rule (dunst "rules" section), applied in declaration order.
/// Later matching rules override earlier ones. A `None` constraint is a
/// wildcard.
#[derive(Debug, Clone, Default)]
pub struct Rule {
    pub appname_re: Option<Regex>,
    pub summary_re: Option<Regex>,
    pub body_re: Option<Regex>,
    pub urgency: Option<Urgency>,
    pub expire_timeout: Option<i32>,
    pub set_urgency: Option<Urgency>,
    pub new_icon: Option<String>,
    pub skip_display: bool,
    /// `category` filter regex.
    pub category_re: Option<Regex>,
    /// `stack_tag` filter regex.
    pub stack_tag_re: Option<Regex>,
    /// `desktop_entry` filter regex.
    pub desktop_entry_re: Option<Regex>,
    /// Override the notification's `transient` flag.
    pub set_transient: Option<bool>,
    /// Drop the notification from history when closed.
    pub history_ignore: bool,
}

/// Resolved color theme for a popup. Colors are RGB tuples; the popup
/// allocates the actual X pixel at draw time.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub bg: (u8, u8, u8),
    pub fg: (u8, u8, u8),
    pub frame: (u8, u8, u8),
    pub body: (u8, u8, u8),
    /// Urgency bar color for [low, normal, critical].
    pub urgency: [(u8, u8, u8); 3],
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            bg: (0, 0, 0),
            fg: (235, 235, 235),
            frame: (60, 60, 60),
            body: (200, 200, 200),
            urgency: [(51, 102, 255), (51, 170, 51), (255, 51, 51)],
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub max_visible: usize,
    pub width: u16,
    pub margin: i16,
    pub gap: i16,
    pub default_timeout_ms: i32,
    pub origin: Corner,
    pub monitor: usize,
    pub font_scale: u16,
    /// Background / foreground / frame RGB colors.
    pub bg: (u8, u8, u8),
    pub fg: (u8, u8, u8),
    pub frame: (u8, u8, u8),
    pub body: (u8, u8, u8),
    /// Urgency bar color per [low, normal, critical].
    pub urgency_colors: [(u8, u8, u8); 3],
    /// Matching rules, applied in order.
    pub rules: Vec<Rule>,
}

impl Default for Config {
    fn default() -> Self {
        let t = Theme::default();
        Self {
            max_visible: 5,
            width: 380,
            margin: 12,
            gap: 8,
            default_timeout_ms: 5000,
            origin: Corner::TopRight,
            monitor: 0,
            font_scale: 2,
            bg: t.bg,
            fg: t.fg,
            frame: t.frame,
            body: t.body,
            urgency_colors: t.urgency,
            rules: Vec::new(),
        }
    }
}

impl Config {
    /// Build the draw-time `Theme` from the configured colors.
    pub fn theme(&self) -> Theme {
        Theme {
            bg: self.bg,
            fg: self.fg,
            frame: self.frame,
            body: self.body,
            urgency: self.urgency_colors,
        }
    }
}

/// The notification daemon. Owns the D-Bus session name (when available) and
/// the popup windows. It also exposes an in-process Rust channel (`NotifyCommand`)
/// so notifications can be raised without D-Bus.
pub struct NotifyDaemon {
    /// D-Bus session connection. `None` when no session bus is available — the
    /// daemon still works via the in-process channel in that case.
    dbus: Option<DbusConn>,
    /// Receiver end of the in-process notification channel.
    recv: Receiver<NotifyCommand>,
    next_id: u32,
    config: Config,
    paused: bool,
    /// Icon cache (file paths + freedesktop theme), shared across popups.
    icon_cache: ImageControl,
    /// Notifications currently shown (each backed by a popup window).
    displayed: Vec<render::Popup>,
    /// Notifications waiting for a free slot.
    waiting: Vec<RawNotification>,
    /// Recently closed notifications (for history recall).
    history: Vec<RawNotification>,
    /// `x-canonical-private-synchronous` key -> currently shown id.
    sync_map: HashMap<String, u32>,
    screen_width: u16,
    screen_height: u16,
}

impl NotifyDaemon {
    /// Create the notification daemon. The D-Bus `org.freedesktop.Notifications`
    /// name is claimed when a session bus is available; otherwise the daemon
    /// runs D-Bus-free and notifications are delivered only through the
    /// in-process Rust channel. Always returns a usable daemon.
    pub fn new(conn: &X11Connection) -> Self {
        let dbus = match DbusConn::new_session() {
            Ok(c) => {
                match c.register_name("org.freedesktop.Notifications", 1 | 2) {
                    Ok(reply) => {
                        log::info!("Claimed org.freedesktop.Notifications (reply={:?})", reply)
                    }
                    Err(e) => log::warn!("Could not claim org.freedesktop.Notifications: {}", e),
                }
                // Register the object paths so libdbus routes method calls to
                // our message queue instead of auto-replying with "UnknownMethod".
                if let Err(e) = c.register_object_path("/org/freedesktop/Notifications") {
                    log::warn!("Failed to register notification object path: {}", e);
                }
                if let Err(e) = c.register_object_path("/org/dunstproject/cmd0") {
                    log::warn!("Failed to register dunst control object path: {}", e);
                }
                Some(c)
            }
            Err(e) => {
                log::warn!(
                    "D-Bus session unavailable; notifications work via in-process channel only: {}",
                    e
                );
                None
            }
        };

        // In-process Rust channel: the Sender side is published globally so any
        // Rust code can raise notifications without D-Bus.
        let (tx, rx) = channel::<NotifyCommand>();
        let _ = NOTIFY_SENDER.set(tx);

        let screen = conn.screen();
        let config = Self::load_config(&Self::config_path());
        let mut d = Self {
            dbus,
            recv: rx,
            next_id: 1,
            config,
            paused: false,
            icon_cache: ImageControl::new(64),
            displayed: Vec::new(),
            waiting: Vec::new(),
            history: Vec::new(),
            sync_map: HashMap::new(),
            screen_width: screen.width_in_pixels,
            screen_height: screen.height_in_pixels,
        };
        d.relayout(conn);
        d
    }

    /// Global sender for the in-process notification channel. Panics if the
    /// daemon has not been created yet (call [`NotifyDaemon::new`] first).
    pub fn channel() -> &'static Sender<NotifyCommand> {
        NOTIFY_SENDER
            .get()
            .expect("notify channel not initialized; create NotifyDaemon first")
    }

    pub fn set_screen_size(&mut self, w: u16, h: u16) {
        self.screen_width = w;
        self.screen_height = h;
    }

    /// Drain and dispatch any pending D-Bus messages. Non-blocking: with a
    /// zero timeout libdbus only processes already-buffered messages. No-op
    /// when the daemon runs without a D-Bus session.
    pub fn process_dbus(&mut self, conn: &X11Connection) {
        let db = match self.dbus.as_ref() {
            Some(db) => db,
            None => return,
        };
        let mut msgs = Vec::new();
        loop {
            match db.incoming(0).next() {
                Some(msg) if msg.msg_type() == MessageType::MethodCall => msgs.push(msg),
                Some(_) => {}
                None => break,
            }
        }
        for msg in msgs {
            self.handle_method_call(msg, conn);
        }
    }

    /// Drain and dispatch notifications delivered through the in-process Rust
    /// channel (the D-Bus-free alternative). Runs once per main-loop iteration.
    pub fn process_internal(&mut self, conn: &X11Connection) {
        while let Ok(cmd) = self.recv.try_recv() {
            match cmd {
                NotifyCommand::Notify(mut notif) => {
                    // Assign a stable id (the D-Bus path does this in
                    // handle_method_call; the channel path must do it here).
                    if notif.id == 0 {
                        notif.id = self.next_id;
                        self.next_id = self.next_id.wrapping_add(1).max(1);
                    }
                    self.enqueue(notif, conn)
                }
                NotifyCommand::Close(id) => self.close(id, CloseReason::ClosedByCaller, conn),
                NotifyCommand::CloseAll => {
                    let ids: Vec<u32> = self.displayed.iter().map(|p| p.notif.id).collect();
                    for id in ids {
                        self.close(id, CloseReason::ClosedByCaller, conn);
                    }
                }
            }
        }
    }

    /// Send a D-Bus message only when a session bus is present.
    fn dbus_send(&self, msg: Message) {
        if let Some(db) = &self.dbus {
            let _ = db.send(msg);
        }
    }

    fn handle_method_call(&mut self, msg: Message, conn: &X11Connection) {
        let iface = msg.interface().map(|s| s.to_string());
        let member = msg.member().map(|s| s.to_string());
        let (iface, member) = match (iface, member) {
            (Some(i), Some(m)) => (i, m),
            _ => return,
        };

        match (iface.as_str(), member.as_str()) {
            ("org.freedesktop.Notifications", "GetCapabilities") => {
                let caps = vec![
                    "body",
                    "actions",
                    "icon-static",
                    "persistence",
                    "image/png",
                    "urgency",
                    "x-canonical-private-synchronous",
                    "x-dunst-stack-tag",
                ];
                let _ = self.dbus_send(msg.method_return().append1(caps));
            }
            ("org.freedesktop.Notifications", "GetServerInformation") => {
                let _ = self.dbus_send(
                    msg.method_return()
                        .append3("rustbox", "rustbox", "1.4.0")
                        .append1("1.2"),
                );
            }
            ("org.dunstproject.cmd0", m) => {
                self.handle_control(m, msg, conn);
            }
            ("org.freedesktop.Notifications", "Notify") => {
                let mut iter = msg.iter_init();
                let app_name: String = iter.read().unwrap_or_default();
                let replaces_id: u32 = iter.read().unwrap_or_default();
                let app_icon: String = iter.read().unwrap_or_default();
                let summary: String = iter.read().unwrap_or_default();
                let body: String = iter.read().unwrap_or_default();
                let actions: Vec<String> = iter.read().unwrap_or_default();
                let hints: HashMap<String, Variant<Box<dyn dbus::arg::RefArg>>> = iter.read().unwrap_or_default();
                let expire: i32 = iter.read().unwrap_or_default();

                let h = parse_hints(&hints);
                let urgency = h.urgency;
                let transient = h.transient;

                // Resolve the replacement id: explicit `replaces_id`, else a
                // matching `x-canonical-private-synchronous` key or a matching
                // `x-dunst-stack-tag` (both group/merge by key).
                let replace_key = h.sync_key.as_ref().or(h.stack_tag.as_ref());
                let replaces_id = if replaces_id != 0 {
                    replaces_id
                } else if let Some(key) = replace_key {
                    self.sync_map.get(key).copied().unwrap_or(0)
                } else {
                    0
                };

                let id = if replaces_id != 0 {
                    replaces_id
                } else {
                    let id = self.next_id;
                    self.next_id = self.next_id.wrapping_add(1).max(1);
                    id
                };

                // Resolve the icon: explicit path > image-path hint > theme
                // lookup via desktop-entry > theme lookup via app name.
                let app_icon = if !app_icon.is_empty() {
                    app_icon.clone()
                } else if let Some(p) = &h.image_path {
                    p.clone()
                } else if let Some(de) = &h.desktop_entry {
                    resolve_theme_icon(de).unwrap_or_default()
                } else if !app_name.is_empty() {
                    resolve_theme_icon(&app_name).unwrap_or_default()
                } else {
                    String::new()
                };
                let icon_data = h.image_data.clone();

                // Track the grouping key (sync_key OR stack_tag) so a later
                // notification with the same key replaces this one.
                if let Some(key) = replace_key {
                    self.sync_map.insert(key.clone(), id);
                }

                let notif = RawNotification {
                    id,
                    app_name: app_name.clone(),
                    app_icon,
                    summary: summary.clone(),
                    body: body.clone(),
                    actions: pair_actions(&actions),
                    urgency,
                    transient,
                    expire_timeout: expire,
                    created: Instant::now(),
                    icon_data,
                    sync_key: h.sync_key.clone(),
                    stack_tag: h.stack_tag.clone(),
                    category: h.category.clone(),
                    desktop_entry: h.desktop_entry.clone(),
                    progress: h.progress,
                    history_ignore: false,
                };
                log::info!(
                    "Notification #{} app={} summary={} body={} actions={} urgency={:?} transient={}",
                    id,
                    app_name,
                    summary,
                    body,
                    notif.actions.len(),
                    urgency,
                    transient,
                );

                if let Some(pos) = self.displayed.iter().position(|p| p.notif.id == id) {
                    let old = self.displayed.remove(pos);
                    old.destroy(conn);
                }
                self.enqueue(notif, conn);

                let _ = self.dbus_send(msg.method_return().append1(id));
            }
            ("org.freedesktop.Notifications", "CloseNotification") => {
                let id: u32 = msg.read1().unwrap_or(0);
                self.close(id, CloseReason::ClosedByCaller, conn);
                let _ = self.dbus_send(msg.method_return());
            }
            _ => {
                let name = ErrorName::new("org.freedesktop.DBus.Error.UnknownMethod").unwrap();
                let _ = self.dbus_send(msg.error(&name, c"no such method"));
            }
        }
    }

    /// Add a notification to the displayed set or the waiting queue.
    fn enqueue(&mut self, mut notif: RawNotification, conn: &X11Connection) {
        let skip = Self::apply_rules(&mut notif, &self.config.rules);
        if skip {
            if let Ok(sig) = Message::new_signal(
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
                "NotificationClosed",
            ) {
                let _ = self.dbus_send(sig.append2(notif.id, CloseReason::Undefined as u32));
            }
            if !notif.history_ignore {
                notif.icon_data = None; // Strip heavy raw image bytes before saving to history
                self.history.push(notif);
                if self.history.len() > 50 {
                    self.history.remove(0);
                }
            }
            return;
        }
        if self.displayed.len() < self.config.max_visible {
            match render::Popup::new(
                conn,
                notif,
                self.screen_width,
                self.screen_height,
                self.displayed.len(),
                self.config.max_visible,
                self.config.origin,
                self.config.margin,
                self.config.gap,
                self.config.width,
                self.config.font_scale,
                &mut self.icon_cache,
                self.config.theme(),
            ) {
                Ok(p) => self.displayed.push(p),
                Err(e) => log::warn!("Failed to create notification popup: {}", e),
            }
        } else {
            // Bound the waiting queue so a flood of notifications cannot grow
            // RAM/VRAM without limit. Drop the oldest pending entry.
            const WAITING_MAX: usize = 100;
            if self.waiting.len() >= WAITING_MAX {
                self.waiting.remove(0);
            }
            self.waiting.push(notif);
        }
        self.relayout(conn);
    }

    /// Recompute every popup's position and refill from the waiting queue.
    pub fn relayout(&mut self, conn: &X11Connection) {
        while self.displayed.len() < self.config.max_visible {
            let next = match self.waiting.first() {
                Some(n) => n.clone(),
                None => break,
            };
            self.waiting.remove(0);
            match render::Popup::new(
                conn,
                next,
                self.screen_width,
                self.screen_height,
                self.displayed.len(),
                self.config.max_visible,
                self.config.origin,
                self.config.margin,
                self.config.gap,
                self.config.width,
                self.config.font_scale,
                &mut self.icon_cache,
                self.config.theme(),
            ) {
                Ok(p) => self.displayed.push(p),
                Err(e) => log::warn!("Failed to create notification popup: {}", e),
            }
        }

        for (i, p) in self.displayed.iter_mut().enumerate() {
            p.reposition(
                conn,
                self.screen_width,
                self.screen_height,
                i,
                self.config.origin,
                self.config.margin,
                self.config.gap,
            );
        }
    }

    /// Close a notification by id, emit `NotificationClosed`, and refill.
    pub fn close(&mut self, id: u32, reason: CloseReason, conn: &X11Connection) {
        if let Some(pos) = self.displayed.iter().position(|p| p.notif.id == id) {
            let p = self.displayed.remove(pos);
            let mut notif = p.notif.clone();
            let sync_key = notif.sync_key.clone();
            p.destroy(conn);
            if !notif.transient && !notif.history_ignore {
                notif.icon_data = None; // Strip heavy raw image bytes before saving to history
                self.history.push(notif);
                if self.history.len() > 50 {
                    self.history.remove(0);
                }
            }
            self.emit_closed(id, reason);
            if let Some(key) = sync_key {
                // Only drop the sync mapping if it still points at this id; a
                // newer notification may have replaced it under the same key.
                if self.sync_map.get(&key).copied() == Some(id) {
                    self.sync_map.remove(&key);
                }
            }
            self.relayout(conn);
        } else {
            self.waiting.retain(|n| n.id != id);
        }
    }

    /// Periodic timeout sweep: close expired notifications.
    pub fn tick(&mut self, conn: &X11Connection) {
        if self.paused {
            return;
        }
        let now = Instant::now();
        let mut to_close: Vec<(u32, CloseReason)> = Vec::new();
        for p in &self.displayed {
            let t = if p.notif.expire_timeout < 0 {
                self.config.default_timeout_ms
            } else {
                p.notif.expire_timeout
            };
            if t > 0 && now.duration_since(p.notif.created).as_millis() as i32 >= t {
                to_close.push((p.notif.id, CloseReason::Expired));
            }
        }
        for (id, reason) in to_close {
            self.close(id, reason, conn);
        }
    }

    /// True if `window` belongs to one of our popups.
    pub fn owns_window(&self, window: u32) -> bool {
        self.displayed.iter().any(|p| p.window == window)
    }

    /// Handle a click inside a popup. Returns the action key invoked (if any).
    pub fn handle_click(&mut self, conn: &X11Connection, window: u32, x: i16, y: i16) -> Option<String> {
        let action = self
            .displayed
            .iter()
            .find(|p| p.window == window)
            .and_then(|p| p.hit_test(x, y));
        let id = self.displayed.iter().find(|p| p.window == window).map(|p| p.notif.id);
        match (action, id) {
            (Some(render::ClickResult::Action(key)), Some(id)) => {
                self.emit_action(id, &key);
                self.close(id, CloseReason::User, conn);
                Some(key)
            }
            (Some(render::ClickResult::Dismiss), Some(id)) => {
                self.close(id, CloseReason::User, conn);
                None
            }
            _ => None,
        }
    }

    fn emit_closed(&self, id: u32, reason: CloseReason) {
        if let Ok(sig) = Message::new_signal(
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
            "NotificationClosed",
        ) {
            let _ = self.dbus_send(sig.append2(id, reason as u32));
        }
    }

    fn emit_action(&self, id: u32, action_key: &str) {
        if let Ok(sig) = Message::new_signal(
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
            "ActionInvoked",
        ) {
            let _ = self.dbus_send(sig.append2(id, action_key.to_string()));
        }
    }

    /// Handle the `org.dunstproject.cmd0` control interface (a `dunstctl`
    /// compatible subset).
    fn handle_control(&mut self, member: &str, msg: Message, conn: &X11Connection) {
        match member {
            "ping" => {
                let _ = self.dbus_send(msg.method_return().append1("pong"));
            }
            "pause" => {
                self.set_paused(true);
                let _ = self.dbus_send(msg.method_return());
            }
            "resume" => {
                self.set_paused(false);
                let _ = self.dbus_send(msg.method_return());
            }
            "isPaused" => {
                let _ = self.dbus_send(msg.method_return().append1(self.paused()));
            }
            "closeAll" => {
                let ids: Vec<u32> = self.displayed.iter().map(|p| p.notif.id).collect();
                for id in ids {
                    self.close(id, CloseReason::User, conn);
                }
                let _ = self.dbus_send(msg.method_return());
            }
            "close" => {
                let id: u32 = msg.read1().unwrap_or(0);
                self.close(id, CloseReason::ClosedByCaller, conn);
                let _ = self.dbus_send(msg.method_return());
            }
            "closeLast" => {
                if let Some(id) = self.displayed.last().map(|p| p.notif.id) {
                    self.close(id, CloseReason::User, conn);
                }
                let _ = self.dbus_send(msg.method_return());
            }
            "historyCount" => {
                let _ = self.dbus_send(msg.method_return().append1(self.history.len() as u32));
            }
            "clearHistory" => {
                self.history.clear();
                let _ = self.dbus_send(msg.method_return());
            }
            "removeFromHistory" => {
                let id: u32 = msg.read1().unwrap_or(0);
                self.history.retain(|n| n.id != id);
                let _ = self.dbus_send(msg.method_return());
            }
            "popHistory" => {
                // Re-display the most recent history entry if a slot is free.
                if let Some(n) = self.history.pop() {
                    self.enqueue(n, conn);
                }
                let _ = self.dbus_send(msg.method_return());
            }
            "configReload" => {
                self.reload_config(conn);
                let _ = self.dbus_send(msg.method_return());
            }
            "history" | "getHistory" => {
                let hist: Vec<(u32, String, String, String, u32)> = self
                    .history
                    .iter()
                    .map(|n| {
                        (
                            n.id,
                            n.app_name.clone(),
                            n.summary.clone(),
                            n.body.clone(),
                            n.urgency as u32,
                        )
                    })
                    .collect();
                let _ = self.dbus_send(msg.method_return().append1(hist));
            }
            "context" => {
                let id: u32 = msg.read1().unwrap_or(0);
                let actions: Vec<(String, String)> = self
                    .displayed
                    .iter()
                    .find(|p| p.notif.id == id)
                    .map(|p| p.notif.actions.clone())
                    .or_else(|| {
                        self.history
                            .iter()
                            .find(|n| n.id == id)
                            .map(|n| n.actions.clone())
                    })
                    .unwrap_or_default();
                let _ = self.dbus_send(msg.method_return().append2(id, actions));
            }
            _ => {
                let name = ErrorName::new("org.freedesktop.DBus.Error.UnknownMethod").unwrap();
                let _ = self.dbus_send(msg.error(&name, c"no such control method"));
            }
        }
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    /// Redraw a popup after an Expose event.
    pub fn redraw_window(&self, conn: &X11Connection, window: u32) {
        if let Some(p) = self.displayed.iter().find(|p| p.window == window) {
            if let Err(e) = p.redraw(conn) {
                log::warn!("Failed to redraw notification popup: {}", e);
            }
        }
    }

    pub fn destroy(&mut self, conn: &X11Connection) {
        for p in self.displayed.drain(..) {
            p.destroy(conn);
        }
    }

    /// Reload the configuration file and re-layout visible popups.
    pub fn reload_config(&mut self, conn: &X11Connection) {
        self.config = Self::load_config(&Self::config_path());
        log::info!(
            "Notification config reloaded: max_visible={}, width={}, rules={}",
            self.config.max_visible,
            self.config.width,
            self.config.rules.len()
        );
        self.relayout(conn);
    }

    /// Resolve the config file path: `~/.config/dunst/dunstrc`, falling back
    /// to `~/.config/rustbox/notifications.conf`.
    fn config_path() -> PathBuf {
        let base = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var("HOME").ok().map(|h| format!("{}/.config", h)))
            .unwrap_or_else(|| ".config".to_string());
        let dunst = Path::new(&base).join("dunst").join("dunstrc");
        if dunst.exists() {
            return dunst;
        }
        Path::new(&base).join("rustbox").join("notifications.conf")
    }

    /// Apply matching rules to `notif` in order, returning whether the
    /// notification should be suppressed from display.
    fn apply_rules(notif: &mut RawNotification, rules: &[Rule]) -> bool {
        let mut skip = false;
        for r in rules {
            let app_ok = match &r.appname_re {
                Some(re) => re.is_match(&notif.app_name),
                None => true,
            };
            let sum_ok = match &r.summary_re {
                Some(re) => re.is_match(&notif.summary),
                None => true,
            };
            let body_ok = match &r.body_re {
                Some(re) => re.is_match(&notif.body),
                None => true,
            };
            let urg_ok = match r.urgency {
                Some(u) => notif.urgency == u,
                None => true,
            };
            let cat_ok = match &r.category_re {
                Some(re) => notif.category.as_deref().map(|c| re.is_match(c)).unwrap_or(false),
                None => true,
            };
            let tag_ok = match &r.stack_tag_re {
                Some(re) => notif.stack_tag.as_deref().map(|c| re.is_match(c)).unwrap_or(false),
                None => true,
            };
            let de_ok = match &r.desktop_entry_re {
                Some(re) => {
                    notif.desktop_entry.as_deref().map(|c| re.is_match(c)).unwrap_or(false)
                }
                None => true,
            };
            if !(app_ok && sum_ok && body_ok && urg_ok && cat_ok && tag_ok && de_ok) {
                continue;
            }
            if let Some(t) = r.expire_timeout {
                notif.expire_timeout = t;
            }
            if let Some(u) = r.set_urgency {
                notif.urgency = u;
            }
            if let Some(icon) = &r.new_icon {
                notif.app_icon = icon.clone();
            }
            if let Some(t) = r.set_transient {
                notif.transient = t;
            }
            if r.history_ignore {
                notif.history_ignore = true;
            }
            if r.skip_display {
                skip = true;
            }
        }
        skip
    }

    /// Parse a dunst-style config file (`dunstrc`-like) into a `Config`.
    /// Unknown keys and sections are ignored; missing files yield defaults.
    pub fn load_config(path: &Path) -> Config {
        let mut cfg = Config::default();
        let data = match std::fs::read_to_string(path) {
            Ok(d) => d,
            Err(_) => {
                log::debug!("No notification config at {:?}; using defaults", path);
                return cfg;
            }
        };
        let mut section = String::new();
        let mut rules: Vec<Rule> = Vec::new();
        let mut cur: Option<Rule> = None;
        let reserved: &[&str] = &[
            "global",
            "urgency_low",
            "urgency_normal",
            "urgency_critical",
            "shortcuts",
            "experimental",
            "frame",
        ];

        for raw_line in data.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                if let Some(r) = cur.take() {
                    rules.push(r);
                }
                let name = line[1..line.len() - 1].trim().to_string();
                section = name.clone();
                if !reserved.contains(&name.as_str()) {
                    let mut r = Rule::default();
                    let app = name.strip_prefix('@').unwrap_or(&name);
                    if !app.is_empty() {
                        r.appname_re = Regex::new(app).ok();
                    }
                    cur = Some(r);
                }
                continue;
            }
            let (key, value) = match line.split_once('=') {
                Some((k, v)) => (
                    k.trim().to_string(),
                    v.trim().trim_matches('"').to_string(),
                ),
                None => continue,
            };
            match section.as_str() {
                "global" => apply_global(&mut cfg, &key, &value),
                "urgency_low" => apply_urgency(&mut cfg, 0, &key, &value),
                "urgency_normal" => apply_urgency(&mut cfg, 1, &key, &value),
                "urgency_critical" => apply_urgency(&mut cfg, 2, &key, &value),
                _ => {
                    if let Some(r) = cur.as_mut() {
                        apply_rule_key(r, &key, &value);
                    }
                }
            }
        }
        if let Some(r) = cur.take() {
            rules.push(r);
        }
        cfg.rules = rules;
        cfg
    }
}

/// Apply a `[global]` key/value pair to `cfg`.
fn apply_global(cfg: &mut Config, key: &str, value: &str) {
    match key {
        "width" => cfg.width = value.parse().unwrap_or(cfg.width),
        "margin" => cfg.margin = value.parse().unwrap_or(cfg.margin),
        "gap" => cfg.gap = value.parse().unwrap_or(cfg.gap),
        "max_visible" => cfg.max_visible = value.parse().unwrap_or(cfg.max_visible),
        "timeout" | "default_timeout" => {
            cfg.default_timeout_ms = value.parse().unwrap_or(cfg.default_timeout_ms)
        }
        "monitor" => cfg.monitor = value.parse().unwrap_or(cfg.monitor),
        "font_scale" => cfg.font_scale = value.parse().unwrap_or(cfg.font_scale).max(1),
        "origin" | "corner" => cfg.origin = parse_corner(value),
        "font" => {
            // Parse a trailing point size like "Mono 12" to pick a bitmap scale.
            if let Some(size) = value.split_whitespace().last().and_then(|s| s.parse::<u16>().ok()) {
                cfg.font_scale = (size / 6).clamp(1, 4);
            }
        }
        "background" => {
            if let Some(c) = parse_color(value) {
                cfg.bg = c;
            }
        }
        "foreground" => {
            if let Some(c) = parse_color(value) {
                cfg.fg = c;
            }
        }
        "frame_color" | "frame" => {
            if let Some(c) = parse_color(value) {
                cfg.frame = c;
            }
        }
        "body_color" | "color" => {
            if let Some(c) = parse_color(value) {
                cfg.body = c;
            }
        }
        _ => {}
    }
}

/// Apply a key/value from an `[urgency_low|normal|critical]` section.
fn apply_urgency(cfg: &mut Config, idx: usize, key: &str, value: &str) {
    match key {
        "background" => {
            if let Some(c) = parse_color(value) {
                cfg.urgency_colors[idx] = c;
            }
        }
        "foreground" | "color" => {
            if let Some(c) = parse_color(value) {
                cfg.body = c;
            }
        }
        _ => {}
    }
}

/// Parse `#rrggbb` / `rrggbb` into an RGB triple.
fn parse_color(value: &str) -> Option<(u8, u8, u8)> {
    let s = value.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Apply a rule-section key/value pair to `r`.
fn apply_rule_key(r: &mut Rule, key: &str, value: &str) {
    match key {
        "appname" => r.appname_re = Regex::new(value).ok(),
        "summary" => r.summary_re = Regex::new(value).ok(),
        "body" => r.body_re = Regex::new(value).ok(),
        "urgency" => r.urgency = parse_urgency(value),
        "timeout" | "expire_timeout" => r.expire_timeout = value.parse().ok(),
        "set_urgency" => r.set_urgency = parse_urgency(value),
        "icon" | "new_icon" => r.new_icon = Some(value.to_string()),
        "skip_display" => r.skip_display = parse_bool(value),
        "category" => r.category_re = Regex::new(value).ok(),
        "stack_tag" => r.stack_tag_re = Regex::new(value).ok(),
        "desktop_entry" => r.desktop_entry_re = Regex::new(value).ok(),
        "set_transient" | "transient" => r.set_transient = Some(parse_bool(value)),
        "history_ignore" => r.history_ignore = parse_bool(value),
        _ => {}
    }
}

fn parse_corner(value: &str) -> Corner {
    match value.to_ascii_lowercase().replace('-', "_").as_str() {
        "top_right" | "topright" => Corner::TopRight,
        "top_left" | "topleft" => Corner::TopLeft,
        "bottom_right" | "bottomright" => Corner::BottomRight,
        "bottom_left" | "bottomleft" => Corner::BottomLeft,
        _ => Corner::TopRight,
    }
}

fn parse_urgency(value: &str) -> Option<Urgency> {
    match value.to_ascii_lowercase().as_str() {
        "low" => Some(Urgency::Low),
        "normal" => Some(Urgency::Normal),
        "critical" => Some(Urgency::Critical),
        _ => None,
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "true" | "yes" | "1" | "on")
}

/// Resolve a theme icon name to a file path, searching the standard
/// freedesktop icon directories. Returns `None` if not found. SVG icons are
/// skipped because the `image` crate cannot decode them.
fn resolve_theme_icon(name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    let name = name.trim_end_matches(".png").trim_end_matches(".svg");
    let themes = [
        "hicolor", "Adwaita", "gnome", "breeze", "breeze-dark", "oxygen",
    ];
    let sizes = [
        "256x256", "256", "128x128", "128", "96x96", "96", "64x64", "64",
        "48x48", "48", "32x32", "32", "24x24", "24", "16x16", "16",
    ];
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(xdg) = std::env::var("XDG_DATA_DIRS").ok() {
        for d in xdg.split(':') {
            dirs.push(PathBuf::from(d));
        }
    }
    dirs.push(PathBuf::from("/usr/local/share"));
    dirs.push(PathBuf::from("/usr/share"));
    if let Some(home) = std::env::var("HOME").ok() {
        dirs.push(PathBuf::from(format!("{}/.local/share", home)));
        dirs.push(PathBuf::from(format!("{}/.icons", home)));
    }
    dirs.push(PathBuf::from("/usr/share/pixmaps"));

    for dir in &dirs {
        let p = dir.join(format!("{}.png", name));
        if p.exists() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    for dir in &dirs {
        for theme in &themes {
            for size in &sizes {
                let p = dir
                    .join("icons")
                    .join(theme)
                    .join(size)
                    .join("apps")
                    .join(format!("{}.png", name));
                if p.exists() {
                    return Some(p.to_string_lossy().into_owned());
                }
            }
        }
    }
    None
}

/// Pair the flat `actions` array ("key1", "label1", ...) into tuples.
fn pair_actions(actions: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < actions.len() {
        out.push((actions[i].clone(), actions[i + 1].clone()));
        i += 2;
    }
    out
}

/// Extract the interesting fields from the `hints` dict of a `Notify` call.
fn parse_hints(hints: &HashMap<String, Variant<Box<dyn dbus::arg::RefArg>>>) -> ParsedHints {
    let mut out = ParsedHints {
        urgency: Urgency::Normal,
        ..Default::default()
    };
    for (k, v) in hints {
        match k.as_str() {
            "urgency" => {
                if let Some(u) = v.0.as_u64() {
                    out.urgency = Urgency::from_u32(u as u32);
                }
            }
            "transient" => {
                if let Some(t) = v.0.as_u64() {
                    out.transient = t != 0;
                }
            }
            "resident" => {
                if let Some(t) = v.0.as_u64() {
                    out.transient = !(t != 0);
                }
            }
            "image-path" => {
                if let Some(s) = v.0.as_str() {
                    out.image_path = Some(s.to_string());
                }
            }
            "desktop-entry" => {
                if let Some(s) = v.0.as_str() {
                    out.desktop_entry = Some(s.to_string());
                }
            }
            "x-canonical-private-synchronous" => {
                if let Some(s) = v.0.as_str() {
                    out.sync_key = Some(s.to_string());
                }
            }
            "x-dunst-stack-tag" => {
                if let Some(s) = v.0.as_str() {
                    out.stack_tag = Some(s.to_string());
                }
            }
            "category" => {
                if let Some(s) = v.0.as_str() {
                    out.category = Some(s.to_string());
                }
            }
            "value" => {
                if let Some(val) = v.0.as_u64() {
                    out.progress = Some((val as u8).clamp(0, 100));
                }
            }
            "image-data" => {
                if let Some(bytes) = decode_image_data(&v.0) {
                    out.image_data = Some(bytes);
                }
            }
            _ => {}
        }
    }
    out
}

/// Decode the freedesktop `image-data` struct `(iiibiiay)` into RGBA8 pixels.
/// Best-effort: any failure yields `None` and the icon is simply omitted.
fn decode_image_data(value: &dyn dbus::arg::RefArg) -> Option<Vec<u8>> {
    let mut iter = value.as_iter()?;
    let w = iter.next()?.as_i64()? as u32;
    let h = iter.next()?.as_i64()? as u32;
    let _rowstride = iter.next()?.as_i64()?;
    let alpha = iter.next()?.as_u64()? != 0;
    let _bits = iter.next()?.as_i64()?;
    let samples = iter.next()?.as_i64()? as usize;
    let pixels: Vec<u8> = iter
        .next()?
        .as_iter()?
        .filter_map(|a| a.as_u64().map(|x| x as u8))
        .collect();
    if w == 0 || h == 0 || samples < 3 {
        return None;
    }
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    if samples == 4 {
        out.extend_from_slice(&pixels);
    } else {
        // Expand RGB -> RGBA.
        for px in pixels.chunks(samples) {
            out.push(px[0]);
            out.push(px[1]);
            out.push(px[2]);
            out.push(if alpha { *px.get(3).unwrap_or(&0xff) } else { 0xff });
        }
    }
    Some(out)
}

/// Builder for complex notifications raised entirely in Rust, without D-Bus.
///
/// Example:
/// ```text
/// use rustbox::notify::notify;
/// notify("Rustbox", "Compilado!")
///     .body("Build concluído sem erros")
///     .icon("rust")
///     .urgency(rustbox::notify::Urgency::Normal)
///     .timeout(4000)
///     .action("open", "Abrir")
///     .show()
///     .ok();
/// ```
pub struct NotificationBuilder {
    notif: RawNotification,
}

impl NotificationBuilder {
    /// Start a notification from `app_name` with the given `summary`.
    pub fn new(app_name: &str, summary: &str) -> Self {
        Self {
            notif: RawNotification {
                id: 0,
                app_name: app_name.to_string(),
                app_icon: String::new(),
                summary: summary.to_string(),
                body: String::new(),
                actions: Vec::new(),
                urgency: Urgency::Normal,
                transient: false,
                expire_timeout: -1,
                created: Instant::now(),
                icon_data: None,
                sync_key: None,
                stack_tag: None,
                category: None,
                desktop_entry: None,
                progress: None,
                history_ignore: false,
            },
        }
    }

    pub fn body(mut self, body: &str) -> Self {
        self.notif.body = body.to_string();
        self
    }

    /// Icon by file path or freedesktop theme icon name.
    pub fn icon(mut self, icon: &str) -> Self {
        self.notif.app_icon = icon.to_string();
        self
    }

    /// Raw RGBA8 icon bytes (precedence over `icon`).
    pub fn icon_data(mut self, rgba: Vec<u8>) -> Self {
        self.notif.icon_data = Some(rgba);
        self
    }

    pub fn urgency(mut self, u: Urgency) -> Self {
        self.notif.urgency = u;
        self
    }

    /// Milliseconds; -1 => configured default, 0 => sticky.
    pub fn timeout(mut self, ms: i32) -> Self {
        self.notif.expire_timeout = ms;
        self
    }

    pub fn transient(mut self, b: bool) -> Self {
        self.notif.transient = b;
        self
    }

    pub fn stack_tag(mut self, tag: &str) -> Self {
        self.notif.stack_tag = Some(tag.to_string());
        self
    }

    pub fn category(mut self, c: &str) -> Self {
        self.notif.category = Some(c.to_string());
        self
    }

    pub fn desktop_entry(mut self, d: &str) -> Self {
        self.notif.desktop_entry = Some(d.to_string());
        self
    }

    /// Render a 0..=100 progress bar in the popup.
    pub fn progress(mut self, value: u8) -> Self {
        self.notif.progress = Some(value.min(100));
        self
    }

    /// `x-canonical-private-synchronous` key: replaces an existing
    /// notification with the same key instead of stacking.
    pub fn sync_key(mut self, key: &str) -> Self {
        self.notif.sync_key = Some(key.to_string());
        self
    }

    /// Drop from history when closed.
    pub fn history_ignore(mut self, b: bool) -> Self {
        self.notif.history_ignore = b;
        self
    }

    /// Add an action button (key + human label).
    pub fn action(mut self, key: &str, label: &str) -> Self {
        self.notif.actions.push((key.to_string(), label.to_string()));
        self
    }

    /// Deliver the notification through the in-process Rust channel (no D-Bus).
    pub fn show(self) -> Result<(), std::sync::mpsc::SendError<NotifyCommand>> {
        NotifyDaemon::channel().send(NotifyCommand::Notify(self.notif))
    }
}

/// Convenience constructor for the [`NotificationBuilder`] (D-Bus-free).
pub fn notify(app_name: &str, summary: &str) -> NotificationBuilder {
    NotificationBuilder::new(app_name, summary)
}

/// Deliver a pre-built [`RawNotification`] through the in-process channel.
pub fn notify_raw(notif: RawNotification) -> Result<(), std::sync::mpsc::SendError<NotifyCommand>> {
    NotifyDaemon::channel().send(NotifyCommand::Notify(notif))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_notif(app: &str, summary: &str, body: &str, urgency: Urgency) -> RawNotification {
        RawNotification {
            id: 0,
            app_name: app.to_string(),
            app_icon: String::new(),
            summary: summary.to_string(),
            body: body.to_string(),
            actions: Vec::new(),
            urgency,
            transient: false,
            expire_timeout: -1,
            created: Instant::now(),
            icon_data: None,
            sync_key: None,
            stack_tag: None,
            category: None,
            desktop_entry: None,
            progress: None,
            history_ignore: false,
        }
    }

    #[test]
    fn config_parsing_global_and_rules() {
        let dir = std::env::temp_dir();
        let path = dir.join("rustbox_test_notifications.conf");
        let content = "\
[global]
width=320
margin=20
origin=top-left

[firefox]
skip_display=true

[crit]
urgency=critical
set_urgency=critical
timeout=12000
";
        std::fs::write(&path, content).unwrap();
        let cfg = NotifyDaemon::load_config(&path);
        std::fs::remove_file(&path).ok();

        assert_eq!(cfg.width, 320);
        assert_eq!(cfg.margin, 20);
        assert_eq!(cfg.origin, Corner::TopLeft);
        assert_eq!(cfg.rules.len(), 2);

        // First rule is the [firefox] section (top-down order).
        assert_eq!(cfg.rules[0].appname_re.as_ref().unwrap().as_str(), "firefox");
        assert!(cfg.rules[0].skip_display);

        // Second rule is [crit].
        assert!(cfg.rules[1].urgency == Some(Urgency::Critical));
        assert!(cfg.rules[1].set_urgency == Some(Urgency::Critical));
        assert_eq!(cfg.rules[1].expire_timeout, Some(12000));
    }

    #[test]
    fn rules_skip_and_override() {
        let rules = vec![
            Rule {
                appname_re: Regex::new("firefox").ok(),
                skip_display: true,
                ..Default::default()
            },
            Rule {
                summary_re: Regex::new("alert").ok(),
                set_urgency: Some(Urgency::Critical),
                expire_timeout: Some(9000),
                ..Default::default()
            },
        ];

        // Matching app -> skipped.
        let mut n1 = sample_notif("firefox", "hi", "body", Urgency::Normal);
        assert!(NotifyDaemon::apply_rules(&mut n1, &rules));

        // Non-matching -> not skipped, unchanged.
        let mut n2 = sample_notif("notify-send", "hi", "body", Urgency::Normal);
        assert!(!NotifyDaemon::apply_rules(&mut n2, &rules));
        assert_eq!(n2.urgency, Urgency::Normal);
        assert_eq!(n2.expire_timeout, -1);

        // Summary match -> override urgency + timeout, not skipped.
        let mut n3 = sample_notif("notify-send", "alert now", "body", Urgency::Low);
        assert!(!NotifyDaemon::apply_rules(&mut n3, &rules));
        assert_eq!(n3.urgency, Urgency::Critical);
        assert_eq!(n3.expire_timeout, 9000);
    }

    #[test]
    fn missing_config_yields_defaults() {
        let cfg = NotifyDaemon::load_config(Path::new("/nonexistent/path/dunst/dunstrc"));
        assert_eq!(cfg.width, 380);
        assert_eq!(cfg.max_visible, 5);
        assert!(cfg.rules.is_empty());
    }

    #[test]
    fn config_parsing_colors() {
        let dir = std::env::temp_dir();
        let path = dir.join("rustbox_test_colors.conf");
        let content = "\
[global]
background=#101010
foreground=#f0f0f0
frame_color=#333333
body_color=#cccccc

[urgency_critical]
background=#ff0000
";
        std::fs::write(&path, content).unwrap();
        let cfg = NotifyDaemon::load_config(&path);
        std::fs::remove_file(&path).ok();

        assert_eq!(cfg.bg, (16, 16, 16));
        assert_eq!(cfg.fg, (240, 240, 240));
        assert_eq!(cfg.frame, (51, 51, 51));
        assert_eq!(cfg.body, (204, 204, 204));
        assert_eq!(cfg.urgency_colors[2], (255, 0, 0));

        let t = cfg.theme();
        assert_eq!(t.bg, (16, 16, 16));
        assert_eq!(t.urgency[2], (255, 0, 0));
    }

    #[test]
    fn theme_icon_resolver_finds_a_known_icon() {        // `ark` ships a PNG under /usr/share/icons on typical desktops; this
        // test is tolerant — it only asserts the resolver returns *a* path
        // when one exists, and stays silent when the icon theme is absent.
        if let Some(path) = resolve_theme_icon("ark") {
            assert!(Path::new(&path).exists());
        }
        // Unknown names must not resolve to a bogus path.
        assert!(resolve_theme_icon("definitely-not-a-real-icon-xyz").is_none());
    }
}
