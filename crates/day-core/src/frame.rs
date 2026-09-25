// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Window-scoped display scheduling (docs/frames.md). Native frame requests are shared by
//! one-shot clients, continuous subscriptions and the legacy `frame_clock` piece. No timers,
//! no implicit physics clamp, and no native request when there is no demand.

use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::ops::ControlFlow;
use std::rc::Rc;
use std::time::Duration;

use crate::tree::RNode;
use day_spec::{CancelFrame, FrameCallback, FrameStamp};

/// A frame opportunity, not confirmation that pixels were presented. All clients of a window
/// receive the same timestamp. `delta` belongs to this subscription: zero on its first frame
/// and after an explicit pause/resume. It is not clamped; simulation clients choose their policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub timestamp: Duration,
    pub delta: Duration,
    pub target_timestamp: Option<Duration>,
}

type Callback = Rc<RefCell<Box<dyn FnMut(Frame) -> ControlFlow<()>>>>;
type Requester = Rc<dyn Fn(RNode, FrameCallback) -> CancelFrame>;
struct Consumer {
    root: RNode,
    cb: Callback,
    active: bool,
    generation: u64,
    last: Option<f64>,
}
#[derive(Default)]
struct Source {
    ticket: u64,
    armed: bool,
    dispatching: bool,
    cancel: Option<CancelFrame>,
}
#[derive(Default)]
struct Driver {
    requester: Option<Requester>,
    consumers: HashMap<u64, Consumer>,
    sources: HashMap<RNode, Source>,
    next_id: u64,
    suspended: bool,
}

day_reactive::tls_slots! {
    frame;
    static DRIVER: RefCell<Driver> = RefCell::new(Driver::default());
    static NATIVE_CALLBACKS: RefCell<(u64, HashMap<u64, FrameCallback>)> = RefCell::new((0, HashMap::new()));
}

/// Install the native scheduler at launch. The requester MUST deliver asynchronously; tests may
/// retain the callback and deliver chosen timestamps manually. Replaces no live registrations.
pub fn install_frame_requester(f: impl Fn(RNode, FrameCallback) -> CancelFrame + 'static) {
    DRIVER.with(|d| d.borrow_mut().requester = Some(Rc::new(f)));
}

/// A reusable source bound to one window. Capture `current()` while building a page, then use it
/// from input handlers or other UI-thread code. A closed window cancels all its registrations.
#[derive(Clone, Copy, Debug)]
pub struct FrameClock {
    root: RNode,
    ui_thread: PhantomData<Rc<()>>,
}
impl FrameClock {
    /// The current page's window (the initial window outside a page build).
    pub fn current() -> Self {
        Self::for_root(crate::toolbar::current_page_window())
    }
    pub(crate) fn for_root(root: RNode) -> Self {
        Self {
            root,
            ui_thread: PhantomData,
        }
    }
    /// Run once at the next frame opportunity. Retain the returned handle until delivery;
    /// dropping it cancels. Requests made inside a callback run on a subsequent frame.
    pub fn request(self, cb: impl FnOnce(Frame) + 'static) -> FrameHandle {
        let mut cb = Some(cb);
        self.subscribe(move |frame| {
            if let Some(cb) = cb.take() {
                cb(frame);
            }
            ControlFlow::Break(())
        })
    }
    /// Run while the callback returns `Continue(())`. `Break(())` pauses the subscription;
    /// `resume()` can start it again. Drop or `cancel()` removes it permanently.
    pub fn subscribe(self, cb: impl FnMut(Frame) -> ControlFlow<()> + 'static) -> FrameHandle {
        let id = DRIVER.with(|d| {
            let mut d = d.borrow_mut();
            d.next_id += 1;
            let id = d.next_id;
            d.consumers.insert(
                id,
                Consumer {
                    root: self.root,
                    cb: Rc::new(RefCell::new(Box::new(cb))),
                    active: true,
                    generation: 0,
                    last: None,
                },
            );
            id
        });
        arm(self.root);
        FrameHandle {
            id,
            ui_thread: PhantomData,
        }
    }
}

/// Request one frame in the current window. Prefer a captured [`FrameClock`] from event handlers.
pub fn request(cb: impl FnOnce(Frame) + 'static) -> FrameHandle {
    FrameClock::current().request(cb)
}
/// Subscribe in the current window. Dropping the returned handle cancels the subscription.
pub fn subscribe(cb: impl FnMut(Frame) -> ControlFlow<()> + 'static) -> FrameHandle {
    FrameClock::current().subscribe(cb)
}

