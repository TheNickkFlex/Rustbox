
pub type WindowId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowType {
    Desktop,
    Dock,
    Toolbar,
    Menu,
    Utility,
    Splash,
    Dialog,
    Normal,
    DropdownMenu,
    PopupMenu,
    Tooltip,
    Notification,
    Combo,
    Dnd,
}

impl WindowType {
    pub fn from_atom(atom: u32) -> Self {
        match atom {
            0 => WindowType::Desktop,
            1 => WindowType::Dock,
            2 => WindowType::Toolbar,
            3 => WindowType::Menu,
            4 => WindowType::Utility,
            5 => WindowType::Splash,
            6 => WindowType::Dialog,
            7 => WindowType::Normal,
            8 => WindowType::DropdownMenu,
            9 => WindowType::PopupMenu,
            10 => WindowType::Tooltip,
            11 => WindowType::Notification,
            12 => WindowType::Combo,
            13 => WindowType::Dnd,
            _ => WindowType::Normal,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct WinClient {
    window: WindowId,
    frame: Option<WindowId>,
    parent: WindowId,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
    border_width: u16,
    desktop: u32,
    name: String,
    icon_name: String,
    class_name: String,
    class_instance: String,
    role: String,
    window_type: WindowType,
    transient_for: Option<WindowId>,
    pid: u32,
    has_focus: bool,
    urgent: bool,
    mapped: bool,
}

impl WinClient {
    pub fn new(window: WindowId, parent: WindowId) -> Self {
        Self {
            window,
            frame: None,
            parent,
            x: 0, y: 0,
            width: 1, height: 1,
            border_width: 0,
            desktop: 0,
            name: String::new(),
            icon_name: String::new(),
            class_name: String::new(),
            class_instance: String::new(),
            role: String::new(),
            window_type: WindowType::Normal,
            transient_for: None,
            pid: 0,
            has_focus: false,
            urgent: false,
            mapped: false,
        }
    }

    pub fn window(&self) -> WindowId {
        self.window
    }

    pub fn frame(&self) -> Option<WindowId> {
        self.frame
    }

    pub fn set_frame(&mut self, frame: WindowId) {
        self.frame = Some(frame);
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    pub fn class_name(&self) -> &str {
        &self.class_name
    }

    pub fn class_instance(&self) -> &str {
        &self.class_instance
    }

    pub fn window_type(&self) -> WindowType {
        self.window_type
    }

    pub fn set_window_type(&mut self, wtype: WindowType) {
        self.window_type = wtype;
    }

    pub fn has_focus(&self) -> bool {
        self.has_focus
    }

    pub fn set_focus(&mut self, focused: bool) {
        self.has_focus = focused;
    }

    pub fn is_urgent(&self) -> bool {
        self.urgent
    }

    pub fn set_urgent(&mut self, urgent: bool) {
        self.urgent = urgent;
    }

    pub fn is_mapped(&self) -> bool {
        self.mapped
    }

    pub fn set_mapped(&mut self, mapped: bool) {
        self.mapped = mapped;
    }
}
