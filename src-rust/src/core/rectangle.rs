use crate::core::Point;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rectangle {
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
}

impl Rectangle {
    pub const fn new(x: i16, y: i16, width: u16, height: u16) -> Self {
        Self { x, y, width, height }
    }

    pub const fn zero() -> Self {
        Self { x: 0, y: 0, width: 0, height: 0 }
    }

    pub fn contains(&self, px: i16, py: i16) -> bool {
        px >= self.x
            && px < self.x.saturating_add(self.width as i16)
            && py >= self.y
            && py < self.y.saturating_add(self.height as i16)
    }

    pub fn intersects(&self, other: &Rectangle) -> bool {
        let ax1 = self.x;
        let ay1 = self.y;
        let ax2 = self.x.saturating_add(self.width as i16);
        let ay2 = self.y.saturating_add(self.height as i16);
        let bx1 = other.x;
        let by1 = other.y;
        let bx2 = other.x.saturating_add(other.width as i16);
        let by2 = other.y.saturating_add(other.height as i16);

        ax1 < bx2 && ax2 > bx1 && ay1 < by2 && ay2 > by1
    }

    pub fn intersect(&self, other: &Rectangle) -> Option<Rectangle> {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = (self.x.saturating_add(self.width as i16))
            .min(other.x.saturating_add(other.width as i16));
        let y2 = (self.y.saturating_add(self.height as i16))
            .min(other.y.saturating_add(other.height as i16));

        if x1 < x2 && y1 < y2 {
            Some(Rectangle {
                x: x1,
                y: y1,
                width: (x2 - x1) as u16,
                height: (y2 - y1) as u16,
            })
        } else {
            None
        }
    }

    pub fn union(&self, other: &Rectangle) -> Rectangle {
        let x1 = self.x.min(other.x);
        let y1 = self.y.min(other.y);
        let x2 = (self.x.saturating_add(self.width as i16))
            .max(other.x.saturating_add(other.width as i16));
        let y2 = (self.y.saturating_add(self.height as i16))
            .max(other.y.saturating_add(other.height as i16));

        Rectangle {
            x: x1,
            y: y1,
            width: (x2.saturating_sub(x1)) as u16,
            height: (y2.saturating_sub(y1)) as u16,
        }
    }

    pub fn right(&self) -> i16 {
        self.x.saturating_add(self.width as i16)
    }

    pub fn bottom(&self) -> i16 {
        self.y.saturating_add(self.height as i16)
    }

    pub fn center(&self) -> Point {
        Point::new(
            self.x.saturating_add((self.width / 2) as i16),
            self.y.saturating_add((self.height / 2) as i16),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn area(&self) -> u32 {
        self.width as u32 * self.height as u32
    }

    pub fn translate(&self, dx: i16, dy: i16) -> Rectangle {
        Rectangle {
            x: self.x.saturating_add(dx),
            y: self.y.saturating_add(dy),
            width: self.width,
            height: self.height,
        }
    }
}

impl From<(i16, i16, u16, u16)> for Rectangle {
    fn from((x, y, w, h): (i16, i16, u16, u16)) -> Self {
        Self { x, y, width: w, height: h }
    }
}
