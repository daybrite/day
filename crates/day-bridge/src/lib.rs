// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! daybridge runtime (docs/bridge.md, DESIGN.md §15.6) — foreign-language implementations of a
//! Rust API.
//!
//! A crate declares one API and supplies implementations per platform; `day build` generates the
//! adapters and the glue. This crate is the small runtime half: the [`bridge!`] macro, the
//! [`Error`] that crosses every boundary, the [`Support`] an arm reports, and the callback tier —
//! [`Done`], [`Registry`] and [`Completion`] — that lets an arm answer after it has returned.
//!
//! ```ignore
//! day_bridge::bridge! {
//!     #[day_bridge::declare]
//!     extern "day" {
//!         fn speak_native(text: &str, done: day_bridge::Done<bool>) -> Result<(), day_bridge::Error>;
//!     }
//!
//!     #[day_bridge::impl(rust, platforms = [other])]
//!     fn speak_native(_text: &str, done: day_bridge::Done<bool>) -> Result<(), day_bridge::Error> {
//!         done.complete(Err(day_bridge::Error::Unsupported));
//!         Ok(())
//!     }
//! }
//! ```
//!
//! The generator is [`day_build::bridge`](../day_build/bridge/index.html), called from the crate's
//! `build.rs`. Nothing here parses anything: see [`bridge!`] for why.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// What a target's arm promises, re-exported from day-spec so a bridged crate's `available()`
/// answers in the same vocabulary as `day::capability()`.
pub use day_spec::Support;

/// The single error type crossing a bridge boundary (docs/bridge.md "Errors").
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// No arm claims this target — what the `other` fallback returns.
    Unsupported,
    /// The arm failed: a Swift `throws`, a Kotlin exception, a thrown JS error, a nonzero C status.
    /// The string is the platform's own message, which is the only detail that survives.
    Foreign(String),
    /// An argument or result was not valid UTF-8.
    Encoding,
    /// The platform runtime was unavailable — no JVM, no `Context`, COM init refused — or a
    /// completion never came because the call never went out.
    Runtime,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported => write!(f, "unsupported on this platform"),
            Error::Foreign(msg) => write!(f, "{msg}"),
            Error::Encoding => write!(f, "invalid UTF-8 across the bridge"),
            Error::Runtime => write!(f, "platform runtime unavailable"),
        }
    }
}

impl std::error::Error for Error {}

// ---------------------------------------------------------------------------
// The callback tier (docs/bridge.md "Callbacks")
// ---------------------------------------------------------------------------

/// The closures waiting on one declared function's completions, keyed by token. The generator
/// emits one `static` per `Done` declaration; a foreign arm receives the token and the generated
/// completion export resolves it here.
pub struct Registry<T: 'static> {
    inner: day_async::TokenRegistry<Result<T, Error>>,
}

impl<T: Send + 'static> Default for Registry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + 'static> Registry<T> {
    /// An empty registry — `const`, so a generated `static` can own it.
    pub const fn new() -> Self {
        Self {
            inner: day_async::TokenRegistry::new(),
        }
    }

    /// Resolve `token` with `value`. `false` when nothing waits under it: already completed,
    /// cancelled, or never issued — every one of which is a no-op by contract.
    pub fn complete(&self, token: u64, value: Result<T, Error>) -> bool {
        self.inner.complete(token, value)
    }

    /// Whether a closure still waits under `token`.
    pub fn is_pending(&self, token: u64) -> bool {
        self.inner.contains(token)
    }

    /// Forget `token` without resolving it — the future that waited was dropped.
    fn cancel(&self, token: u64) -> bool {
        self.inner.remove(token)
    }
}

/// One outstanding completion, handed to an arm. A Rust arm calls [`Done::complete`]; a foreign
/// arm receives [`Done::token`] and completes through the generated symbol.
///
/// A `Done` fires **at most once**: completing consumes it, and a completion arriving after the
/// token was cancelled or already answered finds nothing and does nothing.
pub struct Done<T: Send + 'static> {
    token: u64,
    registry: &'static Registry<T>,
}

