// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Gesture recognition shared by every toolkit (docs/gestures.md "Recognizers").
//!
//! Every native event passes through `route` on its way to a node's handlers. That one
//! place does what no backend has to repeat:
//!
//! - **Velocity.** A drag's and a pan's velocity is estimated from the event samples
//!   ([`VelocityTracker`]); a backend whose recognizer measures its own sends
//!   `Event::GestureVelocity` before `Ended` and that value wins.
//! - **Derived recognizers.** Where `Toolkit::gesture_support` answers `Unsupported`, press-in
//!   and press-out, long press, double tap and a single-contact touch list are derived from the
//!   tap and drag events (`Recognizer`).
//! - **The drag axis.** A drag declared along one axis never reaches its handler when the
//!   pointer set off along the other.
//! - **Disabled subtrees.** Input addressed to a node under `.enabled(false)` is dropped.
//! - **The input kind.** `Event::PointerKindChanged` lands in [`primary_pointer`].

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};

use day_reactive::Signal;
use day_spec::{
    DragAxis, DragPhase, Event, GestureKind, NodeId, Point, PointerKind, Support, Touch, TouchPhase,
};

use crate::tree::{RNode, id_to_rnode, rnode_to_id, with_tree};

/// How long a press is held still before it is a long press, in milliseconds, where day-core
/// derives the gesture. A native recognizer uses the platform's own delay.
pub const LONG_PRESS_MS: u32 = 500;
/// How long a single tap waits for a second one on a piece that handles both, in milliseconds,
/// where day-core derives the double tap. A native recognizer uses the platform's interval.
pub const DOUBLE_TAP_MS: u32 = 300;
/// How far apart two taps may land and still be a double tap, in points.
const DOUBLE_TAP_SLOP: f64 = 40.0;
/// How far a press may travel and still become a long press, in points.
const PRESS_SLOP: f64 = 10.0;
/// How far a drag declared along one axis travels before its direction is judged, in points.
const AXIS_SLOP: f64 = 3.0;
/// The span of samples a velocity is measured over, in seconds.
const VELOCITY_WINDOW: f64 = 0.1;
/// A tap this soon after a press ended belongs to that press, in seconds: some backends report
/// a still press as a zero-length drag and a tap, and the pair is one press.
const SAME_PRESS: f64 = 0.3;

day_reactive::tls_slots! {
    gesture;
    static NODES: RefCell<HashMap<NodeId, NodeState>> = RefCell::new(HashMap::new());
    static POINTER: Cell<Option<Signal<PointerKind>>> = const { Cell::new(None) };
}

/// Estimates a pointer's velocity from timestamped positions: the distance covered over the
/// last tenth of a second, in points per second, and zero once the pointer has rested that
/// long. One per drag and per pan; public so a piece that tracks its own pointer (a custom
/// recognizer over `on_touches`) measures the same way.
#[derive(Clone, Debug, Default)]
pub struct VelocityTracker {
    samples: VecDeque<(f64, Point)>,
}

impl VelocityTracker {
    /// Forget every sample: a new gesture began.
    pub fn reset(&mut self) {
        self.samples.clear();
    }

    /// Record the pointer at `position` at time `at` (seconds on a monotonic clock).
    pub fn push(&mut self, at: f64, position: Point) {
        self.samples.push_back((at, position));
        while self
            .samples
            .front()
            .is_some_and(|(t, _)| at - t > VELOCITY_WINDOW * 2.0)
        {
            self.samples.pop_front();
        }
    }

    /// The velocity at time `now`, in points per second. Zero with fewer than two samples in
    /// the window, or when they span less than a millisecond (a scripted gesture's samples
    /// arrive at once and measure nothing).
    #[must_use]
    pub fn velocity(&self, now: f64) -> Point {
        let mut recent = self
            .samples
            .iter()
            .filter(|(t, _)| now - t <= VELOCITY_WINDOW);
        let Some(&(t0, p0)) = recent.next() else {
            return Point::ZERO;
        };
        let Some(&(t1, p1)) = recent.next_back() else {
            return Point::ZERO;
        };
        let span = t1 - t0;
        if span < 0.001 {
            return Point::ZERO;
        }
        Point::new((p1.x - p0.x) / span, (p1.y - p0.y) / span)
    }
}

/// Which gestures day-core derives for a node, decided when the piece enabled them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Derived {
    pub press: bool,
    pub long_press: bool,
    pub double_tap: bool,
    pub touches: bool,
}

/// What a [`Recognizer`] asks its host to do after an event.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    /// Deliver this event to the node's handlers.
    Emit(Event),
    /// Call [`Recognizer::long_press_due`] with this token after the long-press delay.
    ArmLongPress(u64),
    /// Call [`Recognizer::tap_due`] with this token after the double-tap interval.
    ArmTap(u64),
}

/// Whether a drag declared along one axis is the node's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum AxisGate {
    /// No drag under way.
    #[default]
    Idle,
    /// `Began` arrived and is held back until the pointer has moved far enough to judge.
    Pending { location: Point, translation: Point },
    /// The drag runs along the axis: its events pass.
    Open,
    /// The drag set off across the axis: its events are withheld until it ends.
    Closed,
}

