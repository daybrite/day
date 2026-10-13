// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-async: the std-only async support parts (docs/async.md).
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
/// [`Dropped`] when the sender went away first. Awaitable from any executor: `day::task`, or a
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
    /// Whether a value has arrived (or the sender is gone), without consuming it.
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

/// Waiting closures keyed by token: the shape behind every platform completion that crosses
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
    /// under that token (completed, cancelled, or never registered).
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

    /// How many closures wait, for tests and diagnostics.
    pub fn len(&self) -> usize {
        lock(&self.slots).len()
    }

    /// Whether nothing waits.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---------------------------------------------------------------------------
// Timers
// ---------------------------------------------------------------------------

/// A scheduled timer, for [`unschedule`].
pub type TimerId = u64;

#[cfg(not(target_arch = "wasm32"))]
mod timer_thread {
    use std::cmp::Reverse;
    use std::collections::{BinaryHeap, HashMap};
    use std::sync::{Condvar, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use super::{TimerId, lock};

    type Job = Box<dyn FnOnce() + Send>;

    struct State {
        heap: BinaryHeap<Reverse<(Instant, TimerId)>>,
        jobs: HashMap<TimerId, Job>,
        next: TimerId,
        started: bool,
    }

    struct Timers {
        state: Mutex<State>,
        wake: Condvar,
    }

    fn timers() -> &'static Timers {
        static TIMERS: OnceLock<Timers> = OnceLock::new();
        TIMERS.get_or_init(|| Timers {
            state: Mutex::new(State {
                heap: BinaryHeap::new(),
                jobs: HashMap::new(),
                next: 0,
                started: false,
            }),
            wake: Condvar::new(),
        })
    }

    pub(super) fn schedule(delay: Duration, job: impl FnOnce() + Send + 'static) -> TimerId {
        let t = timers();
        let mut st = lock(&t.state);
        st.next += 1;
        let id = st.next;
        st.heap.push(Reverse((Instant::now() + delay, id)));
        st.jobs.insert(id, Box::new(job));
        if !st.started {
            st.started = std::thread::Builder::new()
                .name("day-timers".into())
                .spawn(run)
                .is_ok();
        }
        t.wake.notify_one();
        id
    }

    pub(super) fn unschedule(id: TimerId) {
        lock(&timers().state).jobs.remove(&id);
    }

    fn run() {
        let t = timers();
        let mut st = lock(&t.state);
        loop {
            let Some(Reverse((at, id))) = st.heap.peek().copied() else {
                st = t.wake.wait(st).unwrap_or_else(|p| p.into_inner());
                continue;
            };
            let now = Instant::now();
            if at > now {
                st = t
                    .wake
                    .wait_timeout(st, at - now)
                    .map(|(g, _)| g)
                    .unwrap_or_else(|p| p.into_inner().0);
                continue;
            }
            st.heap.pop();
            if let Some(job) = st.jobs.remove(&id) {
                drop(st);
                job();
                st = lock(&t.state);
            }
        }
    }
}

/// Run `job` after `delay` on the process's one timer thread, which starts with the first
/// timer. Parts use it for limits and backoff instead of parking a thread per wait. Jobs run one
/// at a time, so a job should hand longer work elsewhere. The browser has no threads: there a
/// timer never fires, and web code keeps its own clock.
pub fn schedule(delay: std::time::Duration, job: impl FnOnce() + Send + 'static) -> TimerId {
    #[cfg(not(target_arch = "wasm32"))]
    {
        timer_thread::schedule(delay, job)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (delay, job);
        0
    }
}

/// Forget a timer that has not fired. A timer that already fired, or an unknown id, is a no-op.
pub fn unschedule(id: TimerId) {
    #[cfg(not(target_arch = "wasm32"))]
    timer_thread::unschedule(id);
    #[cfg(target_arch = "wasm32")]
    let _ = id;
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod timer_tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn timers_fire_in_order_and_unscheduled_ones_never_do() {
        let (tx, rx) = mpsc::channel();
        let start = Instant::now();
        let second = tx.clone();
        let never = tx.clone();
        super::schedule(Duration::from_millis(60), move || second.send(2).unwrap());
        super::schedule(Duration::from_millis(20), move || tx.send(1).unwrap());
        let dropped = super::schedule(Duration::from_millis(40), move || never.send(9).unwrap());
        super::unschedule(dropped);
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(1));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(2));
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        assert!(start.elapsed() >= Duration::from_millis(60));
    }
}

// ---------------------------------------------------------------------------
// Blocking pool
// ---------------------------------------------------------------------------

