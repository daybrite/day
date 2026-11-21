// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Pointer and key input on GTK 4's own controllers (docs/gestures.md, docs/menus.md "Keys",
//! docs/focus.md).
//!
//! Every Day recognizer on a widget is a `GtkGesture` in ONE gesture group, so a drag that
//! claims its sequence (the claim rule, docs/gestures.md "Gestures inside a scroll view") claims
//! it for the widget's tap, long press and press observer too instead of denying them, while
//! an enclosing `GtkScrolledWindow` and every other widget lose it. The pieces:
//!
//! - **Tap and double tap**: one `GtkGestureClick`. Its `n_press` pairs clicks with the
//!   desktop's double-click time and distance; a single tap waits out that time only on a
//!   widget that hears double taps too. A press that moved past the drag threshold, or that
//!   fired a long press, is no tap.
//! - **Long press**: `GtkGestureLongPress` (the desktop's long-press time, cancelled past the
//!   drag threshold).
//! - **Press**: a `GtkGestureDrag` that never claims, used as an observer: in at its begin, out
//!   at its end, at its cancel (an ancestor claimed the sequence), when the pointer leaves the
//!   widget's hit area, or when the widget unmaps.
//! - **Drag**: `GtkGestureDrag`. A touch sequence is claimed once it has traveled
//!   [`AXIS_JUDGE`]; with an axis, that travel decides between claiming and denying it (a mouse
//!   sequence is not contested, and only the axis rule applies). A `GtkGestureSwipe` in the
//!   group measures the release velocity of an unclaimed (mouse) drag.
//! - **Touches**: a capture-phase `GtkEventControllerLegacy` reading touch sequences (and the
//!   primary mouse button as one contact).
//! - **Hit area**: [`DayFixed`] (containers) and [`DayCanvas`] (canvases and shapes) answer
//!   GTK's `contains` with the hit slop, and with nothing at all while no input was asked of
//!   them, so a passive piece lets presses through to what is behind it.
//! - **Keys**: [`HeldKeys`] tracks the keys whose `Down` a controller delivered: GTK repeats a
//!   held key as more `key-pressed` emissions, and `key-released` is the release.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use day_spec::sidetable::SideTable;
use day_spec::{
    DragAxis, DragPhase, Event, GestureKind, Insets, Key, KeyEvent, KeyPhase, Modifiers, NodeId,
    Point, PointerKind, Touch, TouchPhase, ffi_guard,
};
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gdk, glib};

use crate::{Handle, emit, full_route_key, key_modifiers, key_name, widget_key};

/// How far a drag travels before it is judged (claimed, or with an axis, claimed or denied),
/// in points. Below `GtkScrolledWindow`'s own claim at the drag threshold (8 px by default), so
/// a drag decides before an enclosing scroll view takes a touch sequence; and day-core judges
/// an axis at the same 3 points, so the two agree.
const AXIS_JUDGE: f64 = 3.0;

day_core::tls_group! {
    /// Each widget's gesture state, swept with the widget at release.
    static INPUT: SideTable<Rc<Input>> = SideTable::new();
    /// The input kind last reported (`Toolkit::primary_pointer` answers the platform default,
    /// a mouse, before any input).
    static POINTER: Cell<PointerKind> = const { Cell::new(PointerKind::Mouse) };
}

/// Where a widget's drag stands in its pointer sequence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Track {
    /// No sequence.
    #[default]
    Idle,
    /// Down, with an axis, not yet judged: nothing sent.
    Pending,
    /// The node's drag: `Began` was sent; `claimed` once it traveled far enough.
    Mine { claimed: bool },
    /// Set off across the axis: denied, nothing sent.
    NotMine,
}

/// What a drag's travel decides ([`judge`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Judgment {
    /// Not far enough yet.
    Wait,
    /// The node's drag: claim the sequence.
    Claim,
    /// Across the axis: the sequence is not the node's.
    Deny,
}

/// Judge a drag that has traveled `offset`: wait until it has gone [`AXIS_JUDGE`], then claim
/// it, unless an axis is set and the travel runs across it (`DragAxis::leads`, the rule
/// day-core applies to the handler).
fn judge(axis: Option<DragAxis>, offset: Point) -> Judgment {
    if offset.x.hypot(offset.y) < AXIS_JUDGE {
        return Judgment::Wait;
    }
    match axis {
        Some(axis) if !axis.leads(offset) => Judgment::Deny,
        _ => Judgment::Claim,
    }
}

/// What one click release means ([`click_outcome`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Click {
    /// Nothing for this widget.
    Nothing,
    /// A tap, now.
    Tap,
    /// A tap, once the double-click time passes with no second click.
    HeldTap,
    /// A double tap.
    DoubleTap,
}

/// The release of the `n`th click in a run (GTK's `n_press`) on a widget that hears taps
/// (`tap`) and double taps (`double`): an even click completes a pair, so a triple click is a
/// double tap and then a single tap that waits.
fn click_outcome(n: i32, tap: bool, double: bool) -> Click {
    let pair = n > 0 && n % 2 == 0;
    match (pair, tap, double) {
        (true, _, true) => Click::DoubleTap,
        (false, true, true) => Click::HeldTap,
        (_, true, _) => Click::Tap,
        _ => Click::Nothing,
    }
}

/// Whether `(x, y)`, in a `width` × `height` widget's own coordinates, falls inside its bounds
/// grown by `slop` (docs/gestures.md "Hit slop"); leading and trailing follow `rtl`.
fn slop_contains(width: f64, height: f64, slop: Insets, rtl: bool, x: f64, y: f64) -> bool {
    let (left, right) = if rtl {
        (slop.trailing, slop.leading)
    } else {
        (slop.leading, slop.trailing)
    };
    x >= -left && y >= -slop.top && x <= width + right && y <= height + slop.bottom
}

