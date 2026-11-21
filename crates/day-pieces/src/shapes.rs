// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! High-level vector shapes atop the canvas display list: `rectangle`, `circle`, `ellipse`,
//! `capsule`, `rounded_rectangle`, `arc`, `line`, and `polygon`, combined with `shape_group`.
//! Reactive fill/stroke, rotation/scale/translation transforms, and tap/drag gestures.

use std::rc::Rc;

use day_core::*;
use day_spec::{Angle, Event, Ink, Paint, Point, Rect, Shape, Size, UnitPoint};

use crate::*;

// ---------------------------------------------------------------------------
// Shapes (docs/shapes.md): high-level shape pieces atop the canvas display list. Frame-relative
// geometry, reactive fill/stroke, rotation/scale/translation transforms, and tap/drag gestures.
// ---------------------------------------------------------------------------

use day_geometry::Affine;
pub use day_spec::{DragPhase, GestureKind};

/// A shape's geometry, resolved against the rect layout assigns it (frame-relative, SwiftUI-style).
#[derive(Clone, Debug, PartialEq)]
pub enum ShapeKind {
    Rectangle,
    RoundedRectangle {
        corner: Corner,
    },
    Circle,
    Ellipse,
    Capsule,
    /// A stroked arc of the inscribed ellipse; `start` from +x, both positive clockwise.
    Arc {
        start: Angle,
        sweep: Angle,
    },
    /// A stroked segment between two unit points of the resolved rect (stroke-only; fills are
    /// ignored). Unit points resolve unclamped, like every unit-space resolve.
    Line {
        from: UnitPoint,
        to: UnitPoint,
    },
    /// A filled/stroked polygon of unit points resolved against the rect (unclamped: points may
    /// sit outside 0..1).
    Polygon {
        points: Rc<[UnitPoint]>,
    },
}

/// A corner radius, for a shape (`rounded_rectangle(..)`) and a view (`.corner_radius(..)`,
/// [`Surface::corner_radius`]) alike: absolute points, or a `0..=1` fraction of half the
/// shorter side, which follows the frame as it resizes. `f64` and [`Radius`] convert.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Corner {
    Fixed(f64),
    Fraction(f64),
}
impl From<f64> for Corner {
    fn from(v: f64) -> Self {
        Corner::Fixed(v)
    }
}
impl From<Radius> for Corner {
    fn from(r: Radius) -> Self {
        Corner::Fixed(r.points())
    }
}
impl Default for Corner {
    /// Square.
    fn default() -> Self {
        Corner::Fixed(0.0)
    }
}
impl Corner {
    /// Round ends: half the shorter side, a capsule or a circle.
    pub const CAPSULE: Corner = Corner::Fraction(1.0);

