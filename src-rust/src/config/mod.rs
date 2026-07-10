use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::core::Rectangle;

pub struct FluxboxConfig {
    pub init: InitConfig,
    pub keys: KeysConfig,
    pub menu: MenuConfig,
    pub apps: AppsConfig,
    pub startup: Vec<String>,
}

pub struct InitConfig {
    pub session: SessionConfig,
    pub window: WindowConfig,
    pub workspace: WorkspaceConfig,
    pub menu: MenuConfig,
    pub toolbar: ToolbarConfig,
    pub slit: SlitConfig,
    pub focus: FocusConfig,
    pub resources: HashMap<String, String>,
    pub extra_config_files: Vec<PathBuf>,
}

pub struct SessionConfig {
    pub style_file: PathBuf,
    pub screen_count: u32,
    pub auto_raise_delay: u32,
    pub cache_life: u32,
    pub cache_max: u32,
    pub double_click_interval: u32,
    pub opacity: OpacityConfig,
}

pub struct OpacityConfig {
    pub focused: u8,
    pub unfocused: u8,
    pub menu: u8,
    pub window: u8,
    pub slit: u8,
}

pub struct WindowConfig {
    pub placement: PlacementStrategy,
    pub resize_mode: ResizeMode,
    pub focus_hidden: bool,
    pub faked_focus: bool,
    pub focus_new: bool,
    pub focus_last: bool,
    pub focus_click_raises: bool,
    pub tab_alignment: TabAlignment,
    pub tab_width: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementStrategy {
    Cascade,
    UnderMouse,
    RowSmart,
    ColSmart,
    MinOverlap,
    Last,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeMode {
    Bottom,
    Right,
    BottomRight,
    Top,
    Left,
    TopLeft,
    TopRight,
    BottomLeft,
    Center,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabAlignment {
    Left,
    Center,
    Right,
}

pub struct WorkspaceConfig {
    pub names: Vec<String>,
    pub count: u32,
    pub warping: bool,
    pub mouse_warping: bool,
    pub screen: Option<u32>,
    pub rows: u32,
    pub columns: u32,
}

pub struct MenuConfig {
    pub file: PathBuf,
    pub delay: u32,
}

pub struct ToolbarConfig {
    pub visible: bool,
    pub auto_hide: bool,
    pub width_percent: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub on_head: u32,
    pub placement: ToolbarPlacement,
    pub layer: crate::core::Layer,
    pub alpha: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolbarPlacement {
    Top,
    Bottom,
    Left,
    Right,
    Fixed,
}

pub struct SlitConfig {
    pub visible: bool,
    pub auto_hide: bool,
    pub placement: SlitPlacement,
    pub direction: SlitDirection,
    pub layer: crate::core::Layer,
    pub alpha: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlitPlacement {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    BottomLeft,
    TopRight,
    BottomRight,
    Center,
    Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlitDirection {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone)]
pub struct FocusConfig {
    pub model: FocusModel,
    pub same_app: bool,
    pub follows_mouse: bool,
    pub strict_focus: bool,
}

impl Default for FocusConfig {
    fn default() -> Self {
        Self {
            model: FocusModel::ClickToFocus,
            same_app: false,
            follows_mouse: false,
            strict_focus: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusModel {
    ClickToFocus,
    FocusFollowsMouse,
    MouseFocus,
}

pub struct KeysConfig {
    pub file: PathBuf,
    pub bindings: Vec<KeyBinding>,
}

pub struct KeyBinding {
    pub key: String,
    pub mods: KeyModifiers,
    pub command: String,
    pub arguments: Vec<String>,
}

pub struct KeyModifiers {
    pub shift: bool,
    pub control: bool,
    pub mod1: bool,
    pub mod2: bool,
    pub mod3: bool,
    pub mod4: bool,
    pub mod5: bool,
}

pub struct AppsConfig {
    pub groups: Vec<AppGroup>,
}

pub struct AppGroup {
    pub patterns: Vec<AppPattern>,
    pub properties: AppProperties,
}

pub struct AppPattern {
    pub class: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub role: Option<String>,
}

pub struct AppProperties {
    pub workspace: Option<u32>,
    pub head: Option<u32>,
    pub layer: Option<crate::core::Layer>,
    pub alpha: Option<u8>,
    pub hidden: Option<bool>,
    pub sticky: Option<bool>,
    pub shaded: Option<bool>,
    pub maximized: Option<bool>,
    pub minimized: Option<bool>,
    pub fullscreen: Option<bool>,
    pub decorations: Option<WindowDecorations>,
    pub position: Option<Rectangle>,
    pub size: Option<(u16, u16)>,
    pub tab: Option<bool>,
    pub focus_hidden: Option<bool>,
    pub jump: Option<bool>,
}

pub struct WindowDecorations {
    pub titlebar: bool,
    pub handle: bool,
    pub border: bool,
    pub iconify: bool,
    pub maximize: bool,
    pub close: bool,
    pub sticky: bool,
    pub shade: bool,
    pub menu: bool,
}

impl InitConfig {
    pub fn new() -> Self {
        Self {
            session: SessionConfig {
                style_file: PathBuf::from(""),
                screen_count: 1,
                auto_raise_delay: 250,
                cache_life: 5,
                cache_max: 200,
                double_click_interval: 250,
                opacity: OpacityConfig {
                    focused: 255,
                    unfocused: 255,
                    menu: 255,
                    window: 255,
                    slit: 255,
                },
            },
            window: WindowConfig {
                placement: PlacementStrategy::Cascade,
                resize_mode: ResizeMode::BottomRight,
                focus_hidden: false,
                faked_focus: false,
                focus_new: false,
                focus_last: true,
                focus_click_raises: true,
                tab_alignment: TabAlignment::Center,
                tab_width: 64,
            },
            workspace: WorkspaceConfig {
                names: vec!["1".to_string(), "2".to_string(), "3".to_string(), "4".to_string()],
                count: 4,
                warping: true,
                mouse_warping: false,
                screen: None,
                rows: 1,
                columns: 1,
            },
            menu: MenuConfig {
                file: PathBuf::from(""),
                delay: 200,
            },
            toolbar: ToolbarConfig {
                visible: true,
                auto_hide: false,
                width_percent: 100,
                height: 0,
                x: 0,
                y: 0,
                on_head: 0,
                placement: ToolbarPlacement::Bottom,
                layer: crate::core::Layer::DOCK,
                alpha: 255,
            },
            slit: SlitConfig {
                visible: true,
                auto_hide: false,
                placement: SlitPlacement::TopRight,
                direction: SlitDirection::Vertical,
                layer: crate::core::Layer::DOCK,
                alpha: 255,
            },
            focus: FocusConfig {
                model: FocusModel::ClickToFocus,
                same_app: false,
                follows_mouse: false,
                strict_focus: false,
            },
            resources: HashMap::new(),
            extra_config_files: Vec::new(),
        }
    }
}

impl Default for InitConfig {
    fn default() -> Self {
        Self::new()
    }
}

pub fn parse_init_file<P: AsRef<Path>>(path: P) -> Result<InitConfig, anyhow::Error> {
    let content = std::fs::read_to_string(path.as_ref())?;
    let mut config = InitConfig::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('!') || line.starts_with('#') {
            continue;
        }
        if let Some(eq_pos) = line.find(':') {
            let key = line[..eq_pos].trim().to_string();
            let value = line[eq_pos + 1..].trim().to_string();
            config.resources.insert(key, value);
        }
    }

    Ok(config)
}

pub fn find_config_files(config_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let config_dir = config_dir.join("fluxbox");

    let entries = [
        "init",
        "keys",
        "menu",
        "apps",
        "overlay",
        "startup",
        "windowmenu",
    ];

    for entry in &entries {
        let path = config_dir.join(entry);
        if path.exists() {
            files.push(path);
        }
    }

    files
}