/// The input kind behind a pointer event: a touch sequence is a finger; a pen or eraser
/// (named by the event's tool, or by its device) is a stylus; everything else that points is
/// a mouse. (docs/gestures.md "Input kind": trackpads are mice.)
fn kind_for(
    touch: bool,
    source: Option<gdk::InputSource>,
    tool: Option<gdk::DeviceToolType>,
) -> PointerKind {
    use gdk::DeviceToolType as T;
    if touch {
        return PointerKind::Touch;
    }
    if matches!(
        tool,
        Some(T::Pen | T::Eraser | T::Brush | T::Pencil | T::Airbrush)
    ) {
        return PointerKind::Stylus;
    }
    match source {
        Some(gdk::InputSource::Pen) => PointerKind::Stylus,
        Some(gdk::InputSource::Touchscreen) => PointerKind::Touch,
        _ => PointerKind::Mouse,
    }
}

/// Which of a widget's contacts an event is about: the mouse's primary button, or one touch
/// sequence (by its pointer, stable for the sequence's life).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContactSource {
    Mouse,
    Sequence(usize),
}

/// One contact on a widget.
#[derive(Clone, Copy, Debug)]
struct Contact {
    source: ContactSource,
    touch: Touch,
}

/// The contacts down on a widget (docs/gestures.md "Touches"): each change answers the list
/// `Event::Touches` carries, every contact still down plus the one that changed.
#[derive(Debug, Default)]
struct Contacts {
    live: Vec<Contact>,
}

impl Contacts {
    /// Apply one change and answer the list to report, or `None` when the change concerns no
    /// contact this widget holds (a move or an end for a sequence that began elsewhere).
    fn apply(&mut self, source: ContactSource, touch: Touch) -> Option<Vec<Touch>> {
        let at = self.live.iter().position(|c| c.source == source);
        match (touch.phase, at) {
            (TouchPhase::Began, None) => {
                // The lowest id not in use, so a lone mouse is contact 0. Among `len + 1`
                // ids one is always free.
                let id = (0..=self.live.len() as u64)
                    .find(|id| self.live.iter().all(|c| c.touch.id != *id))
                    .unwrap_or_default();
                let mut touch = touch;
                touch.id = id;
                self.live.push(Contact { source, touch });
                self.live.sort_by_key(|c| c.touch.id);
            }
            (TouchPhase::Began, Some(_)) | (_, None) => return None,
            (_, Some(i)) => {
                let id = self.live[i].touch.id;
                let mut touch = touch;
                touch.id = id;
                self.live[i].touch = touch;
            }
        }
        let list = self
            .live
            .iter()
            .map(|c| {
                if c.source == source {
                    c.touch
                } else {
                    let mut still = c.touch;
                    still.phase = TouchPhase::Stationary;
                    still
                }
            })
            .collect();
        self.live.retain(|c| c.touch.is_down());
        Some(list)
    }

    /// The platform took every contact away: the list that says so, or `None` when none was
    /// down.
    fn cancel_all(&mut self) -> Option<Vec<Touch>> {
        if self.live.is_empty() {
            return None;
        }
        let list = self
            .live
            .drain(..)
            .map(|c| {
                let mut gone = c.touch;
                gone.phase = TouchPhase::Cancelled;
                gone
            })
            .collect();
        Some(list)
    }
}

/// The keys whose `Down` a key controller delivered, by hardware keycode (a held key keeps its
/// code whatever Shift does to its keyval), with the `Key` that down named: a further press of
/// one is the platform's auto-repeat, and its release is the `Up` the same node hears.
#[derive(Clone, Default)]
pub(crate) struct HeldKeys(Rc<RefCell<Vec<(u32, Key)>>>);

impl HeldKeys {
    /// Whether the key with `code` is down here already: a press of it is a repeat.
    pub(crate) fn is_held(&self, code: u32) -> bool {
        self.0.borrow().iter().any(|(c, _)| *c == code)
    }

    /// The key with `code` went down and its press was delivered as `key`.
    pub(crate) fn hold(&self, code: u32, key: Key) {
        if !self.is_held(code) {
            self.0.borrow_mut().push((code, key));
        }
    }

    /// The key with `code` came up: the key its down named, if that down was delivered.
    pub(crate) fn release(&self, code: u32) -> Option<Key> {
        let mut held = self.0.borrow_mut();
        let i = held.iter().position(|(c, _)| *c == code)?;
        Some(held.remove(i).1)
    }

    /// Focus left: no release that follows is the node's.
    pub(crate) fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    /// Deliver a press of the key with `code` as `event` (a `Down`) to `id`: a repeat when the
    /// key is already down, else a down that its release will answer.
    pub(crate) fn deliver(&self, id: NodeId, code: u32, event: KeyEvent, repeat: bool) {
        let event = if repeat {
            event.with_phase(KeyPhase::Repeat)
        } else {
            self.hold(code, event.key);
            event
        };
        emit(id, Event::Key(event));
    }

    /// The release of the key with `code`, with `state` held: its `Up` to `id`, when its down
    /// was delivered.
    pub(crate) fn deliver_release(&self, id: NodeId, code: u32, state: gdk::ModifierType) {
        if let Some(key) = self.release(code) {
            let modifiers = Modifiers::from_bits(key_modifiers(state));
            emit(
                id,
                Event::Key(KeyEvent::new(key, modifiers).with_phase(KeyPhase::Up)),
            );
        }
    }
}

/// Clear `held` whenever focus leaves `w` and everything inside it.
pub(crate) fn clear_on_blur(w: &impl IsA<gtk4::Widget>, held: &HeldKeys) {
    let focus = gtk4::EventControllerFocus::new();
    let held = held.clone();
    focus.connect_leave(move |_| held.clear());
    w.add_controller(focus);
}

