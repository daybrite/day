// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Native fault capture on Windows — the counterpart to [`crate::signals_unix`].
//!
//! Without this, a native crash on Windows left NOTHING behind: the panic hook only sees Rust
//! panics, so an access violation or an `abort()` produced a report carrying the session metadata
//! written at startup (pid, app id, versions) and not one word about the crash. That is what a
//! walkthrough failure looked like in CI — "the app crashed", a pid, and no reason.
//!
//! Two entry points, because Windows splits them:
//!
//! * `SetUnhandledExceptionFilter` — structured exceptions (access violation, illegal
//!   instruction, divide-by-zero, stack overflow). The last filter standing before the OS's own
//!   error reporting.
//! * a `SIGABRT` handler — `abort()` unwinds through the CRT, not through SEH, so the filter above
//!   never sees it.
//!
//! Neither catches `__fastfail` (what `RaiseFailFastException` and Rust's own
//! `core::intrinsics::abort` use): it is designed to bypass every user-mode hook and go straight
//! to the kernel. A crash through that route still records nothing, by the platform's design.
//!
//! The record is the SAME `key=value` file the POSIX handler writes, and `sig=` carries the POSIX
//! signal number the exception corresponds to — so `store::signal_name` and the whole report
//! pipeline read a Windows crash with no changes. The raw `NTSTATUS` rides in `code=`, where the
//! POSIX handler puts `si_code`, because on Windows that is the number worth reading.
//!
//! Handler discipline mirrors the POSIX one: everything that can allocate or take a lock happens
//! at [`install`] time, and the handler only formats integers into a fixed stack buffer and writes
//! them. That matters more here than usual — one of the exceptions we catch is a stack overflow,
//! where the faulting thread has a single guard page left to work with.

#![cfg(windows)]

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, AtomicUsize, Ordering};

// ---- Win32 surface ---------------------------------------------------------------------------
// Declared inline rather than pulling in a bindings crate: day-break is a leaf that an app opts
// into, and this is a dozen symbols.

#[repr(C)]
struct ExceptionRecord {
    exception_code: u32,
    exception_flags: u32,
    exception_record: *mut ExceptionRecord,
    exception_address: *mut core::ffi::c_void,
    number_parameters: u32,
    exception_information: [usize; 15],
}

#[repr(C)]
struct ExceptionPointers {
    exception_record: *mut ExceptionRecord,
    context_record: *mut core::ffi::c_void,
}

type Filter = unsafe extern "system" fn(*mut ExceptionPointers) -> i32;

unsafe extern "system" {
    fn SetUnhandledExceptionFilter(filter: Option<Filter>) -> Option<Filter>;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *mut core::ffi::c_void,
        disposition: u32,
        flags: u32,
        template: isize,
    ) -> isize;
    fn WriteFile(
        file: isize,
        buf: *const u8,
        len: u32,
        written: *mut u32,
        overlapped: *mut core::ffi::c_void,
    ) -> i32;
    fn FlushFileBuffers(file: isize) -> i32;
    fn GetCurrentThreadId() -> u32;
    fn GetModuleHandleW(name: *const u16) -> isize;
    fn GetTickCount64() -> u64;
}

// The CRT's own signal seam: `abort()` raises SIGABRT rather than an SEH exception.
unsafe extern "C" {
    fn signal(sig: i32, handler: usize) -> usize;
    fn raise(sig: i32) -> i32;
}

const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 0x0000_0001;
const CREATE_ALWAYS: u32 = 2;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;
const INVALID_HANDLE_VALUE: isize = -1;
/// Let the OS carry on to its default handling (WER) — the chaining the POSIX handler does by
/// restoring the previous disposition.
const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

const SIGABRT: i32 = 22; // MSVC CRT's value, NOT POSIX 6
const SIG_DFL: usize = 0;

// POSIX signal numbers, so the existing `store::signal_name` and report schema apply unchanged.
const POSIX_SIGILL: i64 = 4;
const POSIX_SIGABRT: i64 = 6;
const POSIX_SIGBUS: i64 = 7;
const POSIX_SIGFPE: i64 = 8;
const POSIX_SIGSEGV: i64 = 11;
const POSIX_SIGTRAP: i64 = 5;

