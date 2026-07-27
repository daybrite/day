// ---------------------------------------------------------------------------
// The web (web-dom): the browser's own fetch() through the day-dom shim — the platform stack
// of this target (the browser supplies proxies, TLS, the certificate store, HTTP/2/3), so the
// tier is NativeStack even though only the ASYNC entry points exist: wasm has one thread and
// no blocking waits, so `fetch`/`fetch_to_file`/`fetch_streamed` return `Unsupported` here
// (docs/http.md's matrix) — a blocking wait would deadlock the event loop the completion
// needs.
//
// The bridge is the day-dom callback-id pattern (docs/web.md, the day-part-prefs precedent):
// `day_dom_http_start` carries the request out under a numeric id; the shim runs fetch() with
// an AbortController (plus a timer realizing `Request::timeout` over connect + response head,
// fallback-tier parity) and re-enters wasm EXACTLY once per id through the exports below.
// Like prefs, using this crate on wasm outside a day-dom host page fails at instantiation
// (the imports are unresolved); the web target is `web-dom` (docs/web.md).
// ---------------------------------------------------------------------------

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;

use super::{Download, HttpError, Request, Response, StreamSink, Tier};

pub const TIER: Tier = Tier::NativeStack;

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    /// Start a browser fetch. Buffers are copied out by the shim before it returns (wasm
    /// memory may move under a later await). `has_body` distinguishes "no body" from an
    /// empty one; `timeout_ms` realizes [`Request::timeout`].
    fn day_dom_http_start(
        id: u32,
        method: *const u8,
        method_len: usize,
        url: *const u8,
        url_len: usize,
        headers: *const u8,
        headers_len: usize,
        body: *const u8,
        body_len: usize,
        has_body: u32,
        timeout_ms: f64,
    );
    /// Abort the in-flight fetch for `id` (its AbortController); the completion then arrives
    /// as [`HttpError::Cancelled`]. Unknown/finished ids are a no-op.
    fn day_dom_http_abort(id: u32);
}

type FetchResult = Result<Response, HttpError>;
type Callback = Box<dyn FnOnce(FetchResult) + Send>;

thread_local! {
    /// In-flight completions by request id. wasm is single-threaded, so a thread-local slab
    /// is the whole registry (the same shape day-dom uses for dialogs and timers).
    static PENDING: RefCell<HashMap<u32, Callback>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u32> = const { Cell::new(1) };
}