/// One widget's gesture state, shared by its controllers' handlers.
#[derive(Default)]
struct Input {
    /// The node each enabled kind reports to: wrappers can bring several nodes' modifiers down
    /// to one native view.
    nodes: RefCell<Vec<(GestureKind, NodeId)>>,
    /// The widget's first Day gesture: every later one joins its group.
    leader: glib::WeakRef<gtk4::Gesture>,
    /// `.on_drag_along`'s axis.
    axis: Cell<Option<DragAxis>>,
    /// Which shared recognizers exist: the click (tap and double tap), the press observer.
    has_click: Cell<bool>,
    has_watch: Cell<bool>,
    /// The press point of the current click, whether it moved past the drag threshold, and
    /// whether it fired a long press: either rules the tap out.
    click_at: Cell<Point>,
    click_moved: Cell<bool>,
    held: Cell<bool>,
    /// A single tap waiting out the double-click time, with its timer.
    pending: RefCell<Option<(glib::SourceId, Point)>>,
    drag: Cell<Track>,
    drag_from: Cell<Point>,
    /// The release velocity `GtkGestureSwipe` measured for the current sequence.
    fling: Cell<Option<Point>>,
    drag_cancelled: Cell<bool>,
    /// The press under way (its last point), and where it began.
    press: Cell<Option<Point>>,
    press_from: Cell<Point>,
    contacts: RefCell<Contacts>,
}

impl Input {
    fn node(&self, kind: GestureKind) -> Option<NodeId> {
        self.nodes
            .borrow()
            .iter()
            .find_map(|(k, n)| (*k == kind).then_some(*n))
    }

    /// End the press under way, at its last point.
    fn press_out(&self, at: Option<Point>) {
        if let Some(last) = self.press.take()
            && let Some(node) = self.node(GestureKind::Press)
        {
            emit(
                node,
                Event::Press {
                    pressed: false,
                    location: at.unwrap_or(last),
                },
            );
        }
    }

    /// Report every contact as cancelled.
    fn touches_cancelled(&self) {
        let list = self.contacts.borrow_mut().cancel_all();
        if let (Some(list), Some(node)) = (list, self.node(GestureKind::Touches)) {
            emit(node, Event::Touches(list));
        }
    }

    /// Deliver a held single tap now (a new click took its place).
    fn flush_pending(&self) {
        let pending = self.pending.borrow_mut().take();
        if let Some((source, at)) = pending {
            source.remove();
            if let Some(node) = self.node(GestureKind::Tap) {
                emit(node, Event::Tap(at));
            }
        }
    }

    /// Forget a held single tap (the second click of a pair arrived).
    fn drop_pending(&self) {
        let pending = self.pending.borrow_mut().take();
        if let Some((source, _)) = pending {
            source.remove();
        }
    }
}

fn state(w: &Handle) -> Rc<Input> {
    let key = widget_key(w);
    INPUT.with(|t| {
        t.get(key).unwrap_or_else(|| {
            let input = Rc::new(Input::default());
            t.insert(key, input.clone());
            input
        })
    })
}

/// Add a Day gesture to `w`, in the widget's one gesture group.
pub(crate) fn add_gesture(w: &Handle, gesture: &impl IsA<gtk4::Gesture>) {
    w.add_controller(gesture.upcast_ref::<gtk4::Gesture>().clone());
    let input = state(w);
    match input.leader.upgrade() {
        Some(leader) if leader.widget().as_ref() == Some(w) => leader.group_with(gesture),
        _ => input.leader.set(Some(gesture.upcast_ref())),
    }
}

/// Record that `node` enabled `kind` on `w`: `false` when it was already wired.
pub(crate) fn note(w: &Handle, node: NodeId, kind: GestureKind) -> bool {
    let input = state(w);
    let mut nodes = input.nodes.borrow_mut();
    if nodes.iter().any(|(k, _)| *k == kind) {
        return false;
    }
    nodes.push((kind, node));
    true
}

/// `.on_drag_along`'s axis for `w`'s drag (`Toolkit::set_drag_axis`).
pub(crate) fn set_axis(w: &Handle, axis: Option<DragAxis>) {
    state(w).axis.set(axis);
}

fn settings() -> Option<gtk4::Settings> {
    gtk4::Settings::default()
}

/// The desktop's drag threshold, in points.
fn drag_threshold() -> f64 {
    settings().map_or(8.0, |s| f64::from(s.gtk_dnd_drag_threshold().max(1)))
}

/// Attach the recognizer for `kind` (one of the kinds this module owns) to `w`.
pub(crate) fn attach(w: &Handle, kind: GestureKind) {
    let input = state(w);
    take_input(w);
    match kind {
        GestureKind::Tap | GestureKind::DoubleTap => attach_click(w, &input),
        GestureKind::LongPress => attach_long_press(w, &input),
        GestureKind::Press => attach_watch(w, &input),
        GestureKind::Touches => {
            attach_watch(w, &input);
            attach_touches(w, &input);
        }
        GestureKind::Drag => attach_drag(w, &input),
        _ => {}
    }
}

