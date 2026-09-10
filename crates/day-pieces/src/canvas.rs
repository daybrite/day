// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The `canvas` immediate-mode drawing surface — record a `Draw` display list that backends
//! replay natively, with `frame_clock` for per-frame animation — plus the general `Reactive<T>`
//! (value / `Signal` / closure) abstraction that pieces accept for animatable inputs.

use std::cell::RefCell;
use std::rc::Rc;

use day_core::*;
use day_reactive::{Scope, Signal};
use day_spec::props::*;
use day_spec::{
    Color, DrawOp, Event, FillRule, Paint, PathSeg, Point, Shape, Size, StrokeStyle, kinds,
};

use crate::*;

// ---------------------------------------------------------------------------
// Canvas (§11): record a display list reactively; backends replay natively.
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Draw {
    ops: Vec<DrawOp>,
}

impl Draw {
    /// An empty recorder, for a test that calls a draw function directly and asserts on what
    /// it records — the same list the canvas replays.
    pub fn new() -> Self {
        Draw::default()
    }
    /// What has been recorded so far, in order.
    pub fn ops(&self) -> &[DrawOp] {
        &self.ops
    }
}

/// Build a [`Shape::Path`]: several contours, straight or curved, with a fill rule.
///
/// ```ignore
/// let ring = PathBuilder::new()
///     .rule(FillRule::EvenOdd)          // the inner circle cuts a hole
///     .circle(center, 40.0)
///     .circle(center, 24.0)
///     .build();
/// d.fill(ring, Color::BLUE);
/// ```
#[derive(Clone, Debug, Default)]
pub struct PathBuilder {
    segs: Vec<PathSeg>,
    rule: FillRule,
}

