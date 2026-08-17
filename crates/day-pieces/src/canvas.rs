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

pub struct Draw {
    ops: Vec<DrawOp>,
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

/// Canvas text styling (named fields per the API style rule, docs/api-style.md).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub size: f64,
    pub color: Color,
    pub anchor: day_spec::TextAnchor,
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
    pub fn text(&mut self, text: &str, at: Point, style: TextStyle) {
        self.ops.push(DrawOp::Text {
            text: text.to_owned(),
            at,
            size: style.size,
            color: style.color,
            anchor: style.anchor,
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
pub fn canvas(draw: impl Fn(&mut Draw, Size) + 'static) -> AnyPiece {
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
pub fn frame_clock(tick: impl FnMut(std::time::Duration) + 'static) -> AnyPiece {
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
    .any()
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
