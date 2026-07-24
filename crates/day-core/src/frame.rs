//! Frame clock / continuous-animation driver (§8.4, docs/animation.md).
//!
//! Backends deliver a single vsync-aligned callback through [`day_spec::Platform::request_frame`];
//! this driver owns the registry of frame CONSUMERS (game loops, self-driven interpolations) and
//! re-arms the backend each tick while any remain — stopping requests when none do, so an idle app
//! never wakes the display link (battery). Consumers receive the wall-clock delta since the previous
//! frame, clamped so a backgrounded/paused window can't deliver a huge jump.
//!
//! It is main-thread only (like the rest of the runtime): all state lives in a thread-local and the
//! backend guarantees `request_frame`/its callback run on the main thread.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

/// A consumer callback: `Rc<RefCell<…>>` so a tick can snapshot + call each one without holding the
/// registry borrow (consumers may add/remove others while running).
type FrameCb = Rc<RefCell<dyn FnMut(Duration)>>;
type Requester = Box<dyn Fn(Box<dyn FnOnce(f64)>)>;

#[derive(Default)]
struct Driver {
    /// Forwards to `Platform::request_frame`; installed once at launch.
    requester: Option<Requester>,
    consumers: BTreeMap<u64, FrameCb>,
    next_id: u64,
    /// True while a frame is requested but not yet delivered (so we don't double-arm).
    armed: bool,
    /// Timestamp of the previous delivered frame (seconds), for computing the delta.
    last_ts: Option<f64>,
}

thread_local! {
    static DRIVER: RefCell<Driver> = RefCell::new(Driver::default());
}

/// Install the backend's vsync requester (called once at launch by `launch_with`). `f` forwards to
/// [`day_spec::Platform::request_frame`].
pub fn install_frame_requester(f: impl Fn(Box<dyn FnOnce(f64)>) + 'static) {
    DRIVER.with(|d| d.borrow_mut().requester = Some(Box::new(f)));
}

/// Identifies a registered frame consumer; pass it to [`remove_frame_consumer`] to stop it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameConsumer(u64);

/// Register `cb` to run every animation frame with the delta since the previous frame, and start
/// the vsync loop if it wasn't already running. Remove with [`remove_frame_consumer`]; the
/// `frame_clock` piece ties that to its scope so it stops when the piece unmounts.
pub fn add_frame_consumer(cb: impl FnMut(Duration) + 'static) -> FrameConsumer {
    let id = DRIVER.with(|d| {
        let mut d = d.borrow_mut();
        let id = d.next_id;
        d.next_id += 1;
        d.consumers.insert(id, Rc::new(RefCell::new(cb)));
        id
    });
    arm();
    FrameConsumer(id)
}

/// Stop delivering frames to a consumer. When the last one is removed, the loop stops re-arming and
/// the display link goes idle.
pub fn remove_frame_consumer(c: FrameConsumer) {
    DRIVER.with(|d| {
        let mut d = d.borrow_mut();
        d.consumers.remove(&c.0);
        if d.consumers.is_empty() {
            d.last_ts = None; // reset the delta baseline for the next loop that starts
        }
    });
}

/// Live consumer count — for diagnostics/tests.
pub fn frame_consumer_count() -> usize {
    DRIVER.with(|d| d.borrow().consumers.len())
}

/// Request a frame if there is work and one isn't already pending.
fn arm() {
    let ready = DRIVER.with(|d| {
        let mut d = d.borrow_mut();
        if d.armed || d.consumers.is_empty() || d.requester.is_none() {
            return false;
        }
        d.armed = true;
        true
    });
    if ready {
        // Call the requester outside the mutable borrow (it just schedules a vsync callback).
        DRIVER.with(|d| {
            if let Some(req) = d.borrow().requester.as_ref() {
                req(Box::new(tick));
            }
        });
    }
}

/// Deliver one frame: compute the clamped delta, run every consumer under a batch, flush so the
/// canvas re-records this frame, then re-arm while consumers remain.
fn tick(ts: f64) {
    let dt = DRIVER.with(|d| {
        let mut d = d.borrow_mut();
        d.armed = false;
        let raw = match d.last_ts {
            Some(prev) => (ts - prev).max(0.0),
            None => 1.0 / 60.0, // assume 60fps for the very first frame
        };
        d.last_ts = Some(ts);
        raw
    });
    // A paused/backgrounded window can report a huge gap — clamp to one ~100ms step so physics
    // never explodes on resume.
    let dt = Duration::from_secs_f64(dt.clamp(0.0, 0.1));

    let cbs: Vec<FrameCb> = DRIVER.with(|d| d.borrow().consumers.values().cloned().collect());
    day_reactive::batch(|| {
        for cb in &cbs {
            // `try_borrow_mut` guards the (pathological) case of a consumer re-entering itself.
            if let Ok(mut f) = cb.try_borrow_mut() {
                f(dt);
            }
        }
    });
    // Force the batch's reactions (game-state signal writes) to drain now so the canvas replays
    // this frame rather than one frame late.
    day_reactive::flush_now();

    arm();
}
