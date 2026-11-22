// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Scope-owned asynchronous data with typed errors and retained refresh values.

use crate::{Scope, Signal, batch, create_reaction, spawn_local, untrack};
use std::{
    cell::{Cell, RefCell},
    future::Future,
    rc::Rc,
};

/// An async load's state. Shared values make snapshots cheap without requiring `T` or `E`
/// to implement `Clone`. `Refreshing` and a failed refresh retain their previous value.
#[derive(Debug)]
pub enum LoadState<T, E> {
    /// No source selected, or an initial load was cancelled.
    Idle,
    /// Loading without a previous value.
    Loading,
    /// The last load succeeded.
    Ready(Rc<T>),
    /// Loading with a previous value.
    Refreshing(Rc<T>),
    /// A failed load, with the previous value when there was one.
    Failed {
        error: Rc<E>,
        previous: Option<Rc<T>>,
    },
}

impl<T, E> Clone for LoadState<T, E> {
    fn clone(&self) -> Self {
        match self {
            Self::Idle => Self::Idle,
            Self::Loading => Self::Loading,
            Self::Ready(v) => Self::Ready(v.clone()),
            Self::Refreshing(v) => Self::Refreshing(v.clone()),
            Self::Failed { error, previous } => Self::Failed {
                error: error.clone(),
                previous: previous.clone(),
            },
        }
    }
}

impl<T, E> LoadState<T, E> {
    /// The available value, including during refresh or after a refresh failure.
    pub fn value(&self) -> Option<&T> {
        self.shared_value().map(Rc::as_ref)
    }
    /// The shared available value, without copying its contents.
    pub fn shared_value(&self) -> Option<&Rc<T>> {
        match self {
            Self::Ready(v) | Self::Refreshing(v) => Some(v),
            Self::Failed { previous, .. } => previous.as_ref(),
            Self::Idle | Self::Loading => None,
        }
    }
    /// The typed failure, if the last request failed.
    pub fn error(&self) -> Option<&E> {
        match self {
            Self::Failed { error, .. } => Some(error),
            _ => None,
        }
    }
    /// Whether a request is running, with or without previous data.
    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading | Self::Refreshing(_))
    }
    /// Whether a request is running while previous data remains available.
    pub fn is_refreshing(&self) -> bool {
        matches!(self, Self::Refreshing(_))
    }
}

/// Source-change policy, chosen before the first load starts.
#[derive(Clone, Copy, Debug, Default)]
pub struct AsyncDataOptions {
    /// Retain the last value when the source changes. It is reported as stale until a load
    /// for the new source succeeds. Same-source refetches always retain data; an absent
    /// optional source always clears it, regardless of this setting.
    pub keep_previous: bool,
}

struct Snapshot<T, E> {
    state: LoadState<T, E>,
    stale: bool,
}
impl<T, E> Clone for Snapshot<T, E> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            stale: self.stale,
        }
    }
}
impl<T, E> Snapshot<T, E> {
    fn idle() -> Self {
        Self {
            state: LoadState::Idle,
            stale: false,
        }
    }
}

struct Control<T, E> {
    generation: Cell<u64>,
    running: Cell<bool>,
    abort: RefCell<Option<Box<dyn FnOnce()>>>,
    settled: RefCell<Snapshot<T, E>>,
}
impl<T, E> Control<T, E> {
    fn supersede(&self) -> u64 {
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        self.running.set(false);
        let abort = self.abort.borrow_mut().take();
        if let Some(abort) = abort {
            abort();
        }
        generation
    }
}

