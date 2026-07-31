// ---------------------------------------------------------------------------
// The web (web-dom): the browser's Origin Private File System through the day-dom shim — an
// origin-scoped real file hierarchy, surviving reloads like localStorage but sized for data.
// OPFS only, no fallback store: a context without it (a pre-OPFS browser, or a
// private-browsing/ephemeral session, which WebKit gives no storage backing) answers
// `Unsupported` or `Io` rather than silently landing files somewhere else. Scripted runs use
// a persistent browser profile (scripts/ci/webdom-driver.mjs) so CI exercises real OPFS.
// The bridge is the day-part-http callback-id pattern: `day_dom_fs_start` carries the
// operation out under a numeric id; the shim awaits the OPFS promises and re-enters wasm
// EXACTLY once per id through the exports below. Blocking entry points cannot exist on the
// single browser thread (main-thread OPFS is promise-only), so they return `Unsupported` —
// the async twins and futures are the web surface. Like the other shim-bridged parts, using
// this crate on wasm outside a day-dom host page fails at instantiation.
// ---------------------------------------------------------------------------

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use super::{BytesResult, FsError, ListResult, UnitResult};

/// Operation codes shared with the shim (`day_dom_fs_start`).
const OP_READ: u32 = 0;
const OP_WRITE: u32 = 1;
const OP_REMOVE: u32 = 2;
const OP_LIST: u32 = 3;

/// `list` entries cross joined on the unit separator (the file-picker convention).
const SEP: char = '\u{1f}';

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    /// Start an OPFS operation. Buffers are copied out by the shim before it returns.
    fn day_dom_fs_start(
        id: u32,
        op: u32,
        path: *const u8,
        path_len: usize,
        data: *const u8,
        data_len: usize,
    );
}

/// Every completion resolves to bytes: file contents for read, empty for write/remove, the
/// SEP-joined names for list. The typed wrappers decode.
type Callback = Box<dyn FnOnce(BytesResult)>;

thread_local! {
    static PENDING: RefCell<HashMap<u32, Callback>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u32> = const { Cell::new(1) };
}

fn start(op: u32, path: &str, data: &[u8], on_done: Callback) {
    let id = NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id.wrapping_add(1).max(1));
        id
    });
    // Register BEFORE starting: the shim may fail synchronously (no OPFS) and the completion
    // export re-borrows the registry.
    PENDING.with(|p| p.borrow_mut().insert(id, on_done));
    // SAFETY: the pointers reference live borrows for the duration of the call; the shim
    // copies both buffers out before returning.
    unsafe {
        day_dom_fs_start(id, op, path.as_ptr(), path.len(), data.as_ptr(), data.len());
    }
}

pub fn read(_path: &str) -> BytesResult {
    Err(FsError::Unsupported)
}
pub fn write(_path: &str, _bytes: &[u8]) -> UnitResult {
    Err(FsError::Unsupported)
}
pub fn remove(_path: &str) -> UnitResult {
    Err(FsError::Unsupported)
}
pub fn list(_dir: &str) -> ListResult {
    Err(FsError::Unsupported)
}

pub fn read_async(path: String, on_done: Box<dyn FnOnce(BytesResult) + Send>) {
    start(OP_READ, &path, &[], Box::new(on_done));
}

pub fn write_async(path: String, bytes: Vec<u8>, on_done: Box<dyn FnOnce(UnitResult) + Send>) {
    start(
        OP_WRITE,
        &path,
        &bytes,
        Box::new(move |r| on_done(r.map(|_| ()))),
    );
}

pub fn remove_async(path: String, on_done: Box<dyn FnOnce(UnitResult) + Send>) {
    start(
        OP_REMOVE,
        &path,
        &[],
        Box::new(move |r| on_done(r.map(|_| ()))),
    );
}

pub fn list_async(dir: String, on_done: Box<dyn FnOnce(ListResult) + Send>) {
    start(
        OP_LIST,
        &dir,
        &[],
        Box::new(move |r| {
            on_done(r.map(|bytes| {
                let joined = String::from_utf8_lossy(&bytes);
                if joined.is_empty() {
                    Vec::new()
                } else {
                    joined.split(SEP).map(str::to_owned).collect()
                }
            }))
        }),
    );
}

fn complete(id: u32, result: BytesResult) {
    if let Some(cb) = PENDING.with(|p| p.borrow_mut().remove(&id)) {
        cb(result);
    }
}

// ---------------------------------------------------------------------------
// Exports the shim calls back into — the day-part-http alloc-and-consume convention.
// ---------------------------------------------------------------------------

/// Allocate `len` bytes inside wasm memory for the shim to write a completion buffer into
/// (freed by the export that consumes the pointer).
#[unsafe(no_mangle)]
pub extern "C" fn day_fs_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len);
    let ptr = v.as_mut_ptr();
    std::mem::forget(v);
    ptr
}

fn take_buf(ptr: *mut u8, len: usize) -> Vec<u8> {
    if ptr.is_null() || len == 0 {
        return Vec::new();
    }
    // SAFETY: the shim wrote exactly `len` bytes into a `day_fs_alloc(len)` allocation and
    // hands it over exactly once.
    unsafe { Vec::from_raw_parts(ptr, len, len) }
}

/// Success completion: the operation's bytes payload (empty for write/remove).
#[unsafe(no_mangle)]
pub extern "C" fn day_fs_done(id: u32, ptr: *mut u8, len: usize) {
    complete(id, Ok(take_buf(ptr, len)));
}

/// Failure completion. `kind` indexes the web taxonomy (shim.js mirrors it): 1 `NotFound`,
/// 2 `Unsupported` (no OPFS in this context), else `Io` with the provider's message.
#[unsafe(no_mangle)]
pub extern "C" fn day_fs_failed(id: u32, kind: u32, msg_ptr: *mut u8, msg_len: usize) {
    let msg = String::from_utf8_lossy(&take_buf(msg_ptr, msg_len)).into_owned();
    let err = match kind {
        1 => FsError::NotFound,
        2 => FsError::Unsupported,
        _ => FsError::Io(msg),
    };
    complete(id, Err(err));
}
