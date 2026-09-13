// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Windows: WinHTTP (winhttp.dll) — the system HTTP stack: automatic proxy/PAC
// (WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY), schannel TLS + the Windows certificate stores
// (enterprise/AD roots included). Written blind (no Windows host), like the other parts'
// Windows halves: every symbol is resolved dynamically from winhttp.dll, so a missing DLL
// degrades to `Unsupported` instead of failing to load the process.
//
// Two modes. The blocking entry points (`fetch`, `fetch_to_file`, `fetch_streamed`) run a
// synchronous session on the caller's thread, per this crate's contract. The asynchronous ones
// (`fetch_async`, `fetch_future`) run a `WINHTTP_FLAG_ASYNC` session: WinHTTP's own thread pool
// drives the request and reports through a status callback, so no Rust thread is parked behind
// a request, and closing the request handle from any thread is a real cancellation (2026-09;
// before that a worker thread per request blocked in the synchronous arm).
// ---------------------------------------------------------------------------

#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::ffi::c_void;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use super::{Download, HttpError, Request, Response, Tier};

pub const TIER: Tier = Tier::NativeStack;

type HINTERNET = *mut c_void;
type DWORD = u32;
type BOOL = i32;
type LPCWSTR = *const u16;

const WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY: DWORD = 4;
const WINHTTP_FLAG_ASYNC: DWORD = 0x1000_0000;
const WINHTTP_FLAG_SECURE: DWORD = 0x0080_0000;
const WINHTTP_ADDREQ_FLAG_ADD: DWORD = 0x2000_0000;
const WINHTTP_QUERY_STATUS_CODE: DWORD = 19;
const WINHTTP_QUERY_RAW_HEADERS_CRLF: DWORD = 22;
const WINHTTP_QUERY_FLAG_NUMBER: DWORD = 0x2000_0000;

// WINHTTP_CALLBACK_STATUS_* (winhttp.h) — the notifications the asynchronous session acts on.
const WINHTTP_CALLBACK_FLAG_ALL_NOTIFICATIONS: DWORD = 0xFFFF_FFFF;
const STATUS_HANDLE_CLOSING: DWORD = 0x0000_0800;
const STATUS_HEADERS_AVAILABLE: DWORD = 0x0002_0000;
const STATUS_DATA_AVAILABLE: DWORD = 0x0004_0000;
const STATUS_READ_COMPLETE: DWORD = 0x0008_0000;
const STATUS_REQUEST_ERROR: DWORD = 0x0020_0000;
const STATUS_SENDREQUEST_COMPLETE: DWORD = 0x0000_0020;
/// `WINHTTP_INVALID_STATUS_CALLBACK`: what `WinHttpSetStatusCallback` returns on failure.
const INVALID_STATUS_CALLBACK: usize = usize::MAX;

// ERROR_WINHTTP_* (winhttp.h; 12000-base)
const E_TIMEOUT: DWORD = 12002;
const E_INVALID_URL: DWORD = 12005;
const E_UNRECOGNIZED_SCHEME: DWORD = 12006;
const E_NAME_NOT_RESOLVED: DWORD = 12007;
const E_OPERATION_CANCELLED: DWORD = 12017;
const E_CANNOT_CONNECT: DWORD = 12029;
const E_CONNECTION_ERROR: DWORD = 12030;
const SECURE_ERRORS: [DWORD; 7] = [12037, 12038, 12044, 12045, 12057, 12157, 12175];

/// The status callback the asynchronous session installs (`WINHTTP_STATUS_CALLBACK`).
type StatusCallback = unsafe extern "system" fn(HINTERNET, usize, DWORD, *mut c_void, DWORD);

/// `WINHTTP_ASYNC_RESULT`, what `STATUS_REQUEST_ERROR` points at.
#[repr(C)]
struct AsyncResult {
    result: usize,
    error: DWORD,
}

