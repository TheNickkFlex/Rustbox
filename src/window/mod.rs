use std::collections::HashMap;

use x11rb::protocol::xproto::ConnectionExt;

use crate::core::{Layer, Rectangle};
use crate::x11::X11Connection;

pub mod client;
pub mod frame;
pub mod state;

pub use client::{WinClient, WindowType};
pub use frame::FbWinFrame;
pub use state::WindowState;

pub type WindowId = u32;

pub struct RustboxWindow {
    id: WindowId,
    client: WinClient,
    frame: FbWinFrame,
    state: WindowState,
    workspace: u32,
    layer: Layer,
    geometry: Rectangle,
    normal_hints: NormalHints,
}

pub struct NormalHints {
    pub min_width: u16,
    pub min_height: u16,
    pub max_width: u16,
    pub max_height: u16,
    pub width_inc: u16,
    pub height_inc: u16,
    pub base_width: u16,
    pub base_height: u16,
    pub min_aspect: f64,
    pub max_aspect: f64,
    pub gravity: u32,
}

impl Default for NormalHints {
    fn default() -> Self {
        Self {
            min_width: 1,
            min_height: 1,
            max_width: 32767,
            max_height: 32767,
            width_inc: 1,
            height_inc: 1,
            base_width: 0,
            base_height: 0,
            min_aspect: 0.0,
            max_aspect: 0.0,
            gravity: 0,
        }
    }
}

impl RustboxWindow {
    pub fn new(
        _conn: &X11Connection,
        client: WinClient,
        frame: FbWinFrame,
        workspace: u32,
        _screen: u32,
    ) -> Self {
        Self {
            id: client.window(),
            client,
            frame,
            state: WindowState::new(),
            workspace,
            layer: Layer::NORMAL,
            geometry: Rectangle::zero(),
            normal_hints: NormalHints::default(),
        }
    }

    pub fn id(&self) -> WindowId {
        self.id
    }

    pub fn client(&self) -> &WinClient {
        &self.client
    }

    pub fn client_mut(&mut self) -> &mut WinClient {
        &mut self.client
    }

    pub fn frame(&self) -> &FbWinFrame {
        &self.frame
    }

    /// Repaint the title bar (used in response to Expose events).
    pub fn redraw_title(&mut self, conn: &X11Connection) {
        let _ = self.frame.draw_titlebar(conn);
    }

    /// Make the window (frame + client) visible. Used when its workspace is
    /// switched to.
    pub fn show(&mut self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        self.frame.show(conn)?;
        conn.conn().map_window(self.client.window())?;
        Ok(())
    }

    /// Hide the window (frame + client). Used to keep windows on other
    /// workspaces from being visible. The window stays managed.
    pub fn hide(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        conn.conn().unmap_window(self.client.window())?;
        self.frame.hide(conn)?;
        Ok(())
    }

    pub fn frame_mut(&mut self) -> &mut FbWinFrame {
        &mut self.frame
    }

    pub fn state(&self) -> &WindowState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut WindowState {
        &mut self.state
    }

    pub fn geometry(&self) -> &Rectangle {
        &self.geometry
    }

    pub fn set_geometry(&mut self, geom: Rectangle) {
        self.geometry = geom;
    }

    pub fn normal_hints(&self) -> &NormalHints {
        &self.normal_hints
    }

    pub fn normal_hints_mut(&mut self) -> &mut NormalHints {
        &mut self.normal_hints
    }

