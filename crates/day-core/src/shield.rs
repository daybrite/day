//! The system-gesture / interactive-dismiss shield (docs/cover.md): the registries behind the
//! `defers_system_gestures` and `interactive_dismiss_disabled` modifiers. Each mounted modifier
//! pushes an entry while its subtree is alive; the union of entries is the app's current request.
//!
//! Gesture deferral flows straight to the backend (`Toolkit::defer_system_gestures`) on every
//! change. Dismiss-disabled is a *query*: the `cover` piece reads it — reactively, via the
//! change [`Trigger`] — when deciding whether a native back may close it and when patching the
//! presented surface's `isModalInPresentation`-style flag.

use std::cell::{Cell, RefCell};

use day_reactive::Signal;
use day_spec::Edges;

use crate::tree::{has_tree, with_tree};

thread_local! {
    static DEFERRALS: RefCell<Vec<(u64, Edges)>> = const { RefCell::new(Vec::new()) };
    static DISMISS_DISABLED: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    static NEXT: Cell<u64> = const { Cell::new(1) };
    /// Change counter for reactive readers ([`dismiss_disabled`]), created lazily by
    /// [`changed_signal`]. Root-scoped so it outlives whatever page scope touched it first.
    static CHANGED: Cell<Option<Signal<u64>>> = const { Cell::new(None) };
}

fn changed_signal() -> Signal<u64> {
    CHANGED.with(|c| match c.get() {
        Some(s) => s,
        None => {
            let s = Signal::global(0);
            c.set(Some(s));
            s
        }
    })
}

fn next_token() -> u64 {
    NEXT.with(|c| {
        let t = c.get();
        c.set(t + 1);
        t
    })
}

fn notify_changed() {
    changed_signal().update(|v| *v = v.wrapping_add(1));
}

fn apply_deferral() {
    let union = deferred_edges();
    if has_tree() {
        with_tree(|t| t.defer_system_gestures(union));
    }
    notify_changed();
}

/// Register an edge-deferral request (a `defers_system_gestures` subtree mounted). The backend
/// receives the new union immediately. Pair with [`pop_gesture_deferral`] on unmount.
pub fn push_gesture_deferral(edges: Edges) -> u64 {
    let token = next_token();
    DEFERRALS.with(|d| d.borrow_mut().push((token, edges)));
    apply_deferral();
    token
}

pub fn pop_gesture_deferral(token: u64) {
    DEFERRALS.with(|d| d.borrow_mut().retain(|(t, _)| *t != token));
    apply_deferral();
}

/// The union of every live deferral request (`Edges::NONE` when there are none). Untracked.
pub fn deferred_edges() -> Edges {
    DEFERRALS.with(|d| {
        d.borrow()
            .iter()
            .fold(Edges::NONE, |acc, (_, e)| acc.union(*e))
    })
}

/// Register an interactive-dismiss-disabled request (an `interactive_dismiss_disabled` subtree
/// mounted). Pair with [`pop_dismiss_disabled`] on unmount.
pub fn push_dismiss_disabled() -> u64 {
    let token = next_token();
    DISMISS_DISABLED.with(|d| d.borrow_mut().push(token));
    notify_changed();
    token
}

pub fn pop_dismiss_disabled(token: u64) {
    DISMISS_DISABLED.with(|d| d.borrow_mut().retain(|t| *t != token));
    notify_changed();
}

/// Whether any live subtree asked to disable interactive dismissal. Reads track the change
/// counter, so a binding re-runs as modifiers mount and unmount.
pub fn dismiss_disabled() -> bool {
    changed_signal().track();
    DISMISS_DISABLED.with(|d| !d.borrow().is_empty())
}