fn attach_click(w: &Handle, input: &Rc<Input>) {
    if input.has_click.replace(true) {
        return;
    }
    let click = gtk4::GestureClick::new();
    click.connect_pressed({
        let input = input.clone();
        move |_, n, x, y| {
            ffi_guard::contain((), || {
                input.click_at.set(Point::new(x, y));
                input.click_moved.set(false);
                input.held.set(false);
                // The second click of a pair: the single tap it completes is not delivered.
                if n > 0 && n % 2 == 0 && input.node(GestureKind::DoubleTap).is_some() {
                    input.drop_pending();
                }
            });
        }
    });
    click.connect_update({
        let input = input.clone();
        move |g, sequence| {
            if let Some((x, y)) = g.point(sequence) {
                let from = input.click_at.get();
                if (x - from.x).hypot(y - from.y) > drag_threshold() {
                    input.click_moved.set(true);
                }
            }
        }
    });
    click.connect_released({
        let input = input.clone();
        move |_, n, x, y| {
            ffi_guard::contain((), || {
                if input.click_moved.get() || input.held.replace(false) {
                    return;
                }
                let at = Point::new(x, y);
                let tap = input.node(GestureKind::Tap);
                let double = input.node(GestureKind::DoubleTap);
                match click_outcome(n, tap.is_some(), double.is_some()) {
                    Click::Nothing => {}
                    Click::Tap => {
                        input.flush_pending();
                        if let Some(node) = tap {
                            emit(node, Event::Tap(at));
                        }
                    }
                    Click::HeldTap => {
                        input.flush_pending();
                        let wait = settings().map_or(400, |s| s.gtk_double_click_time().max(1));
                        let timer = {
                            let input = input.clone();
                            glib::timeout_add_local_once(
                                std::time::Duration::from_millis(u64::from(wait.unsigned_abs())),
                                move || {
                                    let due = input.pending.borrow_mut().take();
                                    if let (Some((_, at)), Some(node)) =
                                        (due, input.node(GestureKind::Tap))
                                    {
                                        ffi_guard::contain((), || emit(node, Event::Tap(at)));
                                    }
                                },
                            )
                        };
                        *input.pending.borrow_mut() = Some((timer, at));
                    }
                    Click::DoubleTap => {
                        input.drop_pending();
                        if let Some(node) = double {
                            emit(node, Event::DoubleTap(at));
                        }
                    }
                }
            });
        }
    });
    add_gesture(w, &click);
}

fn attach_long_press(w: &Handle, input: &Rc<Input>) {
    let long = gtk4::GestureLongPress::new();
    long.connect_pressed({
        let input = input.clone();
        move |g, x, y| {
            ffi_guard::contain((), || {
                // The release that follows is not a tap, whoever handles the hold.
                input.held.set(true);
                // A context menu's own long press keeps priority (docs/gestures.md).
                if g.widget().is_some_and(|w| crate::has_context_menu(&w)) {
                    return;
                }
                if let Some(node) = input.node(GestureKind::LongPress) {
                    emit(node, Event::LongPress(Point::new(x, y)));
                }
            });
        }
    });
    add_gesture(w, &long);
}

/// The observer behind press and the touch list's cancellation: a drag gesture that never
/// claims, so it delays and consumes nothing.
fn attach_watch(w: &Handle, input: &Rc<Input>) {
    if input.has_watch.replace(true) {
        return;
    }
    let watch = gtk4::GestureDrag::new();
    watch.connect_drag_begin({
        let input = input.clone();
        move |_, x, y| {
            ffi_guard::contain((), || {
                let at = Point::new(x, y);
                input.press_from.set(at);
                if let Some(node) = input.node(GestureKind::Press)
                    && input.press.get().is_none()
                {
                    input.press.set(Some(at));
                    emit(
                        node,
                        Event::Press {
                            pressed: true,
                            location: at,
                        },
                    );
                }
            });
        }
    });
    watch.connect_drag_update({
        let input = input.clone();
        move |g, dx, dy| {
            ffi_guard::contain((), || {
                if input.press.get().is_none() {
                    return;
                }
                let from = input.press_from.get();
                let at = Point::new(from.x + dx, from.y + dy);
                // Out of the hit area (bounds plus slop) ends the press; coming back does not
                // press again.
                if g.widget().is_some_and(|w| w.contains(at.x, at.y)) {
                    input.press.set(Some(at));
                } else {
                    input.press_out(Some(at));
                }
            });
        }
    });
    watch.connect_drag_end({
        let input = input.clone();
        move |_, dx, dy| {
            let from = input.press_from.get();
            ffi_guard::contain((), || {
                input.press_out(Some(Point::new(from.x + dx, from.y + dy)));
            });
        }
    });
    watch.connect_cancel({
        let input = input.clone();
        move |_, _| {
            ffi_guard::contain((), || {
                input.press_out(None);
                input.touches_cancelled();
            });
        }
    });
    add_gesture(w, &watch);
    // Hidden or removed while down: the press ends.
    w.connect_unmap({
        let input = input.clone();
        move |_| {
            ffi_guard::contain((), || {
                input.press_out(None);
                input.touches_cancelled();
            });
        }
    });
}