impl<T: Send + 'static> Done<T> {
    /// Register `cb` and produce the handle the arm receives.
    pub fn new(
        registry: &'static Registry<T>,
        cb: impl FnOnce(Result<T, Error>) + Send + 'static,
    ) -> Self {
        Self {
            token: registry.inner.insert(cb),
            registry,
        }
    }

    /// The number a foreign arm carries and hands back to the completion symbol.
    pub fn token(&self) -> u64 {
        self.token
    }

    /// Hand the token to a foreign arm: the closure stays registered until the generated
    /// completion symbol resolves it, or the waiting future is dropped.
    pub fn into_token(self) -> u64 {
        self.token
    }

    /// Resolve the waiting closure. A Rust arm's way of answering.
    pub fn complete(self, value: Result<T, Error>) {
        self.registry.complete(self.token, value);
    }
}

/// The awaitable form of a `Done` declaration: what a generated `<fn>_future` returns.
///
/// Resolves with the arm's answer, or with [`Error::Runtime`] when the arm returned without
/// ever completing (the call never went out). Dropping it cancels the wait — a late completion
/// then finds nothing — and runs the cancel hook the generated code attached, when the
/// declaration named a cancel arm.
pub struct Completion<T: Send + 'static> {
    rx: day_async::Oneshot<Result<T, Error>>,
    token: u64,
    registry: &'static Registry<T>,
    on_drop: Option<Box<dyn FnOnce() + Send>>,
}

impl<T: Send + 'static> Completion<T> {
    /// Run `f` when this future is dropped before completing — the generated cancel arm.
    pub fn on_cancel(mut self, f: impl FnOnce() + Send + 'static) -> Self {
        self.on_drop = Some(Box::new(f));
        self
    }

    /// Whether the answer has arrived.
    pub fn is_ready(&self) -> bool {
        self.rx.is_ready()
    }

    /// The token the arm was handed — what a declared cancel arm takes.
    pub fn token(&self) -> u64 {
        self.token
    }
}

impl<T: Send + 'static> Future for Completion<T> {
    type Output = Result<T, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.rx).poll(cx) {
            Poll::Ready(Ok(v)) => Poll::Ready(v),
            Poll::Ready(Err(day_async::Dropped)) => Poll::Ready(Err(Error::Runtime)),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<T: Send + 'static> Drop for Completion<T> {
    fn drop(&mut self) {
        if self.registry.cancel(self.token)
            && let Some(f) = self.on_drop.take()
        {
            f();
        }
    }
}

/// Start a `Done` call in callback form — what a generated `<fn>_async` does.
///
/// `call` receives the handle and runs the arm. The callback fires exactly once: with the arm's
/// answer when it completes, or with the arm's error when it fails to start — the same error
/// this returns — so a caller may rely on either channel alone. `Ok` carries the token, which
/// is what a declared cancel arm takes to find the request it started.
pub fn start_async<T: Send + 'static>(
    registry: &'static Registry<T>,
    cb: impl FnOnce(Result<T, Error>) + Send + 'static,
    call: impl FnOnce(Done<T>) -> Result<(), Error>,
) -> Result<u64, Error> {
    let done = Done::new(registry, cb);
    let token = done.token();
    match call(done) {
        Ok(()) => Ok(token),
        Err(e) => {
            registry.complete(token, Err(e.clone()));
            Err(e)
        }
    }
}

/// Start a `Done` call in future form — what a generated `<fn>_future` does.
pub fn start_future<T: Send + 'static>(
    registry: &'static Registry<T>,
    call: impl FnOnce(Done<T>) -> Result<(), Error>,
) -> Completion<T> {
    let (tx, rx) = day_async::oneshot();
    let done = Done::new(registry, move |v| tx.send(v));
    let token = done.token();
    if let Err(e) = call(done) {
        // Registered before the call, so the slot is still there unless the arm completed it
        // on its way out — in which case this finds nothing, and the arm's answer stands.
        registry.complete(token, Err(e));
    }
    Completion {
        rx,
        token,
        registry,
        on_drop: None,
    }
}

// ---------------------------------------------------------------------------
// The stream tier (docs/bridge.md "Streams")
// ---------------------------------------------------------------------------

/// One delivery on an `Emit<T>` stream: a value, the end, or a failure. The last two are
/// terminal: nothing is delivered after either.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item<T> {
    Value(T),
    End,
    Failed(Error),
}

type StreamCallback<T> = Box<dyn FnMut(Item<T>) + Send>;

