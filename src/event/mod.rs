use std::os::unix::io::AsRawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use libc;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;

use crate::command::CommandRegistry;
use crate::notify::NotifyDaemon;
use crate::screen::BScreen;
use crate::sni::{SniEvent, SniManager};
use crate::x11::{Atom, Event, X11Connection};

pub struct Rustbox {
    conn: X11Connection,
    screens: Vec<BScreen>,
    running: Arc<AtomicBool>,
    command_registry: CommandRegistry,
    display_name: String,
    config_dir: String,
    restart: Arc<AtomicBool>,
    exit_code: i32,
    remote_buffer: String,
    notify: Option<NotifyDaemon>,
    sni: Option<SniManager>,
}

impl Rustbox {
    pub fn new(conn: X11Connection, display_name: &str, config_dir: &str) -> Result<Self, anyhow::Error> {
        let running = Arc::new(AtomicBool::new(true));
        let restart = Arc::new(AtomicBool::new(false));
        let mut rustbox = Self {
            conn,
            screens: Vec::new(),
            running: running.clone(),
            command_registry: CommandRegistry::new(),
            display_name: display_name.to_string(),
            config_dir: config_dir.to_string(),
            restart,
            exit_code: 0,
            remote_buffer: String::new(),
            notify: None,
            sni: None,
        };

        rustbox.init_screens()?;
        log::info!("Rustbox::new: init_screens ok");

        // Create the notification channel before anything else so the zbus
        // thread and the in-process sender can share it.
        let (notify_tx, notify_rx) = std::sync::mpsc::channel();
        let _ = crate::notify::NOTIFY_SENDER.set(notify_tx.clone());

        // Signal channel for emitting D-Bus signals from the zbus thread.
        let (signal_tx, signal_rx) = futures_channel::mpsc::unbounded();

        rustbox.notify = Some(NotifyDaemon::new(&rustbox.conn, notify_rx, signal_tx));
        log::info!("Rustbox::new: notify ok");

        // Sync notification daemon dimensions with the actual screen geometry.
        if let Some(screen) = rustbox.screens.first() {
            let rw = screen.width();
            let rh = screen.height();
            if let Some(n) = rustbox.notify.as_mut() {
                n.set_screen_size(rw, rh);
                log::debug!("NotifyDaemon: synced screen size {}x{}", rw, rh);
            }
        }
        match SniManager::new(notify_tx, signal_rx) {
            Ok(sni) => {
                let activator = sni.activator();
                let ctx_menu = sni.context_menu_activator();
                for screen in rustbox.screens.iter_mut() {
                    screen.set_sni_activator(activator.clone());
                    screen.set_sni_context_menu(ctx_menu.clone());
                }
                rustbox.sni = Some(sni);
                log::info!("StatusNotifierWatcher (SNI) ativo");
            }
            Err(e) => log::warn!("SNI indisponível (sem session bus?): {}", e),
        }
        Ok(rustbox)
    }

    fn init_screens(&mut self) -> Result<(), anyhow::Error> {
        use x11rb::protocol::xproto::{ChangeWindowAttributesAux, EventMask};

        crate::screen::trace_step("init_screens: BScreen::new ...");
        let bscreen = BScreen::new(
            0,
            self.conn.clone(),
            "default",
            self.running.clone(),
            self.restart.clone(),
        )?;
        crate::screen::trace_step("init_screens: BScreen::new done");

        self.conn.conn().change_window_attributes(
            self.conn.root_window(),
            &ChangeWindowAttributesAux::new()
                .event_mask(
                    EventMask::SUBSTRUCTURE_REDIRECT
                        | EventMask::SUBSTRUCTURE_NOTIFY
                        | EventMask::PROPERTY_CHANGE
                        | EventMask::BUTTON_PRESS
                        | EventMask::BUTTON_RELEASE
                        | EventMask::KEY_PRESS
                        | EventMask::KEY_RELEASE
                        | EventMask::POINTER_MOTION
                        | EventMask::ENTER_WINDOW
                        | EventMask::LEAVE_WINDOW,
                ),
        )?;

        // Publish _NET_SUPPORTED so clients know we are EWMH-compliant.
        self.publish_net_supported()?;

        // Receive RandR screen-size changes (e.g. termux-x11 / Xephyr resize)
        // so the toolbar, slit, tray and managed windows can reflow.
        if let Err(e) = x11rb::protocol::randr::select_input(
            self.conn.conn(),
            self.conn.root_window(),
            x11rb::protocol::randr::NotifyMask::SCREEN_CHANGE,
        ) {
            log::warn!("RandR select_input failed (resize handling disabled): {}", e);
        }

        self.screens.push(bscreen);
        self.conn.flush()?;
        Ok(())
    }

