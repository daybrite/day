//! Child-process cleanup on Ctrl-C. `day launch` streams logs from helper processes
//! (the desktop app itself, `simctl launch --console`, `adb logcat`); if the user
//! interrupts, those must not be left running. We register each child's PID and, on
//! SIGINT/SIGTERM, kill them all before exiting.
//!
//! The signal handler only writes one byte to a self-pipe (async-signal-safe); a watcher
//! thread does the actual killing (locks, `kill`, exit) off the signal context.

use std::sync::Mutex;
use std::sync::OnceLock;

static CHILDREN: Mutex<Vec<i32>> = Mutex::new(Vec::new());
static WAKE_WRITE_FD: OnceLock<i32> = OnceLock::new();

/// Track a spawned child so it is killed on interrupt (and by [`kill_all`]).
pub fn register_child(pid: u32) {
    if let Ok(mut c) = CHILDREN.lock() {
        c.push(pid as i32);
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
        // SAFETY: kill(2) with a previously-tracked child pid; SIGTERM lets it clean up.
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
    }
}

/// Install SIGINT/SIGTERM handlers that kill tracked children then exit 130. Idempotent.
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
    unsafe {
        libc::signal(libc::SIGINT, handle_signal as usize);
        libc::signal(libc::SIGTERM, handle_signal as usize);
    }

    std::thread::spawn(move || {
        let mut buf = [0u8; 1];
        // SAFETY: blocking read on the self-pipe read end.
        let _ = unsafe { libc::read(read_fd, buf.as_mut_ptr() as *mut _, 1) };
        kill_all();
        std::process::exit(130); // 128 + SIGINT
    });
}

extern "C" fn handle_signal(_sig: i32) {
    if let Some(&fd) = WAKE_WRITE_FD.get() {
        let byte = [1u8];
        // SAFETY: write(2) is async-signal-safe.
        unsafe {
            libc::write(fd, byte.as_ptr() as *const _, 1);
        }
    }
}
