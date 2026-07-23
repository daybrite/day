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
    pub const ZERO: Size = Size {
        width: 0.0,
        height: 0.0,
    };
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
    pub const ZERO: Rect = Rect {
        origin: Point::ZERO,
        size: Size::ZERO,
    };
    #[inline]
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Rect {
            origin: Point::new(x, y),
            size: Size::new(width, height),
        }
    }
    #[inline]
    pub fn from_size(size: Size) -> Self {
        Rect {
            origin: Point::ZERO,
            size,
        }
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
        Point::new(
            self.origin.x + self.size.width / 2.0,
            self.origin.y + self.size.height / 2.0,
        )
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

/// A 2-D affine transform (CoreGraphics row-vector convention): a point `p` maps to
/// `(a·p.x + c·p.y + tx, b·p.x + d·p.y + ty)`. Used by canvas transform ops for shape
/// rotate/scale/offset — every native 2-D context concatenates it onto its CTM identically.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub tx: f64,
    pub ty: f64,
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    #[inline]
    pub const fn translate(x: f64, y: f64) -> Affine {
        Affine {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: x,
            ty: y,
        }
    }
    #[inline]
    pub const fn scale(sx: f64, sy: f64) -> Affine {
        Affine {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            tx: 0.0,
            ty: 0.0,
        }
    }
    /// Rotation by `radians` (counter-clockwise in a y-up space; clockwise on y-down screens).
    #[inline]
    pub fn rotate(radians: f64) -> Affine {
        let (s, cos) = radians.sin_cos();
        Affine {
            a: cos,
            b: s,
            c: -s,
            d: cos,
            tx: 0.0,
            ty: 0.0,
        }
    }
    /// `self` applied first, then `other` (row-vector product `self · other`).
    #[inline]
    pub fn then(self, o: Affine) -> Affine {
        Affine {
            a: self.a * o.a + self.b * o.c,
            b: self.a * o.b + self.b * o.d,
            c: self.c * o.a + self.d * o.c,
            d: self.c * o.b + self.d * o.d,
            tx: self.tx * o.a + self.ty * o.c + o.tx,
            ty: self.tx * o.b + self.ty * o.d + o.ty,
        }
    }
    #[inline]
    pub fn apply(&self, p: Point) -> Point {
        Point::new(
            self.a * p.x + self.c * p.y + self.tx,
            self.b * p.x + self.d * p.y + self.ty,
        )
    }
    /// Map a point back through the inverse (for hit-testing a transformed shape). None if singular.
    pub fn invert_apply(&self, p: Point) -> Option<Point> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-12 {
            return None;
        }
        let inv = 1.0 / det;
        let x = p.x - self.tx;
        let y = p.y - self.ty;
        Some(Point::new(
            (x * self.d - y * self.c) * inv,
            (y * self.a - x * self.b) * inv,
        ))
    }
    #[inline]
    pub fn is_identity(&self) -> bool {
        *self == Affine::IDENTITY
    }
    #[inline]
    pub fn as_array(&self) -> [f64; 6] {
        [self.a, self.b, self.c, self.d, self.tx, self.ty]
    }
    #[inline]
    pub fn from_array(m: [f64; 6]) -> Affine {
        Affine {
            a: m[0],
            b: m[1],
            c: m[2],
            d: m[3],
            tx: m[4],
            ty: m[5],
        }
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
    pub const ZERO: Insets = Insets {
        top: 0.0,
        leading: 0.0,
        bottom: 0.0,
        trailing: 0.0,
    };
    #[inline]
    pub const fn all(d: f64) -> Self {
        Insets {
            top: d,
            leading: d,
            bottom: d,
            trailing: d,
        }
    }
    #[inline]
    pub const fn symmetric(horizontal: f64, vertical: f64) -> Self {
        Insets {
            top: vertical,
            leading: horizontal,
            bottom: vertical,
            trailing: horizontal,
        }
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

    /// From HSL — `h` in degrees (wraps mod 360), `s`/`l` in `0.0..=1.0`. `Color` is the one color
    /// type every parameter accepts, so this makes HSL usable everywhere a color is.
    pub fn hsl(h: f64, s: f64, l: f64) -> Self {
        Color::hsla(h, s, l, 1.0)
    }
    pub fn hsla(h: f64, s: f64, l: f64, a: f64) -> Self {
        let (s, l) = (s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
        if s == 0.0 {
            return Color::rgba(l, l, l, a);
        }
        let hk = h.rem_euclid(360.0) / 360.0;
        let q = if l < 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };
        let p = 2.0 * l - q;
        Color::rgba(
            hue2rgb(p, q, hk + 1.0 / 3.0),
            hue2rgb(p, q, hk),
            hue2rgb(p, q, hk - 1.0 / 3.0),
            a,
        )
    }

    /// From HSV/HSB — `h` degrees (wraps), `s`/`v` in `0.0..=1.0`.
    pub fn hsv(h: f64, s: f64, v: f64) -> Self {
        Color::hsva(h, s, v, 1.0)
    }
    pub fn hsva(h: f64, s: f64, v: f64, a: f64) -> Self {
        let (s, v) = (s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
        let hh = h.rem_euclid(360.0) / 60.0;
        let c = v * s;
        let x = c * (1.0 - (hh % 2.0 - 1.0).abs());
        let m = v - c;
        let (r, g, b) = match hh as i32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        Color::rgba(r + m, g + m, b + m, a)
    }

    /// Decompose to `(hue°, saturation, lightness)` (HSL). Hue is `0.0` for greys.
    pub fn to_hsl(&self) -> (f64, f64, f64) {
        let (r, g, b) = (self.r, self.g, self.b);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;
        let d = max - min;
        if d.abs() < 1e-9 {
            return (0.0, 0.0, l);
        }
        let s = d / (1.0 - (2.0 * l - 1.0).abs());
        let h = if max == r {
            60.0 * (((g - b) / d).rem_euclid(6.0))
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        (h.rem_euclid(360.0), s.clamp(0.0, 1.0), l)
    }

    /// Interpolate toward `to` in HSL space, taking the shortest hue arc (`t` in `0.0..=1.0`). A
    /// hue-space blend (red→green sweeps through yellow) rather than the muddy RGB straight line —
    /// used by the canvas / self-driven animation path (native widget color animation interpolates
    /// in the toolkit's own space).
    pub fn lerp_hsl(self, to: Color, t: f64) -> Color {
        let (h0, s0, l0) = self.to_hsl();
        let (h1, s1, l1) = to.to_hsl();
        let mut dh = (h1 - h0).rem_euclid(360.0);
        if dh > 180.0 {
            dh -= 360.0;
        }
        let lerp = |a: f64, b: f64| a + (b - a) * t;
        Color::hsla(h0 + dh * t, lerp(s0, s1), lerp(l0, l1), lerp(self.a, to.a))
    }
}

/// HSL hue channel → RGB component (helper for [`Color::hsla`]).
fn hue2rgb(p: f64, q: f64, t: f64) -> f64 {
    let t = t.rem_euclid(1.0);
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 1.0 / 2.0 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

/// The layout proposal: `None` = unconstrained on that axis (§7.2).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Proposal {
    pub width: Option<f64>,
    pub height: Option<f64>,
}

impl Proposal {
    pub const UNCONSTRAINED: Proposal = Proposal {
        width: None,
        height: None,
    };
    #[inline]
    pub const fn new(width: Option<f64>, height: Option<f64>) -> Self {
        Proposal { width, height }
    }
    #[inline]
    pub const fn exact(size: Size) -> Self {
        Proposal {
            width: Some(size.width),
            height: Some(size.height),
        }
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

/// A cheap per-node visual transform (§8.4 animation): translation, uniform/non-uniform scale, and
/// rotation about a unit anchor (`0.0..1.0` within the node's bounds; default center). Distinct
/// from the layout frame — animating a `Transform` never triggers relayout, so it is the vehicle
/// for movement/scaling animation. Each backend composes it onto the node's laid-out frame via its
/// native transform channel (CALayer/GskTransform/RenderTransform/NODE_TRANSFORM/…).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub tx: f64,
    pub ty: f64,
    pub sx: f64,
    pub sy: f64,
    pub rotate_deg: f64,
    /// Anchor for scale/rotation as a unit fraction of the node's bounds (`0.5,0.5` = center).
    pub anchor_x: f64,
    pub anchor_y: f64,
}

impl Default for Transform {
    fn default() -> Self {
        Transform::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        tx: 0.0,
        ty: 0.0,
        sx: 1.0,
        sy: 1.0,
        rotate_deg: 0.0,
        anchor_x: 0.5,
        anchor_y: 0.5,
    };

    #[inline]
    pub const fn translate(tx: f64, ty: f64) -> Transform {
        Transform {
            tx,
            ty,
            ..Transform::IDENTITY
        }
    }
    #[inline]
    pub const fn scale(sx: f64, sy: f64) -> Transform {
        Transform {
            sx,
            sy,
            ..Transform::IDENTITY
        }
    }
    #[inline]
    pub const fn rotate(deg: f64) -> Transform {
        Transform {
            rotate_deg: deg,
            ..Transform::IDENTITY
        }
    }
    /// Whether this transform has no visual effect — backends skip applying it.
    #[inline]
    pub fn is_identity(&self) -> bool {
        *self == Transform::IDENTITY
    }
}

/// Linear interpolation of animatable values (`t` in `0.0..1.0`). This drives the **canvas /
/// self-driven** animation path (docs/shapes.md §5) and Qt's sampled spring; native-widget
/// animation does NOT use it — the toolkit interpolates on its own compositor.
pub trait Animatable: Copy {
    fn lerp(self, to: Self, t: f64) -> Self;
}

#[inline]
fn flerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

impl Animatable for f64 {
    #[inline]
    fn lerp(self, to: f64, t: f64) -> f64 {
        flerp(self, to, t)
    }
}
impl Animatable for Point {
    #[inline]
    fn lerp(self, to: Point, t: f64) -> Point {
        Point::new(flerp(self.x, to.x, t), flerp(self.y, to.y, t))
    }
}
impl Animatable for Size {
    #[inline]
    fn lerp(self, to: Size, t: f64) -> Size {
        Size::new(
            flerp(self.width, to.width, t),
            flerp(self.height, to.height, t),
        )
    }
}
impl Animatable for Rect {
    #[inline]
    fn lerp(self, to: Rect, t: f64) -> Rect {
        Rect {
            origin: self.origin.lerp(to.origin, t),
            size: self.size.lerp(to.size, t),
        }
    }
}
impl Animatable for Color {
    #[inline]
    fn lerp(self, to: Color, t: f64) -> Color {
        Color::rgba(
            flerp(self.r, to.r, t),
            flerp(self.g, to.g, t),
            flerp(self.b, to.b, t),
            flerp(self.a, to.a, t),
        )
    }
}
impl Animatable for Transform {
    #[inline]
    fn lerp(self, to: Transform, t: f64) -> Transform {
        // Anchor snaps to the destination's (it's a coordinate frame, not a visual value).
        Transform {
            tx: flerp(self.tx, to.tx, t),
            ty: flerp(self.ty, to.ty, t),
            sx: flerp(self.sx, to.sx, t),
            sy: flerp(self.sy, to.sy, t),
            rotate_deg: flerp(self.rotate_deg, to.rotate_deg, t),
            anchor_x: to.anchor_x,
            anchor_y: to.anchor_y,
        }
    }
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
    fn hsl_primaries_and_roundtrip() {
        let approx = |a: Color, b: Color| {
            (a.r - b.r).abs() < 1e-6 && (a.g - b.g).abs() < 1e-6 && (a.b - b.b).abs() < 1e-6
        };
        assert!(approx(Color::hsl(0.0, 1.0, 0.5), Color::rgb(1.0, 0.0, 0.0))); // red
        assert!(approx(
            Color::hsl(120.0, 1.0, 0.5),
            Color::rgb(0.0, 1.0, 0.0)
        )); // green
        assert!(approx(
            Color::hsl(240.0, 1.0, 0.5),
            Color::rgb(0.0, 0.0, 1.0)
        )); // blue
        assert!(approx(Color::hsl(0.0, 0.0, 0.5), Color::rgb(0.5, 0.5, 0.5))); // grey
        assert!(approx(Color::hsv(0.0, 1.0, 1.0), Color::rgb(1.0, 0.0, 0.0))); // hsv red
        // Hue wraps, and to_hsl inverts hsl.
        let (h, s, l) = Color::hsl(370.0, 0.6, 0.4).to_hsl();
        assert!((h - 10.0).abs() < 1e-3 && (s - 0.6).abs() < 1e-3 && (l - 0.4).abs() < 1e-3);
        // Shortest-arc hue lerp red→(hue 300, magenta) goes the short way (down through 330), not
        // through green; midpoint hue ≈ 330.
        let mid = Color::hsl(0.0, 1.0, 0.5).lerp_hsl(Color::hsl(300.0, 1.0, 0.5), 0.5);
        assert!((mid.to_hsl().0 - 330.0).abs() < 1.0);
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
