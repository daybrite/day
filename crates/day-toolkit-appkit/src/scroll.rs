// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Scroll views and list viewports (docs/scroll.md, docs/list.md "Visible rows" and
//! "Programmatic scrolling").
//!
//! A `scroll` is a `DayScrollView`, an `NSScrollView` that reports its own movement: the clip
//! view's bounds-changed notification gives the offset and the magnification, the live-scroll
//! notifications and the scroll-wheel event phases give the user's scrolling phases, and
//! `allowsMagnification` gives zoom. Only a scroll view that tracks phases or snaps is a
//! `DayPhasedScrollView`, the subclass that overrides `scrollWheel:`: AppKit turns responsive
//! scrolling off for a class that overrides it, so no other scroll view pays for it.
//!
//! Programmatic scrolls, the scroll view's and the list's alike, move the clip view: a jump with
//! `scrollToPoint:`, an animation through the clip view's animator inside an
//! `NSAnimationContext` group, AppKit's own animated scroll.
use super::*;
use block2::RcBlock;
use day_spec::ScrollPhase;
use objc2_app_kit::{NSClipView, NSEventPhase, NSScrollElasticity};
use std::ptr::NonNull;

// ---------------------------------------------------------------------------
// Pure logic (unit-tested below)
// ---------------------------------------------------------------------------

/// One axis of a minimal reveal: where a viewport `view_len` long, now at `view`, moves so that
/// the span `[lo, lo + len)` shows, moving as little as it can (`scrollRectToVisible:`'s rule). A
/// span already in view keeps the viewport where it is; a span longer than the viewport fills
/// it. Not clamped to the content.
pub(super) fn reveal_origin(view: f64, view_len: f64, lo: f64, len: f64) -> f64 {
    let (a, b) = (lo, lo + len - view_len);
    view.clamp(a.min(b), a.max(b))
}

/// Where the viewport's top goes to show the row spanning `top..bottom` as `align` says
/// (`ListScroll::align`). Not clamped to the content: the clip view does that.
pub(super) fn row_origin(
    view: f64,
    view_len: f64,
    top: f64,
    bottom: f64,
    align: day_spec::RowAlign,
) -> f64 {
    match align {
        day_spec::RowAlign::Nearest => reveal_origin(view, view_len, top, bottom - top),
        day_spec::RowAlign::Start => top,
        day_spec::RowAlign::Center => f64::midpoint(top, bottom) - view_len / 2.0,
        day_spec::RowAlign::End => bottom - view_len,
    }
}

/// How far into a row an edge must reach for the row to count as showing: rows that only
/// touch the viewport's edge (row 99 ends exactly where a viewport starting at row 100 begins)
/// are outside it.
const TOUCH: f64 = 1e-3;

/// The first and last rows with any part inside the viewport `top..bottom`, from the rows
/// `first..first + count` that `rowsInRect:` offered (`ListViewport::first`, `last`). `row`
/// answers a row's `(top, bottom)`. `None` with no rows.
pub(super) fn visible_span(
    top: f64,
    bottom: f64,
    first: usize,
    count: usize,
    row: impl Fn(usize) -> (f64, f64),
) -> Option<(usize, usize)> {
    if count == 0 {
        return None;
    }
    let (mut lo, mut hi) = (first, first + count - 1);
    while lo < hi && row(lo).1 <= top + TOUCH {
        lo += 1;
    }
    while hi > lo && row(hi).0 >= bottom - TOUCH {
        hi -= 1;
    }
    Some((lo, hi))
}

