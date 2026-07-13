use crate::core::Rectangle;

#[derive(Debug, Clone)]
pub struct WindowState {
    pub mapped: bool,
    pub iconic: bool,
    pub shaded: bool,
    pub maximized_vert: bool,
    pub maximized_horz: bool,
    pub fullscreen: bool,
    pub sticky: bool,
    pub hidden: bool,
    pub above: bool,
    pub below: bool,
    pub skip_taskbar: bool,
    pub skip_pager: bool,
    pub demands_attention: bool,
    pub modal: bool,
    pub rescaling: bool,
    pub position: Option<Rectangle>,
    pub workspace: u32,
    pub previous_workspace: u32,
}

impl WindowState {
    pub fn new() -> Self {
        Self {
            mapped: false,
            iconic: false,
            shaded: false,
            maximized_vert: false,
            maximized_horz: false,
            fullscreen: false,
            sticky: false,
            hidden: false,
            above: false,
            below: false,
            skip_taskbar: false,
            skip_pager: false,
            demands_attention: false,
            modal: false,
            rescaling: false,
            position: None,
            workspace: 0,
            previous_workspace: 0,
        }
    }

    pub fn set_state(&mut self, state: &str, enable: bool) {
        match state {
            "shaded" => self.shaded = enable,
            "maximized_vert" | "_NET_WM_STATE_MAXIMIZED_VERT" => self.maximized_vert = enable,
            "maximized_horz" | "_NET_WM_STATE_MAXIMIZED_HORZ" => self.maximized_horz = enable,
            "fullscreen" | "_NET_WM_STATE_FULLSCREEN" => self.fullscreen = enable,
            "sticky" | "_NET_WM_STATE_STICKY" => self.sticky = enable,
            "hidden" | "_NET_WM_STATE_HIDDEN" => self.hidden = enable,
            "above" | "_NET_WM_STATE_ABOVE" => self.above = enable,
            "below" | "_NET_WM_STATE_BELOW" => self.below = enable,
            "skip_taskbar" | "_NET_WM_STATE_SKIP_TASKBAR" => self.skip_taskbar = enable,
            "skip_pager" | "_NET_WM_STATE_SKIP_PAGER" => self.skip_pager = enable,
            "demands_attention" | "_NET_WM_STATE_DEMANDS_ATTENTION" => self.demands_attention = enable,
            "modal" => self.modal = enable,
            _ => {}
        }
    }

    pub fn save_position(&mut self, rect: Rectangle) {
        self.position = Some(rect);
    }

    pub fn restore_position(&self) -> Option<Rectangle> {
        self.position
    }
}

impl Default for WindowState {
    fn default() -> Self {
        Self::new()
    }
}
