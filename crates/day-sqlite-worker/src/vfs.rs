// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The two VFSes behind the engine (docs/persistence.md).
//!
//! **day-mem** is the default: anonymous in-RAM files. `:memory:` databases never open a file
//! at all, but SQLite can still ask the default VFS for spill files in corner cases, and the
//! main-thread app instance must never touch OPFS — so the default answers everything from a
//! crate-private RAM store on every target.
//!
//! **day-opfs** is the worker's VFS: every file operation goes through the ten synchronous
//! primitives in [`fsx`]. On wasm those are `day_sql_fs_*` imports the day-sql worker page
//! implements over pre-opened OPFS sync access handles; on native they are an in-memory fake
//! with the same slot/refcount semantics, which is what lets `cargo test` drive real SQLite
//! through this exact VFS with no browser. Locking is a no-op — one connection per database
//! per worker, and the access handles themselves are origin-exclusive.

use core::ffi::{c_char, c_int, c_void};
use std::cell::RefCell;
use std::collections::HashMap;

use crate::bindings as b;
use crate::os;

/// The synchronous file primitives. One implementation talks to the worker page's OPFS pool,
/// the other to a native fake; the VFS above and the pool verbs in lib.rs see no difference.
pub(crate) mod fsx {
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    mod imp {
        // Implemented by day-sql-worker.js over FileSystemSyncAccessHandle (day-cli resource).
        // The main app instance links these too (same wasm module) but never calls them; its
        // shim provides loud stubs.
        #[link(wasm_import_module = "env")]
        unsafe extern "C" {
            fn day_sql_fs_open(name: *const u8, name_len: usize, create: i32) -> i32;
            fn day_sql_fs_read(slot: u32, off: f64, buf: *mut u8, len: usize) -> i32;
            fn day_sql_fs_write(slot: u32, off: f64, buf: *const u8, len: usize) -> i32;
            fn day_sql_fs_truncate(slot: u32, size: f64) -> i32;
            fn day_sql_fs_size(slot: u32) -> f64;
            fn day_sql_fs_flush(slot: u32) -> i32;
            fn day_sql_fs_close(slot: u32, delete: i32);
            fn day_sql_fs_delete(name: *const u8, name_len: usize) -> i32;
            fn day_sql_fs_exists(name: *const u8, name_len: usize) -> i32;
            fn day_sql_fs_list(buf: *mut u8, cap: usize) -> i32;
        }

        pub fn open(name: &str, create: bool) -> Option<u32> {
            let slot = unsafe { day_sql_fs_open(name.as_ptr(), name.len(), i32::from(create)) };
            u32::try_from(slot).ok()
        }
        pub fn read(slot: u32, off: u64, buf: &mut [u8]) -> Option<usize> {
            let n = unsafe { day_sql_fs_read(slot, off as f64, buf.as_mut_ptr(), buf.len()) };
            usize::try_from(n).ok()
        }
        pub fn write(slot: u32, off: u64, buf: &[u8]) -> bool {
            unsafe { day_sql_fs_write(slot, off as f64, buf.as_ptr(), buf.len()) == 0 }
        }
        pub fn truncate(slot: u32, size: u64) -> bool {
            unsafe { day_sql_fs_truncate(slot, size as f64) == 0 }
        }
        pub fn size(slot: u32) -> Option<u64> {
            let s = unsafe { day_sql_fs_size(slot) };
            if s < 0.0 {
                None
            } else {
                Some(s as u64)
            }
        }
        pub fn flush(slot: u32) -> bool {
            unsafe { day_sql_fs_flush(slot) == 0 }
        }
        pub fn close(slot: u32, delete: bool) {
            unsafe { day_sql_fs_close(slot, i32::from(delete)) }
        }
        pub fn delete(name: &str) -> bool {
            unsafe { day_sql_fs_delete(name.as_ptr(), name.len()) == 0 }
        }
        pub fn exists(name: &str) -> bool {
            unsafe { day_sql_fs_exists(name.as_ptr(), name.len()) == 1 }
        }
        pub fn list() -> Vec<String> {
            let needed = unsafe { day_sql_fs_list(core::ptr::null_mut(), 0) };
            let Ok(needed) = usize::try_from(needed) else {
                return Vec::new();
            };
            if needed == 0 {
                return Vec::new();
            }
            let mut buf = vec![0u8; needed];
            let got = unsafe { day_sql_fs_list(buf.as_mut_ptr(), buf.len()) };
            if usize::try_from(got) != Ok(needed) {
                return Vec::new();
            }
            String::from_utf8_lossy(&buf)
                .split('\u{1f}')
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        }
    }

    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    mod imp {
        // The native fake: same names, same slot/refcount behavior, one HashMap of byte
        // vectors per thread. Tests drive the real engine through it.
        use std::cell::RefCell;
        use std::collections::HashMap;
        use std::rc::Rc;

        struct Slot {
            name: Option<String>,
            data: Rc<RefCell<Vec<u8>>>,
            refs: u32,
        }

        thread_local! {
            static FILES: RefCell<HashMap<String, Rc<RefCell<Vec<u8>>>>> =
                RefCell::new(HashMap::new());
            static SLOTS: RefCell<Vec<Option<Slot>>> = const { RefCell::new(Vec::new()) };
        }

        fn with_slot<R>(slot: u32, f: impl FnOnce(&mut Slot) -> R) -> Option<R> {
            SLOTS.with(|s| s.borrow_mut().get_mut(slot as usize)?.as_mut().map(f))
        }

        pub fn open(name: &str, create: bool) -> Option<u32> {
            // An already-open name shares its slot (refcounted), matching the exclusive OPFS
            // access handles the worker page holds.
            let existing = SLOTS.with(|s| {
                s.borrow().iter().position(|sl| {
                    sl.as_ref()
                        .is_some_and(|sl| sl.name.as_deref() == Some(name) && !name.is_empty())
                })
            });
            if let Some(i) = existing {
                with_slot(i as u32, |sl| sl.refs += 1);
                return Some(i as u32);
            }
            let data = if name.is_empty() {
                Rc::new(RefCell::new(Vec::new()))
            } else {
                let known = FILES.with(|f| f.borrow().get(name).cloned());
                match known {
                    Some(d) => d,
                    None if create => {
                        let d = Rc::new(RefCell::new(Vec::new()));
                        FILES.with(|f| f.borrow_mut().insert(name.to_string(), d.clone()));
                        d
                    }
                    None => return None,
                }
            };
            let slot = Slot {
                name: (!name.is_empty()).then(|| name.to_string()),
                data,
                refs: 1,
            };
            SLOTS.with(|s| {
                let mut s = s.borrow_mut();
                if let Some(i) = s.iter().position(Option::is_none) {
                    s[i] = Some(slot);
                    Some(i as u32)
                } else {
                    s.push(Some(slot));
                    Some(s.len() as u32 - 1)
                }
            })
        }
        pub fn read(slot: u32, off: u64, buf: &mut [u8]) -> Option<usize> {
            with_slot(slot, |sl| {
                let data = sl.data.borrow();
                let off = off as usize;
                if off >= data.len() {
                    return 0;
                }
                let n = buf.len().min(data.len() - off);
                buf[..n].copy_from_slice(&data[off..off + n]);
                n
            })
        }
        pub fn write(slot: u32, off: u64, buf: &[u8]) -> bool {
            with_slot(slot, |sl| {
                let mut data = sl.data.borrow_mut();
                let end = off as usize + buf.len();
                if data.len() < end {
                    data.resize(end, 0);
                }
                data[off as usize..end].copy_from_slice(buf);
            })
            .is_some()
        }
        pub fn truncate(slot: u32, size: u64) -> bool {
            with_slot(slot, |sl| {
                let mut data = sl.data.borrow_mut();
                let size = size as usize;
                if data.len() > size {
                    data.truncate(size);
                } else {
                    data.resize(size, 0);
                }
            })
            .is_some()
        }
        pub fn size(slot: u32) -> Option<u64> {
            with_slot(slot, |sl| sl.data.borrow().len() as u64)
        }
        pub fn flush(_slot: u32) -> bool {
            true
        }
        pub fn close(slot: u32, delete: bool) {
            let name = SLOTS.with(|s| {
                let mut s = s.borrow_mut();
                let entry = s.get_mut(slot as usize)?.as_mut()?;
                entry.refs -= 1;
                if entry.refs > 0 {
                    return None;
                }
                let name = entry.name.take();
                s[slot as usize] = None;
                Some(name)
            });
            if delete {
                if let Some(Some(name)) = name {
                    FILES.with(|f| f.borrow_mut().remove(&name));
                }
            }
        }
        pub fn delete(name: &str) -> bool {
            FILES.with(|f| f.borrow_mut().remove(name)).is_some()
        }
        pub fn exists(name: &str) -> bool {
            FILES.with(|f| f.borrow().contains_key(name))
        }
        pub fn list() -> Vec<String> {
            FILES.with(|f| {
                let mut names: Vec<String> = f.borrow().keys().cloned().collect();
                names.sort();
                names
            })
        }
    }