    /// Read the client's `WM_NORMAL_HINTS` (min/max size, resize increments,
    /// aspect ratio, gravity) and merge it into our cached `NormalHints`.
    /// Called on manage and whenever `WM_NORMAL_HINTS` changes.
    pub fn update_normal_hints(&mut self, conn: &X11Connection) {
        use x11rb::properties::WmSizeHints;
        let wm_normal_hints = conn.atoms().get(crate::x11::Atom::WmNormalHints);
        let reply = match WmSizeHints::get(conn.conn(), self.client.window(), wm_normal_hints)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            Some(Some(r)) => r,
            _ => return,
        };
        let h = &mut self.normal_hints;
        if let Some((mw, mh)) = reply.min_size {
            h.min_width = mw.max(1) as u16;
            h.min_height = mh.max(1) as u16;
        }
        if let Some((mw, mh)) = reply.max_size {
            h.max_width = mw.min(u16::MAX as i32) as u16;
            h.max_height = mh.min(u16::MAX as i32) as u16;
        }
        if let Some((wi, hi)) = reply.size_increment {
            h.width_inc = wi.max(1) as u16;
            h.height_inc = hi.max(1) as u16;
        }
        if let Some((bw, bh)) = reply.base_size {
            h.base_width = bw.max(0) as u16;
            h.base_height = bh.max(0) as u16;
        }
        if let Some((min_a, max_a)) = reply.aspect {
            h.min_aspect = min_a.numerator as f64 / min_a.denominator.max(1) as f64;
            h.max_aspect = max_a.numerator as f64 / max_a.denominator.max(1) as f64;
        }
        // `win_gravity` is stored as a raw u32 elsewhere; Gravity has no
        // primitive cast, so we leave the cached value at its default unless
        // needed by a future gravity-aware resize.
    }

    pub fn workspace(&self) -> u32 {
        self.workspace
    }

    pub fn set_workspace(&mut self, ws: u32) {
        self.workspace = ws;
    }

    pub fn layer(&self) -> Layer {
        self.layer
    }

    pub fn set_layer(&mut self, layer: Layer) {
        self.layer = layer;
    }

    pub fn is_mapped(&self) -> bool {
        self.state.mapped
    }

    pub fn is_iconic(&self) -> bool {
        self.state.iconic
    }

    pub fn is_shaded(&self) -> bool {
        self.state.shaded
    }

    pub fn is_maximized(&self) -> bool {
        self.state.maximized_vert || self.state.maximized_horz
    }

    /// True when `rect` already fills (or exceeds) the workarea in both
    /// dimensions, i.e. the window is already maximized-looking and there is
    /// no smaller "normal" geometry to restore to. Matches both workarea-sized
    /// windows and larger (root-sized) ones that apps open maximized into.
    pub fn covers_workarea(rect: Rectangle, wa: &Rectangle) -> bool {
        let slack = 4i16;
        let right = rect.x + rect.width as i16;
        let bottom = rect.y + rect.height as i16;
        let wa_right = wa.x + wa.width as i16;
        let wa_bottom = wa.y + wa.height as i16;
        // Window spans at least the workarea horizontally and vertically
        // (allowing it to be larger — e.g. full-root maximized windows).
        rect.x <= wa.x + slack
            && rect.y <= wa.y + slack
            && right >= wa_right - slack
            && bottom >= wa_bottom - slack
    }

    /// A sensible centered "normal" window size used as a fallback restore
    /// point when a window was already maximized when managed (so the app
    /// never reopens stuck in the maximized state).
    pub fn default_normal_rect(wa: &Rectangle) -> Rectangle {
        let w = (wa.width as f32 * 0.6) as u16;
        let h = (wa.height as f32 * 0.6) as u16;
        let x = wa.x + ((wa.width - w) / 2) as i16;
        let y = wa.y + ((wa.height - h) / 2) as i16;
        Rectangle::new(x, y, w, h)
    }

    pub fn is_fullscreen(&self) -> bool {
        self.state.fullscreen
    }

    pub fn is_sticky(&self) -> bool {
        self.state.sticky
    }

    pub fn is_hidden(&self) -> bool {
        self.state.hidden
    }

    /// Tear down the frame and free its server-side resources. The client
    /// window is left intact (the caller may reparent it back to root first).
    pub fn destroy(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        self.frame.destroy(conn)?;
        Ok(())
    }

    // ───────────────────────────────────────────────
    //  Window operations
    // ───────────────────────────────────────────────

    pub fn iconify(&mut self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        if self.state.iconic { return Ok(()); }
        self.state.iconic = true;
        self.frame.iconified = true;
        conn.conn().unmap_window(self.client.window())?;
        self.frame.hide(conn)?;
        Ok(())
    }

    pub fn deiconify(&mut self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        if !self.state.iconic { return Ok(()); }
        self.state.iconic = false;
        self.frame.iconified = false;
        self.frame.show(conn)?;
        conn.conn().map_window(self.client.window())?;
        Ok(())
    }

    pub fn maximize(&mut self, conn: &X11Connection, vert: bool, horz: bool, wa: &Rectangle) -> Result<(), anyhow::Error> {
        if self.state.fullscreen || self.state.iconic {
            return Ok(());
        }
        if !self.state.maximized_vert && !self.state.maximized_horz {
            // Only remember the current geometry as the restore point if the
            // window is NOT already filling the workarea. Apps that reopen
            // maximized (e.g. kitty remembering its session) come up already
            // at full workarea size; saving that as the restore rect would make
            // un-maximize a no-op and trap the user in the maximized state.
            if !Self::covers_workarea(self.geometry, wa) {
                self.state.save_position(self.geometry);
            } else {
                self.state.save_position(Self::default_normal_rect(wa));
            }
        }
        self.state.maximized_vert = vert;
        self.state.maximized_horz = horz;
        self.frame.maximized = vert && horz;

        let new_x = if horz { wa.x } else { self.geometry.x };
        let new_y = if vert { wa.y } else { self.geometry.y };
        let new_w = if horz { wa.width } else { self.geometry.width };
        let new_h = if vert { wa.height } else { self.geometry.height };

        self.frame.move_resize(conn, new_x, new_y, new_w, new_h)?;
        self.reconfigure_client(conn, new_w, new_h)?;
        self.geometry = Rectangle::new(new_x, new_y, new_w, new_h);
        Ok(())
    }

    pub fn unmaximize(&mut self, conn: &X11Connection, wa: &Rectangle) -> Result<(), anyhow::Error> {
        if !self.state.maximized_vert && !self.state.maximized_horz {
            return Ok(());
        }
        self.state.maximized_vert = false;
        self.state.maximized_horz = false;
        self.frame.maximized = false;

        // Fall back to a centered default if no sensible restore point exists
        // (e.g. the window was already maximized when it was managed).
        let r = self
            .state
            .restore_position()
            .filter(|r| !Self::covers_workarea(*r, wa))
            .unwrap_or_else(|| Self::default_normal_rect(wa));
        self.frame.move_resize(conn, r.x, r.y, r.width, r.height)?;
        self.reconfigure_client(conn, r.width, r.height)?;
        self.geometry = r;
        Ok(())
    }

    pub fn reconfigure_client(&self, conn: &X11Connection, w: u16, h: u16) -> Result<(), anyhow::Error> {
        use x11rb::protocol::xproto::ConfigureWindowAux;
        let bw = self.frame.border_width();
        let th = self.frame.title_height();
        conn.conn().configure_window(
            self.client.window(),
            &ConfigureWindowAux::new()
                .x(bw as i32)
                .y((bw + th) as i32)
                .width(w as u32)
                .height(h as u32),
        )?;
        Ok(())
    }

    pub fn set_fullscreen(&mut self, conn: &X11Connection, fs: bool, root_w: u16, root_h: u16) -> Result<(), anyhow::Error> {
        use x11rb::protocol::xproto::ConfigureWindowAux;
        if fs == self.state.fullscreen { return Ok(()); }
        self.state.fullscreen = fs;
        let bw = self.frame.border_width();
        let th = self.frame.title_height();
        if fs {
            // Remember the pre-fullscreen geometry so we can always rebuild the
            // decorations on exit — even for already-maximized windows (which do
            // not save a maximize restore point). Kept separate from
            // `position` so it never disturbs unmaximize.
            self.state.fullscreen_restore = Some(self.geometry);
            // Move frame to (0,0) and fill entire screen
            conn.conn().configure_window(
                self.frame.frame_window(),
                &ConfigureWindowAux::new()
                    .x(0).y(0)
                    .width(root_w as u32)
                    .height(root_h as u32),
            )?;
            // Position client at (0,0) inside frame, fill it
            conn.conn().configure_window(
                self.client.window(),
                &ConfigureWindowAux::new()
                    .x(0).y(0)
                    .width(root_w as u32)
                    .height(root_h as u32),
            )?;
            // Hide decorations
            conn.conn().unmap_window(self.frame.title_window())?;
            conn.conn().unmap_window(self.frame.handle_window())?;
            self.geometry = Rectangle::new(0, 0, root_w, root_h);
        } else {
            // Show decorations
            let _ = conn.conn().map_window(self.frame.title_window());
            if !self.is_shaded() {
                let _ = conn.conn().map_window(self.frame.handle_window());
            }
            if let Some(r) = self.state.fullscreen_restore {
                // Restore frame
                conn.conn().configure_window(
                    self.frame.frame_window(),
                    &ConfigureWindowAux::new()
                        .x(r.x as i32).y(r.y as i32)
                        .width(r.width as u32)
                        .height((r.height + th + bw * 2) as u32),
                )?;
                // Restore client inside frame (offset by border + title)
                conn.conn().configure_window(
                    self.client.window(),
                    &ConfigureWindowAux::new()
                        .x(bw as i32).y((bw + th) as i32)
                        .width(r.width as u32)
                        .height(r.height as u32),
                )?;
                // Update title bar
                conn.conn().configure_window(
                    self.frame.title_window(),
                    &ConfigureWindowAux::new().width(r.width as u32),
                )?;
                self.geometry = r;
            }
            // Always repaint the titlebar on exit so the decoration windows
            // never get left showing their blank white background (the
            // "white bar" seen when leaving fullscreen from a maximized window).
            self.frame.draw_titlebar(conn)?;
        }
        self.send_configure_notify(conn)?;
        Ok(())
    }

    /// Move the frame to (x, y) with optional resize.
    pub fn move_resize(
        &mut self,
        conn: &X11Connection,
        x: i16, y: i16,
        w: u16, h: u16,
    ) -> Result<(), anyhow::Error> {
        self.geometry = Rectangle::new(x, y, w, h);
        self.frame.move_resize(conn, x, y, w, h)?;
        self.send_configure_notify(conn)?;
        Ok(())
    }

    pub fn move_to(&self, conn: &X11Connection, x: i16, y: i16) -> Result<(), anyhow::Error> {
        self.frame.move_to(conn, x, y)?;
        Ok(())
    }

    fn send_configure_notify(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        use x11rb::protocol::xproto::ConfigureNotifyEvent;
        let bw = self.frame.border_width();
        let ev = ConfigureNotifyEvent {
            response_type: x11rb::protocol::xproto::CONFIGURE_NOTIFY_EVENT,
            sequence: 0,
            event: self.client.window(),
            window: self.client.window(),
            above_sibling: x11rb::NONE,
            x: (self.geometry.x + bw as i16) as i16,
            y: (self.geometry.y + bw as i16 + self.frame.title_height() as i16) as i16,
            width: self.geometry.width,
            height: self.geometry.height,
            border_width: bw,
            override_redirect: false,
        };
        conn.conn().send_event(false, self.client.window(), x11rb::protocol::xproto::EventMask::NO_EVENT, ev)?;
        Ok(())
    }
}