static CRASH_FILE: AtomicIsize = AtomicIsize::new(INVALID_HANDLE_VALUE);
static HANDLING: AtomicBool = AtomicBool::new(false);
static MODULE_BASE: AtomicUsize = AtomicUsize::new(0);
static START_MS: AtomicU64 = AtomicU64::new(0);
static MAIN_TID: AtomicU32 = AtomicU32::new(0);

/// Install the handlers. `sig_file` is the per-session raw-record path, pre-opened here so the
/// handler never has to. Safe to call once at init; a second call is a no-op.
pub(crate) fn install(sig_file: &Path) {
    if CRASH_FILE.load(Ordering::Acquire) != INVALID_HANDLE_VALUE {
        return;
    }
    let mut wide: Vec<u16> = sig_file.as_os_str().encode_wide().collect();
    wide.push(0);
    // SAFETY: a valid NUL-terminated wide path; the rest are constants.
    let file = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ,
            std::ptr::null_mut(),
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            0,
        )
    };
    if file == INVALID_HANDLE_VALUE {
        return;
    }
    CRASH_FILE.store(file, Ordering::Release);

    // SAFETY: all four are parameterless or take a null module name (= this executable).
    unsafe {
        START_MS.store(GetTickCount64(), Ordering::Release);
        MAIN_TID.store(GetCurrentThreadId(), Ordering::Release);
        MODULE_BASE.store(
            GetModuleHandleW(std::ptr::null()) as usize,
            Ordering::Release,
        );
        SetUnhandledExceptionFilter(Some(seh_filter));
        // Through a fn POINTER first: casting the zero-sized fn ITEM straight to an integer is a
        // different (and lint-rejected) operation. Same dance as the POSIX handler's sa_sigaction.
        let h: extern "C" fn(i32) = abort_handler;
        signal(SIGABRT, h as usize);
    }
}