/// Owns a registration. UI-thread only. Keep it alive for as long as the callback is wanted.
#[must_use = "dropping the frame handle cancels the callback"]
pub struct FrameHandle {
    id: u64,
    ui_thread: PhantomData<Rc<()>>,
}
impl FrameHandle {
    pub fn pause(&self) {
        set_active(self.id, false);
    }
    pub fn resume(&self) {
        set_active(self.id, true);
    }
    pub fn is_active(&self) -> bool {
        DRIVER.with(|d| d.borrow().consumers.get(&self.id).is_some_and(|c| c.active))
    }
    pub fn cancel(&self) {
        remove(self.id);
    }
    /// Keep the registration for this reactive scope; dispose cancels it. This is optional:
    /// clients without a mounted piece can own the handle directly.
    pub fn in_scope(self) {
        day_reactive::Scope::current().on_cleanup(move || drop(self));
    }
}
impl Drop for FrameHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn set_active(id: u64, active: bool) {
    let root = DRIVER.with(|d| {
        let mut d = d.borrow_mut();
        let c = d.consumers.get_mut(&id)?;
        if c.active != active {
            c.active = active;
            c.generation += 1;
            c.last = None;
        }
        Some(c.root)
    });
    if let Some(root) = root {
        reconcile(root);
    }
}
fn remove(id: u64) {
    // User closures can own other handles. Drop them outside the driver borrow so their
    // destructors can cancel those registrations without re-entering a borrowed RefCell.
    let removed = DRIVER.with(|d| d.borrow_mut().consumers.remove(&id));
    let root = removed.as_ref().map(|c| c.root);
    drop(removed);
    if let Some(root) = root {
        reconcile(root);
    }
}
fn reconcile(root: RNode) {
    let cancel = DRIVER.with(|d| {
        let mut d = d.borrow_mut();
        let active = !d.suspended && d.consumers.values().any(|c| c.root == root && c.active);
        if active {
            return None;
        }
        let source = d.sources.get_mut(&root)?;
        source.ticket += 1;
        source.armed = false;
        source.cancel.take()
    });
    if let Some(cancel) = cancel {
        cancel();
    }
    arm(root);
}
fn arm(root: RNode) {
    let request = DRIVER.with(|d| {
        let mut d = d.borrow_mut();
        if d.suspended || !d.consumers.values().any(|c| c.root == root && c.active) {
            return None;
        }
        let requester = d.requester.clone()?;
        let source = d.sources.entry(root).or_default();
        if source.armed || source.dispatching {
            return None;
        }
        source.ticket += 1;
        source.armed = true;
        Some((requester, source.ticket))
    });
    if let Some((requester, ticket)) = request {
        let cancel = requester(root, Box::new(move |stamp| tick(root, ticket, stamp)));
        DRIVER.with(|d| {
            if let Some(s) = d.borrow_mut().sources.get_mut(&root) {
                s.cancel = Some(cancel);
            }
        });
    }
}
fn tick(root: RNode, ticket: u64, stamp: FrameStamp) {
    let callbacks = DRIVER.with(|d| {
        let mut d = d.borrow_mut();
        let Some(source) = d.sources.get_mut(&root) else {
            return Vec::new();
        };
        if !source.armed || source.ticket != ticket {
            return Vec::new();
        }
        source.armed = false;
        source.cancel.take();
        if Duration::try_from_secs_f64(stamp.timestamp).is_err() {
            return Vec::new();
        }
        source.dispatching = true;
        let mut callbacks: Vec<_> = d
            .consumers
            .iter()
            .filter(|(_, c)| c.root == root && c.active)
            .map(|(&id, c)| (id, c.generation, c.cb.clone()))
            .collect();
        callbacks.sort_by_key(|c| c.0);
        callbacks
    });
    day_reactive::batch(|| {
        for (id, generation, cb) in callbacks {
            let frame = DRIVER.with(|d| {
                let mut d = d.borrow_mut();
                if d.suspended {
                    return None;
                }
                let c = d.consumers.get_mut(&id)?;
                if !c.active || c.generation != generation {
                    return None;
                }
                // Ignore backward stamps without moving the baseline backwards.
                let ts = stamp.timestamp.max(c.last.unwrap_or(stamp.timestamp));
                let delta = c.last.map_or(0.0, |last| ts - last);
                c.last = Some(ts);
                Some(Frame {
                    timestamp: Duration::from_secs_f64(ts),
                    delta: Duration::from_secs_f64(delta),
                    target_timestamp: stamp
                        .target_timestamp
                        .filter(|t| t.is_finite() && *t >= ts)
                        .and_then(|t| Duration::try_from_secs_f64(t).ok()),
                })
            });
            let Some(frame) = frame else {
                continue;
            };
            let flow = day_spec::ffi_guard::contain(ControlFlow::Break(()), || {
                cb.try_borrow_mut()
                    .map_or(ControlFlow::Break(()), |mut f| f(frame))
            });
            if flow.is_break() {
                DRIVER.with(|d| {
                    if let Some(c) = d.borrow_mut().consumers.get_mut(&id)
                        && c.generation == generation
                    {
                        c.active = false;
                        c.last = None;
                    }
                });
            }
        }
    });
    day_reactive::flush_now();
    DRIVER.with(|d| {
        if let Some(source) = d.borrow_mut().sources.get_mut(&root) {
            source.dispatching = false;
        }
    });
    reconcile(root);
}