    pub(crate) use imp::*;

    /// Whole-file read — the Export verb. Shares the open handle if the database is open.
    pub(crate) fn read_all(name: &str) -> Option<Vec<u8>> {
        let slot = open(name, false)?;
        let result = size(slot).and_then(|len| {
            let mut buf = vec![0u8; len as usize];
            (read(slot, 0, &mut buf) == Some(len as usize)).then_some(buf)
        });
        close(slot, false);
        result
    }

    /// Whole-file replace — the Import verb. A connection still open on `name` is invalid
    /// afterwards; the driver closes before importing.
    pub(crate) fn write_all(name: &str, bytes: &[u8]) -> bool {
        let Some(slot) = open(name, true) else {
            return false;
        };
        let ok = truncate(slot, 0) && write(slot, 0, bytes) && flush(slot);
        close(slot, false);
        ok
    }
}

// -------------------------------------------------------------------------------------------
// The RAM store behind day-mem
// -------------------------------------------------------------------------------------------

thread_local! {
    /// day-mem's named files (anonymous files live only in their DayFile). Nothing persistent
    /// routes here — the proxy driver sends every real file to the worker — so this is spill
    /// space, not storage.
    static RAM: RefCell<HashMap<String, Vec<u8>>> = RefCell::new(HashMap::new());
}

