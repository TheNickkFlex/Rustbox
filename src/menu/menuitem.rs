

pub type MenuId = u32;

#[derive(Debug, Clone)]
pub enum MenuItemType {
    /// Execute a shell command.
    Exec(String),
    /// Open a submenu (submenu_id, submenu_label).
    Submenu(MenuId, String),
    /// Exit rustbox.
    Exit,
    /// Restart rustbox.
    Restart,
    /// Re-read config.
    Reconfig,
    /// Separator line.
    Separator,
    /// Included submenu from another file (resolved at parse time).
    Include(String),
    /// Workspaces list (generated).
    Workspaces,
    /// Switch to a specific workspace.
    Workspace(u32),
    /// Create a new workspace.
    WorkspaceCreate,
    /// Rename a workspace (the `u32` is the workspace index, `String` is the
    /// new name).
    WorkspaceRename(u32),
    /// Open an interactive "Run Command" dialog.
    RunDialog,
    /// No operation (label-only, disabled).
    Nop,
}

#[derive(Debug, Clone)]
pub struct MenuItem {
    label: String,
    item_type: MenuItemType,
    enabled: bool,
}

impl MenuItem {
    pub fn new(label: &str, item_type: MenuItemType) -> Self {
        let enabled = !matches!(item_type, MenuItemType::Separator | MenuItemType::Nop);
        Self { label: label.to_string(), item_type, enabled }
    }

    pub fn separator() -> Self {
        Self::new("", MenuItemType::Separator)
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn item_type(&self) -> &MenuItemType {
        &self.item_type
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}
