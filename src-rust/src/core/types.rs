#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Align {
    Left,
    Right,
    Center,
    RelativeLeft,
    RelativeRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Gravity {
    NorthWest,
    North,
    NorthEast,
    West,
    Center,
    East,
    SouthWest,
    South,
    SouthEast,
    Static,
}

impl Gravity {
    pub fn to_x11(&self) -> u32 {
        match self {
            Gravity::NorthWest => 1,
            Gravity::North => 2,
            Gravity::NorthEast => 3,
            Gravity::West => 4,
            Gravity::Center => 5,
            Gravity::East => 6,
            Gravity::SouthWest => 7,
            Gravity::South => 8,
            Gravity::SouthEast => 9,
            Gravity::Static => 10,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureType {
    Flat,
    Raised,
    Sunken,
    Grooved,
    Bevel1,
    Bevel2,
    Mica,
    Solid,
    Gradient,
    Pixmap,
    ParentRelative,
    Transparent,
}

impl TextureType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "flat" => Some(TextureType::Flat),
            "raised" => Some(TextureType::Raised),
            "sunken" => Some(TextureType::Sunken),
            "grooved" => Some(TextureType::Grooved),
            "bevel1" => Some(TextureType::Bevel1),
            "bevel2" => Some(TextureType::Bevel2),
            "mica" => Some(TextureType::Mica),
            "solid" => Some(TextureType::Solid),
            "gradient" => Some(TextureType::Gradient),
            "pixmap" => Some(TextureType::Pixmap),
            "parentrelative" => Some(TextureType::ParentRelative),
            "transparent" => Some(TextureType::Transparent),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorderSize {
    pub left: u16,
    pub right: u16,
    pub top: u16,
    pub bottom: u16,
}

impl BorderSize {
    pub const fn new(width: u16) -> Self {
        Self { left: width, right: width, top: width, bottom: width }
    }

    pub const fn zero() -> Self {
        Self { left: 0, right: 0, top: 0, bottom: 0 }
    }

    pub fn width_x(&self) -> u16 {
        self.left + self.right
    }

    pub fn width_y(&self) -> u16 {
        self.top + self.bottom
    }
}