    /// Set the `_NET_SUPPORTED` property on the root window listing every
    /// EWMH atom we understand. This tells clients (like kitty, alacritty,
    /// etc.) that we support the EWMH spec and they can rely on properties
    /// such as `_NET_WORKAREA` for correct positioning.
    fn publish_net_supported(&self) -> Result<(), anyhow::Error> {
        use x11rb::protocol::xproto::ConnectionExt as _;

        let root = self.conn.root_window();
        let atoms = self.conn.atoms();
        let atom_type = atoms.get(Atom::AtomEnum);

        // Collect all EWMH-related atoms we support.
        let supported: Vec<u32> = vec![
            Atom::NetSupported,
            Atom::NetClientList,
            Atom::NetClientListStacking,
            Atom::NetNumberOfDesktops,
            Atom::NetDesktopViewport,
            Atom::NetCurrentDesktop,
            Atom::NetDesktopNames,
            Atom::NetActiveWindow,
            Atom::NetWorkarea,
            Atom::NetSupportingWmCheck,
            Atom::NetCloseWindow,
            Atom::NetWmName,
            Atom::NetWmVisibleName,
            Atom::NetWmIconName,
            Atom::NetWmVisibleIconName,
            Atom::NetWmDesktop,
            Atom::NetWmState,
            Atom::NetWmStateMaximizedVert,
            Atom::NetWmStateMaximizedHorz,
            Atom::NetWmStateFullscreen,
            Atom::NetWmStateHidden,
            Atom::NetWmStateShaded,
            Atom::NetWmStateSkipTaskbar,
            Atom::NetWmStateSkipPager,
            Atom::NetWmStateSticky,
            Atom::NetWmStateAbove,
            Atom::NetWmStateBelow,
            Atom::NetWmStateDemandsAttention,
            Atom::NetWmAllowedActions,
            Atom::NetWmActionMove,
            Atom::NetWmActionResize,
            Atom::NetWmActionMinimize,
            Atom::NetWmActionShade,
            Atom::NetWmActionStick,
            Atom::NetWmActionMaximizeHorz,
            Atom::NetWmActionMaximizeVert,
            Atom::NetWmActionFullscreen,
            Atom::NetWmActionChangeDesktop,
            Atom::NetWmActionClose,
            Atom::NetWmActionAbove,
            Atom::NetWmActionBelow,
            Atom::NetWmWindowType,
            Atom::NetWmWindowTypeDesktop,
            Atom::NetWmWindowTypeDock,
            Atom::NetWmWindowTypeToolbar,
            Atom::NetWmWindowTypeMenu,
            Atom::NetWmWindowTypeUtility,
            Atom::NetWmWindowTypeSplash,
            Atom::NetWmWindowTypeDialog,
            Atom::NetWmWindowTypeNormal,
            Atom::NetWmStrut,
            Atom::NetWmStrutPartial,
            Atom::NetWmIconGeometry,
            Atom::NetWmIcon,
            Atom::NetWmPid,
            Atom::NetWmUserTime,
            Atom::NetWmUserTimeWindow,
            Atom::NetFrameExtents,
            Atom::NetFrameWindow,
            Atom::NetRequestFrameExtents,
        ]
        .into_iter()
        .map(|a| atoms.get(a))
        .collect();

        if atom_type != x11rb::NONE {
            // Convert u32 atom IDs to little-endian bytes for change_property
            let mut data_bytes: Vec<u8> = Vec::new();
            for v in &supported {
                data_bytes.extend_from_slice(&v.to_le_bytes());
            }
            self.conn.conn().change_property(
                x11rb::protocol::xproto::PropMode::REPLACE,
                root,
                atoms.get(Atom::NetSupported),
                atom_type,
                32,
                supported.len() as u32,
                &data_bytes,
            )?;
        }

        Ok(())
    }

    pub fn conn(&self) -> &X11Connection {
        &self.conn
    }

    pub fn screen(&self, num: usize) -> Option<&BScreen> {
        self.screens.get(num)
    }

    pub fn screen_mut(&mut self, num: usize) -> Option<&mut BScreen> {
        self.screens.get_mut(num)
    }

    pub fn current_screen(&self) -> &BScreen {
        &self.screens[0]
    }

