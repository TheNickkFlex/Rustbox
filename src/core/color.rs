use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub struct Color {
    pub red: u16,
    pub green: u16,
    pub blue: u16,
    pub alpha: u16,
    pub pixel: u32,
    pub alloc: ColorAlloc,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ColorAlloc {
    None,
    Pixel(u32),
    Named(Arc<str>),
}

impl Color {
    pub const fn new(r: u16, g: u16, b: u16, a: u16) -> Self {
        Self {
            red: r,
            green: g,
            blue: b,
            alpha: a,
            pixel: 0,
            alloc: ColorAlloc::None,
        }
    }

    pub const fn from_pixel(pixel: u32) -> Self {
        Self {
            red: 0,
            green: 0,
            blue: 0,
            alpha: 0xffff,
            pixel,
            alloc: ColorAlloc::None,
        }
    }

    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim_start_matches('#');
        let (r, g, b, a) = match hex.len() {
            3 => {
                let r = u16::from_str_radix(&hex[0..1], 16).ok()? * 0x1111;
                let g = u16::from_str_radix(&hex[1..2], 16).ok()? * 0x1111;
                let b = u16::from_str_radix(&hex[2..3], 16).ok()? * 0x1111;
                (r, g, b, 0xffff)
            }
            6 => {
                let r = u16::from_str_radix(&hex[0..2], 16).ok()? * 0x101;
                let g = u16::from_str_radix(&hex[2..4], 16).ok()? * 0x101;
                let b = u16::from_str_radix(&hex[4..6], 16).ok()? * 0x101;
                (r, g, b, 0xffff)
            }
            8 => {
                let r = u16::from_str_radix(&hex[0..2], 16).ok()? * 0x101;
                let g = u16::from_str_radix(&hex[2..4], 16).ok()? * 0x101;
                let b = u16::from_str_radix(&hex[4..6], 16).ok()? * 0x101;
                let a = u16::from_str_radix(&hex[6..8], 16).ok()? * 0x101;
                (r, g, b, a)
            }
            12 => {
                let r = u16::from_str_radix(&hex[0..3], 16).ok()?;
                let g = u16::from_str_radix(&hex[3..6], 16).ok()?;
                let b = u16::from_str_radix(&hex[6..9], 16).ok()?;
                let a = u16::from_str_radix(&hex[9..12], 16).ok()?;
                (r, g, b, a)
            }
            _ => return None,
        };
        Some(Self {
            red: r,
            green: g,
            blue: b,
            alpha: a,
            pixel: 0,
            alloc: ColorAlloc::None,
        })
    }

    pub fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        Self {
            red: (r as u16) * 0x101,
            green: (g as u16) * 0x101,
            blue: (b as u16) * 0x101,
            alpha: 0xffff,
            pixel: 0,
            alloc: ColorAlloc::None,
        }
    }

    pub fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            red: (r as u16) * 0x101,
            green: (g as u16) * 0x101,
            blue: (b as u16) * 0x101,
            alpha: (a as u16) * 0x101,
            pixel: 0,
            alloc: ColorAlloc::None,
        }
    }

    pub fn brightness(&self) -> f64 {
        0.299 * self.red as f64 + 0.587 * self.green as f64 + 0.114 * self.blue as f64
    }

    pub fn is_dark(&self) -> bool {
        self.brightness() < 32768.0
    }

    pub fn with_alpha(&self, alpha: u16) -> Self {
        let mut c = self.clone();
        c.alpha = alpha;
        c
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::new(0, 0, 0, 0xffff)
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "#{:04x}{:04x}{:04x}{:04x}",
            self.red, self.green, self.blue, self.alpha
        )
    }
}

#[allow(dead_code)]
pub fn parse_color(s: &str) -> Option<Color> {
    if s.starts_with("rgb:") || s.starts_with("rgba:") {
        let parts: Vec<&str> = if s.starts_with("rgba:") {
            s[5..].split('/').collect()
        } else {
            s[4..].split('/').collect()
        };
        if parts.len() >= 3 {
            let r = u16::from_str_radix(parts[0], 16).ok()?;
            let g = u16::from_str_radix(parts[1], 16).ok()?;
            let b = u16::from_str_radix(parts[2], 16).ok()?;
            let a = parts.get(3).and_then(|v| u16::from_str_radix(v, 16).ok()).unwrap_or(0xffff);
            return Some(Color::new(r, g, b, a));
        }
    }
    Color::from_hex(s)
}