/// Suspend display work when the app backgrounds; foreground resumes demand with a fresh delta.
/// Desktop loss of keyboard focus alone does not suspend visible windows.
pub(crate) fn lifecycle(phase: day_spec::Lifecycle) {
    match phase {
        day_spec::Lifecycle::DidEnterBackground => suspend(true),
        day_spec::Lifecycle::WillEnterForeground | day_spec::Lifecycle::DidBecomeActive => {
            suspend(false)
        }
        _ => {}
    }
}
fn suspend(suspended: bool) {
    let roots = DRIVER.with(|d| {
        let mut d = d.borrow_mut();
        if d.suspended == suspended {
            return Vec::new();
        }
        d.suspended = suspended;
        for c in d.consumers.values_mut() {
            c.last = None;
            c.generation += 1;
        }
        // A first subscription may have been created while suspended, before that window
        // ever acquired a native source. Include its demand when foregrounding too.
        d.sources
            .keys()
            .copied()
            .chain(d.consumers.values().map(|c| c.root))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    });
    for root in roots {
        reconcile(root);
    }
}
pub(crate) fn forget_window(root: RNode) {
    let (cancel, removed) = DRIVER.with(|d| {
        let mut d = d.borrow_mut();
        let ids: Vec<_> = d
            .consumers
            .iter()
            .filter(|(_, c)| c.root == root)
            .map(|(&id, _)| id)
            .collect();
        let removed: Vec<_> = ids
            .into_iter()
            .filter_map(|id| d.consumers.remove(&id))
            .collect();
        (d.sources.remove(&root).and_then(|s| s.cancel), removed)
    });
    drop(removed);
    if let Some(cancel) = cancel {
        cancel();
    }
}

/// Legacy `frame_clock` registration. New code should own a [`FrameHandle`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameConsumer(u64);
pub fn add_frame_consumer(mut cb: impl FnMut(Duration) + 'static) -> FrameConsumer {
    let mut first = true;
    let handle = subscribe(move |frame| {
        // Preserve the original first-frame and <=100ms simulation contract for existing games.
        cb(if std::mem::take(&mut first) {
            Duration::from_secs_f64(1.0 / 60.0)
        } else {
            frame.delta.min(Duration::from_millis(100))
        });
        ControlFlow::Continue(())
    });
    let id = handle.id;
    std::mem::forget(handle);
    FrameConsumer(id)
}
pub fn remove_frame_consumer(c: FrameConsumer) {
    remove(c.0);
}
/// Number of active subscriptions and pending single-frame clients (paused handles excluded).
pub fn frame_consumer_count() -> usize {
    DRIVER.with(|d| d.borrow().consumers.values().filter(|c| c.active).count())
}

