//! High-level vector shapes atop the canvas display list: `rectangle`, `circle`, `ellipse`,
//! `capsule`, `rounded_rectangle`, `arc`, `line`, and `polygon`, combined with `shape_group`.
//! Reactive fill/stroke, rotate/scale/offset transforms, and tap/drag gestures.

use std::rc::Rc;

use day_core::*;
use day_spec::{Color, Event, LinearGradient, Point, RadialGradient, Rect, Shape, Size, UnitPoint};

use crate::*;

// ---------------------------------------------------------------------------
// Shapes (docs/shapes.md): high-level shape pieces atop the canvas display list. Frame-relative
// geometry, reactive fill/stroke, rotate/scale/offset transforms, and tap/drag gestures.
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
    /// A stroked arc of the inscribed ellipse; degrees, 0 = +x, clockwise.
    Arc {
        start_deg: f64,
        sweep_deg: f64,
    },
    /// A stroked segment between two unit points of the resolved rect (stroke-only; fills are
    /// ignored). Unit points resolve unclamped, like every unit-space resolve.
    Line {
        from: UnitPoint,
        to: UnitPoint,
    },
    /// A filled/stroked polygon of unit points resolved against the rect (unclamped — points may
    /// deliberately sit outside 0..1).
    Polygon {
        points: Rc<[UnitPoint]>,
    },
}

