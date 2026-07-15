use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{
    self, Allow, ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt as _, EventMask, ModMask,
};
use x11rb::NONE;

use crate::config::FocusConfig;
use crate::core::{Rectangle, Strut};
use crate::keys::{self, KeyAction, KeyBinding};
use crate::menu::menu::Menu;
use crate::menu::menuitem::{MenuItem, MenuItemType};
use crate::render::font::Font;
#[cfg(feature = "wallpaper")]
use crate::render::image::Image;
use crate::slit::FbSlit;
use crate::sni::{ActivateRequest, SniEvent};
use futures_channel::mpsc::UnboundedSender;
use crate::toolbar::{FbToolbar, ToolbarAction, ToolbarPlacement};
use crate::tray::FbTray;
use crate::window::{FbWinFrame, RustboxWindow, WinClient, WindowId, WindowManager};
use crate::workspace::Workspace;
use crate::x11::{Atom, Event, X11Connection};

/// Wallpaper bundled at compile time (scr file of the project).
#[cfg(feature = "wallpaper")]
const WALLPAPER_BYTES: &[u8] = include_bytes!("../wallpaper.png");

/// Append a line to ~/.rustbox/startup.log and flush, so a startup failure
/// can be localized even when no terminal is available.
pub fn trace_step(msg: &str) {
    if let Ok(home) = std::env::var("HOME") {
        let path = format!("{}/.rustbox/startup.log", home);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            use std::io::Write;
            let _ = writeln!(f, "[startup] {}", msg);
            let _ = f.flush();
        }
    }
}


/// Log (instead of silently discarding) best-effort X11 protocol errors,
/// while still dropping the unused cookie/value. Used to replace the many
/// `let _ = conn...(...)` sites that matter for correctness.
trait LogIfErr<T> {
    fn log_if_err(self, ctx: &str) -> Option<T>;
}

impl<T, E: std::fmt::Debug> LogIfErr<T> for Result<T, E> {
    fn log_if_err(self, ctx: &str) -> Option<T> {
        match self {
            Ok(v) => Some(v),
            Err(e) => {
                log::debug!("{ctx}: {e:?}");
                None
            }
        }
    }
}

pub type ScreenNum = usize;

/// A pending graceful close: we asked the client to delete itself via
/// `WM_DELETE_WINDOW` and are waiting for it to actually unmap/destroy before
/// forcibly killing it.
struct PendingClose {
    window: WindowId,
    sent_at: std::time::Instant,
    timeout: std::time::Duration,
}

#[derive(Debug, Clone, Copy)]
enum DialogAction {
    RenameWorkspace(u32),
    RunCommand,
}

struct DialogState {
    window: u32,
    frame: u32,
    gc: u32,
    title: String,
    text: String,
    action: DialogAction,
    min_kc: u8,
    syms_per: usize,
    kbd_syms: Vec<u32>,
    /// Blinking text-cursor state (toggled once per clock tick).
    cursor_on: bool,
    /// X11 cursor id created for this dialog (freed on close).
    cursor_id: u32,
}