// -------------------------------------------------------------------------------------------
// File objects
// -------------------------------------------------------------------------------------------

/// What day-opfs file handles carry after `base`.
#[repr(C)]
struct DayFile {
    base: b::sqlite3_file,
    slot: u32,
    delete_on_close: bool,
}

/// What day-mem file handles carry: the whole file inline, plus its RAM key if named.
#[repr(C)]
struct MemFile {
    base: b::sqlite3_file,
    data: *mut Vec<u8>,
    name: *mut Option<String>,
    delete_on_close: bool,
}

const SHORT_READ_FILL: u8 = 0;

unsafe fn cstr<'a>(p: *const c_char) -> &'a str {
    if p.is_null() {
        ""
    } else {
        core::ffi::CStr::from_ptr(p).to_str().unwrap_or("")
    }
}

// --- day-opfs io methods -------------------------------------------------------------------

unsafe extern "C" fn opfs_close(f: *mut b::sqlite3_file) -> c_int {
    let f = f.cast::<DayFile>();
    fsx::close((*f).slot, (*f).delete_on_close);
    b::SQLITE_OK
}

unsafe extern "C" fn opfs_read(
    f: *mut b::sqlite3_file,
    buf: *mut c_void,
    amt: c_int,
    ofst: b::sqlite3_int64,
) -> c_int {
    let f = f.cast::<DayFile>();
    let out = core::slice::from_raw_parts_mut(buf.cast::<u8>(), amt as usize);
    match fsx::read((*f).slot, ofst as u64, out) {
        Some(n) if n == amt as usize => b::SQLITE_OK,
        Some(n) => {
            out[n..].fill(SHORT_READ_FILL);
            b::SQLITE_IOERR_SHORT_READ
        }
        None => b::SQLITE_IOERR_READ,
    }
}

unsafe extern "C" fn opfs_write(
    f: *mut b::sqlite3_file,
    buf: *const c_void,
    amt: c_int,
    ofst: b::sqlite3_int64,
) -> c_int {
    let f = f.cast::<DayFile>();
    let data = core::slice::from_raw_parts(buf.cast::<u8>(), amt as usize);
    if fsx::write((*f).slot, ofst as u64, data) {
        b::SQLITE_OK
    } else {
        b::SQLITE_IOERR_WRITE
    }
}

unsafe extern "C" fn opfs_truncate(f: *mut b::sqlite3_file, size: b::sqlite3_int64) -> c_int {
    let f = f.cast::<DayFile>();
    if fsx::truncate((*f).slot, size as u64) {
        b::SQLITE_OK
    } else {
        b::SQLITE_IOERR_TRUNCATE
    }
}

