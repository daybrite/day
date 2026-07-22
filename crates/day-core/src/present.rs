//! Imperative presentation (docs/dialogs.md): a minimal single-threaded async executor
//! (`task`) plus the pending-request registry that routes a `present(spec).await` through
//! the tree to the backend and its `Event::PresentResult` answer back to the future.
//!
//! Everything is thread-local and `!Send` — Day has one UI thread. The executor is std-only
//! (no async runtime): tasks are boxed futures polled on the main loop; a presentation
//! future parks a `Waker` that re-polls its task through `day_reactive::on_main`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use day_spec::present::{PresentResult, PresentSpec};

use crate::with_tree;

type LocalFuture = Pin<Box<dyn Future<Output = ()> + 'static>>;

thread_local! {
    /// Live async flows. `None` = currently being polled (taken out to avoid re-entrant borrow).
    static TASKS: RefCell<HashMap<u64, Option<LocalFuture>>> = RefCell::new(HashMap::new());
    static NEXT_TASK: Cell<u64> = const { Cell::new(1) };
    static PENDING: RefCell<HashMap<u64, PendingEntry>> = RefCell::new(HashMap::new());
    static NEXT_REQ: Cell<u64> = const { Cell::new(1) };
}

/// An app-writable scratch directory (docs/files.md) — re-exported from `day_spec::present` so
/// `day_core::app_temp_dir()` keeps working for the pieces layer's file-save staging.
pub use day_spec::present::app_temp_dir;

struct PendingEntry {
    shared: Rc<PendingShared>,
    spec: PresentSpec,
}

struct PendingShared {
    result: RefCell<Option<PresentResult>>,
    waker: RefCell<Option<Waker>>,
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

/// A handle to a spawned [`task`]. `Copy` and `!Send` (the executor is thread-local).
///
/// Task ids are monotonic and never reused, so a stale handle is always a harmless miss:
/// [`TaskHandle::abort`] after completion is a no-op and [`TaskHandle::is_finished`] stays true.
#[derive(Clone, Copy)]
pub struct TaskHandle {
    id: u64,
    _not_send: std::marker::PhantomData<*const ()>,
}

impl TaskHandle {
    /// Remove and drop the task's future. An in-flight `.await` cancels via `Drop` (e.g. a
    /// `FetchFuture` inside cancels its platform request). No-op if the task already finished.
    pub fn abort(self) {
        // Take the future OUT of the map and drop it after the RefCell borrow ends: a future
        // whose Drop re-enters the executor (spawns a task, aborts another handle) would
        // otherwise hit a live borrow. If the task is mid-poll its slot was taken (`None`) —
        // removing the entry then makes `poll_task`'s Pending put-back find nothing, and the
        // future drops there instead.
        let fut = TASKS.with(|t| t.borrow_mut().remove(&self.id));
        drop(fut);
    }

    /// Whether the task no longer runs — completed or aborted.
    pub fn is_finished(self) -> bool {
        !TASKS.with(|t| t.borrow().contains_key(&self.id))
    }
}

/// Spawn an async flow onto Day's main-loop executor. This is the opt-in seam for actions
/// that open modals or pickers: `button.action(|| day::task(async move { … .await … }))`.
/// The future is polled once before this returns; the returned handle can [`TaskHandle::abort`]
/// it and is freely discardable.
pub fn task(fut: impl Future<Output = ()> + 'static) -> TaskHandle {
    let id = NEXT_TASK.with(|c| {
        let v = c.get();
        c.set(v + 1);
        v
    });
    TASKS.with(|t| t.borrow_mut().insert(id, Some(Box::pin(fut))));
    poll_task(id);
    TaskHandle {
        id,
        _not_send: std::marker::PhantomData,
    }
}

fn poll_task(id: u64) {
    let fut = TASKS.with(|t| t.borrow_mut().get_mut(&id).and_then(|s| s.take()));
    let Some(mut fut) = fut else {
        return; // finished or spuriously woken
    };
    let waker = task_waker(id);
    let mut cx = Context::from_waker(&waker);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(()) => {
            TASKS.with(|t| {
                t.borrow_mut().remove(&id);
            });
        }
        Poll::Pending => {
            TASKS.with(|t| {
                if let Some(slot) = t.borrow_mut().get_mut(&id) {
                    *slot = Some(fut);
                }
            });
        }
    }
}

// Minimal RawWaker: the "data" pointer IS the task id; waking re-polls it on the main loop.
fn task_waker(id: u64) -> Waker {
    // SAFETY: the vtable only ever reads the pointer back as an integer id; nothing is
    // dereferenced, so any bit pattern is sound.
    unsafe { Waker::from_raw(raw_waker(id as usize as *const ())) }
}

fn raw_waker(data: *const ()) -> RawWaker {
    RawWaker::new(data, &VTABLE)
}

static VTABLE: RawWakerVTable = RawWakerVTable::new(
    raw_waker, // clone
    |d| wake_task(d as usize as u64),
    |d| wake_task(d as usize as u64),
    |_| {},
);

fn wake_task(id: u64) {
    day_reactive::on_main(move || poll_task(id));
}

// ---------------------------------------------------------------------------
// Presentation
// ---------------------------------------------------------------------------

/// Present a native modal and await its answer (docs/dialogs.md). The pieces layer wraps
/// this in `Alert`/`confirm`/`prompt`; call it directly for a custom `PresentSpec`.
pub fn present(spec: PresentSpec) -> PresentFuture {
    let req = NEXT_REQ.with(|c| {
        let v = c.get();
        c.set(v + 1);
        v
    });
    PresentFuture {
        req,
        spec: Some(spec),
        shared: Rc::new(PendingShared {
            result: RefCell::new(None),
            waker: RefCell::new(None),
        }),
        presented: false,
    }
}

