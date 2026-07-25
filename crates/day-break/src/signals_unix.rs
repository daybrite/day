//! POSIX signal handlers for native faults (SIGSEGV/SIGBUS/SIGILL/SIGFPE/SIGABRT/SIGTRAP).
//!
//! A signal handler runs in an async-signal-safe context: no allocation, no locks, no `libc`
//! calls beyond the async-signal-safe set (`write`, `fsync`, `clock_gettime`, `sigaction`,
//! `raise`). So EVERYTHING risky is done at [`install`] time (normal context): the report file is
//! opened, the alternate stack is allocated, the ASLR slide and monotonic epoch are captured, and
//! previous dispositions are saved. The [`handler`] only formats integers into a fixed stack
//! buffer and `write(2)`s them, then **chains**: it restores the previous handler and either
//! re-raises (abort/trap) or returns so the faulting instruction re-executes and the kernel
//! redelivers to the restored handler (the Breakpad protocol) — preserving ART's libsigchain on
//! Android and HiviewDFX's FaultLoggerd on OHOS so their tombstones/faultlogs still generate.

#![cfg(unix)]

use std::cell::UnsafeCell;
use std::ffi::CString;
use std::mem::MaybeUninit;
use std::path::Path;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU64, AtomicUsize, Ordering};

/// The signals we handle, in a fixed order that indexes [`PREV`].
const SIGS: [i32; 6] = [
    libc::SIGSEGV,
    libc::SIGBUS,
    libc::SIGILL,
    libc::SIGFPE,
    libc::SIGABRT,
    libc::SIGTRAP,
];

static CRASH_FD: AtomicI32 = AtomicI32::new(-1);
static HANDLING: AtomicBool = AtomicBool::new(false);
static SLIDE: AtomicUsize = AtomicUsize::new(0);
static MONO_START_NS: AtomicU64 = AtomicU64::new(0);
static MAIN_TID: AtomicI64 = AtomicI64::new(0);

/// Saved previous dispositions, parallel to [`SIGS`]. `UnsafeCell` behind a `Sync` wrapper: written
/// once at install (single-threaded init), read only from the handler after being restored.
struct Prev([UnsafeCell<MaybeUninit<libc::sigaction>>; 6]);
// SAFETY: written once at install before any handler can fire; the handler only reads its own slot.
unsafe impl Sync for Prev {}
static PREV: Prev = Prev([const { UnsafeCell::new(MaybeUninit::uninit()) }; 6]);

/// Install the handlers. `sig_file` is the pre-opened per-session raw-record path. Safe to call
/// once at init; a second call is a no-op (fd already set).
pub(crate) fn install(sig_file: &Path) {
    if CRASH_FD.load(Ordering::Acquire) >= 0 {
        return;
    }
    // Pre-open the raw record (normal context; CString alloc is fine here).
    let Ok(cpath) = CString::new(sig_file.as_os_str().to_string_lossy().as_bytes()) else {
        return;
    };
    // SAFETY: standard open(2); flags are constants, path is a valid NUL-terminated C string.
    let fd = unsafe {
        libc::open(
            cpath.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return;
    }
    CRASH_FD.store(fd, Ordering::Release);

    MONO_START_NS.store(mono_ns(), Ordering::Release);
    MAIN_TID.store(raw_tid(), Ordering::Release);
    SLIDE.store(load_slide(), Ordering::Release);

    // SAFETY: sigaltstack + sigaction with valid, fully-initialized structs; handler is a valid
    // extern "C" fn; PREV slots are written exactly once, here.
    unsafe {
        // A dedicated alt stack so a fault on an overflowed thread stack can still run us.
        let size = 64 * 1024usize;
        let mem = libc::mmap(
            null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        );
        if mem != libc::MAP_FAILED {
            let ss = libc::stack_t {
                ss_sp: mem,
                ss_flags: 0,
                ss_size: size,
            };
            libc::sigaltstack(&ss, null_mut());
        }

        let mut sa: libc::sigaction = std::mem::zeroed();
        // Cast through a fn POINTER first (not the zero-sized fn item) before the integer cast.
        let h: extern "C" fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void) = handler;
        sa.sa_sigaction = h as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
        libc::sigfillset(&mut sa.sa_mask);
        for (i, &sig) in SIGS.iter().enumerate() {
            let mut prev: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(sig, &sa, &mut prev) == 0 {
                *PREV.0[i].get() = MaybeUninit::new(prev);
            }
        }
    }
}