    /// The radius in points in a frame of `size`: at most half the shorter side.
    #[must_use]
    pub fn points(self, size: Size) -> f64 {
        let cap = (size.width.min(size.height) / 2.0).max(0.0);
        match self {
            Corner::Fixed(v) => v.clamp(0.0, cap),
            Corner::Fraction(f) => f.clamp(0.0, 1.0) * cap,
        }
    }
    /// Whether the radius depends on the frame.
    #[must_use]
    pub fn is_relative(self) -> bool {
        matches!(self, Corner::Fraction(_))
    }
    fn resolve(self, rect: Rect) -> f64 {
        self.points(rect.size)
    }
}
impl ShapeKind {
    /// Lower to a drawable geometry within `rect`.
    fn geometry(self, rect: Rect) -> Shape {
        match self {
            ShapeKind::Rectangle => Shape::Rect(rect),
            ShapeKind::RoundedRectangle { corner } => {
                Shape::RoundedRect(rect, corner.resolve(rect))
            }
            ShapeKind::Ellipse => Shape::Ellipse(rect),
            ShapeKind::Capsule => {
                Shape::RoundedRect(rect, rect.size.width.min(rect.size.height) / 2.0)
            }
            ShapeKind::Circle => {
                let d = rect.size.width.min(rect.size.height);
                let c = rect.center();
                Shape::Ellipse(Rect::new(c.x - d / 2.0, c.y - d / 2.0, d, d))
            }
            ShapeKind::Arc { start, sweep } => Shape::Arc { rect, start, sweep },
            ShapeKind::Line { from, to } => Shape::Line(from.resolve(rect), to.resolve(rect)),
            ShapeKind::Polygon { points } => {
                Shape::Polygon(points.iter().map(|p| p.resolve(rect)).collect())
            }
        }
    }
    /// Point-in-shape test (in the shape's own, untransformed coordinates).
    fn contains(self, rect: Rect, p: Point) -> bool {
        fn in_rect(r: Rect, p: Point) -> bool {
            p.x >= r.min_x() && p.x <= r.max_x() && p.y >= r.min_y() && p.y <= r.max_y()
        }
        fn in_ellipse(r: Rect, p: Point) -> bool {
            if r.size.width <= 0.0 || r.size.height <= 0.0 {
                return false;
            }
            let c = r.center();
            let dx = (p.x - c.x) / (r.size.width / 2.0);
            let dy = (p.y - c.y) / (r.size.height / 2.0);
            dx * dx + dy * dy <= 1.0
        }
        /// Even-odd ray cast: count edge crossings of the +x ray from `p`.
        fn in_polygon(pts: &[Point], p: Point) -> bool {
            if pts.len() < 3 {
                return false;
            }
            let mut inside = false;
            let mut j = pts.len() - 1;
            for i in 0..pts.len() {
                let (a, b) = (pts[i], pts[j]);
                if (a.y > p.y) != (b.y > p.y) {
                    let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
                    if p.x < x {
                        inside = !inside;
                    }
                }
                j = i;
            }
            inside
        }
        match self.geometry(rect) {
            Shape::Ellipse(r) => in_ellipse(r, p),
            Shape::Rect(r) | Shape::RoundedRect(r, _) => in_rect(r, p),
            Shape::Polygon(pts) => in_polygon(&pts, p),
            _ => in_rect(rect, p), // arc / line: bounding-box fallback
        }
    }
}

/// Drag info delivered to an `.on_drag` handler (docs/gestures.md "Gestures").
///
/// `translation` is cumulative from where the pointer went down; `location` is where it is
/// now, in the piece's own coordinates. The two are not redundant, because `Began` does not
/// always arrive at the press: a backend whose recognizer has to watch a few events before it
/// calls the gesture a drag (appkit raises `Began` on the first drag event) reports where it
/// recognized the drag, with the translation from the press already applied. So the press is
/// [`Drag::start`] (`location - translation`) on every backend and in every phase; take the
/// anchor from there rather than from `Began`'s `location`, or a gesture measured from the
/// anchor trails the pointer by however far it moved before the recognizer engaged.
///
/// `velocity` is how fast the pointer is moving, in points per second: at `Ended` it is the
/// release velocity a fling or a spring starts from.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Drag {
    pub phase: DragPhase,
    pub location: Point,
    pub translation: Point,
    /// Points per second along the piece's axes. The platform recognizer's own measurement
    /// at the release where it has one, Day's estimate from the last tenth of a second of
    /// movement otherwise; zero once the pointer has rested that long.
    pub velocity: Point,
}

impl Drag {
    /// A drag event with no velocity; [`Drag::with_velocity`] adds one.
    #[must_use]
    pub fn new(phase: DragPhase, location: Point, translation: Point) -> Drag {
        Drag {
            phase,
            location,
            translation,
            velocity: Point::ZERO,
        }
    }

    /// This event with a velocity, in points per second.
    #[must_use]
    pub fn with_velocity(mut self, velocity: Point) -> Drag {
        self.velocity = velocity;
        self
    }

    /// Where the pointer went down, in the piece's coordinates: `location - translation`.
    #[must_use]
    pub fn start(&self) -> Point {
        Point::new(
            self.location.x - self.translation.x,
            self.location.y - self.translation.y,
        )
    }

    /// The release speed, in points per second: the length of [`Drag::velocity`].
    #[must_use]
    pub fn speed(&self) -> f64 {
        self.velocity.x.hypot(self.velocity.y)
    }
}

/// One pinch-gesture event, delivered to a `.on_pinch(…)` handler (docs/shapes.md). `scale`
/// is cumulative since `Began` (1.0 = unchanged); `location` is the centroid, the anchor a
/// zoom keeps stationary.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Pinch {
    pub phase: DragPhase,
    pub scale: f64,
    pub location: Point,
}

impl Pinch {
    /// A pinch event.
    #[must_use]
    pub fn new(phase: DragPhase, scale: f64, location: Point) -> Pinch {
        Pinch {
            phase,
            scale,
            location,
        }
    }
}

