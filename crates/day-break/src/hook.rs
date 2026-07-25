//! The Rust panic hook and the day-core contained-panic observer.
//!
//! The hook fires for EVERY panic — including one day-core will contain at a trampoline boundary
//! and one a third-party `catch_unwind` will swallow. So it only *records* a pending artifact
//! (never aborts), and correlation is resolved afterward:
//!
//! - day-core contains it → [`observe_contained`] runs on the same thread, synchronously, right
//!   after the catch, and renames this thread's just-written `pending-*` to `contained-*`.
//! - the process actually dies → the `pending-*` survives and next-launch reconcile finalizes it
//!   as a fatal panic (the sentinel is still present because WillTerminate never ran).

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

thread_local! {
    /// The `pending-*` file this thread's most recent panic wrote, so a same-thread containment
    /// can downgrade exactly that record.
    static LAST_PENDING: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Install the chained panic hook. Idempotent; preserves the previous hook (default stderr print,
/// or an earlier reporter) so day-break composes rather than replaces.
pub(crate) fn install() {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Never let the reporter's own failure mask the panic — record best-effort, then chain.
        record(info);
        prev(info);
    }));
}

fn record(info: &std::panic::PanicHookInfo<'_>) {
    let Some(state) = crate::state() else { return };

    // day-core's downcast idiom (crates/day-core/src/lib.rs).
    let mut message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_string());
    state.redact(&mut message);

    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_default();
    let thread = std::thread::current()
        .name()
        .unwrap_or("unnamed")
        .to_string();
    let backtrace = std::backtrace::Backtrace::force_capture().to_string();
    let uptime_ms = state.uptime_ms();

    let seq = state.next_seq();
    let fields = [
        ("message", message.clone()),
        ("location", location.clone()),
        ("thread", thread.clone()),
        ("uptime_ms", uptime_ms.to_string()),
        ("backtrace", backtrace),
    ];
    if let Ok(path) = crate::store::write_pending(&state.dir, &state.sid, seq, &fields) {
        LAST_PENDING.with(|p| *p.borrow_mut() = Some(path));
    }

    // §8.5 reporter hook — panic context is ordinary code; callbacks may allocate.
    let info = crate::CrashInfo {
        message,
        location,
        thread,
    };
    for cb in state.on_crash_snapshot() {
        cb(&info);
    }
}

/// Registered with day-core via [`day_core::set_contained_panic_observer`]; runs on the panicking
/// thread after day-core catches and resets. Downgrades this thread's pending panic to a
/// non-fatal `contained` record — the app is still alive.
pub(crate) fn observe_contained() {
    LAST_PENDING.with(|p| {
        if let Some(path) = p.borrow_mut().take() {
            crate::store::downgrade_to_contained(&path);
        }
    });
}