impl PathBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    /// Which points count as inside where contours overlap (default [`FillRule::NonZero`]).
    pub fn rule(mut self, rule: FillRule) -> Self {
        self.rule = rule;
        self
    }
    /// Start a new contour.
    pub fn move_to(mut self, p: Point) -> Self {
        self.segs.push(PathSeg::Move(p));
        self
    }
    pub fn line_to(mut self, p: Point) -> Self {
        self.segs.push(PathSeg::Line(p));
        self
    }
    /// Quadratic bezier through control point `c` to `p`.
    pub fn quad_to(mut self, c: Point, p: Point) -> Self {
        self.segs.push(PathSeg::Quad(c, p));
        self
    }
    /// Cubic bezier with control points `c1`, `c2` to `p`.
    pub fn cubic_to(mut self, c1: Point, c2: Point, p: Point) -> Self {
        self.segs.push(PathSeg::Cubic(c1, c2, p));
        self
    }
    /// Close the current contour back to its start.
    pub fn close(mut self) -> Self {
        self.segs.push(PathSeg::Close);
        self
    }
    /// A whole circular contour, as four cubics.
    ///
    /// 0.5523 is the standard circle-from-beziers constant (`4/3·tan(π/8)`); the error against a
    /// true circle is under a thousandth of the radius, which is well inside a pixel at any size
    /// a UI draws.
    /// Append a circular arc: `sweep_deg` of a circle of `radius` about `center`, starting at
    /// `start_deg`. Degrees, `0` = the +x axis, positive sweeping CLOCKWISE — the same convention
    /// [`Shape::Arc`] and [`PathBuilder::circle`] already use, so the crate has one.
    ///
    /// A *segment*, not a shape: the arc joins whatever came before it, which is what lets a
    /// donut wedge, a rounded gauge or a pie slice with a hole be ONE closed path. Without it
    /// every such figure is hand-rolled from cubics — `day-piece-charts` carried forty lines of
    /// exactly this to draw a wedge.
    ///
    /// Emitted as cubics rather than as an arc op on the wire, deliberately. Every rasterizer
    /// under Day has its own rule for joining an arc to the line before it, and its own
    /// flattening tolerance; the same beziers everywhere means the same pixels everywhere.
    ///
    /// A cubic cannot BE a circle, so this is an approximation, and the quarter turns it splits
    /// into are where that is cheapest: the radial error of a cubic quarter-circle peaks at
    /// 2.7 × 10⁻⁴ of the radius — a quarter of a pixel on a circle a thousand points across, and
    /// proportionally less on anything smaller. Splitting finer would halve nothing anyone can
    /// see and double the segment count. It is the same tradeoff Core Graphics, cairo and every
    /// SVG renderer make.
    ///
    /// Continues the current subpath when there is one, and starts a new one otherwise.
    pub fn arc_to(mut self, center: Point, radius: f64, start_deg: f64, sweep_deg: f64) -> Self {
        let (start, sweep) = (start_deg.to_radians(), sweep_deg.to_radians());
        let at = |a: f64| Point::new(center.x + radius * a.cos(), center.y + radius * a.sin());
        // Reach the arc's start: a line from wherever the path is, or a move if it is nowhere.
        // `Close` ends a subpath, so a path that just closed is "nowhere" too.
        let fresh = self.segs.last().is_none_or(|s| matches!(s, PathSeg::Close));
        let first = at(start);
        self.segs.push(if fresh {
            PathSeg::Move(first)
        } else {
            PathSeg::Line(first)
        });
        if !radius.is_finite() || radius <= 0.0 || !sweep.is_finite() || sweep == 0.0 {
            return self;
        }
        // A cubic tracks a circle well up to a quarter turn and visibly wanders past it, so a
        // long sweep is split rather than approximated in one piece.
        let steps = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil() as usize;
        let delta = sweep / steps as f64;
        // The exact control-handle length for a circular arc of this sweep. Negative for a
        // negative sweep, which is what turns the handles around for a counter-clockwise arc.
        let k = 4.0 / 3.0 * (delta / 4.0).tan() * radius;
        let mut a = start;
        for _ in 0..steps {
            let b = a + delta;
            let (p0, p1) = (at(a), at(b));
            self.segs.push(PathSeg::Cubic(
                Point::new(p0.x - k * a.sin(), p0.y + k * a.cos()),
                Point::new(p1.x + k * b.sin(), p1.y - k * b.cos()),
                p1,
            ));
            a = b;
        }
        self
    }

    pub fn circle(self, center: Point, radius: f64) -> Self {
        const K: f64 = 0.552_284_749_8;
        let (cx, cy, r, k) = (center.x, center.y, radius, radius * K);
        self.move_to(Point::new(cx + r, cy))
            .cubic_to(
                Point::new(cx + r, cy + k),
                Point::new(cx + k, cy + r),
                Point::new(cx, cy + r),
            )
            .cubic_to(
                Point::new(cx - k, cy + r),
                Point::new(cx - r, cy + k),
                Point::new(cx - r, cy),
            )
            .cubic_to(
                Point::new(cx - r, cy - k),
                Point::new(cx - k, cy - r),
                Point::new(cx, cy - r),
            )
            .cubic_to(
                Point::new(cx + k, cy - r),
                Point::new(cx + r, cy - k),
                Point::new(cx + r, cy),
            )
            .close()
    }
    /// A contour through `pts` as a CATMULL-ROM spline converted to cubics — the smooth line a
    /// chart wants through its data points, without the caller doing bezier arithmetic.
    ///
    /// The curve passes through every point (unlike a plain bezier fit), and the tangent at each
    /// point follows its neighbors. `tension` 0.0 is a straight polyline and 1.0 is the
    /// standard Catmull-Rom; values above about 1.2 overshoot visibly.
    pub fn smooth_polyline(mut self, pts: &[Point], tension: f64) -> Self {
        if pts.len() < 2 {
            return match pts.first() {
                Some(p) => self.move_to(*p),
                None => self,
            };
        }
        self.segs.push(PathSeg::Move(pts[0]));
        let t = tension / 6.0;
        for i in 0..pts.len() - 1 {
            // The neighbors on each side, clamped at the ends so the first and last segments
            // keep the same construction as the middle ones.
            let p0 = pts[i.saturating_sub(1)];
            let (p1, p2) = (pts[i], pts[i + 1]);
            let p3 = pts[(i + 2).min(pts.len() - 1)];
            self.segs.push(PathSeg::Cubic(
                Point::new(p1.x + (p2.x - p0.x) * t, p1.y + (p2.y - p0.y) * t),
                Point::new(p2.x - (p3.x - p1.x) * t, p2.y - (p3.y - p1.y) * t),
                p2,
            ));
        }
        self
    }
    /// Finish, as a [`Shape`] ready for `fill`, `stroke` or `clip`.
    pub fn build(self) -> Shape {
        Shape::Path(day_spec::Path {
            segs: self.segs,
            rule: self.rule,
        })
    }
}

