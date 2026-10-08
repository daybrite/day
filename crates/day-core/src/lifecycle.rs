// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! App-lifecycle callbacks (docs/lifecycle.md). An app registers closures for [`day_spec::Lifecycle`]
//! phases with [`on_lifecycle`]; each backend, at the matching moment in its native app/activity
//! delegate, emits `Event::Lifecycle(phase)` (or day-core dispatches the launch phases uniformly),
//! and the event pump routes it here to run the closures inside a reactive batch, the same rails
//! as `Event::MenuAction`, so a lifecycle handler that writes signals updates the UI like any
//! callback.
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

/// `DidExit` was delivered (it is delivered once). Process-global rather than thread-local:
/// the delivery from `atexit` can run after the thread-locals are gone.
static EXITED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// What the exit line names: the app's display name, its version, and when it launched.
/// Process-global for the same reason.
static LAUNCHED: std::sync::Mutex<Option<Launched>> = std::sync::Mutex::new(None);

struct Launched {
    name: String,
    version: Option<String>,
    #[cfg(not(target_arch = "wasm32"))]
    at: std::time::Instant,
}

/// Record the app's name and version for the exit line, and start its clock. Called by
/// `launch_with` before `WillLaunch`.
pub fn note_launch(name: String, version: Option<String>) {
    if let Ok(mut l) = LAUNCHED.lock() {
        *l = Some(Launched {
            name,
            version,
            #[cfg(not(target_arch = "wasm32"))]
            at: std::time::Instant::now(),
        });
    }
}

/// Day's last line: `Clean exit for <app> v<version> (<profile>) after HH:MM:SS. Memory usage:
/// <n> MB`. The version is the app's (`WindowOptions::version`), else the `DAY_APP_VERSION` the
/// CLI sets on a launch; the profile is this build's; the time is since `launch_with`; the memory
/// is the process's resident set, where the platform tells (`resident_memory`).
fn exit_summary() -> String {
    let (name, version, elapsed) = {
        let l = LAUNCHED.lock().ok();
        let l = l.as_deref().and_then(Option::as_ref);
        let name = l
            .map(|l| l.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "the app".to_string());
        let version = l
            .and_then(|l| l.version.clone())
            .or_else(|| std::env::var("DAY_APP_VERSION").ok())
            .filter(|v| !v.is_empty());
        #[cfg(not(target_arch = "wasm32"))]
        let elapsed = l.map(|l| l.at.elapsed().as_secs());
        #[cfg(target_arch = "wasm32")]
        let elapsed = crate::frame::uptime_secs();
        (name, version, elapsed)
    };
    format_exit_summary(
        &name,
        version.as_deref(),
        cfg!(debug_assertions),
        elapsed,
        resident_memory(),
    )
}

/// The exit line's text; see `exit_summary`. Factored for the test below.
fn format_exit_summary(
    name: &str,
    version: Option<&str>,
    debug: bool,
    elapsed_secs: Option<u64>,
    memory_bytes: Option<u64>,
) -> String {
    let version = match version {
        Some(v) => format!(" v{v}"),
        None => String::new(),
    };
    let profile = if debug { "debug" } else { "release" };
    let elapsed = match elapsed_secs {
        Some(s) => format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60),
        None => "an unknown time".to_string(),
    };
    let memory = match memory_bytes {
        Some(b) => format!("{:.2} MB", b as f64 / 1e6),
        None => "unknown".to_string(),
    };
    format!("Clean exit for {name}{version} ({profile}) after {elapsed}. Memory usage: {memory}")
}