/// The per-node state machine behind the derived recognizers, free of clocks and timers so it
/// can be tested directly: every method takes the time, and timers are requested as
/// [`Action`]s and answered through [`Recognizer::long_press_due`] and [`Recognizer::tap_due`].
#[derive(Debug, Default)]
pub(crate) struct Recognizer {
    pub derived: Derived,
    /// The piece handles a single tap.
    pub wants_tap: bool,
    /// The piece handles a double tap.
    pub wants_double_tap: bool,
    /// The piece handles a drag.
    pub wants_drag: bool,
    pub axis: Option<DragAxis>,
    gate: AxisGate,
    /// The press point while the pointer is down.
    pressed: Option<Point>,
    /// The press moved past [`PRESS_SLOP`], which rules a long press out.
    moved: bool,
    /// This press already fired its long press, which rules a tap out.
    held: bool,
    press_ended_at: f64,
    swallow_tap_until: f64,
    long_press_token: u64,
    /// A single tap waiting out the double-tap interval: where, when, and its timer token.
    pending_tap: Option<(Point, f64, u64)>,
    /// The previous tap on a piece that handles double taps only.
    last_tap: Option<(Point, f64)>,
    next_token: u64,
}

impl Recognizer {
    fn token(&mut self) -> u64 {
        self.next_token += 1;
        self.next_token
    }

    /// Whether the pointer is down on the node, as far as the derived press knows.
    pub fn is_pressed(&self) -> bool {
        self.pressed.is_some()
    }

    fn press_in(&mut self, at: Point, moved: bool, out: &mut Vec<Action>) {
        self.pressed = Some(at);
        self.moved = moved;
        self.held = false;
        if self.derived.press {
            out.push(Action::Emit(Event::Press {
                pressed: true,
                location: at,
            }));
        }
        if self.derived.long_press && !moved {
            self.long_press_token = self.token();
            out.push(Action::ArmLongPress(self.long_press_token));
        }
    }

    fn press_out(&mut self, at: Point, now: f64, out: &mut Vec<Action>) {
        if self.pressed.take().is_none() {
            return;
        }
        self.long_press_token = 0;
        self.press_ended_at = now;
        if self.held {
            self.swallow_tap_until = now + SAME_PRESS;
        }
        if self.derived.press {
            out.push(Action::Emit(Event::Press {
                pressed: false,
                location: at,
            }));
        }
    }

    /// A press event, native or scripted: the long press is timed from it where derived.
    pub fn on_press(&mut self, pressed: bool, location: Point, now: f64, out: &mut Vec<Action>) {
        if pressed {
            self.pressed = Some(location);
            self.moved = false;
            self.held = false;
            if self.derived.long_press {
                self.long_press_token = self.token();
                out.push(Action::ArmLongPress(self.long_press_token));
            }
        } else if self.pressed.take().is_none() {
            // A release with no press under way (a toolkit reporting both edges of a press it
            // did not start here) ends nothing.
            return;
        } else {
            self.long_press_token = 0;
            self.press_ended_at = now;
            if self.held {
                self.swallow_tap_until = now + SAME_PRESS;
            }
        }
        out.push(Action::Emit(Event::Press { pressed, location }));
    }

