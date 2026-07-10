use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    self, ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt as _, EventMask,
};

use crate::config::FocusConfig;
use crate::core::{Rectangle, Strut};
use crate::slit::FbSlit;
use crate::toolbar::{FbToolbar, ToolbarAction, ToolbarPlacement};
use crate::window::{FbWinFrame, FluxboxWindow, WinClient, WindowId, WindowManager};
use crate::workspace::Workspace;
use crate::x11::{Atom, Event, X11Connection};

pub type ScreenNum = usize;

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
    name: String,
    focus_config: FocusConfig,
    toolbar: FbToolbar,
    slit: FbSlit,
    taskbar_order: Vec<WindowId>,
    hidden: std::collections::HashSet<WindowId>,
}

impl BScreen {
    pub fn new(screen_num: ScreenNum, conn: X11Connection, name: &str) -> Result<Self, anyhow::Error> {
        let screen = conn.screen();
        let root = screen.root;
        let width = screen.width_in_pixels;
        let height = screen.height_in_pixels;

        let workspace_names: Vec<String> = (0..4).map(|i| format!("{}", i + 1)).collect();

        let toolbar = FbToolbar::new(
            &conn,
            width,
            height,
            workspace_names.clone(),
            ToolbarPlacement::Bottom,
        )?;
        let slit = FbSlit::new(&conn, width, height, crate::slit::SlitPlacement::Right)?;

        let mut screen = Self {
            screen_num,
            conn,
            root_window: root,
            root_width: width,
            root_height: height,
            workspaces: Vec::new(),
            current_workspace: 0,
            hidden: std::collections::HashSet::new(),
            window_manager: WindowManager::new(),
            struts: Strut::zero(),
            workarea: Rectangle::new(0, 0, width, height),
            name: name.to_string(),
            focus_config: FocusConfig::default(),
            toolbar,
            slit,
            taskbar_order: Vec::new(),
        };

        // Give the root a visible cursor so the pointer is not invisible, and
        // paint a solid desktop background.
        let black = screen.conn.screen().black_pixel;
        let white = screen.conn.screen().white_pixel;
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
        // Original Fluxbox desktop is a neutral gray rather than black.
        let gray = match screen
            .conn
            .conn()
            .alloc_color(screen.conn.screen().default_colormap, 0x8080, 0x8080, 0x8080)
        {
            Ok(cookie) => cookie.reply().map(|r| r.pixel).unwrap_or(0x808080),
            Err(_) => 0x808080,
        };
        let _ = screen.conn.conn().change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().background_pixel(gray),
        );
        let _ = screen.conn.conn().clear_area(false, root, 0, 0, 0, 0);

        for (i, name) in workspace_names.iter().enumerate() {
            let ws = Workspace::new(i as u32, name);
            screen.workspaces.push(ws);
        }

        screen.recalc_struts();
        screen.toolbar.show(&screen.conn)?;
        screen.toolbar.render(&screen.conn)?;
        screen.conn.flush()?;

        screen.scan_windows()?;
        Ok(screen)
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

    pub fn add_window(&mut self, window: FluxboxWindow, workspace: u32) {
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

    pub fn update_workarea(&mut self) {
        self.workarea = Rectangle::new(
            self.struts.left as i16,
            self.struts.top as i16,
            self.root_width.saturating_sub(self.struts.left + self.struts.right),
            self.root_height.saturating_sub(self.struts.top + self.struts.bottom),
        );
    }

    pub fn set_struts(&mut self, struts: Strut) {
        self.struts = struts;
        self.update_workarea();
    }

    pub fn reconfigure(&self, _conn: &X11Connection) -> Result<(), anyhow::Error> {
        Ok(())
    }

    /// Tear down the toolbar and slit and reparent dock apps back to root so
    /// they survive a clean WM shutdown.
    pub fn destroy(&mut self) -> Result<(), anyhow::Error> {
        self.toolbar.destroy(&self.conn)?;
        self.slit.destroy(&self.conn)?;
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
        // A MapRequest is only a request: the WM must actually map the client.
        // Without this the client stays unmapped and its content is invisible.
        self.conn.conn().map_window(window)?;

        let fbwin = FluxboxWindow::new(
            &self.conn,
            client,
            frame,
            self.current_workspace,
            self.screen_num as u32,
        );
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
        let _ = self
            .conn
            .conn()
            .set_input_focus(xproto::InputFocus::PARENT, window, 0u32);
        self.update_toolbar();
        self.conn.flush()?;
        Ok(())
    }

    /// Ask a client to close: send `WM_DELETE_WINDOW`, then `kill_client` as a
    /// guaranteed fallback. The resulting Unmap/Destroy triggers unmanage.
    fn close_window(&mut self, window: u32) -> Result<(), anyhow::Error> {
        if self.window_manager.get_window(window).is_none() {
            return Ok(());
        }
        let atoms = self.conn.atoms();
        let wm_protocols = atoms.get(Atom::WmProtocols);
        let wm_delete = atoms.get(Atom::WmDeleteWindow);
        if wm_protocols != x11rb::NONE && wm_delete != x11rb::NONE {
            let data = xproto::ClientMessageData::from([wm_delete, 0, 0, 0, 0]);
            let ev = xproto::ClientMessageEvent::new(32, window, wm_protocols, data);
            let _ = self
                .conn
                .conn()
                .send_event(false, window, xproto::EventMask::NO_EVENT, &ev);
        }
        let _ = self.conn.conn().kill_client(window);
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
        Ok(())
    }

    pub fn handle_event(&mut self, event: &Event) -> Result<(), anyhow::Error> {
        match event {
            Event::MapRequest(e) => {
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
                    // We unmapped this window ourselves for workspace hiding;
                    // do not treat it as a real client unmap.
                    return Ok(());
                }
                if self.slit.owns_window(e.window) {
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
                if self.slit.owns_window(e.window) {
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
                }
            }
            Event::Expose(e) => {
                if e.window == self.toolbar.window_id() {
                    self.toolbar.handle_expose(&self.conn)?;
                } else if self.slit.owns_window(e.window) {
                    self.slit.handle_expose(&self.conn)?;
                } else {
                    for w in self.window_manager.windows() {
                        if e.window == w.frame().title_window() {
                            w.redraw_title(&self.conn);
                            break;
                        }
                    }
                }
            }
            Event::ButtonPress(e) => {
                if e.event == self.toolbar.window_id() {
                    match self.toolbar.handle_button_press(e.event_x, e.event_y) {
                        ToolbarAction::Workspace(i) => {
                            self.set_current_workspace(i as u32);
                        }
                        ToolbarAction::Window(i) => {
                            if let Some(&wid) = self.taskbar_order.get(i) {
                                let _ = self.focus_window(wid);
                            }
                        }
                        ToolbarAction::None => {}
                    }
                } else {
                    // A press on a frame's title bar: close button, else focus.
                    let mut action: Option<(WindowId, bool)> = None;
                    for w in self.window_manager.windows() {
                        if e.event == w.frame().title_window() {
                            action = Some((w.id(), w.frame().is_close_press(e.event_x, e.event_y)));
                            break;
                        }
                    }
                    if let Some((id, close)) = action {
                        if close {
                            let _ = self.close_window(id);
                        } else {
                            let _ = self.focus_window(id);
                        }
                    }
                }
            }
            Event::KeyPress(e) => {
                log::debug!("KeyPress code={}", e.detail);
            }
            _ => {}
        }
        Ok(())
    }
}
