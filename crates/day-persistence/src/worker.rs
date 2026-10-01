// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Queue-confined persistence. Neither model handles nor reactive signals cross the queue.
use crate::{DbError, DbErrorKind, ModelContainer};
use std::{
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    task::{Context, Poll},
};

type Job = Box<dyn FnOnce(&mut Session) -> Result<(), DbError> + Send>;
struct Session {
    db: ModelContainer,
    revision: u64,
    undo: Option<day_model::UndoStack>,
    needs_publish: bool,
    observers: Vec<Box<dyn Observation>>,
}
trait Observation {
    fn refresh(&mut self, db: &ModelContainer, revision: u64) -> Result<bool, DbError>;
}
impl Session {
    fn publish(&mut self) -> Result<(), DbError> {
        if !std::mem::take(&mut self.needs_publish) {
            return Ok(());
        }
        let mut error = None;
        self.observers.retain_mut(|o| {
            if error.is_some() {
                return true;
            }
            match o.refresh(&self.db, self.revision) {
                Ok(keep) => keep,
                Err(e) => {
                    error = Some(e);
                    true
                }
            }
        });
        error.map_or(Ok(()), Err)
    }
}

/// A committed value and the worker-local transaction revision it represents.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkerSnapshot<T> {
    pub revision: u64,
    pub value: T,
}
struct Latest<T> {
    value: Option<Result<WorkerSnapshot<T>, DbError>>,
    closed: bool,
    waker: Option<std::task::Waker>,
}
/// A bounded, latest-value subscription. Slow consumers receive the newest committed result,
/// not an unbounded queue of intermediate refreshes. Drop to unsubscribe. Values are compared
/// on the worker, so unchanged results do not wake the UI. This is not a transaction event log.
pub struct WorkerSubscription<T> {
    slot: Arc<Mutex<Latest<T>>>,
}
impl<T> WorkerSubscription<T> {
    /// Await the next changed snapshot; `None` after the worker closes. Cancelling this await
    /// is safe: it does not consume a value until it returns Ready.
    pub async fn next(&mut self) -> Option<Result<WorkerSnapshot<T>, DbError>> {
        std::future::poll_fn(|cx| {
            let mut slot = lock(&self.slot);
            if let Some(value) = slot.value.take() {
                return Poll::Ready(Some(value));
            }
            if slot.closed {
                return Poll::Ready(None);
            }
            slot.waker = Some(cx.waker().clone());
            Poll::Pending
        })
        .await
    }
}
struct Projection<T, F> {
    slot: std::sync::Weak<Mutex<Latest<T>>>,
    project: F,
    previous: Option<T>,
}
impl<T, F> Drop for Projection<T, F> {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.upgrade() {
            let waker = {
                let mut slot = lock(&slot);
                slot.closed = true;
                slot.waker.take()
            };
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
}
impl<T: Clone + PartialEq + Send + 'static, F: Fn(&ModelContainer) -> Result<T, DbError>>
    Observation for Projection<T, F>
{
    fn refresh(&mut self, db: &ModelContainer, revision: u64) -> Result<bool, DbError> {
        let Some(slot) = self.slot.upgrade() else {
            return Ok(false);
        };
        let value = match db.worker_read(|db| (self.project)(db)) {
            Ok(value) if self.previous.as_ref() == Some(&value) => return Ok(true),
            Ok(value) => {
                self.previous = Some(value.clone());
                Ok(WorkerSnapshot { revision, value })
            }
            Err(error) => {
                self.previous = None;
                Err(error)
            }
        };
        let fatal = value
            .as_ref()
            .err()
            .filter(|e| matches!(e.kind, DbErrorKind::Closed | DbErrorKind::Panicked))
            .cloned();
        let waker = {
            let mut slot = lock(&slot);
            slot.value = Some(value);
            slot.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        fatal.map_or(Ok(true), Err)
    }
}
#[derive(Clone, Debug)]
pub struct WorkerOptions {
    /// Maximum accepted requests waiting behind the running operation. Admission never blocks.
    pub capacity: usize,
    pub name: String,
}
impl Default for WorkerOptions {
    fn default() -> Self {
        Self {
            capacity: 256,
            name: "day-persistence".into(),
        }
    }
}
struct Control {
    sender: Option<SyncSender<Job>>,
    finished: bool,
    error: Option<DbError>,
    waiters: Vec<day_async::Deliver<Result<(), DbError>>>,
    thread: Option<std::thread::ThreadId>,
}
/// An explicitly asynchronous database queue. Construct the container in `open`'s factory:
/// its connection, caches, model handles and query observers stay on this worker until close.
/// Only owned `Send` arguments and results cross threads. Clones share FIFO admission order.
/// Do not return model `Store`/`Elem` handles or signals: their cross-thread facilities do not
/// make them independent of this queue's connection and lifetime. Return model values/IDs.
///
/// A container cannot accidentally escape through an owned result:
/// ```compile_fail
/// use day_persistence::{DatabaseWorker, ModelContainer};
/// fn escape(worker: &DatabaseWorker) {
///     let request = worker.read(|db| Ok(db.clone()));
/// }
/// ```
#[derive(Clone)]
pub struct DatabaseWorker {
    control: Arc<Mutex<Control>>,
}

/// An eagerly submitted operation. Dropping a read cancels it if it has not started. Dropping
/// a write's result never cancels an accepted transaction: await it to learn commit success.
#[must_use = "await the request to observe completion and persistence errors"]
pub struct WorkerRequest<R> {
    receiver: day_async::Oneshot<Result<R, DbError>>,
    cancelled: Arc<AtomicBool>,
    cancel_on_drop: bool,
}
impl<R> Future for WorkerRequest<R> {
    type Output = Result<R, DbError>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.receiver)
            .poll(cx)
            .map(|r| r.unwrap_or_else(|_| Err(closed())))
    }
}
impl<R> Drop for WorkerRequest<R> {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.cancelled.store(true, Ordering::Release);
        }
    }
}
fn closed() -> DbError {
    DbError::new(DbErrorKind::Closed, "database worker is closed")
}
fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|e| e.into_inner())
}
impl DatabaseWorker {
    /// Open/migrate on the owning queue, without blocking the caller's executor. The factory
    /// may create non-Send drivers, schema hooks and model state because it runs on the worker.
    pub async fn open(
        factory: impl FnOnce() -> Result<ModelContainer, DbError> + Send + 'static,
    ) -> Result<Self, DbError> {
        Self::open_with(WorkerOptions::default(), factory).await
    }