    pub fn current_screen_mut(&mut self) -> &mut BScreen {
        &mut self.screens[0]
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn command_registry(&self) -> &CommandRegistry {
        &self.command_registry
    }

    pub fn should_restart(&self) -> bool {
        self.restart.load(Ordering::Relaxed)
    }

    pub fn set_restart(&mut self, restart: bool) {
        self.restart.store(restart, Ordering::Relaxed);
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }

    pub fn shutdown(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }

    pub fn handle_event(&mut self, event: &Event) -> Result<(), anyhow::Error> {
        // Notification popups are override-redirect windows we own; route
        // their input/expose to the daemon and stop here.
        if let Event::ButtonPress(e) = event {
            if let Some(n) = self.notify.as_mut() {
                if n.owns_window(e.event) {
                    n.handle_click(&self.conn, e.event, e.event_x, e.event_y);
                    return Ok(());
                }
            }
        }
        if let Event::Expose(e) = event {
            if let Some(n) = self.notify.as_mut() {
                if n.owns_window(e.window) {
                    n.redraw_window(&self.conn, e.window);
                    return Ok(());
                }
            }
        }

        // Remote commands (rustbox-remote) are addressed to the root window
        // and handled globally rather than per-screen.
        if let Event::ClientMessage(e) = event {
            let remote = self.conn.atoms().get(Atom::RustboxRemote);
            if e.type_ == remote {
                let data = e.data.as_data8();
                for &b in &data {
                    if b == 0 {
                        break;
                    }
                    self.remote_buffer.push(b as char);
                }
                // A NUL byte terminates the command; flush and execute.
                if data.contains(&0) || self.remote_buffer.len() >= 1024 {
                    let cmd = std::mem::take(&mut self.remote_buffer);
                    self.handle_remote_command(cmd.trim());
                }
            }
        }

        // Property-based remote command (reliable on servers that do not
        // deliver synthetic ClientMessages to the root window, e.g. termux-x11).
        if let Event::PropertyNotify(e) = event {
            let cmd_atom = self.conn.atoms().get(Atom::RustboxRemoteCmd);
            if cmd_atom != x11rb::NONE
                && e.window == self.conn.root_window()
                && e.atom == cmd_atom
            {
                if let Some(cmd) = self.read_remote_property() {
                    let cmd = cmd.trim().to_string();
                    if !cmd.is_empty() {
                        self.handle_remote_command(&cmd);
                    }
                }
            }
        }

        for screen in &mut self.screens {
            screen.handle_event(event)?;
        }
        Ok(())
    }

    /// Read the `RUSTBOX_REMOTE_CMD` property from the root window (set by
    /// `rustbox-remote`) and return its text payload.
    fn read_remote_property(&self) -> Option<String> {
        let cmd_atom = self.conn.atoms().get(Atom::RustboxRemoteCmd);
        if cmd_atom == x11rb::NONE {
            return None;
        }
        let cookie = self
            .conn
            .conn()
            .get_property(false, self.conn.root_window(), cmd_atom, x11rb::NONE, 0, 4096)
            .ok()?;
        let reply = cookie.reply().ok()?;
        String::from_utf8(reply.value).ok()
    }

    /// Process a command received from `rustbox-remote`. Only a fixed set of
    /// known commands is honoured (matching upstream behaviour).
    pub fn handle_remote_command(&mut self, cmd: &str) {
        log::info!("Remote command: {}", cmd);
        match cmd {
            "restart" | "restartwm" => {
                self.set_restart(true);
                self.shutdown();
            }
            "quit" | "exit" => {
                self.shutdown();
            }
            "reconfig" | "reconfigure" => {
                if let Err(e) = self.reconfigure() {
                    log::warn!("Reconfigure failed: {}", e);
                }
            }
            _ if cmd.starts_with("setworkspace") => {
                if let Some(arg) = cmd.split_whitespace().nth(1) {
                    if let Ok(n) = arg.parse::<u32>() {
                        if let Some(screen) = self.screen_mut(0) {
                            screen.set_current_workspace(n.saturating_sub(1));
                        }
                    }
                }
            }
            _ if cmd.starts_with("workspace rename") => {
                let parts: Vec<&str> = cmd.splitn(4, ' ').collect();
                if parts.len() >= 3 {
                    if let Ok(idx) = parts[2].parse::<u32>() {
                        let new_name = if parts.len() > 3 { parts[3..].join(" ") } else { String::new() };
                        if let Some(screen) = self.screen_mut(0) {
                            screen.set_workspace_name(idx, &new_name);
                        }
                    }
                }
            }
            _ => {
                log::warn!("Unknown remote command: {}", cmd);
            }
        }
    }

    /// Dispatch a single event, isolating a panic in any handler so one bad
    /// event can't take down the whole WM. With `panic = "unwind"` (default
    /// since we removed `panic = "abort"`), `catch_unwind` recovers and we log
    /// the failure instead of killing every managed window.
    fn dispatch_event(&mut self, e: &Event) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle_event(e)));
        match result {
            Ok(Ok(())) => {}
            Ok(Err(err)) => log::error!("Event handling error: {:?}", err),
            Err(_) => log::error!("PANIC ao processar evento (recuperado); ver ~/.rustbox/panic.log"),
        }
    }

    pub fn event_loop(&mut self) -> Result<(), anyhow::Error> {
        self.conn.flush()?;

        let mut last_clock_tick = std::time::Instant::now();
        const CLOCK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

        while self.is_running() {
            // Drain every already-queued event first so we never block while
            // work is pending. Handlers issue X requests of their own; we flush
            // once after the whole batch to coalesce them into a single write.
            while let Some(raw) = self.conn.conn().poll_for_event()? {
                if let Some(e) = Event::from_x11(raw) {
                    self.dispatch_event(&e);
                }
            }
            self.conn.flush()?;

            // Periodic clock redraw — the toolbar clock only updates on render,
            // and no X event arrives just because the second changed.
            if last_clock_tick.elapsed() >= CLOCK_INTERVAL {
                for screen in &mut self.screens {
                    let _ = screen.toolbar_render(&self.conn);
                    screen.blink_dialog_cursor();
                    let _ = screen.check_pending_closes();
                }
                self.conn.flush()?;
                last_clock_tick = std::time::Instant::now();
            }

            // Block until an X event actually arrives, a D-Bus notification
            // message arrives, or the clock interval elapses. With no events
            // and no pending clock tick the thread genuinely sleeps in the
            // kernel (poll(2)) instead of waking 100x/second.
            let timeout = CLOCK_INTERVAL.saturating_sub(last_clock_tick.elapsed());
            let timeout_ms = timeout.as_millis().min(i32::MAX as u128) as i32;

            let sni_fd = self.sni.as_ref().map(|s| s.wake_fd()).unwrap_or(-1);
            let mut pfds: [libc::pollfd; 2] = [
                libc::pollfd {
                    fd: self.conn.conn().stream().as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: sni_fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let nfds = if sni_fd >= 0 { 2 } else { 1 };
            let _ = unsafe { libc::poll(pfds.as_mut_ptr(), nfds, timeout_ms) };

            // Drain SNI events (StatusNotifierItem register/update/unregister)
            // delivered by the watcher thread via its self-pipe.
            if sni_fd >= 0
                && (pfds[1].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR)) != 0
            {
                let events: Vec<SniEvent> = if let Some(sni) = self.sni.as_mut() {
                    sni.drain_wake();
                    let mut v = Vec::new();
                    loop {
                        match sni.try_recv() {
                            Some(ev) => v.push(ev),
                            None => break,
                        }
                    }
                    v
                } else {
                    Vec::new()
                };
                if !events.is_empty() {
                    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        for ev in events {
                            for screen in self.screens.iter_mut() {
                                let _ = screen.sni_event(ev.clone());
                            }
                        }
                    }));
                    if let Err(payload) = res {
                        let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                            (*s).to_string()
                        } else if let Some(s) = payload.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "payload desconhecido".to_string()
                        };
                        log::error!("PANIC no loop SNI (recuperado): {}", msg);
                    }
                }
            }

            // Drain and dispatch in-process channel notifications, then
            // sweep for expired notifications.
            if let Some(n) = self.notify.as_mut() {
                let conn = &self.conn;
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    n.process_internal(conn);
                    n.tick(conn);
                }));
                if let Err(payload) = res {
                    let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                        (*s).to_string()
                    } else if let Some(s) = payload.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "payload desconhecido".to_string()
                    };
                    log::error!("PANIC no loop de notificacoes (recuperado): {}", msg);
                }
            }
        }

        // Clean up docked UI before exiting.
        for screen in &mut self.screens {
            if let Err(e) = screen.destroy() {
                log::warn!("Error destroying screen resources: {}", e);
            }
        }
        if let Some(n) = self.notify.as_mut() {
            n.destroy(&self.conn);
        }

        Ok(())
    }

    pub fn reconfigure(&mut self) -> Result<(), anyhow::Error> {
        log::info!("Reconfiguring Rustbox...");
        for screen in &mut self.screens {
            screen.reconfigure()?;
        }
        if let Some(n) = self.notify.as_mut() {
            // Use the BScreen's live root geometry (just refreshed by
            // screen.reconfigure above) instead of conn.screen() which
            // holds the *initial* connection-setup values and never updates
            // on RandR events.
            if let Some(first) = self.screens.first() {
                n.set_screen_size(first.width(), first.height());
            }
            n.reload_config(&self.conn);
        }
        Ok(())
    }
}