pub struct PresentFuture {
    req: u64,
    spec: Option<PresentSpec>,
    shared: Rc<PendingShared>,
    presented: bool,
}

impl Future for PresentFuture {
    type Output = PresentResult;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<PresentResult> {
        if let Some(result) = self.shared.result.borrow_mut().take() {
            return Poll::Ready(result);
        }
        if !self.presented {
            self.presented = true;
            let req = self.req;
            let spec = self.spec.take().expect("present spec");
            PENDING.with(|p| {
                p.borrow_mut().insert(
                    req,
                    PendingEntry {
                        shared: self.shared.clone(),
                        spec: spec.clone(),
                    },
                )
            });
            with_tree(|t| t.present(req, &spec));
        }
        *self.shared.waker.borrow_mut() = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// Deliver a native answer (the modal already dismissed itself). Called from `pump_events`
/// on `Event::PresentResult`.
pub fn resolve_presentation(req: u64, result: PresentResult) {
    let entry = PENDING.with(|p| p.borrow_mut().remove(&req));
    if let Some(entry) = entry {
        *entry.shared.result.borrow_mut() = Some(result);
        if let Some(waker) = entry.shared.waker.borrow_mut().take() {
            waker.wake();
        }
    }
}

/// Answer a still-open modal programmatically (dayscript). Resolves with the given result
/// FIRST (removing the pending request), then dismisses the native control — so the native
/// dismissal's own completion event finds nothing pending and is a no-op. False = no such
/// pending request.
pub fn respond_presentation(req: u64, result: PresentResult) -> bool {
    if PENDING.with(|p| !p.borrow().contains_key(&req)) {
        return false;
    }
    resolve_presentation(req, result);
    with_tree(|t| t.dismiss(req));
    true
}

/// The most recently opened still-pending presentation, for dayscript inspection/response.
pub fn pending_presentation() -> Option<(u64, PresentSpec)> {
    PENDING.with(|p| {
        p.borrow()
            .iter()
            .max_by_key(|(req, _)| **req)
            .map(|(req, entry)| (*req, entry.spec.clone()))
    })
}

// ---------------------------------------------------------------------------
// Executor tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod task_tests {
    use super::*;
    use std::cell::Cell;
    use std::task::Waker;

    /// Every executor test is single-threaded, so an INLINE poster (run the closure now) is
    /// correct. `install_main_poster` is first-install-wins, so repeated `init` calls are fine;
    /// `TASKS` is thread-local, so tests don't see each other's tasks.
    fn init() {
        day_reactive::install_main_poster(|f| f());
    }

    /// Sets its flag when dropped — observes whether an aborted future was actually destroyed.
    struct DropFlag(Rc<Cell<bool>>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    fn pending_forever(flag: Rc<Cell<bool>>) -> impl Future<Output = ()> {
        let guard = DropFlag(flag);
        std::future::poll_fn(move |_| {
            let _ = &guard;
            Poll::Pending
        })
    }

    #[test]
    fn task_runs_to_completion() {
        init();
        let ran = Rc::new(Cell::new(false));
        let r = ran.clone();
        let h = task(async move { r.set(true) });
        assert!(ran.get());
        assert!(h.is_finished());
    }

    #[test]
    fn abort_drops_pending_future() {
        init();
        let dropped = Rc::new(Cell::new(false));
        let h = task(pending_forever(dropped.clone()));
        assert!(!h.is_finished());
        assert!(!dropped.get());
        h.abort();
        assert!(dropped.get());
        assert!(h.is_finished());
    }

    #[test]
    fn abort_after_completion_is_noop() {
        init();
        let h = task(async {});
        assert!(h.is_finished());
        h.abort();
        assert!(h.is_finished());
    }

    /// A task that aborts ITSELF from inside `poll`: the slot is already taken (`None`), abort
    /// removes the map entry, and the `Pending` put-back finds nothing — the future must drop
    /// exactly once, after the poll returns, with no `RefCell` re-borrow.
    #[test]
    fn abort_self_while_polling() {
        init();
        let handle: Rc<Cell<Option<TaskHandle>>> = Rc::new(Cell::new(None));
        let dropped = Rc::new(Cell::new(false));
        let waker_slot: Rc<RefCell<Option<Waker>>> = Rc::new(RefCell::new(None));

        let h_cell = handle.clone();
        let w_slot = waker_slot.clone();
        let guard = DropFlag(dropped.clone());
        let h = task(std::future::poll_fn(move |cx| {
            let _ = &guard;
            match h_cell.get() {
                None => {
                    *w_slot.borrow_mut() = Some(cx.waker().clone());
                    Poll::Pending
                }
                Some(h) => {
                    h.abort();
                    Poll::Pending
                }
            }
        }));
        handle.set(Some(h));
        assert!(!dropped.get());
        let waker = waker_slot
            .borrow_mut()
            .take()
            .expect("waker from first poll");
        waker.wake(); // inline poster: re-polls now; the second poll self-aborts
        assert!(dropped.get());
        assert!(h.is_finished());
    }

    /// Aborting a future whose Drop re-enters the executor (spawns a new task) must not panic:
    /// `abort` releases the `TASKS` borrow before the future is destroyed.
    #[test]
    fn abort_reentrancy_from_drop() {
        init();
        struct SpawnOnDrop;
        impl Drop for SpawnOnDrop {
            fn drop(&mut self) {
                task(async {});
            }
        }
        let guard = SpawnOnDrop;
        let h = task(std::future::poll_fn(move |_| {
            let _ = &guard;
            Poll::<()>::Pending
        }));
        h.abort();
        assert!(h.is_finished());
    }
}
