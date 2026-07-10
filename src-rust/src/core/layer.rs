use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Layer(u16);

impl Layer {
    pub const DESKTOP: Layer = Layer(0);
    pub const BOTTOM: Layer = Layer(2);
    pub const NORMAL: Layer = Layer(4);
    pub const TOP: Layer = Layer(6);
    pub const DOCK: Layer = Layer(8);
    pub const ABOVE_DOCK: Layer = Layer(10);
    pub const MENU: Layer = Layer(12);
    pub const OSD: Layer = Layer(14);
    pub const TOOLTIP: Layer = Layer(16);
    pub const FULLSCREEN: Layer = Layer(18);

    pub const MIN: Layer = Layer(0);
    pub const MAX: Layer = Layer(20);

    pub fn new(num: u16) -> Self {
        Layer(num.min(Self::MAX.0))
    }

    pub fn num(&self) -> u16 {
        self.0
    }

    pub fn raise(&self) -> Layer {
        Layer((self.0 + 1).min(Self::MAX.0))
    }

    pub fn lower(&self) -> Layer {
        Layer(self.0.saturating_sub(1))
    }

    pub fn is_above(&self, other: Layer) -> bool {
        self.0 > other.0
    }
}

impl PartialOrd for Layer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Layer {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl From<u16> for Layer {
    fn from(n: u16) -> Self {
        Layer::new(n)
    }
}

#[derive(Debug, Clone)]
pub struct LayerItem<T> {
    pub item: T,
    pub layer: Layer,
}

impl<T> LayerItem<T> {
    pub fn new(item: T, layer: Layer) -> Self {
        Self { item, layer }
    }
}