/// Headers cross the boundary as a flat `u32-LE key-len, key, u32-LE value-len, value`
/// record stream, both directions (shim.js mirrors it) — no JSON, so no escaping concerns,
/// and order + duplicates survive byte-exact.
fn encode_headers(headers: &[(String, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (k, v) in headers {
        out.extend_from_slice(&(k.len() as u32).to_le_bytes());
        out.extend_from_slice(k.as_bytes());
        out.extend_from_slice(&(v.len() as u32).to_le_bytes());
        out.extend_from_slice(v.as_bytes());
    }
    out
}

fn decode_headers(buf: &[u8]) -> Vec<(String, String)> {
    fn field<'b>(buf: &'b [u8], i: &mut usize) -> Option<&'b [u8]> {
        let len = u32::from_le_bytes(buf.get(*i..*i + 4)?.try_into().ok()?) as usize;
        *i += 4;
        let bytes = buf.get(*i..*i + len)?;
        *i += len;
        Some(bytes)
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < buf.len() {
        let Some(k) = field(buf, &mut i) else { break };
        let Some(v) = field(buf, &mut i) else { break };
        out.push((
            String::from_utf8_lossy(k).into_owned(),
            String::from_utf8_lossy(v).into_owned(),
        ));
    }
    out
}

/// Blocking entry points cannot exist on the browser's single thread — the wait would starve
/// the event loop that delivers the completion. Use `fetch_async`/`fetch_future`.
pub fn fetch(_req: &Request) -> Result<Response, HttpError> {
    Err(HttpError::Unsupported)
}

/// No filesystem in the browser sandbox (docs/web.md).
pub fn fetch_to_file(_req: &Request, _dest: &Path) -> Result<Download, HttpError> {
    Err(HttpError::Unsupported)
}

/// The streaming trait is blocking by design; see `fetch` above.
pub fn fetch_streamed(_req: &Request, _sink: &mut dyn StreamSink) -> Result<Download, HttpError> {
    Err(HttpError::Unsupported)
}

/// Natively async on the browser event loop; `on_done` runs on the UI thread (the only
/// thread there is).
pub fn fetch_async(req: Request, on_done: Callback) {
    let _ = fetch_async_cancellable(req, on_done);
}

/// [`fetch_async`] plus a cancel closure firing the shim's AbortController — a real platform
/// cancel (docs/http.md's cancel matrix); the completion then arrives as `Cancelled`.
pub fn fetch_async_cancellable(
    req: Request,
    on_done: Callback,
) -> Option<Box<dyn FnOnce() + Send>> {
    let id = NEXT_ID.with(|n| {
        let id = n.get();
        // Skip 0 on wrap: 0 would read as "unset" in a debugger; ids recycle after 4 billion
        // requests, far past any overlap with a live one.
        n.set(id.wrapping_add(1).max(1));
        id
    });
    // Register BEFORE starting: the shim may deliver a failure (bad URL) synchronously, and
    // the completion export re-borrows the registry.
    PENDING.with(|p| p.borrow_mut().insert(id, on_done));
    let headers = encode_headers(&req.headers);
    let (body_ptr, body_len, has_body) = match &req.body {
        Some(b) => (b.as_ptr(), b.len(), 1u32),
        None => (std::ptr::null(), 0, 0u32),
    };
    let method = req.method.as_str();
    // SAFETY: every pointer references memory owned by `req`/`headers`, alive for the whole
    // call; the shim copies the buffers out before returning.
    unsafe {
        day_dom_http_start(
            id,
            method.as_ptr(),
            method.len(),
            req.url.as_ptr(),
            req.url.len(),
            headers.as_ptr(),
            headers.len(),
            body_ptr,
            body_len,
            has_body,
            req.timeout.as_secs_f64() * 1000.0,
        );
    }
    Some(Box::new(move || unsafe { day_dom_http_abort(id) }))
}

fn complete(id: u32, result: FetchResult) {
    // A missing id means this completion lost a race with one already delivered (the shim
    // completes exactly once, so in practice: never) — dropping it is the safe answer.
    if let Some(cb) = PENDING.with(|p| p.borrow_mut().remove(&id)) {
        cb(result);
    }
}

// ---------------------------------------------------------------------------
// Exports the shim calls back into. Buffers arrive in `day_http_alloc` allocations and are
// owned (and freed) here — the same alloc-and-consume convention as day-dom's
// `day_dom_alloc`/`take_string`.
// ---------------------------------------------------------------------------

/// Allocate `len` bytes inside wasm memory for the shim to write a completion buffer into
/// (freed by the export that consumes the pointer).
#[unsafe(no_mangle)]
pub extern "C" fn day_http_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len);
    let ptr = v.as_mut_ptr();
    std::mem::forget(v);
    ptr
}

fn take_buf(ptr: *mut u8, len: usize) -> Vec<u8> {
    if ptr.is_null() || len == 0 {
        return Vec::new();
    }
    // SAFETY: the shim wrote exactly `len` bytes into a `day_http_alloc(len)` allocation and
    // hands it over exactly once.
    unsafe { Vec::from_raw_parts(ptr, len, len) }
}

/// Success completion: HTTP status (0 for non-HTTP responses), the flat header records, and
/// the buffered body. 4xx/5xx arrive HERE, per the crate contract — not as errors.
#[unsafe(no_mangle)]
pub extern "C" fn day_http_done(
    id: u32,
    status: u32,
    headers_ptr: *mut u8,
    headers_len: usize,
    body_ptr: *mut u8,
    body_len: usize,
) {
    let headers = decode_headers(&take_buf(headers_ptr, headers_len));
    let body = take_buf(body_ptr, body_len);
    complete(
        id,
        Ok(Response {
            status: status as u16,
            headers,
            body,
        }),
    );
}

/// Failure completion. `kind` indexes the web taxonomy (shim.js mirrors it): 1 `BadUrl`,
/// 2 `Timeout`, 3 `Cancelled`, else `Io` — the browser deliberately collapses DNS / connect
/// / TLS detail into one opaque network failure (docs/http.md's error mapping).
#[unsafe(no_mangle)]
pub extern "C" fn day_http_failed(id: u32, kind: u32, msg_ptr: *mut u8, msg_len: usize) {
    let msg = String::from_utf8_lossy(&take_buf(msg_ptr, msg_len)).into_owned();
    let err = match kind {
        1 => HttpError::BadUrl(msg),
        2 => HttpError::Timeout,
        3 => HttpError::Cancelled,
        _ => HttpError::Io(msg),
    };
    complete(id, Err(err));
}