/// Where an emulated snap (`ScrollProps::snap`) settles content that came to rest at `offset`
/// along the scroll axis: the nearest multiple of the interval, or the nearest page within one
/// page of `start`, the offset the gesture began at. `extent` is the viewport's length along the
/// axis and `max` the largest offset the content allows; the end of the content is a stop too, so
/// a last partial page can be reached. `None` when the view does not snap.
pub(super) fn snap_stop(
    offset: f64,
    start: f64,
    extent: f64,
    snap: ScrollSnap,
    max: f64,
) -> Option<f64> {
    let max = max.max(0.0);
    // `None`, and an interval or a page with no usable length, rest anywhere.
    let stop = match snap {
        ScrollSnap::Interval(step) if step.is_finite() && step > 0.0 => {
            (offset / step).round() * step
        }
        ScrollSnap::Page if extent.is_finite() && extent > 0.0 => {
            let from = (start / extent).round();
            (offset / extent).round().clamp(from - 1.0, from + 1.0) * extent
        }
        _ => return None,
    };
    let stop = stop.clamp(0.0, max);
    Some(if (offset - max).abs() < (offset - stop).abs() {
        max
    } else {
        stop
    })
}

/// One step of an `NSEvent`'s `phase` or `momentumPhase`, as the phase tracker reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WheelStep {
    /// No gesture: a classic mouse wheel tick (for `phase`), no momentum (for `momentumPhase`).
    None,
    /// The fingers are on the trackpad but nothing moves yet (`MayBegin`).
    Touch,
    Began,
    Changed,
    Ended,
}

impl WheelStep {
    fn of(phase: NSEventPhase) -> Self {
        if phase.contains(NSEventPhase::Began) {
            Self::Began
        } else if phase.intersects(NSEventPhase::Changed | NSEventPhase::Stationary) {
            Self::Changed
        } else if phase.intersects(NSEventPhase::Ended | NSEventPhase::Cancelled) {
            Self::Ended
        } else if phase.contains(NSEventPhase::MayBegin) {
            Self::Touch
        } else {
            Self::None
        }
    }
}

/// What moved a scroll view, as the phase tracker hears it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PhaseInput {
    /// `NSScrollViewWillStartLiveScrollNotification`: a trackpad gesture or a scroller drag.
    LiveStart,
    /// `NSScrollViewDidEndLiveScrollNotification`: the live scroll, momentum included, is over.
    LiveEnd,
    /// A `scrollWheel:` event with its gesture and momentum phases.
    Wheel {
        gesture: WheelStep,
        momentum: WheelStep,
    },
    /// No wheel event since the last one that armed the quiet timer.
    Quiet,
}

/// The tracker's answer to one input: the phase now, and whether to (re)arm the quiet timer
/// that ends a burst AppKit gives no end for (a classic wheel, a gesture with neither momentum
/// nor a live-scroll end).
pub(super) fn next_phase(now: ScrollPhase, input: PhaseInput) -> (ScrollPhase, bool) {
    match input {
        PhaseInput::LiveStart => (ScrollPhase::Dragging, false),
        // Every motion over: the live scroll ended, the momentum ran out, or a burst went quiet.
        PhaseInput::LiveEnd
        | PhaseInput::Quiet
        | PhaseInput::Wheel {
            momentum: WheelStep::Ended,
            ..
        } => (ScrollPhase::Idle, false),
        PhaseInput::Wheel {
            momentum: WheelStep::Began | WheelStep::Changed,
            ..
        } => (ScrollPhase::Coasting, false),
        PhaseInput::Wheel { gesture, .. } => match gesture {
            WheelStep::Began | WheelStep::Changed => (ScrollPhase::Dragging, false),
            // The fingers lifted: momentum or the live-scroll end follows at once if AppKit
            // has one; otherwise the quiet timer ends the scroll.
            WheelStep::Ended => (now, true),
            WheelStep::Touch => (now, false),
            // A classic wheel tick: moving now, over once the ticks stop.
            WheelStep::None => (ScrollPhase::Dragging, true),
        },
    }
}

/// How long a wheel burst must be quiet before it is over: the scroll piece's own emulation
/// waits as long (docs/scroll.md "Offset and phase").
const WHEEL_QUIET_MS: u32 = 150;

// ---------------------------------------------------------------------------
// The scroll view
// ---------------------------------------------------------------------------