/// One rotation-gesture event, delivered to a `.on_rotate(…)` handler (docs/canvas.md "Zoom
/// and pan").
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Rotate {
    pub phase: DragPhase,
    /// Cumulative since `Began` for a two-finger gesture; a trackpad tick is a lone `Changed`
    /// carrying that tick's own angle. Positive is clockwise on screen.
    pub rotation: Angle,
    /// The gesture's centroid in canvas coordinates (`Point::ZERO` where unknown, as Pinch does).
    pub location: Point,
}

impl Rotate {
    /// A rotation event.
    #[must_use]
    pub fn new(phase: DragPhase, rotation: Angle, location: Point) -> Rotate {
        Rotate {
            phase,
            rotation,
            location,
        }
    }
}

/// One pan-gesture event, delivered to a `.on_pan(…)` handler (docs/shapes.md). `delta` is
/// the movement since the previous event, so apply it incrementally; a discrete wheel tick
/// arrives as a lone `Changed`. `velocity` is how fast the pan is moving, in points per
/// second, measured the way [`Drag::velocity`] is.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Pan {
    pub phase: DragPhase,
    pub delta: Point,
    pub location: Point,
    /// Points per second; see [`Drag::velocity`].
    pub velocity: Point,
}

impl Pan {
    /// A pan event with no velocity; [`Pan::with_velocity`] adds one.
    #[must_use]
    pub fn new(phase: DragPhase, delta: Point, location: Point) -> Pan {
        Pan {
            phase,
            delta,
            location,
            velocity: Point::ZERO,
        }
    }

    /// This event with a velocity, in points per second.
    #[must_use]
    pub fn with_velocity(mut self, velocity: Point) -> Pan {
        self.velocity = velocity;
        self
    }
}

/// The drawable description of a shape: everything but gestures. Cloneable so [`shape_group`]
/// can collect many descriptions into one canvas closure (docs/shapes.md §3.6).
#[derive(Clone)]
struct ShapeSpec {
    kind: Reactive<ShapeKind>,
    fill: Option<Reactive<Paint>>,
    stroke: Option<(Reactive<Ink>, Reactive<f64>)>,
    inset: Reactive<f64>,
    rotation: Reactive<Angle>,
    scale: Reactive<f64>,
    translation: (Reactive<f64>, Reactive<f64>),
    /// Unit-space sub-rect of the bounds this shape resolves in (`.at`).
    at: Option<Rect>,
}

/// A shape piece: one data-oriented piece parameterized by `ShapeKind`, rendered atop the canvas.
#[must_use]
pub struct ShapePiece {
    spec: ShapeSpec,
    on_tap: Option<Rc<dyn Fn()>>,
    on_drag: Option<Rc<dyn Fn(Drag)>>,
}

/// The unified constructor: `shape(ShapeKind::RoundedRectangle { corner: 12.0.into() })`.
pub fn shape<M>(kind: impl IntoReactive<ShapeKind, M>) -> ShapePiece {
    ShapePiece {
        spec: ShapeSpec {
            kind: kind.into_reactive(),
            fill: None,
            stroke: None,
            inset: Reactive::Const(0.0),
            rotation: Reactive::Const(Angle::ZERO),
            scale: Reactive::Const(1.0),
            translation: (Reactive::Const(0.0), Reactive::Const(0.0)),
            at: None,
        },
        on_tap: None,
        on_drag: None,
    }
}
/// SwiftUI-ergonomic sugar; all build the same `ShapePiece`.
pub fn rectangle() -> ShapePiece {
    shape(ShapeKind::Rectangle)
}
pub fn circle() -> ShapePiece {
    shape(ShapeKind::Circle)
}
pub fn ellipse() -> ShapePiece {
    shape(ShapeKind::Ellipse)
}
pub fn capsule() -> ShapePiece {
    shape(ShapeKind::Capsule)
}
pub fn rounded_rectangle(corner: impl Into<Corner>) -> ShapePiece {
    shape(ShapeKind::RoundedRectangle {
        corner: corner.into(),
    })
}
/// A stroked arc of the frame's inscribed ellipse: `arc(Angle::deg(-90.0), Angle::deg(270.0))`.
pub fn arc(start: Angle, sweep: Angle) -> ShapePiece {
    shape(ShapeKind::Arc { start, sweep })
}
/// A stroked segment between two unit points of the frame: `line((0.16, 0.5), (0.84, 0.5))`.
pub fn line(from: (f64, f64), to: (f64, f64)) -> ShapePiece {
    shape(ShapeKind::Line {
        from: UnitPoint::new(from.0, from.1),
        to: UnitPoint::new(to.0, to.1),
    })
}
/// A polygon of unit points of the frame: `polygon([(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)])`.
pub fn polygon(points: impl IntoIterator<Item = (f64, f64)>) -> ShapePiece {
    shape(ShapeKind::Polygon {
        points: points
            .into_iter()
            .map(|(x, y)| UnitPoint::new(x, y))
            .collect(),
    })
}

