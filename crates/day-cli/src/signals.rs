// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Cleanup on Ctrl-C. `day launch` streams logs from helper processes (the desktop app itself,
//! `simctl launch --console`, `adb logcat`); if the user interrupts, those must not be left
//! running. We register each child's PID and kill them all on interrupt (and on the normal-exit
//! path).
//!
//! A launch on a device, emulator or simulator has a second half that no pid reaches: the app
//! itself is not a child of this process. Those register a STOP COMMAND instead
//! ([`register_remote_stop`]), so Ctrl-C ends the app on the device the way it ends a desktop
//! app — rather than only taking its logs away.
//!
//! Unix does the real work: a SIGINT/SIGTERM handler writes one byte to a self-pipe
//! (async-signal-safe); a watcher thread does the killing off the signal context. On
//! Windows the console already delivers Ctrl-C to every process in the group, and Day's
//! only Windows backend (xaml) has no `adb`/`simctl` log watchers, so `install` is a
//! no-op there and `kill_all` (used on the normal-exit path) terminates by PID.

use std::sync::Mutex;
#[cfg(unix)]
use std::sync::OnceLock;

/// A tracked child, and whether it is the APP itself. A desktop launch spawns the app as a child
/// of this process; everything else tracked here is a helper `day` owns outright — a log pump, the
/// web driver. Only the app is something a run can be asked to leave behind.
struct Child {
    pid: u32,
    is_app: bool,
}

static CHILDREN: Mutex<Vec<Child>> = Mutex::new(Vec::new());
static REMOTE_STOPS: Mutex<Vec<Vec<String>>> = Mutex::new(Vec::new());
/// Files to put back on the way out: path → contents before this run touched it, or `None` when
/// the file did not exist. `--day-src` registers `Cargo.lock` here, because cargo rewrites it to
/// record the patched sources and a flag that promises to change nothing must not leave a tracked
/// file modified by an interrupt (`crate::patch`).
type Restore = (std::path::PathBuf, Option<Vec<u8>>);
static RESTORES: Mutex<Vec<Restore>> = Mutex::new(Vec::new());
/// Held for the whole of [`kill_all`]; see there.
static TEARDOWN: Mutex<()> = Mutex::new(());

/// Restore `path` to `before` if this process is interrupted. Paired with [`forget_restore`],
/// which the normal path calls once it has done the restoring itself.
pub fn register_restore(path: &std::path::Path, before: Option<&[u8]>) {
    if let Ok(mut r) = RESTORES.lock() {
        r.retain(|(p, _)| p != path);
        r.push((path.to_path_buf(), before.map(<[u8]>::to_vec)));
    }
}

/// Drop a registration — the guard restored the file itself on the way out of the build.
pub fn forget_restore(path: &std::path::Path) {
    if let Ok(mut r) = RESTORES.lock() {
        r.retain(|(p, _)| p != path);
    }
}

/// Track a spawned child so it is killed on interrupt (and by [`kill_all`]).
pub fn register_child(pid: u32) {
    if let Ok(mut c) = CHILDREN.lock() {
        c.push(Child { pid, is_app: false });
    }
}

/// Track the APP itself, spawned as a child by a desktop launch. Killed like any other child on
/// interrupt and on the normal-exit path — unless [`forget_app_children`] spares it first.
pub fn register_app_child(pid: u32) {
    if let Ok(mut c) = CHILDREN.lock() {
        c.push(Child { pid, is_app: true });
    }
}

/// The most recently spawned child's pid — the app itself on a desktop launch. The crash
/// post-mortem uses it to pick THIS run's report out of a directory where every build of the app
/// files under the same process name (`crate::diagnose`).
pub fn last_child() -> Option<u32> {
    CHILDREN.lock().ok().and_then(|c| c.last().map(|c| c.pid))
}

/// Track an app that is NOT a child of this process — one running on a device, emulator or
/// simulator — as the command line that stops it (`adb shell am force-stop <id>`).
///
/// Killing pids cannot reach these. The only host-side process a device launch owns is the log
/// pump, so Ctrl-C used to take the logs away and leave the app running on the device, which is
/// the opposite of what the same keystroke does to a desktop app. Registered per launch, and run
/// on interrupt and on the normal-exit path alike.
pub fn register_remote_stop(argv: Vec<String>) {
    if let Ok(mut r) = REMOTE_STOPS.lock() {
        r.push(argv);
    }
}

/// Drop the registered device stops without running them — for a run that DELIBERATELY leaves the
/// app up (`--keep-alive`, whose whole promise is that the app outlives `day`). Without this the
/// normal-exit `kill_all` would honor the interrupt contract over the explicit flag and stop the
/// app anyway. The log-pump children are still reaped; only the app is spared.
pub fn forget_remote_stops() {
    if let Ok(mut r) = REMOTE_STOPS.lock() {
        r.clear();
    }
}