/// Canvas text styling (named fields per the API style rule, docs/api-style.md). Fill what you
/// set and take the rest from `..Default::default()`: 12 points, black, top-leading, the
/// platform's own face.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    /// Absolute canvas points — no accessibility scale (docs/canvas.md "Text").
    pub size: f64,
    pub color: Color,
    pub anchor: day_spec::TextAnchor,
    /// Family, weight and slant (docs/fonts.md); the default is the platform's UI face.
    pub font: day_spec::CanvasFont,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            size: 12.0,
            color: Color::BLACK,
            anchor: day_spec::TextAnchor::LEADING,
            font: day_spec::CanvasFont::default(),
        }
    }
}

impl Draw {
    /// Fill a shape with a solid color or a [`LinearGradient`] (both convert to [`Paint`];
    /// gradient unit points resolve against the shape's bounding box — docs/shapes.md §3.2).
    pub fn fill(&mut self, shape: Shape, paint: impl Into<Paint>) {
        self.ops.push(DrawOp::Fill(shape, paint.into()));
    }
    /// Stroke a shape with a solid color at `width` — the everyday case.
    pub fn stroke(&mut self, shape: Shape, color: Color, width: f64) {
        self.ops.push(DrawOp::Stroke(
            shape,
            Paint::Solid(color),
            StrokeStyle::width(width),
        ));
    }
    /// Stroke with a full [`StrokeStyle`] (dash, cap, join) and any paint, gradients included.
    pub fn stroke_styled(&mut self, shape: Shape, paint: impl Into<Paint>, style: StrokeStyle) {
        self.ops
            .push(DrawOp::Stroke(shape, paint.into(), style.clone()));
    }
    /// Draw `shape` once at EVERY position in `at` — one op for the whole batch.
    ///
    /// The template is authored around the ORIGIN and each copy is it translated by one point, so
    /// a 6-point dot is `Shape::Ellipse(Rect::new(-3.0, -3.0, 6.0, 6.0))`. Order is drawing order.
    ///
    /// Reach for this the moment a drawing has more marks than it has kinds of mark. A scatter of
    /// fifty thousand points drawn one `fill` at a time is fifty thousand ops to build, compare
    /// and clone on every frame that re-records; as one stamp it is one op and a flat array of
    /// coordinates, and a backend can put every copy into a single path (docs/canvas.md
    /// "Stamping"). Every copy shares the shape, size and paint — group by whatever varies.
    pub fn stamp(&mut self, shape: Shape, at: Vec<Point>, paint: impl Into<Paint>) {
        self.ops.push(DrawOp::Stamp(Box::new(day_spec::Stamp {
            shape,
            at,
            paint: paint.into(),
            stroke: None,
        })));
    }

    /// [`Draw::stamp`], stroking each copy instead of filling it.
    pub fn stamp_styled(
        &mut self,
        shape: Shape,
        at: Vec<Point>,
        paint: impl Into<Paint>,
        style: StrokeStyle,
    ) {
        self.ops.push(DrawOp::Stamp(Box::new(day_spec::Stamp {
            shape,
            at,
            paint: paint.into(),
            stroke: Some(style),
        })));
    }