struct Api {
    open: unsafe extern "system" fn(LPCWSTR, DWORD, LPCWSTR, LPCWSTR, DWORD) -> HINTERNET,
    connect: unsafe extern "system" fn(HINTERNET, LPCWSTR, u16, DWORD) -> HINTERNET,
    open_request: unsafe extern "system" fn(
        HINTERNET,
        LPCWSTR,
        LPCWSTR,
        LPCWSTR,
        LPCWSTR,
        *const LPCWSTR,
        DWORD,
    ) -> HINTERNET,
    set_timeouts: unsafe extern "system" fn(HINTERNET, i32, i32, i32, i32) -> BOOL,
    add_headers: unsafe extern "system" fn(HINTERNET, LPCWSTR, DWORD, DWORD) -> BOOL,
    send: unsafe extern "system" fn(
        HINTERNET,
        LPCWSTR,
        DWORD,
        *const c_void,
        DWORD,
        DWORD,
        usize,
    ) -> BOOL,
    receive: unsafe extern "system" fn(HINTERNET, *mut c_void) -> BOOL,
    query_headers: unsafe extern "system" fn(
        HINTERNET,
        DWORD,
        LPCWSTR,
        *mut c_void,
        *mut DWORD,
        *mut DWORD,
    ) -> BOOL,
    query_data: unsafe extern "system" fn(HINTERNET, *mut DWORD) -> BOOL,
    read_data: unsafe extern "system" fn(HINTERNET, *mut c_void, DWORD, *mut DWORD) -> BOOL,
    close: unsafe extern "system" fn(HINTERNET) -> BOOL,
    set_status_callback:
        unsafe extern "system" fn(HINTERNET, Option<StatusCallback>, DWORD, usize) -> usize,
}

unsafe extern "system" {
    fn LoadLibraryW(name: LPCWSTR) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn GetLastError() -> DWORD;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// The `sym!` transmute target is the concrete `Api` fn-pointer field it's assigned to, so an explicit
// `transmute::<_, FnPtr>` per call would just restate that field type — allow the annotation lint for
// this generic GetProcAddress loader.
#[allow(clippy::missing_transmute_annotations)]
fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        let lib = LoadLibraryW(wide("winhttp.dll").as_ptr());
        if lib.is_null() {
            return None;
        }
        macro_rules! sym {
            ($name:literal) => {{
                let p = GetProcAddress(lib, concat!($name, "\0").as_ptr());
                if p.is_null() {
                    return None;
                }
                std::mem::transmute(p)
            }};
        }
        Some(Api {
            open: sym!("WinHttpOpen"),
            connect: sym!("WinHttpConnect"),
            open_request: sym!("WinHttpOpenRequest"),
            set_timeouts: sym!("WinHttpSetTimeouts"),
            add_headers: sym!("WinHttpAddRequestHeaders"),
            send: sym!("WinHttpSendRequest"),
            receive: sym!("WinHttpReceiveResponse"),
            query_headers: sym!("WinHttpQueryHeaders"),
            query_data: sym!("WinHttpQueryDataAvailable"),
            read_data: sym!("WinHttpReadData"),
            close: sym!("WinHttpCloseHandle"),
            set_status_callback: sym!("WinHttpSetStatusCallback"),
        })
    })
    .as_ref()
}

/// Minimal URL split: (https?, host, port, path+query). IPv6 literals and userinfo are out of
/// scope for v1 (docs/http.md).
fn split_url(url: &str) -> Result<(bool, String, u16, String), HttpError> {
    let bad = || HttpError::BadUrl(url.to_string());
    let (secure, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(bad());
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if authority.is_empty() || authority.contains('@') || authority.contains('[') {
        return Err(bad());
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().map_err(|_| bad())?),
        None => (authority.to_string(), if secure { 443 } else { 80 }),
    };
    Ok((secure, host, port, path.to_string()))
}

fn map_error(code: DWORD) -> HttpError {
    match code {
        E_TIMEOUT => HttpError::Timeout,
        E_OPERATION_CANCELLED => HttpError::Cancelled,
        E_NAME_NOT_RESOLVED => HttpError::Dns,
        E_CANNOT_CONNECT | E_CONNECTION_ERROR => HttpError::Connect,
        E_INVALID_URL | E_UNRECOGNIZED_SCHEME => HttpError::BadUrl(format!("winhttp {code}")),
        c if SECURE_ERRORS.contains(&c) => HttpError::Tls(format!("winhttp secure failure {c}")),
        c => HttpError::Io(format!("winhttp error {c}")),
    }
}

/// RAII close for the three WinHTTP handles.
struct Handle<'a>(&'a Api, HINTERNET);
impl Drop for Handle<'_> {
    fn drop(&mut self) {
        if !self.1.is_null() {
            unsafe { (self.0.close)(self.1) };
        }
    }
}