/// Why a [`Blocking`] future resolved without its job's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockingError {
    /// The job panicked. The panic went to the panic hook like any other; the pool thread that
    /// ran it carries on with the next job.
    Panicked,
    /// The job never ran: the pool could not start its first thread.
    Closed,
}

impl std::fmt::Display for BlockingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            BlockingError::Panicked => "the blocking job panicked",
            BlockingError::Closed => "the blocking job never ran: no pool thread could start",
        })
    }
}

impl std::error::Error for BlockingError {}

/// The future [`spawn_blocking`] returns: the job's value once a pool thread has run it.
///
/// Dropping it before the job starts cancels the job, which then never runs. Dropping it while
/// the job runs lets the job finish on its thread and discards the value: a blocking call has no
/// way to be interrupted. [`Blocking::detach`] keeps the job without the value. Awaitable from
/// any executor, `day::task` included.
#[must_use = "await the future to receive the job's value, or `detach` it"]
pub struct Blocking<T> {
    inner: BlockingInner<T>,
}

enum BlockingInner<T> {
    /// Queued for the pool; the value arrives through the oneshot.
    #[cfg(not(target_arch = "wasm32"))]
    Pooled {
        receiver: Oneshot<Result<T, BlockingError>>,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
        detached: bool,
    },
    /// No thread will run it (the web, or a synchronous test harness): it runs at the first
    /// poll, on the polling thread.
    Inline(Option<Box<dyn FnOnce() -> T>>),
}

impl<T> Future for Blocking<T> {
    type Output = Result<T, BlockingError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: `inner` is never moved out of the pinned future; the oneshot is `Unpin`.
        let inner = unsafe { &mut self.get_unchecked_mut().inner };
        // An inline job needs no waker: it is ready at this poll.
        let _ = &cx;
        match inner {
            #[cfg(not(target_arch = "wasm32"))]
            BlockingInner::Pooled { receiver, .. } => Pin::new(receiver)
                .poll(cx)
                .map(|r| r.unwrap_or(Err(BlockingError::Closed))),
            // `catch_unwind` is a plain call under the web's abort-on-panic; it does its job on
            // the targets that unwind.
            BlockingInner::Inline(job) => Poll::Ready(match job.take() {
                Some(job) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(job))
                    .map_err(|_| BlockingError::Panicked),
                None => Err(BlockingError::Closed),
            }),
        }
    }
}

