//! Pixel-space rectangles. Everything downstream of the layout engine
//! works in physical pixels of the target display.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// Construct from origin and size.
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    /// Shrink by `d` on every side (grow if negative).
    pub fn inset(self, d: f32) -> Self {
        Self::new(self.x + d, self.y + d, self.w - 2.0 * d, self.h - 2.0 * d)
    }
    /// Grow by `d` on every side (a negative inset).
    pub fn expand(self, d: f32) -> Self {
        self.inset(-d)
    }
    /// X coordinate of the right edge.
    pub fn right(self) -> f32 {
        self.x + self.w
    }
    /// Y coordinate of the bottom edge.
    pub fn bottom(self) -> f32 {
        self.y + self.h
    }
    /// Midpoint of the rectangle.
    pub fn center(self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
    /// A sub-rectangle of the given size, centered inside `self`.
    pub fn centered_sub(self, w: f32, h: f32) -> Self {
        Self::new(
            self.x + (self.w - w) * 0.5,
            self.y + (self.h - h) * 0.5,
            w,
            h,
        )
    }
    /// True when the point lies inside (right/bottom edges exclusive).
    pub fn contains(self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }
}
