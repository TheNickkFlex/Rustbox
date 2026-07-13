#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Point {
    pub x: i16,
    pub y: i16,
}

impl Point {
    pub const fn new(x: i16, y: i16) -> Self {
        Self { x, y }
    }

    pub const fn zero() -> Self {
        Self { x: 0, y: 0 }
    }
}

impl From<(i16, i16)> for Point {
    fn from((x, y): (i16, i16)) -> Self {
        Self { x, y }
    }
}
