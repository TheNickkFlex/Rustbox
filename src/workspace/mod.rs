use crate::window::WindowId;

#[derive(Debug, Clone)]
pub struct Workspace {
    id: u32,
    name: String,
    windows: Vec<WindowId>,
    last_focused: Option<WindowId>,
    stacking_order: Vec<WindowId>,
}

impl Workspace {
    pub fn new(id: u32, name: &str) -> Self {
        Self {
            id,
            name: name.to_string(),
            windows: Vec::new(),
            last_focused: None,
            stacking_order: Vec::new(),
        }
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: &str) {
        self.name = name.to_string();
    }

    pub fn add_window(&mut self, id: WindowId) {
        if !self.windows.contains(&id) {
            self.windows.push(id);
        }
        self.last_focused = Some(id);
    }

    pub fn remove_window(&mut self, id: WindowId) {
        self.windows.retain(|&w| w != id);
        self.stacking_order.retain(|&w| w != id);
        if self.last_focused == Some(id) {
            self.last_focused = self.windows.last().copied();
        }
    }

    pub fn has_window(&self, id: WindowId) -> bool {
        self.windows.contains(&id)
    }

    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    pub fn windows(&self) -> &[WindowId] {
        &self.windows
    }

    pub fn last_focused(&self) -> Option<WindowId> {
        self.last_focused
    }

    pub fn set_last_focused(&mut self, id: WindowId) {
        self.last_focused = Some(id);
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    pub fn raise_window(&mut self, id: WindowId) {
        self.stacking_order.retain(|&w| w != id);
        self.stacking_order.push(id);
    }

    pub fn lower_window(&mut self, id: WindowId) {
        self.stacking_order.retain(|&w| w != id);
        self.stacking_order.insert(0, id);
    }
}