/// A scroll view's realize-time preferences and what it last reported. Both scroll view
/// classes carry it.
pub(super) struct ScrollIvars {
    node: NodeId,
    horizontal: bool,
    track_offset: bool,
    track_phase: bool,
    snap: ScrollSnap,
    zoom: Option<ZoomRange>,
    /// The offset last sent as `Event::ScrollChanged`.
    offset: Cell<Option<(f64, f64)>>,
    /// The scale last sent as `Event::ScrollZoomed`.
    scale: Cell<f64>,
    /// The phase the tracker is in, sent or not.
    phase: Cell<ScrollPhase>,
    /// The offset along the scroll axis where the user's scroll began (a `Page` snap moves at
    /// most one page from it).
    start: Cell<f64>,
    /// Bumped by every wheel event, so a quiet timer armed before a newer event does nothing.
    wheel: Cell<u64>,
    /// Bumped by every `ScrollPatch::Zoom`, so an animated zoom that a newer one replaced does
    /// not land its own scale when its group completes.
    zooms: Cell<u64>,
}

define_class!(
    /// A scroll view that reports its offset and scale (docs/scroll.md).
    #[unsafe(super(NSScrollView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayScrollView"]
    #[ivars = ScrollIvars]
    pub(super) struct DayScrollView;

    unsafe impl NSObjectProtocol for DayScrollView {}

    impl DayScrollView {
        /// The clip view's bounds changed: the content moved or the magnification changed.
        #[unsafe(method(dayBoundsChanged:))]
        fn bounds_changed(&self, _notification: &NSNotification) {
            ffi_guard::contain((), || report(self, self.ivars(), false));
        }

        /// A pinch ended: the final scale, even when it equals the last one reported.
        #[unsafe(method(dayLiveMagnifyEnded:))]
        fn live_magnify_ended(&self, _notification: &NSNotification) {
            ffi_guard::contain((), || report(self, self.ivars(), true));
        }
    }
);

define_class!(
    /// A scroll view that also tracks the user's scrolling phases or snaps: it reads every
    /// `scrollWheel:` event for its gesture and momentum phases, which costs it AppKit's
    /// responsive scrolling, so only a view that needs them is one.
    #[unsafe(super(NSScrollView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayPhasedScrollView"]
    #[ivars = ScrollIvars]
    pub(super) struct DayPhasedScrollView;

    unsafe impl NSObjectProtocol for DayPhasedScrollView {}

    impl DayPhasedScrollView {
        #[unsafe(method(dayBoundsChanged:))]
        fn bounds_changed(&self, _notification: &NSNotification) {
            ffi_guard::contain((), || report(self, self.ivars(), false));
        }

        #[unsafe(method(dayLiveMagnifyEnded:))]
        fn live_magnify_ended(&self, _notification: &NSNotification) {
            ffi_guard::contain((), || report(self, self.ivars(), true));
        }

        #[unsafe(method(dayLiveScrollStarted:))]
        fn live_scroll_started(&self, _notification: &NSNotification) {
            ffi_guard::contain((), || track(self, self.ivars(), PhaseInput::LiveStart));
        }

        #[unsafe(method(dayLiveScrollEnded:))]
        fn live_scroll_ended(&self, _notification: &NSNotification) {
            ffi_guard::contain((), || track(self, self.ivars(), PhaseInput::LiveEnd));
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            ffi_guard::contain((), || {
                let _: () = unsafe { msg_send![super(self), scrollWheel: event] };
                let gesture = WheelStep::of(unsafe { event.phase() });
                let momentum = WheelStep::of(unsafe { event.momentumPhase() });
                let ivars = self.ivars();
                ivars.wheel.set(ivars.wheel.get().wrapping_add(1));
                track(self, ivars, PhaseInput::Wheel { gesture, momentum });
            });
        }
    }
);

/// A Day scroll view's native view and its state, whichever of the two classes it is.
fn parts(h: &NSView) -> Option<(&NSScrollView, &ScrollIvars)> {
    if let Some(sv) = h.downcast_ref::<DayScrollView>() {
        return Some((sv, sv.ivars()));
    }
    h.downcast_ref::<DayPhasedScrollView>()
        .map(|sv| (&**sv, sv.ivars()))
}