/// Structured exceptions. Returns CONTINUE_SEARCH so Windows Error Reporting still runs — the
/// record is an addition to the platform's own handling, never a replacement for it.
unsafe extern "system" fn seh_filter(info: *mut ExceptionPointers) -> i32 {
    if HANDLING.swap(true, Ordering::SeqCst) {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    // SAFETY: the OS hands the filter a valid ExceptionPointers with a non-null record.
    unsafe {
        if info.is_null() || (*info).exception_record.is_null() {
            return EXCEPTION_CONTINUE_SEARCH;
        }
        let rec = (*info).exception_record;
        let code = (*rec).exception_code;
        // An access violation's second parameter is the address that was touched; for everything
        // else the faulting instruction is the best address we have.
        let addr = if code == 0xC000_0005 && (*rec).number_parameters >= 2 {
            (*rec).exception_information[1]
        } else {
            (*rec).exception_address as usize
        };
        write_record(
            posix_signo(code),
            code as i64,
            addr,
            (*rec).exception_address as usize,
        );
    }
    EXCEPTION_CONTINUE_SEARCH
}

/// `abort()` — the CRT path, which never reaches the SEH filter above.
extern "C" fn abort_handler(_sig: i32) {
    if !HANDLING.swap(true, Ordering::SeqCst) {
        write_record(POSIX_SIGABRT, 0, 0, 0);
    }
    // Chain: restore the default and re-raise, so the CRT's own abort (and WER behind it) still
    // happens. Returning here would let the CRT continue aborting anyway, but re-raising keeps the
    // shape identical to the POSIX handler's contract.
    // SAFETY: restoring SIG_DFL and re-raising is the documented way to defer to default handling.
    unsafe {
        signal(SIGABRT, SIG_DFL);
        raise(SIGABRT);
    }
}

/// The POSIX signal an exception corresponds to, so one reader serves both platforms.
fn posix_signo(code: u32) -> i64 {
    match code {
        0xC000_0005 => POSIX_SIGSEGV,              // ACCESS_VIOLATION
        0xC000_00FD => POSIX_SIGSEGV,              // STACK_OVERFLOW — a fault on the guard page
        0x8000_0002 => POSIX_SIGBUS,               // DATATYPE_MISALIGNMENT
        0xC000_001D => POSIX_SIGILL,               // ILLEGAL_INSTRUCTION
        0xC000_001E => POSIX_SIGILL,               // INVALID_DISPOSITION
        0xC000_0094..=0xC000_009A => POSIX_SIGFPE, // the INT_/FLT_ arithmetic family
        0x8000_0003 => POSIX_SIGTRAP,              // BREAKPOINT
        0x8000_0004 => POSIX_SIGTRAP,              // SINGLE_STEP
        _ => POSIX_SIGSEGV, // unknown fault: SEGV is the least misleading default
    }
}

fn write_record(signo: i64, code: i64, addr: usize, pc: usize) {
    let file = CRASH_FILE.load(Ordering::Acquire);
    if file == INVALID_HANDLE_VALUE {
        return;
    }
    let mut b = Buf::new();
    b.kv_i(b"sig=", signo);
    b.kv_i(b"code=", code);
    b.kv_u(b"addr=", addr as u64);
    b.kv_u(b"pc=", pc as u64);
    b.kv_u(b"slide=", MODULE_BASE.load(Ordering::Relaxed) as u64);
    // SAFETY: parameterless Win32 query.
    let tid = unsafe { GetCurrentThreadId() };
    b.kv_u(b"tid=", tid as u64);
    b.kv_i(b"main=", (tid == MAIN_TID.load(Ordering::Relaxed)) as i64);
    // SAFETY: parameterless Win32 query; saturating so a wrapped tick count cannot underflow.
    let now = unsafe { GetTickCount64() };
    b.kv_u(
        b"up_ms=",
        now.saturating_sub(START_MS.load(Ordering::Relaxed)),
    );
    // SAFETY: `file` is a valid handle owned for the process's life; the buffer outlives the call.
    unsafe {
        let bytes = b.as_bytes();
        let mut written: u32 = 0;
        WriteFile(
            file,
            bytes.as_ptr(),
            bytes.len() as u32,
            &mut written,
            std::ptr::null_mut(),
        );
        FlushFileBuffers(file);
    }
}

// ---- handler-safe formatting -----------------------------------------------------------------
// The POSIX file's `Buf`, kept here rather than shared: that one is `#![cfg(unix)]` and this must
// not depend on it, and the type is 40 lines of pure byte pushing with no platform in it.

/// A fixed stack buffer that only appends bytes and base-10 integers — no allocation, no locks.
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
        if v == 0 {
            self.push(b"0");
            return;
        }
        let mut tmp = [0u8; 20];
        let mut i = tmp.len();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exception_codes_map_to_the_posix_signal_the_report_names() {
        assert_eq!(posix_signo(0xC000_0005), POSIX_SIGSEGV); // access violation
        assert_eq!(posix_signo(0xC000_00FD), POSIX_SIGSEGV); // stack overflow
        assert_eq!(posix_signo(0xC000_001D), POSIX_SIGILL);
        assert_eq!(posix_signo(0xC000_0094), POSIX_SIGFPE); // int divide by zero
        assert_eq!(posix_signo(0x8000_0003), POSIX_SIGTRAP);
        assert_eq!(posix_signo(0x8000_0002), POSIX_SIGBUS);
        // Anything unrecognized still reports as a fault rather than as "no signal".
        assert_eq!(posix_signo(0xDEAD_BEEF), POSIX_SIGSEGV);
    }

    #[test]
    fn buf_writes_the_key_value_shape_the_store_parses() {
        let mut b = Buf::new();
        b.kv_i(b"sig=", 11);
        b.kv_i(b"code=", -1);
        b.kv_u(b"addr=", 0);
        assert_eq!(b.as_bytes(), b"sig=11\ncode=-1\naddr=0\n");
    }

    #[test]
    fn buf_cannot_overrun_its_fixed_storage() {
        let mut b = Buf::new();
        for _ in 0..200 {
            b.kv_u(b"pc=", u64::MAX);
        }
        assert!(b.as_bytes().len() <= 512);
    }
}
