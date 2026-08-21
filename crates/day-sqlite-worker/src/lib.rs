// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The SQLite engine behind day-persistence on web-dom (docs/persistence.md).
//!
//! The vendored amalgamation compiles with no libc and no wasm-bindgen: `vendor/shim` renames
//! every libc symbol it needs onto the freestanding C subset compiled beside it, and the
//! handful of Rust-side shims in this file (allocator, UTC localtime, entropy, abort). The
//! only imports the wasm module gains are `day_dom_now_ms`/`day_dom_entropy` and — for the
//! worker instance — the ten `day_sql_fs_*` OPFS primitives, all provided by day-cli's shim
//! pages, so the module instantiates in Day's raw-wasm pipeline.
//!
//! One wasm, two instantiations. The **app instance** may use [`Connection::open_memory`]
//! in-process (`:memory:` engines never touch a file). The **worker instance** services the
//! SharedArrayBuffer channel: the worker page copies each request into wasm memory and calls
//! [`day_sql_exec`], which runs it against real OPFS files through the day-opfs VFS and
//! returns the reply bytes. The [`protocol`] module is the wire format both sides share.
//!
//! Everything — VFS, connection layer, worker loop — compiles and unit-tests on native hosts
//! against an in-memory OPFS fake; only the browser glue is wasm-specific.
//!
//! The C tree under `vendor/` and the shim recipe come from sqlite-wasm-rs (MIT, see
//! `vendor/LICENSE`); SQLite itself is public domain.

#[allow(
    non_upper_case_globals,
    non_camel_case_types,
    non_snake_case,
    dead_code,
    improper_ctypes,
    clippy::all
)]
#[rustfmt::skip]
mod bindings;
pub mod protocol;
mod vfs;

use core::ffi::{c_char, c_int, c_long, c_void};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use bindings as b;
use protocol::{Reply, Req, Value};

// -------------------------------------------------------------------------------------------
// Host services: time and entropy
// -------------------------------------------------------------------------------------------

mod os {
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    mod imp {
        #[link(wasm_import_module = "env")]
        unsafe extern "C" {
            fn day_dom_now_ms() -> f64;
            fn day_dom_entropy(buf: *mut u8, len: usize);
            fn day_sql_log(msg: *const u8, len: usize);
        }
        pub fn now_ms() -> f64 {
            unsafe { day_dom_now_ms() }
        }
        pub fn entropy(buf: &mut [u8]) {
            unsafe { day_dom_entropy(buf.as_mut_ptr(), buf.len()) }
        }
        /// A traced statement, to the worker page's console (`[day-sql]` lines in devtools).
        pub fn log_line(s: &str) {
            unsafe { day_sql_log(s.as_ptr(), s.len()) }
        }
    }

    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    mod imp {
        pub fn now_ms() -> f64 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as f64)
                .unwrap_or(0.0)
        }
        pub fn log_line(s: &str) {
            eprintln!("[day-sql] {s}");
        }
        pub fn entropy(buf: &mut [u8]) {
            // SQLite wants seed material, not key material. RandomState is std's OS-seeded
            // hasher — enough for temp names and PRNG seeding in the native test build.
            use std::hash::{BuildHasher, Hasher};
            let state = std::collections::hash_map::RandomState::new();
            for (i, chunk) in buf.chunks_mut(8).enumerate() {
                let mut h = state.build_hasher();
                h.write_usize(i);
                let bytes = h.finish().to_le_bytes();
                chunk.copy_from_slice(&bytes[..chunk.len()]);
            }
        }
    }

    pub(crate) use imp::*;
}

// -------------------------------------------------------------------------------------------
// The C shims (vendor/shim/wasm-shim.h renames; recipe from sqlite-wasm-rs, MIT)
// -------------------------------------------------------------------------------------------

// dlmalloc's alignment: enough for every SQLite allocation.
const ALIGN: usize = core::mem::size_of::<usize>() * 2;

/// # Safety
/// C `malloc` contract; the size is stored ahead of the returned block for free/realloc.
#[no_mangle]
pub unsafe extern "C" fn rust_sqlite_wasm_malloc(size: usize) -> *mut c_void {
    let layout = core::alloc::Layout::from_size_align_unchecked(size + ALIGN, ALIGN);
    let ptr = std::alloc::alloc(layout);
    if ptr.is_null() {
        return core::ptr::null_mut();
    }
    *ptr.cast::<usize>() = size;
    ptr.add(ALIGN).cast()
}

/// # Safety
/// Only pointers from `rust_sqlite_wasm_malloc`/`realloc`.
#[no_mangle]
pub unsafe extern "C" fn rust_sqlite_wasm_free(ptr: *mut c_void) {
    let ptr: *mut u8 = ptr.sub(ALIGN).cast();
    let size = *(ptr.cast::<usize>());
    let layout = core::alloc::Layout::from_size_align_unchecked(size + ALIGN, ALIGN);
    std::alloc::dealloc(ptr, layout);
}