/// The content offset as `Event::ScrollChanged` and `Toolkit::scroll_offset` speak it: in the
/// viewport's points, so a zoomed view's offset grows with the scale.
fn viewport_offset(sv: &NSScrollView) -> (f64, f64) {
    let origin = unsafe { sv.contentView() }.bounds().origin;
    let scale = unsafe { sv.magnification() };
    (origin.x * scale, origin.y * scale)
}

/// Send what changed: the scale (with `zoom`) and the offset (with `track_offset`). `settle`
/// sends the scale even when it did not change, the final report a binding relies on to learn a
/// clamped value.
fn report(sv: &NSScrollView, ivars: &ScrollIvars, settle: bool) {
    if ivars.zoom.is_some() {
        let scale = unsafe { sv.magnification() };
        if settle || (scale - ivars.scale.get()).abs() > 1e-6 {
            ivars.scale.set(scale);
            emit(ivars.node, Event::ScrollZoomed(scale));
        }
    }
    if ivars.track_offset {
        let now = viewport_offset(sv);
        if ivars.offset.replace(Some(now)) != Some(now) {
            emit(ivars.node, Event::ScrollChanged(Point::new(now.0, now.1)));
        }
    }
}

/// The offset along the scroll axis, in content points.
fn along(sv: &NSScrollView, ivars: &ScrollIvars) -> f64 {
    let origin = unsafe { sv.contentView() }.bounds().origin;
    if ivars.horizontal { origin.x } else { origin.y }
}

/// Feed one input to the phase tracker: send the phase it moves to, arm the quiet timer, and
/// snap where the scroll ended.
fn track(sv: &NSScrollView, ivars: &ScrollIvars, input: PhaseInput) {
    let before = ivars.phase.get();
    let (after, quiet) = next_phase(before, input);
    if before == ScrollPhase::Idle && after != ScrollPhase::Idle {
        ivars.start.set(along(sv, ivars));
    }
    if quiet {
        arm_quiet(sv, ivars);
    }
    if after == ScrollPhase::Idle && before != ScrollPhase::Idle && snap(sv, ivars) {
        return; // the snap animation coasts, then rests
    }
    enter(ivars, after);
}

fn enter(ivars: &ScrollIvars, phase: ScrollPhase) {
    if ivars.phase.replace(phase) != phase && ivars.track_phase {
        emit(ivars.node, Event::ScrollPhase(phase));
    }
}

/// End a wheel burst after `WHEEL_QUIET_MS` without another wheel event.
fn arm_quiet(sv: &NSScrollView, ivars: &ScrollIvars) {
    let generation = ivars.wheel.get();
    // A +1 pointer across the post (the closure must be Send, which Retained is not; everything
    // stays on the main thread).
    let this = Retained::into_raw(sv.retain()) as usize;
    <AppKit as Platform>::post_delayed(
        WHEEL_QUIET_MS,
        Box::new(move || {
            // Reclaim the +1 first, so every return balances it.
            let Some(sv) = (unsafe { Retained::from_raw(this as *mut NSScrollView) }) else {
                return;
            };
            if let Some((sv, ivars)) = parts(&sv)
                && ivars.wheel.get() == generation
            {
                track(sv, ivars, PhaseInput::Quiet);
            }
        }),
    );
}

/// The scroll came to rest: animate it to the nearest stop when it snaps (emulated, docs/
/// scroll.md "Snapping"), coasting until it gets there. `false` when there is nothing to do.
fn snap(sv: &NSScrollView, ivars: &ScrollIvars) -> bool {
    let clip = unsafe { sv.contentView() };
    let bounds = clip.bounds();
    let Some(doc) = (unsafe { sv.documentView() }) else {
        return false;
    };
    let content = doc.frame().size;
    let (extent, max) = if ivars.horizontal {
        (bounds.size.width, content.width - bounds.size.width)
    } else {
        (bounds.size.height, content.height - bounds.size.height)
    };
    let offset = along(sv, ivars);
    let Some(stop) = snap_stop(offset, ivars.start.get(), extent, ivars.snap, max) else {
        return false;
    };
    if (stop - offset).abs() < 0.5 {
        return false;
    }
    enter(ivars, ScrollPhase::Coasting);
    let mut to = bounds.origin;
    if ivars.horizontal {
        to.x = stop;
    } else {
        to.y = stop;
    }
    let view = sv.retain();
    scroll_clip_to(
        sv,
        to,
        true,
        Some(Box::new(move || {
            if let Some((_, ivars)) = parts(&view) {
                enter(ivars, ScrollPhase::Idle);
            }
        })),
    );
    true
}

