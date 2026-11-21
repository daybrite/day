// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Scrolling on GTK 4's own scrolled window (docs/scroll.md, docs/list.md).
//!
//! - **Glides**: every programmatic scroll ([`glide`]) steps the `GtkAdjustment`s from a tick
//!   callback on the window's frame clock. GTK has no animated "scroll the adjustment to"
//!   call (its own kinetic scrolling is a tick callback too), so an animated scroll is an
//!   ease-out of [`GLIDE_US`]; an instant one sets the value on the next frame. The target is
//!   asked again on every frame ([`Aim`]), because a list's rows and a scroll's content are
//!   laid out by GTK a frame after Day changed them, and an automatic row's height is known
//!   only once GTK measured it. A newer glide on the same scrolled window supersedes the
//!   running one, and a wheel, press or touch on it cancels it.
//! - **Offset**: `value-changed` on both adjustments, coalesced to one
//!   [`Event::ScrollChanged`] per frame ([`track_offset`]).
//! - **Phases**: [`track_phase`] reads the user's own input on the scrolled window. A touchpad
//!   gesture is GTK's `scroll-begin` / `scroll-end` / `decelerate`; a wheel has no begin or
//!   end, so a burst of wheel steps is one drag that ends after [`WHEEL_QUIET_MS`]; a touch
//!   drag and a scroll-bar drag are their press and release around the adjustment's movement.
//!   Movement after a release is coasting until the adjustment holds still.
//! - **The reader's scrolls**: [`track_reader`] tells a list that the reader moved it, from
//!   the same input: a wheel or touchpad step that can move the list, or a movement of the
//!   adjustment while a finger or the pointer holds the content or a scroll bar, or just after
//!   a key that scrolls. A glide sets the adjustment with none of these active.
//! - **Row deltas**: [`plan_splice`] turns a list's sequential row deltas into
//!   `g_list_model_items_changed` runs, and refuses deltas that do not fit the model.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use day_spec::props::RowDelta;
use day_spec::sidetable::SideTable;
use day_spec::{Event, NodeId, Point, RowAlign, ScrollPhase};
use gtk4::glib;
use gtk4::prelude::*;

use crate::emit;

/// How long an animated scroll takes, in microseconds of frame time: the ease-out the brief
/// asks of a platform with no scroll animation of its own.
pub(crate) const GLIDE_US: f64 = 250_000.0;

/// How many frames a glide keeps trying before it gives up: about four seconds, for rows that
/// never get laid out (a list that is not shown).
const GLIDE_FRAMES: u32 = 240;

/// How long a wheel must rest before a burst of wheel steps counts as one finished drag.
pub(crate) const WHEEL_QUIET_MS: u64 = 150;

/// How long the adjustment must hold still after the user let go before scrolling is idle.
const COAST_QUIET_MS: u64 = 100;

/// One `g_list_model_items_changed` run on a list's model: at `at`, `removed` rows go and
/// `added` rows arrive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ModelOp {
    pub at: u32,
    pub removed: u32,
    pub added: u32,
}