    /// Confine everything drawn afterwards to `shape`.
    ///
    /// The clip lasts until the enclosing [`Draw::restore`], so the usual shape is
    /// `save` → `clip` → draw → `restore`; [`Draw::clipped`] does exactly that for you.
    pub fn clip(&mut self, shape: Shape) {
        self.ops.push(DrawOp::Clip(shape));
    }
    /// Draw `f` clipped to `shape`, restoring the previous clip afterwards.
    pub fn clipped(&mut self, shape: Shape, f: impl FnOnce(&mut Draw)) {
        self.save();
        self.clip(shape);
        f(self);
        self.restore();
    }
    /// One line of text hung on `at` per the style's anchor (docs/canvas.md "Text"); measure it
    /// first with `day::measure_text` when the drawing needs its extent.
    pub fn text(&mut self, text: &str, at: Point, style: TextStyle) {
        self.ops.push(DrawOp::Text {
            text: text.to_owned(),
            at,
            size: style.size,
            color: style.color,
            anchor: style.anchor,
            font: style.font,
        });
    }
    /// Save the current transform/clip; pair with [`Draw::restore`].
    pub fn save(&mut self) {
        self.ops.push(DrawOp::Save);
    }
    /// Restore the transform/clip saved by the matching [`Draw::save`].
    pub fn restore(&mut self) {
        self.ops.push(DrawOp::Restore);
    }
    /// Multiply an affine onto the current transform (shape rotate/scale/offset, §11).
    pub fn concat(&mut self, m: day_geometry::Affine) {
        self.ops.push(DrawOp::Concat(m));
    }
    /// Draw within `m` applied to the CTM, restoring afterwards.
    pub fn transformed(&mut self, m: day_geometry::Affine, f: impl FnOnce(&mut Draw)) {
        self.save();
        self.concat(m);
        f(self);
        self.restore();
    }
}

/// Create + wire a reactive canvas leaf with a given flex: the draw closure re-records on any
/// tracked read and on `FrameChanged`; replay is equality-gated by `DrawOp: PartialEq` (§4.2).
/// Shared by [`canvas`] (intrinsic) and [`shape`] (grows to fill, §shapes).
pub(crate) fn canvas_leaf(
    cx: &mut BuildCx,
    flex: Flex,
    draw: impl Fn(&mut Draw, Size) + 'static,
) -> RNode {
    use day_reactive::{Trigger, bind};
    let node = cx.leaf(kinds::CANVAS, &CanvasProps::default(), flex);
    let trig = Trigger::new();
    cx.on(node, move |ev| {
        if matches!(ev, Event::FrameChanged(_)) {
            trig.notify();
        }
    });
    let draw = std::rc::Rc::new(draw);
    let d2 = draw.clone();
    bind(
        move || {
            trig.track();
            let size = with_tree(|t| t.node_frame(node))
                .map(|f| f.size)
                .unwrap_or(Size::new(0.0, 0.0));
            let mut d = Draw { ops: Vec::new() };
            (d2)(&mut d, size);
            d.ops
        },
        move |ops: &Vec<DrawOp>| {
            with_tree(|t| t.replay(node, ops.clone()));
        },
    );
    node
}

/// The drawing closure is a binding: signal reads re-record; layout size changes re-record
/// (via FrameChanged); replay is equality-gated by DrawOp's PartialEq (§4.2).
pub fn canvas(draw: impl Fn(&mut Draw, Size) + 'static) -> impl Piece {
    piece_fn(move |cx| canvas_leaf(cx, Flex::default(), draw))
}

/// A frame clock (§8.4): an invisible, zero-size piece that calls `tick` every animation frame with
/// the wall-clock delta since the previous frame, for as long as it is mounted. Drop it into the
/// tree (e.g. behind a `canvas` in a `zstack`) to drive a game loop or self-driven animation: the
/// tick mutates state `Signal`s, and a `canvas` reading them re-records that frame.
///
/// Backend-executed vsync: Day re-arms the platform's display link only while a `frame_clock` (or
/// other consumer) is live and stops when the last one unmounts — no idle wakeups. The delta is
/// clamped (≤100 ms) so a backgrounded window can't deliver a huge jump.
///
/// ```ignore
/// zstack((
///     canvas(move |d, sz| draw(d, sz, state)).grow(),
///     frame_clock(move |dt| step(dt, state)),
/// ))
/// ```
pub fn frame_clock(tick: impl FnMut(std::time::Duration) + 'static) -> impl Piece {
    type TickSlot = Rc<RefCell<Option<Box<dyn FnMut(std::time::Duration)>>>>;
    // Registered on first build (in the mounting scope) and removed when that scope is disposed.
    let slot: TickSlot = Rc::new(RefCell::new(Some(Box::new(tick))));
    piece_fn(move |cx| {
        if let Some(cb) = slot.borrow_mut().take() {
            let id = day_core::add_frame_consumer(cb);
            Scope::current().on_cleanup(move || day_core::remove_frame_consumer(id));
        }
        label("").frame(0.0, 0.0).build(cx)
    })
}