/// `run`'s head callback: sees (status, headers) before the body; returns false to abort. The `'a`
/// keeps the trait object non-`'static` (a bare `dyn` alias would default to `+ 'static`).
type OnHead<'a> = dyn FnMut(u16, &[(String, String)]) -> bool + 'a;
/// `run`'s result: (status, headers, body length).
type RunResult = (u16, Vec<(String, String)>, u64);

/// Run the request; `on_head` sees status+headers before the body (false = abort), `sink`
/// receives body chunks (Vec buffer, file, or a caller StreamSink).
fn run(
    req: &Request,
    on_head: &mut OnHead<'_>,
    sink: &mut dyn FnMut(&[u8]) -> Result<(), HttpError>,
) -> Result<RunResult, HttpError> {
    let api = api().ok_or(HttpError::Unsupported)?;
    let (secure, host, port, path) = split_url(&req.url)?;
    let ms = i32::try_from(req.timeout.as_millis()).unwrap_or(i32::MAX);

    unsafe {
        let session = Handle(
            api,
            (api.open)(
                wide("day-part-http").as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                std::ptr::null(),
                std::ptr::null(),
                0,
            ),
        );
        if session.1.is_null() {
            return Err(map_error(GetLastError()));
        }
        let conn = Handle(api, (api.connect)(session.1, wide(&host).as_ptr(), port, 0));
        if conn.1.is_null() {
            return Err(map_error(GetLastError()));
        }
        let request = Handle(
            api,
            (api.open_request)(
                conn.1,
                wide(req.method.as_str()).as_ptr(),
                wide(&path).as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                if secure { WINHTTP_FLAG_SECURE } else { 0 },
            ),
        );
        if request.1.is_null() {
            return Err(map_error(GetLastError()));
        }
        (api.set_timeouts)(request.1, ms, ms, ms, ms);
        if !req.headers.is_empty() {
            let joined: String = req
                .headers
                .iter()
                .map(|(k, v)| format!("{k}: {v}\r\n"))
                .collect();
            let w = wide(&joined);
            // -1 length = the whole NUL-terminated string.
            if (api.add_headers)(request.1, w.as_ptr(), DWORD::MAX, WINHTTP_ADDREQ_FLAG_ADD) == 0 {
                return Err(map_error(GetLastError()));
            }
        }
        let empty: [u8; 0] = [];
        let body = req.body.as_deref().unwrap_or(&empty);
        let ok = (api.send)(
            request.1,
            std::ptr::null(),
            0,
            body.as_ptr() as *const c_void,
            body.len() as DWORD,
            body.len() as DWORD,
            0,
        );
        if ok == 0 {
            return Err(map_error(GetLastError()));
        }
        if (api.receive)(request.1, std::ptr::null_mut()) == 0 {
            return Err(map_error(GetLastError()));
        }

        let (status, headers) = read_head(api, request.1)?;

        if !on_head(status, &headers) {
            return Err(HttpError::Io("aborted".into()));
        }

        // Body: available/read loop into the sink.
        let mut written: u64 = 0;
        let mut chunk = vec![0u8; 65536];
        loop {
            let mut avail: DWORD = 0;
            if (api.query_data)(request.1, &mut avail) == 0 {
                return Err(map_error(GetLastError()));
            }
            if avail == 0 {
                break;
            }
            let take = (avail as usize).min(chunk.len()) as DWORD;
            let mut read: DWORD = 0;
            if (api.read_data)(
                request.1,
                chunk.as_mut_ptr() as *mut c_void,
                take,
                &mut read,
            ) == 0
            {
                return Err(map_error(GetLastError()));
            }
            if read == 0 {
                break;
            }
            sink(&chunk[..read as usize])?;
            written += read as u64;
        }

        Ok((status, headers, written))
    }
}