/// Drop the APP from the kill list without stopping it — for a run that DELIBERATELY leaves it up
/// (`--keep-alive`, whose whole promise is that the app outlives `day`). [`forget_remote_stops`] is
/// the device half of that promise; this is the desktop half, where the app is a CHILD of this
/// process and the normal-exit `kill_all` would otherwise terminate the very thing the flag exists
/// to keep — reporting "left running" over a window that had just closed.
///
/// Helpers stay registered and are still reaped: an orphaned log pump holds the inherited stdout
/// that CI then waits on forever.
pub fn forget_app_children() {
    if let Ok(mut c) = CHILDREN.lock() {
        c.retain(|c| !c.is_app);
    }
}

/// Stop everything this `day` invocation started: tracked children, then the apps running on
/// devices. Used on the normal-exit path too, so log watchers for a target that has finished
/// don't linger while other targets run.
pub fn kill_all() {
    // Serialized, because two threads reach here on an interrupt: the signal watcher, and the
    // main thread the moment killing the log pump lets its `join` return. Without this the main
    // thread wins, returns from `main`, and `process::exit` tears down the watcher — mid
    // `adb force-stop`, so the app on the device outlived the Ctrl-C that was meant to end it.
    // Whoever arrives second now waits for the first to finish rather than exiting out from
    // under it.
    let _teardown = TEARDOWN.lock().unwrap_or_else(|e| e.into_inner());
    let pids = CHILDREN
        .lock()
        .map(|mut c| std::mem::take(&mut *c))
        .unwrap_or_default();
    for child in pids {
        kill_one(child.pid);
    }
    // After the children, so the log pump is already gone and the stop itself does not get
    // streamed back as app output.
    let stops = REMOTE_STOPS
        .lock()
        .map(|mut r| std::mem::take(&mut *r))
        .unwrap_or_default();
    for argv in stops {
        if let Some((exe, args)) = argv.split_first() {
            let _ = std::process::Command::new(exe)
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
    // Last, once nothing is still running that could write the file again: a cargo killed
    // mid-build may still be flushing its lockfile while its pid is being signaled.
    let restores = RESTORES
        .lock()
        .map(|mut r| std::mem::take(&mut *r))
        .unwrap_or_default();
    for (path, before) in restores {
        crate::patch::restore(&path, before.as_deref());
    }
}

#[cfg(unix)]
fn kill_one(pid: u32) {
    // SAFETY: kill(2) with a previously-tracked child pid; SIGTERM lets it clean up.
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
}

#[cfg(not(unix))]
fn kill_one(pid: u32) {
    // No POSIX signals; terminate the child (and its tree) by pid.
    let _ = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .status();
}

#[cfg(unix)]
static WAKE_WRITE_FD: OnceLock<i32> = OnceLock::new();

/// Install interrupt handling that kills tracked children then exits. Idempotent.
///
/// Unix: SIGINT/SIGTERM → self-pipe → watcher thread → [`kill_all`] → exit 130.
#[cfg(unix)]
pub fn install() {
    static DONE: OnceLock<()> = OnceLock::new();
    if DONE.set(()).is_err() {
        return;
    }

    let mut fds = [0i32; 2];
    // SAFETY: standard self-pipe construction.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return;
    }
    let (read_fd, write_fd) = (fds[0], fds[1]);
    let _ = WAKE_WRITE_FD.set(write_fd);

    // SAFETY: install async-signal-safe handler (only writes a byte to the pipe).
    let handler = handle_signal as *const () as usize;
    unsafe {
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }

    std::thread::spawn(move || {
        let mut buf = [0u8; 1];
        // SAFETY: blocking read on the self-pipe read end.
        let _ = unsafe { libc::read(read_fd, buf.as_mut_ptr() as *mut _, 1) };
        kill_all();
        std::process::exit(130); // 128 + SIGINT
    });
}

/// Windows: the console delivers Ctrl-C to every child in the group already, and there
/// are no log-watcher children on this platform, so there is nothing to install; the
/// normal-exit `kill_all` still reaps any tracked child.
#[cfg(not(unix))]
pub fn install() {}

#[cfg(unix)]
extern "C" fn handle_signal(_sig: i32) {
    if let Some(&fd) = WAKE_WRITE_FD.get() {
        let byte = [1u8];
        // SAFETY: write(2) is async-signal-safe.
        unsafe {
            libc::write(fd, byte.as_ptr() as *const _, 1);
        }
    }
}