/// # Safety
/// Only pointers from `rust_sqlite_wasm_malloc`/`realloc`.
#[no_mangle]
pub unsafe extern "C" fn rust_sqlite_wasm_realloc(
    ptr: *mut c_void,
    new_size: usize,
) -> *mut c_void {
    let ptr: *mut u8 = ptr.sub(ALIGN).cast();
    let size = *(ptr.cast::<usize>());
    let layout = core::alloc::Layout::from_size_align_unchecked(size + ALIGN, ALIGN);
    let ptr = std::alloc::realloc(ptr, layout, new_size + ALIGN);
    if ptr.is_null() {
        return core::ptr::null_mut();
    }
    *ptr.cast::<usize>() = new_size;
    ptr.add(ALIGN).cast()
}

/// # Safety
/// C `calloc` contract.
#[no_mangle]
pub unsafe extern "C" fn rust_sqlite_wasm_calloc(num: usize, size: usize) -> *mut c_void {
    let Some(total) = num.checked_mul(size) else {
        return core::ptr::null_mut();
    };
    let ptr: *mut u8 = rust_sqlite_wasm_malloc(total).cast();
    if !ptr.is_null() {
        core::ptr::write_bytes(ptr, 0, total);
    }
    ptr.cast()
}

/// # Safety
/// wasi `getentropy` contract: fills `buf_len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rust_sqlite_wasm_getentropy(buf: *mut u8, buf_len: usize) -> u16 {
    os::entropy(core::slice::from_raw_parts_mut(buf, buf_len));
    0
}

/// # Safety
/// C `__assert_fail` contract — an assertion in the C tree is a bug worth stopping on.
#[no_mangle]
pub unsafe extern "C" fn rust_sqlite_wasm_assert_fail(
    expr: *const c_char,
    file: *const c_char,
    line: c_int,
    _func: *const c_char,
) {
    let expr = core::ffi::CStr::from_ptr(expr).to_string_lossy();
    let file = core::ffi::CStr::from_ptr(file).to_string_lossy();
    panic!("sqlite assertion failed: {expr} ({file}:{line})");
}

/// # Safety
/// C `abort` contract.
#[no_mangle]
pub unsafe extern "C" fn rust_sqlite_wasm_abort() -> ! {
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    core::arch::wasm32::unreachable();
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    std::process::abort();
}

/// musl's `struct tm` (what `<time.h>` in the vendored headers declares).
#[repr(C)]
pub struct Tm {
    tm_sec: c_int,
    tm_min: c_int,
    tm_hour: c_int,
    tm_mday: c_int,
    tm_mon: c_int,
    tm_year: c_int,
    tm_wday: c_int,
    tm_yday: c_int,
    tm_isdst: c_int,
    tm_gmtoff: c_long,
    tm_zone: *mut c_char,
}

/// UTC civil breakdown for `t` — Howard Hinnant's `civil_from_days`. No timezone database
/// exists in the browser sandbox, so SQLite's `'localtime'` modifier resolves to UTC here;
/// apps that need the viewer's zone format in the UI layer, where the platform knows it.
fn utc_breakdown(t: i64) -> Tm {
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    const CUM: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let yday = CUM[(m - 1) as usize] + i64::from(leap && m > 2) + d - 1;
    Tm {
        tm_sec: (secs % 60) as c_int,
        tm_min: ((secs / 60) % 60) as c_int,
        tm_hour: (secs / 3600) as c_int,
        tm_mday: d as c_int,
        tm_mon: (m - 1) as c_int,
        tm_year: (y - 1900) as c_int,
        tm_wday: (days + 4).rem_euclid(7) as c_int,
        tm_yday: yday as c_int,
        tm_isdst: 0,
        tm_gmtoff: 0,
        tm_zone: core::ptr::null_mut(),
    }
}

/// # Safety
/// C `localtime` contract: returns a shared static buffer, single-threaded callers only —
/// which this engine is by construction (SQLITE_THREADSAFE=0).
#[no_mangle]
pub unsafe extern "C" fn rust_sqlite_wasm_localtime(t: *const i64) -> *mut Tm {
    static mut TM: Tm = Tm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: 0,
        tm_mon: 0,
        tm_year: 0,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: 0,
        tm_gmtoff: 0,
        tm_zone: core::ptr::null_mut(),
    };
    let tm = core::ptr::addr_of_mut!(TM);
    *tm = utc_breakdown(*t);
    tm
}