unsafe extern "C" fn opfs_sync(f: *mut b::sqlite3_file, _flags: c_int) -> c_int {
    let f = f.cast::<DayFile>();
    if fsx::flush((*f).slot) {
        b::SQLITE_OK
    } else {
        b::SQLITE_IOERR_FSYNC
    }
}

unsafe extern "C" fn opfs_file_size(f: *mut b::sqlite3_file, out: *mut b::sqlite3_int64) -> c_int {
    let f = f.cast::<DayFile>();
    match fsx::size((*f).slot) {
        Some(s) => {
            *out = s as b::sqlite3_int64;
            b::SQLITE_OK
        }
        None => b::SQLITE_IOERR_FSTAT,
    }
}

// --- shared trivial io methods -------------------------------------------------------------

unsafe extern "C" fn io_lock(_f: *mut b::sqlite3_file, _level: c_int) -> c_int {
    // One connection per database per context; the OPFS access handle is the real lock.
    b::SQLITE_OK
}
unsafe extern "C" fn io_unlock(_f: *mut b::sqlite3_file, _level: c_int) -> c_int {
    b::SQLITE_OK
}
unsafe extern "C" fn io_check_reserved(_f: *mut b::sqlite3_file, out: *mut c_int) -> c_int {
    *out = 0;
    b::SQLITE_OK
}
unsafe extern "C" fn io_file_control(
    _f: *mut b::sqlite3_file,
    _op: c_int,
    _arg: *mut c_void,
) -> c_int {
    b::SQLITE_NOTFOUND
}
unsafe extern "C" fn io_sector_size(_f: *mut b::sqlite3_file) -> c_int {
    4096
}
unsafe extern "C" fn io_device_characteristics(_f: *mut b::sqlite3_file) -> c_int {
    0
}

// --- day-mem io methods --------------------------------------------------------------------

unsafe extern "C" fn mem_close(f: *mut b::sqlite3_file) -> c_int {
    let f = f.cast::<MemFile>();
    let data = Box::from_raw((*f).data);
    let name = Box::from_raw((*f).name);
    match (*name, (*f).delete_on_close) {
        (Some(n), true) => {
            RAM.with(|r| r.borrow_mut().remove(&n));
        }
        (Some(n), false) => {
            RAM.with(|r| r.borrow_mut().insert(n, *data));
        }
        (None, _) => {}
    }
    b::SQLITE_OK
}

unsafe extern "C" fn mem_read(
    f: *mut b::sqlite3_file,
    buf: *mut c_void,
    amt: c_int,
    ofst: b::sqlite3_int64,
) -> c_int {
    let data = &*(*f.cast::<MemFile>()).data;
    let out = core::slice::from_raw_parts_mut(buf.cast::<u8>(), amt as usize);
    let off = ofst as usize;
    let n = if off >= data.len() {
        0
    } else {
        out.len().min(data.len() - off)
    };
    out[..n].copy_from_slice(&data[off..off + n]);
    if n == out.len() {
        b::SQLITE_OK
    } else {
        out[n..].fill(SHORT_READ_FILL);
        b::SQLITE_IOERR_SHORT_READ
    }
}

unsafe extern "C" fn mem_write(
    f: *mut b::sqlite3_file,
    buf: *const c_void,
    amt: c_int,
    ofst: b::sqlite3_int64,
) -> c_int {
    let data = &mut *(*f.cast::<MemFile>()).data;
    let src = core::slice::from_raw_parts(buf.cast::<u8>(), amt as usize);
    let end = ofst as usize + src.len();
    if data.len() < end {
        data.resize(end, 0);
    }
    data[ofst as usize..end].copy_from_slice(src);
    b::SQLITE_OK
}

unsafe extern "C" fn mem_truncate(f: *mut b::sqlite3_file, size: b::sqlite3_int64) -> c_int {
    let data = &mut *(*f.cast::<MemFile>()).data;
    data.truncate(size as usize);
    b::SQLITE_OK
}

unsafe extern "C" fn mem_sync(_f: *mut b::sqlite3_file, _flags: c_int) -> c_int {
    b::SQLITE_OK
}

unsafe extern "C" fn mem_file_size(f: *mut b::sqlite3_file, out: *mut b::sqlite3_int64) -> c_int {
    *out = (*(*f.cast::<MemFile>()).data).len() as b::sqlite3_int64;
    b::SQLITE_OK
}