/// The process's resident memory in bytes, from the platform's own accounting: the task's
/// physical footprint on Apple platforms, the resident set from `/proc` on Linux and its
/// relatives, the working set on Windows, the linear memory on wasm. `None` where none is
/// readable, which the exit line reports as unknown.
pub fn resident_memory() -> Option<u64> {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        // task_info(TASK_VM_INFO): `phys_footprint` is what Xcode's memory gauge shows.
        #[repr(C)]
        struct TaskVmInfo {
            // The struct is long; only the fields up to `phys_footprint` matter here, and
            // `task_info` fills as many bytes as asked for.
            virtual_size: u64,
            region_count: i32,
            page_size: i32,
            resident_size: u64,
            resident_size_peak: u64,
            device: u64,
            device_peak: u64,
            internal: u64,
            internal_peak: u64,
            external: u64,
            external_peak: u64,
            reusable: u64,
            reusable_peak: u64,
            purgeable_volatile_pmap: u64,
            purgeable_volatile_resident: u64,
            purgeable_volatile_virtual: u64,
            compressed: u64,
            compressed_peak: u64,
            compressed_lifetime: u64,
            phys_footprint: u64,
        }
        unsafe extern "C" {
            fn mach_task_self() -> u32;
            fn task_info(task: u32, flavor: u32, info: *mut TaskVmInfo, count: *mut u32) -> i32;
        }
        const TASK_VM_INFO: u32 = 22;
        let mut info = std::mem::MaybeUninit::<TaskVmInfo>::zeroed();
        let mut count = (std::mem::size_of::<TaskVmInfo>() / std::mem::size_of::<u32>()) as u32;
        // SAFETY: `task_info` writes at most `count` 32-bit words into `info`, which is sized to
        // hold them, and reports what it wrote back in `count`.
        let ok = unsafe {
            task_info(
                mach_task_self(),
                TASK_VM_INFO,
                info.as_mut_ptr(),
                &mut count,
            )
        };
        if ok != 0 {
            return None;
        }
        // SAFETY: a zero return means the kernel filled the structure.
        let info = unsafe { info.assume_init() };
        return Some(if info.phys_footprint > 0 {
            info.phys_footprint
        } else {
            info.resident_size
        });
    }
    #[cfg(any(target_os = "linux", target_os = "android", target_env = "ohos"))]
    {
        // /proc/self/statm: size resident shared text lib data dt, in pages.
        let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
        let resident: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
        unsafe extern "C" {
            fn sysconf(name: i32) -> i64;
        }
        #[cfg(target_os = "android")]
        const SC_PAGESIZE: i32 = 40;
        #[cfg(not(target_os = "android"))]
        const SC_PAGESIZE: i32 = 30;
        // SAFETY: sysconf reads a system constant and has no preconditions.
        let page = unsafe { sysconf(SC_PAGESIZE) };
        return Some(resident * u64::try_from(page).unwrap_or(4096));
    }
    #[cfg(windows)]
    {
        #[repr(C)]
        struct ProcessMemoryCounters {
            cb: u32,
            page_fault_count: u32,
            peak_working_set_size: usize,
            working_set_size: usize,
            quota_peak_paged_pool_usage: usize,
            quota_paged_pool_usage: usize,
            quota_peak_non_paged_pool_usage: usize,
            quota_non_paged_pool_usage: usize,
            pagefile_usage: usize,
            peak_pagefile_usage: usize,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentProcess() -> isize;
            fn K32GetProcessMemoryInfo(
                process: isize,
                counters: *mut ProcessMemoryCounters,
                cb: u32,
            ) -> i32;
        }
        let mut counters = std::mem::MaybeUninit::<ProcessMemoryCounters>::zeroed();
        let cb = std::mem::size_of::<ProcessMemoryCounters>() as u32;
        // SAFETY: the counters structure is sized by `cb`, and the pseudo-handle needs no
        // closing.
        let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), counters.as_mut_ptr(), cb) };
        if ok == 0 {
            return None;
        }
        // SAFETY: a non-zero return means the structure was filled.
        return Some(unsafe { counters.assume_init() }.working_set_size as u64);
    }
    #[cfg(target_arch = "wasm32")]
    {
        // The linear memory's size in 64 KiB pages: what the module has asked the host for.
        return Some(core::arch::wasm32::memory_size(0) as u64 * 65536);
    }
    #[allow(unreachable_code)]
    None
}