struct SlotState<T> {
    queue: std::collections::VecDeque<Item<T>>,
    /// A thread is draining `queue` into the callback; others only enqueue.
    delivering: bool,
    /// A terminal item was enqueued, or the consumer stopped: later pushes are refused.
    accepting: bool,
    /// The consumer stopped: queued items are dropped undelivered.
    stopped: bool,
}

struct StreamSlot<T> {
    state: std::sync::Mutex<SlotState<T>>,
    callback: std::sync::Mutex<Option<StreamCallback<T>>>,
}

fn lock_ignoring_poison<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The consumers of one `Emit<T>` declaration's streams, keyed by token — the stream
/// counterpart of [`Registry`]. The generator emits one `static` per declaration.
///
/// Delivery rules that make a token safe to hand to a platform thread:
///
/// - **In order, one at a time.** Items pushed from several threads (a response callback and
///   a writer thread, say) reach the consumer serially, in push order. The first pusher drains
///   the queue; the others only enqueue.
/// - **Re-entrancy is allowed.** The consumer may stop its own stream, or cause another push to
///   it, from inside the callback: neither waits on the callback's lock.
/// - **Terminal once.** After `End` or `Failed` is pushed, further pushes return `false`;
///   after [`Streams::stop`], queued items are dropped.
pub struct Streams<T: 'static> {
    slots: std::sync::Mutex<std::collections::BTreeMap<u64, std::sync::Arc<StreamSlot<T>>>>,
}

impl<T: Send + 'static> Default for Streams<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + 'static> Streams<T> {
    /// An empty set of streams — `const`, so a generated `static` can own it.
    pub const fn new() -> Self {
        Self {
            slots: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    fn insert(&self, cb: impl FnMut(Item<T>) + Send + 'static) -> u64 {
        let token = day_async::next_token();
        let slot = std::sync::Arc::new(StreamSlot {
            state: std::sync::Mutex::new(SlotState {
                queue: std::collections::VecDeque::new(),
                delivering: false,
                accepting: true,
                stopped: false,
            }),
            callback: std::sync::Mutex::new(Some(Box::new(cb))),
        });
        lock_ignoring_poison(&self.slots).insert(token, slot);
        token
    }

    /// Deliver what a generated export received: status 0 is a value (or a failure, when the
    /// value did not convert), 2 the end, anything else a failure.
    pub fn deliver(&self, token: u64, status: i32, outcome: Result<T, Error>) -> bool {
        let item = match (status, outcome) {
            (0, Ok(v)) => Item::Value(v),
            (2, _) => Item::End,
            (_, Err(e)) => Item::Failed(e),
            (_, Ok(_)) => Item::Failed(Error::Runtime),
        };
        self.push(token, item)
    }

    /// Push a value. `false` when the stream has ended, failed, or been stopped.
    pub fn emit(&self, token: u64, value: T) -> bool {
        self.push(token, Item::Value(value))
    }

    /// End the stream.
    pub fn end(&self, token: u64) -> bool {
        self.push(token, Item::End)
    }

    /// Fail the stream.
    pub fn fail(&self, token: u64, error: Error) -> bool {
        self.push(token, Item::Failed(error))
    }

    /// Whether a consumer still listens under `token`.
    pub fn is_open(&self, token: u64) -> bool {
        lock_ignoring_poison(&self.slots).contains_key(&token)
    }

    /// The consumer stops listening: queued items are dropped, later pushes refused, and the
    /// callback released as soon as no delivery is running.
    pub fn stop(&self, token: u64) -> bool {
        let Some(slot) = lock_ignoring_poison(&self.slots).remove(&token) else {
            return false;
        };
        {
            let mut st = lock_ignoring_poison(&slot.state);
            st.stopped = true;
            st.accepting = false;
            st.queue.clear();
        }
        // From inside the callback this lock is held by the same thread: `try_lock` then fails
        // and the draining loop releases the callback when it returns.
        if let Ok(mut cb) = slot.callback.try_lock() {
            *cb = None;
        }
        true
    }

    fn push(&self, token: u64, item: Item<T>) -> bool {
        let terminal = !matches!(item, Item::Value(_));
        let slot = {
            let mut map = lock_ignoring_poison(&self.slots);
            if terminal {
                map.remove(&token)
            } else {
                map.get(&token).cloned()
            }
        };
        let Some(slot) = slot else {
            return false;
        };
        {
            let mut st = lock_ignoring_poison(&slot.state);
            if !st.accepting {
                return false;
            }
            st.queue.push_back(item);
            if terminal {
                st.accepting = false;
            }
            if st.delivering {
                return true;
            }
            st.delivering = true;
        }
        loop {
            let next = {
                let mut st = lock_ignoring_poison(&slot.state);
                if st.stopped {
                    st.queue.clear();
                }
                match st.queue.pop_front() {
                    Some(item) => item,
                    None => {
                        st.delivering = false;
                        break;
                    }
                }
            };
            let ends = !matches!(next, Item::Value(_));
            let mut cb = lock_ignoring_poison(&slot.callback);
            if let Some(f) = cb.as_mut() {
                f(next);
            }
            if ends || lock_ignoring_poison(&slot.state).stopped {
                *cb = None;
            }
        }
        true
    }
}

/// One open stream, handed to an arm. A Rust arm calls [`Emit::emit`] and then [`Emit::end`] or
/// [`Emit::fail`]; a foreign arm receives [`Emit::token`] and delivers through the generated
/// `<fn>_emit` / `<fn>_end` / `<fn>_fail` helpers.
pub struct Emit<T: Send + 'static> {
    token: u64,
    streams: &'static Streams<T>,
}

impl<T: Send + 'static> Clone for Emit<T> {
    fn clone(&self) -> Self {
        Self {
            token: self.token,
            streams: self.streams,
        }
    }
}

