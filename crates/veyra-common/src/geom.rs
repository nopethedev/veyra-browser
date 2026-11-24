//! Geometry primitives. All layout values are `f32` CSS pixels (device
//! independent); the rasteriser scales at paint time.

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

impl Size {
    pub const ZERO: Size = Size { w: 0.0, h: 0.0 };
    pub fn new(w: f32, h: f32) -> Self {
        Size { w, h }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Point = Point { x: 0.0, y: 0.0 };
    pub fn new(x: f32, y: f32) -> Self {
        Point { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const ZERO: Rect = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };

    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w: w.max(0.0), h: h.max(0.0) }
    }

    pub fn from_size(x: f32, y: f32, s: Size) -> Self {
        Rect::new(x, y, s.w, s.h)
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn origin(&self) -> Point {
        Point::new(self.x, self.y)
    }
    pub fn size(&self) -> Size {
        Size::new(self.w, self.h)
    }
    pub fn center(&self) -> Point {
        Point::new(self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
    pub fn intersection(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = self.right().max(o.right());
        let y1 = self.bottom().max(o.bottom());
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }
    pub fn inset(&self, e: &Edges) -> Rect {
        Rect::new(
            self.x + e.left,
            self.y + e.top,
            self.w - e.left - e.right,
            self.h - e.top - e.bottom,
        )
    }
    pub fn translated(&self, dx: f32, dy: f32) -> Rect {
        Rect { x: self.x + dx, y: self.y + dy, ..*self }
    }
    pub fn round_to_pixels(&self) -> Rect {
        let x = self.x.floor();
        let y = self.y.floor();
        Rect::new(x, y, (self.right().ceil() - x).max(0.0), (self.bottom().ceil() - y).max(0.0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Edges {
    pub const ZERO: Edges = Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 };
    pub fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Edges { top, right, bottom, left }
    }
    pub fn all(v: f32) -> Self {
        Edges { top: v, right: v, bottom: v, left: v }
    }
    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }
    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }
    pub fn is_zero(&self) -> bool {
        self.top == 0.0 && self.right == 0.0 && self.bottom == 0.0 && self.left == 0.0
    }
}

/// Clamp helper shared by layout code.
pub fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v.is_nan() {
        lo
    } else if hi < lo {
        lo
    } else {
        v.max(lo).min(hi)
    }
}

/// Finite-or-default, used to defend layout against NaN/Infinity from CSS.
pub fn finite(v: f32, default: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        default
    }
}