    pub async fn open_with(
        options: WorkerOptions,
        factory: impl FnOnce() -> Result<ModelContainer, DbError> + Send + 'static,
    ) -> Result<Self, DbError> {
        if options.capacity == 0 {
            return Err(DbError::new(
                DbErrorKind::Busy,
                "worker capacity must be positive",
            ));
        }
        let (sender, receiver) = mpsc::sync_channel::<Job>(options.capacity);
        let control = Arc::new(Mutex::new(Control {
            sender: Some(sender),
            finished: false,
            error: None,
            waiters: Vec::new(),
            thread: None,
        }));
        let weak = Arc::downgrade(&control);
        let (ready, opened) = day_async::oneshot();
        std::thread::Builder::new()
            .name(options.name)
            .spawn(move || {
                if let Some(control) = weak.upgrade() {
                    lock(&control).thread = Some(std::thread::current().id());
                }
                let result = catch_unwind(AssertUnwindSafe(|| {
                    let scope = day_reactive::Scope::detached();
                    let result = scope.enter(|| {
                        let container = match factory() {
                            Ok(container) => container,
                            Err(error) => {
                                ready.send(Err(error.clone()));
                                return Err(error);
                            }
                        };
                        ready.send(Ok(()));
                        // The thread holds no sender or strong worker reference. Dropping the last
                        // client drains accepted work, then closes the connection on this thread.
                        let mut session = Session {
                            db: container,
                            revision: 0,
                            undo: None,
                            needs_publish: false,
                            observers: Vec::new(),
                        };
                        let result = catch_unwind(AssertUnwindSafe(|| {
                            while let Ok(job) = receiver.recv() {
                                job(&mut session)?;
                                // Bound projection work during bursts without starving observers.
                                for _ in 1..32 {
                                    match receiver.try_recv() {
                                        Ok(job) => job(&mut session)?,
                                        Err(_) => break,
                                    }
                                }
                                session.publish()?;
                            }
                            session.db.save()
                        }))
                        .unwrap_or_else(|_| {
                            Err(DbError::new(
                                DbErrorKind::Panicked,
                                "database worker panicked",
                            ))
                        });
                        session.observers.clear();
                        session.undo.take();
                        session.db.worker_dispose_caches();
                        result
                    });
                    scope.dispose();
                    result
                }))
                .unwrap_or_else(|_| {
                    Err(DbError::new(
                        DbErrorKind::Panicked,
                        "database worker panicked",
                    ))
                });
                if let Some(control) = weak.upgrade() {
                    let waiters = {
                        let mut state = lock(&control);
                        state.sender.take();
                        state.finished = true;
                        state.error = result.as_ref().err().cloned();
                        std::mem::take(&mut state.waiters)
                    };
                    for waiter in waiters {
                        waiter.send(result.clone());
                    }
                }
            })
            .map_err(|e| DbError::driver(e.to_string()))?;
        opened.await.map_err(|_| {
            DbError::new(DbErrorKind::Panicked, "database worker failed during open")
        })??;
        Ok(Self { control })
    }

