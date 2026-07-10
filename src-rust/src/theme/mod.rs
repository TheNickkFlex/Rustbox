use std::collections::HashMap;
use std::path::Path;

use crate::core::Color;
use crate::render::texture::Texture;

pub struct Theme {
    name: String,
    parent: Option<Box<Theme>>,
    resources: HashMap<String, String>,
}

impl Theme {
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            parent: None,
            resources: HashMap::new(),
        }
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, anyhow::Error> {
        let content = std::fs::read_to_string(path.as_ref())?;
        let mut theme = Theme::new(
            path.as_ref().file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
        );

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('!') || line.starts_with('#') {
                continue;
            }

            if let Some(eq_pos) = line.find(':') {
                let key = line[..eq_pos].trim().to_string();
                let value = line[eq_pos + 1..].trim().to_string();
                theme.resources.insert(key, value);
            }
        }

        Ok(theme)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.resources.get(key).map(|s| s.as_str())
    }

    pub fn get_color(&self, key: &str) -> Option<Color> {
        self.get(key).and_then(|v| {
            if v.starts_with("rgb:") || v.starts_with('#') {
                Color::from_hex(v.trim_start_matches("rgb:"))
            } else {
                Color::from_hex(v)
            }
        })
    }

    pub fn get_int(&self, key: &str) -> Option<i32> {
        self.get(key).and_then(|v| v.parse::<i32>().ok())
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key).map(|v| {
            matches!(v.to_lowercase().as_str(), "true" | "yes" | "1" | "on")
        })
    }

    pub fn get_texture(&self, key: &str) -> Option<Texture> {
        let value = self.get(key)?;
        let mut texture = Texture::new();
        let parts: Vec<&str> = value.split_whitespace().collect();

        for part in parts {
            let lower = part.to_lowercase();
            if let Some(tt) = crate::core::TextureType::from_str(part) {
                texture.type_ = tt;
            } else if lower.starts_with("gradient") {
                texture.type_ = crate::core::TextureType::Gradient;
                let grad_name = lower.trim_start_matches("gradient");
                texture.gradient = match grad_name {
                    "diagonal" => crate::render::texture::GradientType::Diagonal,
                    "horizontal" => crate::render::texture::GradientType::Horizontal,
                    "vertical" => crate::render::texture::GradientType::Vertical,
                    "crossdiagonal" => crate::render::texture::GradientType::CrossDiagonal,
                    "pyramid" => crate::render::texture::GradientType::Pyramid,
                    "rectangle" => crate::render::texture::GradientType::Rectangle,
                    "pipecross" => crate::render::texture::GradientType::PipeCross,
                    "elliptic" => crate::render::texture::GradientType::Elliptic,
                    "mirrorhorizontal" => crate::render::texture::GradientType::MirrorHorizontal,
                    "mirrorvertical" => crate::render::texture::GradientType::MirrorVertical,
                    _ => crate::render::texture::GradientType::Horizontal,
                };
            } else if part.starts_with('#') || part.starts_with("rgb:") {
                if texture.color.red == 0 && texture.color.green == 0 && texture.color.blue == 0 {
                    texture.color = Color::from_hex(part).unwrap_or_default();
                } else {
                    texture.color_to = Color::from_hex(part).unwrap_or_default();
                }
            } else if let Ok(n) = part.parse::<u16>() {
                texture.bevel_width = n;
            }
        }

        Some(texture)
    }

    pub fn merge(&mut self, other: Theme) {
        for (k, v) in other.resources {
            self.resources.entry(k).or_insert(v);
        }
    }
}

pub struct ThemeManager {
    current_theme: Option<Theme>,
    theme_path: Option<String>,
}

impl ThemeManager {
    pub fn new() -> Self {
        Self {
            current_theme: None,
            theme_path: None,
        }
    }

    pub fn load_theme<P: AsRef<Path>>(&mut self, path: P) -> Result<(), anyhow::Error> {
        let theme = Theme::load(&path)?;
        self.theme_path = Some(path.as_ref().to_string_lossy().to_string());
        self.current_theme = Some(theme);
        Ok(())
    }

    pub fn theme(&self) -> Option<&Theme> {
        self.current_theme.as_ref()
    }

    pub fn theme_mut(&mut self) -> Option<&mut Theme> {
        self.current_theme.as_mut()
    }

    pub fn theme_path(&self) -> Option<&str> {
        self.theme_path.as_deref()
    }
}

impl Default for ThemeManager {
    fn default() -> Self {
        Self::new()
    }
}