fn attach_drag(w: &Handle, input: &Rc<Input>) {
    let drag = gtk4::GestureDrag::new();
    let send = |input: &Input, phase: DragPhase, offset: Point| {
        if let Some(node) = input.node(GestureKind::Drag) {
            let from = input.drag_from.get();
            emit(
                node,
                Event::Drag {
                    phase,
                    location: Point::new(from.x + offset.x, from.y + offset.y),
                    translation: offset,
                },
            );
        }
    };
    drag.connect_drag_begin({
        let input = input.clone();
        move |_, x, y| {
            ffi_guard::contain((), || {
                input.drag_from.set(Point::new(x, y));
                input.fling.set(None);
                input.drag_cancelled.set(false);
                // With no axis the drag is the node's from the press (day-core reads the
                // press point from `Began`); with one, nothing is said until it is judged.
                if input.axis.get().is_none() {
                    input.drag.set(Track::Mine { claimed: false });
                    send(&input, DragPhase::Began, Point::ZERO);
                } else {
                    input.drag.set(Track::Pending);
                }
            });
        }
    });
    drag.connect_drag_update({
        let input = input.clone();
        move |g, dx, dy| {
            ffi_guard::contain((), || {
                let offset = Point::new(dx, dy);
                // Only a touch sequence is contested: GtkScrolledWindow's kinetic drag is
                // touch-only, and a mouse sequence (GDK gives it no sequence) belongs to the
                // widget that took the press. Claiming also ends GtkGestureSwipe at once, so a
                // claimed drag leaves its release velocity to day-core's estimate.
                let decide = |state: gtk4::EventSequenceState| {
                    if g.current_sequence().is_some() {
                        g.set_state(state);
                        input.fling.set(None);
                    }
                };
                match input.drag.get() {
                    Track::Mine { claimed } => {
                        if !claimed && judge(None, offset) == Judgment::Claim {
                            decide(gtk4::EventSequenceState::Claimed);
                            input.drag.set(Track::Mine { claimed: true });
                        }
                        send(&input, DragPhase::Changed, offset);
                    }
                    Track::Pending => match judge(input.axis.get(), offset) {
                        Judgment::Wait => {}
                        Judgment::Claim => {
                            decide(gtk4::EventSequenceState::Claimed);
                            input.drag.set(Track::Mine { claimed: true });
                            send(&input, DragPhase::Began, Point::ZERO);
                            send(&input, DragPhase::Changed, offset);
                        }
                        Judgment::Deny => {
                            // Not the node's: the scroll view around it may take the sequence.
                            input.drag.set(Track::NotMine);
                            decide(gtk4::EventSequenceState::Denied);
                        }
                    },
                    Track::Idle | Track::NotMine => {}
                }
            });
        }
    });
    let swipe = gtk4::GestureSwipe::new();
    drag.connect_drag_end({
        let input = input.clone();
        let swipe = swipe.downgrade();
        move |_, dx, dy| {
            ffi_guard::contain((), || {
                if !matches!(input.drag.replace(Track::Idle), Track::Mine { .. }) {
                    return;
                }
                // The velocity GTK measured at the release (the swipe gesture in the group
                // usually reports it just before), never for a cancelled sequence.
                let fling = input.fling.take().or_else(|| {
                    swipe
                        .upgrade()
                        .and_then(|s| s.velocity())
                        .map(|(vx, vy)| Point::new(vx, vy))
                });
                if let (false, Some(velocity), Some(node)) = (
                    input.drag_cancelled.get(),
                    fling,
                    input.node(GestureKind::Drag),
                ) {
                    emit(
                        node,
                        Event::GestureVelocity {
                            kind: GestureKind::Drag,
                            velocity,
                        },
                    );
                }
                send(&input, DragPhase::Ended, Point::new(dx, dy));
            });
        }
    });
    drag.connect_cancel({
        let input = input.clone();
        move |_, _| input.drag_cancelled.set(true)
    });
    add_gesture(w, &drag);
    swipe.connect_swipe({
        let input = input.clone();
        move |_, vx, vy| input.fling.set(Some(Point::new(vx, vy)))
    });
    add_gesture(w, &swipe);
}

/// A legacy event's position in `w`'s own coordinates (GDK reports it against the surface).
fn local_point(w: &gtk4::Widget, event: &gdk::Event) -> Option<Point> {
    let (x, y) = event.position()?;
    let native = w.native()?;
    let (sx, sy) = native.surface_transform();
    let root = native.upcast_ref::<gtk4::Widget>();
    let p = root.compute_point(
        w,
        &gtk4::graphene::Point::new((x - sx) as f32, (y - sy) as f32),
    )?;
    Some(Point::new(f64::from(p.x()), f64::from(p.y())))
}

fn event_kind(event: &gdk::Event) -> Option<PointerKind> {
    use gdk::EventType as T;
    let touch = match event.event_type() {
        T::TouchBegin | T::TouchUpdate => true,
        T::ButtonPress | T::MotionNotify | T::ProximityIn => false,
        _ => return None,
    };
    // A pointer event GDK made up from a touch is the touch's, already counted.
    if !touch && event.is_pointer_emulated() {
        return None;
    }
    Some(kind_for(
        touch,
        event.device().map(|d| d.source()),
        event.device_tool().map(|t| t.tool_type()),
    ))
}

fn attach_touches(w: &Handle, input: &Rc<Input>) {
    use gdk::EventType as T;
    let legacy = gtk4::EventControllerLegacy::new();
    // Capture: the contacts that went down anywhere on the node, whoever claims them inside.
    legacy.set_propagation_phase(gtk4::PropagationPhase::Capture);
    legacy.connect_event({
        let input = input.clone();
        move |c, event| {
            ffi_guard::contain(glib::Propagation::Proceed, || {
                let (Some(w), Some(node)) = (c.widget(), input.node(GestureKind::Touches)) else {
                    return glib::Propagation::Proceed;
                };
                let mouse_button = || {
                    event
                        .downcast_ref::<gdk::ButtonEvent>()
                        .is_some_and(|b| b.button() == gdk::BUTTON_PRIMARY)
                        && !event.is_pointer_emulated()
                };
                let (source, phase) = match event.event_type() {
                    T::TouchBegin | T::TouchUpdate | T::TouchEnd | T::TouchCancel => {
                        let sequence = event.event_sequence().as_ptr() as usize;
                        let phase = match event.event_type() {
                            T::TouchBegin => TouchPhase::Began,
                            T::TouchUpdate => TouchPhase::Moved,
                            T::TouchEnd => TouchPhase::Ended,
                            _ => TouchPhase::Cancelled,
                        };
                        (ContactSource::Sequence(sequence), phase)
                    }
                    T::ButtonPress if mouse_button() => (ContactSource::Mouse, TouchPhase::Began),
                    T::ButtonRelease if mouse_button() => (ContactSource::Mouse, TouchPhase::Ended),
                    T::MotionNotify if !event.is_pointer_emulated() => {
                        (ContactSource::Mouse, TouchPhase::Moved)
                    }
                    _ => return glib::Propagation::Proceed,
                };
                let Some(at) = local_point(&w, event) else {
                    return glib::Propagation::Proceed;
                };
                let kind = event_kind(event).unwrap_or(match source {
                    ContactSource::Mouse => PointerKind::Mouse,
                    ContactSource::Sequence(_) => PointerKind::Touch,
                });
                let pressure = event.axis(gdk::AxisUse::Pressure).unwrap_or(1.0);
                let touch = Touch::new(0, phase, at)
                    .with_pressure(pressure)
                    .with_kind(kind);
                let list = input.contacts.borrow_mut().apply(source, touch);
                if let Some(list) = list {
                    emit(node, Event::Touches(list));
                }
                glib::Propagation::Proceed
            })
        }
    });
    w.add_controller(legacy);
}