    /// One drag event. Derives the press and the touch list from its phases, then lets the
    /// drag itself through unless the axis gate withholds it.
    pub fn on_drag(
        &mut self,
        phase: DragPhase,
        location: Point,
        translation: Point,
        now: f64,
        out: &mut Vec<Action>,
    ) {
        let travel = translation.x.hypot(translation.y);
        let derive_press = self.derived.press || self.derived.long_press;
        match phase {
            DragPhase::Began => {
                if derive_press {
                    // A recognizer that waited for movement reports `Began` with the
                    // translation already applied: the press point is where it started.
                    let at = Point::new(location.x - translation.x, location.y - translation.y);
                    self.press_in(at, travel > PRESS_SLOP / 2.0, out);
                }
                if self.derived.touches {
                    out.push(Action::Emit(Event::Touches(vec![Touch::new(
                        0,
                        TouchPhase::Began,
                        location,
                    )])));
                }
            }
            DragPhase::Changed => {
                if derive_press && travel > PRESS_SLOP && !self.moved {
                    self.moved = true;
                    self.long_press_token = 0;
                }
                if self.derived.touches {
                    out.push(Action::Emit(Event::Touches(vec![Touch::new(
                        0,
                        TouchPhase::Moved,
                        location,
                    )])));
                }
            }
            DragPhase::Ended => {
                if derive_press {
                    self.press_out(location, now, out);
                }
                if self.derived.touches {
                    out.push(Action::Emit(Event::Touches(vec![Touch::new(
                        0,
                        TouchPhase::Ended,
                        location,
                    )])));
                }
            }
        }
        let drag = |phase, location, translation| {
            Action::Emit(Event::Drag {
                phase,
                location,
                translation,
            })
        };
        let Some(axis) = self.axis else {
            out.push(drag(phase, location, translation));
            return;
        };
        match (phase, self.gate) {
            (DragPhase::Began, _) => {
                if travel >= AXIS_SLOP {
                    if axis.leads(translation) {
                        self.gate = AxisGate::Open;
                        out.push(drag(phase, location, translation));
                    } else {
                        self.gate = AxisGate::Closed;
                    }
                } else {
                    self.gate = AxisGate::Pending {
                        location,
                        translation,
                    };
                }
            }
            (
                DragPhase::Changed,
                AxisGate::Pending {
                    location: began_at,
                    translation: began_by,
                },
            ) => {
                if travel >= AXIS_SLOP {
                    if axis.leads(translation) {
                        self.gate = AxisGate::Open;
                        out.push(drag(DragPhase::Began, began_at, began_by));
                        out.push(drag(phase, location, translation));
                    } else {
                        self.gate = AxisGate::Closed;
                    }
                }
            }
            (DragPhase::Changed, AxisGate::Open) => out.push(drag(phase, location, translation)),
            (DragPhase::Ended, AxisGate::Open) => {
                self.gate = AxisGate::Idle;
                out.push(drag(phase, location, translation));
            }
            // A flick whose movement arrives only with its release (no `Changed` in between)
            // is decided there, and delivered whole when it went the drag's way.
            (
                DragPhase::Ended,
                AxisGate::Pending {
                    location: began_at,
                    translation: began_by,
                },
            ) => {
                self.gate = AxisGate::Idle;
                if travel >= AXIS_SLOP && axis.leads(translation) {
                    out.push(drag(DragPhase::Began, began_at, began_by));
                    out.push(drag(phase, location, translation));
                }
            }
            (DragPhase::Ended, _) => self.gate = AxisGate::Idle,
            (DragPhase::Changed, _) => {}
        }
    }

    /// One tap. Swallows the release of a long press, derives a press for a backend whose
    /// drag begins only with movement, and pairs taps into a double tap.
    pub fn on_tap(&mut self, at: Point, now: f64, out: &mut Vec<Action>) {
        if now < self.swallow_tap_until {
            self.swallow_tap_until = 0.0;
            return;
        }
        if self.derived.press && self.pressed.is_none() && now - self.press_ended_at > SAME_PRESS {
            out.push(Action::Emit(Event::Press {
                pressed: true,
                location: at,
            }));
            out.push(Action::Emit(Event::Press {
                pressed: false,
                location: at,
            }));
            self.press_ended_at = now;
        }
        if !(self.derived.double_tap && self.wants_double_tap) {
            out.push(Action::Emit(Event::Tap(at)));
            return;
        }
        let near = |a: Point, b: Point| (a.x - b.x).hypot(a.y - b.y) <= DOUBLE_TAP_SLOP;
        let window = f64::from(DOUBLE_TAP_MS) / 1000.0;
        if self.wants_tap {
            match self.pending_tap.take() {
                Some((first, when, _)) if now - when <= window && near(first, at) => {
                    out.push(Action::Emit(Event::DoubleTap(at)));
                }
                stale => {
                    // A tap that outlived its interval without its timer (the app was
                    // suspended) is delivered now, ahead of the new one that takes its place.
                    if let Some((first, ..)) = stale {
                        out.push(Action::Emit(Event::Tap(first)));
                    }
                    let token = self.token();
                    self.pending_tap = Some((at, now, token));
                    out.push(Action::ArmTap(token));
                }
            }
        } else {
            match self.last_tap.take() {
                Some((first, when)) if now - when <= window && near(first, at) => {
                    out.push(Action::Emit(Event::DoubleTap(at)));
                }
                _ => self.last_tap = Some((at, now)),
            }
            out.push(Action::Emit(Event::Tap(at)));
        }
    }

    /// The long-press timer `token` fired: a long press, if that press is still held still.
    pub fn long_press_due(&mut self, token: u64, out: &mut Vec<Action>) {
        if token == 0 || token != self.long_press_token || self.moved {
            return;
        }
        let Some(at) = self.pressed else { return };
        self.long_press_token = 0;
        self.held = true;
        out.push(Action::Emit(Event::LongPress(at)));
    }

    /// The double-tap timer `token` fired: the single tap it held back, if no second tap came.
    pub fn tap_due(&mut self, token: u64, out: &mut Vec<Action>) {
        if let Some((at, _, pending)) = self.pending_tap
            && pending == token
        {
            self.pending_tap = None;
            out.push(Action::Emit(Event::Tap(at)));
        }
    }

    /// End a press the node can no longer finish (its subtree was disabled).
    pub fn cancel(&mut self, now: f64, out: &mut Vec<Action>) {
        if let Some(at) = self.pressed {
            self.press_out(at, now, out);
            if !self.derived.press {
                out.push(Action::Emit(Event::Press {
                    pressed: false,
                    location: at,
                }));
            }
        }
        self.pending_tap = None;
        self.gate = AxisGate::Idle;
    }
}