// -------------------------------------------------------------------------------------------
// VFS methods
// -------------------------------------------------------------------------------------------

static OPFS_IO: b::sqlite3_io_methods = b::sqlite3_io_methods {
    iVersion: 1,
    xClose: Some(opfs_close),
    xRead: Some(opfs_read),
    xWrite: Some(opfs_write),
    xTruncate: Some(opfs_truncate),
    xSync: Some(opfs_sync),
    xFileSize: Some(opfs_file_size),
    xLock: Some(io_lock),
    xUnlock: Some(io_unlock),
    xCheckReservedLock: Some(io_check_reserved),
    xFileControl: Some(io_file_control),
    xSectorSize: Some(io_sector_size),
    xDeviceCharacteristics: Some(io_device_characteristics),
    xShmMap: None,
    xShmLock: None,
    xShmBarrier: None,
    xShmUnmap: None,
    xFetch: None,
    xUnfetch: None,
};

static MEM_IO: b::sqlite3_io_methods = b::sqlite3_io_methods {
    iVersion: 1,
    xClose: Some(mem_close),
    xRead: Some(mem_read),
    xWrite: Some(mem_write),
    xTruncate: Some(mem_truncate),
    xSync: Some(mem_sync),
    xFileSize: Some(mem_file_size),
    xLock: Some(io_lock),
    xUnlock: Some(io_unlock),
    xCheckReservedLock: Some(io_check_reserved),
    xFileControl: Some(io_file_control),
    xSectorSize: Some(io_sector_size),
    xDeviceCharacteristics: Some(io_device_characteristics),
    xShmMap: None,
    xShmLock: None,
    xShmBarrier: None,
    xShmUnmap: None,
    xFetch: None,
    xUnfetch: None,
};

unsafe extern "C" fn opfs_open(
    _vfs: *mut b::sqlite3_vfs,
    name: b::sqlite3_filename,
    file: *mut b::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    let f = file.cast::<DayFile>();
    (*f).base.pMethods = core::ptr::null();
    let create = flags & b::SQLITE_OPEN_CREATE != 0;
    let Some(slot) = fsx::open(cstr(name), create) else {
        return b::SQLITE_CANTOPEN;
    };
    (*f).slot = slot;
    (*f).delete_on_close = flags & b::SQLITE_OPEN_DELETEONCLOSE != 0;
    (*f).base.pMethods = &OPFS_IO;
    if !out_flags.is_null() {
        *out_flags = flags;
    }
    b::SQLITE_OK
}

unsafe extern "C" fn mem_open(
    _vfs: *mut b::sqlite3_vfs,
    name: b::sqlite3_filename,
    file: *mut b::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    let f = file.cast::<MemFile>();
    (*f).base.pMethods = core::ptr::null();
    let name = cstr(name);
    let (data, key) = if name.is_empty() {
        (Vec::new(), None)
    } else {
        let known = RAM.with(|r| r.borrow().get(name).cloned());
        match known {
            Some(d) => (d, Some(name.to_string())),
            None if flags & b::SQLITE_OPEN_CREATE != 0 => (Vec::new(), Some(name.to_string())),
            None => return b::SQLITE_CANTOPEN,
        }
    };
    (*f).data = Box::into_raw(Box::new(data));
    (*f).name = Box::into_raw(Box::new(key));
    (*f).delete_on_close = flags & b::SQLITE_OPEN_DELETEONCLOSE != 0;
    (*f).base.pMethods = &MEM_IO;
    if !out_flags.is_null() {
        *out_flags = flags;
    }
    b::SQLITE_OK
}

unsafe extern "C" fn opfs_delete(
    _vfs: *mut b::sqlite3_vfs,
    name: *const c_char,
    _sync_dir: c_int,
) -> c_int {
    fsx::delete(cstr(name));
    b::SQLITE_OK
}

unsafe extern "C" fn mem_delete(
    _vfs: *mut b::sqlite3_vfs,
    name: *const c_char,
    _sync_dir: c_int,
) -> c_int {
    RAM.with(|r| r.borrow_mut().remove(cstr(name)));
    b::SQLITE_OK
}

unsafe extern "C" fn opfs_access(
    _vfs: *mut b::sqlite3_vfs,
    name: *const c_char,
    _flags: c_int,
    out: *mut c_int,
) -> c_int {
    *out = c_int::from(fsx::exists(cstr(name)));
    b::SQLITE_OK
}

