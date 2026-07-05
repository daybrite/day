//! Child-process cleanup on Ctrl-C. `day launch` streams logs from helper processes
//! (the desktop app itself, `simctl launch --console`, `adb logcat`); if the user
//! interrupts, those must not be left running. We register each child's PID and kill them
//! all on interrupt (and on the normal-exit path).
//!
//! Unix does the real work: a SIGINT/SIGTERM handler writes one byte to a self-pipe
//! (async-signal-safe); a watcher thread does the killing off the signal context. On
//! Windows the console already delivers Ctrl-C to every process in the group, and Day's
//! only Windows backend (winui) has no `adb`/`simctl` log watchers, so `install` is a
//! no-op there and `kill_all` (used on the normal-exit path) terminates by PID.

use std::sync::Mutex;
#[cfg(unix)]
use std::sync::OnceLock;

static CHILDREN: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// Track a spawned child so it is killed on interrupt (and by [`kill_all`]).
pub fn register_child(pid: u32) {
    if let Ok(mut c) = CHILDREN.lock() {
        c.push(pid);
    }
}

/// Kill every tracked child now (used on the normal-exit path too, so log watchers for
/// a target that has finished don't linger while other targets run).
pub fn kill_all() {
    let pids = CHILDREN
        .lock()
        .map(|mut c| std::mem::take(&mut *c))
        .unwrap_or_default();
    for pid in pids {
        kill_one(pid);
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