/// Build the native scroll view for `kinds::SCROLL` with its preferences (`ScrollProps`).
pub(super) fn realize(mtm: MainThreadMarker, node: NodeId, p: &ScrollProps) -> Retained<NSView> {
    let ivars = ScrollIvars {
        node,
        horizontal: p.horizontal,
        track_offset: p.track_offset,
        track_phase: p.track_phase,
        snap: p.snap,
        zoom: p.zoom,
        offset: Cell::new(None),
        scale: Cell::new(1.0),
        phase: Cell::new(ScrollPhase::Idle),
        start: Cell::new(0.0),
        wheel: Cell::new(0),
        zooms: Cell::new(0),
    };
    let phased = p.track_phase || p.snap != ScrollSnap::None;
    let sv: Retained<NSScrollView> = if phased {
        let this = DayPhasedScrollView::alloc(mtm).set_ivars(ivars);
        let this: Retained<DayPhasedScrollView> = unsafe { msg_send![super(this), init] };
        Retained::into_super(this)
    } else {
        let this = DayScrollView::alloc(mtm).set_ivars(ivars);
        let this: Retained<DayScrollView> = unsafe { msg_send![super(this), init] };
        Retained::into_super(this)
    };
    let hidden = p.indicators == ScrollIndicators::Hidden;
    unsafe {
        sv.setHasVerticalScroller(!p.horizontal && !hidden);
        sv.setHasHorizontalScroller(p.horizontal && !hidden);
        sv.setDrawsBackground(false);
        // Overlay scrollers ALWAYS: day lays content out at the scroll's full frame width, and
        // a legacy scroller (the "always show scroll bars" system setting, or a mouse attached)
        // would reserve ~15pt and clip trailing content. Overlay floats above content instead,
        // and matches the GTK/Qt backends.
        sv.setScrollerStyle(objc2_app_kit::NSScrollerStyle::Overlay);
        // `false` stops dead at the edges; `true` rubber-bands even when the content fits.
        let (along, across) = match p.bounce {
            None => (NSScrollElasticity::Automatic, NSScrollElasticity::Automatic),
            Some(false) => (NSScrollElasticity::None, NSScrollElasticity::None),
            Some(true) => (NSScrollElasticity::Allowed, NSScrollElasticity::Automatic),
        };
        let (vertical, horizontal) = if p.horizontal {
            (across, along)
        } else {
            (along, across)
        };
        sv.setVerticalScrollElasticity(vertical);
        sv.setHorizontalScrollElasticity(horizontal);
        if let Some(zoom) = p.zoom {
            sv.setAllowsMagnification(true);
            sv.setMinMagnification(zoom.min);
            sv.setMaxMagnification(zoom.max);
        }
    }
    let doc = DayFlipped::new(mtm);
    unsafe { sv.setDocumentView(Some(doc.as_ref())) };
    let center = objc2_foundation::NSNotificationCenter::defaultCenter();
    let clip = unsafe { sv.contentView() };
    // No observer without a reason (docs/scroll.md): a plain scroll view hears nothing.
    unsafe {
        if p.track_offset || p.zoom.is_some() {
            clip.setPostsBoundsChangedNotifications(true);
            center.addObserver_selector_name_object(
                &sv,
                sel!(dayBoundsChanged:),
                Some(objc2_app_kit::NSViewBoundsDidChangeNotification),
                Some(&clip),
            );
        }
        if phased {
            center.addObserver_selector_name_object(
                &sv,
                sel!(dayLiveScrollStarted:),
                Some(objc2_app_kit::NSScrollViewWillStartLiveScrollNotification),
                Some(&sv),
            );
            center.addObserver_selector_name_object(
                &sv,
                sel!(dayLiveScrollEnded:),
                Some(objc2_app_kit::NSScrollViewDidEndLiveScrollNotification),
                Some(&sv),
            );
        }
        if p.zoom.is_some() {
            center.addObserver_selector_name_object(
                &sv,
                sel!(dayLiveMagnifyEnded:),
                Some(objc2_app_kit::NSScrollViewDidEndLiveMagnifyNotification),
                Some(&sv),
            );
        }
    }
    Retained::into_super(sv)
}