pub struct BScreen {
    screen_num: ScreenNum,
    conn: X11Connection,
    root_window: u32,
    root_width: u16,
    root_height: u16,
    workspaces: Vec<Workspace>,
    current_workspace: u32,
    window_manager: WindowManager,
    struts: Strut,
    workarea: Rectangle,
    /// Connected monitors (from RandR CRTCs). Used so maximize/fullscreen and
    /// the published _NET_WORKAREA target a single monitor instead of the
    /// whole virtual screen on multi-head setups.
    monitors: Vec<Rectangle>,
    /// The monitor we treat as "primary" (contains 0,0, else the largest).
    primary_rect: Rectangle,
    name: String,
    focus_config: FocusConfig,
    toolbar: FbToolbar,
            slit: FbSlit,
            tray: FbTray,
    /// Channel to request StatusNotifierItem activation (left-click).
    sni_activator: Option<UnboundedSender<ActivateRequest>>,
    taskbar_order: Vec<WindowId>,
    hidden: std::collections::HashSet<WindowId>,
    /// Window ids of decoration frames we created ourselves. Used to ignore
    /// the `MapRequest`/`CreateNotify` our own (non-override-redirect) frames
    /// generate, so we never try to re-manage them.
    frame_ids: std::collections::HashSet<WindowId>,
    /// Pending graceful close requests (WM_DELETE_WINDOW with timeout).
    pending_closes: std::collections::HashMap<WindowId, PendingClose>,
    root_menu: Option<Menu>,
    menu_visible: bool,
    /// Current drag operation (move or resize).
    drag_state: DragState,
    key_bindings: Vec<KeyBinding>,
    /// Raw (modmask, key-name, action) list kept so we can re-resolve
    /// keycodes after a keyboard MappingNotify (layout switch).
    raw_bindings: Vec<(ModMask, String, KeyAction)>,
    dialog: Option<DialogState>,
    /// Shared flag to signal the event loop to stop.
    running: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

enum DragState {
    None,
    Moving { window_id: WindowId, off_x: i16, off_y: i16 },
    Resizing {
        window_id: WindowId,
        corner: u8,
        start_root_x: i16,
        start_root_y: i16,
        win_x: i16,
        win_y: i16,
        win_w: u16,
        win_h: u16,
    },
}

impl DragState {
    fn is_active(&self) -> bool {
        !matches!(self, DragState::None)
    }
}

impl BScreen {
    pub fn new(
        screen_num: ScreenNum,
        conn: X11Connection,
        name: &str,
        running: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Self, anyhow::Error> {
        let screen = conn.screen();
        let root = screen.root;
        let width = screen.width_in_pixels;
        let height = screen.height_in_pixels;
        trace_step("BScreen::new: start");

        let workspace_names: Vec<String> = (0..4).map(|i| format!("{}", i + 1)).collect();

        let toolbar = FbToolbar::new(
            &conn,
            width,
            height,
            workspace_names.clone(),
            ToolbarPlacement::Bottom,
        )?;
        trace_step("BScreen::new: toolbar ok");
        let slit = FbSlit::new(&conn, width, height, crate::slit::SlitPlacement::Right)?;
        trace_step("BScreen::new: slit ok");
        let mut tray = FbTray::new(&conn, width, height)?;
        tray.set_anchor(toolbar.tray_right_anchor());
        trace_step("BScreen::new: tray ok");

        let mut screen = Self {
            screen_num,
            conn,
            root_window: root,
            root_width: width,
            root_height: height,
            workspaces: Vec::new(),
            current_workspace: 0,
            hidden: std::collections::HashSet::new(),
            frame_ids: std::collections::HashSet::new(),
            pending_closes: std::collections::HashMap::new(),
            window_manager: WindowManager::new(),
            struts: Strut::zero(),
            workarea: Rectangle::new(0, 0, width, height),
            monitors: vec![Rectangle::new(0, 0, width, height)],
            primary_rect: Rectangle::new(0, 0, width, height),
            name: name.to_string(),
            focus_config: FocusConfig::default(),
            toolbar,
            slit,
            tray,
            sni_activator: None,
            taskbar_order: Vec::new(),
            root_menu: None,
            menu_visible: false,
            drag_state: DragState::None,
            key_bindings: Vec::new(),
            raw_bindings: Vec::new(),
            dialog: None,
            running,
        };

        // Give the root a visible cursor so the pointer is not invisible, and
        // paint a solid desktop background.
        if let Ok(cursor_font) = screen.conn.conn().generate_id() {
            if screen.conn.conn().open_font(cursor_font, b"cursor").is_ok() {
                if let Ok(cursor) = screen.conn.conn().generate_id() {
                    if screen
                        .conn
                        .conn()
                        .create_glyph_cursor(
                            cursor,
                            cursor_font,
                            cursor_font,
                            68,
                            69,
                            0,
                            0,
                            0,
                            0xffff,
                            0xffff,
                            0xffff,
                        )
                        .is_ok()
                    {
                        let _ = screen.conn.conn().change_window_attributes(
                            root,
                            &ChangeWindowAttributesAux::new().cursor(cursor),
                        );
                    }
                    let _ = screen.conn.conn().close_font(cursor_font);
                }
            }
        }
        // Original Rustbox desktop is a neutral gray rather than black. Paint
        // the bundled wallpaper if it loads; otherwise fall back to gray.
        let gray = match screen
            .conn
            .conn()
            .alloc_color(screen.conn.screen().default_colormap, 0x8080, 0x8080, 0x8080)
        {
            Ok(cookie) => cookie.reply().map(|r| r.pixel).unwrap_or(0x808080),
            Err(_) => 0x808080,
        };

        // Always paint a solid gray root first, so the desktop is visible even
        // if the wallpaper step fails for any reason.
        let _ = screen.conn.conn().change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().background_pixel(gray),
        );
        let _ = screen.conn.conn().clear_area(false, root, 0, 0, 0, 0);

        // Wallpaper is opt-in and fully isolated: compiled behind the
        // `wallpaper` feature, skipped at runtime when RUSTBOX_NO_WALLPAPER is
        // set, and any failure just keeps the gray background. It can never
        // prevent the WM from starting.
        #[cfg(feature = "wallpaper")]
        if std::env::var("RUSTBOX_NO_WALLPAPER").is_err() {
            let rdepth = screen.conn.screen().root_depth;
            let maxreq = screen.conn.conn().setup().maximum_request_length;
            trace_step(&format!(
                "wallpaper: tentando aplicar (root_depth={}, tela={}x{}, max_req={})",
                rdepth, width, height, maxreq
            ));
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let wallpaper_bytes = std::env::var("HOME").ok().and_then(|home| {
                    let png = format!("{}/.rustbox/wallpaper.png", home);
                    if std::path::Path::new(&png).exists() {
                        std::fs::read(png).ok()
                    } else {
                        None
                    }
                });

                let src: &[u8] = match &wallpaper_bytes {
                    Some(b) => b.as_slice(),
                    None => WALLPAPER_BYTES,
                };
                if let Ok(img) = Image::from_memory(src) {
                    if let Ok(scaled) = img.scale(width as u32, height as u32) {
                        match scaled.create_pixmap(screen.conn.conn(), screen.conn.screen(), root) {
                            Ok(pix) => {
                                let _ = screen.conn.conn().change_window_attributes(
                                    root,
                                    &ChangeWindowAttributesAux::new().background_pixmap(pix),
                                );
                                let _ = screen.conn.conn().clear_area(false, root, 0, 0, 0, 0);
                                trace_step("wallpaper: APLICADO COM SUCESSO");
                            }
                            Err(e) => {
                                trace_step(&format!("wallpaper: FALHA create_pixmap: {e}"));
                                log::warn!("wallpaper: falha ao criar pixmap: {e}; usando cinza");
                            }
                        }
                    } else {
                        trace_step("wallpaper: FALHA ao redimensionar imagem");
                        log::warn!("wallpaper: falha ao redimensionar imagem; usando cinza");
                    }
                } else {
                    trace_step("wallpaper: FALHA ao decodificar imagem (formato nao suportado?)");
                    log::warn!("wallpaper: falha ao decodificar imagem; usando cinza");
                }
            }));
        }
        trace_step("BScreen::new: background set");

        // Force the glibc memory allocator to release all freed pages (from the
        // large decoded wallpaper image and scale buffers) back to the OS.
        #[cfg(target_os = "linux")]
        unsafe {
            libc::malloc_trim(0);
        }

        for (i, name) in workspace_names.iter().enumerate() {
            let ws = Workspace::new(i as u32, name);
            screen.workspaces.push(ws);
        }

        // Load persisted workspace names (overrides the defaults above).
        screen.load_workspace_names();
        screen.toolbar.set_workspace_names(
            screen.workspaces.iter().map(|w| w.name().to_string()).collect(),
        );

        screen.recalc_struts();
        screen.query_monitors();
        trace_step("BScreen::new: query_monitors ok");
        screen.update_workarea();
        screen.toolbar.show(&screen.conn)?;
        screen.toolbar.render(&screen.conn)?;
        trace_step("BScreen::new: toolbar shown+rendered");

        // Publish EWMH properties so clients know the usable workarea.
        let _ = screen.publish_workarea();

        // Claim the system tray selection so tray-aware clients can dock.
        if let Err(e) = screen.tray.claim_selection(&screen.conn, 0) {
            log::warn!("Failed to claim _NET_SYSTEM_TRAY_S0: {}", e);
        }

        screen.conn.flush()?;
        trace_step("BScreen::new: flushed; scanning windows");

        screen.scan_windows()?;
        trace_step("BScreen::new: scan_windows ok");

        // Load and apply keybindings
        screen.init_keys()?;
        trace_step("BScreen::new: init_keys ok");

        Ok(screen)
    }

    /// Load keybindings from file (or defaults) and register X11 grabs.
    fn init_keys(&mut self) -> Result<(), anyhow::Error> {
        let config_path = {
            let home = std::env::var("HOME").unwrap_or_default();
            format!("{}/.rustbox/keys", home)
        };

        let raw = keys::load_keys_file(&config_path);
        let raw = if raw.is_empty() {
            keys::default_bindings()
        } else {
            raw
        };

        // Resolve key names to keycodes so we can match them in the handler
        self.key_bindings = keys::resolve_bindings(self.conn.conn(), &raw);

        // Register passive grabs on the root window for each binding
        for b in &self.key_bindings {
            let _ = self.conn.conn().grab_key(
                false, // owner_events: don't deliver to the client
                self.root_window,
                b.modmask,
                b.keycode,
                xproto::GrabMode::ASYNC,
                xproto::GrabMode::ASYNC,
            );
        }
        self.conn.flush()?;
        Ok(())
    }

    /// Re-resolve and re-grab key bindings after a keyboard `MappingNotify`
    /// (e.g. a layout/keymap switch that changes keycodes). Old grabs are
    /// released first so stale keycodes don't linger.
    fn refresh_keyboard_mapping(&mut self) -> Result<(), anyhow::Error> {
        let numlock = ModMask::M2;
        // Release previous passive grabs.
        for b in &self.key_bindings {
            let _ = self.conn.conn().ungrab_key(b.keycode, self.root_window, b.modmask);
            let _ = self.conn.conn().ungrab_key(
                b.keycode,
                self.root_window,
                keys::combine_modmask(b.modmask, numlock),
            );
        }
        // Re-resolve keycodes from the (possibly changed) keyboard mapping.
        self.key_bindings = keys::resolve_bindings(self.conn.conn(), &self.raw_bindings);
        for b in &self.key_bindings {
            let _ = self.conn.conn().grab_key(
                false,
                self.root_window,
                b.modmask,
                b.keycode,
                xproto::GrabMode::ASYNC,
                xproto::GrabMode::ASYNC,
            );
            let both = keys::combine_modmask(b.modmask, numlock);
            let _ = self.conn.conn().grab_key(
                false,
                self.root_window,
                both,
                b.keycode,
                xproto::GrabMode::ASYNC,
                xproto::GrabMode::ASYNC,
            );
        }
        self.conn.flush()?;
        log::info!("Reloaded keyboard mapping ({} bindings)", self.key_bindings.len());
        Ok(())
    }

    /// Adopt any pre-existing top-level windows so we manage them on startup.
    fn scan_windows(&mut self) -> Result<(), anyhow::Error> {
        let children = self.conn.conn().query_tree(self.root_window)?.reply()?.children;
        for child in children {
            let attrs = match self.conn.conn().get_window_attributes(child)?.reply() {
                Ok(a) => a,
                Err(_) => continue,
            };
            // Never manage unmapped windows.
            if attrs.map_state == xproto::MapState::UNMAPPED {
                continue;
            }
            // Dock apps (override-redirect applets or _NET_WM_WINDOW_TYPE_DOCK)
            // go to the slit instead of being framed.
            if attrs.override_redirect {
                if self.is_dock_app(child)? {
                    self.slit.add_window(&self.conn, child)?;
                }
                continue;
            }
            self.manage_window(child)?;
        }
        self.recalc_struts();
        Ok(())
    }

    /// Detect a dock applet: either it requests `_NET_WM_WINDOW_TYPE_DOCK`, or
    /// it is a small `override_redirect` window (classic wmaker/bbtools style).
    fn is_dock_app(&self, window: u32) -> Result<bool, anyhow::Error> {
        let prop = self.conn.atoms().get(Atom::NetWmWindowType);
        let dock = self.conn.atoms().get(Atom::NetWmWindowTypeDock);
        if prop != x11rb::NONE && dock != x11rb::NONE {
            if let Ok(reply) = self
                .conn
                .conn()
                .get_property(false, window, prop, 0u32, 0, 1024)?
                .reply()
            {
                let types: Vec<u32> = reply.value32().map(|it| it.collect()).unwrap_or_default();
                if types.contains(&dock) {
                    return Ok(true);
                }
            }
        }
        // Fallback: small override-redirect windows are treated as dockapps.
        if let Ok(attrs) = self.conn.conn().get_window_attributes(window)?.reply() {
            if attrs.override_redirect {
                if let Ok(g) = self.conn.conn().get_geometry(window)?.reply() {
                    if g.width <= 128 && g.height <= 128 {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    pub fn screen_num(&self) -> ScreenNum {
        self.screen_num
    }

    pub fn root_window(&self) -> u32 {
        self.root_window
    }

    pub fn width(&self) -> u16 {
        self.root_width
    }

    pub fn height(&self) -> u16 {
        self.root_height
    }

    pub fn workarea(&self) -> &Rectangle {
        &self.workarea
    }

    pub fn current_workspace(&self) -> u32 {
        self.current_workspace
    }

    pub fn set_current_workspace(&mut self, ws: u32) {
        if ws < self.workspaces.len() as u32 {
            log::info!("Switching to workspace {}", ws);
            self.current_workspace = ws;
            self.toolbar.set_current_workspace(ws);
            for window in self.window_manager.windows_mut() {
                let on_ws = window.workspace() == ws;
                if on_ws {
                    self.hidden.remove(&window.id());
                    if let Err(e) = window.show(&self.conn) {
                        log::error!("show window {} failed: {e}", window.id());
                    }
                } else {
                    self.hidden.insert(window.id());
                    if let Err(e) = window.hide(&self.conn) {
                        log::error!("hide window {} failed: {e}", window.id());
                    }
                }
            }
            self.update_toolbar();
            let _ = self.conn.flush();
            let _ = self.publish_workarea();
        }
    }

    /// Rebuild the screen strut from the toolbar and slit reservations so the
    /// workarea excludes the docked UI.
    fn recalc_struts(&mut self) {
        let mut s = Strut::zero();
        s.expand(&self.toolbar.strut());
        s.expand(&self.slit.strut());
        self.set_struts(s);
    }

    pub fn set_workspace_name(&mut self, idx: u32, name: &str) {
        if let Some(ws) = self.workspace_mut(idx) {
            ws.set_name(name);
        }
        self.toolbar.set_workspace_names(self.workspaces.iter().map(|w| w.name().to_string()).collect());
        self.toolbar.render(&self.conn).ok();
        let _ = self.conn.flush();
        let _ = self.save_workspace_names();
    }

    /// Path to the workspace configuration file.
    fn workspaces_config_path() -> String {
        if let Ok(home) = std::env::var("HOME") {
            format!("{}/.rustbox/workspaces.conf", home)
        } else {
            "/tmp/rustbox-workspaces.conf".to_string()
        }
    }

    /// Save workspace names to a file so they persist across WM restarts.
    fn save_workspace_names(&self) -> Result<(), anyhow::Error> {
        let path = Self::workspaces_config_path();
        let content = self
            .workspaces
            .iter()
            .map(|w| format!("{}:{}", w.id(), w.name()))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::create_dir_all(
            std::path::Path::new(&path).parent().unwrap_or(std::path::Path::new("/tmp")),
        )?;
        std::fs::write(&path, &content)?;
        Ok(())
    }

    /// Load workspace names from the config file. Only loads names for
    /// existing workspace IDs; ignores unknown IDs.
    fn load_workspace_names(&mut self) {
        let path = Self::workspaces_config_path();
        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines() {
                if let Some((id_str, name)) = line.split_once(':') {
                    if let Ok(id) = id_str.parse::<u32>() {
                        if let Some(ws) = self.workspace_mut(id) {
                            ws.set_name(name);
                        }
                    }
                }
            }
        }
    }

    pub fn workspace(&self, idx: u32) -> Option<&Workspace> {
        self.workspaces.get(idx as usize)
    }

    pub fn workspace_mut(&mut self, idx: u32) -> Option<&mut Workspace> {
        self.workspaces.get_mut(idx as usize)
    }

    pub fn workspaces(&self) -> &[Workspace] {
        &self.workspaces
    }

    pub fn workspace_count(&self) -> u32 {
        self.workspaces.len() as u32
    }

    pub fn window_manager(&self) -> &WindowManager {
        &self.window_manager
    }

    pub fn window_manager_mut(&mut self) -> &mut WindowManager {
        &mut self.window_manager
    }

    pub fn add_window(&mut self, window: RustboxWindow, workspace: u32) {
        let id = window.id();
        self.window_manager.add_window(window);
        if let Some(ws) = self.workspace_mut(workspace) {
            ws.add_window(id);
        }
    }

    pub fn remove_window(&mut self, id: WindowId) {
        self.window_manager.remove_window(id);
        for ws in &mut self.workspaces {
            ws.remove_window(id);
        }
    }

    /// Query connected monitors via RandR CRTCs and pick the "primary" one
    /// (the monitor containing 0,0, else the largest by area). On failure or
    /// no CRTCs we fall back to the whole root window, so single-monitor
    /// setups behave exactly as before.
    fn query_monitors(&mut self) {
        let resources = match self
            .conn
            .conn()
            .randr_get_screen_resources(self.root_window)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            Some(r) => r,
            None => {
                self.monitors = vec![Rectangle::new(0, 0, self.root_width, self.root_height)];
                self.primary_rect = Rectangle::new(0, 0, self.root_width, self.root_height);
                return;
            }
        };
        let mut rects = Vec::new();
        for crtc in resources.crtcs {
            if let Some(info) = self
                .conn
                .conn()
                .randr_get_crtc_info(crtc, 0u32)
                .ok()
                .and_then(|c| c.reply().ok())
            {
                if info.width > 0 && info.height > 0 {
                    rects.push(Rectangle::new(info.x as i16, info.y as i16, info.width, info.height));
                }
            }
        }
        if rects.is_empty() {
            rects.push(Rectangle::new(0, 0, self.root_width, self.root_height));
        }
        self.monitors = rects;
        self.primary_rect = self
            .monitors
            .iter()
            .copied()
            .find(|r| {
                r.x <= 0
                    && r.y <= 0
                    && (r.x as i32 + r.width as i32) > 0
                    && (r.y as i32 + r.height as i32) > 0
            })
            .or_else(|| {
                self.monitors
                    .iter()
                    .copied()
                    .max_by_key(|r| r.width as u32 * r.height as u32)
            })
            .unwrap_or_else(|| Rectangle::new(0, 0, self.root_width, self.root_height));
        log::info!(
            "Detected {} monitor(s), primary {}x{} at ({},{})",
            self.monitors.len(),
            self.primary_rect.width,
            self.primary_rect.height,
            self.primary_rect.x,
            self.primary_rect.y
        );
    }

    pub fn update_workarea(&mut self) {
        // Base the workarea on the primary monitor, then subtract reserved
        // struts. On a single-monitor setup primary_rect == the whole root.
        let x = self.primary_rect.x + self.struts.left as i16;
        let y = self.primary_rect.y + self.struts.top as i16;
        let w = (self.primary_rect.width as i32 - (self.struts.left + self.struts.right) as i32).max(1) as u16;
        let h = (self.primary_rect.height as i32 - (self.struts.top + self.struts.bottom) as i32).max(1) as u16;
        self.workarea = Rectangle::new(x, y, w, h);
    }

    /// Publish `_NET_WORKAREA` on the root window so EWMH-compliant clients
    /// (like kitty, alacritty, etc.) know the usable area and don't overlap
    /// the toolbar, slit, or tray.
    pub fn publish_workarea(&self) -> Result<(), anyhow::Error> {
        let root = self.root_window;
        let atoms = self.conn.atoms();
        let cardinal = atoms.get(Atom::Cardinal);

        // _NET_WORKAREA: x, y, width, height ( Cardinal, 32-bit per desktop)
        let wa = &self.workarea;
        let workarea_data = vec![
            wa.x as u32,
            wa.y as u32,
            wa.width as u32,
            wa.height as u32,
        ];

        if cardinal != x11rb::NONE {
            // Convert u32 values to little-endian bytes for change_property
            let mut data_bytes: Vec<u8> = Vec::new();
            for v in &workarea_data {
                data_bytes.extend_from_slice(&v.to_le_bytes());
            }
            let _ = self.conn.conn().change_property(
                xproto::PropMode::REPLACE,
                root,
                atoms.get(Atom::NetWorkarea),
                cardinal,
                32,
                workarea_data.len() as u32,
                &data_bytes,
            );
        }

        // _NET_NUMBER_OF_DESKTOPS
        let ndesktops = self.workspaces.len() as u32;
        if cardinal != x11rb::NONE {
            let _ = self.conn.conn().change_property(
                xproto::PropMode::REPLACE,
                root,
                atoms.get(Atom::NetNumberOfDesktops),
                cardinal,
                32,
                1,
                &ndesktops.to_le_bytes(),
            );
        }

        // _NET_CURRENT_DESKTOP
        if cardinal != x11rb::NONE {
            let _ = self.conn.conn().change_property(
                xproto::PropMode::REPLACE,
                root,
                atoms.get(Atom::NetCurrentDesktop),
                cardinal,
                32,
                1,
                &self.current_workspace.to_le_bytes(),
            );
        }

        Ok(())
    }

    pub fn set_struts(&mut self, struts: Strut) {
        self.struts = struts;
        self.update_workarea();
        let _ = self.publish_workarea();
    }

    /// Push the tray's current width into the toolbar so its window-list stops
    /// before the tray icons, and re-render the toolbar on top of that.
    fn sync_tray(&mut self) -> Result<(), anyhow::Error> {
        let w = self.tray.current_width();
        self.toolbar.set_tray_reserve(w);
        self.toolbar.render(&self.conn)?;
        self.tray.reposition(&self.conn)?;
        Ok(())
    }

    pub fn reconfigure(&mut self) -> Result<(), anyhow::Error> {
        // The cached screen info in x11rb is not updated by RandR, so read the
        // live root window geometry to discover the current size.
        let geo = self
            .conn
            .conn()
            .get_geometry(self.root_window)?
            .reply()
            .map_err(|e| anyhow::anyhow!("get_geometry root failed: {}", e))?;
        let new_w = geo.width;
        let new_h = geo.height;

        if new_w == self.root_width && new_h == self.root_height {
            return Ok(());
        }

        log::info!("Screen resized to {}x{}; reflowing UI", new_w, new_h);
        self.root_width = new_w;
        self.root_height = new_h;

        // Re-create the wallpaper at the new resolution so it fills the
        // screen instead of being cropped or mis-scaled.
        #[cfg(feature = "wallpaper")]
        self.re_apply_wallpaper(new_w, new_h);

        // Refit the dock UI to the new screen size.
        self.toolbar.reconfigure(&self.conn, new_w, new_h)?;
        self.slit.reconfigure(&self.conn, new_w, new_h)?;
        let anchor = self.toolbar.tray_right_anchor();
        self.tray.reconfigure(&self.conn, new_w, new_h, anchor)?;
        let w = self.tray.current_width();
        self.toolbar.set_tray_reserve(w);
        self.toolbar.render(&self.conn)?;

        // Recompute reserved space and the usable workarea.
        self.recalc_struts();
        self.query_monitors();
        self.update_workarea();
        let _ = self.publish_workarea();
        let wa = self.workarea;

        // Reflow every managed client into the new workarea.
        for win in self.window_manager.windows_mut() {
            if win.is_iconic() {
                continue;
            }
            if win.is_fullscreen() {
                win.set_fullscreen(
                    &self.conn,
                    true,
                    self.primary_rect.width,
                    self.primary_rect.height,
                )?;
            } else if win.is_maximized() {
                let mv = win.state().maximized_vert;
                let mh = win.state().maximized_horz;
                win.maximize(&self.conn, mv, mh, &wa)?;
            } else {
                let g = *win.geometry();
                let nx = g
                    .x
                    .clamp(wa.x, (wa.x + wa.width as i16).saturating_sub(g.width as i16));
                let ny = g
                    .y
                    .clamp(wa.y, (wa.y + wa.height as i16).saturating_sub(g.height as i16));
                win.move_resize(&self.conn, nx, ny, g.width, g.height)?;
            }
        }

        self.conn.flush()?;
        Ok(())
    }

    /// Re-create the wallpaper pixmap at a new screen size (called on
    /// resolution change). Any failure is silently ignored — the background
    /// simply keeps whatever it had before.
    #[cfg(feature = "wallpaper")]
    fn re_apply_wallpaper(&mut self, new_w: u16, new_h: u16) {
        if std::env::var("RUSTBOX_NO_WALLPAPER").is_ok() {
            return;
        }
        let root = self.root_window;
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let wallpaper_bytes = std::env::var("HOME").ok().and_then(|home| {
                let png = format!("{}/.rustbox/wallpaper.png", home);
                if std::path::Path::new(&png).exists() {
                    std::fs::read(png).ok()
                } else {
                    None
                }
            });

            let src: &[u8] = match &wallpaper_bytes {
                Some(b) => b.as_slice(),
                None => WALLPAPER_BYTES,
            };
            if let Ok(img) = Image::from_memory(src) {
                if let Ok(scaled) = img.scale(new_w as u32, new_h as u32) {
                    if let Ok(pix) =
                        scaled.create_pixmap(self.conn.conn(), self.conn.screen(), root)
                    {
                        let _ = self.conn.conn().change_window_attributes(
                            root,
                            &ChangeWindowAttributesAux::new().background_pixmap(pix),
                        );
                        let _ = self.conn.conn().clear_area(false, root, 0, 0, 0, 0);
                    }
                }
            }
        }));
        // Trim free heap memory after scaling the wallpaper on resize.
        #[cfg(target_os = "linux")]
        unsafe {
            libc::malloc_trim(0);
        }
    }

    /// Wire the SNI activation channel (called once the `SniManager` exists).
    pub fn set_sni_activator(&mut self, activator: UnboundedSender<ActivateRequest>) {
        self.sni_activator = Some(activator);
    }

    /// Dispatch a `SniEvent` to the tray (registration / removal / icon update).
    pub fn sni_event(&mut self, ev: SniEvent) -> Result<(), anyhow::Error> {
        match ev {
            SniEvent::Registered { service } => self.tray.sni_add(&self.conn, &service)?,
            SniEvent::Unregistered { service } => self.tray.sni_remove(&self.conn, &service)?,
            SniEvent::Updated {
                service,
                width,
                height,
                argb,
            } => self.tray.sni_update(&self.conn, &service, width, height, &argb)?,
        }
        self.conn.conn().flush()?;
        Ok(())
    }

    /// Tear down the toolbar and slit and reparent dock apps back to root so
    /// they survive a clean WM shutdown.
    pub fn destroy(&mut self) -> Result<(), anyhow::Error> {
        self.toolbar.destroy(&self.conn)?;
        self.slit.destroy(&self.conn)?;
        self.tray.destroy(&self.conn)?;
        self.conn.flush()?;
        Ok(())
    }

    /// Reparent a top-level client into a decorated frame and start tracking it.
    fn manage_window(&mut self, window: u32) -> Result<(), anyhow::Error> {
        if self.window_manager.get_window(window).is_some() {
            return Ok(());
        }

        // Ignore override-redirect clients (tooltips, menus, our own frames).
        if let Ok(attrs) = self.conn.conn().get_window_attributes(window)?.reply() {
            if attrs.override_redirect {
                return Ok(());
            }
        }

        let geom = self.conn.conn().get_geometry(window)?.reply()?;
        log::debug!(
            "manage: req geom x={} y={} w={} h={} border={}",
            geom.x, geom.y, geom.width, geom.height, geom.border_width
        );

        let name = self.read_wm_name(window);
        let mut client = WinClient::new(window, self.root_window);
        client.set_name(name.clone());
        let mut frame = FbWinFrame::new(
            &self.conn,
            self.root_window,
            window,
            geom.width,
            geom.height,
        )?;
        frame.set_label(name);
        frame.move_to(&self.conn, geom.x, geom.y)?;
        frame.show(&self.conn)?;
        frame.draw_titlebar(&self.conn)?;
        // Track this frame so we ignore the MapRequest it generates (frames are
        // no longer override-redirect; see FbWinFrame::new).
        self.frame_ids.insert(frame.frame_window());
        // A MapRequest is only a request: the WM must actually map the client.
        // Without this the client stays unmapped and its content is invisible.
        self.conn.conn().map_window(window)?;

        let fbwin = RustboxWindow::new(
            &self.conn,
            client,
            frame,
            self.current_workspace,
            self.screen_num as u32,
        );
        let mut fbwin = fbwin;
        fbwin.set_geometry(crate::core::Rectangle::new(geom.x, geom.y, geom.width, geom.height));
        fbwin.update_normal_hints(&self.conn);
        // Some apps (e.g. kitty with a saved session) (re)open already
        // maximized. The WM has no "before" geometry in that case, so flag it
        // as maximized on manage and let maximize() stash a sane restore point.
        // This prevents the window from being stuck maximized forever.
        if RustboxWindow::covers_workarea(*fbwin.geometry(), &self.workarea) {
            let _ = fbwin.maximize(&self.conn, true, true, &self.workarea);
        }
        self.add_window(fbwin, self.current_workspace);

        self.update_toolbar();
        let _ = self.focus_window(window);

        // Receive structure + property events for the client itself so we learn
        // about its unmap/destroy/title changes.
        self.conn.conn().change_window_attributes(
            window,
            &ChangeWindowAttributesAux::new().event_mask(
                EventMask::STRUCTURE_NOTIFY
                    | EventMask::SUBSTRUCTURE_NOTIFY
                    | EventMask::PROPERTY_CHANGE
                    | EventMask::ENTER_WINDOW,
            ),
        )?;

        // Passive grab: intercept ButtonPress on the client before the client
        // consumes it, so we can change focus.
        for btn in [xproto::ButtonIndex::M1, xproto::ButtonIndex::M2, xproto::ButtonIndex::M3] {
            let _ = self.conn.conn().grab_button(
                false,                                  // owner_events
                window,                                 // grab_window
                EventMask::BUTTON_PRESS,                // event_mask
                xproto::GrabMode::SYNC,                 // pointer_mode
                xproto::GrabMode::ASYNC,                // keyboard_mode
                NONE,                                   // confine_to
                NONE,                                   // cursor
                btn,                                    // button
                ModMask::ANY,                           // modifiers
            );
        }

        self.conn.flush()?;
        // Diagnostics: confirm the client is actually reparented, mapped and
        // positioned inside the frame.
        if let Ok(cg) = self.conn.conn().get_geometry(window)?.reply() {
            if let Ok(ca) = self.conn.conn().get_window_attributes(window)?.reply() {
                log::debug!(
                    "manage: client now parent-relative x={} y={} w={} h={} map_state={:?}",
                    cg.x, cg.y, cg.width, cg.height, ca.map_state
                );
            }
        }
        log::debug!("Managed window {}", window);
        let _ = self.toolbar.raise(&self.conn);
        Ok(())
    }

    fn unmanage_window(&mut self, window: u32, reparent: bool) -> Result<(), anyhow::Error> {
        if let Some(fbwin) = self.window_manager.get_window_mut(window) {
            fbwin.destroy(&self.conn)?;
        }
        if reparent {
            // Hand the client back to the root so it survives our shutdown and
            // can be re-managed later (e.g. on uniconify by the app).
            let _ = self
                .conn
                .conn()
                .reparent_window(window, self.root_window, 0, 0);
        }
        if let Some(w) = self.window_manager.get_window(window) {
            self.frame_ids.remove(&w.frame().frame_window());
        }
        self.pending_closes.remove(&window);
        self.remove_window(window);
        // If we just dropped the active window, pick a new one to focus.
        if self.window_manager.active_window() == Some(window) {
            let next = self.window_manager.windows().next().map(|w| w.id());
            if let Some(id) = next {
                let _ = self.focus_window(id);
            }
        }
        self.update_toolbar();
        log::debug!(
            "Unmanaged window {} — remaining windows: {}",
            window,
            self.window_manager.windows().count()
        );
        // Repaint the root background where the window lived. Destroying the
        // frame exposes the root, and without this the area stays black
        // instead of showing the gray/wallpaper background.
        let _ = self.conn.conn().clear_area(false, self.root_window, 0, 0, 0, 0);
        Ok(())
    }

    /// Read the best available window name (`_NET_WM_NAME`, then `WM_NAME`).
    fn read_wm_name(&self, window: u32) -> String {
        let atoms = self.conn.atoms();
        for atom in [atoms.get(Atom::NetWmName), atoms.get(Atom::WmName)] {
            if atom == x11rb::NONE {
                continue;
            }
            if let Ok(cookie) = self
                .conn
                .conn()
                .get_property(false, window, atom, 0u32, 0, 1024)
            {
                if let Ok(reply) = cookie.reply() {
                    if let Ok(s) = String::from_utf8(reply.value) {
                        let trimmed = s.trim_end_matches('\0').to_string();
                        if !trimmed.is_empty() {
                            return trimmed;
                        }
                    }
                }
            }
        }
        String::new()
    }

    /// Give keyboard focus to a managed client and reflect it in the frames.
    fn focus_window(&mut self, window: u32) -> Result<(), anyhow::Error> {
        if self.window_manager.get_window(window).is_none() {
            return Ok(());
        }
        self.window_manager.set_active_window(window);
        for w in self.window_manager.windows_mut() {
            let focused = w.id() == window;
            w.frame_mut().set_focused(focused, &self.conn)?;
        }
        // Raise the focused window's frame to the top of the stacking order,
        // then re-raise the toolbar so it stays above all managed windows.
        if let Some(w) = self.window_manager.get_window(window) {
            let _ = w.frame().raise(&self.conn);
        }
        let _ = self.toolbar.raise(&self.conn);
        // ICCCM-compliant focus handoff to the client.
        self.set_client_focus(window);
        self.update_toolbar();
        self.conn.flush()?;
        Ok(())
    }

    /// ICCCM-compliant focus handoff to a client window.
    ///
    /// * If the client supports `WM_TAKE_FOCUS`, send it a `WM_TAKE_FOCUS`
    ///   ClientMessage so it can set the input focus itself.
    /// * Otherwise (or in addition, when the client's `WM_HINTS.input` is true
    ///   or absent) call `XSetInputFocus` ourselves. Clients that are
    ///   "Globally Active" (`input == false` and `WM_TAKE_FOCUS` support) are
    ///   left to set focus on their own — this is exactly what makes Java/Swing
    ///   and other toolkits accept keyboard input under minimal WMs.
    fn set_client_focus(&self, window: u32) {
        let atoms = self.conn.atoms();
        let supports_take_focus =
            self.client_supports_protocol(window, atoms.get(Atom::WmTakeFocus));
        let input_hint = self.read_input_hint(window);

        if supports_take_focus {
            self.send_take_focus(window);
        }

        // Set the input focus ourselves unless the client is "Globally Active"
        // (input == false) and relies solely on WM_TAKE_FOCUS to grab focus.
        if input_hint != Some(false) {
            self.conn
                .conn()
                .set_input_focus(xproto::InputFocus::PARENT, window, 0u32)
                .log_if_err("set_input_focus");
        }
    }

    /// Whether `window` lists `protocol` in its `WM_PROTOCOLS` property.
    fn client_supports_protocol(&self, window: u32, protocol: u32) -> bool {
        if protocol == x11rb::NONE {
            return false;
        }
        let protocols_atom = self.conn.atoms().get(Atom::WmProtocols);
        if protocols_atom == x11rb::NONE {
            return false;
        }
        let reply = match self
            .conn
            .conn()
            .get_property(false, window, protocols_atom, xproto::AtomEnum::ATOM, 0, u32::MAX)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            Some(r) => r,
            None => return false,
        };
        reply
            .value32()
            .map(|it| it.into_iter().any(|a| a == protocol))
            .unwrap_or(false)
    }

    /// Read the `input` field of the client's `WM_HINTS` (None if absent).
    fn read_input_hint(&self, window: u32) -> Option<bool> {
        use x11rb::properties::WmHints;
        match WmHints::get(self.conn.conn(), window)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            // The getter may return None when the property is absent.
            Some(Some(h)) => h.input,
            _ => None,
        }
    }

    /// Send a `WM_TAKE_FOCUS` ClientMessage to `window`.
    fn send_take_focus(&self, window: u32) {
        let atoms = self.conn.atoms();
        let wm_protocols = atoms.get(Atom::WmProtocols);
        let wm_take_focus = atoms.get(Atom::WmTakeFocus);
        if wm_protocols == x11rb::NONE || wm_take_focus == x11rb::NONE {
            return;
        }
        let data = xproto::ClientMessageData::from([wm_take_focus, 0, 0, 0, 0]);
        let ev = xproto::ClientMessageEvent::new(32, window, wm_protocols, data);
        self.conn
            .conn()
            .send_event(false, window, xproto::EventMask::NO_EVENT, &ev)
            .log_if_err("send_event WM_TAKE_FOCUS");
    }

    /// Ask a client to close. If it supports `WM_DELETE_WINDOW` we send that
    /// request and wait (up to a grace period) for it to unmap/destroy itself;
    /// only then do we `kill_client` as a guaranteed fallback. Apps without
    /// `WM_DELETE_WINDOW` support are killed immediately, since there is no
    /// graceful path. This avoids destroying `WM_DELETE_WINDOW`-aware apps
    /// (editors, IDEs, ...) before they can save their state.
    fn close_window(&mut self, window: u32) -> Result<(), anyhow::Error> {
        if self.window_manager.get_window(window).is_none() {
            return Ok(());
        }
        let atoms = self.conn.atoms();
        if self.client_supports_protocol(window, atoms.get(Atom::WmDeleteWindow)) {
            let wm_protocols = atoms.get(Atom::WmProtocols);
            let wm_delete = atoms.get(Atom::WmDeleteWindow);
            let data = xproto::ClientMessageData::from([wm_delete, 0, 0, 0, 0]);
            let ev = xproto::ClientMessageEvent::new(32, window, wm_protocols, data);
            let _ = self
                .conn
                .conn()
                .send_event(false, window, xproto::EventMask::NO_EVENT, &ev)
                .log_if_err("send_event WM_DELETE_WINDOW");
            self.pending_closes.insert(
                window,
                PendingClose {
                    window,
                    sent_at: std::time::Instant::now(),
                    timeout: std::time::Duration::from_secs(5),
                },
            );
        } else {
            let _ = self.conn.conn().kill_client(window).log_if_err("kill_client");
        }
        self.conn.flush()?;
        Ok(())
    }

    /// Forcibly kill clients that ignored our `WM_DELETE_WINDOW` request past
    /// the grace period. Called from the event loop's periodic tick.
    pub(crate) fn check_pending_closes(&mut self) -> Result<(), anyhow::Error> {
        let now = std::time::Instant::now();
        let expired: Vec<WindowId> = self
            .pending_closes
            .iter()
            .filter(|(_, pc)| now.saturating_duration_since(pc.sent_at) >= pc.timeout)
            .map(|(w, _)| *w)
            .collect();
        if expired.is_empty() {
            return Ok(());
        }
        for w in expired {
            log::warn!("WM_DELETE_WINDOW ignorado por {}, matando cliente", w);
            let _ = self.conn.conn().kill_client(w).log_if_err("kill_client (timeout)");
            self.pending_closes.remove(&w);
        }
        self.conn.flush()?;
        Ok(())
    }

    /// Rebuild the toolbar taskbar from the current window list.
    fn update_toolbar(&mut self) {
        let active = self.window_manager.active_window();
        let current_ws = self.current_workspace;
        let mut items = Vec::new();
        let mut order = Vec::new();
        for w in self.window_manager.windows() {
            if w.workspace() != current_ws {
                continue;
            }
            let name = if !w.client().name().is_empty() {
                w.client().name().to_string()
            } else {
                format!("Window {}", w.id())
            };
            let name = if w.is_iconic() {
                format!("[{}] ", name)
            } else {
                name
            };
            items.push((name, Some(w.id()) == active));
            order.push(w.id());
        }
        self.taskbar_order = order;
        self.toolbar.set_window_items(items);
        let _ = self.toolbar.render(&self.conn);
        let _ = self.conn.flush();
    }

    /// Redraw the toolbar (used by the periodic clock tick). Cheaper than
    /// update_toolbar because it does not rebuild the window list.
    pub fn toolbar_render(&mut self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        self.toolbar.refresh_battery();
        self.toolbar.render(conn)?;
        conn.flush()?;
        Ok(())
    }

    fn configure_request(&mut self, window: u32, mask: u16, x: i16, y: i16, w: u16, h: u16, bw: u16, sm: xproto::StackMode) -> Result<(), anyhow::Error> {
        const CF_X: u16 = 1 << 0;
        const CF_Y: u16 = 1 << 1;
        const CF_WIDTH: u16 = 1 << 2;
        const CF_HEIGHT: u16 = 1 << 3;
        const CF_BORDER: u16 = 1 << 4;
        const CF_STACK: u16 = 1 << 6;

        let mut aux = ConfigureWindowAux::new();
        log::debug!(
            "configure_request: win={} mask={:#x} x={} y={} w={} h={} bw={}",
            window, mask, x, y, w, h, bw
        );
        if mask & CF_X != 0 {
            aux = aux.x(x as i32);
        }
        if mask & CF_Y != 0 {
            aux = aux.y(y as i32);
        }
        if mask & CF_WIDTH != 0 {
            aux = aux.width(w as u32);
        }
        if mask & CF_HEIGHT != 0 {
            aux = aux.height(h as u32);
        }
        if mask & CF_BORDER != 0 {
            aux = aux.border_width(bw as u32);
        }
        if mask & CF_STACK != 0 {
            aux = aux.stack_mode(sm);
        }

        // A managed client lives inside our frame; its requested geometry is
        // relative to the frame, so we apply it directly to the client.
        self.conn.conn().configure_window(window, &aux)?;

        // When the client requests a new size, resize the frame to match.
        if (mask & (CF_WIDTH | CF_HEIGHT)) != 0 {
            if let Some(rw) = self.window_manager.get_window_mut(window) {
                rw.frame_mut().resize(&self.conn, w, h)?;
            }
        }
        Ok(())
    }

    pub fn handle_event(&mut self, event: &Event) -> Result<(), anyhow::Error> {
        let etype = match event {
            Event::KeyPress(_) => "KeyPress",
            Event::KeyRelease(_) => "KeyRelease",
            Event::ButtonPress(_) => "ButtonPress",
            Event::ButtonRelease(_) => "ButtonRelease",
            Event::MotionNotify(_) => "MotionNotify",
            Event::Expose(_) => "Expose",
            Event::MapRequest(_) => "MapRequest",
            Event::UnmapNotify(_) => "UnmapNotify",
            Event::DestroyNotify(_) => "DestroyNotify",
            Event::ConfigureRequest(_) => "ConfigureRequest",
            Event::EnterNotify(_) => "EnterNotify",
            Event::LeaveNotify(_) => "LeaveNotify",
            Event::PropertyNotify(_) => "PropertyNotify",
            Event::ClientMessage(_) => "ClientMessage",
            Event::MapNotify(_) => "MapNotify",
            _ => "Other",
        };
        log::debug!(
            "handle_event: type={} dialog_active={}",
            etype,
            self.dialog.is_some()
        );
        // If dialog is active, intercept events for it first.
        if self.dialog.is_some() {
            match event {
                Event::KeyPress(e) => {
                    return self.handle_dialog_key(e);
                }
                Event::Expose(e) => {
                    let win = self.dialog.as_ref().unwrap().window;
                    if e.window == win {
                        return self.render_dialog();
                    }
                }
                Event::ButtonPress(e) => {
                    let (win, frame) = {
                        let dlg = self.dialog.as_ref().unwrap();
                        (dlg.window, dlg.frame)
                    };
                    let frame_children: Vec<u32> = self.window_manager
                        .get_window(win)
                        .map(|w| {
                            let f = w.frame();
                            vec![f.frame_window, f.title_window, f.handle_window]
                        })
                        .unwrap_or_default();
                    if e.event == win
                        || e.event == frame
                        || frame_children.contains(&e.event)
                    {
                        // Click inside the dialog — keep it open and re-focus.
                        let _ = self.conn.conn().set_input_focus(
                            xproto::InputFocus::PARENT,
                            win,
                            0u32,
                        );
                        let _ = self.conn.conn().allow_events(Allow::REPLAY_POINTER, 0u32);
                        // Don't return — let the event fall through to the
                        // normal handler so titlebar buttons (close, iconify)
                        // and move/resize still work.
                    } else {
                        // Click outside the dialog — ignore (don't dismiss).
                        let _ = self.conn.conn().allow_events(Allow::REPLAY_POINTER, 0u32);
                        return Ok(());
                    }
                }
                _ => {}
            }
        }

        // If any menu is visible, route motion and button events to it first.
        if self.menu_visible {
            match event {
                Event::MotionNotify(e) => {
                    if self.route_menu_motion(e.root_x, e.root_y)? {
                        return Ok(());
                    }
                }
                Event::ButtonPress(e) => {
                    if self.route_menu_click(e.root_x, e.root_y)? {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }

        match event {
            Event::RandRScreenChangeNotify(_) | Event::RandRNotify(_) => {
                // Screen size/configuration changed (e.g. termux-x11 or Xephyr
                // was resized). Reflow the toolbar, slit, tray and all managed
                // windows to the new geometry.
                self.reconfigure()?;
            }
            Event::MappingNotify(e) => {
                // Keyboard mapping changed (layout/keymap switch). Refresh our
                // passive key grabs so bindings keep working. Only full keyboard
                // remaps (request == 1) affect keycodes; others (modifier/keyboard
                // mapping changes) still warrant a refresh.
                if e.request == xproto::Mapping::KEYBOARD {
                    self.refresh_keyboard_mapping()?;
                }
            }
            Event::MapRequest(e) => {
                // Ignore MapRequests from our own decoration frames.
                if self.frame_ids.contains(&e.window) {
                    return Ok(());
                }
                // Only adopt direct children of the root that we don't own.
                if e.parent == self.root_window {
                    if self.is_dock_app(e.window)? {
                        self.slit.add_window(&self.conn, e.window)?;
                        self.recalc_struts();
                    } else {
                        self.manage_window(e.window)?;
                    }
                }
            }
            Event::CreateNotify(e) => {
                // Catch dockapps that appear after startup (they are usually
                // override-redirect and so never generate a MapRequest).
                if e.parent == self.root_window && e.override_redirect {
                    if self.is_dock_app(e.window)? {
                        self.slit.add_window(&self.conn, e.window)?;
                        self.recalc_struts();
                    }
                }
            }
            Event::ConfigureRequest(e) => {
                self.configure_request(
                    e.window,
                    u16::from(e.value_mask),
                    e.x,
                    e.y,
                    e.width,
                    e.height,
                    e.border_width,
                    e.stack_mode,
                )?;
            }
            Event::UnmapNotify(e) => {
                if self.hidden.contains(&e.window) {
                    return Ok(());
                }
                // Window was iconified — we intentionally unmapped it
                if let Some(w) = self.window_manager.get_window(e.window) {
                    if w.is_iconic() {
                        return Ok(());
                    }
                }
                // Never treat our own dialog window's (reparent) unmap as a
                // close — it is managed and closed explicitly via close_dialog.
                if self.dialog.as_ref().map(|d| d.window) == Some(e.window) {
                    return Ok(());
                }
                if self.tray.owns_window(e.window) {
                    if e.window != self.tray.window_id() {
                        self.tray.undock_window(&self.conn, e.window)?;
                        self.sync_tray()?;
                    }
                } else if self.slit.owns_window(e.window) {
                    if e.window != self.slit.window_id() {
                        self.slit.remove_window(&self.conn, e.window)?;
                        self.recalc_struts();
                    }
                } else if self.window_manager.get_window(e.window).is_some() {
                    self.unmanage_window(e.window, true)?;
                }
            }
            Event::DestroyNotify(e) => {
                self.hidden.remove(&e.window);
                if self.tray.owns_window(e.window) {
                    if e.window != self.tray.window_id() {
                        self.tray.undock_window(&self.conn, e.window)?;
                        self.sync_tray()?;
                    }
                } else if self.slit.owns_window(e.window) {
                    if e.window != self.slit.window_id() {
                        self.slit.remove_window(&self.conn, e.window)?;
                        self.recalc_struts();
                    }
                } else if self.window_manager.get_window(e.window).is_some() {
                    self.unmanage_window(e.window, false)?;
                }
            }
            Event::PropertyNotify(e) => {
                let atoms = self.conn.atoms();
                let is_name =
                    e.atom == atoms.get(Atom::WmName) || e.atom == atoms.get(Atom::NetWmName);
                if is_name {
                    let name = self.read_wm_name(e.window);
                    if name.is_empty() {
                        return Ok(());
                    }
                    if let Some(fbwin) = self.window_manager.get_window_mut(e.window) {
                        fbwin.client_mut().set_name(name.clone());
                        fbwin.frame_mut().set_label(name);
                        let _ = fbwin.frame_mut().draw_titlebar(&self.conn);
                    }
                    self.update_toolbar();
                    self.conn.flush()?;
                } else if e.atom == atoms.get(Atom::WmNormalHints) {
                    // Client changed its size hints; refresh our cache so the
                    // next resize honours the new min/max/increments.
                    if let Some(fbwin) = self.window_manager.get_window_mut(e.window) {
                        fbwin.update_normal_hints(&self.conn);
                    }
                }
            }
            Event::Expose(e) => {
                if e.window == self.toolbar.window_id() {
                    self.toolbar.handle_expose(&self.conn)?;
                } else if self.slit.owns_window(e.window) {
                    self.slit.handle_expose(&self.conn)?;
                } else if self.tray.owns_window(e.window) {
                    if e.window == self.tray.popup_window_id() {
                        self.tray.handle_popup_expose(&self.conn)?;
                    } else {
                        self.tray.handle_expose(&self.conn)?;
                    }
                } else if self.menu_visible {
                    // Check if the expose is for any open menu window.
                    if let Some(menu) = &self.root_menu {
                        if menu.find_menu_by_window(e.window).is_some() {
                            menu.render(&self.conn)?;
                            self.conn.flush()?;
                        }
                    }
                } else {
                    let win_id = self.window_manager.windows()
                        .find(|w| {
                            e.window == w.frame().title_window()
                                || e.window == w.frame().frame_window()
                        })
                        .map(|w| w.id());
                    if let Some(id) = win_id {
                        if let Some(w) = self.window_manager.get_window_mut(id) {
                            w.redraw_title(&self.conn);
                        }
                    }
                }
            }
            Event::ButtonPress(e) => {
                let mod_alt = ModMask::M1;

                // Dismiss the tray overflow popup when clicking anywhere outside
                // the tray/popup (unless the click is on the chevron itself,
                // handled below).
                if self.tray.popup_open() && e.event != self.tray.window_id()
                    && e.event != self.tray.popup_window_id()
                {
                    self.tray.maybe_close_popup(&self.conn, e.event)?;
                }

                if e.event == self.root_window && e.detail == 3 {
                    self.show_root_menu(e.root_x, e.root_y)?;
                } else if e.event == self.tray.window_id() {
                    if self.tray.handle_button_press(e.event_x, e.event_y) {
                        self.tray.toggle_popup(&self.conn)?;
                    } else if let Some(service) = self.tray.sni_slot_at(e.event_x, e.event_y) {
                        if let Some(tx) = &self.sni_activator {
                            let _ = tx.unbounded_send(ActivateRequest {
                                service,
                                x: e.root_x as i32,
                                y: e.root_y as i32,
                            });
                        }
                    }
                } else if e.event == self.toolbar.window_id() {
                    match self.toolbar.handle_button_press(e.event_x, e.event_y) {
                        ToolbarAction::Workspace(i) => { self.set_current_workspace(i as u32); }
                        ToolbarAction::Window(i) => {
                            if let Some(&wid) = self.taskbar_order.get(i) {
                                let active = self.window_manager.active_window();
                                if let Some(w) = self.window_manager.get_window(wid) {
                                    if w.is_iconic() {
                                        let _ = w;
                                        if let Some(w_mut) = self.window_manager.get_window_mut(wid) {
                                            let _ = w_mut.deiconify(&self.conn);
                                        }
                                        if let Some(w) = self.window_manager.get_window(wid) {
                                            let _ = w.frame().raise(&self.conn);
                                        }
                                        self.update_toolbar();
                                    } else if active == Some(wid) {
                                        let _ = w;
                                        if let Some(w_mut) = self.window_manager.get_window_mut(wid) {
                                            let _ = w_mut.iconify(&self.conn);
                                        }
                                        self.update_toolbar();
                                    }
                                }
                                let _ = self.focus_window(wid);
                            }
                        }
                        ToolbarAction::None => {}
                    }
                } else {
                    let mut target: Option<WindowId> = None;
                    for w in self.window_manager.windows() {
                        if e.event == w.id()
                            || e.event == w.frame().frame_window()
                            || e.event == w.frame().title_window()
                            || e.event == w.frame().handle_window()
                        {
                            target = Some(w.id());
                            break;
                        }
                    }

                    let win_id = match target {
                        Some(id) => id,
                        None => return Ok(()),
                    };

                    let is_title = self.window_manager.windows()
                        .find(|w| w.id() == win_id)
                        .map(|w| e.event == w.frame().title_window())
                        .unwrap_or(false);

                    let is_handle = self.window_manager.windows()
                        .find(|w| w.id() == win_id)
                        .map(|w| e.event == w.frame().handle_window())
                        .unwrap_or(false);

                    let mod_alt_down = u16::from(e.state) & u16::from(mod_alt) != 0;

                    // Click on the resize handle at the bottom of the frame
                    // starts a resize (left button, no modifier needed).
                    if is_handle && e.detail == 1 {
                        // Convert handle_window-local coords to frame-relative.
                        let handle_id = self.window_manager.windows()
                            .find(|w| w.id() == win_id)
                            .map(|w| w.frame().handle_window());
                        let (local_x, local_y) = if let Some(hw) = handle_id {
                            if let Ok(geo) = self.conn.conn().get_geometry(hw) {
                                if let Ok(reply) = geo.reply() {
                                    (e.event_x + reply.x, e.event_y + reply.y)
                                } else {
                                    (e.event_x, e.event_y)
                                }
                            } else {
                                (e.event_x, e.event_y)
                            }
                        } else {
                            (e.event_x, e.event_y)
                        };
                        self.focus_window(win_id)?;
                        self.start_resize(win_id, e.root_x, e.root_y, local_x, local_y)?;
                        return Ok(());
                    }

                    if is_title {
                        if let Some(w) = self.window_manager.get_window_mut(win_id) {
                            if let Some(btn) = w.frame().hit_test_button(e.event_x, e.event_y) {
                                match btn {
                                    crate::window::frame::ButtonType::Close => {
                                        let _ = self.close_window(win_id);
                                        return Ok(());
                                    }
                                    crate::window::frame::ButtonType::Maximize => {
                                        if w.is_fullscreen() {
                                            let _ = w.set_fullscreen(&self.conn, false, self.root_width, self.root_height);
                                        } else if w.is_maximized() || RustboxWindow::covers_workarea(*w.geometry(), &self.workarea) {
                                            let _ = w.unmaximize(&self.conn, &self.workarea);
                                        } else {
                                            let _ = w.maximize(&self.conn, true, true, &self.workarea);
                                        }
                                        let _ = w.redraw_title(&self.conn);
                                        let _ = self.toolbar.raise(&self.conn);
                                        self.conn.flush()?;
                                        return Ok(());
                                    }
                                    crate::window::frame::ButtonType::Iconify => {
                                        let _ = w.iconify(&self.conn);
                                        self.update_toolbar();
                                        self.conn.flush()?;
                                        return Ok(());
                                    }
                                    _ => {}
                                }
                            }
                        }

                        // Click+drag on the titlebar moves the window
                        // (no modifier needed — standard WM behavior).
                        if e.detail == 1 {
                            self.focus_window(win_id)?;
                            self.start_move(win_id, e.root_x, e.root_y)?;
                            return Ok(());
                        }

                        self.focus_window(win_id)?;
                    } else {
                        if mod_alt_down && e.detail == 3 {
                            self.start_resize(win_id, e.root_x, e.root_y, e.event_x, e.event_y)?;
                            return Ok(());
                        }

                        self.focus_window(win_id)?;
                    }
                }
                // Replay any frozen pointer event from the passive button grab
                // (click-to-focus on client content) so the client receives it.
                let _ = self.conn.conn().allow_events(Allow::REPLAY_POINTER, 0u32);
            }
            Event::ButtonRelease(e) => {
                if self.drag_state.is_active() {
                    self.end_drag(e.root_x, e.root_y)?;
                }
            }
            Event::MotionNotify(e) => {
                if self.drag_state.is_active() {
                    self.handle_drag(e.root_x, e.root_y)?;
                    return Ok(());
                }
            }
            Event::KeyPress(e) => {
                let action = keys::match_key(&self.key_bindings, u16::from(e.state), e.detail)
                    .cloned();
                if let Some(action) = action {
                    self.dispatch_key_action(&action)?;
                }
            }
            Event::ClientMessage(e) => {
                log::debug!("ClientMessage: type={:#x} window={:#x}", e.type_, e.window);
                let opcode_atom = self.conn.atoms().get(Atom::NetSystemTrayOpcode);
                log::debug!("  opcode_atom={:#x} match={}", opcode_atom, e.type_ == opcode_atom);
                if opcode_atom != x11rb::NONE && e.type_ == opcode_atom {
                    let timestamp = e.data.as_data32()[0];
                    let opcode = e.data.as_data32()[1];
                    let client = e.data.as_data32()[2];
                    let _ = self.tray.handle_opcode(
                        &self.conn, opcode, client, timestamp,
                    );
                    let _ = self.sync_tray();
                }
            }
            _ => {}
        }
        Ok(())
    }

    // ───────────────────────────────────────────────
    //  Root menu
    // ───────────────────────────────────────────────

    /// Show the root menu at the given screen coordinates.
    fn show_root_menu(&mut self, x: i16, y: i16) -> Result<(), anyhow::Error> {
        // Rebuild the menu every time so workspace list stays up-to-date.
        let mut items = Vec::new();

        // Workspace navigation submenu.
        for i in 0..self.workspaces.len() {
            let ws = &self.workspaces[i];
            let label = if ws.name().parse::<u32>().is_ok() {
                format!("Workspace {}", ws.name())
            } else {
                format!("{}", ws.name())
            };
            items.push(MenuItem::new(&label, MenuItemType::Workspace(i as u32)));
        }

        items.push(MenuItem::separator());
        items.push(MenuItem::new("Add Workspace", MenuItemType::WorkspaceCreate));
        let ws_label = format!("Rename Workspace {}", self.current_workspace + 1);
        items.push(MenuItem::new(&ws_label, MenuItemType::WorkspaceRename(self.current_workspace)));

        items.push(MenuItem::separator());
        items.push(MenuItem::new("Run Command...", MenuItemType::RunDialog));
        items.push(MenuItem::new("kitty terminal", MenuItemType::Exec("kitty".to_string())));
        items.push(MenuItem::new("Firefox", MenuItemType::Exec("firefox".to_string())));
        items.push(MenuItem::separator());
        items.push(MenuItem::new("Reload", MenuItemType::Reconfig));
        items.push(MenuItem::new("Restart", MenuItemType::Restart));
        items.push(MenuItem::new("Exit", MenuItemType::Exit));

        // Destroy old menu if it exists
        if let Some(old) = self.root_menu.take() {
            let _ = old.destroy(&self.conn);
        }
        self.root_menu = Some(Menu::new(
            &self.conn,
            "Rustbox",
            items,
            self.root_width,
            self.root_height,
        )?);

        if let Some(menu) = &mut self.root_menu {
            menu.position_and_show(&self.conn, x, y)?;
            self.menu_visible = true;
            self.conn.flush()?;
        }
        Ok(())
    }

    /// Route a motion event to open menus. Returns `true` if consumed.
    fn route_menu_motion(&mut self, x: i16, y: i16) -> Result<bool, anyhow::Error> {
        let conn = self.conn.clone();
        if let Some(menu) = &mut self.root_menu {
            // Find the deepest submenu by walking the chain.
            let deepest = menu.deepest_submenu();
            let local_x = x - deepest.screen_x();
            let local_y = y - deepest.screen_y();
            let inside = local_x >= 0 && local_y >= 0
                && local_x < deepest.width() as i16
                && local_y < deepest.height() as i16;

            if inside {
                menu.handle_motion(&conn, local_x, local_y)?;
                return Ok(true);
            }

            let local_x = x - menu.screen_x();
            let local_y = y - menu.screen_y();
            if local_x >= 0
                && local_y >= 0
                && local_x < menu.width() as i16
                && local_y < menu.height() as i16
            {
                menu.handle_motion(&conn, local_x, local_y)?;
                return Ok(true);
            }
        }

        // Not inside any menu — close all.
        self.close_menus()?;
        Ok(true)
    }

    /// Route a button click to open menus. Returns `true` if consumed.
    fn route_menu_click(&mut self, x: i16, y: i16) -> Result<bool, anyhow::Error> {
        let conn = self.conn.clone();
        if let Some(menu) = &mut self.root_menu {
            // Check deepest submenu first.
            // For now, just check root menu (submenu routing TBD).
            let local_x = x - menu.screen_x();
            let local_y = y - menu.screen_y();
            if local_x >= 0
                && local_y >= 0
                && local_x < menu.width() as i16
                && local_y < menu.height() as i16
            {
                if let Some(action) = menu.handle_click(&conn, local_x, local_y)? {
                    self.execute_menu_action(action)?;
                }
                return Ok(true);
            }
        }

        // Click outside all menus — close.
        self.close_menus()?;
        Ok(true)
    }

    /// Execute a menu action and close menus.
    fn execute_menu_action(&mut self, action: MenuItemType) -> Result<(), anyhow::Error> {
        match action {
            MenuItemType::Exec(cmd) => {
                self.close_menus()?;
                log::info!("Menu exec: {}", cmd);
                let mut cmd_proc = std::process::Command::new("sh");
                cmd_proc
                    .args(["-c", &cmd])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                cmd_proc.env("DISPLAY", self.conn.display_name());
                let _ = cmd_proc.spawn();
            }
            MenuItemType::Exit => {
                self.close_menus()?;
                log::info!("Exit via menu");
                self.running.store(false, std::sync::atomic::Ordering::Relaxed);
            }
            MenuItemType::Restart => {
                self.close_menus()?;
                log::info!("Restart via menu");
                // TODO: signal Rustbox to restart
            }
            MenuItemType::Reconfig => {
                self.close_menus()?;
                log::info!("Reconfig via menu");
                // TODO: reload config
            }
            MenuItemType::Submenu(..) => {
                log::info!("Submenu click — opening not yet implemented");
            }
            MenuItemType::Workspace(n) => {
                self.close_menus()?;
                log::info!("Switch to workspace {}", n);
                self.set_current_workspace(n);
            }
            MenuItemType::WorkspaceCreate => {
                self.close_menus()?;
                log::info!("Create new workspace");
                let ws_id = self.workspaces.len() as u32;
                let name = format!("{}", ws_id + 1);
                self.workspaces.push(Workspace::new(ws_id, &name));
                self.save_workspace_names()?;
                self.set_current_workspace(ws_id);
            }
            MenuItemType::WorkspaceRename(ws_idx) => {
                self.close_menus()?;
                self.show_rename_dialog(ws_idx)?;
            }
            MenuItemType::RunDialog => {
                self.close_menus()?;
                self.show_run_dialog()?;
            }
            _ => {}
        }
        Ok(())
    }

    fn create_dialog(
        &mut self,
        title: String,
        initial_text: String,
        action: DialogAction,
    ) -> Result<(), anyhow::Error> {
        const W: u16 = 320;
        const H: u16 = 40;

        let screen = self.conn.screen();
        let root = self.conn.root_window();
        let fg = crate::hooks::or(&crate::hooks::DIALOG_FG, screen.black_pixel);
        let bg = crate::hooks::or(&crate::hooks::DIALOG_BG, screen.white_pixel);

        let cx = (screen.width_in_pixels as i16 - W as i16) / 2;
        let cy = (screen.height_in_pixels as i16 - H as i16) / 2;

        let window = self.conn.conn().generate_id()?;
        self.conn.conn().create_window(
            0, window, root, cx, cy, W, H, 0,
            xproto::WindowClass::INPUT_OUTPUT, 0,
            &xproto::CreateWindowAux::new()
                .background_pixel(bg)
                .event_mask(EventMask::EXPOSURE | EventMask::KEY_PRESS | EventMask::BUTTON_PRESS),
        )?;

        let dlg_atom = self.conn.atoms().get(Atom::RustboxDialog);
        if dlg_atom != x11rb::NONE {
            let _ = self.conn.conn().change_property(
                xproto::PropMode::REPLACE,
                window,
                dlg_atom,
                self.conn.atoms().get(Atom::Cardinal),
                32,
                1,
                &[1u8, 0, 0, 0],
            );
        }

        let wm_name_atom = self.conn.atoms().get(Atom::WmName);
        if wm_name_atom != x11rb::NONE {
            let bytes = title.as_bytes();
            let _ = self.conn.conn().change_property(
                xproto::PropMode::REPLACE,
                window,
                wm_name_atom,
                self.conn.atoms().get(Atom::Utf8String),
                8,
                bytes.len() as u32,
                bytes,
            );
        }

        let gc = self.conn.conn().generate_id()?;
        self.conn.conn().create_gc(gc, window, &xproto::CreateGCAux::new().foreground(fg))?;

        let hints = x11rb::properties::WmHints {
            input: Some(true),
            ..Default::default()
        };
        let _ = hints.set(self.conn.conn(), window)?;

        let wm_class_atom = self.conn.atoms().get(Atom::WmClass);
        let _ = self.conn.conn().change_property(
            xproto::PropMode::REPLACE,
            window,
            wm_class_atom,
            self.conn.atoms().get(Atom::Utf8String),
            8,
            b"rustbox-dialog\0Rustbox\0".len() as u32,
            b"rustbox-dialog\0Rustbox\0",
        );

        let cursor_font = self.conn.conn().generate_id()?;
        let _ = self.conn.conn().open_font(cursor_font, b"cursor");
        let cursor = self.conn.conn().generate_id()?;
        let _ = self.conn.conn().create_glyph_cursor(
            cursor, cursor_font, cursor_font,
            152, 153,
            0, 0, 0, 0xffff, 0xffff, 0xffff,
        );
        // The glyph cursor copies the font's glyphs, so the font can be freed now.
        let _ = self.conn.conn().close_font(cursor_font);
        let _ = self.conn.conn().change_window_attributes(
            window,
            &ChangeWindowAttributesAux::new().cursor(cursor),
        );

        self.manage_window(window)?;

        // Remove maximize button — unnecessary for a dialog.
        if let Some(fbwin) = self.window_manager.get_window_mut(window) {
            fbwin.frame_mut().no_maximize = true;
            fbwin.frame_mut().layout_buttons();
            let _ = fbwin.redraw_title(&self.conn);
        }

        let _ = self.conn.conn().change_window_attributes(
            window,
            &ChangeWindowAttributesAux::new().event_mask(
                EventMask::EXPOSURE | EventMask::KEY_PRESS | EventMask::BUTTON_PRESS,
            ),
        );

        let frame = self
            .window_manager
            .get_window(window)
            .map(|w| w.frame().frame_window)
            .unwrap_or(window);

        let setup = self.conn.conn().setup();
        let min_kc = setup.min_keycode;
        let max_kc = setup.max_keycode;
        let (kbd_syms, syms_per) = self.conn.conn()
            .get_keyboard_mapping(min_kc, max_kc - min_kc + 1)?
            .reply()
            .map(|r| (r.keysyms, r.keysyms_per_keycode as usize))
            .unwrap_or_default();

        self.dialog = Some(DialogState {
            window,
            frame,
            gc,
            title,
            text: initial_text,
            action,
            min_kc,
            syms_per,
            kbd_syms,
            cursor_on: true,
            cursor_id: cursor,
        });

        self.render_dialog()?;

        if let Ok(reply) = self.conn.conn().get_input_focus()?.reply() {
            log::debug!(
                "DIAG focus após create_dialog: atual={:#x} esperado={:#x}",
                reply.focus, window
            );
        }

        self.conn.flush()?;
        Ok(())
    }

    fn show_rename_dialog(&mut self, ws_idx: u32) -> Result<(), anyhow::Error> {
        let ws_idx_val = ws_idx as usize;
        let current = if ws_idx_val < self.workspaces.len() {
            self.workspaces[ws_idx_val].name().to_string()
        } else {
            String::new()
        };
        self.create_dialog(
            format!("Rename Workspace {}", ws_idx + 1),
            current,
            DialogAction::RenameWorkspace(ws_idx),
        )
    }

    fn show_run_dialog(&mut self) -> Result<(), anyhow::Error> {
        self.create_dialog(
            "Run Command".to_string(),
            String::new(),
            DialogAction::RunCommand,
        )
    }

    fn handle_dialog_key(&mut self, e: &xproto::KeyPressEvent) -> Result<(), anyhow::Error> {
        let dlg = match self.dialog.as_mut() {
            Some(d) => d,
            None => return Ok(()),
        };

        let idx = (e.detail.saturating_sub(dlg.min_kc)) as usize * dlg.syms_per;
        let keysym = dlg.kbd_syms.get(idx).copied().unwrap_or(0);
        let shift = u16::from(e.state) & u16::from(xproto::ModMask::SHIFT) != 0;
        let keysym = if shift {
            dlg.kbd_syms.get(idx + 1).copied().unwrap_or(keysym)
        } else {
            keysym
        };

        // Re-assert input focus so the dialog window keeps receiving key events
        // (termux-x11 may lose focus unexpectedly).
        let _ = self.conn.conn().set_input_focus(
            xproto::InputFocus::PARENT,
            dlg.window,
            0u32,
        );

        match keysym {
            0xff0d | 0xff8d => {
                let result = std::mem::take(&mut dlg.text);
                let action = dlg.action;
                match action {
                    DialogAction::RenameWorkspace(ws_idx) => {
                        if !result.is_empty() {
                            self.set_workspace_name(ws_idx, &result);
                        }
                    }
                    DialogAction::RunCommand => {
                        self.close_dialog()?;
                        if !result.is_empty() {
                            log::info!("Run command from dialog: {}", result);
                            let mut cmd_proc = std::process::Command::new("sh");
                            cmd_proc
                                .args(["-c", &result])
                                .stdin(std::process::Stdio::null())
                                .stdout(std::process::Stdio::null())
                                .stderr(std::process::Stdio::null());
                            cmd_proc.env("DISPLAY", self.conn.display_name());
                            let _ = cmd_proc.spawn();
                        }
                        return Ok(());
                    }
                }
                self.close_dialog()?;
            }
            0xff1b => {
                self.close_dialog()?;
            }
            0xff08 | 0x007f => {
                dlg.text.pop();
                self.render_dialog()?;
            }
            ks if ks >= 0x0020 && ks <= 0x007e => {
                dlg.text.push(char::from_u32(ks).unwrap_or('?'));
                self.render_dialog()?;
            }
            _ => {}
        }
        Ok(())
    }

    fn close_dialog(&mut self) -> Result<(), anyhow::Error> {
        if let Some(dlg) = self.dialog.take() {
            let _ = self.conn.conn().free_gc(dlg.gc);
            let _ = self.conn.conn().free_cursor(dlg.cursor_id);

            // Unmanage the dialog — this destroys its frame + client window.
            let _ = self.unmanage_window(dlg.window, false);
            self.conn.flush()?;
        }
        Ok(())
    }

    fn render_dialog(&self) -> Result<(), anyhow::Error> {
        const W: u16 = 320;
        const H: u16 = 40;
        const INSET: i16 = 4;

        let dlg = match self.dialog.as_ref() {
            Some(d) => d,
            None => return Ok(()),
        };
        let screen = self.conn.screen();
        let fg = crate::hooks::or(&crate::hooks::DIALOG_FG, screen.black_pixel);
        let bg = crate::hooks::or(&crate::hooks::DIALOG_BG, screen.white_pixel);
        let font = Font::new("fixed");
        let window = dlg.window;
        let gc = dlg.gc;

        // The prompt is shown in the frame's titlebar; the client area only
        // holds the edit field.
        self.conn.conn().change_gc(gc, &xproto::ChangeGCAux::new().foreground(bg))?;
        self.conn.conn().poly_fill_rectangle(window, gc, &[xproto::Rectangle { x: 0, y: 0, width: W, height: H }])?;

        // Edit text (dark on white), vertically centred.
        let et_y = H as i16 / 2 + font.height() as i16 / 2 - font.descent() as i16;
        font.draw_text_on_bg(self.conn.conn(), window, gc, INSET, et_y, &dlg.text, fg, bg)?;

        // Blinking text cursor: a thin vertical bar at the end of the text so
        // the user can see where typing will go.
        if dlg.cursor_on {
            let tw = font.text_width(self.conn.conn(), &dlg.text)? as i16;
            let cur_x = INSET + tw;
            log::debug!("render_dialog cursor_on=true cur_x={} text_len={}", cur_x, dlg.text.chars().count());
            self.conn.conn().poly_fill_rectangle(
                window,
                gc,
                &[xproto::Rectangle {
                    x: cur_x,
                    y: 6,
                    width: 2,
                    height: H - 12,
                }],
            )?;
        }

        // Re-assert focus and stacking (termux-x11 sometimes loses them).
        let _ = self.conn.conn().set_input_focus(
            xproto::InputFocus::PARENT,
            window,
            0u32,
        );
        let _ = self.conn.conn().configure_window(
            window,
            &ConfigureWindowAux::new().stack_mode(xproto::StackMode::ABOVE),
        );

        Ok(())
    }

    /// Toggle the dialog's text-cursor blink state and redraw. Driven by the
    /// periodic clock tick in the event loop so the cursor animates even when
    /// no X events arrive.
    pub(crate) fn blink_dialog_cursor(&mut self) {
        if let Some(dlg) = self.dialog.as_mut() {
            dlg.cursor_on = !dlg.cursor_on;
            log::debug!("blink_dialog_cursor called, now cursor_on={}", dlg.cursor_on);
            let _ = self.render_dialog();
        }
    }

    /// Hide (unmap) the root menu and any open submenus.
    // ───────────────────────────────────────────────
    //  Drag (move / resize)
    // ───────────────────────────────────────────────

    /// Grab the pointer for a move/resize and only enter the drag state if the
    /// grab actually succeeded. Without checking `GrabStatus`, a failed grab
    /// (e.g. `AlreadyGrabbed`) would leave `drag_state` set while no pointer
    /// events reach us — the window would then follow the mouse forever.
    fn try_grab_pointer_for_drag(&mut self, state: DragState) -> Result<(), anyhow::Error> {
        let reply = self
            .conn
            .conn()
            .grab_pointer(
                false,
                self.root_window,
                EventMask::BUTTON_RELEASE | EventMask::BUTTON_MOTION,
                xproto::GrabMode::ASYNC,
                xproto::GrabMode::ASYNC,
                NONE,
                NONE,
                0u32,
            )?
            .reply()?;
        if reply.status == xproto::GrabStatus::SUCCESS {
            self.drag_state = state;
        } else {
            log::warn!(
                "grab_pointer falhou no drag (status={:?}); cancelando",
                reply.status
            );
        }
        Ok(())
    }

    fn start_move(&mut self, win_id: WindowId, root_x: i16, root_y: i16) -> Result<(), anyhow::Error> {
        if let Some(w) = self.window_manager.get_window(win_id) {
            let off_x = root_x - w.geometry().x;
            let off_y = root_y - w.geometry().y;
            self.try_grab_pointer_for_drag(DragState::Moving { window_id: win_id, off_x, off_y })?;
        }
        Ok(())
    }

    fn start_resize(
        &mut self, win_id: WindowId,
        root_x: i16, root_y: i16,
        local_x: i16, local_y: i16,
    ) -> Result<(), anyhow::Error> {
        if let Some(w) = self.window_manager.get_window(win_id) {
            let bw = w.frame().border_width() as i16;
            let fw = w.geometry().width as i16 + bw * 2;
            let fh = w.geometry().height as i16 + w.frame().title_height() as i16 + bw * 2;
            let corner: u8 = if local_x < fw / 3 { 1 } else if local_x > fw * 2 / 3 { 2 } else { 0 }
                | if local_y < fh / 3 { 4 } else if local_y > fh * 2 / 3 { 8 } else { 0 };
            let g = *w.geometry();
            self.try_grab_pointer_for_drag(DragState::Resizing {
                window_id: win_id,
                corner,
                start_root_x: root_x,
                start_root_y: root_y,
                win_x: g.x,
                win_y: g.y,
                win_w: g.width,
                win_h: g.height,
            })?;
        }
        Ok(())
    }

    fn handle_drag(&mut self, root_x: i16, root_y: i16) -> Result<(), anyhow::Error> {
        match self.drag_state {
            DragState::Moving { window_id, off_x, off_y } => {
                if let Some(w) = self.window_manager.get_window_mut(window_id) {
                    let mut new_x = root_x - off_x;
                    let mut new_y = root_y - off_y;
                    // The taskbar (and other struts) take priority: a normal
                    // window must not be dragged over them. Fullscreen windows
                    // are exempt so they can cover the whole screen.
                    if !w.state().fullscreen {
                        let g = *w.geometry();
                        let wa = self.workarea;
                        new_x = if g.width as i16 >= wa.width as i16 {
                            wa.x
                        } else {
                            new_x.clamp(wa.x, wa.x + wa.width as i16 - g.width as i16)
                        };
                        new_y = if g.height as i16 >= wa.height as i16 {
                            wa.y
                        } else {
                            new_y.clamp(wa.y, wa.y + wa.height as i16 - g.height as i16)
                        };
                    }
                    w.frame_mut().move_to(&self.conn, new_x, new_y)?;
                }
            }
            DragState::Resizing { window_id, corner, start_root_x, start_root_y, win_x, win_y, win_w, win_h } => {
                // Delta from the mouse position at grab time (root coords).
                // Each tick re-computes from the ORIGINAL base geometry so
                // the deltas never compound — matches upstream Fluxbox logic.
                let dx = root_x - start_root_x;
                let dy = root_y - start_root_y;

                // Size constraints from the client's WM_NORMAL_HINTS.
                let mut min_w = 50i16;
                let mut min_h = 30i16;
                let mut max_w = i16::MAX;
                let mut max_h = i16::MAX;
                let mut width_inc: u16 = 1;
                let mut height_inc: u16 = 1;
                let mut base_w: u16 = 0;
                let mut base_h: u16 = 0;
                if let Some(win) = self.window_manager.get_window(window_id) {
                    let h = win.normal_hints();
                    min_w = h.min_width.max(1) as i16;
                    min_h = h.min_height.max(1) as i16;
                    max_w = (h.max_width as i32).min(i16::MAX as i32) as i16;
                    max_h = (h.max_height as i32).min(i16::MAX as i32) as i16;
                    width_inc = h.width_inc.max(1);
                    height_inc = h.height_inc.max(1);
                    base_w = h.base_width;
                    base_h = h.base_height;
                }

                let w = win_w as i16;
                let h = win_h as i16;

                let mut new_x = win_x;
                let mut new_y = win_y;
                let mut new_w = w;
                let mut new_h = h;

                if corner & 1 != 0 {
                    // left edge: width shrinks, x shifts right
                    new_w = (w - dx).max(min_w);
                    new_x = win_x + (w - new_w);
                } else if corner & 2 != 0 {
                    // right edge: width grows, x unchanged
                    new_w = (w + dx).max(min_w);
                }
                if corner & 4 != 0 {
                    // top edge: height shrinks, y shifts down
                    new_h = (h - dy).max(min_h);
                    new_y = win_y + (h - new_h);
                } else if corner & 8 != 0 {
                    // bottom edge: height grows, y unchanged
                    new_h = (h + dy).max(min_h);
                }

                let (mut new_w, mut new_h) = (new_w as u16, new_h as u16);
                if let Some(w) = self.window_manager.get_window_mut(window_id) {
                    // The taskbar (and other struts) take priority: a normal
                    // window must not be resized over them. We cap the size to
                    // the workarea while keeping the *fixed* (opposite) edge
                    // anchored. Fullscreen windows are exempt.
                    if !w.state().fullscreen {
                        let wa = self.workarea;
                        let right = win_x + win_w as i16;
                        let bottom = win_y + win_h as i16;
                        if corner & 1 != 0 {
                            // left edge dragged: right edge fixed
                            new_w = ((new_w as i16)
                                .min(right - wa.x)
                                .max(min_w)
                                .min(max_w)) as u16;
                            new_x = right - new_w as i16;
                        } else if corner & 2 != 0 {
                            // right edge dragged: left edge fixed
                            new_w = ((new_w as i16)
                                .min((wa.x + wa.width as i16) - win_x)
                                .max(min_w)
                                .min(max_w)) as u16;
                        }
                        if corner & 4 != 0 {
                            // top edge dragged: bottom edge fixed
                            new_h = ((new_h as i16)
                                .min(bottom - wa.y)
                                .max(min_h)
                                .min(max_h)) as u16;
                            new_y = bottom - new_h as i16;
                        } else if corner & 8 != 0 {
                            // bottom edge dragged: top edge fixed
                            new_h = ((new_h as i16)
                                .min((wa.y + wa.height as i16) - win_y)
                                .max(min_h)
                                .min(max_h)) as u16;
                        }
                    }
                    // Snap to the client's resize increments and clamp to
                    // min/max size (WM_NORMAL_HINTS).
                    new_w = base_w + ((new_w.saturating_sub(base_w)) / width_inc) * width_inc;
                    new_h = base_h + ((new_h.saturating_sub(base_h)) / height_inc) * height_inc;
                    new_w = new_w.clamp(min_w as u16, max_w as u16);
                    new_h = new_h.clamp(min_h as u16, max_h as u16);
                    // Re-anchor the fixed edges after snapping.
                    if corner & 1 != 0 {
                        new_x = (win_x + win_w as i16) - new_w as i16;
                    }
                    if corner & 4 != 0 {
                        new_y = (win_y + win_h as i16) - new_h as i16;
                    }
                    let _ = w.frame_mut().move_resize(&self.conn, new_x, new_y, new_w, new_h);
                    w.set_geometry(Rectangle::new(new_x, new_y, new_w, new_h));
                    let _ = w.reconfigure_client(&self.conn, new_w, new_h);
                }
            }
            DragState::None => {}
        }
        Ok(())
    }

    fn end_drag(&mut self, _root_x: i16, _root_y: i16) -> Result<(), anyhow::Error> {
        let g = std::mem::replace(&mut self.drag_state, DragState::None);
        let _ = self.conn.conn().ungrab_pointer(0u32);

        if let DragState::Moving { window_id, .. } = g {
            // Update stored geometry after move.
            if let Some(w) = self.window_manager.get_window_mut(window_id) {
                if let Ok(g) = self.conn.conn().get_geometry(w.frame().frame_window())?.reply() {
                    let mut geo = *w.geometry();
                    geo.x = g.x;
                    geo.y = g.y;
                    w.set_geometry(geo);
                }
            }
            self.conn.flush()?;
        }
        Ok(())
    }

    /// Execute a keybinding action.
    fn dispatch_key_action(&mut self, action: &KeyAction) -> Result<(), anyhow::Error> {
        match action {
            KeyAction::NextWindow => {
                if let Some(active) = self.window_manager.active_window() {
                    let ids: Vec<_> = self.window_manager.windows().map(|w| w.id()).collect();
                    if let Some(pos) = ids.iter().position(|&id| id == active) {
                        let next = ids.get((pos + 1) % ids.len()).copied();
                        if let Some(next) = next {
                            let _ = self.focus_window(next);
                        }
                    }
                }
            }
            KeyAction::PrevWindow => {
                if let Some(active) = self.window_manager.active_window() {
                    let ids: Vec<_> = self.window_manager.windows().map(|w| w.id()).collect();
                    if ids.is_empty() {
                        return Ok(());
                    }
                    if let Some(pos) = ids.iter().position(|&id| id == active) {
                        let prev = if pos == 0 { ids.len() - 1 } else { pos - 1 };
                        let _ = self.focus_window(ids[prev]);
                    }
                }
            }
            KeyAction::Close => {
                if let Some(active) = self.window_manager.active_window() {
                    let _ = self.close_window(active);
                }
            }
            KeyAction::Iconify => {
                if let Some(active) = self.window_manager.active_window() {
                    if let Some(w) = self.window_manager.get_window_mut(active) {
                        let _ = w.iconify(&self.conn);
                    }
                    self.update_toolbar();
                }
            }
            KeyAction::Maximize => {
                if let Some(active) = self.window_manager.active_window() {
                    if let Some(w) = self.window_manager.get_window_mut(active) {
                        if w.is_maximized() {
                            let _ = w.unmaximize(&self.conn, &self.workarea);
                        } else {
                            let _ = w.maximize(&self.conn, true, true, &self.workarea);
                        }
                        let _ = w.redraw_title(&self.conn);
                        let _ = self.toolbar.raise(&self.conn);
                    }
                }
            }
            KeyAction::GoToWorkspace(n) => {
                let idx = n.saturating_sub(1) as u32;
                if idx < self.workspaces.len() as u32 {
                    self.set_current_workspace(idx);
                }
            }
            KeyAction::GoToNextWorkspace => {
                let next = (self.current_workspace + 1) % self.workspaces.len() as u32;
                self.set_current_workspace(next);
            }
            KeyAction::GoToPrevWorkspace => {
                let prev = if self.current_workspace == 0 {
                    self.workspaces.len() as u32 - 1
                } else {
                    self.current_workspace - 1
                };
                self.set_current_workspace(prev);
            }
            KeyAction::ShowMenu => {
                // Show the root menu at the center of the screen (or under the
                // mouse pointer if we had it — use center as fallback).
                let cx = (self.root_width as i16) / 2;
                let cy = (self.root_height as i16) / 2;
                self.show_root_menu(cx, cy)?;
            }
            KeyAction::Exit => {
                log::info!("Exit via keybinding");
                self.running.store(false, std::sync::atomic::Ordering::Relaxed);
            }
            KeyAction::Restart => {
                log::info!("Restart via keybinding");
                // TODO: signal Rustbox to restart
            }
            KeyAction::Move => {
                if let Some(active) = self.window_manager.active_window() {
                    let _ = self.conn.conn().grab_pointer(
                        false, self.root_window,
                        EventMask::BUTTON_RELEASE | EventMask::BUTTON_MOTION,
                        xproto::GrabMode::ASYNC, xproto::GrabMode::ASYNC,
                        NONE, NONE, 0u32,
                    );
                    self.drag_state = DragState::Moving { window_id: active, off_x: 0, off_y: 0 };
                }
            }
            KeyAction::Resize => {
                if let Some(active) = self.window_manager.active_window() {
                    if let Some(w) = self.window_manager.get_window(active) {
                        let g = *w.geometry();
                        self.drag_state = DragState::Resizing {
                            window_id: active,
                            corner: 2 | 8, // right + bottom
                            start_root_x: 0,
                            start_root_y: 0,
                            win_x: g.x,
                            win_y: g.y,
                            win_w: g.width,
                            win_h: g.height,
                        };
                    }
                    let _ = self.conn.conn().grab_pointer(
                        false, self.root_window,
                        EventMask::BUTTON_RELEASE | EventMask::BUTTON_MOTION,
                        xproto::GrabMode::ASYNC, xproto::GrabMode::ASYNC,
                        NONE, NONE, 0u32,
                    );
                }
            }
            KeyAction::Exec(cmd) => {
                let mut cmd_proc = std::process::Command::new("sh");
                cmd_proc
                    .args(["-c", cmd])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                cmd_proc.env("DISPLAY", self.conn.display_name());
                let _ = cmd_proc.spawn();
            }
        }
        Ok(())
    }

    /// Hide (unmap) the root menu and any open submenus.
    fn close_menus(&mut self) -> Result<(), anyhow::Error> {
        self.menu_visible = false;
        let conn = self.conn.clone();
        if let Some(menu) = &mut self.root_menu {
            hide_submenus(&conn, menu)?;
            menu.hide(&conn)?;
        }
        self.conn.flush()?;
        Ok(())
    }
}

fn hide_submenus(conn: &X11Connection, menu: &Menu) -> Result<(), anyhow::Error> {
    if let Some(sub) = &menu.submenu() {
        hide_submenus(conn, sub)?;
        sub.hide(conn)?;
    }
    Ok(())
}