impl<T> Blocking<T> {
    /// Let the job run to completion without waiting for its value: for work whose effect is
    /// its own completion callback. A pooled job keeps its queue slot; an inline one runs now.
    pub fn detach(mut self) {
        match &mut self.inner {
            #[cfg(not(target_arch = "wasm32"))]
            BlockingInner::Pooled { detached, .. } => *detached = true,
            BlockingInner::Inline(job) => {
                if let Some(job) = job.take() {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<T> Drop for Blocking<T> {
    fn drop(&mut self) {
        if let BlockingInner::Pooled {
            cancelled,
            detached: false,
            ..
        } = &self.inner
        {
            cancelled.store(true, std::sync::atomic::Ordering::Release);
        }
    }
}

/// Run `job` on a pool thread and resolve to its value on the awaiting side.
///
/// The one bridge from an `async` flow to a call that blocks or computes for a while: a file
/// read, a parse, a hash, a database open, a `block_on` into some other runtime. The pool is
/// process-wide, starts with the first job, holds at most [`blocking_limit`] threads, and lets a
/// thread go after a few idle seconds; jobs beyond the limit queue in order. Nothing here knows
/// about Day's executor, so a part can await it under `day::task` or a test's `block_on` alike.
///
/// On the web, where no thread exists, the job runs inline at the future's first poll. Keep
/// web jobs short, or branch on `cfg!(target_arch = "wasm32")` and use the platform's async API.
/// A synchronous test harness (the mock toolkit) asks for the same with
/// [`set_blocking_inline`].
///
/// ```ignore
/// let bytes = day::spawn_blocking(move || std::fs::read(&path)).await;
/// match bytes {
///     Ok(Ok(bytes)) => contents.set(bytes),
///     Ok(Err(io)) => status.set(io.to_string()),
///     Err(e) => status.set(e.to_string()),
/// }
/// ```
pub fn spawn_blocking<T: Send + 'static>(job: impl FnOnce() -> T + Send + 'static) -> Blocking<T> {
    if blocking_inline() {
        return Blocking {
            inner: BlockingInner::Inline(Some(Box::new(job))),
        };
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        let (tx, rx) = oneshot();
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let run = move || {
            // The awaiting side is gone: skip the work. (Dropping `tx` here is unobserved.)
            if flag.load(Ordering::Acquire) {
                return;
            }
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
            tx.send(out.map_err(|_| BlockingError::Panicked));
        };
        // A refused job drops its `tx` with the box, so the receiver resolves to `Closed`.
        blocking_pool::submit(Box::new(run));
        Blocking {
            inner: BlockingInner::Pooled {
                receiver: rx,
                cancelled,
                detached: false,
            },
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        unreachable!("the web always runs blocking jobs inline")
    }
}

static BLOCKING_INLINE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Make every later [`spawn_blocking`] run its job inline at the first poll, as the web does,
/// instead of on a pool thread. For a synchronous test harness whose main-loop posts run on
/// the calling thread (the mock toolkit), where a wake from a pool thread would have no loop
/// to land on. Process-wide; the web ignores it (always inline).
pub fn set_blocking_inline(inline: bool) {
    BLOCKING_INLINE.store(inline, std::sync::atomic::Ordering::Release);
}

/// Whether [`spawn_blocking`] runs jobs inline: always on the web, else per
/// [`set_blocking_inline`].
pub fn blocking_inline() -> bool {
    cfg!(target_arch = "wasm32") || BLOCKING_INLINE.load(std::sync::atomic::Ordering::Acquire)
}

/// How many threads the blocking pool may hold at once: the machine's parallelism, at least
/// four (a job waiting behind three slow ones still gets a thread), at most thirty-two. Zero on
/// the web, where jobs run inline.
pub fn blocking_limit() -> usize {
    #[cfg(not(target_arch = "wasm32"))]
    {
        blocking_pool::limit()
    }
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
}

/// How many pool threads exist right now, idle ones included: for diagnostics and tests.
pub fn blocking_threads() -> usize {
    #[cfg(not(target_arch = "wasm32"))]
    {
        blocking_pool::threads()
    }
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod blocking_pool {
    use std::collections::VecDeque;
    use std::sync::{Condvar, Mutex, OnceLock};
    use std::time::Duration;

    use super::lock;

    type Job = Box<dyn FnOnce() + Send>;

    /// How long a thread with nothing to do waits for a job before it exits.
    const IDLE_FOR: Duration = Duration::from_secs(2);

    struct State {
        queue: VecDeque<Job>,
        /// Threads alive, working or waiting.
        threads: usize,
        /// Threads waiting on `wake`.
        idle: usize,
    }

    struct Pool {
        state: Mutex<State>,
        wake: Condvar,
        limit: usize,
    }

    fn pool() -> &'static Pool {
        static POOL: OnceLock<Pool> = OnceLock::new();
        POOL.get_or_init(|| Pool {
            state: Mutex::new(State {
                queue: VecDeque::new(),
                threads: 0,
                idle: 0,
            }),
            wake: Condvar::new(),
            limit: std::thread::available_parallelism()
                .map_or(4, |n| n.get())
                .clamp(4, 32),
        })
    }

    pub(super) fn limit() -> usize {
        pool().limit
    }

    pub(super) fn threads() -> usize {
        lock(&pool().state).threads
    }

    /// Queue `job`; start a thread when no idle one will take it and the limit allows. A job
    /// that no thread can ever run (the first thread failed to start) is dropped, which drops
    /// its delivery.
    pub(super) fn submit(job: Job) {
        let p = pool();
        let mut st = lock(&p.state);
        st.queue.push_back(job);
        // A wake lost to a race costs latency, not the job: a woken thread drains the queue
        // before it waits again, and the spawn below covers the queue outgrowing the waiters.
        if st.queue.len() > st.idle && st.threads < p.limit {
            match std::thread::Builder::new()
                .name("day-blocking".into())
                .spawn(worker)
            {
                Ok(_) => st.threads += 1,
                Err(_) if st.threads == 0 => {
                    st.queue.pop_back();
                    return;
                }
                // Other threads exist; they will get to it.
                Err(_) => {}
            }
        }
        p.wake.notify_one();
    }

    fn worker() {
        let p = pool();
        let mut st = lock(&p.state);
        loop {
            while let Some(job) = st.queue.pop_front() {
                drop(st);
                job();
                st = lock(&p.state);
            }
            st.idle += 1;
            let (guard, waited) = p
                .wake
                .wait_timeout(st, IDLE_FOR)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            st = guard;
            st.idle -= 1;
            if waited.timed_out() && st.queue.is_empty() {
                st.threads -= 1;
                return;
            }
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod blocking_tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    use super::tests::block_on;
    use super::{BlockingError, blocking_limit, blocking_threads, spawn_blocking};

    /// Inline mode is process-wide and the pool is shared, so these tests take turns: one
    /// toggling inline mode under another's thread assertion would fail it.
    fn turn() -> std::sync::MutexGuard<'static, ()> {
        static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
        TURN.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn a_job_runs_on_another_thread_and_delivers_its_value() {
        let _turn = turn();
        let here = std::thread::current().id();
        let out = block_on(spawn_blocking(move || {
            (std::thread::current().id() != here, 6 * 7)
        }));
        assert_eq!(out, Ok((true, 42)));
    }

    #[test]
    fn a_panicking_job_is_an_error_and_the_pool_carries_on() {
        let _turn = turn();
        let out: Result<(), _> = block_on(spawn_blocking(|| panic!("job failed")));
        assert_eq!(out, Err(BlockingError::Panicked));
        assert_eq!(block_on(spawn_blocking(|| 1)), Ok(1));
    }

    #[test]
    fn a_future_dropped_before_its_job_starts_cancels_the_job() {
        let _turn = turn();
        // Hold every pool thread so the marker job has to queue.
        let limit = blocking_limit();
        let (release, held) = mpsc::channel::<()>();
        let held = Arc::new(std::sync::Mutex::new(held));
        let started = Arc::new(AtomicUsize::new(0));
        let holders: Vec<_> = (0..limit)
            .map(|_| {
                let held = held.clone();
                let started = started.clone();
                spawn_blocking(move || {
                    started.fetch_add(1, Ordering::SeqCst);
                    // One receiver, shared: each holder takes one release.
                    let _ = held.lock().unwrap().recv();
                })
            })
            .collect();
        while started.load(Ordering::SeqCst) < limit {
            std::thread::sleep(Duration::from_millis(1));
        }
        let ran = Arc::new(AtomicBool::new(false));
        let marker = {
            let ran = ran.clone();
            spawn_blocking(move || ran.store(true, Ordering::SeqCst))
        };
        drop(marker);
        for _ in 0..limit {
            release.send(()).unwrap();
        }
        for h in holders {
            assert_eq!(block_on(h), Ok(()));
        }
        // Every holder finished, so the queue behind them has drained too.
        assert_eq!(block_on(spawn_blocking(|| ())), Ok(()));
        assert!(!ran.load(Ordering::SeqCst), "a cancelled job ran");
    }

    #[test]
    fn many_jobs_complete_within_the_thread_limit() {
        let _turn = turn();
        let limit = blocking_limit();
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let jobs: Vec<_> = (0..limit * 8)
            .map(|i| {
                let (live, peak) = (live.clone(), peak.clone());
                spawn_blocking(move || {
                    let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(2));
                    live.fetch_sub(1, Ordering::SeqCst);
                    i
                })
            })
            .collect();
        let mut sum = 0;
        for j in jobs {
            sum += block_on(j).unwrap();
        }
        assert_eq!(sum, (0..limit * 8).sum::<usize>());
        assert!(peak.load(Ordering::SeqCst) <= limit, "{peak:?} > {limit}");
        assert!(blocking_threads() <= limit);
    }

    #[test]
    fn a_detached_job_still_runs() {
        let _turn = turn();
        let (tx, rx) = mpsc::channel();
        spawn_blocking(move || tx.send(5).unwrap()).detach();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(5));
    }

    #[test]
    fn an_inline_job_runs_at_the_first_poll_on_the_polling_thread() {
        let _turn = turn();
        let here = std::thread::current().id();
        super::set_blocking_inline(true);
        let fut = spawn_blocking(move || std::thread::current().id() == here);
        let detached = spawn_blocking(|| ());
        super::set_blocking_inline(false);
        assert_eq!(block_on(fut), Ok(true));
        detached.detach();
    }

    #[test]
    fn idle_threads_leave_after_a_while() {
        let _turn = turn();
        assert_eq!(block_on(spawn_blocking(|| ())), Ok(()));
        assert!(blocking_threads() >= 1);
        // Other tests in this binary may keep the pool busy; wait for a quiet span.
        let deadline = std::time::Instant::now() + Duration::from_secs(12);
        while blocking_threads() > 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(blocking_threads(), 0, "idle pool threads did not exit");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc as StdArc;
    use std::task::Wake;

    /// The ~20-line park/unpark executor every part's future tests use.
    pub(super) fn block_on<F: Future>(mut fut: F) -> F::Output {
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