/// Report the input kind behind `window`'s pointer events (docs/gestures.md "Input kind"): a
/// capture-phase legacy controller that sends `Event::PointerKindChanged` on a change.
pub(crate) fn watch_pointer_kind(window: &impl IsA<gtk4::Widget>) {
    let legacy = gtk4::EventControllerLegacy::new();
    legacy.set_propagation_phase(gtk4::PropagationPhase::Capture);
    legacy.connect_event(|_, event| {
        ffi_guard::contain(glib::Propagation::Proceed, || {
            if let Some(kind) = event_kind(event)
                && POINTER.with(|p| p.replace(kind)) != kind
            {
                emit(day_spec::WINDOW_NODE, Event::PointerKindChanged(kind));
            }
            glib::Propagation::Proceed
        })
    });
    window.add_controller(legacy);
}

/// Focus `w` on a primary press, before the press reaches its recognizers (a capture-phase
/// click that never claims), unless the press landed on a focusable widget inside it, which
/// takes focus itself.
pub(crate) fn focus_on_press(w: &Handle) {
    let click = gtk4::GestureClick::new();
    click.set_propagation_phase(gtk4::PropagationPhase::Capture);
    click.connect_pressed(|g, _, x, y| {
        ffi_guard::contain((), || {
            let Some(w) = g.widget() else { return };
            let mut hit = w.pick(x, y, gtk4::PickFlags::DEFAULT);
            while let Some(inner) = hit {
                if inner == w {
                    break;
                }
                if inner.is_focusable() {
                    return;
                }
                hit = inner.parent();
            }
            w.grab_focus();
        });
    });
    add_gesture(w, &click);
}

/// The keyboard of a focusable view: a canvas, or a container made `.focusable()`
/// (docs/menus.md). While the view itself holds focus, a press the node handles is its event
/// with its phase; the full route (`.keyboard()`) composes through an input method context.
pub(crate) fn wire_keys(w: &Handle, id: NodeId) {
    let held = HeldKeys::default();
    let im = gtk4::IMMulticontext::new();
    im.set_client_widget(Some(w));
    // The key a commit comes from, while `filter_keypress` runs: its code, and whether it is a
    // repeat. A commit outside a press (an input method's own timing) names no key.
    let typing: Rc<Cell<Option<(u32, bool)>>> = Rc::default();
    im.connect_commit({
        let (held, typing) = (held.clone(), typing.clone());
        move |_, text| {
            ffi_guard::contain((), || {
                if text.is_empty() || !day_spec::keys::text(id) {
                    return;
                }
                let event = KeyEvent::typed_text(text, 0);
                match typing.get() {
                    Some((code, repeat)) if event.key != Key::Unidentified => {
                        held.deliver(id, code, event, repeat);
                    }
                    _ => emit(id, Event::Key(event)),
                }
            });
        }
    });
    w.connect_has_focus_notify({
        let (held, im) = (held.clone(), im.clone());
        move |w| {
            ffi_guard::contain((), || {
                if w.has_focus() {
                    // Only a view that took the whole keyboard starts compositions.
                    if day_spec::keys::text(id) {
                        im.focus_in();
                    }
                } else {
                    // A composition in progress, and every held key, go with the focus.
                    im.reset();
                    im.focus_out();
                    held.clear();
                }
            });
        }
    });
    let keys = gtk4::EventControllerKey::new();
    keys.connect_key_pressed({
        let held = held.clone();
        move |controller, key, code, state| {
            ffi_guard::contain(glib::Propagation::Proceed, || {
                // A key from a focused child bubbles through here: it is not this view's.
                if !controller.widget().is_some_and(|w| w.has_focus()) {
                    return glib::Propagation::Proceed;
                }
                // An arrow nobody asked for keeps traveling, so a scrolled window around the
                // view still scrolls with the keyboard.
                if !day_spec::keys::handled(id) {
                    return glib::Propagation::Proceed;
                }
                let repeat = held.is_held(code);
                if day_spec::keys::text(id) {
                    if let Some(event) = controller.current_event() {
                        typing.set(Some((code, repeat)));
                        let taken = im.filter_keypress(&event);
                        typing.set(None);
                        if taken {
                            return glib::Propagation::Stop;
                        }
                    }
                    let Some(event) = full_route_key(key, state) else {
                        return glib::Propagation::Proceed;
                    };
                    held.deliver(id, code, event, repeat);
                    return glib::Propagation::Stop;
                }
                let Some(name) = key_name(key, state) else {
                    return glib::Propagation::Proceed;
                };
                held.deliver(
                    id,
                    code,
                    KeyEvent::named(name, key_modifiers(state)),
                    repeat,
                );
                glib::Propagation::Stop
            })
        }
    });
    keys.connect_key_released(move |_, _, code, state| {
        ffi_guard::contain((), || held.deliver_release(id, code, state));
    });
    w.add_controller(keys);
}

/// What decides whether a [`DayFixed`] or [`DayCanvas`] takes pointer input, and where.
#[derive(Default)]
pub(crate) struct HitArea {
    /// A recognizer, a menu, a drop target, a cursor or focus asked for input.
    input: Cell<bool>,
    /// A canvas that hears keys takes the press that focuses it.
    keys: Cell<Option<NodeId>>,
    slop: Cell<Insets>,
}

impl HitArea {
    fn contains(&self, w: &gtk4::Widget, x: f64, y: f64) -> bool {
        let takes = self.input.get() || self.keys.get().is_some_and(day_spec::keys::handled);
        takes
            && slop_contains(
                f64::from(w.width()),
                f64::from(w.height()),
                self.slop.get(),
                w.direction() == gtk4::TextDirection::Rtl,
                x,
                y,
            )
    }
}

mod hit {
    use super::HitArea;
    use gtk4::glib;
    use gtk4::prelude::*;
    use gtk4::subclass::prelude::*;

