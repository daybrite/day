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
}