/// The model runs that apply `deltas` (sequential, docs/list.md "Keyed updates") to a model of
/// `current` rows, or `None` when they do not fit it: a delta names a row past the end of the
/// list as the earlier deltas left it, or the deltas end at a row count other than `expected`
/// (the source's). The caller reloads then; a splice against rows the model does not have is
/// how UIKit's table crashed.
///
/// A move is a removal and an insertion. Neighboring inserts at one index (an append arrives
/// as one `Insert(n)` per row, each above the last) and neighboring removals (highest index
/// first) merge into one run each.
pub(crate) fn plan_splice(
    current: usize,
    deltas: &[RowDelta],
    expected: usize,
) -> Option<Vec<ModelOp>> {
    fn push(op: ModelOp, ops: &mut Vec<ModelOp>) {
        if let Some(last) = ops.last_mut() {
            // Inserts at the same index, or just below the run: one longer run.
            if last.removed == 0
                && op.removed == 0
                && (op.at == last.at || op.at == last.at + last.added)
            {
                last.added += op.added;
                return;
            }
            // Removals at the same index (forward) or just above it (backward).
            if last.added == 0 && op.added == 0 {
                if op.at == last.at {
                    last.removed += op.removed;
                    return;
                }
                if op.at + op.removed == last.at {
                    last.at = op.at;
                    last.removed += op.removed;
                    return;
                }
            }
        }
        ops.push(op);
    }
    let mut count = current;
    let mut ops: Vec<ModelOp> = Vec::new();
    for delta in deltas {
        match *delta {
            RowDelta::Insert(at) => {
                if at > count {
                    return None;
                }
                push(
                    ModelOp {
                        at: u32::try_from(at).ok()?,
                        removed: 0,
                        added: 1,
                    },
                    &mut ops,
                );
                count += 1;
            }
            RowDelta::Remove(at) => {
                if at >= count {
                    return None;
                }
                push(
                    ModelOp {
                        at: u32::try_from(at).ok()?,
                        removed: 1,
                        added: 0,
                    },
                    &mut ops,
                );
                count -= 1;
            }
            RowDelta::Move(from, to) => {
                // `to` is an index in the list after the removal.
                if from >= count || to >= count {
                    return None;
                }
                if from == to {
                    continue;
                }
                push(
                    ModelOp {
                        at: u32::try_from(from).ok()?,
                        removed: 1,
                        added: 0,
                    },
                    &mut ops,
                );
                push(
                    ModelOp {
                        at: u32::try_from(to).ok()?,
                        removed: 0,
                        added: 1,
                    },
                    &mut ops,
                );
            }
        }
    }
    (count == expected).then_some(ops)
}

/// The first and last rows with any part inside a viewport `page` tall, from the laid rows'
/// `(row, top, height)` in the viewport's coordinates (rows GTK keeps above or below it, its
/// overscan, are excluded). `None` when no laid row is in view.
pub(crate) fn visible_range(rows: &[(usize, f64, f64)], page: f64) -> Option<(usize, usize)> {
    // Half a pixel either way: a row that ends exactly at the top edge is not in view.
    const EPS: f64 = 0.5;
    let mut range: Option<(usize, usize)> = None;
    for &(row, top, height) in rows {
        if height <= 0.0 || top >= page - EPS || top + height <= EPS {
            continue;
        }
        range = Some(match range {
            Some((first, last)) => (first.min(row), last.max(row)),
            None => (row, row),
        });
    }
    range
}

/// The first and last rows in view of a list whose rows are all `pitch` tall, scrolled to
/// `value` in a viewport `page` tall, out of `n` rows.
pub(crate) fn pitch_range(value: f64, page: f64, pitch: f64, n: usize) -> Option<(usize, usize)> {
    if n == 0 || pitch <= 0.0 || page <= 0.0 {
        return None;
    }
    let first = (value.max(0.0) / pitch).floor() as usize;
    let last = (((value.max(0.0) + page) / pitch).ceil() as usize).saturating_sub(1);
    Some((first.min(n - 1), last.max(first).min(n - 1)))
}

/// Where a list scrolls so a row `top` and `height` tall (content coordinates) lands as
/// `align` says (docs/list.md "Programmatic scrolling"), from `value` in a viewport `page`
/// tall whose offset runs from 0 to `end`. `inset` is how far below the viewport's top the row
/// must stop (the pinned header copy of a sectioned list).
pub(crate) fn aligned_offset(
    value: f64,
    page: f64,
    end: f64,
    top: f64,
    height: f64,
    align: RowAlign,
    inset: f64,
) -> f64 {
    let at = match align {
        RowAlign::Start => top - inset,
        RowAlign::Center => top + height / 2.0 - page / 2.0,
        RowAlign::End => top + height - page,
        RowAlign::Nearest => {
            if top - inset < value {
                top - inset
            } else if top + height > value + page {
                // A row taller than the viewport shows its top.
                (top + height - page).min(top - inset)
            } else {
                value
            }
        }
    };
    at.clamp(0.0, end.max(0.0))
}

/// The minimal offset along one axis that brings `lo..hi` (content coordinates) into a
/// viewport `page` long scrolled to `value` (docs/scroll.md "How a target becomes a scroll").
pub(crate) fn reveal(value: f64, page: f64, lo: f64, hi: f64) -> f64 {
    let mut v = value;
    if hi > v + page {
        v = hi - page;
    }
    if lo < v {
        v = lo;
    }
    v
}

