// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-async — the std-only async support parts (docs/async.md).
//!
//! Day runs futures on its own main-loop executor (`day::task`) and never brings in an async
//! runtime. What a part needs beside that executor is small and was, until this crate, written
//! once per part: a **oneshot** future that a completion arriving on any thread resolves, and a
//! **token registry** that hands a platform a plain number and looks the waiting closure up when
//! the number comes back. Both live here, with the locking rules the copies converged on:
//! deliver outside the lock, and treat an unknown token as a no-op.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

// ---------------------------------------------------------------------------
// Oneshot
// ---------------------------------------------------------------------------

struct Slot<T> {
    value: Option<T>,
    waker: Option<Waker>,
    /// The sender is gone (delivered or dropped); with `value` empty that is [`Dropped`].
    closed: bool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A poisoned slot holds nothing a later poll could misread: the flags below are set last.
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The sending half went away without delivering. A [`Oneshot`] resolves to this rather than
/// pending forever, so a completion that never comes is an error the caller sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dropped;

impl std::fmt::Display for Dropped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the completion was dropped before it delivered")
    }
}

impl std::error::Error for Dropped {}

/// The sending half of a [`oneshot`]. `Send` whenever `T` is, so a platform's completion thread
/// can hold it; delivering wakes the receiver outside the lock.
pub struct Deliver<T> {
    slot: Arc<Mutex<Slot<T>>>,
    sent: bool,
}

impl<T> Deliver<T> {
    /// Resolve the receiver with `value`. A second delivery is impossible by construction: this
    /// consumes the sender.
    pub fn send(mut self, value: T) {
        self.sent = true;
        let waker = {
            let mut s = lock(&self.slot);
            s.value = Some(value);
            s.closed = true;
            s.waker.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
    }
}

impl<T> Drop for Deliver<T> {
    fn drop(&mut self) {
        if self.sent {
            return;
        }
        let waker = {
            let mut s = lock(&self.slot);
            s.closed = true;
            s.waker.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
    }
}

/// The receiving half of a [`oneshot`]: a future resolving to the delivered value, or to
/// [`Dropped`] when the sender went away first. Awaitable from any executor — `day::task`, or a
/// test's park/unpark `block_on`.
pub struct Oneshot<T> {
    slot: Arc<Mutex<Slot<T>>>,
}

impl<T> Future for Oneshot<T> {
    type Output = Result<T, Dropped>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut s = lock(&self.slot);
        if let Some(v) = s.value.take() {
            return Poll::Ready(Ok(v));
        }
        if s.closed {
            return Poll::Ready(Err(Dropped));
        }
        s.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl<T> Oneshot<T> {
    /// Whether a value has arrived (or the sender is gone) — without consuming it.
    pub fn is_ready(&self) -> bool {
        let s = lock(&self.slot);
        s.value.is_some() || s.closed
    }
}

/// A single-value channel between one completion and one future.
pub fn oneshot<T>() -> (Deliver<T>, Oneshot<T>) {
    let slot = Arc::new(Mutex::new(Slot {
        value: None,
        waker: None,
        closed: false,
    }));
    (
        Deliver {
            slot: slot.clone(),
            sent: false,
        },
        Oneshot { slot },
    )
}

// ---------------------------------------------------------------------------
// Token registry
// ---------------------------------------------------------------------------

/// The process-wide token counter, shared by every registry so a token identifies one waiting
/// closure anywhere in the process. Starts at 1: zero is "no token" in every foreign spelling.
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

/// A fresh token, never reused for the life of the process.
pub fn next_token() -> u64 {
    NEXT_TOKEN.fetch_add(1, Ordering::Relaxed)
}

type Callback<T> = Box<dyn FnOnce(T) + Send>;

/// Waiting closures keyed by token — the shape behind every platform completion that crosses
/// an FFI boundary as a number (docs/bridge.md "Callbacks").
///
/// Rules that make it safe to hand a token to code in another language:
///
/// - **Register before the call goes out.** A platform may complete synchronously, or fail
///   before returning; a slot that exists first is found either way.
/// - **An unknown token is a no-op.** A late, duplicate, or cancelled completion finds nothing
///   and does nothing.
/// - **Deliver outside the lock.** The closure may register a new token or drop a handle; the
///   registry is never held while it runs.
///
/// `const`-constructible, so a generated `static` can own one per declaration.
pub struct TokenRegistry<T> {
    slots: Mutex<BTreeMap<u64, Callback<T>>>,
}

impl<T> Default for TokenRegistry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> TokenRegistry<T> {
    /// An empty registry.
    pub const fn new() -> Self {
        Self {
            slots: Mutex::new(BTreeMap::new()),
        }
    }

