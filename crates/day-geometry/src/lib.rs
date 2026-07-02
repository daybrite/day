//! day-geometry — plain `Copy` value types shared by layout, canvas, and the toolkit spec.
//! Everything is in points (density-independent); backends convert to device pixels (§7.9).

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const ZERO: Point = Point { x: 0.0, y: 0.0 };
    #[inline]
    pub const fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }
    #[inline]
    pub fn offset(self, dx: f64, dy: f64) -> Self {
        Point::new(self.x + dx, self.y + dy)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

impl Size {
    pub const ZERO: Size = Size { width: 0.0, height: 0.0 };
    #[inline]
    pub const fn new(width: f64, height: f64) -> Self {
        Size { width, height }
    }
    #[inline]
    pub fn max(self, other: Size) -> Size {
        Size::new(self.width.max(other.width), self.height.max(other.height))
    }
    /// Approximate equality on the half-pixel epsilon used by frame diffing (§7.9).
    #[inline]
    pub fn approx_eq(self, other: Size, eps: f64) -> bool {
        (self.width - other.width).abs() <= eps && (self.height - other.height).abs() <= eps
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    pub const ZERO: Rect = Rect { origin: Point::ZERO, size: Size::ZERO };
    #[inline]
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Rect { origin: Point::new(x, y), size: Size::new(width, height) }
    }
    #[inline]
    pub fn from_size(size: Size) -> Self {
        Rect { origin: Point::ZERO, size }
    }
    #[inline]
    pub fn min_x(&self) -> f64 {
        self.origin.x
    }
    #[inline]
    pub fn min_y(&self) -> f64 {
        self.origin.y
    }
    #[inline]
    pub fn max_x(&self) -> f64 {
        self.origin.x + self.size.width
    }
    #[inline]
    pub fn max_y(&self) -> f64 {
        self.origin.y + self.size.height
    }
    #[inline]
    pub fn center(&self) -> Point {
        Point::new(self.origin.x + self.size.width / 2.0, self.origin.y + self.size.height / 2.0)
    }
    #[inline]
    pub fn inset(&self, d: f64) -> Rect {
        self.inset_by(Insets::all(d))
    }
    pub fn inset_by(&self, i: Insets) -> Rect {
        Rect::new(
            self.origin.x + i.leading,
            self.origin.y + i.top,
            (self.size.width - i.leading - i.trailing).max(0.0),
            (self.size.height - i.top - i.bottom).max(0.0),
        )
    }
    pub fn intersects(&self, other: &Rect) -> bool {
        self.min_x() < other.max_x()
            && other.min_x() < self.max_x()
            && self.min_y() < other.max_y()
            && other.min_y() < self.max_y()
    }
    /// Approximate equality on the half-pixel epsilon used by frame diffing (§7.9).
    pub fn approx_eq(&self, other: &Rect, eps: f64) -> bool {
        (self.origin.x - other.origin.x).abs() <= eps
            && (self.origin.y - other.origin.y).abs() <= eps
            && self.size.approx_eq(other.size, eps)
    }
}

/// Logical insets: `leading`/`trailing` resolve against the layout direction at place time (§7.8).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub top: f64,
    pub leading: f64,
    pub bottom: f64,
    pub trailing: f64,
}

impl Insets {
    pub const ZERO: Insets = Insets { top: 0.0, leading: 0.0, bottom: 0.0, trailing: 0.0 };
    #[inline]
    pub const fn all(d: f64) -> Self {
        Insets { top: d, leading: d, bottom: d, trailing: d }
    }
    #[inline]
    pub const fn symmetric(horizontal: f64, vertical: f64) -> Self {
        Insets { top: vertical, leading: horizontal, bottom: vertical, trailing: horizontal }
    }
    #[inline]
    pub fn horizontal(&self) -> f64 {
        self.leading + self.trailing
    }
    #[inline]
    pub fn vertical(&self) -> f64 {
        self.top + self.bottom
    }
}

/// sRGB color, 0.0–1.0 components. Semantic theme tokens (§6.3) resolve to these in the backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Color {
    pub const fn rgba(r: f64, g: f64, b: f64, a: f64) -> Self {
        Color { r, g, b, a }
    }
    pub const fn rgb(r: f64, g: f64, b: f64) -> Self {
        Color::rgba(r, g, b, 1.0)
    }
    pub const BLACK: Color = Color::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Color = Color::rgb(1.0, 1.0, 1.0);
    pub const CLEAR: Color = Color::rgba(0.0, 0.0, 0.0, 0.0);
    /// 0xRRGGBB
    pub const fn hex(v: u32) -> Self {
        Color::rgb(
            ((v >> 16) & 0xff) as f64 / 255.0,
            ((v >> 8) & 0xff) as f64 / 255.0,
            (v & 0xff) as f64 / 255.0,
        )
    }
}

/// The layout proposal: `None` = unconstrained on that axis (§7.2).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Proposal {
    pub width: Option<f64>,
    pub height: Option<f64>,
}

impl Proposal {
    pub const UNCONSTRAINED: Proposal = Proposal { width: None, height: None };
    #[inline]
    pub const fn new(width: Option<f64>, height: Option<f64>) -> Self {
        Proposal { width, height }
    }
    #[inline]
    pub const fn exact(size: Size) -> Self {
        Proposal { width: Some(size.width), height: Some(size.height) }
    }
    /// Quantized key for the measurement cache (§7.4): tenth-of-a-point buckets.
    pub fn cache_key(&self) -> (u64, u64) {
        #[inline]
        fn q(v: Option<f64>) -> u64 {
            match v {
                None => u64::MAX,
                Some(f) => (f * 10.0).round().max(0.0) as u64,
            }
        }
        (q(self.width), q(self.height))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutDirection {
    #[default]
    Ltr,
    Rtl,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_insets() {
        let r = Rect::new(10.0, 10.0, 100.0, 50.0).inset_by(Insets::symmetric(4.0, 2.0));
        assert_eq!(r, Rect::new(14.0, 12.0, 92.0, 46.0));
    }

    #[test]
    fn proposal_cache_key_quantizes() {
        assert_eq!(
            Proposal::new(Some(100.02), None).cache_key(),
            Proposal::new(Some(100.04), None).cache_key()
        );
        assert_ne!(
            Proposal::new(Some(100.0), None).cache_key(),
            Proposal::UNCONSTRAINED.cache_key()
        );
    }
}