unsafe extern "C" fn mem_access(
    _vfs: *mut b::sqlite3_vfs,
    name: *const c_char,
    _flags: c_int,
    out: *mut c_int,
) -> c_int {
    *out = c_int::from(RAM.with(|r| r.borrow().contains_key(cstr(name))));
    b::SQLITE_OK
}

unsafe extern "C" fn vfs_full_pathname(
    _vfs: *mut b::sqlite3_vfs,
    name: *const c_char,
    n_out: c_int,
    out: *mut c_char,
) -> c_int {
    let src = cstr(name).as_bytes();
    if src.len() + 1 > n_out as usize {
        return b::SQLITE_CANTOPEN;
    }
    core::ptr::copy_nonoverlapping(src.as_ptr().cast::<c_char>(), out, src.len());
    *out.add(src.len()) = 0;
    b::SQLITE_OK
}

unsafe extern "C" fn vfs_randomness(
    _vfs: *mut b::sqlite3_vfs,
    n: c_int,
    out: *mut c_char,
) -> c_int {
    let buf = core::slice::from_raw_parts_mut(out.cast::<u8>(), n as usize);
    os::entropy(buf);
    n
}

unsafe extern "C" fn vfs_sleep(_vfs: *mut b::sqlite3_vfs, _micros: c_int) -> c_int {
    // Single-threaded: nothing to wait for, nothing to yield to.
    0
}

unsafe extern "C" fn vfs_current_time(_vfs: *mut b::sqlite3_vfs, out: *mut f64) -> c_int {
    *out = 2440587.5 + os::now_ms() / 86_400_000.0;
    b::SQLITE_OK
}

unsafe extern "C" fn vfs_current_time_i64(
    _vfs: *mut b::sqlite3_vfs,
    out: *mut b::sqlite3_int64,
) -> c_int {
    // Julian-day epoch in milliseconds + Unix milliseconds, SQLite's convention.
    *out = 210_866_760_000_000 + os::now_ms() as b::sqlite3_int64;
    b::SQLITE_OK
}

unsafe extern "C" fn vfs_get_last_error(
    _vfs: *mut b::sqlite3_vfs,
    _n: c_int,
    _out: *mut c_char,
) -> c_int {
    b::SQLITE_OK
}

fn make_vfs(name: &'static core::ffi::CStr, sz_os_file: c_int, mem: bool) -> b::sqlite3_vfs {
    b::sqlite3_vfs {
        iVersion: 2,
        szOsFile: sz_os_file,
        mxPathname: 512,
        pNext: core::ptr::null_mut(),
        zName: name.as_ptr(),
        pAppData: core::ptr::null_mut(),
        xOpen: Some(if mem { mem_open } else { opfs_open }),
        xDelete: Some(if mem { mem_delete } else { opfs_delete }),
        xAccess: Some(if mem { mem_access } else { opfs_access }),
        xFullPathname: Some(vfs_full_pathname),
        xDlOpen: None,
        xDlError: None,
        xDlSym: None,
        xDlClose: None,
        xRandomness: Some(vfs_randomness),
        xSleep: Some(vfs_sleep),
        xCurrentTime: Some(vfs_current_time),
        xGetLastError: Some(vfs_get_last_error),
        xCurrentTimeInt64: Some(vfs_current_time_i64),
        xSetSystemCall: None,
        xGetSystemCall: None,
        xNextSystemCall: None,
    }
}

/// The name day-opfs registers under; connections opt in via `sqlite3_open_v2`'s zVfs.
pub(crate) const OPFS_VFS: &core::ffi::CStr = c"day-opfs";

/// Register day-mem (default) and day-opfs — `sqlite3_os_init`'s body.
pub(crate) fn register_all() -> c_int {
    let mem = Box::leak(Box::new(make_vfs(
        c"day-mem",
        core::mem::size_of::<MemFile>() as c_int,
        true,
    )));
    let opfs = Box::leak(Box::new(make_vfs(
        OPFS_VFS,
        core::mem::size_of::<DayFile>() as c_int,
        false,
    )));
    let rc = unsafe { b::sqlite3_vfs_register(mem, 1) };
    if rc != b::SQLITE_OK {
        return rc;
    }
    unsafe { b::sqlite3_vfs_register(opfs, 0) }
}