    /// Execute a read on the queue. Query-only mode and dirty-cache checks reject accidental
    /// writes; SQL reads and row materialization never run on the caller's thread.
    pub fn read<R: Send + 'static>(
        &self,
        read: impl FnOnce(&ModelContainer) -> Result<R, DbError> + Send + 'static,
    ) -> WorkerRequest<R> {
        self.submit(true, move |session| session.db.worker_read(read))
    }

    /// Execute one atomic transaction, then acknowledge its durable commit. Intermediate saves
    /// and read-your-writes stay within the transaction. A failed operation restores the cache.
    /// Accepted writes survive dropping this future. Admission failure means nothing ran.
    pub fn write<R: Send + 'static>(
        &self,
        write: impl FnOnce(&ModelContainer) -> Result<R, DbError> + Send + 'static,
    ) -> WorkerRequest<R> {
        self.submit(false, move |session| {
            let checkpoint = session.undo.as_ref().map(|undo| undo.checkpoint());
            let group = session.undo.as_ref().map(|undo| undo.begin_group("edit"));
            let result = session.db.worker_write(write);
            drop(group); // Seal even when no UI observer caused a reactive turn-end drain.
            if result.is_ok() {
                session.revision += 1;
                session.needs_publish = true;
            } else if let (Some(undo), Some(checkpoint)) = (&session.undo, checkpoint) {
                undo.restore_checkpoint(checkpoint);
            }
            result
        })
    }

    /// Install one queue-confined history. Call once before editing. Background imports should
    /// use `day_model::with_author` inside `write` so they do not enter user undo history.
    pub fn enable_undo(&self, levels: usize) -> WorkerRequest<()> {
        self.submit(false, move |session| {
            if session.undo.is_some() {
                return Err(DbError::driver("worker undo already installed"));
            }
            session.undo = Some(session.db.undo(levels));
            Ok(())
        })
    }

    /// Owned availability for a UI's native undo bridge; no reactive handles cross queues.
    pub fn undo_status(&self) -> WorkerRequest<(bool, bool)> {
        self.submit(true, |session| {
            Ok(session
                .undo
                .as_ref()
                .map(|u| (u.can_undo().get_untracked(), u.can_redo().get_untracked()))
                .unwrap_or_default())
        })
    }

    /// Replay and commit atomically on the owning queue. A failed commit restores the history.
    pub fn undo(&self, redo: bool) -> WorkerRequest<bool> {
        self.submit(false, move |session| {
            let Some(undo) = &session.undo else {
                return Ok(false);
            };
            let checkpoint = undo.checkpoint();
            let result = session
                .db
                .worker_write(|_| Ok(if redo { undo.redo() } else { undo.undo() }));
            if result.is_ok() {
                session.revision += 1;
                session.needs_publish = true;
            } else {
                undo.restore_checkpoint(checkpoint);
            }
            result
        })
    }

    /// Observe an owned projection after committed writes. The projection must be read-only;
    /// it runs on the database queue. Registration participates in FIFO admission, so the first
    /// result includes all writes accepted before it. Subsequent results may coalesce revisions.
    pub async fn observe<T: Clone + PartialEq + Send + 'static>(
        &self,
        project: impl Fn(&ModelContainer) -> Result<T, DbError> + Send + 'static,
    ) -> Result<WorkerSubscription<T>, DbError> {
        let slot = Arc::new(Mutex::new(Latest {
            value: None,
            closed: false,
            waker: None,
        }));
        let projection = Projection {
            slot: Arc::downgrade(&slot),
            project,
            previous: None,
        };
        self.submit(true, move |session| {
            session.observers.push(Box::new(projection));
            session.needs_publish = true;
            Ok(())
        })
        .await?;
        Ok(WorkerSubscription { slot })
    }

    /// Create a consistent SQLite backup after all earlier accepted work. Backup runs outside
    /// an explicit transaction because SQLite's VACUUM INTO requires autocommit mode. The
    /// driver supplies the snapshot guarantee. The destination must not already exist.
    pub fn backup_to(&self, path: impl Into<std::path::PathBuf>) -> WorkerRequest<()> {
        let path = path.into();
        self.submit(true, move |session| session.db.backup_to(&path))
    }

    /// Explicitly merge writes from another connection on this queue, then refresh projections.
    /// Applications using one worker as the sole writer do not need polling.
    pub fn check_external(&self) -> WorkerRequest<bool> {
        self.submit(true, |session| {
            let changed = session.db.check_external()?;
            if changed {
                session.revision += 1;
                session.needs_publish = true;
            }
            Ok(changed)
        })
    }

    fn submit<R: Send + 'static>(
        &self,
        read: bool,
        operation: impl FnOnce(&mut Session) -> Result<R, DbError> + Send + 'static,
    ) -> WorkerRequest<R> {
        let (deliver, receiver) = day_async::oneshot();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        // Keep the completion in a shared slot so rejected jobs can deliver a precise error.
        let completion = Arc::new(Mutex::new(Some(deliver)));
        let done = completion.clone();
        let job: Job = Box::new(move |db| {
            let result = if read && cancel.load(Ordering::Acquire) {
                Err(DbError::new(
                    DbErrorKind::Cancelled,
                    "database read cancelled before execution",
                ))
            } else {
                catch_unwind(AssertUnwindSafe(|| operation(db))).unwrap_or_else(|_| {
                    Err(DbError::new(
                        DbErrorKind::Panicked,
                        "database operation panicked",
                    ))
                })
            };
            let fatal = result
                .as_ref()
                .err()
                .filter(|e| matches!(e.kind, DbErrorKind::Panicked | DbErrorKind::Closed))
                .cloned();
            let deliver = lock(&done).take();
            if let Some(deliver) = deliver {
                deliver.send(result);
            }
            fatal.map_or(Ok(()), Err)
        });
        let mut rejected_job = None;
        let rejected = {
            let state = lock(&self.control);
            if state.thread == Some(std::thread::current().id()) {
                Some(DbError::driver(
                    "a database worker cannot submit work to its own queue",
                ))
            } else {
                match &state.sender {
                    None => Some(state.error.clone().unwrap_or_else(closed)),
                    Some(sender) => match sender.try_send(job) {
                        Ok(()) => None,
                        Err(mpsc::TrySendError::Full(job)) => {
                            rejected_job = Some(job);
                            Some(DbError::new(
                                DbErrorKind::Busy,
                                "database worker queue is full",
                            ))
                        }
                        Err(mpsc::TrySendError::Disconnected(job)) => {
                            rejected_job = Some(job);
                            Some(closed())
                        }
                    },
                }
            }
        };
        // Captures may run arbitrary Drop code (including another submission). Never drop
        // a rejected closure while holding the queue's admission lock.
        drop(rejected_job);
        if let Some(error) = rejected {
            let deliver = lock(&completion).take();
            if let Some(deliver) = deliver {
                deliver.send(Err(error));
            }
        }
        WorkerRequest {
            receiver,
            cancelled,
            cancel_on_drop: read,
        }
    }

    /// Last-chance synchronous shutdown for an OS termination callback that cannot await.
    /// Normal application code should use `close().await`. This waits for every accepted
    /// operation and may take as long as the longest transaction. Never call it while holding
    /// a lock a submitted operation needs. Calling it from this worker is rejected.
    pub fn close_blocking(&self) -> Result<(), DbError> {
        struct Unpark(std::thread::Thread);
        impl std::task::Wake for Unpark {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = std::task::Waker::from(Arc::new(Unpark(std::thread::current())));
        let mut context = Context::from_waker(&waker);
        let mut close = std::pin::pin!(self.close());
        loop {
            match close.as_mut().poll(&mut context) {
                Poll::Ready(result) => return result,
                Poll::Pending => std::thread::park(),
            }
        }
    }

    /// Stop admission for every clone, drain accepted writes, and close on the owning queue.
    /// No join or disk flush blocks the caller. Repeated close calls share completion.
    pub async fn close(&self) -> Result<(), DbError> {
        let (deliver, receiver) = day_async::oneshot();
        {
            let mut state = lock(&self.control);
            if state.thread == Some(std::thread::current().id()) {
                return Err(DbError::driver(
                    "a database worker cannot wait for its own close",
                ));
            }
            state.sender.take();
            if state.finished {
                return state.error.clone().map_or(Ok(()), Err);
            }
            state.waiters.push(deliver);
        }
        receiver.await.map_err(|_| closed())?
    }
}