/// `Toolkit::scroll_offset` for a scroll view: its content offset in viewport points.
pub(super) fn offset(h: &NSView) -> Point {
    match h.downcast_ref::<NSScrollView>() {
        Some(sv) => {
            let (x, y) = viewport_offset(sv);
            Point::new(x, y)
        }
        None => Point::ZERO,
    }
}

/// `ScrollPatch::Zoom`: zoom to `scale`, clamped to the view's range, keeping the viewport's
/// center where it is, animated or not; the view reports the scale it settles on.
pub(super) fn zoom(h: &NSView, scale: f64, animated: bool) {
    let Some((sv, ivars)) = parts(h) else {
        return;
    };
    let Some(range) = ivars.zoom else {
        return;
    };
    if !scale.is_finite() {
        return;
    }
    let scale = scale.clamp(range.min, range.max);
    let zoom = ivars.zooms.get().wrapping_add(1);
    ivars.zooms.set(zoom);
    let clip = unsafe { sv.contentView() };
    let bounds = clip.bounds();
    let center = NSPoint::new(
        bounds.origin.x + bounds.size.width / 2.0,
        bounds.origin.y + bounds.size.height / 2.0,
    );
    // Reduce Motion lands the zoom at once (docs/scroll.md), as it does a scroll; AppKit would
    // otherwise apply the animator's change instantly, about another anchor.
    if !animated || day_core::reduce_motion_now() {
        land_zoom(sv, scale, center);
        report(sv, ivars, true);
        return;
    }
    // The animator steps the magnification; the view reports each step through its clip view's
    // bounds, and the scale it rests on when the group completes.
    let (target, done) = (sv.retain(), sv.retain());
    let changes = RcBlock::new(move |_ctx: NonNull<NSAnimationContext>| {
        // Through key-value coding: the animator proxy resolves `magnification` dynamically,
        // so a typed `setMagnification:` send fails objc2's method check.
        let animator: Retained<AnyObject> = unsafe { msg_send![&*target, animator] };
        let value = NSNumber::new_f64(scale);
        let key = NSString::from_str("magnification");
        let _: () = unsafe { msg_send![&*animator, setValue: &*value, forKey: &*key] };
    });
    let completion = RcBlock::new(move || {
        ffi_guard::contain((), || {
            let Some((sv, ivars)) = parts(&done) else {
                return;
            };
            // A newer zoom replaced this one: AppKit still completes this group, and landing
            // its scale now would undo the newer zoom, which reports where it rests itself.
            if ivars.zooms.get() != zoom {
                return;
            }
            // Exactly the scale asked for, about the center the zoom started from.
            land_zoom(sv, scale, center);
            report(sv, ivars, true);
        });
    });
    unsafe { NSAnimationContext::runAnimationGroup_completionHandler(&changes, Some(&completion)) };
}

/// Set `sv`'s magnification to `scale` with `center` (document coordinates) at the middle of the
/// viewport. `setMagnification:centeredAtPoint:` does nothing when the scale is already `scale`
/// (an animator that landed it about another anchor), so the clip view is then moved to put
/// `center` back in the middle.
fn land_zoom(sv: &NSScrollView, scale: f64, center: NSPoint) {
    unsafe { sv.setMagnification_centeredAtPoint(scale, center) };
    let clip = unsafe { sv.contentView() };
    let mut bounds = clip.bounds();
    let off_x = bounds.origin.x + bounds.size.width / 2.0 - center.x;
    let off_y = bounds.origin.y + bounds.size.height / 2.0 - center.y;
    if off_x.abs() < 0.5 && off_y.abs() < 0.5 {
        return;
    }
    bounds.origin = NSPoint::new(
        center.x - bounds.size.width / 2.0,
        center.y - bounds.size.height / 2.0,
    );
    let to = clip.constrainBoundsRect(bounds).origin;
    clip.scrollToPoint(to);
    sv.reflectScrolledClipView(&clip);
}