/// The ease-out of an animated scroll: cubic, fast at first and settling at the end.
pub(crate) fn eased(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// What a glide aims at, asked again on every frame.
pub(crate) enum Aim {
    /// The rows or the content are not laid out yet: ask again next frame.
    Wait,
    /// There is nothing to scroll to.
    Stop,
    /// This frame's target offsets (`None` leaves an axis alone). `exact` is false while the
    /// target is an estimate (an automatic row GTK has not measured yet), which keeps the glide
    /// refining after its animation ran out.
    At {
        v: Option<f64>,
        h: Option<f64>,
        exact: bool,
    },
}

thread_local! {
    /// Scrolled window key → the serial of the glide that owns it. A glide whose serial is no
    /// longer here stops at its next frame.
    static GLIDES: SideTable<u64> = SideTable::new();
    /// Scrolled window key → the scrolled window, for [`settled`]. Kept past the glide's end;
    /// only a key that [`GLIDES`] still holds counts.
    static GLIDE_VIEWS: SideTable<glib::WeakRef<gtk4::ScrolledWindow>> = SideTable::new();
    /// Scrolled windows that carry the input watcher that cancels a glide.
    static GLIDE_WATCHED: SideTable<()> = SideTable::new();
    static GLIDE_SERIAL: Cell<u64> = const { Cell::new(0) };
}

/// Scroll `sw` to where `aim` says, animated or on the next frame (see the module doc), then
/// run `rested` once the view rests there (not when the glide was superseded, cancelled or
/// gave up). Any glide already running on `sw` stops.
pub(crate) fn glide(
    sw: &gtk4::ScrolledWindow,
    animated: bool,
    aim: impl Fn(&gtk4::ScrolledWindow) -> Aim + 'static,
    rested: impl Fn(&gtk4::ScrolledWindow) + 'static,
) {
    let key = sw.as_ptr() as usize;
    let serial = GLIDE_SERIAL.with(|s| {
        s.set(s.get() + 1);
        s.get()
    });
    GLIDES.with(|t| t.insert(key, serial));
    GLIDE_VIEWS.with(|t| t.insert(key, sw.downgrade()));
    watch_user_input(sw, key);
    // `(frame time, v, h)` at the first frame with a target: where the ease starts.
    let start: Cell<Option<(i64, f64, f64)>> = Cell::new(None);
    let frames = Cell::new(0_u32);
    sw.add_tick_callback(move |w, clock| {
        let Some(sw) = w.downcast_ref::<gtk4::ScrolledWindow>() else {
            return glib::ControlFlow::Break;
        };
        if GLIDES.with(|t| t.get(key)) != Some(serial) {
            return glib::ControlFlow::Break;
        }
        frames.set(frames.get() + 1);
        let finish = || {
            if GLIDES.with(|t| t.get(key)) == Some(serial) {
                GLIDES.with(|t| t.remove(key));
            }
            glib::ControlFlow::Break
        };
        if frames.get() > GLIDE_FRAMES {
            return finish();
        }
        let (v, h, exact) = match aim(sw) {
            Aim::Wait => return glib::ControlFlow::Continue,
            Aim::Stop => return finish(),
            Aim::At { v, h, exact } => (v, h, exact),
        };
        let (vadj, hadj) = (sw.vadjustment(), sw.hadjustment());
        let now = clock.frame_time();
        let (t0, v0, h0) = start.get().unwrap_or_else(|| {
            let s = (now, vadj.value(), hadj.value());
            start.set(Some(s));
            s
        });
        let t = if animated {
            ((now - t0) as f64 / GLIDE_US).clamp(0.0, 1.0)
        } else {
            1.0
        };
        // Done once the eased run is over and the value set on an earlier frame held through
        // the layout since: an instant glide sets on its first frame and checks on its second.
        if t >= 1.0 && exact && arrived(&vadj, v) && arrived(&hadj, h) {
            rested(sw);
            return finish();
        }
        let k = eased(t);
        if let Some(v) = v {
            vadj.set_value(v0 + (v - v0) * k);
        }
        if let Some(h) = h {
            hadj.set_value(h0 + (h - h0) * k);
        }
        glib::ControlFlow::Continue
    });
}

/// Whether a glide is running on `sw`.
pub(crate) fn gliding(sw: &gtk4::ScrolledWindow) -> bool {
    GLIDES.with(|t| t.contains(sw.as_ptr() as usize))
}

/// Whether every programmatic scroll has come to rest (`Toolkit::ui_idle`): no glide is still
/// running on a shown scrolled window. A list reports its viewport after the frame that lays
/// out a move (docs/list.md "Visible rows"), and a glide rests one frame after that, so a step
/// that waits for this reads the rows the scroll brought into view rather than the ones it left.
/// A scrolled window that is not mapped gets no frames, so its glide never ends and is not
/// counted.
pub(crate) fn settled() -> bool {
    let mut moving = false;
    GLIDE_VIEWS.with(|views| {
        views.for_each(|key, view| {
            moving = moving
                || (GLIDES.with(|t| t.contains(key))
                    && view.upgrade().is_some_and(|sw| sw.is_mapped()));
        });
    });
    !moving
}

/// Whether `adj` rests at `target` as far as its range allows.
fn arrived(adj: &gtk4::Adjustment, target: Option<f64>) -> bool {
    target.is_none_or(|x| {
        let end = (adj.upper() - adj.page_size()).max(adj.lower());
        (adj.value() - x.clamp(adj.lower(), end)).abs() < 0.5
    })
}

/// The user's own scrolling (a wheel step, a press on the content or a scroll bar, a touch)
/// cancels a glide on `sw`, the way a finger stops a fling. Observed in the capture phase and
/// never stopped there, so the input still does what it does.
fn watch_user_input(sw: &gtk4::ScrolledWindow, key: usize) {
    if GLIDE_WATCHED.with(|t| t.insert(key, ())).is_some() {
        return;
    }
    let legacy = gtk4::EventControllerLegacy::new();
    legacy.set_propagation_phase(gtk4::PropagationPhase::Capture);
    legacy.connect_event(move |_, event| {
        if matches!(
            event.event_type(),
            gtk4::gdk::EventType::Scroll
                | gtk4::gdk::EventType::ButtonPress
                | gtk4::gdk::EventType::TouchBegin
        ) {
            GLIDES.with(|t| t.remove(key));
        }
        glib::Propagation::Proceed
    });
    sw.add_controller(legacy);
}

/// The content offset of `sw` in points from the content's top-leading corner, as
/// `Toolkit::scroll_offset` and [`Event::ScrollChanged`] report it.
pub(crate) fn offset_of(sw: &gtk4::ScrolledWindow) -> Point {
    Point::new(sw.hadjustment().value(), sw.vadjustment().value())
}

/// Report `sw`'s offset to `node` as it moves ([`Event::ScrollChanged`],
/// `ScrollProps::track_offset`): at most once per frame, on the frame after a move, so the
/// last report is always the offset the view rests at.
pub(crate) fn track_offset(sw: &gtk4::ScrolledWindow, node: NodeId) {
    let queued = Rc::new(Cell::new(false));
    for adj in [sw.vadjustment(), sw.hadjustment()] {
        let queued = queued.clone();
        let sw = sw.downgrade();
        adj.connect_value_changed(move |_| {
            if queued.replace(true) {
                return;
            }
            let Some(sw) = sw.upgrade() else {
                return;
            };
            let queued = queued.clone();
            sw.add_tick_callback(move |w, _| {
                queued.set(false);
                if let Some(sw) = w.downcast_ref::<gtk4::ScrolledWindow>() {
                    emit(node, Event::ScrollChanged(offset_of(sw)));
                }
                glib::ControlFlow::Break
            });
        });
    }
}

/// The phase tracker of one scrolled window ([`track_phase`]).
struct Phases {
    node: NodeId,
    /// The phase last sent.
    current: Cell<ScrollPhase>,
    /// A touchpad gesture is between its `scroll-begin` and `scroll-end`.
    gesture: Cell<bool>,
    /// A finger is on the content, or the pointer holds a scroll bar.
    held: Cell<bool>,
    /// A `scroll-end` waits for the `decelerate` GTK sends right after it.
    ending: Cell<bool>,
    /// The pending "the wheel rested" or "the content stopped" check; the next wheel step or
    /// movement replaces it.
    quiet: RefCell<Option<glib::SourceId>>,
}

impl Phases {
    fn enter(&self, next: ScrollPhase) {
        if self.current.replace(next) != next {
            emit(self.node, Event::ScrollPhase(next));
        }
    }

    /// Run `then` after `ms` without another call to this (a newer check replaces it).
    fn after_quiet(self: &Rc<Self>, ms: u64, then: fn(&Self)) {
        if let Some(old) = self.quiet.borrow_mut().take() {
            old.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
            if let Some(me) = weak.upgrade() {
                me.quiet.borrow_mut().take();
                then(&me);
            }
        });
        *self.quiet.borrow_mut() = Some(id);
    }

    /// The user let go: coast while the content keeps moving, else stop.
    fn release(self: &Rc<Self>, coasting: bool) {
        if self.current.get() == ScrollPhase::Idle {
            return;
        }
        if coasting {
            self.enter(ScrollPhase::Coasting);
            self.after_quiet(COAST_QUIET_MS, |me| me.enter(ScrollPhase::Idle));
        } else {
            if let Some(old) = self.quiet.borrow_mut().take() {
                old.remove();
            }
            self.enter(ScrollPhase::Idle);
        }
    }
}