    #[derive(Default)]
    pub struct DayFixedImp {
        pub(crate) area: HitArea,
        /// The surface border this container draws after its children
        /// ([`super::set_border_over`]), with the corner radii it follows.
        pub(crate) border: std::cell::Cell<Option<(day_spec::Border, day_spec::CornerRadii)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DayFixedImp {
        const NAME: &'static str = "DayFixed";
        type Type = super::DayFixed;
        type ParentType = gtk4::Fixed;
    }

    impl ObjectImpl for DayFixedImp {}

    impl WidgetImpl for DayFixedImp {
        fn contains(&self, x: f64, y: f64) -> bool {
            self.area.contains(self.obj().upcast_ref(), x, y)
        }

        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            // The children first, then the border over them (docs/surfaces.md): GTK's own
            // CSS border and box shadows are drawn before the children.
            self.parent_snapshot(snapshot);
            if let Some((border, radii)) = self.border.get() {
                super::snapshot_border(self.obj().upcast_ref(), snapshot, border, radii);
            }
        }
    }

    impl FixedImpl for DayFixedImp {}

    #[derive(Default)]
    pub struct DayCanvasImp {
        pub(crate) area: HitArea,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DayCanvasImp {
        const NAME: &'static str = "DayCanvas";
        type Type = super::DayCanvas;
        type ParentType = gtk4::DrawingArea;
    }

    impl ObjectImpl for DayCanvasImp {}

    impl WidgetImpl for DayCanvasImp {
        fn contains(&self, x: f64, y: f64) -> bool {
            self.area.contains(self.obj().upcast_ref(), x, y)
        }
    }