extern "C" fn handler(sig: libc::c_int, info: *mut libc::siginfo_t, _uc: *mut libc::c_void) {
    // A nested fault while we're mid-record: don't touch the file again — restore and re-raise.
    if HANDLING.swap(true, Ordering::SeqCst) {
        unsafe { chain(sig) };
        return;
    }
    let fd = CRASH_FD.load(Ordering::Acquire);
    if fd >= 0 {
        let mut b = Buf::new();
        b.kv_i(b"sig=", sig as i64);
        // SAFETY: the kernel hands us a valid siginfo for SA_SIGINFO handlers.
        unsafe {
            b.kv_i(b"code=", (*info).si_code as i64);
            b.kv_u(b"addr=", si_addr(info) as u64);
        }
        b.kv_u(b"pc=", pc_from_ucontext(_uc) as u64);
        b.kv_u(b"slide=", SLIDE.load(Ordering::Relaxed) as u64);
        let tid = raw_tid();
        b.kv_i(b"tid=", tid);
        b.kv_i(b"main=", (tid == MAIN_TID.load(Ordering::Relaxed)) as i64);
        b.kv_u(b"up_ms=", uptime_ms());
        // SAFETY: fd is a valid open descriptor; write/fsync are async-signal-safe.
        unsafe {
            write_all(fd, b.as_bytes());
            libc::fsync(fd);
        }
    }
    // SAFETY: restores the saved disposition and re-raises / returns per the chaining protocol.
    unsafe { chain(sig) };
}

/// Restore the previous disposition and continue the crash: re-raise for non-restarting signals,
/// return for faults (the instruction re-executes and the kernel redelivers to the restored
/// handler — the system crash reporter then runs).
unsafe fn chain(sig: libc::c_int) {
    if let Some(idx) = SIGS.iter().position(|&s| s == sig) {
        let prev = unsafe { (*PREV.0[idx].get()).assume_init_ref() };
        unsafe { libc::sigaction(sig, prev, null_mut()) };
    } else {
        // Unknown signal — fall back to default.
        let mut dfl: libc::sigaction = unsafe { std::mem::zeroed() };
        dfl.sa_sigaction = libc::SIG_DFL;
        unsafe { libc::sigaction(sig, &dfl, null_mut()) };
    }
    match sig {
        libc::SIGABRT | libc::SIGTRAP => {
            // Not instruction-restarting: unblock and re-raise so the restored handler sees it.
            let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
            unsafe {
                libc::sigemptyset(&mut set);
                libc::sigaddset(&mut set, sig);
                libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, null_mut());
                libc::raise(sig);
            }
        }
        _ => { /* return: faulting instruction re-executes → kernel redelivers to restored handler */
        }
    }
}

// ---- async-signal-safe formatting ----------------------------------------------------------

/// A fixed stack buffer that only appends bytes and base-10 integers — pure memory ops, no alloc.
struct Buf {
    data: [u8; 512],
    len: usize,
}