#[derive(Default)]
struct NodeState {
    recognizer: Recognizer,
    drag: VelocityTracker,
    pan: VelocityTracker,
    /// The pan's position so far: its deltas summed, which is what the tracker samples.
    pan_position: Point,
    /// The latest velocity of the drag and of the pan, as handlers read it.
    velocity: [Point; 2],
    /// The velocity the backend's own recognizer reported for the gesture about to end.
    native_velocity: [Option<Point>; 2],
}

fn slot(kind: GestureKind) -> usize {
    usize::from(kind == GestureKind::Pan)
}

fn with_state<R>(id: NodeId, f: impl FnOnce(&mut NodeState) -> R) -> R {
    NODES.with(|n| f(n.borrow_mut().entry(id).or_default()))
}

fn pointer_signal() -> Signal<PointerKind> {
    POINTER.with(|p| {
        if let Some(s) = p.get() {
            s
        } else {
            let initial = if crate::tree::has_tree() {
                with_tree(|t| t.primary_pointer())
            } else {
                PointerKind::platform_default()
            };
            let s = Signal::global(initial);
            p.set(Some(s));
            s
        }
    })
}

/// The kind of pointer the person is using: touch, mouse or stylus, following the last input
/// (docs/gestures.md "Input kind"). A tracked read, so wording and affordances that depend on it
/// update when an iPad's trackpad or a laptop's touch screen comes into use. Before any input
/// it is the platform's usual kind.
#[must_use]
pub fn primary_pointer() -> PointerKind {
    pointer_signal().get()
}

/// The velocity of `node`'s drag or pan (`kind`) as of the event being handled, in points per
/// second: the backend's own measurement at the release where it sent one, day-core's estimate
/// from the event samples otherwise. What `Drag::velocity` and `Pan::velocity` carry.
#[must_use]
pub fn velocity(node: NodeId, kind: GestureKind) -> Point {
    NODES.with(|n| {
        n.borrow()
            .get(&node)
            .map_or(Point::ZERO, |s| s.velocity[slot(kind)])
    })
}

/// How the active toolkit delivers a gesture (docs/gestures.md "Recognizers"): `Native` for the
/// platform's own recognizer, `Emulated` where day-core derives it from taps and drags (a
/// derived `Touches` reports one contact), `Unsupported` where it never fires.
#[must_use]
pub fn gesture_support(kind: GestureKind) -> Support {
    match with_tree(|t| t.gesture_support(kind)) {
        Support::Unsupported
            if matches!(
                kind,
                GestureKind::LongPress
                    | GestureKind::DoubleTap
                    | GestureKind::Press
                    | GestureKind::Touches
            ) =>
        {
            Support::Emulated
        }
        other => other,
    }
}

/// Record that `node` enabled `kind`, and whether day-core derives it (the tree's
/// `enable_gesture` decides, since it owns the toolkit).
pub(crate) fn note_enabled(node: NodeId, kind: GestureKind, derived: bool) {
    with_state(node, |s| {
        let r = &mut s.recognizer;
        match kind {
            GestureKind::Tap => r.wants_tap = true,
            GestureKind::Drag => r.wants_drag = true,
            GestureKind::DoubleTap => {
                r.wants_double_tap = true;
                r.derived.double_tap = derived;
            }
            GestureKind::LongPress => r.derived.long_press = derived,
            GestureKind::Press => r.derived.press = derived,
            GestureKind::Touches => r.derived.touches = derived,
            _ => {}
        }
    });
}

/// Record the axis `node`'s drag follows (`Decorate::on_drag_along`).
pub(crate) fn set_axis(node: NodeId, axis: Option<DragAxis>) {
    with_state(node, |s| s.recognizer.axis = axis);
}

/// Drop a removed node's gesture state (the tree's removal path calls it).
pub(crate) fn forget(node: NodeId) {
    let _ = NODES.try_with(|n| n.borrow_mut().remove(&node));
}

fn is_input(ev: &Event) -> bool {
    matches!(
        ev,
        Event::Pressed
            | Event::Tap(_)
            | Event::DoubleTap(_)
            | Event::LongPress(_)
            | Event::Press { pressed: true, .. }
            | Event::Touches(_)
            | Event::Drag { .. }
            | Event::Pan { .. }
            | Event::Pinch { .. }
            | Event::Rotate { .. }
            | Event::Hover {
                phase: DragPhase::Began | DragPhase::Changed,
                ..
            }
            | Event::Key(_)
            | Event::ContextMenu { .. }
            // A link in a label is a control too: a disabled group's links do not open.
            | Event::LinkActivated(_)
    )
}

fn target(id: NodeId, kind: GestureKind) -> NodeId {
    if id == day_spec::WINDOW_NODE {
        return id;
    }
    with_tree(|t| t.gesture_target(id_to_rnode(id), kind)).map_or(id, rnode_to_id)
}