/// Reactive asynchronous data. A `Copy`, main-thread handle owned by its creation scope.
///
/// A changed source cancels the old request and clears its data by default. [`Self::refetch`]
/// keeps the previous value, including if the new request fails. Scope disposal cancels
/// pending work. A completion from an obsolete request never changes the state.
///
/// Reads are tracked; use [`crate::untrack`] for action callbacks. The state is read-only:
/// writes go through the fetcher, so request bookkeeping cannot be bypassed.
pub struct AsyncData<T: 'static, E: 'static = std::convert::Infallible> {
    snapshot: Signal<Snapshot<T, E>>,
    control: Signal<Rc<Control<T, E>>>,
    refetch_count: Signal<u64>,
}
impl<T, E> Copy for AsyncData<T, E> {}
impl<T, E> Clone for AsyncData<T, E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static, E: 'static> AsyncData<T, E> {
    /// Load immediately and whenever the tracked source changes. Only reads in `source`
    /// subscribe; reads in `fetcher` do not. Futures and typed errors need not be `Send`.
    pub fn new<S, F>(source: impl Fn() -> S + 'static, fetcher: impl Fn(S) -> F + 'static) -> Self
    where
        S: Clone + PartialEq + 'static,
        F: Future<Output = Result<T, E>> + 'static,
    {
        Self::with_options(source, fetcher, AsyncDataOptions::default())
    }
    /// Like [`Self::new`], with an explicit source-change policy.
    pub fn with_options<S, F>(
        source: impl Fn() -> S + 'static,
        fetcher: impl Fn(S) -> F + 'static,
        options: AsyncDataOptions,
    ) -> Self
    where
        S: Clone + PartialEq + 'static,
        F: Future<Output = Result<T, E>> + 'static,
    {
        Self::optional_with_options(move || Some(source()), fetcher, options)
    }
    /// Load only when the source is `Some`. `None` cancels work and clears data and errors.
    pub fn optional<S, F>(
        source: impl Fn() -> Option<S> + 'static,
        fetcher: impl Fn(S) -> F + 'static,
    ) -> Self
    where
        S: Clone + PartialEq + 'static,
        F: Future<Output = Result<T, E>> + 'static,
    {
        Self::optional_with_options(source, fetcher, AsyncDataOptions::default())
    }
    /// Like [`Self::optional`], with an explicit source-change policy.
    pub fn optional_with_options<S, F>(
        source: impl Fn() -> Option<S> + 'static,
        fetcher: impl Fn(S) -> F + 'static,
        options: AsyncDataOptions,
    ) -> Self
    where
        S: Clone + PartialEq + 'static,
        F: Future<Output = Result<T, E>> + 'static,
    {
        let scope = Scope::current();
        let snapshot = Signal::new(Snapshot::idle());
        let control = Rc::new(Control {
            generation: Cell::new(0),
            running: Cell::new(false),
            abort: RefCell::new(None),
            settled: RefCell::new(Snapshot::idle()),
        });
        let data = Self {
            snapshot,
            control: Signal::new(control.clone()),
            refetch_count: Signal::new(0),
        };
        let cleanup = control.clone();
        scope.on_cleanup(move || {
            cleanup.supersede();
        });
        let previous = RefCell::new(None::<(Option<S>, u64)>);
        let value_source = Rc::new(RefCell::new(None::<S>));
        create_reaction(
            Rc::new(move || {
                let count = data.refetch_count.get();
                let source = source();
                untrack(|| {
                    let changed = {
                        let mut previous = previous.borrow_mut();
                        if previous
                            .as_ref()
                            .is_some_and(|(key, n)| key == &source && *n == count)
                        {
                            return;
                        }
                        let changed = previous.as_ref().is_none_or(|(key, _)| key != &source);
                        *previous = Some((source.clone(), count));
                        changed
                    };
                    let generation = control.supersede();
                    if !scope.is_alive() || control.generation.get() != generation {
                        return;
                    }
                    let Some(source) = source else {
                        *value_source.borrow_mut() = None;
                        *control.settled.borrow_mut() = Snapshot::idle();
                        snapshot.set(Snapshot::idle());
                        return;
                    };
                    let mut settled = control.settled.borrow().clone();
                    if changed && !options.keep_previous {
                        settled = Snapshot::idle();
                        *value_source.borrow_mut() = None;
                    }
                    settled.stale = settled.state.value().is_some()
                        && value_source.borrow().as_ref() != Some(&source);
                    *control.settled.borrow_mut() = settled.clone();
                    let state = settled
                        .state
                        .shared_value()
                        .map_or(LoadState::Loading, |v| LoadState::Refreshing(v.clone()));
                    control.running.set(true);
                    snapshot.set(Snapshot {
                        state,
                        stale: settled.stale,
                    });
                    if !scope.is_alive() || control.generation.get() != generation {
                        return;
                    }
                    let future = fetcher(source.clone());
                    if !scope.is_alive() || control.generation.get() != generation {
                        return;
                    }
                    let done = control.clone();
                    let value_source = value_source.clone();
                    let abort = spawn_local(Box::pin(async move {
                        let result = future.await;
                        if !scope.is_alive() || done.generation.get() != generation {
                            return;
                        }
                        done.running.set(false);
                        let old_abort = done.abort.borrow_mut().take();
                        drop(old_abort);
                        let next = match result {
                            Ok(value) => {
                                *value_source.borrow_mut() = Some(source);
                                Snapshot {
                                    state: LoadState::Ready(Rc::new(value)),
                                    stale: false,
                                }
                            }
                            Err(error) => Snapshot {
                                state: LoadState::Failed {
                                    error: Rc::new(error),
                                    previous: settled.state.shared_value().cloned(),
                                },
                                stale: settled.stale,
                            },
                        };
                        *done.settled.borrow_mut() = next.clone();
                        snapshot.set(next);
                    }));
                    // Eager polling can finish, cancel, dispose or start another request. Never
                    // install an obsolete abort handle over the current request's handle.
                    if control.generation.get() == generation
                        && control.running.get()
                        && scope.is_alive()
                    {
                        *control.abort.borrow_mut() = Some(abort);
                    } else {
                        abort();
                    }
                });
            }),
            1,
        );
        data
    }
    /// A cheap, read-only snapshot; clones shared pointers, not `T` or `E`.
    pub fn state(self) -> LoadState<T, E> {
        self.snapshot.with(|s| s.state.clone())
    }
    /// Borrow the whole state without cloning it.
    pub fn with_state<R>(self, read: impl FnOnce(&LoadState<T, E>) -> R) -> R {
        self.snapshot.with(|s| read(&s.state))
    }
    /// Clone the available value, including during refresh and after refresh failure.
    pub fn value(self) -> Option<T>
    where
        T: Clone,
    {
        self.with_state(|s| s.value().cloned())
    }
    /// Borrow the available value, with no `Clone` bound.
    pub fn with_value<R>(self, read: impl FnOnce(Option<&T>) -> R) -> R {
        self.with_state(|s| read(s.value()))
    }
    /// Clone the typed error, if present.
    pub fn error(self) -> Option<E>
    where
        E: Clone,
    {
        self.with_state(|s| s.error().cloned())
    }
    /// Borrow the typed error, with no `Clone` bound.
    pub fn with_error<R>(self, read: impl FnOnce(Option<&E>) -> R) -> R {
        self.with_state(|s| read(s.error()))
    }
    /// Whether a value is available, including while refreshing or after a refresh failure.
    pub fn has_value(self) -> bool {
        self.with_value(|v| v.is_some())
    }
    /// Whether any request is in flight.
    pub fn loading(self) -> bool {
        self.with_state(LoadState::is_loading)
    }
    /// Whether a request is in flight with a retained value.
    pub fn refreshing(self) -> bool {
        self.with_state(LoadState::is_refreshing)
    }
    /// Whether the retained value belongs to a different source from the current request.
    /// An ordinary same-source refresh is not stale in this sense.
    pub fn stale(self) -> bool {
        self.snapshot.with(|s| s.stale)
    }
    /// Force a request for the current source, retaining its previous value. Batched calls
    /// coalesce; an absent optional source stays idle.
    pub fn refetch(self) {
        self.refetch_count.update(|n| *n = n.wrapping_add(1));
    }
    /// Cancel the running request and restore its last settled state. Initial cancellation
    /// returns to idle; cancellation is not an error. Does nothing after scope disposal.
    pub fn cancel(self) {
        let control = untrack(|| self.control.try_with(Clone::clone));
        let Some(control) = control else {
            return;
        };
        if !control.running.get() {
            return;
        }
        let generation = control.supersede();
        if control.generation.get() == generation {
            let settled = control.settled.borrow().clone();
            batch(|| self.snapshot.set(settled));
        }
    }
}

impl<T: 'static> AsyncData<T> {
    /// Load data that cannot fail, without wrapping the result in `Ok`.
    pub fn infallible<S, F>(
        source: impl Fn() -> S + 'static,
        fetcher: impl Fn(S) -> F + 'static,
    ) -> Self
    where
        S: Clone + PartialEq + 'static,
        F: Future<Output = T> + 'static,
    {
        Self::new(source, move |s| {
            let future = fetcher(s);
            async move { Ok(future.await) }
        })
    }
}