impl<T: Send + 'static> Emit<T> {
    /// Register `cb` and produce the handle the arm receives.
    pub fn new(streams: &'static Streams<T>, cb: impl FnMut(Item<T>) + Send + 'static) -> Self {
        Self {
            token: streams.insert(cb),
            streams,
        }
    }

    /// The number a foreign arm carries and hands back to the generated helpers.
    pub fn token(&self) -> u64 {
        self.token
    }

    /// Hand the token to a foreign arm; the consumer stays registered until the stream ends,
    /// fails, or is stopped.
    pub fn into_token(self) -> u64 {
        self.token
    }

    /// Deliver a value. `false` once the stream is over.
    pub fn emit(&self, value: T) -> bool {
        self.streams.emit(self.token, value)
    }

    /// End the stream.
    pub fn end(&self) -> bool {
        self.streams.end(self.token)
    }

    /// Fail the stream.
    pub fn fail(&self, error: Error) -> bool {
        self.streams.fail(self.token, error)
    }

    /// Whether the consumer still listens.
    pub fn is_open(&self) -> bool {
        self.streams.is_open(self.token)
    }
}

/// Start an `Emit` call — what a generated `<fn>_stream` does. `cb` receives every item, in
/// order, from whichever thread the platform delivers on. When the arm fails to start, `cb`
/// receives that failure (once) and this returns it. `Ok` carries the token a stop call takes.
pub fn start_stream<T: Send + 'static>(
    streams: &'static Streams<T>,
    cb: impl FnMut(Item<T>) + Send + 'static,
    call: impl FnOnce(Emit<T>) -> Result<(), Error>,
) -> Result<u64, Error> {
    let emit = Emit::new(streams, cb);
    let token = emit.token();
    match call(emit) {
        Ok(()) => Ok(token),
        Err(e) => {
            streams.fail(token, e.clone());
            Err(e)
        }
    }
}

/// Run a generated completion export's body with panics contained: the export is called from
/// C, the JVM, or the browser, and a panic unwinding into any of them is fatal.
pub fn guard(f: impl FnOnce()) {
    day_spec::ffi_guard::contain((), f);
}

/// Take ownership of a buffer the web shim allocated with `day_dom_alloc(len)` and filled — the
/// (ptr, len) a JavaScript completion passes a string or bytes through (docs/web.md).
///
/// # Safety
///
/// `ptr` must come from a `Vec::<u8>::with_capacity(len)` the shim filled with `len` bytes and
/// handed over; this is the one reconstitution, and the caller must never touch it again.
#[doc(hidden)]
pub unsafe fn __take_wasm(ptr: *mut u8, len: usize) -> Vec<u8> {
    if ptr.is_null() || len == 0 {
        return Vec::new();
    }
    // SAFETY: the caller's contract above.
    unsafe { Vec::from_raw_parts(ptr, len, len) }
}