/// # Safety
/// Called once by `sqlite3_initialize` (SQLITE_OS_OTHER contract): registers day-mem
/// (default) and day-opfs.
#[no_mangle]
pub unsafe extern "C" fn sqlite3_os_init() -> c_int {
    vfs::register_all()
}

/// # Safety
/// `sqlite3_shutdown` counterpart; nothing to unwind.
#[no_mangle]
pub unsafe extern "C" fn sqlite3_os_end() -> c_int {
    b::SQLITE_OK
}

// -------------------------------------------------------------------------------------------
// The connection layer
// -------------------------------------------------------------------------------------------

/// An engine failure: SQLite's result code and its message.
#[derive(Clone, Debug, PartialEq)]
pub struct SqlError {
    pub code: i32,
    pub message: String,
}

impl std::fmt::Display for SqlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (sqlite code {})", self.message, self.code)
    }
}

impl std::error::Error for SqlError {}

fn err(code: c_int, message: impl Into<String>) -> SqlError {
    SqlError {
        code,
        message: message.into(),
    }
}

/// One open database. `open_memory` works in any instance (no file I/O at all);
/// `open_opfs` routes through the day-opfs VFS and is the worker's shape.
/// A statement-trace sink (`Connection::trace_stmt`).
pub type TraceFn = Box<dyn Fn(&str)>;

pub struct Connection {
    db: *mut b::sqlite3,
    /// The installed statement-trace closure (`trace_stmt`) — heap-boxed so the pointer the
    /// engine holds stays put while the Connection moves; dropped after `db` closes.
    trace: Option<Box<TraceFn>>,
}

unsafe extern "C" fn sql_trace_cb(
    ev: core::ffi::c_uint,
    ctx: *mut core::ffi::c_void,
    p: *mut core::ffi::c_void,
    x: *mut core::ffi::c_void,
) -> c_int {
    if ev != b::SQLITE_TRACE_STMT || ctx.is_null() {
        return 0;
    }
    // SAFETY: ctx is the connection's boxed trace closure, alive until after sqlite3_close
    // (field order in Connection); SQLITE_THREADSAFE=0 means no concurrent invocation.
    let f = unsafe { &*(ctx as *const TraceFn) };
    unsafe {
        let expanded = b::sqlite3_expanded_sql(p.cast());
        if !expanded.is_null() {
            if let Ok(s) = core::ffi::CStr::from_ptr(expanded).to_str() {
                f(s);
            }
            b::sqlite3_free(expanded.cast());
        } else if !x.is_null() {
            // The engine could not expand (OOM, or a trigger frame): the unexpanded text.
            if let Ok(s) = core::ffi::CStr::from_ptr(x.cast()).to_str() {
                f(s);
            }
        }
    }
    0
}

impl Connection {
    fn open_with(path: &str, vfs: Option<&core::ffi::CStr>) -> Result<Connection, SqlError> {
        let path = std::ffi::CString::new(path).map_err(|_| err(1, "NUL in database name"))?;
        let mut db: *mut b::sqlite3 = core::ptr::null_mut();
        let flags = b::SQLITE_OPEN_READWRITE | b::SQLITE_OPEN_CREATE;
        let rc = unsafe {
            b::sqlite3_open_v2(
                path.as_ptr(),
                &mut db,
                flags,
                vfs.map_or(core::ptr::null(), |v| v.as_ptr()),
            )
        };
        if rc != b::SQLITE_OK {
            let message = if db.is_null() {
                "out of memory opening database".to_string()
            } else {
                let m = unsafe { errmsg(db) };
                unsafe { b::sqlite3_close(db) };
                m
            };
            return Err(err(rc, message));
        }
        Ok(Connection { db, trace: None })
    }

    /// Install the engine's per-statement trace (`sqlite3_trace_v2`, SQLITE_TRACE_STMT): `f`
    /// sees every statement this connection executes, with bound parameters expanded.
    pub fn trace_stmt(&mut self, f: TraceFn) {
        let boxed: Box<TraceFn> = Box::new(f);
        let ctx = &*boxed as *const TraceFn as *mut core::ffi::c_void;
        // SAFETY: ctx points into `boxed`, stored on self below — stable while the engine
        // holds it, and cleared implicitly when the connection closes.
        unsafe {
            b::sqlite3_trace_v2(self.db, b::SQLITE_TRACE_STMT, Some(sql_trace_cb), ctx);
        }
        self.trace = Some(boxed);
    }

    /// A private in-memory database — the only kind the main-thread app instance may open.
    pub fn open_memory() -> Result<Connection, SqlError> {
        Self::open_with(":memory:", None)
    }

    /// A named database through the day-opfs VFS (real OPFS in the worker, the in-memory
    /// fake in native tests).
    pub fn open_opfs(name: &str) -> Result<Connection, SqlError> {
        Self::open_with(name, Some(vfs::OPFS_VFS))
    }