/// Run a recognizer step for `node` and carry out what it asks: collect the events to deliver
/// and start the timers.
fn step(
    node: NodeId,
    f: &mut dyn FnMut(&mut Recognizer, &mut Vec<Action>),
) -> Vec<(NodeId, Event)> {
    let mut actions = Vec::new();
    with_state(node, |s| f(&mut s.recognizer, &mut actions));
    let mut out = Vec::new();
    for action in actions {
        match action {
            Action::Emit(ev) => out.push((node, ev)),
            Action::ArmLongPress(token) => arm(node, LONG_PRESS_MS, Due::LongPress(token)),
            Action::ArmTap(token) => arm(node, DOUBLE_TAP_MS, Due::Tap(token)),
        }
    }
    out
}

/// Send the taps or drags (`kind`) in `events` on to the nearest piece above `node` that
/// handles them, when `node` itself only watched them to derive a press: a pressable row
/// inside a tappable card must not swallow the card's tap.
///
/// Each one arrives at the piece above as if it had been delivered there: its point moved into
/// that piece's coordinates, through that piece's own recognizer (its drag axis, its double
/// tap, its own press), and a drag carries the release velocity `node` measured or was sent.
fn pass_on(
    node: NodeId,
    kind: GestureKind,
    events: Vec<(NodeId, Event)>,
    now: f64,
) -> Vec<(NodeId, Event)> {
    let handles_it = NODES.with(|n| {
        n.borrow().get(&node).is_none_or(|s| {
            let r = &s.recognizer;
            // Only a node day-core attached the recognizer to for its own derivation can
            // have received an event it does not handle.
            let watched = r.derived.press || r.derived.long_press;
            match kind {
                GestureKind::Tap => !watched || r.wants_tap || r.wants_double_tap,
                _ => !(watched || r.derived.touches) || r.wants_drag,
            }
        })
    });
    if handles_it {
        return events;
    }
    let Some((above, offset)) = with_tree(|t| {
        let below = id_to_rnode(node);
        let above = t.gesture_ancestor(below, kind)?;
        // Before a layout there is no offset to apply; the point is the best there is.
        Some((above, t.offset_in(below, above).unwrap_or(Point::ZERO)))
    }) else {
        return events;
    };
    let above = rnode_to_id(above);
    let moved = |p: Point| Point::new(p.x + offset.x, p.y + offset.y);
    let mut out = Vec::new();
    for (id, ev) in events {
        match (kind, ev) {
            (GestureKind::Tap, Event::Tap(at)) => out.extend(tap_at(above, moved(at), now)),
            (
                GestureKind::Drag,
                Event::Drag {
                    phase,
                    location,
                    translation,
                },
            ) => {
                let released =
                    (phase == DragPhase::Ended).then(|| velocity(node, GestureKind::Drag));
                out.extend(drag_at(
                    above,
                    phase,
                    moved(location),
                    translation,
                    now,
                    released,
                ));
            }
            (_, ev) => out.push((id, ev)),
        }
    }
    out
}

/// Deliver a drag to `node` (already resolved as its target): track its velocity, run its
/// recognizer, and pass it on if `node` only watched it. `native` is a release velocity a
/// recognizer below already settled on, which wins over this node's estimate like the
/// backend's own measurement does.
fn drag_at(
    node: NodeId,
    phase: DragPhase,
    location: Point,
    translation: Point,
    now: f64,
    native: Option<Point>,
) -> Vec<(NodeId, Event)> {
    with_state(node, |s| {
        if phase == DragPhase::Began {
            s.drag.reset();
            s.native_velocity[0] = None;
        }
        if native.is_some() {
            s.native_velocity[0] = native;
        }
        s.drag.push(now, translation);
        s.velocity[0] = match (phase, s.native_velocity[0].take()) {
            (DragPhase::Ended, Some(native)) => native,
            _ => s.drag.velocity(now),
        };
    });
    let out = step(node, &mut |r, out| {
        r.on_drag(phase, location, translation, now, out);
    });
    pass_on(node, GestureKind::Drag, out, now)
}

/// Deliver a tap to `node` (already resolved as its target): its recognizer holds it back for
/// a double tap or turns it into one, and it is passed on if `node` only watched it.
fn tap_at(node: NodeId, at: Point, now: f64) -> Vec<(NodeId, Event)> {
    // Nothing is derived for a node that never enabled a gesture through the tree.
    if NODES.with(|n| n.borrow().contains_key(&node)) {
        let out = step(node, &mut |r, out| r.on_tap(at, now, out));
        pass_on(node, GestureKind::Tap, out, now)
    } else {
        vec![(node, Event::Tap(at))]
    }
}

/// What a timer answers when it fires.
#[derive(Clone, Copy)]
enum Due {
    LongPress(u64),
    Tap(u64),
}

