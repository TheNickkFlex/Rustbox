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

pub struct FluxboxWindow {
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

impl FluxboxWindow {
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
    pub fn redraw_title(&self, conn: &X11Connection) {
        let _ = self.frame.draw_titlebar(conn);
    }

    /// Make the window (frame + client) visible. Used when its workspace is
    /// switched to.
    pub fn show(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
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

    pub fn geometry(&self) -> &Rectangle {
        &self.geometry
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
}

pub struct WindowManager {
    windows: HashMap<WindowId, FluxboxWindow>,
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

    pub fn add_window(&mut self, window: FluxboxWindow) {
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

    pub fn get_window(&self, id: WindowId) -> Option<&FluxboxWindow> {
        self.windows.get(&id)
    }

    pub fn get_window_mut(&mut self, id: WindowId) -> Option<&mut FluxboxWindow> {
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

    pub fn windows(&self) -> impl Iterator<Item = &FluxboxWindow> {
        self.windows.values()
    }

    pub fn windows_mut(&mut self) -> impl Iterator<Item = &mut FluxboxWindow> {
        self.windows.values_mut()
    }

    pub fn count(&self) -> usize {
        self.windows.len()
    }

    pub fn iter_by_stacking(&self) -> impl Iterator<Item = &FluxboxWindow> {
        self.stacking_order.iter().filter_map(|id| self.windows.get(id))
    }
}

impl Default for WindowManager {
    fn default() -> Self {
        Self::new()
    }
}