/// The HarmonyOS side of the callback tier: how generated Rust reaches an ArkTS arm, and how a
/// completion comes back (docs/bridge.md "Callbacks"). Every ArkTS arm runs on the JS thread; the
/// ArkUI shim owns that dispatch, and this module finds the shim's entry at run time so a bridged
/// crate keeps no link-time dependency on the toolkit — the same `dlsym` idiom
/// day-part-permissions uses.
#[cfg(all(target_os = "linux", target_env = "ohos"))]
pub mod arkts {
    use std::ffi::{c_char, c_int, c_void};

    /// One argument crossing to ArkTS: `kind` selects the field (0 bool, 1 i32, 2 i64,
    /// 3 f64, 4 str, 5 bytes), matching `DayArkArg` in the ArkUI shim.
    #[repr(C)]
    pub struct Arg {
        pub kind: i32,
        pub i: i64,
        pub f: f64,
        pub ptr: *const u8,
        pub len: usize,
    }

    impl Arg {
        fn plain(kind: i32, i: i64, f: f64) -> Self {
            Self {
                kind,
                i,
                f,
                ptr: std::ptr::null(),
                len: 0,
            }
        }
        pub fn bool(v: bool) -> Self {
            Self::plain(0, i64::from(v), 0.0)
        }
        pub fn i32(v: i32) -> Self {
            Self::plain(1, i64::from(v), 0.0)
        }
        pub fn i64(v: i64) -> Self {
            Self::plain(2, v, 0.0)
        }
        pub fn f64(v: f64) -> Self {
            Self::plain(3, 0, v)
        }
        pub fn str(v: &str) -> Self {
            Self {
                kind: 4,
                i: 0,
                f: 0.0,
                ptr: v.as_ptr(),
                len: v.len(),
            }
        }
        pub fn bytes(v: &[u8]) -> Self {
            Self {
                kind: 5,
                i: 0,
                f: 0.0,
                ptr: v.as_ptr(),
                len: v.len(),
            }
        }
    }

    type InvokeFn = unsafe extern "C" fn(*const c_char, *const Arg, usize, u64, *mut Arg) -> c_int;
    type OnJsThreadFn = unsafe extern "C" fn() -> c_int;

    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    fn lookup(name: &std::ffi::CStr) -> *mut c_void {
        // SAFETY: a plain symbol lookup in the running process, exactly as
        // day-part-permissions resolves the shim's prompter.
        unsafe { dlsym(std::ptr::null_mut(), name.as_ptr()) }
    }

    /// Call the ArkTS function registered under `symbol` with `args`, on the JS thread. `done`
    /// is the completion token (0 for a synchronous function): the shim passes it as the
    /// trailing argument, and completes it itself when the arm returns a promise. A call from
    /// the JS thread runs inline; from any other thread it is posted and the caller parks until
    /// the loop has run it, so `Ok` still means the arm accepted the call.
    pub fn invoke(symbol: &str, args: &[Arg], done: u64) -> Result<(), super::Error> {
        invoke_value(symbol, args, done).map(|_| ())
    }

    /// [`invoke`], keeping the arm's return value: a scalar the shim marshals into an [`Arg`]
    /// (`i` for booleans and integers, `f` for numbers, both set for a JS number). What a
    /// declaration returning `Result<T, Error>` reads its `T` from.
    pub fn invoke_value(symbol: &str, args: &[Arg], done: u64) -> Result<Arg, super::Error> {
        let entry = lookup(c"day_arkui_bridge_invoke");
        if entry.is_null() {
            return Err(super::Error::Runtime);
        }
        let Ok(name) = std::ffi::CString::new(symbol) else {
            return Err(super::Error::Encoding);
        };
        let mut ret = Arg::plain(-1, 0, 0.0);
        // SAFETY: the symbol is day-arkui's export with exactly this signature.
        let invoke: InvokeFn = unsafe { std::mem::transmute(entry) };
        match unsafe { invoke(name.as_ptr(), args.as_ptr(), args.len(), done, &mut ret) } {
            0 => Ok(ret),
            2 => Err(super::Error::Runtime),
            _ => Err(super::Error::Foreign(format!("{symbol} failed"))),
        }
    }

