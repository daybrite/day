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

/// Spawn an async flow onto Day's main-loop executor. This is the opt-in seam for actions
/// that open modals or pickers: `button.action(|| day::task(async move { … .await … }))`.
pub fn task(fut: impl Future<Output = ()> + 'static) {
    let id = NEXT_TASK.with(|c| {
        let v = c.get();
        c.set(v + 1);
        v
    });
    TASKS.with(|t| t.borrow_mut().insert(id, Some(Box::pin(fut))));
    poll_task(id);
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