/// Move `scroll`'s clip view so its bounds origin is `to` (clamped to the content), with AppKit's
/// animated scroll when `animated`, else at once; then run `done`. A jump also replaces an
/// animated scroll still running, so the last command wins either way.
pub(super) fn scroll_clip_to(
    scroll: &NSScrollView,
    to: NSPoint,
    animated: bool,
    done: Option<Box<dyn FnOnce()>>,
) {
    let clip: Retained<NSClipView> = unsafe { scroll.contentView() };
    let mut bounds = clip.bounds();
    bounds.origin = to;
    let to = clip.constrainBoundsRect(bounds).origin;
    if animated {
        let target = clip.clone();
        let changes = RcBlock::new(move |_ctx: NonNull<NSAnimationContext>| {
            let animator: Retained<NSClipView> = unsafe { msg_send![&*target, animator] };
            animator.setBoundsOrigin(to);
        });
        let (scroll, done) = (scroll.retain(), Cell::new(done));
        let completion = RcBlock::new(move || {
            ffi_guard::contain((), || {
                scroll.reflectScrolledClipView(&scroll.contentView());
                if let Some(done) = done.take() {
                    done();
                }
            });
        });
        unsafe {
            NSAnimationContext::runAnimationGroup_completionHandler(&changes, Some(&completion));
        }
    } else {
        // Stop one in flight, which would otherwise go on moving the clip view after the jump.
        settle_clip_at(&clip, to);
        clip.scrollToPoint(to);
        unsafe { scroll.reflectScrolledClipView(&clip) };
        if let Some(done) = done {
            done();
        }
    }
}

/// Set `clip`'s bounds origin to `to` through a zero-length animation of the same property,
/// which stops an animated scroll still running.
fn settle_clip_at(clip: &NSClipView, to: NSPoint) {
    unsafe {
        NSAnimationContext::beginGrouping();
        NSAnimationContext::currentContext().setDuration(0.0);
        let animator: Retained<NSClipView> = msg_send![clip, animator];
        animator.setBoundsOrigin(to);
        NSAnimationContext::endGrouping();
    }
}

/// Stop an animated scroll of `scroll` that is still running where it is now: the reader took
/// the list (docs/list.md "Following the end"), and a scroll the app asked for must not carry
/// it on past them.
pub(super) fn stop_clip_animation(scroll: &NSScrollView) {
    let clip: Retained<NSClipView> = unsafe { scroll.contentView() };
    settle_clip_at(&clip, clip.bounds().origin);
}

#[cfg(test)]
mod tests {
    use super::*;
    use day_spec::RowAlign;

    #[test]
    fn reveal_moves_as_little_as_it_can() {
        // In view: stays.
        assert_eq!(reveal_origin(100.0, 50.0, 110.0, 20.0), 100.0);
        // Below: its bottom at the viewport's bottom.
        assert_eq!(reveal_origin(100.0, 50.0, 160.0, 20.0), 130.0);
        // Above: its top at the viewport's top.
        assert_eq!(reveal_origin(100.0, 50.0, 40.0, 20.0), 40.0);
        // Longer than the viewport: fills it, from wherever is nearest.
        assert_eq!(reveal_origin(0.0, 50.0, 100.0, 200.0), 100.0);
        assert_eq!(reveal_origin(400.0, 50.0, 100.0, 200.0), 250.0);
        assert_eq!(reveal_origin(150.0, 50.0, 100.0, 200.0), 150.0);
    }