/// Backend callback registry. Native APIs carry integer tickets, never pointers to Rust closures:
/// a cancelled or late native callback cannot dereference freed memory. UI-thread only.
#[doc(hidden)]
pub mod native {
    use super::*;
    pub fn register(cb: FrameCallback) -> u64 {
        NATIVE_CALLBACKS.with(|c| {
            let mut c = c.borrow_mut();
            c.0 += 1;
            let id = c.0;
            c.1.insert(id, cb);
            id
        })
    }
    pub fn cancel(id: u64) {
        NATIVE_CALLBACKS.with(|c| c.borrow_mut().1.remove(&id));
    }
    pub fn deliver(id: u64, stamp: FrameStamp) {
        let cb = NATIVE_CALLBACKS.with(|c| c.borrow_mut().1.remove(&id));
        if let Some(cb) = cb {
            day_spec::ffi_guard::contain((), || cb(stamp));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::VecDeque;

    struct Manual {
        pending: Rc<RefCell<VecDeque<(RNode, FrameCallback)>>>,
        requests: Rc<Cell<usize>>,
        cancellations: Rc<Cell<usize>>,
    }
    impl Manual {
        fn new() -> Self {
            DRIVER.with(|d| *d.borrow_mut() = Driver::default());
            let pending = Rc::new(RefCell::new(VecDeque::new()));
            let requests = Rc::new(Cell::new(0));
            let cancellations = Rc::new(Cell::new(0));
            let (q, r, c) = (pending.clone(), requests.clone(), cancellations.clone());
            install_frame_requester(move |root, cb| {
                r.set(r.get() + 1);
                q.borrow_mut().push_back((root, cb));
                let c = c.clone();
                Box::new(move || c.set(c.get() + 1))
            });
            Self {
                pending,
                requests,
                cancellations,
            }
        }
        fn fire(&self, ts: f64) {
            let (_, cb) = self
                .pending
                .borrow_mut()
                .pop_front()
                .expect("pending frame");
            cb(FrameStamp {
                timestamp: ts,
                target_timestamp: Some(ts + 1.0 / 120.0),
            });
        }
        fn clock(&self) -> FrameClock {
            FrameClock::for_root(RNode::default())
        }
    }
    #[test]
    fn clients_share_one_request_and_idle_after_delivery() {
        let m = Manual::new();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let s = seen.clone();
        let a = m.clock().request(move |f| s.borrow_mut().push(f));
        let s = seen.clone();
        let b = m.clock().request(move |f| s.borrow_mut().push(f));
        assert_eq!(m.requests.get(), 1);
        m.fire(10.0);
        assert_eq!(seen.borrow().len(), 2);
        assert_eq!(seen.borrow()[0], seen.borrow()[1]);
        assert_eq!(seen.borrow()[0].delta, Duration::ZERO);
        assert!(seen.borrow()[0].target_timestamp.is_some());
        assert!(!a.is_active() && !b.is_active());
        assert!(m.pending.borrow().is_empty());
    }
    #[test]
    fn dropping_last_client_cancels_and_late_delivery_is_harmless() {
        let m = Manual::new();
        let a = m.clock().request(|_| panic!("cancelled callback ran"));
        drop(a);
        assert_eq!(m.cancellations.get(), 1);
        m.fire(10.0); // emulate a callback already queued before native cancellation
        assert_eq!(frame_consumer_count(), 0);
        assert_eq!(m.requests.get(), 1);
    }
    #[test]
    fn cancellation_during_dispatch_skips_the_later_callback() {
        let m = Manual::new();
        let later = Rc::new(RefCell::new(None::<FrameHandle>));
        let l = later.clone();
        let _first = m.clock().request(move |_| {
            l.borrow_mut().take();
        });
        *later.borrow_mut() = Some(m.clock().request(|_| panic!("cancelled during dispatch")));
        m.fire(1.0);
        assert!(m.pending.borrow().is_empty());
    }
    #[test]
    fn requests_from_a_callback_run_on_the_next_frame() {
        let m = Manual::new();
        let later = Rc::new(RefCell::new(None::<FrameHandle>));
        let l = later.clone();
        let seen = Rc::new(Cell::new(false));
        let s = seen.clone();
        let clock = m.clock();
        let _first = clock.request(move |_| {
            *l.borrow_mut() = Some(clock.request(move |_| s.set(true)));
        });
        m.fire(1.0);
        assert!(!seen.get());
        assert_eq!(m.requests.get(), 2);
        m.fire(2.0);
        assert!(seen.get());
    }
    #[test]
    fn timestamps_are_unclamped_and_resume_starts_a_new_baseline() {
        let m = Manual::new();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let s = seen.clone();
        let handle = m.clock().subscribe(move |f| {
            s.borrow_mut().push(f.delta);
            ControlFlow::Continue(())
        });
        m.fire(1.0);
        m.fire(1.75);
        assert_eq!(seen.borrow()[1], Duration::from_millis(750));
        handle.pause();
        m.fire(2.0); // cancelled pending frame
        handle.resume();
        m.fire(50.0);
        assert_eq!(seen.borrow()[2], Duration::ZERO);
    }
    #[test]
    fn stopping_then_waking_a_subscription_does_not_poll() {
        let m = Manual::new();
        let calls = Rc::new(Cell::new(0));
        let c = calls.clone();
        let handle = m.clock().subscribe(move |_| {
            c.set(c.get() + 1);
            ControlFlow::Break(())
        });
        m.fire(1.0);
        assert!(!handle.is_active());
        assert!(m.pending.borrow().is_empty());
        handle.resume();
        handle.resume();
        assert_eq!(m.requests.get(), 2);
        m.fire(5.0);
        assert_eq!(calls.get(), 2);
    }
    #[test]
    fn windows_have_independent_native_sources() {
        let m = Manual::new();
        let mut roots = slotmap::SlotMap::<RNode, ()>::with_key();
        let one = roots.insert(());
        let two = roots.insert(());
        let _a = FrameClock::for_root(one).request(|_| {});
        let b = FrameClock::for_root(two).subscribe(|_| ControlFlow::Continue(()));
        assert_eq!(m.requests.get(), 2);
        assert_eq!(m.pending.borrow()[0].0, one);
        assert_eq!(m.pending.borrow()[1].0, two);
        forget_window(two);
        assert!(!b.is_active());
        m.fire(1.0);
        m.fire(1.0);
        assert!(m.pending.borrow().is_empty());
    }
    #[test]
    fn background_suspends_and_foreground_resets_delta() {
        let m = Manual::new();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let s = seen.clone();
        let _handle = m.clock().subscribe(move |f| {
            s.borrow_mut().push(f.delta);
            ControlFlow::Continue(())
        });
        m.fire(1.0);
        lifecycle(day_spec::Lifecycle::DidEnterBackground);
        m.fire(2.0);
        assert_eq!(seen.borrow().len(), 1);
        lifecycle(day_spec::Lifecycle::WillEnterForeground);
        m.fire(80.0);
        assert_eq!(seen.borrow()[1], Duration::ZERO);
    }
    #[test]
    fn malformed_and_backward_stamps_do_not_poison_the_clock() {
        let m = Manual::new();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let s = seen.clone();
        let _handle = m.clock().subscribe(move |f| {
            s.borrow_mut().push(f.delta);
            ControlFlow::Continue(())
        });
        m.fire(f64::NAN);
        m.fire(5.0);
        m.fire(4.0);
        m.fire(5.25);
        assert_eq!(
            *seen.borrow(),
            vec![Duration::ZERO, Duration::ZERO, Duration::from_millis(250)]
        );
    }

    #[test]
    fn first_subscription_created_in_background_starts_on_foreground() {
        let m = Manual::new();
        lifecycle(day_spec::Lifecycle::DidEnterBackground);
        let seen = Rc::new(Cell::new(false));
        let s = seen.clone();
        let _handle = m.clock().request(move |frame| {
            assert_eq!(frame.delta, Duration::ZERO);
            s.set(true);
        });
        assert_eq!(m.requests.get(), 0);
        lifecycle(day_spec::Lifecycle::WillEnterForeground);
        assert_eq!(m.requests.get(), 1);
        m.fire(20.0);
        assert!(seen.get());
    }
    #[test]
    fn cancelled_native_tickets_can_never_run_a_new_callback() {
        let calls = Rc::new(Cell::new(0));
        let first = native::register(Box::new(|_| panic!("stale native ticket")));
        native::cancel(first);
        let c = calls.clone();
        let second = native::register(Box::new(move |_| c.set(c.get() + 1)));
        native::deliver(first, FrameStamp::new(1.0));
        native::deliver(second, FrameStamp::new(1.0));
        native::deliver(second, FrameStamp::new(2.0));
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn dropping_callbacks_can_cancel_other_handles() {
        let m = Manual::new();
        let inner = m.clock().request(|_| panic!("cancelled inner callback"));
        let outer = m.clock().subscribe(move |_| {
            let _ = &inner;
            ControlFlow::Continue(())
        });
        drop(outer);
        assert_eq!(frame_consumer_count(), 0);
        m.fire(1.0);
    }

    #[test]
    fn closing_a_window_drops_nested_handles_outside_the_driver_borrow() {
        let m = Manual::new();
        let inner = m.clock().request(|_| panic!("closed window"));
        let _outer = m.clock().subscribe(move |_| {
            let _ = &inner;
            ControlFlow::Continue(())
        });
        forget_window(m.clock().root);
        assert_eq!(frame_consumer_count(), 0);
        m.fire(1.0);
    }
}
