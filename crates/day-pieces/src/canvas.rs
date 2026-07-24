//! The `canvas` immediate-mode drawing surface — record a `Draw` display list that backends
//! replay natively, with `frame_clock` for per-frame animation — plus the general `Reactive<T>`
//! (value / `Signal` / closure) abstraction that pieces accept for animatable inputs.

use std::cell::RefCell;
use std::rc::Rc;

use day_core::*;
use day_reactive::{Scope, Signal};
use day_spec::props::*;
use day_spec::{Color, DrawOp, Event, Paint, Point, Shape, Size, kinds};

use crate::*;

// ---------------------------------------------------------------------------
// Canvas (§11): record a display list reactively; backends replay natively.
// ---------------------------------------------------------------------------

pub struct Draw {
    ops: Vec<DrawOp>,
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
    pub fn stroke(&mut self, shape: Shape, color: Color, width: f64) {
        self.ops.push(DrawOp::Stroke(shape, color, width));
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
// Reactive<T>: a value, a Signal, or a closure — the generalisation of IntoText/IntoFraction.
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
