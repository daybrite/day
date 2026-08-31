// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! App-lifecycle callbacks (docs/lifecycle.md). An app registers closures for [`day_spec::Lifecycle`]
//! phases with [`on_lifecycle`]; each backend, at the matching moment in its native app/activity
//! delegate, emits `Event::Lifecycle(phase)` (or day-core dispatches the launch phases uniformly),
//! and the event pump routes it here to run the closures inside a reactive batch — the same rails as
//! `Event::MenuAction`, so a lifecycle handler that writes signals updates the UI like any callback.
//!
//! Not every platform has every phase (a desktop app doesn't really enter the background), so a
//! handler registered for a phase the running backend doesn't deliver gets a one-time warning, and
//! apps can guard with [`lifecycle_supported`] (runtime) or `day::require_lifecycle!` (compile time).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use day_spec::Lifecycle;

/// The registered handlers for one phase.
type Handlers = Vec<Rc<dyn Fn()>>;

day_reactive::tls_slots! {
    lifecycle;
    static HANDLERS: RefCell<HashMap<Lifecycle, Handlers>> =
        RefCell::new(HashMap::new());
    /// Phases we've already warned about being unsupported (warn once, not per-handler).
    static WARNED: RefCell<std::collections::HashSet<Lifecycle>> =
        RefCell::new(std::collections::HashSet::new());
}

/// Forget every registered handler — a RE-MOUNT (docs/appearance.md): the app's `root()` is
/// about to run again and re-register, and handlers left from the previous mount would fire a
/// second time for every phase.
pub fn reset_handlers() {
    HANDLERS.with(|h| h.borrow_mut().clear());
    WARNED.with(|w| w.borrow_mut().clear());
}

/// Register `f` to run whenever the app reaches `phase`. Handlers run in registration order, in a
/// reactive batch (signal writes coalesce into one UI update). Register early — before `launch`, or
/// at the top of the root builder — so `WillLaunch`/`DidLaunch` handlers are in place when they fire.
///
/// If the running backend doesn't deliver `phase` (e.g. `DidEnterBackground` on desktop), the handler
/// is kept but will never run, and a one-time warning is logged. Prefer guarding the registration with
/// [`lifecycle_supported`] or the `day::require_lifecycle!` compile-time check.
pub fn on_lifecycle(phase: Lifecycle, f: impl Fn() + 'static) {
    HANDLERS.with(|h| h.borrow_mut().entry(phase).or_default().push(Rc::new(f)));
    // If the backend is already up we can check support now; otherwise `launch_with` sweeps
    // pre-registered phases once the tree exists (see `warn_unsupported_registrations`).
    if crate::tree::has_tree() {
        warn_if_unsupported(phase);
    }
}

/// Run every handler registered for `phase`, in a reactive batch. Called by the event pump on
/// `Event::Lifecycle`, and directly by `launch_with` for the launch phases.
pub fn dispatch_lifecycle(phase: Lifecycle) {
    let handlers = HANDLERS.with(|h| h.borrow().get(&phase).cloned().unwrap_or_default());
    if handlers.is_empty() {
        return;
    }
    let mut any_panicked = false;
    day_reactive::batch(|| {
        for f in &handlers {
            // Lifecycle callbacks run inside native trampolines (applicationWillTerminate,
            // GApplication::shutdown, the Android onDestroy JNI frame, …) that ABORT the process on
            // unwind. Contain each handler like the event pump does (DESIGN.md §8.5): a panic here —
            // classically an `eprintln!` hitting a closed stderr pipe during teardown — would
            // otherwise turn a clean exit into a spurious crash. `notify_contained_panic` runs
            // per-panic so a crash reporter (day-break) downgrades that handler's report to
            // contained, not fatal.
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f())).is_err() {
                any_panicked = true;
                log::warn!(
                    "an on_lifecycle({}) handler panicked and was contained — the app continues.",
                    phase.name()
                );
                crate::notify_contained_panic();
            }
        }
    });
    // Reset the reactive runtime AFTER the batch closes — `recover_from_panic` rewrites the batch
    // depth, so calling it mid-batch underflows the close.
    if any_panicked {
        day_reactive::recover_from_panic();
    }
}

/// Does the running backend deliver `phase`? Use this to guard registration at runtime:
/// `if day::lifecycle_supported(Lifecycle::DidEnterBackground) { on_lifecycle(...) }`.
///
/// Returns the universal answer (`phase.is_universal()`) if called before the backend is up.
pub fn lifecycle_supported(phase: Lifecycle) -> bool {
    if crate::tree::has_tree() {
        crate::with_tree(|t| t.supports_lifecycle(phase))
    } else {
        phase.is_universal()
    }
}

fn warn_if_unsupported(phase: Lifecycle) {
    if lifecycle_supported(phase) {
        return;
    }
    let first = WARNED.with(|w| w.borrow_mut().insert(phase));
    if first {
        log::warn!(
            "an `on_lifecycle({})` handler was registered, but this backend never delivers \
             that phase, so it will not run. Guard it with `day::lifecycle_supported(..)` or a \
             `day::require_lifecycle!(..)` compile-time check (docs/lifecycle.md).",
            phase.name()
        );
    }
}

/// Warn once for every ALREADY-registered phase the (now-known) backend doesn't deliver. Called by
/// `launch_with` right after the backend/tree is installed, covering handlers registered before launch.
pub fn warn_unsupported_registrations() {
    let phases: Vec<Lifecycle> = HANDLERS.with(|h| h.borrow().keys().copied().collect());
    for p in phases {
        warn_if_unsupported(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn dispatch_runs_handlers_for_the_phase_only() {
        thread_local! {
            static A: Cell<u32> = const { Cell::new(0) };
            static B: Cell<u32> = const { Cell::new(0) };
        }
        on_lifecycle(Lifecycle::DidLaunch, || A.with(|c| c.set(c.get() + 1)));
        on_lifecycle(Lifecycle::DidLaunch, || A.with(|c| c.set(c.get() + 1)));
        on_lifecycle(Lifecycle::WillTerminate, || B.with(|c| c.set(c.get() + 1)));

        dispatch_lifecycle(Lifecycle::DidLaunch);
        assert_eq!(A.with(Cell::get), 2, "both DidLaunch handlers ran");
        assert_eq!(B.with(Cell::get), 0, "WillTerminate handler did not run");

        dispatch_lifecycle(Lifecycle::WillTerminate);
        assert_eq!(B.with(Cell::get), 1);

        // A phase with no handlers is a silent no-op.
        dispatch_lifecycle(Lifecycle::DidReceiveMemoryWarning);
    }

    #[test]
    fn panicking_handler_is_contained_and_siblings_still_run() {
        thread_local! {
            static RAN: Cell<u32> = const { Cell::new(0) };
        }
        // A handler that panics (e.g. an `eprintln!` on a broken stderr pipe during teardown) must
        // NOT propagate — dispatch runs inside a native trampoline that would abort on unwind.
        on_lifecycle(Lifecycle::WillResignActive, || {
            panic!("boom in a lifecycle handler")
        });
        on_lifecycle(Lifecycle::WillResignActive, || {
            RAN.with(|c| c.set(c.get() + 1))
        });

        // Returns normally (containment) rather than unwinding, and the sibling still ran.
        dispatch_lifecycle(Lifecycle::WillResignActive);
        assert_eq!(
            RAN.with(Cell::get),
            1,
            "the non-panicking sibling handler still ran"
        );
    }
}