/// Report the phases of user-driven scrolling on `sw` to `node` ([`Event::ScrollPhase`],
/// `ScrollProps::track_phase`; the module doc lists the sources). A programmatic scroll moves
/// the adjustment with none of them active and sends nothing.
pub(crate) fn track_phase(sw: &gtk4::ScrolledWindow, node: NodeId) {
    let me = Rc::new(Phases {
        node,
        current: Cell::new(ScrollPhase::Idle),
        gesture: Cell::new(false),
        held: Cell::new(false),
        ending: Cell::new(false),
        quiet: RefCell::new(None),
    });
    // Wheel and touchpad: GTK's scroll controller, in the capture phase so a child that
    // handles the wheel itself does not hide it, and never claiming the event.
    let scroll = gtk4::EventControllerScroll::new(
        gtk4::EventControllerScrollFlags::BOTH_AXES | gtk4::EventControllerScrollFlags::KINETIC,
    );
    scroll.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let me = me.clone();
        scroll.connect_scroll_begin(move |_| {
            me.gesture.set(true);
            me.enter(ScrollPhase::Dragging);
        });
    }
    {
        let me = me.clone();
        scroll.connect_scroll(move |_, _, _| {
            if !me.gesture.get() {
                // A wheel step: no begin and no end, so the burst ends when the wheel rests.
                me.enter(ScrollPhase::Dragging);
                me.after_quiet(WHEEL_QUIET_MS, |me| me.enter(ScrollPhase::Idle));
            }
            glib::Propagation::Proceed
        });
    }
    {
        let me = me.clone();
        scroll.connect_scroll_end(move |_| {
            me.gesture.set(false);
            // GTK sends `decelerate` right after this when the gesture ended moving; without
            // it the content stopped with the fingers.
            me.ending.set(true);
            let me = me.clone();
            glib::idle_add_local_once(move || {
                if me.ending.replace(false) {
                    me.release(false);
                }
            });
        });
    }
    {
        let me = me.clone();
        scroll.connect_decelerate(move |_, vx, vy| {
            me.ending.set(false);
            me.release(vx.hypot(vy) > 1.0);
        });
    }
    sw.add_controller(scroll);
    // A finger on the content: GtkScrolledWindow's own kinetic drag moves it, with no signal
    // of its own, so the touch sequence and the adjustment tell the phases.
    let touch = gtk4::EventControllerLegacy::new();
    touch.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let me = me.clone();
        touch.connect_event(move |_, event| {
            match event.event_type() {
                gtk4::gdk::EventType::TouchBegin => me.held.set(true),
                gtk4::gdk::EventType::TouchEnd | gtk4::gdk::EventType::TouchCancel => {
                    me.held.set(false);
                    lift(&me);
                }
                _ => {}
            }
            glib::Propagation::Proceed
        });
    }
    sw.add_controller(touch);
    // A scroll bar held by the pointer. GtkScrolledWindow makes its bars at construction.
    for bar in [sw.vscrollbar(), sw.hscrollbar()] {
        let grab = gtk4::EventControllerLegacy::new();
        grab.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let me = me.clone();
        grab.connect_event(move |_, event| {
            match event.event_type() {
                gtk4::gdk::EventType::ButtonPress => me.held.set(true),
                gtk4::gdk::EventType::ButtonRelease => {
                    me.held.set(false);
                    // A scroll bar has no momentum.
                    me.release(false);
                }
                _ => {}
            }
            glib::Propagation::Proceed
        });
        bar.add_controller(grab);
    }
    // Movement: dragging while held, coasting goes on while the content moves.
    for adj in [sw.vadjustment(), sw.hadjustment()] {
        let me = me.clone();
        adj.connect_value_changed(move |_| {
            if me.held.get() {
                me.enter(ScrollPhase::Dragging);
            } else if me.current.get() == ScrollPhase::Coasting {
                me.after_quiet(COAST_QUIET_MS, |me| me.enter(ScrollPhase::Idle));
            }
        });
    }
}