fn arm(node: NodeId, ms: u32, due: Due) {
    crate::task(async move {
        crate::sleep(ms).await;
        // The node may be gone by now; a state entry is only read, never created, here.
        if !NODES.with(|n| n.borrow().contains_key(&node)) {
            return;
        }
        let events = step(node, &mut |r, out| match due {
            Due::LongPress(token) => r.long_press_due(token, out),
            Due::Tap(token) => r.tap_due(token, out),
        });
        for (id, ev) in events {
            crate::tree::dispatch_routed(id, &ev);
        }
    });
}

/// Enable or disable everything under `node` (`Decorate::enabled`, docs/forms.md "Disabling a
/// group"): the toolkit disables the native controls, and from here on day-core drops the
/// input addressed to the subtree. A press under way when the subtree is disabled ends.
pub fn set_subtree_enabled(node: RNode, enabled: bool) {
    with_tree(|t| t.set_subtree_enabled(node, enabled));
    if !enabled {
        cancel_disabled();
    }
}

/// End the presses under a subtree that was just disabled, so nothing stays pressed.
fn cancel_disabled() {
    let pressed: Vec<NodeId> = NODES.with(|n| {
        n.borrow()
            .iter()
            .filter(|(_, s)| s.recognizer.is_pressed())
            .map(|(id, _)| *id)
            .collect()
    });
    let now = crate::frame::now_secs();
    for id in pressed {
        if with_tree(|t| t.subtree_disabled(id_to_rnode(id))) {
            for (id, ev) in step(id, &mut |r, out| r.cancel(now, out)) {
                crate::tree::dispatch_routed(id, &ev);
            }
        }
    }
}