/// A corner radius: absolute points, or a 0..1 fraction of `min(width, height)`.
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
impl Corner {
    fn resolve(self, rect: Rect) -> f64 {
        let cap = rect.size.width.min(rect.size.height) / 2.0;
        match self {
            Corner::Fixed(v) => v.clamp(0.0, cap),
            Corner::Fraction(f) => f.clamp(0.0, 1.0) * cap,
        }
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
            ShapeKind::Arc {
                start_deg,
                sweep_deg,
            } => Shape::Arc {
                rect,
                start_deg,
                sweep_deg,
            },
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

/// Drag info delivered to a shape's `.on_drag` handler.
#[derive(Clone, Copy, Debug)]
pub struct Drag {
    pub phase: DragPhase,
    pub location: Point,
    pub translation: Point,
}

/// The drawable description of a shape — everything but gestures. Cloneable so [`shape_group`]
/// can collect many descriptions into one canvas closure (docs/shapes.md §3.6).
#[derive(Clone)]
struct ShapeSpec {
    kind: Reactive<ShapeKind>,
    fill: Option<Reactive<Color>>,
    fill_linear: Option<Reactive<LinearGradient>>,
    fill_radial: Option<Reactive<RadialGradient>>,
    stroke: Option<(Reactive<Color>, Reactive<f64>)>,
    inset: Reactive<f64>,
    rotate: Reactive<f64>,
    scale: Reactive<f64>,
    offset: (Reactive<f64>, Reactive<f64>),
    /// Unit-space sub-rect of the bounds this shape resolves in (`.at`).
    at: Option<Rect>,
}

/// A shape piece — one data-oriented piece parameterised by `ShapeKind`, rendered atop the canvas.
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
            fill_linear: None,
            fill_radial: None,
            stroke: None,
            inset: Reactive::Const(0.0),
            rotate: Reactive::Const(0.0),
            scale: Reactive::Const(1.0),
            offset: (Reactive::Const(0.0), Reactive::Const(0.0)),
            at: None,
        },
        on_tap: None,
        on_drag: None,
    }
}
/// SwiftUI-ergonomic sugar — all build the same `ShapePiece`.
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
pub fn arc(start_deg: f64, sweep_deg: f64) -> ShapePiece {
    shape(ShapeKind::Arc {
        start_deg,
        sweep_deg,
    })
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

impl ShapePiece {
    pub fn fill<M>(mut self, p: impl IntoReactive<Color, M>) -> Self {
        self.spec.fill = Some(p.into_reactive());
        self
    }
    /// Fill with a [`LinearGradient`] (unit points resolve against the shape's bounds). Takes
    /// precedence over [`Self::fill`] when both are set; reactive like every other property:
    /// `rectangle().fill_linear(move || sky_gradient(state.get()))`.
    pub fn fill_linear<M>(mut self, g: impl IntoReactive<LinearGradient, M>) -> Self {
        self.spec.fill_linear = Some(g.into_reactive());
        self
    }
    /// Fill with a [`RadialGradient`] (center + radius in the unit space of the shape's bounds,
    /// stretching elliptically in non-square bounds). Precedence: radial over linear over solid.
    pub fn fill_radial<M>(mut self, g: impl IntoReactive<RadialGradient, M>) -> Self {
        self.spec.fill_radial = Some(g.into_reactive());
        self
    }
    pub fn stroke<M1, M2>(
        mut self,
        color: impl IntoReactive<Color, M1>,
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
    /// Rotate the drawn shape about its centre, in degrees.
    pub fn rotate<M>(mut self, deg: impl IntoReactive<f64, M>) -> Self {
        self.spec.rotate = deg.into_reactive();
        self
    }
    /// Scale the drawn shape about its centre (uniform).
    pub fn scale<M>(mut self, s: impl IntoReactive<f64, M>) -> Self {
        self.spec.scale = s.into_reactive();
        self
    }
    /// Translate the drawn shape within its frame.
    pub fn offset<M1, M2>(
        mut self,
        x: impl IntoReactive<f64, M1>,
        y: impl IntoReactive<f64, M2>,
    ) -> Self {
        self.spec.offset = (x.into_reactive(), y.into_reactive());
        self
    }
    /// Resolve this shape inside the fractional sub-rect `(fx, fy, fw, fh)` of its bounds —
    /// unit-space, applied before [`Self::inset`]. The workhorse for composing glyphs in a
    /// [`shape_group`], mirroring hand-drawn `Rect::new(ox + fx * s, oy + fy * s, …)` canvas code.
    pub fn at(mut self, fx: f64, fy: f64, fw: f64, fh: f64) -> Self {
        self.spec.at = Some(Rect::new(fx, fy, fw, fh));
        self
    }
    /// Fire when the shape is tapped (path-precise — the tap is tested against the resolved path).
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

/// Compose the shape's rotate/scale (about its centre) + offset into one affine.
fn shape_transform(rect: Rect, rot_deg: f64, scale: f64, ox: f64, oy: f64) -> Affine {
    let c = rect.center();
    Affine::translate(-c.x, -c.y)
        .then(Affine::scale(scale, scale))
        .then(Affine::rotate(rot_deg.to_radians()))
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

/// Record one shape description into `d`, resolved within `bounds` — shared by
/// [`ShapePiece`]'s own canvas leaf and by [`shape_group`] / [`shape_group_fn`].
fn record_shape(spec: &ShapeSpec, d: &mut Draw, bounds: Rect) {
    let bounds = resolve_at(spec.at, bounds);
    let kind = spec.kind.get();
    // A centered stroke overflows the geometry by half its width; inset closed shapes by w/2 so
    // the whole stroke stays inside the view bounds — backends that clip a canvas to its bounds
    // (Qt/Android/WinUI) would otherwise cut the stroke's outer edge. (SwiftUI `strokeBorder`
    // behavior.) Fill-only shapes are unaffected (stroke_half = 0). Line/Polygon are exempt:
    // they resolve exactly at their authored unit points, and a line's rect is legitimately
    // degenerate (zero-height for a horizontal segment), so they skip the empty-rect bail too.
    let open = matches!(kind, ShapeKind::Line { .. } | ShapeKind::Polygon { .. });
    let stroke_half = if open {
        0.0
    } else {
        spec.stroke
            .as_ref()
            .map(|(_, w)| w.get() / 2.0)
            .unwrap_or(0.0)
    };
    let rect = bounds.inset(spec.inset.get() + stroke_half);
    if !open && (rect.size.width <= 0.0 || rect.size.height <= 0.0) {
        return;
    }
    let geom = kind.geometry(rect);
    let m = shape_transform(
        rect,
        spec.rotate.get(),
        spec.scale.get(),
        spec.offset.0.get(),
        spec.offset.1.get(),
    );
    let transformed = !m.is_identity();
    if transformed {
        d.save();
        d.concat(m);
    }
    if !matches!(geom, Shape::Line(..)) {
        if let Some(g) = &spec.fill_radial {
            d.fill(geom.clone(), g.get());
        } else if let Some(g) = &spec.fill_linear {
            d.fill(geom.clone(), g.get());
        } else if let Some(fill) = &spec.fill {
            d.fill(geom.clone(), fill.get());
        }
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

/// Flatten many shape descriptions into ONE canvas leaf — one native view no matter how many
/// shapes (docs/shapes.md §3.6). Shapes draw in order; reactive properties re-record the group.
/// Child gestures are not wired inside a group — put `.on_tap` on the group via [`Decorate`].
pub fn shape_group(shapes: impl IntoIterator<Item = ShapePiece>) -> AnyPiece {
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
/// on `FrameChanged`, exactly like [`canvas`] — for geometry that depends on the final size
/// (e.g. data mapped along the width).
pub fn shape_group_fn(shapes: impl Fn(Size) -> Vec<ShapePiece> + 'static) -> AnyPiece {
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

        // Path-precise tap: inverse-transform the point, then test against the resolved geometry.
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
                        spec.rotate.get_untracked(),
                        spec.scale.get_untracked(),
                        spec.offset.0.get_untracked(),
                        spec.offset.1.get_untracked(),
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
                    on_drag(Drag {
                        phase: *phase,
                        location: *location,
                        translation: *translation,
                    });
                }
            });
        }
        node
    }
}