impl Buf {
    fn new() -> Buf {
        Buf {
            data: [0; 512],
            len: 0,
        }
    }
    fn push(&mut self, bytes: &[u8]) {
        for &c in bytes {
            if self.len < self.data.len() {
                self.data[self.len] = c;
                self.len += 1;
            }
        }
    }
    fn push_u(&mut self, mut v: u64) {
        let mut tmp = [0u8; 20];
        let mut i = tmp.len();
        if v == 0 {
            self.push(b"0");
            return;
        }
        while v > 0 {
            i -= 1;
            tmp[i] = b'0' + (v % 10) as u8;
            v /= 10;
        }
        self.push(&tmp[i..]);
    }
    fn kv_u(&mut self, key: &[u8], v: u64) {
        self.push(key);
        self.push_u(v);
        self.push(b"\n");
    }
    fn kv_i(&mut self, key: &[u8], v: i64) {
        self.push(key);
        if v < 0 {
            self.push(b"-");
            self.push_u(v.unsigned_abs());
        } else {
            self.push_u(v as u64);
        }
        self.push(b"\n");
    }
    fn as_bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }
}

unsafe fn write_all(fd: libc::c_int, mut buf: &[u8]) {
    while !buf.is_empty() {
        let n = unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len()) };
        if n <= 0 {
            // EINTR → retry; anything else → give up (we're crashing anyway).
            if n < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            break;
        }
        buf = &buf[n as usize..];
    }
}

// ---- per-platform primitives ---------------------------------------------------------------

unsafe fn si_addr(info: *mut libc::siginfo_t) -> *mut libc::c_void {
    // libc exposes si_addr() as an accessor on Linux and Apple targets.
    unsafe { (*info).si_addr() }
}

fn mono_ns() -> u64 {
    let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
    // SAFETY: valid timespec out-param; CLOCK_MONOTONIC is async-signal-safe.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    (ts.tv_sec as u64) * 1_000_000_000 + (ts.tv_nsec as u64)
}

fn uptime_ms() -> u64 {
    let start = MONO_START_NS.load(Ordering::Relaxed);
    let now = mono_ns();
    now.saturating_sub(start) / 1_000_000
}

fn raw_tid() -> i64 {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // SAFETY: gettid is always available on Linux/Android and never fails.
        unsafe { libc::syscall(libc::SYS_gettid) as i64 }
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        let mut tid: u64 = 0;
        // SAFETY: null pthread_t means the current thread; valid out-param.
        unsafe { libc::pthread_threadid_np(0, &mut tid) };
        tid as i64
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    )))]
    {
        0
    }
}

/// The main program's ASLR load slide, so `pc - slide` symbolizes offline. Captured at init to keep
/// the handler free of `dl_iterate_phdr`/`dyld` (both take locks).
fn load_slide() -> usize {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        extern "C" fn cb(
            info: *mut libc::dl_phdr_info,
            _size: libc::size_t,
            data: *mut libc::c_void,
        ) -> libc::c_int {
            // First callback is the main program; record its base and stop.
            unsafe { *(data as *mut usize) = (*info).dlpi_addr as usize };
            1
        }
        let mut base: usize = 0;
        // SAFETY: standard dl_iterate_phdr with a matching callback + out-param.
        unsafe { libc::dl_iterate_phdr(Some(cb), &mut base as *mut usize as *mut libc::c_void) };
        base
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        0
    }
}

/// Program counter at the fault, extracted from the ucontext. Linux/Android x86_64 + aarch64 are
/// supported; other targets return 0 (the address is a nice-to-have, not required).
#[allow(unused_variables)]
fn pc_from_ucontext(uc: *mut libc::c_void) -> usize {
    #[cfg(all(
        any(target_os = "linux", target_os = "android"),
        target_arch = "x86_64"
    ))]
    unsafe {
        let uc = uc as *mut libc::ucontext_t;
        (*uc).uc_mcontext.gregs[libc::REG_RIP as usize] as usize
    }
    #[cfg(all(
        any(target_os = "linux", target_os = "android"),
        target_arch = "aarch64"
    ))]
    unsafe {
        let uc = uc as *mut libc::ucontext_t;
        (*uc).uc_mcontext.pc as usize
    }
    #[cfg(not(all(
        any(target_os = "linux", target_os = "android"),
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    {
        0
    }
}