#[day_macros::builder(ShapePieceBuilder)]
impl ShapePiece {
    /// Fill with any [`Paint`]: a [`Color`](day_spec::Color), a
    /// [`SystemColor`](day_spec::SystemColor), an [`Ink`], or a linear, radial or conic
    /// gradient whose unit points resolve against the shape's bounds. Reactive like every
    /// other property: `rectangle().fill(move || sky_gradient(state.get()))`.
    pub fn fill<M>(mut self, paint: impl IntoReactive<Paint, M>) -> Self {
        self.spec.fill = Some(paint.into_reactive());
        self
    }
    pub fn stroke<M1, M2>(
        mut self,
        color: impl IntoReactive<Ink, M1>,
        width: impl IntoReactive<f64, M2>,
    ) -> Self {
        self.spec.stroke = Some((color.into_reactive(), width.into_reactive()));
        self
    }
    /// Uniform inset applied before resolving geometry (keeps strokes inside the frame).
    pub fn inset<M>(mut self, v: impl IntoReactive<f64, M>) -> Self {
        self.spec.inset = v.into_reactive();
        self
    }
    /// Rotate the drawn shape clockwise about its center, as [`Decorate::rotation`] turns a
    /// view.
    pub fn rotation<M>(mut self, angle: impl IntoReactive<Angle, M>) -> Self {
        self.spec.rotation = angle.into_reactive();
        self
    }
    /// Scale the drawn shape about its center (uniform), as [`Decorate::scale`] scales a view.
    pub fn scale<M>(mut self, s: impl IntoReactive<f64, M>) -> Self {
        self.spec.scale = s.into_reactive();
        self
    }
    /// Move the drawn shape within its frame by (`x`, `y`) points, as
    /// [`Decorate::translation`] moves a view.
    pub fn translation<M1, M2>(
        mut self,
        x: impl IntoReactive<f64, M1>,
        y: impl IntoReactive<f64, M2>,
    ) -> Self {
        self.spec.translation = (x.into_reactive(), y.into_reactive());
        self
    }
    /// Resolve this shape inside the fractional sub-rect `(fx, fy, fw, fh)` of its bounds
    /// (unit-space, applied before [`Self::inset`]). The workhorse for composing glyphs in a
    /// [`shape_group`], mirroring hand-drawn `Rect::new(ox + fx * s, oy + fy * s, …)` canvas code.
    pub fn at(mut self, fx: f64, fy: f64, fw: f64, fh: f64) -> Self {
        self.spec.at = Some(Rect::new(fx, fy, fw, fh));
        self
    }
    /// Fire when the shape is tapped (path-precise: the tap is tested against the resolved path).
    pub fn on_tap(mut self, f: impl Fn() + 'static) -> Self {
        self.on_tap = Some(Rc::new(f));
        self
    }
    /// Fire on each phase of a drag over the shape.
    pub fn on_drag(mut self, f: impl Fn(Drag) + 'static) -> Self {
        self.on_drag = Some(Rc::new(f));
        self
    }
}

/// Compose the shape's rotation/scale (about its center) + translation into one affine.
fn shape_transform(rect: Rect, rotation: Angle, scale: f64, ox: f64, oy: f64) -> Affine {
    let c = rect.center();
    Affine::translate(-c.x, -c.y)
        .then(Affine::scale(scale, scale))
        .then(Affine::rotate(rotation))
        .then(Affine::translate(c.x, c.y))
        .then(Affine::translate(ox, oy))
}

/// Map an `.at` unit-space sub-rect into `bounds` (identity when unset).
fn resolve_at(at: Option<Rect>, bounds: Rect) -> Rect {
    match at {
        Some(u) => Rect::new(
            bounds.origin.x + u.origin.x * bounds.size.width,
            bounds.origin.y + u.origin.y * bounds.size.height,
            u.size.width * bounds.size.width,
            u.size.height * bounds.size.height,
        ),
        None => bounds,
    }
}

