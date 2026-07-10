use crate::command::Command;

pub struct MenuSystem {
    root_menus: Vec<Menu>,
    menu_stack: Vec<u32>,
    current_menu: Option<u32>,
    visible: bool,
}

impl MenuSystem {
    pub fn new() -> Self {
        Self {
            root_menus: Vec::new(),
            menu_stack: Vec::new(),
            current_menu: None,
            visible: false,
        }
    }
}

impl Default for MenuSystem {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Menu {
    id: u32,
    label: String,
    items: Vec<MenuItem>,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
    visible: bool,
}

impl Menu {
    pub fn new(id: u32, label: &str) -> Self {
        Self {
            id,
            label: label.to_string(),
            items: Vec::new(),
            x: 0, y: 0,
            width: 0, height: 0,
            visible: false,
        }
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn add_item(&mut self, item: MenuItem) {
        self.items.push(item);
    }

    pub fn items(&self) -> &[MenuItem] {
        &self.items
    }

    pub fn position(&self) -> (i16, i16) {
        (self.x, self.y)
    }

    pub fn set_position(&mut self, x: i16, y: i16) {
        self.x = x;
        self.y = y;
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }
}

pub struct MenuItem {
    label: String,
    icon: Option<String>,
    item_type: MenuItemType,
    enabled: bool,
    selected: bool,
}

pub enum MenuItemType {
    Normal(Box<dyn Command>),
    Submenu(u32),
    Separator,
    Checkmark(bool),
    Radio(bool),
}

impl MenuItem {
    pub fn new(label: &str) -> Self {
        Self {
            label: label.to_string(),
            icon: None,
            item_type: MenuItemType::Normal(Box::new(crate::command::ExecCommand {
                command: String::new(),
            })),
            enabled: true,
            selected: false,
        }
    }

    pub fn separator() -> Self {
        Self {
            label: String::new(),
            icon: None,
            item_type: MenuItemType::Separator,
            enabled: false,
            selected: false,
        }
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

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn set_command(&mut self, command: Box<dyn Command>) {
        self.item_type = MenuItemType::Normal(command);
    }

    pub fn set_submenu(&mut self, menu_id: u32) {
        self.item_type = MenuItemType::Submenu(menu_id);
    }
}