    #[test]
    fn rows_land_where_their_alignment_says() {
        // Row 100 of 32-point rows in a 160-point viewport.
        let (top, bottom) = (3200.0, 3232.0);
        assert_eq!(row_origin(0.0, 160.0, top, bottom, RowAlign::Start), 3200.0);
        assert_eq!(row_origin(0.0, 160.0, top, bottom, RowAlign::End), 3072.0);
        assert_eq!(
            row_origin(0.0, 160.0, top, bottom, RowAlign::Center),
            3136.0
        );
        assert_eq!(
            row_origin(0.0, 160.0, top, bottom, RowAlign::Nearest),
            3072.0
        );
        assert_eq!(
            row_origin(3100.0, 160.0, top, bottom, RowAlign::Nearest),
            3100.0
        );
    }

    #[test]
    fn rows_that_only_touch_the_viewport_are_outside_it() {
        let row = |i: usize| (i as f64 * 32.0, (i + 1) as f64 * 32.0);
        // At row 100's top, rowsInRect offers 99..=105 (99 and 105 touch the edges).
        assert_eq!(visible_span(3200.0, 3360.0, 99, 7, row), Some((100, 104)));
        // Half a row past: both partial rows count.
        assert_eq!(visible_span(3216.0, 3376.0, 100, 6, row), Some((100, 105)));
        assert_eq!(visible_span(0.0, 160.0, 0, 0, row), None);
        // One row taller than the viewport.
        assert_eq!(
            visible_span(10.0, 20.0, 0, 1, |_| (0.0, 100.0)),
            Some((0, 0))
        );
    }

    #[test]
    fn snaps_rest_on_their_stops() {
        assert_eq!(snap_stop(130.0, 0.0, 100.0, ScrollSnap::None, 1000.0), None);
        assert_eq!(
            snap_stop(130.0, 0.0, 100.0, ScrollSnap::Interval(40.0), 1000.0),
            Some(120.0)
        );
        assert_eq!(
            snap_stop(160.0, 100.0, 100.0, ScrollSnap::Page, 1000.0),
            Some(200.0)
        );
        assert_eq!(
            snap_stop(140.0, 100.0, 100.0, ScrollSnap::Page, 1000.0),
            Some(100.0)
        );
        // A page snap moves at most one page from where the scroll began.
        assert_eq!(
            snap_stop(480.0, 100.0, 100.0, ScrollSnap::Page, 1000.0),
            Some(200.0)
        );
        // The content's end is a stop: the last partial page is reachable.
        assert_eq!(
            snap_stop(950.0, 900.0, 100.0, ScrollSnap::Page, 950.0),
            Some(950.0)
        );
        assert_eq!(
            snap_stop(10.0, 0.0, 100.0, ScrollSnap::Interval(0.0), 1000.0),
            None
        );
    }

    #[test]
    fn phases_follow_gestures_momentum_and_wheels() {
        use PhaseInput::{LiveEnd, LiveStart, Quiet, Wheel};
        use ScrollPhase::{Coasting, Dragging, Idle};
        let wheel = |gesture, momentum| Wheel { gesture, momentum };
        // A trackpad flick: drag, lift, coast, rest.
        let mut now = Idle;
        for (input, want) in [
            (wheel(WheelStep::Touch, WheelStep::None), Idle),
            (LiveStart, Dragging),
            (wheel(WheelStep::Began, WheelStep::None), Dragging),
            (wheel(WheelStep::Changed, WheelStep::None), Dragging),
            (wheel(WheelStep::Ended, WheelStep::None), Dragging),
            (wheel(WheelStep::None, WheelStep::Began), Coasting),
            (wheel(WheelStep::None, WheelStep::Changed), Coasting),
            (wheel(WheelStep::None, WheelStep::Ended), Idle),
            (LiveEnd, Idle),
        ] {
            now = next_phase(now, input).0;
            assert_eq!(now, want, "{input:?}");
        }
        // Lifting with no momentum: the quiet timer is armed and ends the scroll.
        assert_eq!(
            next_phase(Dragging, wheel(WheelStep::Ended, WheelStep::None)),
            (Dragging, true)
        );
        // A classic wheel: moving at the first tick, idle once the ticks stop.
        assert_eq!(
            next_phase(Idle, wheel(WheelStep::None, WheelStep::None)),
            (Dragging, true)
        );
        assert_eq!(next_phase(Dragging, Quiet), (Idle, false));
    }
}