// ---------------------------------------------------------------------------
// Reactive<T>: a value, a Signal, or a closure — the generalization of IntoText/IntoFraction.
// ---------------------------------------------------------------------------

/// A parameter that is either a constant or a reactive source. `get()` is a tracked read, so any
/// `Reactive` used inside a canvas draw closure makes that shape re-record when the source changes.
pub enum Reactive<T: Clone + 'static> {
    Const(T),
    Dyn(Rc<dyn Fn() -> T>),
}
impl<T: Clone + 'static> Clone for Reactive<T> {
    fn clone(&self) -> Self {
        match self {
            Reactive::Const(v) => Reactive::Const(v.clone()),
            Reactive::Dyn(f) => Reactive::Dyn(f.clone()),
        }
    }
}
impl<T: Clone + 'static> Reactive<T> {
    pub fn get(&self) -> T {
        match self {
            Reactive::Const(v) => v.clone(),
            Reactive::Dyn(f) => f(),
        }
    }
    pub fn get_untracked(&self) -> T {
        match self {
            Reactive::Const(v) => v.clone(),
            Reactive::Dyn(f) => day_reactive::untrack(|| f()),
        }
    }
}
/// Disjoint-marker conversion (like [`IntoText`]): accepts `T`, `Signal<T>`, or `Fn() -> T`.
pub trait IntoReactive<T: Clone + 'static, M> {
    fn into_reactive(self) -> Reactive<T>;
}
impl<T: Clone + 'static> IntoReactive<T, StaticMark> for T {
    fn into_reactive(self) -> Reactive<T> {
        Reactive::Const(self)
    }
}
impl<T: Clone + 'static> IntoReactive<T, SignalMark> for Signal<T> {
    fn into_reactive(self) -> Reactive<T> {
        Reactive::Dyn(Rc::new(move || self.get()))
    }
}
impl<T: Clone + 'static, F: Fn() -> T + 'static> IntoReactive<T, FnMark> for F {
    fn into_reactive(self) -> Reactive<T> {
        Reactive::Dyn(Rc::new(self))
    }
}

#[cfg(test)]
mod arc_tests {
    use super::PathBuilder;
    use day_spec::{PathSeg, Point};

    /// Walk a built path, sampling every cubic densely, and hand back the points.
    fn sample(segs: &[PathSeg], per_seg: usize) -> Vec<Point> {
        let mut out = Vec::new();
        let mut cur = Point::ZERO;
        for seg in segs {
            match seg {
                PathSeg::Move(p) | PathSeg::Line(p) => {
                    cur = *p;
                    out.push(cur);
                }
                PathSeg::Cubic(c1, c2, p) => {
                    for i in 1..=per_seg {
                        let t = i as f64 / per_seg as f64;
                        let u = 1.0 - t;
                        // de Casteljau, written out: the cubic Bernstein basis.
                        out.push(Point::new(
                            u * u * u * cur.x
                                + 3.0 * u * u * t * c1.x
                                + 3.0 * u * t * t * c2.x
                                + t * t * t * p.x,
                            u * u * u * cur.y
                                + 3.0 * u * u * t * c1.y
                                + 3.0 * u * t * t * c2.y
                                + t * t * t * p.y,
                        ));
                    }
                    cur = *p;
                }
                PathSeg::Quad(..) | PathSeg::Close => {}
            }
        }
        out
    }