    /// Whether the caller is on the JS thread — the UI thread of a Day app on HarmonyOS, where
    /// a blocking wait for an ArkTS answer would deadlock. `false` when no host is running.
    pub fn on_js_thread() -> bool {
        let entry = lookup(c"day_arkui_bridge_on_js_thread");
        if entry.is_null() {
            return false;
        }
        // SAFETY: the symbol is day-arkui's export with exactly this signature.
        let f: OnJsThreadFn = unsafe { std::mem::transmute(entry) };
        unsafe { f() != 0 }
    }

    /// Copy a buffer the shim passes to a completion export; it is valid only during the call.
    ///
    /// # Safety
    ///
    /// `ptr` must point at `len` readable bytes, or be null.
    #[doc(hidden)]
    pub unsafe fn __bytes(ptr: *const u8, len: usize) -> Vec<u8> {
        if ptr.is_null() || len == 0 {
            return Vec::new();
        }
        // SAFETY: the caller's contract above.
        unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
    }
}

/// Declare a crate's bridge: the API and its per-platform implementations.
///
/// **The body is discarded.** This macro expands to nothing but an `include!` of the code
/// day-build generated from the same source text, which is what lets an arm contain Swift, Kotlin,
/// ArkTS, JavaScript, C, or C++ — the tokens are never resolved by rustc, only lexed. It is also
/// why daybridge needs no procedural macro, and why DESIGN.md §5.1's "no required macro anywhere in
/// the framework" still holds: this is opt-in sugar that lowers to plain generated Rust.
///
/// Foreign code inside an arm must nevertheless *lex* as Rust tokens, which idiomatic JavaScript and
/// ArkTS do not — a backtick is not a Rust token, and `'zh-CN'` lexes as a malformed lifetime. That
/// is why inline arms carry their body in a raw string.
#[macro_export]
macro_rules! bridge {
    ($($body:tt)*) => {
        include!(concat!(env!("OUT_DIR"), "/day-bridge/mod.rs"));
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::task::{Wake, Waker};

    fn block_on<F: Future>(mut fut: F) -> F::Output {
        struct Unpark(std::thread::Thread);
        impl Wake for Unpark {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        // SAFETY: `fut` lives on this stack frame and is never moved after being pinned.
        let mut fut = unsafe { Pin::new_unchecked(&mut fut) };
        loop {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
            std::thread::park();
        }
    }

    static REG: Registry<bool> = Registry::new();

    #[test]
    fn a_rust_arm_completes_and_the_future_resolves() {
        let fut = start_future(&REG, |done| {
            std::thread::spawn(move || done.complete(Ok(true)));
            Ok(())
        });
        assert_eq!(block_on(fut), Ok(true));
    }

    #[test]
    fn a_foreign_style_token_round_trip() {
        let fut = start_future(&REG, |done| {
            let token = done.into_token();
            assert!(REG.is_pending(token));
            std::thread::spawn(move || {
                assert!(REG.complete(token, Ok(false)));
                assert!(!REG.complete(token, Ok(true)), "at most once");
            });
            Ok(())
        });
        assert_eq!(block_on(fut), Ok(false));
    }

    #[test]
    fn an_arm_that_fails_to_start_fails_the_future_with_its_own_error() {
        let fut = start_future(&REG, |_done| Err(Error::Foreign("no engine".into())));
        assert_eq!(block_on(fut), Err(Error::Foreign("no engine".into())));
    }

    #[test]
    fn an_arm_that_returns_ok_without_completing_leaves_the_future_pending() {
        let fut = start_future(&REG, |done| {
            let _ = done.into_token();
            Ok(())
        });
        assert!(!fut.is_ready());
    }

    #[test]
    fn dropping_the_future_cancels_the_slot_and_runs_the_hook() {
        let hook = Arc::new(Mutex::new(false));
        let h = hook.clone();
        let mut token = 0;
        let fut = start_future(&REG, |done| {
            token = done.into_token();
            Ok(())
        })
        .on_cancel(move || *h.lock().unwrap() = true);
        assert!(REG.is_pending(token));
        drop(fut);
        assert!(!REG.is_pending(token));
        assert!(*hook.lock().unwrap());
        assert!(
            !REG.complete(token, Ok(true)),
            "a late completion is a no-op"
        );
    }

    #[test]
    fn the_callback_form_fires_exactly_once_on_start_failure() {
        let hits = Arc::new(Mutex::new(0));
        let h = hits.clone();
        let r = start_async(
            &REG,
            move |v| {
                assert_eq!(v, Err(Error::Runtime));
                *h.lock().unwrap() += 1;
            },
            |_done| Err(Error::Runtime),
        );
        assert_eq!(r, Err(Error::Runtime));
        assert_eq!(*hits.lock().unwrap(), 1);
        let token = start_async(&REG, |_| {}, |_done| Ok(())).expect("started");
        assert!(REG.is_pending(token), "the token names the live slot");
    }

    static STREAMS: Streams<u32> = Streams::new();

    type Seen = Arc<Mutex<Vec<Item<u32>>>>;

    fn collector() -> (Seen, impl FnMut(Item<u32>) + Send + 'static) {
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        (seen, move |item| s.lock().unwrap().push(item))
    }

    #[test]
    fn a_stream_delivers_in_order_then_ends_once() {
        let (seen, cb) = collector();
        let token = start_stream(&STREAMS, cb, |emit| {
            let token = emit.into_token();
            std::thread::spawn(move || {
                for v in 0..100 {
                    assert!(STREAMS.emit(token, v));
                }
                assert!(STREAMS.end(token));
                assert!(!STREAMS.emit(token, 999), "nothing after the end");
                assert!(!STREAMS.end(token), "the end is delivered once");
            })
            .join()
            .unwrap();
            Ok(())
        })
        .expect("started");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 101);
        assert!(
            seen[..100]
                .iter()
                .enumerate()
                .all(|(i, v)| *v == Item::Value(i as u32))
        );
        assert_eq!(seen[100], Item::End);
        assert!(!STREAMS.is_open(token));
    }

    #[test]
    fn pushes_from_many_threads_reach_the_consumer_serially() {
        let busy = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (b, c) = (busy.clone(), count.clone());
        let emit = Emit::new(&STREAMS, move |_item| {
            assert!(
                !b.swap(true, std::sync::atomic::Ordering::SeqCst),
                "two deliveries overlapped"
            );
            std::thread::yield_now();
            c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            b.store(false, std::sync::atomic::Ordering::SeqCst);
        });
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let e = emit.clone();
                std::thread::spawn(move || {
                    for v in 0..200 {
                        e.emit(v);
                    }
                })
            })
            .collect();
        for w in workers {
            w.join().unwrap();
        }
        emit.end();
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 8 * 200 + 1);
    }

    #[test]
    fn the_consumer_may_stop_its_own_stream_from_inside_the_callback() {
        let token_cell = Arc::new(Mutex::new(0u64));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (t, s) = (token_cell.clone(), seen.clone());
        let emit = Emit::new(&STREAMS, move |item| {
            s.lock().unwrap().push(item.clone());
            if item == Item::Value(2) {
                assert!(STREAMS.stop(*t.lock().unwrap()));
            }
        });
        *token_cell.lock().unwrap() = emit.token();
        for v in 0..5 {
            emit.emit(v);
        }
        assert!(!emit.end(), "stopped streams refuse the end");
        assert_eq!(
            *seen.lock().unwrap(),
            vec![Item::Value(0), Item::Value(1), Item::Value(2)]
        );
        assert!(!emit.is_open());
    }

    #[test]
    fn a_stream_that_fails_to_start_delivers_that_failure_once() {
        let (seen, cb) = collector();
        let r = start_stream(&STREAMS, cb, |_emit| Err(Error::Foreign("no radio".into())));
        assert_eq!(r, Err(Error::Foreign("no radio".into())));
        assert_eq!(
            *seen.lock().unwrap(),
            vec![Item::Failed(Error::Foreign("no radio".into()))]
        );
    }

    #[test]
    fn deliver_maps_export_statuses() {
        let (seen, cb) = collector();
        let emit = Emit::new(&STREAMS, cb);
        let token = emit.token();
        assert!(STREAMS.deliver(token, 0, Ok(7)));
        assert!(STREAMS.deliver(token, 0, Err(Error::Encoding)));
        assert!(
            !STREAMS.deliver(token, 2, Err(Error::Runtime)),
            "already failed"
        );
        assert_eq!(
            *seen.lock().unwrap(),
            vec![Item::Value(7), Item::Failed(Error::Encoding)]
        );
    }
}