    impl DrawingAreaImpl for DayCanvasImp {}
}

glib::wrapper! {
    /// A Day container: a `GtkFixed` that takes pointer input only where asked (pass-through,
    /// docs/gestures.md), within its hit slop.
    pub struct DayFixed(ObjectSubclass<hit::DayFixedImp>)
        @extends gtk4::Fixed, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

glib::wrapper! {
    /// A Day canvas (and every shape): a `GtkDrawingArea` that takes pointer input only where
    /// asked, within its hit slop.
    pub struct DayCanvas(ObjectSubclass<hit::DayCanvasImp>)
        @extends gtk4::DrawingArea, gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl DayFixed {
    /// A passive container; `take_input` makes it hit-testable.
    pub(crate) fn new() -> Self {
        glib::Object::new()
    }
}

impl DayCanvas {
    /// A passive canvas for `id`, hit-testable while `id` hears keys or once `take_input`.
    pub(crate) fn new(id: NodeId) -> Self {
        let canvas: Self = glib::Object::new();
        canvas.imp().area.keys.set(Some(id));
        canvas
    }
}

/// Have `w` draw `border` over its children, inside its frame and following `radii`, or no
/// border for `None`. Only a Day container ([`DayFixed`]) can: `false` for any other widget,
/// whose surface keeps the border in its CSS, under its content.
pub(crate) fn set_border_over(
    w: &gtk4::Widget,
    border: Option<(day_spec::Border, day_spec::CornerRadii)>,
) -> bool {
    let Some(f) = w.downcast_ref::<DayFixed>() else {
        return false;
    };
    let was = f.imp().border.replace(border);
    if was.is_some() || border.is_some() {
        f.queue_draw();
    }
    true
}

/// Append `border` along `w`'s bounds rounded by `radii`, its ink resolved for the appearance
/// drawn now: a border band GTK renders the way it renders a CSS border.
fn snapshot_border(
    w: &gtk4::Widget,
    snapshot: &gtk4::Snapshot,
    border: day_spec::Border,
    radii: day_spec::CornerRadii,
) {
    let (width, height) = (w.width() as f32, w.height() as f32);
    if width <= 0.0 || height <= 0.0 || border.width <= 0.0 {
        return;
    }
    let corner = |r: f64| gtk4::graphene::Size::new(r.max(0.0) as f32, r.max(0.0) as f32);
    let mut outline = gtk4::gsk::RoundedRect::new(
        gtk4::graphene::Rect::new(0.0, 0.0, width, height),
        corner(radii.top_left),
        corner(radii.top_right),
        corner(radii.bottom_right),
        corner(radii.bottom_left),
    );
    // Radii larger than the box (a capsule's) scale down together, as CSS's do.
    outline.normalize();
    let band = (border.width as f32).min(width.min(height) / 2.0);
    let c = border.color.resolved();
    let rgba = gtk4::gdk::RGBA::new(c.r as f32, c.g as f32, c.b as f32, c.a as f32);
    snapshot.append_border(&outline, &[band; 4], &[rgba; 4]);
}

fn hit_area(w: &gtk4::Widget) -> Option<&HitArea> {
    if let Some(f) = w.downcast_ref::<DayFixed>() {
        return Some(&f.imp().area);
    }
    w.downcast_ref::<DayCanvas>().map(|c| &c.imp().area)
}

/// `w` takes pointer input from now on (pass-through ends): it has a recognizer, a menu, a
/// drag source or drop target, a cursor, or focus.
pub(crate) fn take_input(w: &gtk4::Widget) {
    match hit_area(w) {
        Some(area) => area.input.set(true),
        None => w.set_can_target(true),
    }
}

/// Grow `w`'s hit area by `slop` (`Toolkit::set_hit_slop`). Only Day's containers and canvases
/// can answer GTK's hit test with more than their bounds: a label, an image or a native control
/// keeps its own.
pub(crate) fn set_slop(w: &gtk4::Widget, slop: Insets) {
    if let Some(area) = hit_area(w) {
        area.slop.set(slop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_run_pairs_into_double_taps_and_holds_the_single_only_when_both_are_heard() {
        // Tap alone: every click taps at once, as before.
        assert_eq!(click_outcome(1, true, false), Click::Tap);
        assert_eq!(click_outcome(2, true, false), Click::Tap);
        // Double tap alone: nothing waits, nothing taps.
        assert_eq!(click_outcome(1, false, true), Click::Nothing);
        assert_eq!(click_outcome(2, false, true), Click::DoubleTap);
        // Both: the single waits; the pair is the double tap; a triple ends on a held single.
        assert_eq!(click_outcome(1, true, true), Click::HeldTap);
        assert_eq!(click_outcome(2, true, true), Click::DoubleTap);
        assert_eq!(click_outcome(3, true, true), Click::HeldTap);
        assert_eq!(click_outcome(4, true, true), Click::DoubleTap);
        assert_eq!(click_outcome(1, false, false), Click::Nothing);
    }

    #[test]
    fn a_drag_is_judged_after_three_points_along_or_across_its_axis() {
        let p = Point::new;
        assert_eq!(judge(None, p(1.0, 1.0)), Judgment::Wait);
        assert_eq!(judge(None, p(0.0, 5.0)), Judgment::Claim);
        let h = Some(DragAxis::Horizontal);
        assert_eq!(judge(h, p(2.0, 0.0)), Judgment::Wait);
        assert_eq!(judge(h, p(4.0, 1.0)), Judgment::Claim);
        assert_eq!(judge(h, p(-4.0, 4.0)), Judgment::Claim);
        assert_eq!(judge(h, p(1.0, 4.0)), Judgment::Deny);
        assert_eq!(judge(Some(DragAxis::Vertical), p(5.0, 1.0)), Judgment::Deny);
    }

    #[test]
    fn hit_slop_grows_each_edge_and_mirrors_leading_and_trailing() {
        let slop = Insets {
            top: 1.0,
            leading: 10.0,
            bottom: 2.0,
            trailing: 20.0,
        };
        let inside = |rtl, x, y| slop_contains(12.0, 12.0, slop, rtl, x, y);
        assert!(inside(false, -10.0, 6.0));
        assert!(!inside(false, -10.5, 6.0));
        assert!(inside(false, 32.0, 6.0));
        assert!(!inside(false, 32.5, 6.0));
        assert!(inside(false, 6.0, -1.0) && !inside(false, 6.0, -1.5));
        assert!(inside(false, 6.0, 14.0) && !inside(false, 6.0, 14.5));
        // Right to left: leading is the right edge.
        assert!(inside(true, -20.0, 6.0) && !inside(true, -20.5, 6.0));
        assert!(inside(true, 22.0, 6.0) && !inside(true, 22.5, 6.0));
        // No slop: the bounds.
        assert!(slop_contains(12.0, 12.0, Insets::ZERO, false, 12.0, 0.0));
        assert!(!slop_contains(12.0, 12.0, Insets::ZERO, false, -0.5, 0.0));
    }

    #[test]
    fn the_input_kind_reads_touch_tool_then_device() {
        use gdk::{DeviceToolType as T, InputSource as S};
        assert_eq!(kind_for(true, Some(S::Mouse), None), PointerKind::Touch);
        assert_eq!(
            kind_for(false, Some(S::Mouse), Some(T::Pen)),
            PointerKind::Stylus
        );
        assert_eq!(
            kind_for(false, Some(S::Mouse), Some(T::Mouse)),
            PointerKind::Mouse
        );
        assert_eq!(kind_for(false, Some(S::Pen), None), PointerKind::Stylus);
        assert_eq!(
            kind_for(false, Some(S::Touchscreen), None),
            PointerKind::Touch
        );
        assert_eq!(kind_for(false, Some(S::Touchpad), None), PointerKind::Mouse);
        assert_eq!(kind_for(false, None, None), PointerKind::Mouse);
    }

    #[test]
    fn contacts_keep_their_ids_and_report_an_ending_once() {
        let mut c = Contacts::default();
        let t = |phase, x| Touch::new(99, phase, Point::new(x, 0.0));
        let a = ContactSource::Sequence(0xa0);
        let b = ContactSource::Sequence(0xb0);
        let ids = |l: &[Touch]| l.iter().map(|t| (t.id, t.phase)).collect::<Vec<_>>();
        let one = c.apply(a, t(TouchPhase::Began, 1.0)).unwrap_or_default();
        assert_eq!(ids(&one), vec![(0, TouchPhase::Began)]);
        let two = c.apply(b, t(TouchPhase::Began, 2.0)).unwrap_or_default();
        assert_eq!(
            ids(&two),
            vec![(0, TouchPhase::Stationary), (1, TouchPhase::Began)]
        );
        let lift = c.apply(a, t(TouchPhase::Ended, 1.0)).unwrap_or_default();
        assert_eq!(
            ids(&lift),
            vec![(0, TouchPhase::Ended), (1, TouchPhase::Stationary)]
        );
        // The lifted finger is gone; a new one reuses its id.
        let moved = c.apply(b, t(TouchPhase::Moved, 3.0)).unwrap_or_default();
        assert_eq!(ids(&moved), vec![(1, TouchPhase::Moved)]);
        let again = c
            .apply(ContactSource::Mouse, t(TouchPhase::Began, 4.0))
            .unwrap_or_default();
        assert_eq!(
            ids(&again),
            vec![(0, TouchPhase::Began), (1, TouchPhase::Stationary)]
        );
        // A move for a contact that began elsewhere says nothing.
        assert!(
            c.apply(ContactSource::Sequence(0xc0), t(TouchPhase::Moved, 0.0))
                .is_none()
        );
        let gone = c.cancel_all().unwrap_or_default();
        assert_eq!(
            ids(&gone),
            vec![(0, TouchPhase::Cancelled), (1, TouchPhase::Cancelled)]
        );
        assert!(c.cancel_all().is_none());
    }

    #[test]
    fn held_keys_tell_a_repeat_and_answer_the_release_once() {
        let held = HeldKeys::default();
        assert!(!held.is_held(38));
        held.hold(38, Key::ArrowRight);
        assert!(held.is_held(38));
        assert_eq!(held.release(38), Some(Key::ArrowRight));
        assert_eq!(held.release(38), None);
        held.hold(38, Key::ArrowRight);
        held.clear();
        assert_eq!(held.release(38), None);
    }
}