/// A finger lifted after dragging the content: `GtkScrolledWindow` starts its kinetic
/// deceleration in the same turn when the finger was moving, so a movement within the next
/// [`COAST_QUIET_MS`] is coasting; none, and the drag ended where it stopped.
fn lift(me: &Rc<Phases>) {
    if me.current.get() != ScrollPhase::Dragging {
        return;
    }
    me.enter(ScrollPhase::Coasting);
    me.after_quiet(COAST_QUIET_MS, |me| me.enter(ScrollPhase::Idle));
}

/// How long after a key that scrolls a movement of the list is still the key's, in
/// microseconds: `GtkListView` moves its cursor at once but scrolls to it at the next layout.
const KEY_SCROLL_US: i64 = 250_000;

/// Whether `key` scrolls a vertical list: `GtkListView` moves its cursor by a row, a page or to
/// either end on these and scrolls the cursor into view, and `GtkScrolledWindow` scrolls on
/// their Control forms.
fn scrolls_list(key: gtk4::gdk::Key) -> bool {
    use gtk4::gdk::Key;
    [
        Key::Up,
        Key::Down,
        Key::Page_Up,
        Key::Page_Down,
        Key::Home,
        Key::End,
        Key::KP_Up,
        Key::KP_Down,
        Key::KP_Page_Up,
        Key::KP_Page_Down,
        Key::KP_Home,
        Key::KP_End,
    ]
    .contains(&key)
}