/// Forget every registered handler: a re-mount (docs/appearance.md). The app's `root()` is
/// about to run again and re-register, and handlers left from the previous mount would fire a
/// second time for every phase.
pub fn reset_handlers() {
    HANDLERS.with(|h| h.borrow_mut().clear());
    WARNED.with(|w| w.borrow_mut().clear());
}

/// Register `f` to run whenever the app reaches `phase`. Handlers run in registration order, in a
/// reactive batch (signal writes coalesce into one UI update). Register early (before `launch`, or
/// at the top of the root builder) so `WillLaunch`/`DidLaunch` handlers are in place when they
/// fire.
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
/// `Event::Lifecycle`, and directly by `launch_with` for the launch phases and for `DidExit`.
///
/// `DidExit` is delivered once: the first delivery runs the handlers and then writes Day's exit
/// line (`exit_summary`), and any later one is dropped, since a backend may emit it at its last
/// native moment and `launch_with` emits it again when the loop returns.
pub fn dispatch_lifecycle(phase: Lifecycle) {
    // The last moment the windows still answer for their frames (docs/windows.md
    // "Remembered frames"); a platform that ends the app itself (AppKit's terminate) passes here.
    if phase == Lifecycle::WillTerminate {
        crate::windows::save_remembered_frames();
        crate::present::shutdown_tasks();
    }
    if phase == Lifecycle::DidExit {
        if EXITED.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        // From `atexit` the thread-locals may already be gone (Rust tears them down at exit
        // too, in an order nobody controls); the handlers need them, the line does not.
        if HANDLERS.try_with(|_| ()).is_ok() {
            crate::present::shutdown_tasks();
            dispatch_handlers(phase);
        }
        log::info!("{}", exit_summary());
        return;
    }
    dispatch_handlers(phase);
}

fn dispatch_handlers(phase: Lifecycle) {
    crate::frame::lifecycle(phase);
    let handlers = HANDLERS.with(|h| h.borrow().get(&phase).cloned().unwrap_or_default());
    if handlers.is_empty() {
        return;
    }
    let mut any_panicked = false;
    day_reactive::batch(|| {
        for f in &handlers {
            // Lifecycle callbacks run inside native trampolines (applicationWillTerminate,
            // GApplication::shutdown, the Android onDestroy JNI frame, …) that abort the process on
            // unwind. Contain each handler like the event pump does (DESIGN.md §8.5): a panic here
            // (classically an `eprintln!` hitting a closed stderr pipe during teardown) would
            // otherwise turn a clean exit into a spurious crash. `notify_contained_panic` runs
            // per-panic so a crash reporter (day-piece-break) downgrades that handler's report to
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
    // Reset the reactive runtime after the batch closes: `recover_from_panic` rewrites the batch
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
    fn the_exit_line_reads_as_documented() {
        assert_eq!(
            format_exit_summary(
                "Day Rise",
                Some("0.4.5"),
                true,
                Some(3 * 3600 + 7 * 60 + 9),
                Some(12_345_678)
            ),
            "Clean exit for Day Rise v0.4.5 (debug) after 03:07:09. Memory usage: 12.35 MB"
        );
        assert_eq!(
            format_exit_summary("", None, false, None, None),
            "Clean exit for  (release) after an unknown time. Memory usage: unknown"
        );
    }

    #[test]
    fn resident_memory_is_readable_here() {
        // Every platform the tests run on has a probe; the test process is at least a megabyte.
        assert!(resident_memory().is_some_and(|b| b > 1 << 20));
    }

    #[test]
    fn did_exit_runs_its_handlers_once() {
        thread_local! {
            static N: Cell<u32> = const { Cell::new(0) };
        }
        let task = crate::task(std::future::pending());
        on_lifecycle(Lifecycle::DidExit, move || {
            assert!(task.is_finished());
            N.with(|c| c.set(c.get() + 1));
        });
        dispatch_lifecycle(Lifecycle::DidExit);
        dispatch_lifecycle(Lifecycle::DidExit);
        assert_eq!(N.with(Cell::get), 1);
        EXITED.store(false, std::sync::atomic::Ordering::SeqCst);
    }

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
        // not propagate; dispatch runs inside a native trampoline that would abort on unwind.
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