pub struct WindowManager {
    windows: HashMap<WindowId, RustboxWindow>,
    focus_order: Vec<WindowId>,
    stacking_order: Vec<WindowId>,
    last_focused: Option<WindowId>,
    active_window: Option<WindowId>,
}

impl WindowManager {
    pub fn new() -> Self {
        Self {
            windows: HashMap::new(),
            focus_order: Vec::new(),
            stacking_order: Vec::new(),
            last_focused: None,
            active_window: None,
        }
    }

    pub fn add_window(&mut self, window: RustboxWindow) {
        let id = window.id();
        self.windows.insert(id, window);
        self.stacking_order.push(id);
    }

    pub fn remove_window(&mut self, id: WindowId) {
        self.windows.remove(&id);
        self.focus_order.retain(|&x| x != id);
        self.stacking_order.retain(|&x| x != id);
        if self.active_window == Some(id) {
            self.active_window = None;
        }
        if self.last_focused == Some(id) {
            self.last_focused = None;
        }
    }

    pub fn get_window(&self, id: WindowId) -> Option<&RustboxWindow> {
        self.windows.get(&id)
    }

    pub fn get_window_mut(&mut self, id: WindowId) -> Option<&mut RustboxWindow> {
        self.windows.get_mut(&id)
    }

    pub fn active_window(&self) -> Option<WindowId> {
        self.active_window
    }

    pub fn set_active_window(&mut self, id: WindowId) {
        if self.windows.contains_key(&id) {
            if let Some(prev) = self.active_window {
                self.last_focused = Some(prev);
            }
            self.active_window = Some(id);
            self.focus_order.retain(|&x| x != id);
            self.focus_order.push(id);
        }
    }

    pub fn windows(&self) -> impl Iterator<Item = &RustboxWindow> {
        self.windows.values()
    }

    pub fn windows_mut(&mut self) -> impl Iterator<Item = &mut RustboxWindow> {
        self.windows.values_mut()
    }

    pub fn count(&self) -> usize {
        self.windows.len()
    }

    pub fn iter_by_stacking(&self) -> impl Iterator<Item = &RustboxWindow> {
        self.stacking_order.iter().filter_map(|id| self.windows.get(id))
    }
}

impl Default for WindowManager {
    fn default() -> Self {
        Self::new()
    }
}