/// Whether a wheel or touchpad step of `dy` moves `adj`: not up at the top, not down at the end.
fn step_moves(adj: &gtk4::Adjustment, dy: f64) -> bool {
    let end = (adj.upper() - adj.page_size()).max(adj.lower());
    (dy < 0.0 && adj.value() > adj.lower() + 0.5) || (dy > 0.0 && adj.value() < end - 0.5)
}

/// Report each scroll the reader starts on a list's `sw` through `report` (docs/list.md
/// "Following the end", `ListSource::reader_scroll`), and nothing else:
///
/// - a wheel or touchpad step that can move the list, from GTK's scroll controller in the
///   capture phase (the scrolled window moves the adjustment in the same dispatch);
/// - a finger on the content, or the pointer on a scroll bar (its slider or its trough): the
///   adjustment moving while one is held, as [`track_phase`] tells a drag;
/// - a key that scrolls ([`scrolls_list`]) while the list has the focus: the adjustment moving
///   within [`KEY_SCROLL_US`] after it. The key stops a running glide first, as a wheel step
///   or a press does ([`watch_user_input`]): the glide would otherwise carry the list on past
///   the key's scroll on its next frame.
///
/// A glide ([`glide`]), a reload, a splice, a resize and content growth move the adjustment with
/// none of these active. The momentum of a reader's touch drag or touchpad swipe moves it with
/// none active either, and needs no report: its start was reported.
pub(crate) fn track_reader(sw: &gtk4::ScrolledWindow, report: Rc<dyn Fn()>) {
    // A finger on the content or the pointer on a scroll bar.
    let held = Rc::new(Cell::new(false));
    // The monotonic time until which a movement is a key's.
    let key_until = Rc::new(Cell::new(0_i64));
    let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
    scroll.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let (report, adj) = (report.clone(), sw.vadjustment());
        scroll.connect_scroll(move |_, _, dy| {
            if step_moves(&adj, dy) {
                report();
            }
            glib::Propagation::Proceed
        });
    }
    sw.add_controller(scroll);
    let touch = gtk4::EventControllerLegacy::new();
    touch.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let held = held.clone();
        touch.connect_event(move |_, event| {
            match event.event_type() {
                gtk4::gdk::EventType::TouchBegin => held.set(true),
                gtk4::gdk::EventType::TouchEnd | gtk4::gdk::EventType::TouchCancel => {
                    held.set(false);
                }
                _ => {}
            }
            glib::Propagation::Proceed
        });
    }
    sw.add_controller(touch);
    let grab = gtk4::EventControllerLegacy::new();
    grab.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let held = held.clone();
        grab.connect_event(move |_, event| {
            match event.event_type() {
                gtk4::gdk::EventType::ButtonPress => held.set(true),
                gtk4::gdk::EventType::ButtonRelease => held.set(false),
                _ => {}
            }
            glib::Propagation::Proceed
        });
    }
    sw.vscrollbar().add_controller(grab);
    let keys = gtk4::EventControllerKey::new();
    keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let (key_until, sw_key) = (key_until.clone(), sw.as_ptr() as usize);
        keys.connect_key_pressed(move |_, key, _, _| {
            if scrolls_list(key) {
                GLIDES.with(|t| t.remove(sw_key));
                key_until.set(glib::monotonic_time() + KEY_SCROLL_US);
            }
            glib::Propagation::Proceed
        });
    }
    sw.add_controller(keys);
    let weak = sw.downgrade();
    sw.vadjustment().connect_value_changed(move |_| {
        let keyed = glib::monotonic_time() < key_until.get()
            && weak.upgrade().is_some_and(|sw| !gliding(&sw));
        if held.get() || keyed {
            report();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(at: u32, removed: u32, added: u32) -> ModelOp {
        ModelOp { at, removed, added }
    }

    #[test]
    fn plan_splice_merges_an_append_and_a_bottom_up_removal() {
        // 200 rows appended to 3: one `Insert(3)` per row, each above the one before.
        let append = vec![RowDelta::Insert(3); 200];
        assert_eq!(plan_splice(3, &append, 203), Some(vec![op(3, 0, 200)]));
        // Removals arrive highest index first.
        let remove = [
            RowDelta::Remove(5),
            RowDelta::Remove(4),
            RowDelta::Remove(3),
        ];
        assert_eq!(plan_splice(10, &remove, 7), Some(vec![op(3, 3, 0)]));
        // Forward removals at one index merge too.
        let forward = [RowDelta::Remove(2), RowDelta::Remove(2)];
        assert_eq!(plan_splice(4, &forward, 2), Some(vec![op(2, 2, 0)]));
    }

    #[test]
    fn plan_splice_turns_a_move_into_a_removal_and_an_insert() {
        // The keyed case: the last of four rows moves to the top.
        assert_eq!(
            plan_splice(4, &[RowDelta::Move(3, 0)], 4),
            Some(vec![op(3, 1, 0), op(0, 0, 1)])
        );
        // A move onto itself is nothing.
        assert_eq!(plan_splice(4, &[RowDelta::Move(2, 2)], 4), Some(vec![]));
        // An insert at the top then a removal from the middle stay separate runs.
        assert_eq!(
            plan_splice(3, &[RowDelta::Insert(0), RowDelta::Remove(2)], 3),
            Some(vec![op(0, 0, 1), op(2, 1, 0)])
        );
    }

    #[test]
    fn plan_splice_refuses_deltas_that_disagree_with_the_model() {
        // An index past the end of the list as the earlier deltas left it.
        assert_eq!(plan_splice(2, &[RowDelta::Insert(3)], 3), None);
        assert_eq!(plan_splice(2, &[RowDelta::Remove(2)], 1), None);
        assert_eq!(plan_splice(2, &[RowDelta::Move(0, 2)], 2), None);
        assert_eq!(
            plan_splice(2, &[RowDelta::Remove(1), RowDelta::Remove(1)], 0),
            None
        );
        // Deltas that fit but end at a count other than the source's: the model was behind.
        assert_eq!(plan_splice(0, &[RowDelta::Insert(0)], 3), None);
        assert_eq!(plan_splice(5, &[], 4), None);
    }

    #[test]
    fn visible_range_counts_rows_partly_in_view_and_skips_overscan() {
        // 32-point rows scrolled so row 100 starts at the top of a 160-point viewport.
        let rows: Vec<(usize, f64, f64)> = (96..108)
            .map(|r| (r, (r as f64 - 100.0) * 32.0, 32.0))
            .collect();
        assert_eq!(visible_range(&rows, 160.0), Some((100, 104)));
        // Shifted 10 points down: row 99 shows its last 10, row 104 its first 22.
        let shifted: Vec<(usize, f64, f64)> =
            rows.iter().map(|&(r, t, h)| (r, t + 10.0, h)).collect();
        assert_eq!(visible_range(&shifted, 160.0), Some((99, 104)));
        assert_eq!(visible_range(&[], 160.0), None);
        assert_eq!(visible_range(&[(3, 200.0, 32.0)], 160.0), None);
    }

    #[test]
    fn pitch_range_matches_the_laid_rows() {
        assert_eq!(pitch_range(3200.0, 160.0, 32.0, 300), Some((100, 104)));
        assert_eq!(pitch_range(6272.0, 160.0, 32.0, 300), Some((196, 200)));
        assert_eq!(pitch_range(0.0, 160.0, 32.0, 3), Some((0, 2)));
        assert_eq!(pitch_range(0.0, 160.0, 32.0, 0), None);
    }

    #[test]
    fn aligned_offset_follows_each_alignment_and_clamps() {
        let end = 300.0 * 32.0 - 160.0;
        let row = |r: f64| r * 32.0;
        assert_eq!(
            aligned_offset(0.0, 160.0, end, row(100.0), 32.0, RowAlign::Start, 0.0),
            3200.0
        );
        assert_eq!(
            aligned_offset(0.0, 160.0, end, row(200.0), 32.0, RowAlign::End, 0.0),
            6272.0
        );
        assert_eq!(
            aligned_offset(0.0, 160.0, end, row(100.0), 32.0, RowAlign::Center, 0.0),
            3136.0
        );
        // Row 0 at the end just stays at the top; the last row at the start stops at the end.
        assert_eq!(
            aligned_offset(500.0, 160.0, end, 0.0, 32.0, RowAlign::End, 0.0),
            0.0
        );
        assert_eq!(
            aligned_offset(0.0, 160.0, end, row(299.0), 32.0, RowAlign::Start, 0.0),
            end
        );
        // Nearest: no move when in view, the least move otherwise, below a pinned header.
        assert_eq!(
            aligned_offset(3200.0, 160.0, end, row(102.0), 32.0, RowAlign::Nearest, 0.0),
            3200.0
        );
        assert_eq!(
            aligned_offset(3200.0, 160.0, end, row(110.0), 32.0, RowAlign::Nearest, 0.0),
            row(111.0) - 160.0
        );
        assert_eq!(
            aligned_offset(3200.0, 160.0, end, row(90.0), 32.0, RowAlign::Nearest, 24.0),
            row(90.0) - 24.0
        );
    }

    #[test]
    fn reveal_moves_least_and_eased_settles() {
        assert_eq!(reveal(0.0, 120.0, 1199.0, 1200.0), 1080.0);
        assert_eq!(reveal(1080.0, 120.0, 0.0, 1.0), 0.0);
        assert_eq!(reveal(40.0, 120.0, 60.0, 100.0), 40.0);
        assert_eq!(eased(0.0), 0.0);
        assert_eq!(eased(1.0), 1.0);
        assert!(eased(0.5) > 0.5);
        assert_eq!(eased(2.0), 1.0);
    }
}