    /// Run semicolon-separated statements with no parameters and no rows.
    pub fn execute_batch(&self, sql: &str) -> Result<(), SqlError> {
        let mut rest = sql.as_bytes();
        while !rest.is_empty() {
            let (stmt, tail) = self.prepare(rest)?;
            let Some(stmt) = stmt else {
                rest = tail;
                continue; // whitespace / comments
            };
            let rc = loop {
                match unsafe { b::sqlite3_step(stmt.0) } {
                    b::SQLITE_ROW => continue,
                    code => break code,
                }
            };
            if rc != b::SQLITE_DONE {
                return Err(err(rc, unsafe { errmsg(self.db) }));
            }
            drop(stmt);
            rest = tail;
        }
        Ok(())
    }

    /// Run one statement; the count of changed rows comes back.
    pub fn execute(&self, sql: &str, params: &[Value]) -> Result<u64, SqlError> {
        let (stmt, _) = self.prepare(sql.as_bytes())?;
        let stmt = stmt.ok_or_else(|| err(1, "empty statement"))?;
        self.bind(&stmt, params)?;
        match unsafe { b::sqlite3_step(stmt.0) } {
            b::SQLITE_DONE | b::SQLITE_ROW => Ok(unsafe { b::sqlite3_changes(self.db) } as u64),
            rc => Err(err(rc, unsafe { errmsg(self.db) })),
        }
    }

    /// Run one statement, delivering each row to `f`.
    pub fn query(
        &self,
        sql: &str,
        params: &[Value],
        f: &mut dyn FnMut(Vec<Value>),
    ) -> Result<(), SqlError> {
        let (stmt, _) = self.prepare(sql.as_bytes())?;
        let stmt = stmt.ok_or_else(|| err(1, "empty statement"))?;
        self.bind(&stmt, params)?;
        loop {
            match unsafe { b::sqlite3_step(stmt.0) } {
                b::SQLITE_ROW => {
                    let n = unsafe { b::sqlite3_column_count(stmt.0) };
                    let mut row = Vec::with_capacity(n as usize);
                    for i in 0..n {
                        row.push(unsafe { column_value(stmt.0, i) });
                    }
                    f(row);
                }
                b::SQLITE_DONE => return Ok(()),
                rc => return Err(err(rc, unsafe { errmsg(self.db) })),
            }
        }
    }

    fn prepare<'a>(&self, sql: &'a [u8]) -> Result<(Option<Stmt>, &'a [u8]), SqlError> {
        let mut stmt: *mut b::sqlite3_stmt = core::ptr::null_mut();
        let mut tail: *const c_char = core::ptr::null();
        let rc = unsafe {
            b::sqlite3_prepare_v2(
                self.db,
                sql.as_ptr().cast(),
                sql.len() as c_int,
                &mut stmt,
                &mut tail,
            )
        };
        if rc != b::SQLITE_OK {
            return Err(err(rc, unsafe { errmsg(self.db) }));
        }
        let consumed = if tail.is_null() {
            sql.len()
        } else {
            unsafe { tail.offset_from(sql.as_ptr().cast()) as usize }
        };
        Ok((
            (!stmt.is_null()).then_some(Stmt(stmt)),
            &sql[consumed.min(sql.len())..],
        ))
    }

    fn bind(&self, stmt: &Stmt, params: &[Value]) -> Result<(), SqlError> {
        // SQLITE_TRANSIENT: SQLite copies the buffer before the call returns.
        let transient: b::sqlite3_destructor_type = unsafe { core::mem::transmute(-1isize) };
        for (i, v) in params.iter().enumerate() {
            let i = i as c_int + 1;
            let rc = unsafe {
                match v {
                    Value::Null => b::sqlite3_bind_null(stmt.0, i),
                    Value::Int(n) => b::sqlite3_bind_int64(stmt.0, i, *n),
                    Value::Real(r) => b::sqlite3_bind_double(stmt.0, i, *r),
                    Value::Text(t) => b::sqlite3_bind_text(
                        stmt.0,
                        i,
                        t.as_ptr().cast(),
                        t.len() as c_int,
                        transient,
                    ),
                    Value::Blob(bl) => b::sqlite3_bind_blob(
                        stmt.0,
                        i,
                        bl.as_ptr().cast(),
                        bl.len() as c_int,
                        transient,
                    ),
                }
            };
            if rc != b::SQLITE_OK {
                return Err(err(rc, unsafe { errmsg(self.db) }));
            }
        }
        Ok(())
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        unsafe { b::sqlite3_close(self.db) };
    }
}

/// Finalized on drop, so every prepare path cleans up.
struct Stmt(*mut b::sqlite3_stmt);