    /// Store `cb` under a fresh token.
    pub fn insert(&self, cb: impl FnOnce(T) + Send + 'static) -> u64 {
        let token = next_token();
        lock(&self.slots).insert(token, Box::new(cb));
        token
    }

    /// Remove the closure for `token` and run it with `value`. `false` when no closure waits
    /// under that token — completed, cancelled, or never registered.
    pub fn complete(&self, token: u64, value: T) -> bool {
        let cb = lock(&self.slots).remove(&token);
        match cb {
            Some(cb) => {
                cb(value);
                true
            }
            None => false,
        }
    }

    /// Forget `token` without running its closure: the waiting side gave up.
    pub fn remove(&self, token: u64) -> bool {
        lock(&self.slots).remove(&token).is_some()
    }

    /// Whether a closure still waits under `token`.
    pub fn contains(&self, token: u64) -> bool {
        lock(&self.slots).contains_key(&token)
    }

    /// How many closures wait — for tests and diagnostics.
    pub fn len(&self) -> usize {
        lock(&self.slots).len()
    }

    /// Whether nothing waits.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc as StdArc;
    use std::task::Wake;

    /// The ~20-line park/unpark executor every part's future tests use.
    fn block_on<F: Future>(mut fut: F) -> F::Output {
        struct Unpark(std::thread::Thread);
        impl Wake for Unpark {
            fn wake(self: StdArc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(StdArc::new(Unpark(std::thread::current())));
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

    #[test]
    fn delivers_across_a_thread() {
        let (tx, rx) = oneshot::<u32>();
        std::thread::spawn(move || tx.send(7));
        assert_eq!(block_on(rx), Ok(7));
    }

    #[test]
    fn a_dropped_sender_resolves_rather_than_hangs() {
        let (tx, rx) = oneshot::<u32>();
        drop(tx);
        assert_eq!(block_on(rx), Err(Dropped));
    }

    #[test]
    fn a_value_sent_before_the_first_poll_is_ready_at_once() {
        let (tx, rx) = oneshot::<&str>();
        tx.send("now");
        assert!(rx.is_ready());
        assert_eq!(block_on(rx), Ok("now"));
    }

    static REG: TokenRegistry<i32> = TokenRegistry::new();

    #[test]
    fn a_token_completes_exactly_once() {
        let hits = StdArc::new(Mutex::new(Vec::new()));
        let h = hits.clone();
        let token = REG.insert(move |v| h.lock().unwrap().push(v));
        assert!(REG.contains(token));
        assert!(REG.complete(token, 1));
        assert!(!REG.complete(token, 2), "a second completion finds nothing");
        assert_eq!(*hits.lock().unwrap(), vec![1]);
    }

    #[test]
    fn a_removed_token_never_runs() {
        let token = REG.insert(|_| panic!("cancelled closures never run"));
        assert!(REG.remove(token));
        assert!(!REG.complete(token, 0));
    }

    #[test]
    fn a_completion_may_register_again_from_inside_the_closure() {
        // Would deadlock if the registry were held while the closure runs.
        let token = REG.insert(|_| {
            let _ = REG.insert(|_| {});
        });
        assert!(REG.complete(token, 0));
    }

    #[test]
    fn tokens_are_process_unique() {
        let a = next_token();
        let b = next_token();
        assert!(b > a);
        assert_ne!(a, 0);
    }
}