/// The response head once WinHTTP has it: the numeric status and the parsed raw headers
/// (`"HTTP/1.1 200 OK\r\nK: V\r\n…"`, first line dropped). Shared by both modes.
unsafe fn read_head(
    api: &Api,
    request: HINTERNET,
) -> Result<(u16, Vec<(String, String)>), HttpError> {
    unsafe {
        let mut status: DWORD = 0;
        let mut len = std::mem::size_of::<DWORD>() as DWORD;
        if (api.query_headers)(
            request,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            std::ptr::null(),
            (&mut status) as *mut DWORD as *mut c_void,
            &mut len,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(map_error(GetLastError()));
        }

        let mut headers = Vec::new();
        let mut hlen: DWORD = 0;
        (api.query_headers)(
            request,
            WINHTTP_QUERY_RAW_HEADERS_CRLF,
            std::ptr::null(),
            std::ptr::null_mut(),
            &mut hlen,
            std::ptr::null_mut(),
        );
        if hlen > 0 {
            let mut buf = vec![0u16; (hlen as usize).div_ceil(2)];
            if (api.query_headers)(
                request,
                WINHTTP_QUERY_RAW_HEADERS_CRLF,
                std::ptr::null(),
                buf.as_mut_ptr() as *mut c_void,
                &mut hlen,
                std::ptr::null_mut(),
            ) != 0
            {
                let raw = String::from_utf16_lossy(&buf[..(hlen as usize) / 2]);
                for line in raw.lines().skip(1) {
                    if let Some((k, v)) = line.split_once(':') {
                        headers.push((k.trim().to_string(), v.trim().to_string()));
                    }
                }
            }
        }
        Ok((status as u16, headers))
    }
}

// ---------------------------------------------------------------------------
// Asynchronous mode
// ---------------------------------------------------------------------------

/// The completion callback of one asynchronous request.
type OnDone = Box<dyn FnOnce(Result<Response, HttpError>) + Send>;

/// What the cancel closure and the status callback share: the request handle, and whether it
/// has been closed. `request` goes null in `STATUS_HANDLE_CLOSING`, the last notification WinHTTP
/// sends for a handle, so a cancel that arrives afterwards finds nothing to close.
struct Grip {
    request: HINTERNET,
    closing: bool,
}

// SAFETY: an HINTERNET is an opaque handle WinHTTP documents as usable from any thread;
// `WinHttpCloseHandle` in particular is the documented way to cancel from another thread.
unsafe impl Send for Grip {}

/// One in-flight asynchronous request. Boxed; its address is the request's context value, and
/// WinHTTP hands it back to the status callback on every notification. Freed in
/// `STATUS_HANDLE_CLOSING`.
struct InFlight {
    api: &'static Api,
    session: HINTERNET,
    conn: HINTERNET,
    grip: Arc<Mutex<Grip>>,
    /// Kept alive for the whole request: `WinHttpSendRequest` reads the body asynchronously.
    body: Vec<u8>,
    status: u16,
    headers: Vec<(String, String)>,
    received: Vec<u8>,
    chunk: Vec<u8>,
    on_done: Option<OnDone>,
}

impl InFlight {
    /// Deliver once; a second call (an error after a delivery, or the cancel racing a
    /// completion) finds nothing. Then close the request handle, whose last notification frees
    /// this state.
    fn finish(&mut self, result: Result<Response, HttpError>) {
        if let Some(cb) = self.on_done.take() {
            cb(result);
        }
        let request = {
            let mut g = lock(&self.grip);
            if g.closing {
                std::ptr::null_mut()
            } else {
                g.closing = true;
                g.request
            }
        };
        if !request.is_null() {
            unsafe { (self.api.close)(request) };
        }
    }

    fn fail(&mut self, code: DWORD) {
        self.finish(Err(map_error(code)));
    }