/// Route one event on its way to the handlers: the events to deliver in its place, each with
/// the node it goes to. Most events come back unchanged; see the module docs for the rest.
pub(crate) fn route(id: NodeId, ev: Event) -> Vec<(NodeId, Event)> {
    match &ev {
        Event::PointerKindChanged(kind) => {
            pointer_signal().set(*kind);
            return Vec::new();
        }
        Event::GestureVelocity { kind, velocity } => {
            let node = target(id, *kind);
            with_state(node, |s| s.native_velocity[slot(*kind)] = Some(*velocity));
            return Vec::new();
        }
        _ => {}
    }
    if id != day_spec::WINDOW_NODE
        && is_input(&ev)
        && with_tree(|t| t.subtree_disabled(id_to_rnode(id)))
    {
        return Vec::new();
    }
    let now = crate::frame::now_secs();
    match ev {
        Event::Drag {
            phase,
            location,
            translation,
        } => drag_at(
            target(id, GestureKind::Drag),
            phase,
            location,
            translation,
            now,
            None,
        ),
        Event::Pan { phase, delta, .. } => {
            let node = target(id, GestureKind::Pan);
            with_state(node, |s| {
                if phase == DragPhase::Began {
                    s.pan.reset();
                    s.pan_position = Point::ZERO;
                    s.native_velocity[1] = None;
                }
                s.pan_position = Point::new(s.pan_position.x + delta.x, s.pan_position.y + delta.y);
                s.pan.push(now, s.pan_position);
                s.velocity[1] = match (phase, s.native_velocity[1].take()) {
                    (DragPhase::Ended, Some(native)) => native,
                    _ => s.pan.velocity(now),
                };
            });
            vec![(node, ev)]
        }
        Event::Tap(at) => tap_at(target(id, GestureKind::Tap), at, now),
        Event::Press { pressed, location } => {
            let node = target(id, GestureKind::Press);
            step(node, &mut |r, out| r.on_press(pressed, location, now, out))
        }
        other => vec![(id, other)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emitted(actions: &[Action]) -> Vec<Event> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Emit(ev) => Some(ev.clone()),
                _ => None,
            })
            .collect()
    }

    fn p(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn velocity_is_distance_over_the_last_tenth_of_a_second() {
        let mut v = VelocityTracker::default();
        assert_eq!(v.velocity(0.0), Point::ZERO, "no samples");
        v.push(0.00, p(0.0, 0.0));
        assert_eq!(v.velocity(0.0), Point::ZERO, "one sample");
        v.push(0.05, p(50.0, -25.0));
        let now = v.velocity(0.05);
        assert!((now.x - 1000.0).abs() < 1e-6 && (now.y + 500.0).abs() < 1e-6);
        // An older stretch drops out of the window: only the last 100 ms count.
        v.push(0.20, p(60.0, -25.0));
        v.push(0.25, p(80.0, -25.0));
        let later = v.velocity(0.25);
        assert!((later.x - 400.0).abs() < 1e-6 && later.y.abs() < 1e-6);
        // A pointer that rested before the release has no velocity.
        assert_eq!(v.velocity(0.50), Point::ZERO);
        // Samples that arrive at once measure nothing, they do not divide by zero.
        let mut burst = VelocityTracker::default();
        burst.push(1.0, p(0.0, 0.0));
        burst.push(1.0, p(90.0, 0.0));
        assert_eq!(burst.velocity(1.0), Point::ZERO);
        burst.reset();
        assert_eq!(burst.velocity(1.0), Point::ZERO);
    }

    #[test]
    fn press_and_long_press_are_derived_from_drag_phases() {
        let mut r = Recognizer {
            derived: Derived {
                press: true,
                long_press: true,
                ..Derived::default()
            },
            ..Recognizer::default()
        };
        let mut out = Vec::new();
        r.on_drag(DragPhase::Began, p(5.0, 5.0), Point::ZERO, 0.0, &mut out);
        let token = out
            .iter()
            .find_map(|a| match a {
                Action::ArmLongPress(t) => Some(*t),
                _ => None,
            })
            .expect("a still press arms the long press");
        assert!(matches!(
            emitted(&out)[0],
            Event::Press { pressed: true, .. }
        ));
        assert!(r.is_pressed());
        out.clear();
        r.long_press_due(token, &mut out);
        assert_eq!(emitted(&out), vec![Event::LongPress(p(5.0, 5.0))]);
        out.clear();
        r.on_drag(DragPhase::Ended, p(5.0, 5.0), Point::ZERO, 0.6, &mut out);
        assert!(matches!(
            emitted(&out)[0],
            Event::Press { pressed: false, .. }
        ));
        // The release of a long press is not a tap.
        out.clear();
        r.on_tap(p(5.0, 5.0), 0.61, &mut out);
        assert!(emitted(&out).is_empty(), "{out:?}");
        // The tap after that is one again, and it brings no second press pair with it while
        // the first press is this recent.
        r.on_tap(p(5.0, 5.0), 0.7, &mut out);
        assert_eq!(emitted(&out), vec![Event::Tap(p(5.0, 5.0))]);
    }

    #[test]
    fn a_press_that_moves_or_lifts_never_becomes_a_long_press() {
        let derived = Derived {
            long_press: true,
            ..Derived::default()
        };
        let mut moved = Recognizer {
            derived,
            ..Recognizer::default()
        };
        let mut out = Vec::new();
        moved.on_drag(DragPhase::Began, p(0.0, 0.0), Point::ZERO, 0.0, &mut out);
        let Some(Action::ArmLongPress(token)) = out.first().cloned() else {
            panic!("armed: {out:?}")
        };
        moved.on_drag(
            DragPhase::Changed,
            p(30.0, 0.0),
            p(30.0, 0.0),
            0.1,
            &mut out,
        );
        out.clear();
        moved.long_press_due(token, &mut out);
        assert!(out.is_empty(), "moved past the slop: {out:?}");

        let mut lifted = Recognizer {
            derived,
            ..Recognizer::default()
        };
        lifted.on_drag(DragPhase::Began, p(0.0, 0.0), Point::ZERO, 0.0, &mut out);
        let Some(Action::ArmLongPress(token)) = out.first().cloned() else {
            panic!("armed: {out:?}")
        };
        lifted.on_drag(DragPhase::Ended, p(0.0, 0.0), Point::ZERO, 0.1, &mut out);
        out.clear();
        lifted.long_press_due(token, &mut out);
        assert!(out.is_empty(), "lifted first: {out:?}");
        // A press derived for nobody emits no press events of its own.
        assert!(
            !emitted(&out)
                .iter()
                .any(|e| matches!(e, Event::Press { .. }))
        );
    }

    #[test]
    fn a_tap_alone_is_a_whole_press_where_the_drag_needs_movement() {
        let mut r = Recognizer {
            derived: Derived {
                press: true,
                ..Derived::default()
            },
            wants_tap: true,
            ..Recognizer::default()
        };
        let mut out = Vec::new();
        r.on_tap(p(1.0, 2.0), 5.0, &mut out);
        assert_eq!(
            emitted(&out),
            vec![
                Event::Press {
                    pressed: true,
                    location: p(1.0, 2.0)
                },
                Event::Press {
                    pressed: false,
                    location: p(1.0, 2.0)
                },
                Event::Tap(p(1.0, 2.0)),
            ]
        );
        // A backend that sends the zero-length drag AND the tap gets one pair, from the drag.
        out.clear();
        r.on_drag(DragPhase::Began, p(1.0, 2.0), Point::ZERO, 9.0, &mut out);
        r.on_drag(DragPhase::Ended, p(1.0, 2.0), Point::ZERO, 9.05, &mut out);
        r.on_tap(p(1.0, 2.0), 9.06, &mut out);
        let presses = emitted(&out)
            .iter()
            .filter(|e| matches!(e, Event::Press { .. }))
            .count();
        assert_eq!(presses, 2, "{out:?}");
    }

    #[test]
    fn a_single_tap_waits_only_when_a_double_tap_is_handled_too() {
        // Tap alone: straight through.
        let mut plain = Recognizer {
            wants_tap: true,
            ..Recognizer::default()
        };
        let mut out = Vec::new();
        plain.on_tap(p(0.0, 0.0), 0.0, &mut out);
        assert_eq!(out, vec![Action::Emit(Event::Tap(p(0.0, 0.0)))]);

        // Both: the first tap is held, and delivered when its timer fires.
        let both = || Recognizer {
            derived: Derived {
                double_tap: true,
                ..Derived::default()
            },
            wants_tap: true,
            wants_double_tap: true,
            ..Recognizer::default()
        };
        let mut r = both();
        out.clear();
        r.on_tap(p(3.0, 3.0), 1.0, &mut out);
        let [Action::ArmTap(token)] = out[..] else {
            panic!("held: {out:?}")
        };
        out.clear();
        r.tap_due(token, &mut out);
        assert_eq!(emitted(&out), vec![Event::Tap(p(3.0, 3.0))]);

        // Both, and a second tap inside the interval: one double tap, no single tap, and the
        // first tap's timer finds nothing left to deliver.
        let mut r = both();
        out.clear();
        r.on_tap(p(3.0, 3.0), 1.0, &mut out);
        let [Action::ArmTap(token)] = out[..] else {
            panic!("held: {out:?}")
        };
        out.clear();
        r.on_tap(p(4.0, 4.0), 1.2, &mut out);
        assert_eq!(emitted(&out), vec![Event::DoubleTap(p(4.0, 4.0))]);
        out.clear();
        r.tap_due(token, &mut out);
        assert_eq!(out, Vec::<Action>::new());

        // Both, but the second tap lands far away: two single taps.
        let mut r = both();
        out.clear();
        r.on_tap(p(0.0, 0.0), 1.0, &mut out);
        r.on_tap(p(200.0, 0.0), 1.1, &mut out);
        assert_eq!(emitted(&out), vec![Event::Tap(p(0.0, 0.0))]);

        // Double tap alone: nothing waits.
        let mut only = Recognizer {
            derived: Derived {
                double_tap: true,
                ..Derived::default()
            },
            wants_double_tap: true,
            ..Recognizer::default()
        };
        out.clear();
        only.on_tap(p(0.0, 0.0), 1.0, &mut out);
        only.on_tap(p(1.0, 0.0), 1.1, &mut out);
        assert_eq!(
            emitted(&out),
            vec![
                Event::Tap(p(0.0, 0.0)),
                Event::DoubleTap(p(1.0, 0.0)),
                Event::Tap(p(1.0, 0.0))
            ]
        );
    }

    #[test]
    fn a_drag_along_one_axis_withholds_the_other() {
        let drags = |r: &mut Recognizer, to: Point| {
            let mut out = Vec::new();
            r.on_drag(DragPhase::Began, p(10.0, 10.0), Point::ZERO, 0.0, &mut out);
            r.on_drag(
                DragPhase::Changed,
                p(10.0 + to.x, 10.0 + to.y),
                to,
                0.01,
                &mut out,
            );
            r.on_drag(
                DragPhase::Ended,
                p(10.0 + to.x, 10.0 + to.y),
                to,
                0.02,
                &mut out,
            );
            emitted(&out)
                .into_iter()
                .filter_map(|e| match e {
                    Event::Drag { phase, .. } => Some(phase),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let mut r = Recognizer {
            axis: Some(DragAxis::Horizontal),
            ..Recognizer::default()
        };
        assert_eq!(
            drags(&mut r, p(40.0, 5.0)),
            vec![DragPhase::Began, DragPhase::Changed, DragPhase::Ended]
        );
        assert_eq!(drags(&mut r, p(5.0, 40.0)), Vec::<DragPhase>::new());
        // A press that never moves is nobody's drag.
        assert_eq!(drags(&mut r, Point::ZERO), Vec::<DragPhase>::new());
        // And the gate reopens for the next gesture.
        assert_eq!(drags(&mut r, p(-40.0, 0.0)).len(), 3);
        // A flick whose movement comes only with the release is decided there.
        let flick = |r: &mut Recognizer, to: Point| {
            let mut out = Vec::new();
            r.on_drag(DragPhase::Began, p(10.0, 10.0), Point::ZERO, 0.0, &mut out);
            r.on_drag(DragPhase::Ended, p(10.0 + to.x, 10.0), to, 0.02, &mut out);
            emitted(&out).len()
        };
        assert_eq!(flick(&mut r, p(30.0, 0.0)), 2, "Began and Ended");
        assert_eq!(flick(&mut r, p(0.0, 30.0)), 0);
        assert_eq!(flick(&mut r, p(1.0, 0.0)), 0, "too short to be a drag");
        // A single contact is derived from the same phases.
        let mut touches = Recognizer {
            derived: Derived {
                touches: true,
                ..Derived::default()
            },
            ..Recognizer::default()
        };
        let mut out = Vec::new();
        touches.on_drag(DragPhase::Began, p(1.0, 1.0), Point::ZERO, 0.0, &mut out);
        touches.on_drag(DragPhase::Ended, p(2.0, 1.0), p(1.0, 0.0), 0.1, &mut out);
        let phases: Vec<TouchPhase> = emitted(&out)
            .iter()
            .filter_map(|e| match e {
                Event::Touches(t) => Some(t[0].phase),
                _ => None,
            })
            .collect();
        assert_eq!(phases, vec![TouchPhase::Began, TouchPhase::Ended]);
    }
}