/// Record one shape description into `d`, resolved within `bounds`. Shared by
/// [`ShapePiece`]'s canvas leaf and by [`shape_group`] / [`shape_group_fn`].
fn record_shape(spec: &ShapeSpec, d: &mut Draw, bounds: Rect) {
    let bounds = resolve_at(spec.at, bounds);
    let kind = spec.kind.get();
    // A centered stroke overflows the geometry by half its width; inset closed shapes by w/2 so
    // the whole stroke stays inside the view bounds; backends that clip a canvas to its bounds
    // (Qt/Android/XAML) would otherwise cut the stroke's outer edge. (SwiftUI `strokeBorder`
    // behavior.) Fill-only shapes are unaffected (stroke_half = 0). Line/Polygon are exempt:
    // they resolve exactly at their authored unit points, and a line's rect is legitimately
    // degenerate (zero-height for a horizontal segment), so they skip the empty-rect bail too.
    let open = matches!(kind, ShapeKind::Line { .. } | ShapeKind::Polygon { .. });
    let stroke_half = if open {
        0.0
    } else {
        spec.stroke.as_ref().map_or(0.0, |(_, w)| w.get() / 2.0)
    };
    let rect = bounds.inset(spec.inset.get() + stroke_half);
    if !open && (rect.size.width <= 0.0 || rect.size.height <= 0.0) {
        return;
    }
    let geom = kind.geometry(rect);
    let m = shape_transform(
        rect,
        spec.rotation.get(),
        spec.scale.get(),
        spec.translation.0.get(),
        spec.translation.1.get(),
    );
    let transformed = !m.is_identity();
    if transformed {
        d.save();
        d.concat(m);
    }
    if !matches!(geom, Shape::Line(..))
        && let Some(fill) = &spec.fill
    {
        d.fill(geom.clone(), fill.get());
    }
    if let Some((c, w)) = &spec.stroke {
        d.stroke(geom, c.get(), w.get());
    }
    if transformed {
        d.restore();
    }
}

/// A shape greedily fills its proposed size (SwiftUI semantics).
fn shape_flex() -> Flex {
    Flex {
        grow_w: true,
        grow_h: true,
        ..Default::default()
    }
}

/// Flatten many shape descriptions into one canvas leaf: one native view no matter how many
/// shapes (docs/shapes.md §3.6). Shapes draw in order; reactive properties re-record the group.
/// Child gestures are not wired inside a group, so put `.on_tap` on the group via [`Decorate`].
pub fn shape_group(shapes: impl IntoIterator<Item = ShapePiece>) -> impl Piece {
    let specs: Vec<ShapeSpec> = shapes.into_iter().map(|s| s.spec).collect();
    piece_fn(move |cx| {
        canvas_leaf(cx, shape_flex(), move |d, size| {
            let bounds = Rect::from_size(size);
            for spec in &specs {
                record_shape(spec, d, bounds);
            }
        })
    })
}

/// Size-aware [`shape_group`]: the closure derives the shapes from the laid-out size and re-runs
/// on `FrameChanged`, exactly like [`canvas`], for geometry that depends on the final size
/// (e.g. data mapped along the width).
pub fn shape_group_fn(shapes: impl Fn(Size) -> Vec<ShapePiece> + 'static) -> impl Piece {
    piece_fn(move |cx| {
        canvas_leaf(cx, shape_flex(), move |d, size| {
            let bounds = Rect::from_size(size);
            for piece in shapes(size) {
                record_shape(&piece.spec, d, bounds);
            }
        })
    })
}

impl Piece for ShapePiece {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let ShapePiece {
            spec,
            on_tap,
            on_drag,
        } = self;

        let draw_spec = spec.clone();
        let node = canvas_leaf(cx, shape_flex(), move |d, size| {
            record_shape(&draw_spec, d, Rect::from_size(size));
        });