    /// The property that matters: every point of the emitted curve is on the circle. Pinning the
    /// control points instead would pin the construction rather than the result — and it is the
    /// result a rasterizer draws.
    #[test]
    fn an_arc_stays_on_its_circle() {
        let c = Point::new(100.0, 50.0);
        let r = 40.0;
        for sweep in [15.0, 90.0, 180.0, 359.0, -90.0, -270.0] {
            let path = PathBuilder::new().arc_to(c, r, 30.0, sweep).build();
            let day_spec::Shape::Path(p) = path else {
                panic!("arc_to builds a path")
            };
            let worst = sample(&p.segs, 24)
                .iter()
                .map(|q| ((q.x - c.x).powi(2) + (q.y - c.y).powi(2)).sqrt() - r)
                .fold(0.0f64, |m, e| m.max(e.abs()));
            // The known peak for a cubic quarter-circle, with a little headroom. Asserting the
            // real figure rather than a loose one is the point: if a future change split the
            // sweep differently or got a handle length wrong, this is what would notice.
            assert!(worst < r * 2.8e-4, "sweep {sweep}: off by {worst}");
            // …and it is genuinely that good, not accidentally better — a construction that
            // silently started flattening to line segments would pass a one-sided bound.
            if sweep.abs() >= 90.0 {
                assert!(
                    worst > r * 1e-6,
                    "sweep {sweep}: suspiciously exact ({worst})"
                );
            }
        }
    }

    /// Degrees, 0 = +x, positive CLOCKWISE — the same convention `Shape::Arc` and `circle` use.
    /// A wrong sign here is the kind of thing that only shows up as a donut drawn inside out.
    #[test]
    fn arc_endpoints_follow_the_clockwise_degree_convention() {
        let c = Point::new(0.0, 0.0);
        let near = |a: Point, b: Point| (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9;
        let ends = |start: f64, sweep: f64| {
            let day_spec::Shape::Path(p) = PathBuilder::new().arc_to(c, 10.0, start, sweep).build()
            else {
                panic!()
            };
            let pts = sample(&p.segs, 1);
            (*pts.first().unwrap(), *pts.last().unwrap())
        };
        // 0° is +x; a quarter turn clockwise lands on +y, which is DOWN in canvas space.
        let (a, b) = ends(0.0, 90.0);
        assert!(near(a, Point::new(10.0, 0.0)), "{a:?}");
        assert!(near(b, Point::new(0.0, 10.0)), "{b:?}");
        // …and the same quarter counter-clockwise lands on -y.
        let (_, b) = ends(0.0, -90.0);
        assert!(near(b, Point::new(0.0, -10.0)), "{b:?}");
    }

    /// An arc JOINS what came before — that is the whole reason it is a segment and not a shape.
    #[test]
    fn an_arc_joins_the_current_subpath_but_starts_a_fresh_one_after_close() {
        let c = Point::new(0.0, 0.0);
        let segs = |b: PathBuilder| {
            let day_spec::Shape::Path(p) = b.build() else {
                panic!()
            };
            p.segs
        };
        // Nothing before it: the arc opens the subpath.
        let s = segs(PathBuilder::new().arc_to(c, 5.0, 0.0, 90.0));
        assert!(matches!(s[0], PathSeg::Move(_)), "{:?}", s[0]);
        // Something before it: the arc is reached by a line, so the figure is one contour.
        let s = segs(
            PathBuilder::new()
                .move_to(Point::new(9.0, 9.0))
                .arc_to(c, 5.0, 0.0, 90.0),
        );
        assert!(matches!(s[1], PathSeg::Line(_)), "{:?}", s[1]);
        // After a close there is no current point, so it opens a new subpath rather than drawing
        // a stray line back from wherever the last contour ended.
        let s = segs(
            PathBuilder::new()
                .move_to(Point::new(9.0, 9.0))
                .line_to(Point::new(1.0, 1.0))
                .close()
                .arc_to(c, 5.0, 0.0, 90.0),
        );
        assert!(matches!(s[3], PathSeg::Move(_)), "{:?}", s[3]);
        // A degenerate arc still reaches its start point and emits no curve.
        let s = segs(PathBuilder::new().arc_to(c, 5.0, 0.0, 0.0));
        assert_eq!(s.len(), 1);
        let s = segs(PathBuilder::new().arc_to(c, 0.0, 0.0, 90.0));
        assert_eq!(s.len(), 1);
    }
}