impl Drop for Stmt {
    fn drop(&mut self) {
        unsafe { b::sqlite3_finalize(self.0) };
    }
}

unsafe fn errmsg(db: *mut b::sqlite3) -> String {
    let p = b::sqlite3_errmsg(db);
    if p.is_null() {
        "unknown sqlite error".to_string()
    } else {
        core::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

unsafe fn column_value(stmt: *mut b::sqlite3_stmt, i: c_int) -> Value {
    match b::sqlite3_column_type(stmt, i) {
        b::SQLITE_INTEGER => Value::Int(b::sqlite3_column_int64(stmt, i)),
        b::SQLITE_FLOAT => Value::Real(b::sqlite3_column_double(stmt, i)),
        b::SQLITE_TEXT => {
            let p = b::sqlite3_column_text(stmt, i);
            let n = b::sqlite3_column_bytes(stmt, i) as usize;
            let bytes = if p.is_null() {
                &[][..]
            } else {
                core::slice::from_raw_parts(p, n)
            };
            Value::Text(String::from_utf8_lossy(bytes).into_owned())
        }
        b::SQLITE_BLOB => {
            let p = b::sqlite3_column_blob(stmt, i);
            let n = b::sqlite3_column_bytes(stmt, i) as usize;
            let bytes = if p.is_null() {
                &[][..]
            } else {
                core::slice::from_raw_parts(p.cast::<u8>(), n)
            };
            Value::Blob(bytes.to_vec())
        }
        _ => Value::Null,
    }
}

// -------------------------------------------------------------------------------------------
// The worker loop
// -------------------------------------------------------------------------------------------

thread_local! {
    static CONNS: RefCell<HashMap<u32, Connection>> = RefCell::new(HashMap::new());
    static NEXT_CONN: Cell<u32> = const { Cell::new(1) };
    /// The staged reply `day_sql_exec` hands the worker page: `[len: u32 LE][bytes]`. Lives
    /// until the next request — the channel is strictly one request at a time.
    static REPLY: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Service one request — the worker loop's whole body, and the seam the native tests drive.
pub fn handle_request(req: &[u8]) -> Vec<u8> {
    let reply = match protocol::decode_req(req) {
        Err(_) => Reply::Err("malformed request".to_string()),
        Ok(req) => dispatch(req),
    };
    protocol::encode_reply(&reply)
}

fn dispatch(req: Req) -> Reply {
    match req {
        Req::Open { name, trace } => match Connection::open_opfs(&name) {
            Ok(mut conn) => {
                if trace {
                    conn.trace_stmt(Box::new(os::log_line));
                }
                let id = NEXT_CONN.with(|n| {
                    let id = n.get();
                    n.set(id.wrapping_add(1).max(1));
                    id
                });
                CONNS.with(|c| c.borrow_mut().insert(id, conn));
                Reply::Conn(id)
            }
            Err(e) => Reply::Err(e.to_string()),
        },
        Req::Close { conn } => {
            CONNS.with(|c| c.borrow_mut().remove(&conn));
            Reply::Ok
        }
        Req::Batch { conn, sql } => with_conn(conn, |c| c.execute_batch(&sql).map(|_| Reply::Ok)),
        Req::Exec { conn, sql, params } => {
            with_conn(conn, |c| c.execute(&sql, &params).map(Reply::Changes))
        }
        Req::Query { conn, sql, params } => with_conn(conn, |c| {
            let mut rows = Vec::new();
            c.query(&sql, &params, &mut |row| rows.push(row))
                .map(|_| Reply::Rows(rows))
        }),
        Req::Exists { name } => Reply::Bool(vfs::fsx::exists(&name)),
        Req::List => Reply::Names(vfs::fsx::list()),
        Req::Delete { name } => {
            vfs::fsx::delete(&name);
            Reply::Ok
        }
        Req::Export { name } => match vfs::fsx::read_all(&name) {
            Some(bytes) => Reply::Bytes(bytes),
            None => Reply::Err(format!("no database named {name:?}")),
        },
        Req::Import { name, bytes } => {
            if vfs::fsx::write_all(&name, &bytes) {
                Reply::Ok
            } else {
                Reply::Err(format!("could not write {name:?}"))
            }
        }
    }
}

fn with_conn(id: u32, f: impl FnOnce(&Connection) -> Result<Reply, SqlError>) -> Reply {
    CONNS.with(|c| {
        // Take the connection out for the call: statement callbacks must not find the
        // registry borrowed.
        let Some(conn) = c.borrow_mut().remove(&id) else {
            return Reply::Err(format!("no connection {id}"));
        };
        let reply = match f(&conn) {
            Ok(r) => r,
            Err(e) => Reply::Err(e.to_string()),
        };
        c.borrow_mut().insert(id, conn);
        reply
    })
}

/// Allocate `len` bytes for the worker page to write a request into (consumed by
/// [`day_sql_exec`]).
#[no_mangle]
pub extern "C" fn day_sql_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len);
    let ptr = v.as_mut_ptr();
    core::mem::forget(v);
    ptr
}

/// # Safety
/// `ptr` must be a `day_sql_alloc(len)` buffer holding a complete request; ownership
/// transfers here. Returns a pointer to `[reply_len: u32 LE][reply bytes]`, valid until the
/// next call.
#[no_mangle]
pub unsafe extern "C" fn day_sql_exec(ptr: *mut u8, len: usize) -> *const u8 {
    let req = Vec::from_raw_parts(ptr, len, len);
    let reply = handle_request(&req);
    REPLY.with(|r| {
        let mut r = r.borrow_mut();
        r.clear();
        r.extend_from_slice(&(reply.len() as u32).to_le_bytes());
        r.extend_from_slice(&reply);
        r.as_ptr()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(req: Req) -> Reply {
        protocol::decode_reply(&handle_request(&protocol::encode_req(&req))).expect("well-formed")
    }

    fn open(name: &str) -> u32 {
        match ask(Req::Open {
            name: name.into(),
            trace: false,
        }) {
            Reply::Conn(id) => id,
            other => panic!("open {name}: {other:?}"),
        }
    }

    #[test]
    fn memory_connection_round_trips_every_value_kind() {
        let conn = Connection::open_memory().expect("open");
        conn.execute_batch("CREATE TABLE t (a, b, c, d, e)")
            .expect("create");
        let params = vec![
            Value::Null,
            Value::Int(i64::MIN),
            Value::Real(2.5),
            Value::Text("héllo — quote' and \u{1f}".into()),
            Value::Blob(vec![0, 1, 255, 0]),
        ];
        let n = conn
            .execute("INSERT INTO t VALUES (?, ?, ?, ?, ?)", &params)
            .expect("insert");
        assert_eq!(n, 1);
        let mut rows = Vec::new();
        conn.query("SELECT a, b, c, d, e FROM t", &[], &mut |r| rows.push(r))
            .expect("select");
        assert_eq!(rows, vec![params]);
    }

    #[test]
    fn batch_runs_multiple_statements_and_reports_the_failing_one() {
        let conn = Connection::open_memory().expect("open");
        conn.execute_batch(
            "CREATE TABLE a (x); -- comment\nCREATE TABLE b (y);\nINSERT INTO a VALUES (1);",
        )
        .expect("batch");
        let mut count = Vec::new();
        conn.query("SELECT count(*) FROM a", &[], &mut |r| count.push(r))
            .expect("count");
        assert_eq!(count, vec![vec![Value::Int(1)]]);

        let e = conn
            .execute_batch("INSERT INTO a VALUES (2); INSERT INTO nope VALUES (3);")
            .expect_err("second statement is bad");
        assert!(e.message.contains("nope"), "{e}");
    }

    #[test]
    fn transactions_roll_back() {
        let conn = Connection::open_memory().expect("open");
        conn.execute_batch("CREATE TABLE t (x); BEGIN; INSERT INTO t VALUES (1); ROLLBACK;")
            .expect("batch");
        let mut n = Vec::new();
        conn.query("SELECT count(*) FROM t", &[], &mut |r| n.push(r))
            .expect("count");
        assert_eq!(n, vec![vec![Value::Int(0)]]);
    }

    #[test]
    fn fts5_and_rtree_are_compiled_in() {
        let conn = Connection::open_memory().expect("open");
        conn.execute_batch(
            "CREATE VIRTUAL TABLE ft USING fts5(body);\
             CREATE VIRTUAL TABLE geo USING rtree(id, minx, maxx, miny, maxy);",
        )
        .expect("both engines present");
        conn.execute(
            "INSERT INTO ft VALUES (?)",
            &[Value::Text("golden harbor light".into())],
        )
        .expect("insert");
        let mut hits = Vec::new();
        conn.query(
            "SELECT count(*) FROM ft WHERE ft MATCH ?",
            &[Value::Text("harbor".into())],
            &mut |r| hits.push(r),
        )
        .expect("match");
        assert_eq!(hits, vec![vec![Value::Int(1)]]);
    }

    #[test]
    fn dates_work_without_a_timezone_database() {
        let conn = Connection::open_memory().expect("open");
        let mut out = Vec::new();
        conn.query(
            "SELECT datetime(0, 'unixepoch'), date('now'), datetime(86400, 'unixepoch', 'localtime')",
            &[],
            &mut |r| out.push(r),
        )
        .expect("dates");
        let row = &out[0];
        assert_eq!(row[0], Value::Text("1970-01-01 00:00:00".into()));
        let Value::Text(today) = &row[1] else {
            panic!("date('now'): {row:?}")
        };
        assert!(today.starts_with("20"), "sane current year: {today}");
        // 'localtime' is UTC here (no tz database in the sandbox).
        assert_eq!(row[2], Value::Text("1970-01-02 00:00:00".into()));
    }

    #[test]
    fn utc_breakdown_matches_known_dates() {
        let tm = utc_breakdown(1_787_616_000); // 2026-08-25 00:00:00 UTC, a Tuesday
        assert_eq!(
            (tm.tm_year, tm.tm_mon, tm.tm_mday, tm.tm_wday, tm.tm_yday),
            (126, 7, 25, 2, 236)
        );
        let tm = utc_breakdown(0);
        assert_eq!(
            (tm.tm_year, tm.tm_mon, tm.tm_mday, tm.tm_wday, tm.tm_yday),
            (70, 0, 1, 4, 0)
        );
        // Leap-year boundary: 2024-12-31 is yday 365.
        let tm = utc_breakdown(1_735_603_200);
        assert_eq!(
            (tm.tm_year, tm.tm_mon, tm.tm_mday, tm.tm_yday),
            (124, 11, 31, 365)
        );
    }

    #[test]
    fn opfs_database_persists_across_close_and_reopen() {
        let conn = Connection::open_opfs("persist.db").expect("open");
        conn.execute_batch("CREATE TABLE t (x)").expect("create");
        for i in 0..100 {
            conn.execute("INSERT INTO t VALUES (?)", &[Value::Int(i)])
                .expect("insert");
        }
        drop(conn);

        let conn = Connection::open_opfs("persist.db").expect("reopen");
        let mut n = Vec::new();
        conn.query("SELECT count(*), sum(x) FROM t", &[], &mut |r| n.push(r))
            .expect("count");
        assert_eq!(n, vec![vec![Value::Int(100), Value::Int(4950)]]);
        drop(conn);
        vfs::fsx::delete("persist.db");
    }

    #[test]
    fn commits_leave_no_journal_behind() {
        let conn = Connection::open_opfs("clean.db").expect("open");
        conn.execute_batch("CREATE TABLE t (x); BEGIN; INSERT INTO t VALUES (1); COMMIT;")
            .expect("batch");
        drop(conn);
        let names = vfs::fsx::list();
        assert!(
            names.iter().any(|n| n == "clean.db"),
            "database listed: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n.contains("journal")),
            "journal deleted after commit: {names:?}"
        );
        vfs::fsx::delete("clean.db");
    }

    #[test]
    fn worker_protocol_full_round_trip() {
        let conn = open("proto.db");
        assert_eq!(
            ask(Req::Batch {
                conn,
                sql: "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT)".into()
            }),
            Reply::Ok
        );
        assert_eq!(
            ask(Req::Exec {
                conn,
                sql: "INSERT INTO notes (id, body) VALUES (?, ?)".into(),
                params: vec![Value::Int(1), Value::Text("rain later".into())],
            }),
            Reply::Changes(1)
        );
        assert_eq!(
            ask(Req::Query {
                conn,
                sql: "SELECT id, body FROM notes".into(),
                params: vec![],
            }),
            Reply::Rows(vec![vec![Value::Int(1), Value::Text("rain later".into())]])
        );
        // Errors carry SQLite's message, and the connection survives them.
        let Reply::Err(msg) = ask(Req::Query {
            conn,
            sql: "SELECT nope FROM notes".into(),
            params: vec![],
        }) else {
            panic!("bad column must error")
        };
        assert!(msg.contains("nope"), "{msg}");
        assert_eq!(
            ask(Req::Query {
                conn,
                sql: "SELECT count(*) FROM notes".into(),
                params: vec![],
            }),
            Reply::Rows(vec![vec![Value::Int(1)]])
        );
        assert_eq!(ask(Req::Close { conn }), Reply::Ok);
        let Reply::Err(gone) = ask(Req::Exec {
            conn,
            sql: "INSERT INTO notes (id) VALUES (2)".into(),
            params: vec![],
        }) else {
            panic!("closed connection must error")
        };
        assert!(gone.contains("no connection"), "{gone}");
        assert_eq!(
            ask(Req::Delete {
                name: "proto.db".into()
            }),
            Reply::Ok
        );
    }

    #[test]
    fn pool_verbs_export_import_and_list() {
        let conn = open("source.db");
        assert_eq!(
            ask(Req::Batch {
                conn,
                sql: "CREATE TABLE t (x); INSERT INTO t VALUES (42);".into()
            }),
            Reply::Ok
        );
        assert_eq!(ask(Req::Close { conn }), Reply::Ok);

        assert_eq!(
            ask(Req::Exists {
                name: "source.db".into()
            }),
            Reply::Bool(true)
        );
        assert_eq!(
            ask(Req::Exists {
                name: "nope.db".into()
            }),
            Reply::Bool(false)
        );

        let Reply::Bytes(image) = ask(Req::Export {
            name: "source.db".into(),
        }) else {
            panic!("export must answer bytes")
        };
        assert!(
            image.starts_with(b"SQLite format 3\0"),
            "a real SQLite file image"
        );

        assert_eq!(
            ask(Req::Import {
                name: "copy.db".into(),
                bytes: image
            }),
            Reply::Ok
        );
        let copy = open("copy.db");
        assert_eq!(
            ask(Req::Query {
                conn: copy,
                sql: "SELECT x FROM t".into(),
                params: vec![],
            }),
            Reply::Rows(vec![vec![Value::Int(42)]])
        );
        assert_eq!(ask(Req::Close { conn: copy }), Reply::Ok);

        let Reply::Names(names) = ask(Req::List) else {
            panic!("list must answer names")
        };
        assert!(names.contains(&"source.db".to_string()), "{names:?}");
        assert!(names.contains(&"copy.db".to_string()), "{names:?}");

        assert_eq!(
            ask(Req::Delete {
                name: "source.db".into()
            }),
            Reply::Ok
        );
        assert_eq!(
            ask(Req::Delete {
                name: "copy.db".into()
            }),
            Reply::Ok
        );
        assert_eq!(
            ask(Req::Exists {
                name: "source.db".into()
            }),
            Reply::Bool(false)
        );
    }

    #[test]
    fn two_databases_are_independent() {
        let a = open("ind-a.db");
        let bconn = open("ind-b.db");
        assert_eq!(
            ask(Req::Batch {
                conn: a,
                sql: "CREATE TABLE only_a (x)".into()
            }),
            Reply::Ok
        );
        let Reply::Err(msg) = ask(Req::Query {
            conn: bconn,
            sql: "SELECT * FROM only_a".into(),
            params: vec![],
        }) else {
            panic!("b must not see a's table")
        };
        assert!(msg.contains("only_a"), "{msg}");
        assert_eq!(ask(Req::Close { conn: a }), Reply::Ok);
        assert_eq!(ask(Req::Close { conn: bconn }), Reply::Ok);
        assert_eq!(
            ask(Req::Delete {
                name: "ind-a.db".into()
            }),
            Reply::Ok
        );
        assert_eq!(
            ask(Req::Delete {
                name: "ind-b.db".into()
            }),
            Reply::Ok
        );
    }

    #[test]
    fn trace_stmt_sees_every_statement_with_parameters_expanded() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let seen: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        let mut conn = Connection::open_memory().expect("open");
        conn.trace_stmt(Box::new(move |s| sink.borrow_mut().push(s.to_string())));
        conn.execute_batch("CREATE TABLE t (a, b)").expect("create");
        conn.execute(
            "INSERT INTO t VALUES (?, ?)",
            &[Value::Int(7), Value::Text("rain later".into())],
        )
        .expect("insert");
        let mut n = Vec::new();
        conn.query("SELECT count(*) FROM t", &[], &mut |r| n.push(r))
            .expect("count");
        let log = seen.borrow();
        assert!(log.iter().any(|s| s.contains("CREATE TABLE t")), "{log:?}");
        assert!(
            log.iter()
                .any(|s| s.contains("INSERT INTO t VALUES (7, 'rain later')")),
            "parameters expand: {log:?}"
        );
        assert!(log.iter().any(|s| s.contains("SELECT count(*)")), "{log:?}");
    }

    #[test]
    fn malformed_requests_answer_an_error() {
        let reply = protocol::decode_reply(&handle_request(&[9, 9, 9])).expect("well-formed");
        assert_eq!(reply, Reply::Err("malformed request".to_string()));
    }

    #[test]
    fn day_sql_exec_stages_a_length_prefixed_reply() {
        let req = protocol::encode_req(&Req::List);
        let ptr = day_sql_alloc(req.len());
        // SAFETY: writing exactly len bytes into a day_sql_alloc(len) buffer, then handing
        // ownership to day_sql_exec — its documented contract.
        let reply = unsafe {
            core::ptr::copy_nonoverlapping(req.as_ptr(), ptr, req.len());
            let out = day_sql_exec(ptr, req.len());
            let len = u32::from_le_bytes(core::slice::from_raw_parts(out, 4).try_into().unwrap());
            core::slice::from_raw_parts(out.add(4), len as usize).to_vec()
        };
        assert!(matches!(
            protocol::decode_reply(&reply),
            Ok(Reply::Names(_))
        ));
    }
}
