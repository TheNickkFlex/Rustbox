#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Strut {
    pub left: u16,
    pub right: u16,
    pub top: u16,
    pub bottom: u16,
}

impl Strut {
    pub const fn new(left: u16, right: u16, top: u16, bottom: u16) -> Self {
        Self { left, right, top, bottom }
    }

    pub const fn zero() -> Self {
        Self { left: 0, right: 0, top: 0, bottom: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.left == 0 && self.right == 0 && self.top == 0 && self.bottom == 0
    }

    pub fn expand(&mut self, other: &Strut) {
        self.left = self.left.max(other.left);
        self.right = self.right.max(other.right);
        self.top = self.top.max(other.top);
        self.bottom = self.bottom.max(other.bottom);
    }
}

impl From<(u16, u16, u16, u16)> for Strut {
    fn from((l, r, t, b): (u16, u16, u16, u16)) -> Self {
        Self::new(l, r, t, b)
    }
}
