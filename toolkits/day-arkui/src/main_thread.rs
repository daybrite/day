// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Posting to the JS thread, which is Day's UI thread on this backend (docs/harmonyos.md).
//!
//! The carrier is a NAPI threadsafe function (`napi-ohos`): a Rust job is queued from any
//! thread, and the runtime delivers it on the JS thread's event loop, where the call-back
//! closure runs it. Jobs posted before the host's `start()` has created the function wait in a
//! pending list and go out as soon as it exists, in order.

use std::sync::{Mutex, OnceLock};
use std::thread::ThreadId;

use napi_ohos::bindgen_prelude::{FnArgs, Function, JsObjectValue, Unknown};
use napi_ohos::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_ohos::{Env, Status};

pub type Job = Box<dyn FnOnce() + Send>;

type Poster = ThreadsafeFunction<Job, (), (), Status, false, false, 0>;

static POSTER: OnceLock<Poster> = OnceLock::new();
static PENDING: Mutex<Vec<Job>> = Mutex::new(Vec::new());
static JS_THREAD: OnceLock<ThreadId> = OnceLock::new();

/// Remember the JS thread and create the poster. Called from the host's NAPI entry points,
/// which run on that thread; a repeat call is a no-op.
pub fn init(env: &Env) -> napi_ohos::Result<()> {
    JS_THREAD.get_or_init(|| std::thread::current().id());
    if POSTER.get().is_some() {
        return Ok(());
    }
    // The JS half is a no-op: the work happens in the call-back closure, on the JS thread,
    // before the (empty) call reaches it.
    let sink: Function<'_, (), ()> = env.create_function_from_closure("dayPost", |_| Ok(()))?;
    let poster = sink
        .build_threadsafe_function::<Job>()
        .callee_handled::<false>()
        .build_callback(|ctx| {
            // A posted closure runs app code: contain panics like every FFI entry
            // (day_spec::ffi_guard), since this frame is the runtime's.
            day_spec::ffi_guard::contain((), ctx.value);
            Ok(())
        })?;
    let _ = POSTER.set(poster);
    let pending = std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()));
    for job in pending {
        post(job);
    }
    Ok(())
}

/// Run `job` on the JS thread, after everything posted before it.
pub fn post(job: Job) {
    match POSTER.get() {
        Some(p) => {
            p.call(job, ThreadsafeFunctionCallMode::NonBlocking);
        }
        None => PENDING.lock().unwrap_or_else(|e| e.into_inner()).push(job),
    }
}

day_reactive::tls_slots! {
    main_thread;
    /// Jobs waiting on a `setTimeout`, by ticket: the JS callback carries the ticket only.
    static DELAYED: std::cell::RefCell<std::collections::HashMap<u64, Job>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    static NEXT_DELAYED: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

/// Run `job` on the JS thread after (at least) `ms` milliseconds: the timer door behind
/// `day::sleep` (docs/async.md). The timer is the host runtime's own `setTimeout`, armed on the
/// JS thread (a hop through [`post`] from anywhere else), so a sleep costs no helper thread.
/// When the runtime offers no timer (no environment yet), the job runs on the next post.
pub fn post_delayed(ms: u32, job: Job) {
    // A JS timer delay is a signed 32-bit count; a longer one runs as a chain of timers.
    const STEP: u32 = i32::MAX as u32;
    if ms > STEP {
        post_delayed(STEP, Box::new(move || post_delayed(ms - STEP, job)));
        return;
    }
    post(Box::new(move || {
        let ticket = NEXT_DELAYED.with(|n| {
            let t = n.get();
            n.set(t.wrapping_add(1).max(1));
            t
        });
        DELAYED.with(|d| d.borrow_mut().insert(ticket, job));
        if arm_timeout(ticket, ms).is_none()
            && let Some(job) = DELAYED.with(|d| d.borrow_mut().remove(&ticket))
        {
            log::warn!("day-arkui: no setTimeout for a delayed post; running it now");
            day_spec::ffi_guard::contain((), job);
        }
    }));
}

/// `globalThis.setTimeout(handler, ms)`, as napi types it.
type SetTimeout<'a> = Function<'a, FnArgs<(Function<'a, (), ()>, f64)>, Unknown<'a>>;

/// `globalThis.setTimeout(fire(ticket), ms)` on the JS thread; `None` when any step fails.
fn arm_timeout(ticket: u64, ms: u32) -> Option<()> {
    let env = crate::host_api::env()?;
    let fire = env
        .create_function_from_closure("dayDelayed", move |_| {
            if let Some(job) = DELAYED.with(|d| d.borrow_mut().remove(&ticket)) {
                // Timer callbacks run app code inside the runtime's frame: contained, like a post.
                day_spec::ffi_guard::contain((), job);
            }
            Ok(())
        })
        .ok()?;
    let set_timeout: SetTimeout<'_> = env
        .get_global()
        .ok()?
        .get_named_property_unchecked("setTimeout")
        .ok()?;
    set_timeout
        .call(FnArgs {
            data: (fire, f64::from(ms)),
        })
        .ok()?;
    Some(())
}

/// A job for the JS thread from the JS thread: the closure never changes threads, so it need
/// not be `Send` (an `Event` payload, say). Callers outside the JS thread use [`post`].
struct SameThread(Box<dyn FnOnce()>);
// SAFETY: only ever created and run on the JS thread (see `post_local`).
unsafe impl Send for SameThread {}
impl SameThread {
    // A method, so the posted closure captures the wrapper whole rather than its (non-Send)
    // field.
    fn run(self) {
        (self.0)()
    }
}

/// Run `job` on the JS thread, later in this loop turn's order; JS thread only.
pub fn post_local(job: Box<dyn FnOnce()>) {
    debug_assert!(on_js_thread() || JS_THREAD.get().is_none());
    let job = SameThread(job);
    post(Box::new(move || job.run()));
}

/// Whether the caller is the JS thread, where an ArkTS answer cannot be awaited.
pub fn on_js_thread() -> bool {
    JS_THREAD.get() == Some(&std::thread::current().id())
}

/// Whether the poster exists yet (a cross-thread call that needs an answer can't wait on a
/// job that has nowhere to run).
pub fn ready() -> bool {
    POSTER.get().is_some()
}