        // Path-precise tap: inverse-transform the tap location, then test against the resolved
        // geometry.
        if let Some(on_tap) = on_tap {
            with_tree(|t| t.enable_gesture(node, GestureKind::Tap));
            cx.on(node, move |ev| {
                if let Event::Tap(p) = ev
                    && let Some(f) = with_tree(|t| t.node_frame(node))
                {
                    let bounds = resolve_at(spec.at, Rect::from_size(f.size));
                    let rect = bounds.inset(spec.inset.get_untracked());
                    let m = shape_transform(
                        rect,
                        spec.rotation.get_untracked(),
                        spec.scale.get_untracked(),
                        spec.translation.0.get_untracked(),
                        spec.translation.1.get_untracked(),
                    );
                    let local = m.invert_apply(*p).unwrap_or(*p);
                    if spec.kind.get_untracked().contains(rect, local) {
                        on_tap();
                    }
                }
            });
        }
        if let Some(on_drag) = on_drag {
            with_tree(|t| t.enable_gesture(node, GestureKind::Drag));
            cx.on(node, move |ev| {
                if let Event::Drag {
                    phase,
                    location,
                    translation,
                } = ev
                {
                    let velocity =
                        day_core::gesture::velocity(day_core::rnode_to_id(node), GestureKind::Drag);
                    on_drag(Drag::new(*phase, *location, *translation).with_velocity(velocity));
                }
            });
        }
        node
    }
}

// --- Typed builders, forwarded through `Decorated` (docs/api-style.md) ---

// ---------------------------------------------------------------------------
// Conformance cases (docs/testing.md)
// ---------------------------------------------------------------------------

/// The shapes' `#[day::test]` cases, next to their constructors.
#[cfg(feature = "conformance")]
pub(crate) mod conformance {
    use day_core::Piece;
    use day_core::conformance::{Case, Drive};
    use day_spec::{Cap, Color};

    use crate::*;

    /// `shape` on a white ground, so a sample outside it reads white on every theme.
    fn on_white(s: ShapePiece) -> impl Piece {
        zstack((rectangle().fill(Color::WHITE), s))
            .id(day_core::id!(ground))
            .frame(80.0, 80.0)
    }

    /// A filled rectangle fills its frame.
    #[day_macros::test(day_core)]
    fn shape_fill() -> Case {
        Case::new()
            .proves_duty("snapshot_window")
            .proves_duty("prepare_snapshot")
            .proves_cap(Cap::Snapshot)
            .requires(Cap::Snapshot)
            .page(|| on_white(rectangle().fill(Color::rgb(1.0, 0.0, 0.0))))
            .drive(|d: Drive| async move {
                d.sample_pixel("ground", 0.5, 0.5, "#ff0000").await?;
                d.sample_pixel("ground", 0.05, 0.05, "#ff0000").await
            })
    }

    /// A circle fills its middle and leaves its frame's corners to what is below.
    #[day_macros::test(day_core)]
    fn shape_circle() -> Case {
        Case::new()
            .requires(Cap::Snapshot)
            .page(|| on_white(circle().fill(Color::rgb(0.0, 0.0, 1.0))))
            .shot("default")
            .drive(|d: Drive| async move {
                d.sample_pixel("ground", 0.5, 0.5, "#0000ff").await?;
                d.sample_pixel("ground", 0.04, 0.04, "#ffffff").await
            })
    }

    /// A rounded rectangle's corners are cut, its middle filled.
    #[day_macros::test(day_core)]
    fn shape_rounded() -> Case {
        Case::new()
            .requires(Cap::Snapshot)
            .page(|| on_white(rounded_rectangle(30.0).fill(Color::rgb(0.0, 0.0, 1.0))))
            .drive(|d: Drive| async move {
                d.sample_pixel("ground", 0.5, 0.5, "#0000ff").await?;
                d.sample_pixel("ground", 0.03, 0.03, "#ffffff").await?;
                d.sample_pixel("ground", 0.5, 0.03, "#0000ff").await
            })
    }

    /// A shape group places each shape in its own fraction of the frame.
    #[day_macros::test(day_core)]
    fn shape_group_places() -> Case {
        Case::new()
            .requires(Cap::Snapshot)
            .page(|| {
                shape_group([
                    rectangle()
                        .fill(Color::rgb(1.0, 0.0, 0.0))
                        .at(0.0, 0.0, 0.5, 1.0),
                    rectangle()
                        .fill(Color::rgb(0.0, 0.0, 1.0))
                        .at(0.5, 0.0, 0.5, 1.0),
                ])
                .id(day_core::id!(group))
                .frame(80.0, 40.0)
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("group", 0.25, 0.5, "#ff0000").await?;
                d.sample_pixel("group", 0.75, 0.5, "#0000ff").await
            })
    }
}