    /// Ask for the next piece of the body; the answer arrives as `STATUS_DATA_AVAILABLE`.
    fn query_more(&mut self, request: HINTERNET) {
        if unsafe { (self.api.query_data)(request, std::ptr::null_mut()) } == 0 {
            let code = unsafe { GetLastError() };
            self.fail(code);
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The asynchronous session's status callback, on one of WinHTTP's threads. `context` is the
/// `InFlight` address for request handles and 0 for the session and connection handles, whose
/// notifications carry nothing this arm acts on.
unsafe extern "system" fn on_status(
    handle: HINTERNET,
    context: usize,
    status: DWORD,
    info: *mut c_void,
    info_len: DWORD,
) {
    if context == 0 {
        return;
    }
    let state = context as *mut InFlight;
    // SAFETY: `context` was set from `Box::into_raw` in `fetch_async_cancellable` and is freed
    // only in `STATUS_HANDLE_CLOSING` below, the last notification for the handle.
    let st = unsafe { &mut *state };
    match status {
        STATUS_SENDREQUEST_COMPLETE => {
            if unsafe { (st.api.receive)(handle, std::ptr::null_mut()) } == 0 {
                let code = unsafe { GetLastError() };
                st.fail(code);
            }
        }
        STATUS_HEADERS_AVAILABLE => match unsafe { read_head(st.api, handle) } {
            Ok((code, headers)) => {
                st.status = code;
                st.headers = headers;
                st.query_more(handle);
            }
            Err(e) => st.finish(Err(e)),
        },
        STATUS_DATA_AVAILABLE => {
            // SAFETY: WinHTTP documents `info` as a DWORD for this notification.
            let avail = if info.is_null() {
                0
            } else {
                unsafe { *(info as *const DWORD) }
            };
            if avail == 0 {
                let response = Response {
                    status: st.status,
                    headers: std::mem::take(&mut st.headers),
                    body: std::mem::take(&mut st.received),
                };
                st.finish(Ok(response));
            } else {
                let take = (avail as usize).min(st.chunk.len()) as DWORD;
                let ok = unsafe {
                    (st.api.read_data)(
                        handle,
                        st.chunk.as_mut_ptr() as *mut c_void,
                        take,
                        std::ptr::null_mut(),
                    )
                };
                if ok == 0 {
                    let code = unsafe { GetLastError() };
                    st.fail(code);
                }
            }
        }
        STATUS_READ_COMPLETE => {
            let read = info_len as usize;
            if read == 0 {
                let response = Response {
                    status: st.status,
                    headers: std::mem::take(&mut st.headers),
                    body: std::mem::take(&mut st.received),
                };
                st.finish(Ok(response));
            } else {
                let n = read.min(st.chunk.len());
                let (chunk, received) = (&st.chunk, &mut st.received);
                received.extend_from_slice(&chunk[..n]);
                st.query_more(handle);
            }
        }
        STATUS_REQUEST_ERROR => {
            // SAFETY: WinHTTP documents `info` as a WINHTTP_ASYNC_RESULT for this notification.
            let code = if info.is_null() {
                E_CONNECTION_ERROR
            } else {
                unsafe { (*(info as *const AsyncResult)).error }
            };
            st.fail(code);
        }
        STATUS_HANDLE_CLOSING => {
            // The last notification: the request handle is gone. A cancel that beat every
            // completion is delivered here, so the caller always hears exactly one answer.
            {
                let mut g = lock(&st.grip);
                g.request = std::ptr::null_mut();
                g.closing = true;
            }
            if let Some(cb) = st.on_done.take() {
                cb(Err(HttpError::Cancelled));
            }
            unsafe {
                (st.api.close)(st.conn);
                (st.api.close)(st.session);
                // SAFETY: allocated by `Box::into_raw`; no notification follows this one.
                drop(Box::from_raw(state));
            }
        }
        _ => {}
    }
}

/// Start `req` on an asynchronous session; `on_done` runs on a WinHTTP thread with the result.
/// Returns the cancel closure, or `None` when the request never started (the callback has
/// then already been called with the error).
pub fn fetch_async_cancellable(req: Request, on_done: OnDone) -> Option<Box<dyn FnOnce() + Send>> {
    match start_async(&req, on_done) {
        Ok(grip) => Some(Box::new(move || {
            let request = {
                let mut g = lock(&grip);
                if g.closing {
                    std::ptr::null_mut()
                } else {
                    g.closing = true;
                    g.request
                }
            };
            if !request.is_null()
                && let Some(api) = api()
            {
                // Closing the handle from another thread is WinHTTP's cancellation: the
                // callback sees REQUEST_ERROR (12017) or HANDLE_CLOSING and delivers Cancelled.
                unsafe { (api.close)(request) };
            }
        })),
        Err((cb, e)) => {
            cb(Err(e));
            None
        }
    }
}

/// Fire-and-forget asynchronous start.
pub fn fetch_async(req: Request, on_done: OnDone) {
    let _ = fetch_async_cancellable(req, on_done);
}

/// Open the asynchronous session and send. On failure the callback comes back with the error,
/// so the caller can still deliver it exactly once.
fn start_async(req: &Request, on_done: OnDone) -> Result<Arc<Mutex<Grip>>, (OnDone, HttpError)> {
    let Some(api) = api() else {
        return Err((on_done, HttpError::Unsupported));
    };
    let (secure, host, port, path) = match split_url(&req.url) {
        Ok(parts) => parts,
        Err(e) => return Err((on_done, e)),
    };
    let ms = i32::try_from(req.timeout.as_millis()).unwrap_or(i32::MAX);

    unsafe {
        let session = (api.open)(
            wide("day-part-http").as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            std::ptr::null(),
            std::ptr::null(),
            WINHTTP_FLAG_ASYNC,
        );
        if session.is_null() {
            return Err((on_done, map_error(GetLastError())));
        }
        // Every handle opened under the session inherits the callback.
        if (api.set_status_callback)(
            session,
            Some(on_status),
            WINHTTP_CALLBACK_FLAG_ALL_NOTIFICATIONS,
            0,
        ) == INVALID_STATUS_CALLBACK
        {
            let code = GetLastError();
            (api.close)(session);
            return Err((on_done, map_error(code)));
        }
        let conn = (api.connect)(session, wide(&host).as_ptr(), port, 0);
        if conn.is_null() {
            let code = GetLastError();
            (api.close)(session);
            return Err((on_done, map_error(code)));
        }
        let request = (api.open_request)(
            conn,
            wide(req.method.as_str()).as_ptr(),
            wide(&path).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            if secure { WINHTTP_FLAG_SECURE } else { 0 },
        );
        if request.is_null() {
            let code = GetLastError();
            (api.close)(conn);
            (api.close)(session);
            return Err((on_done, map_error(code)));
        }
        (api.set_timeouts)(request, ms, ms, ms, ms);
        if !req.headers.is_empty() {
            let joined: String = req
                .headers
                .iter()
                .map(|(k, v)| format!("{k}: {v}\r\n"))
                .collect();
            let w = wide(&joined);
            if (api.add_headers)(request, w.as_ptr(), DWORD::MAX, WINHTTP_ADDREQ_FLAG_ADD) == 0 {
                let code = GetLastError();
                (api.close)(request);
                (api.close)(conn);
                (api.close)(session);
                return Err((on_done, map_error(code)));
            }
        }

        let grip = Arc::new(Mutex::new(Grip {
            request,
            closing: false,
        }));
        let state = Box::new(InFlight {
            api,
            session,
            conn,
            grip: grip.clone(),
            body: req.body.clone().unwrap_or_default(),
            status: 0,
            headers: Vec::new(),
            received: Vec::new(),
            chunk: vec![0u8; 65536],
            on_done: Some(on_done),
        });
        let context = Box::into_raw(state);
        let st = &*context;
        let ok = (api.send)(
            request,
            std::ptr::null(),
            0,
            st.body.as_ptr() as *const c_void,
            st.body.len() as DWORD,
            st.body.len() as DWORD,
            context as usize,
        );
        if ok == 0 {
            // Never sent: no notification will come for this handle, so take the state back
            // and close everything here.
            let code = GetLastError();
            let mut st = Box::from_raw(context);
            (api.close)(request);
            (api.close)(conn);
            (api.close)(session);
            let cb = st.on_done.take().expect("set just above");
            return Err((cb, map_error(code)));
        }
        Ok(grip)
    }
}

pub fn fetch(req: &Request) -> Result<Response, HttpError> {
    let mut body = Vec::new();
    let (status, headers, _) = run(req, &mut |_, _| true, &mut |chunk| {
        body.extend_from_slice(chunk);
        Ok(())
    })?;
    Ok(Response {
        status,
        headers,
        body,
    })
}

pub fn fetch_to_file(req: &Request, dest: &Path) -> Result<Download, HttpError> {
    use std::io::Write;
    let mut file = std::fs::File::create(dest).map_err(|e| HttpError::Io(e.to_string()))?;
    let (status, headers, bytes_written) = run(req, &mut |_, _| true, &mut |chunk| {
        file.write_all(chunk)
            .map_err(|e| HttpError::Io(e.to_string()))
    })?;
    Ok(Download {
        status,
        headers,
        bytes_written,
    })
}

pub fn fetch_streamed(
    req: &Request,
    sink: &mut dyn super::StreamSink,
) -> Result<Download, HttpError> {
    // Two-phase borrow: run() takes separate head/chunk callbacks over the one sink.
    let sink = std::cell::RefCell::new(sink);
    let (status, headers, bytes_written) = run(
        req,
        &mut |status, headers| sink.borrow_mut().head(status, headers),
        &mut |chunk| sink.borrow_mut().chunk(chunk),
    )?;
    Ok(Download {
        status,
        headers,
        bytes_written,
    })
}
