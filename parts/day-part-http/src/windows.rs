// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Windows: WinHTTP (winhttp.dll), the system HTTP stack, as the transport beneath the portable
// client: automatic proxy and PAC (WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY), schannel TLS with the
// Windows certificate stores (enterprise and AD roots included), HTTP/2 where the system offers
// it. Every symbol is resolved at run time from winhttp.dll and crypt32.dll, so a missing library
// degrades to `Unsupported` instead of failing to load the process.
//
// One asynchronous session per client (WINHTTP_FLAG_ASYNC): WinHTTP's own thread pool drives each
// exchange and reports through one status callback, so no Rust thread is parked behind a request.
// Redirects come back to the client (WINHTTP_OPTION_REDIRECT_POLICY never), cookies stay with the
// client (WINHTTP_DISABLE_COOKIES), and a body is read only while the client's demand lasts: one
// WinHttpReadData at a time, the next one started by the completion or by a later grant. Uploads
// go out through WinHttpWriteData in pieces, reporting progress, with chunked framing written here
// when the length is unknown. WebSockets upgrade a request (WINHTTP_OPTION_UPGRADE_TO_WEB_SOCKET)
// and then run on WinHTTP's WebSocket calls, receiving only while demand lasts.
//
// Closing a handle is WinHTTP's cancellation from any thread. Each handle carries a boxed context
// that STATUS_HANDLE_CLOSING, the handle's last notification, frees.
// ---------------------------------------------------------------------------

#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::cell::UnsafeCell;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::io::Read;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::client::{
    Answer, Capabilities, Completion, Event, Events, Head, Message, Metrics, Prepared,
    PreparedBody, QuestionId, Socket, Transfer, Transport, TransportConfig, WsEvent, WsEvents,
};
use crate::{HttpError, Identity, Tier};

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
/// `WINHTTP_IGNORE_REQUEST_TOTAL_LENGTH`: the length travels in a header, or the body is chunked.
const IGNORE_TOTAL_LENGTH: DWORD = 0;

// WINHTTP_OPTION_* (winhttp.h)
const OPTION_CONTEXT_VALUE: DWORD = 45;
const OPTION_CLIENT_CERT_CONTEXT: DWORD = 47;
const OPTION_DISABLE_FEATURE: DWORD = 63;
const OPTION_MAX_CONNS_PER_SERVER: DWORD = 73;
const OPTION_MAX_CONNS_PER_1_0_SERVER: DWORD = 74;
const OPTION_REDIRECT_POLICY: DWORD = 88;
const OPTION_UPGRADE_TO_WEB_SOCKET: DWORD = 114;
const OPTION_ENABLE_HTTP_PROTOCOL: DWORD = 133;
const OPTION_HTTP_PROTOCOL_USED: DWORD = 134;
const REDIRECT_POLICY_NEVER: DWORD = 0;
const DISABLE_COOKIES: DWORD = 1;
const PROTOCOL_FLAG_HTTP2: DWORD = 0x1;
const PROTOCOL_FLAG_HTTP3: DWORD = 0x2;

// WINHTTP_CALLBACK_STATUS_* (winhttp.h): the notifications the transport acts on.
const WINHTTP_CALLBACK_FLAG_ALL_NOTIFICATIONS: DWORD = 0xFFFF_FFFF;
const STATUS_HANDLE_CLOSING: DWORD = 0x0000_0800;
const STATUS_SENDREQUEST_COMPLETE: DWORD = 0x0040_0000;
const STATUS_HEADERS_AVAILABLE: DWORD = 0x0002_0000;
const STATUS_READ_COMPLETE: DWORD = 0x0008_0000;
const STATUS_REQUEST_ERROR: DWORD = 0x0020_0000;
const STATUS_WRITE_COMPLETE: DWORD = 0x0010_0000;
const STATUS_CLOSE_COMPLETE: DWORD = 0x0200_0000;
/// `WINHTTP_INVALID_STATUS_CALLBACK`: what `WinHttpSetStatusCallback` returns on failure.
const INVALID_STATUS_CALLBACK: usize = usize::MAX;

// WINHTTP_WEB_SOCKET_BUFFER_TYPE
const WS_BINARY_MESSAGE: u32 = 0;
const WS_BINARY_FRAGMENT: u32 = 1;
const WS_UTF8_MESSAGE: u32 = 2;
const WS_UTF8_FRAGMENT: u32 = 3;
const WS_CLOSE: u32 = 4;

// ERROR_WINHTTP_* (winhttp.h; 12000-base)
const E_TIMEOUT: DWORD = 12002;
const E_INVALID_URL: DWORD = 12005;
const E_UNRECOGNIZED_SCHEME: DWORD = 12006;
const E_NAME_NOT_RESOLVED: DWORD = 12007;
const E_OPERATION_CANCELLED: DWORD = 12017;
const E_CANNOT_CONNECT: DWORD = 12029;
const E_CONNECTION_ERROR: DWORD = 12030;
const SECURE_ERRORS: [DWORD; 7] = [12037, 12038, 12044, 12045, 12057, 12157, 12175];

// crypt32.dll
const PKCS12_NO_PERSIST_KEY: DWORD = 0x0000_8000;
const ENCODING_X509_PKCS7: DWORD = 0x0001_0001;
const CERT_FIND_ANY: DWORD = 0;
const CERT_FIND_HAS_PRIVATE_KEY: DWORD = 0x0015_0000;

/// How much one read, write or WebSocket receive moves.
const PIECE: usize = 64 << 10;

/// The status callback the session installs (`WINHTTP_STATUS_CALLBACK`).
type StatusCallback = unsafe extern "system" fn(HINTERNET, usize, DWORD, *mut c_void, DWORD);

/// `WINHTTP_ASYNC_RESULT`, what `STATUS_REQUEST_ERROR` points at.
// Laid out for the C side, which reads fields Rust never does.
#[allow(dead_code)]
#[repr(C)]
struct AsyncResult {
    result: usize,
    error: DWORD,
}

/// `WINHTTP_WEB_SOCKET_STATUS`, what a WebSocket receive or send completion points at.
#[repr(C)]
struct WsStatus {
    bytes: DWORD,
    buffer_type: u32,
}

/// `CERT_CONTEXT`, whose size `WINHTTP_OPTION_CLIENT_CERT_CONTEXT` takes.
// Laid out for the C side, which reads fields Rust never does.
#[allow(dead_code)]
#[repr(C)]
struct CertContext {
    encoding: DWORD,
    encoded: *const u8,
    encoded_len: DWORD,
    info: *const c_void,
    store: *mut c_void,
}

/// `CRYPT_DATA_BLOB`.
// Laid out for the C side, which reads fields Rust never does.
#[allow(dead_code)]
#[repr(C)]
struct DataBlob {
    len: DWORD,
    data: *const u8,
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
    set_option: unsafe extern "system" fn(HINTERNET, DWORD, *const c_void, DWORD) -> BOOL,
    query_option: unsafe extern "system" fn(HINTERNET, DWORD, *mut c_void, *mut DWORD) -> BOOL,
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
    write_data: unsafe extern "system" fn(HINTERNET, *const c_void, DWORD, *mut DWORD) -> BOOL,
    receive: unsafe extern "system" fn(HINTERNET, *mut c_void) -> BOOL,
    query_headers: unsafe extern "system" fn(
        HINTERNET,
        DWORD,
        LPCWSTR,
        *mut c_void,
        *mut DWORD,
        *mut DWORD,
    ) -> BOOL,
    read_data: unsafe extern "system" fn(HINTERNET, *mut c_void, DWORD, *mut DWORD) -> BOOL,
    close: unsafe extern "system" fn(HINTERNET) -> BOOL,
    set_status_callback:
        unsafe extern "system" fn(HINTERNET, Option<StatusCallback>, DWORD, usize) -> usize,
    /// The WebSocket calls, present since Windows 8.
    ws: Option<WsApi>,
}

struct WsApi {
    complete_upgrade: unsafe extern "system" fn(HINTERNET, usize) -> HINTERNET,
    send: unsafe extern "system" fn(HINTERNET, u32, *const c_void, DWORD) -> DWORD,
    receive:
        unsafe extern "system" fn(HINTERNET, *mut c_void, DWORD, *mut DWORD, *mut u32) -> DWORD,
    close: unsafe extern "system" fn(HINTERNET, u16, *const c_void, DWORD) -> DWORD,
    query_close_status:
        unsafe extern "system" fn(HINTERNET, *mut u16, *mut c_void, DWORD, *mut DWORD) -> DWORD,
}

struct Crypt {
    pfx_import: unsafe extern "system" fn(*const DataBlob, LPCWSTR, DWORD) -> *mut c_void,
    find_certificate: unsafe extern "system" fn(
        *mut c_void,
        DWORD,
        DWORD,
        DWORD,
        *const c_void,
        *const CertContext,
    ) -> *const CertContext,
    free_certificate: unsafe extern "system" fn(*const CertContext) -> BOOL,
    close_store: unsafe extern "system" fn(*mut c_void, DWORD) -> BOOL,
}

// SAFETY: function pointers into system libraries that stay loaded for the life of the process.
unsafe impl Send for Api {}
unsafe impl Sync for Api {}
unsafe impl Send for Crypt {}
unsafe impl Sync for Crypt {}

unsafe extern "system" {
    fn LoadLibraryW(name: LPCWSTR) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn GetLastError() -> DWORD;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Look a symbol up as a function pointer.
///
/// # Safety
/// `T` must be the pointer type matching the symbol's signature.
unsafe fn symbol<T: Copy>(library: *mut c_void, name: &str) -> Option<T> {
    if std::mem::size_of::<T>() != std::mem::size_of::<*mut c_void>() {
        return None;
    }
    let name: Vec<u8> = name.bytes().chain(std::iter::once(0)).collect();
    // SAFETY: a module handle LoadLibraryW returned and a NUL-terminated name.
    let address = unsafe { GetProcAddress(library, name.as_ptr()) };
    if address.is_null() {
        return None;
    }
    // SAFETY: same size, and the caller names the symbol's true type.
    Some(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&address) })
}

fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(|| {
        // SAFETY: a system library name; a missing library returns null.
        let lib = unsafe { LoadLibraryW(wide("winhttp.dll").as_ptr()) };
        if lib.is_null() {
            return None;
        }
        // SAFETY: each symbol is looked up as its documented signature.
        unsafe {
            let ws = (|| {
                Some(WsApi {
                    complete_upgrade: symbol(lib, "WinHttpWebSocketCompleteUpgrade")?,
                    send: symbol(lib, "WinHttpWebSocketSend")?,
                    receive: symbol(lib, "WinHttpWebSocketReceive")?,
                    close: symbol(lib, "WinHttpWebSocketClose")?,
                    query_close_status: symbol(lib, "WinHttpWebSocketQueryCloseStatus")?,
                })
            })();
            Some(Api {
                open: symbol(lib, "WinHttpOpen")?,
                connect: symbol(lib, "WinHttpConnect")?,
                open_request: symbol(lib, "WinHttpOpenRequest")?,
                set_timeouts: symbol(lib, "WinHttpSetTimeouts")?,
                set_option: symbol(lib, "WinHttpSetOption")?,
                query_option: symbol(lib, "WinHttpQueryOption")?,
                add_headers: symbol(lib, "WinHttpAddRequestHeaders")?,
                send: symbol(lib, "WinHttpSendRequest")?,
                write_data: symbol(lib, "WinHttpWriteData")?,
                receive: symbol(lib, "WinHttpReceiveResponse")?,
                query_headers: symbol(lib, "WinHttpQueryHeaders")?,
                read_data: symbol(lib, "WinHttpReadData")?,
                close: symbol(lib, "WinHttpCloseHandle")?,
                set_status_callback: symbol(lib, "WinHttpSetStatusCallback")?,
                ws,
            })
        }
    })
    .as_ref()
}

fn crypt() -> Option<&'static Crypt> {
    static CRYPT: OnceLock<Option<Crypt>> = OnceLock::new();
    CRYPT
        .get_or_init(|| {
            // SAFETY: a system library name; a missing library returns null.
            let lib = unsafe { LoadLibraryW(wide("crypt32.dll").as_ptr()) };
            if lib.is_null() {
                return None;
            }
            // SAFETY: each symbol is looked up as its documented signature.
            unsafe {
                Some(Crypt {
                    pfx_import: symbol(lib, "PFXImportCertStore")?,
                    find_certificate: symbol(lib, "CertFindCertificateInStore")?,
                    free_certificate: symbol(lib, "CertFreeCertificateContext")?,
                    close_store: symbol(lib, "CertCloseStore")?,
                })
            }
        })
        .as_ref()
}

/// What WinHTTP offers on this system; nothing when winhttp.dll does not load.
pub(crate) fn capabilities() -> Capabilities {
    let Some(api) = api() else {
        return Capabilities::default();
    };
    let websockets = api.ws.is_some();
    Capabilities {
        streaming: true,
        upload_streaming: true,
        upload_progress: true,
        manual_redirects: true,
        auth_questions: false,
        native_auth_schemes: false,
        server_trust: false,
        client_identity: crypt().is_some(),
        platform_cookies: false,
        platform_cache: false,
        metrics: true,
        websockets,
        websocket_ping: false,
        websocket_headers: websockets,
        wait_for_connectivity: false,
    }
}

/// The transport for one client: an asynchronous session with the client's settings.
pub(crate) fn transport(config: &TransportConfig) -> Arc<dyn Transport> {
    Arc::new(WinTransport {
        session: Session::open(config),
        identity: config.identity.as_ref().map(ClientCert::import),
    })
}

/// Minimal URL split: (secure, host, port, path and query). `ws` and `wss` map onto `http` and
/// `https`. IPv6 literals and userinfo are not taken (docs/http.md).
fn split_url(url: &str) -> Result<(bool, String, u16, String), HttpError> {
    let bad = || HttpError::BadUrl(url.to_string());
    let (secure, rest) = [
        ("https://", true),
        ("wss://", true),
        ("http://", false),
        ("ws://", false),
    ]
    .iter()
    .find_map(|(scheme, secure)| {
        (url.len() >= scheme.len() && url[..scheme.len()].eq_ignore_ascii_case(scheme))
            .then(|| (*secure, &url[scheme.len()..]))
    })
    .ok_or_else(bad)?;
    let (authority, path) = match rest.find(['/', '?']) {
        Some(i) if rest.as_bytes()[i] == b'?' => (&rest[..i], format!("/{}", &rest[i..])),
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/".to_string()),
    };
    let path = path.split('#').next().unwrap_or("/").to_string();
    if authority.is_empty() || authority.contains('@') || authority.contains('[') {
        return Err(bad());
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().map_err(|_| bad())?),
        None => (authority.to_string(), if secure { 443 } else { 80 }),
    };
    Ok((secure, host, port, path))
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

/// The last error WinHTTP recorded on this thread.
fn last_error() -> HttpError {
    // SAFETY: reads this thread's last-error value.
    map_error(unsafe { GetLastError() })
}

fn set_dword(api: &Api, handle: HINTERNET, option: DWORD, value: DWORD) -> bool {
    // SAFETY: a DWORD option reads `size_of::<DWORD>()` bytes from the pointer for the call.
    unsafe {
        (api.set_option)(
            handle,
            option,
            (&value as *const DWORD).cast(),
            std::mem::size_of::<DWORD>() as DWORD,
        ) != 0
    }
}

/// One asynchronous session: a client's exchanges share its connection pool, and each keeps it
/// alive until its own handles are gone.
struct Session {
    api: &'static Api,
    handle: HINTERNET,
}

// SAFETY: an HINTERNET is an opaque handle WinHTTP documents as usable from any thread.
unsafe impl Send for Session {}
unsafe impl Sync for Session {}

impl Session {
    fn open(config: &TransportConfig) -> Result<Arc<Session>, HttpError> {
        let api = api().ok_or(HttpError::Unsupported)?;
        // SAFETY: NUL-terminated agent string; null proxy name and bypass for automatic proxy.
        let handle = unsafe {
            (api.open)(
                wide("day-part-http").as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                std::ptr::null(),
                std::ptr::null(),
                WINHTTP_FLAG_ASYNC,
            )
        };
        if handle.is_null() {
            return Err(last_error());
        }
        let session = Arc::new(Session { api, handle });
        // Every handle opened under the session inherits the callback.
        // SAFETY: a live session handle and a callback that lives for the whole process.
        let previous = unsafe {
            (api.set_status_callback)(
                handle,
                Some(on_status),
                WINHTTP_CALLBACK_FLAG_ALL_NOTIFICATIONS,
                0,
            )
        };
        if previous == INVALID_STATUS_CALLBACK {
            return Err(last_error());
        }
        // HTTP/2 arrived in Windows 10 1607; an older system keeps HTTP/1.1.
        set_dword(
            api,
            handle,
            OPTION_ENABLE_HTTP_PROTOCOL,
            PROTOCOL_FLAG_HTTP2,
        );
        if let Some(limit) = config.max_per_host {
            set_dword(api, handle, OPTION_MAX_CONNS_PER_SERVER, limit.max(1));
            set_dword(api, handle, OPTION_MAX_CONNS_PER_1_0_SERVER, limit.max(1));
        }
        Ok(session)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: the last reference to the session; every exchange under it is gone.
        unsafe { (self.api.close)(self.handle) };
    }
}

/// A client identity, imported once per client and kept for its exchanges.
struct ClientCert {
    crypt: &'static Crypt,
    store: *mut c_void,
    context: *const CertContext,
}

// SAFETY: a certificate context and store are immutable once imported; crypt32 documents both
// as usable from any thread.
unsafe impl Send for ClientCert {}
unsafe impl Sync for ClientCert {}

impl ClientCert {
    fn import(identity: &Identity) -> Result<Arc<ClientCert>, HttpError> {
        let crypt = crypt().ok_or(HttpError::Unsupported)?;
        let blob = DataBlob {
            len: DWORD::try_from(identity.pkcs12.len())
                .map_err(|_| HttpError::Tls("the client identity is too large".into()))?,
            data: identity.pkcs12.as_ptr(),
        };
        let password = wide(&identity.password);
        // SAFETY: the blob points at bytes that outlive the call; the password is NUL-terminated.
        let mut store =
            unsafe { (crypt.pfx_import)(&blob, password.as_ptr(), PKCS12_NO_PERSIST_KEY) };
        if store.is_null() && identity.password.is_empty() {
            // An archive without a password may expect none rather than an empty one.
            // SAFETY: as above, with no password.
            store = unsafe { (crypt.pfx_import)(&blob, std::ptr::null(), PKCS12_NO_PERSIST_KEY) };
        }
        if store.is_null() {
            // SAFETY: reads this thread's last-error value.
            let code = unsafe { GetLastError() };
            return Err(HttpError::Tls(format!(
                "the client identity could not be read ({code})"
            )));
        }
        let find = |kind: DWORD| {
            // SAFETY: an open store; a null previous context starts the search.
            unsafe {
                (crypt.find_certificate)(
                    store,
                    ENCODING_X509_PKCS7,
                    0,
                    kind,
                    std::ptr::null(),
                    std::ptr::null(),
                )
            }
        };
        let mut context = find(CERT_FIND_HAS_PRIVATE_KEY);
        if context.is_null() {
            context = find(CERT_FIND_ANY);
        }
        if context.is_null() {
            // SAFETY: the store this function opened.
            unsafe { (crypt.close_store)(store, 0) };
            return Err(HttpError::Tls(
                "the client identity holds no certificate".into(),
            ));
        }
        Ok(Arc::new(ClientCert {
            crypt,
            store,
            context,
        }))
    }
}

impl Drop for ClientCert {
    fn drop(&mut self) {
        // SAFETY: the context and store this value imported; WinHTTP holds its own references.
        unsafe {
            (self.crypt.free_certificate)(self.context);
            (self.crypt.close_store)(self.store, 0);
        }
    }
}

struct WinTransport {
    session: Result<Arc<Session>, HttpError>,
    identity: Option<Result<Arc<ClientCert>, HttpError>>,
}

impl WinTransport {
    /// The session and identity an exchange needs, or why it cannot start.
    fn parts(&self) -> Result<(Arc<Session>, Option<Arc<ClientCert>>), HttpError> {
        let session = self.session.clone()?;
        let cert = match &self.identity {
            None => None,
            Some(identity) => Some(identity.clone()?),
        };
        Ok((session, cert))
    }
}

impl Transport for WinTransport {
    fn capabilities(&self) -> Capabilities {
        capabilities()
    }

    fn start(&self, prepared: Prepared, events: Events) -> Arc<dyn Transfer> {
        match self
            .parts()
            .and_then(|(session, cert)| Exchange::begin(session, cert, prepared, events.clone()))
        {
            Ok(exchange) => exchange,
            Err(error) => {
                events(Event::Failed(error));
                Arc::new(Dead)
            }
        }
    }

    fn websocket(&self, prepared: Prepared, events: WsEvents) -> Arc<dyn Socket> {
        let opened = self.parts().and_then(|(session, cert)| {
            let api: &'static Api = session.api;
            let ws = api.ws.as_ref().ok_or(HttpError::Unsupported)?;
            WsConn::begin(session.clone(), ws, cert, prepared, events.clone())
        });
        match opened {
            Ok(socket) => socket,
            Err(error) => {
                events(WsEvent::Failed(error));
                Arc::new(Dead)
            }
        }
    }
}

/// The grip on an exchange or WebSocket that never started.
struct Dead;

impl Transfer for Dead {
    fn demand(&self, _chunks: u32) {}
    fn answer(&self, _id: QuestionId, _answer: Answer) {}
    fn cancel(&self) {}
}

impl Socket for Dead {
    fn send(&self, _message: Message, done: Completion) {
        done(Err(HttpError::Unsupported));
    }
    fn ping(&self, done: Completion) {
        done(Err(HttpError::Unsupported));
    }
    fn close(&self, _code: u16, _reason: &str) {}
    fn demand(&self, _messages: u32) {}
}

/// What one handle's notifications reach. Boxed, and its address is the handle's context value;
/// STATUS_HANDLE_CLOSING frees it.
enum Context {
    Exchange(Arc<Exchange>),
    /// The request a WebSocket upgrades.
    Upgrade(Arc<WsConn>),
    /// The WebSocket handle the upgrade returned.
    Socket(Arc<WsConn>),
}

impl Context {
    fn into_raw(self) -> usize {
        Box::into_raw(Box::new(self)) as usize
    }
}

/// Give a request handle its context. On failure the box is freed here, since no notification
/// will carry it.
fn attach(api: &Api, request: HINTERNET, context: Context) -> Result<usize, HttpError> {
    let raw = context.into_raw();
    // SAFETY: a pointer-sized option read from a live local for the call.
    let ok = unsafe {
        (api.set_option)(
            request,
            OPTION_CONTEXT_VALUE,
            (&raw as *const usize).cast(),
            std::mem::size_of::<usize>() as DWORD,
        )
    };
    if ok == 0 {
        let error = last_error();
        // SAFETY: the box made above, which WinHTTP never received.
        drop(unsafe { Box::from_raw(raw as *mut Context) });
        return Err(error);
    }
    Ok(raw)
}

/// The session's status callback, on one of WinHTTP's threads. `context` is 0 for the session and
/// connection handles, whose notifications carry nothing the transport acts on.
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
    let raw = context as *mut Context;
    if status == STATUS_HANDLE_CLOSING {
        // SAFETY: boxed by `Context::into_raw`, and no notification follows this one.
        let context = unsafe { Box::from_raw(raw) };
        match *context {
            Context::Exchange(exchange) => exchange.closed(),
            Context::Upgrade(conn) => conn.request_closed(),
            Context::Socket(conn) => conn.socket_closed(),
        }
        return;
    }
    // SAFETY: the box stays alive until STATUS_HANDLE_CLOSING above.
    let context = unsafe { &*raw };
    // SAFETY: WinHTTP documents `info` per notification; each handler reads only that shape.
    let error = || {
        if info.is_null() {
            E_CONNECTION_ERROR
        } else {
            unsafe { (*(info as *const AsyncResult)).error }
        }
    };
    match context {
        Context::Exchange(exchange) => match status {
            STATUS_SENDREQUEST_COMPLETE => exchange.write_next(),
            STATUS_WRITE_COMPLETE => exchange.written(),
            STATUS_HEADERS_AVAILABLE => exchange.head(),
            STATUS_READ_COMPLETE => exchange.read(info_len as usize),
            STATUS_REQUEST_ERROR => exchange.fail(map_error(error())),
            _ => {}
        },
        Context::Upgrade(conn) => match status {
            STATUS_SENDREQUEST_COMPLETE => conn.receive_response(),
            STATUS_HEADERS_AVAILABLE => conn.upgrade(),
            STATUS_REQUEST_ERROR => conn.fail(map_error(error())),
            _ => {}
        },
        Context::Socket(conn) => {
            let ws_status = || {
                // SAFETY: a WebSocket completion points at a WINHTTP_WEB_SOCKET_STATUS.
                (!info.is_null()).then(|| unsafe { &*(info as *const WsStatus) })
            };
            match status {
                STATUS_READ_COMPLETE => match ws_status() {
                    Some(s) => conn.received(s.bytes as usize, s.buffer_type),
                    None => conn.fail(HttpError::Io("the WebSocket receive failed".into())),
                },
                STATUS_WRITE_COMPLETE => conn.sent(),
                STATUS_CLOSE_COMPLETE => conn.close_complete(handle),
                STATUS_REQUEST_ERROR => conn.fail(map_error(error())),
                _ => {}
            }
        }
    }
}

/// A connection and request handle, before a context owns them.
struct Opened {
    connection: HINTERNET,
    request: HINTERNET,
}

impl Opened {
    fn close(&self, api: &Api) {
        // SAFETY: handles this exchange opened and no context owns yet.
        unsafe {
            (api.close)(self.request);
            (api.close)(self.connection);
        }
    }
}

/// Open a request with the transport's fixed options: the idle bound, redirects and cookies left
/// to the client, the client identity, and the request headers.
fn open_request(
    session: &Session,
    cert: Option<&ClientCert>,
    method: &str,
    url: &str,
    headers: &[(String, String)],
    idle: Duration,
) -> Result<Opened, HttpError> {
    let api = session.api;
    let (secure, host, port, path) = split_url(url)?;
    // SAFETY: a live session handle and NUL-terminated strings that outlive each call.
    unsafe {
        let connection = (api.connect)(session.handle, wide(&host).as_ptr(), port, 0);
        if connection.is_null() {
            return Err(last_error());
        }
        let request = (api.open_request)(
            connection,
            wide(method).as_ptr(),
            wide(&path).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            if secure { WINHTTP_FLAG_SECURE } else { 0 },
        );
        if request.is_null() {
            let error = last_error();
            (api.close)(connection);
            return Err(error);
        }
        let opened = Opened {
            connection,
            request,
        };
        let ms = i32::try_from(idle.as_millis()).unwrap_or(i32::MAX).max(1);
        (api.set_timeouts)(request, ms, ms, ms, ms);
        set_dword(api, request, OPTION_REDIRECT_POLICY, REDIRECT_POLICY_NEVER);
        set_dword(api, request, OPTION_DISABLE_FEATURE, DISABLE_COOKIES);
        match cert {
            Some(cert) => {
                (api.set_option)(
                    request,
                    OPTION_CLIENT_CERT_CONTEXT,
                    cert.context.cast(),
                    std::mem::size_of::<CertContext>() as DWORD,
                );
            }
            // WINHTTP_NO_CLIENT_CERT_CONTEXT: a server that asks for a certificate gets none,
            // and answers as it does to any client without one.
            None => {
                (api.set_option)(request, OPTION_CLIENT_CERT_CONTEXT, std::ptr::null(), 0);
            }
        }
        if !headers.is_empty() {
            let joined: String = headers
                .iter()
                .map(|(k, v)| format!("{k}: {v}\r\n"))
                .collect();
            let joined = wide(&joined);
            // A length of -1 takes the whole NUL-terminated string.
            if (api.add_headers)(
                request,
                joined.as_ptr(),
                DWORD::MAX,
                WINHTTP_ADDREQ_FLAG_ADD,
            ) == 0
            {
                let error = last_error();
                opened.close(api);
                return Err(error);
            }
        }
        Ok(opened)
    }
}

/// The response head once WinHTTP has it: the numeric status and the raw headers
/// (`"HTTP/1.1 200 OK\r\nK: V\r\n…"`, first line dropped).
fn read_head(api: &Api, request: HINTERNET) -> Result<(u16, Vec<(String, String)>), HttpError> {
    let mut status: DWORD = 0;
    let mut len = std::mem::size_of::<DWORD>() as DWORD;
    // SAFETY: a numeric query writes one DWORD into `status`.
    let ok = unsafe {
        (api.query_headers)(
            request,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            std::ptr::null(),
            (&mut status as *mut DWORD).cast(),
            &mut len,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    let mut headers = Vec::new();
    let mut bytes: DWORD = 0;
    // SAFETY: a null buffer asks for the size in bytes.
    unsafe {
        (api.query_headers)(
            request,
            WINHTTP_QUERY_RAW_HEADERS_CRLF,
            std::ptr::null(),
            std::ptr::null_mut(),
            &mut bytes,
            std::ptr::null_mut(),
        )
    };
    if bytes > 0 {
        let mut buf = vec![0u16; (bytes as usize).div_ceil(2)];
        // SAFETY: `buf` holds `bytes` bytes of UTF-16.
        let ok = unsafe {
            (api.query_headers)(
                request,
                WINHTTP_QUERY_RAW_HEADERS_CRLF,
                std::ptr::null(),
                buf.as_mut_ptr().cast(),
                &mut bytes,
                std::ptr::null_mut(),
            )
        };
        if ok != 0 {
            let raw = String::from_utf16_lossy(&buf[..(bytes as usize / 2).min(buf.len())]);
            for line in raw.lines().skip(1) {
                if let Some((k, v)) = line.split_once(':') {
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
        }
    }
    Ok((u16::try_from(status).unwrap_or(0), headers))
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

// ---------------------------------------------------------------------------
// Exchanges
// ---------------------------------------------------------------------------

/// Where an upload's bytes come from.
enum Source {
    None,
    Bytes { data: Arc<Vec<u8>>, at: usize },
    File(std::fs::File),
    Stream(Box<dyn Read + Send>),
}

fn read_some(reader: &mut dyn Read, buf: &mut [u8]) -> Result<usize, HttpError> {
    loop {
        match reader.read(buf) {
            Ok(n) => return Ok(n),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(HttpError::Io(e.to_string())),
        }
    }
}

/// A request body on its way out, one piece per WinHttpWriteData.
struct Upload {
    source: Source,
    /// Body bytes still to send, when the length is known.
    remaining: Option<u64>,
    total: Option<u64>,
    chunked: bool,
    finished: bool,
    sent: u64,
    /// The piece in flight, kept alive until STATUS_WRITE_COMPLETE, and the body bytes it carries
    /// (a chunk's framing is not body).
    out: Vec<u8>,
    out_body: usize,
}

impl Upload {
    /// Fill `out` with the next piece; `false` once the body is out.
    fn next(&mut self) -> Result<bool, HttpError> {
        if self.finished {
            return Ok(false);
        }
        let limit = self.remaining.map_or(PIECE, |r| {
            usize::try_from(r.min(PIECE as u64)).unwrap_or(PIECE)
        });
        let mut data = vec![0u8; limit];
        let n = if limit == 0 {
            0
        } else {
            match &mut self.source {
                Source::None => 0,
                Source::Bytes { data: bytes, at } => {
                    let n = (bytes.len() - *at).min(limit);
                    data[..n].copy_from_slice(&bytes[*at..*at + n]);
                    *at += n;
                    n
                }
                Source::File(file) => read_some(file, &mut data)?,
                Source::Stream(reader) => read_some(reader.as_mut(), &mut data)?,
            }
        };
        data.truncate(n);
        if let Some(remaining) = &mut self.remaining {
            if n == 0 && *remaining > 0 {
                return Err(HttpError::Io(
                    "the request body ended before its declared length".into(),
                ));
            }
            *remaining -= n as u64;
        }
        self.out_body = n;
        if n == 0 {
            self.finished = true;
            if !self.chunked {
                return Ok(false);
            }
            self.out = b"0\r\n\r\n".to_vec();
        } else if self.chunked {
            let mut framed = format!("{n:X}\r\n").into_bytes();
            framed.extend_from_slice(&data);
            framed.extend_from_slice(b"\r\n");
            self.out = framed;
        } else {
            self.out = data;
        }
        Ok(true)
    }
}

struct ExState {
    events: Option<Events>,
    head: bool,
    reading: bool,
    demand: u32,
    closing: bool,
    first_byte: Option<Duration>,
    received: u64,
}

/// One HTTP exchange: its handles, its upload, and the reads its demand allows.
struct Exchange {
    session: Arc<Session>,
    _identity: Option<Arc<ClientCert>>,
    connection: HINTERNET,
    request: HINTERNET,
    url: String,
    started: Instant,
    /// Filled by the one read in flight; `ExState::reading` keeps everything else away from it.
    buffer: UnsafeCell<Box<[u8]>>,
    state: Mutex<ExState>,
    upload: Mutex<Upload>,
}

// SAFETY: WinHTTP handles are usable from any thread. The read buffer is written only by the read
// in flight and read only by that read's completion, and `ExState::reading`, taken under the lock,
// admits one read at a time.
unsafe impl Send for Exchange {}
unsafe impl Sync for Exchange {}

impl Exchange {
    fn begin(
        session: Arc<Session>,
        identity: Option<Arc<ClientCert>>,
        prepared: Prepared,
        events: Events,
    ) -> Result<Arc<Exchange>, HttpError> {
        let api = session.api;
        let mut headers: Vec<(String, String)> = prepared
            .headers
            .iter()
            .filter(|(k, _)| {
                !k.eq_ignore_ascii_case("content-length")
                    && !k.eq_ignore_ascii_case("transfer-encoding")
            })
            .cloned()
            .collect();
        let length = match &prepared.body {
            PreparedBody::Empty => Some(0),
            PreparedBody::Bytes(data) => Some(data.len() as u64),
            PreparedBody::File { len, .. } => Some(*len),
            PreparedBody::Stream { len, .. } => *len,
        };
        let source = match prepared.body {
            PreparedBody::Empty => Source::None,
            PreparedBody::Bytes(data) => Source::Bytes { data, at: 0 },
            PreparedBody::File { path, .. } => {
                Source::File(std::fs::File::open(&path).map_err(|e| HttpError::Io(e.to_string()))?)
            }
            PreparedBody::Stream { reader, .. } => {
                Source::Stream(reader.take().ok_or_else(|| {
                    HttpError::Io("the request body stream was already sent".into())
                })?)
            }
        };
        // WinHttpSendRequest takes a 32-bit length: a longer body declares its own, and a body of
        // unknown length goes out chunked, framed by `Upload::next`.
        let total_length = match length {
            Some(n) => DWORD::try_from(n).unwrap_or_else(|_| {
                headers.push(("Content-Length".into(), n.to_string()));
                IGNORE_TOTAL_LENGTH
            }),
            None => {
                headers.push(("Transfer-Encoding".into(), "chunked".into()));
                IGNORE_TOTAL_LENGTH
            }
        };
        let opened = open_request(
            &session,
            identity.as_deref(),
            &prepared.method,
            &prepared.url,
            &headers,
            prepared.timeout_idle,
        )?;
        let exchange = Arc::new(Exchange {
            session: session.clone(),
            _identity: identity,
            connection: opened.connection,
            request: opened.request,
            url: prepared.url,
            started: Instant::now(),
            buffer: UnsafeCell::new(vec![0u8; PIECE].into_boxed_slice()),
            state: Mutex::new(ExState {
                events: Some(events),
                head: false,
                reading: false,
                demand: 0,
                closing: false,
                first_byte: None,
                received: 0,
            }),
            upload: Mutex::new(Upload {
                source,
                remaining: length,
                total: length,
                chunked: length.is_none(),
                finished: false,
                sent: 0,
                out: Vec::new(),
                out_body: 0,
            }),
        });
        let context = match attach(api, exchange.request, Context::Exchange(exchange.clone())) {
            Ok(context) => context,
            Err(error) => {
                opened.close(api);
                return Err(error);
            }
        };
        // SAFETY: a live request handle with its context; the body follows through
        // WinHttpWriteData once the send completes.
        let ok = unsafe {
            (api.send)(
                exchange.request,
                std::ptr::null(),
                0,
                std::ptr::null(),
                0,
                total_length,
                context,
            )
        };
        if ok == 0 {
            exchange.fail(last_error());
        }
        Ok(exchange)
    }

    fn api(&self) -> &'static Api {
        self.session.api
    }

    fn emit(&self, event: Event) {
        let events = lock(&self.state).events.clone();
        if let Some(events) = events {
            events(event);
        }
    }

    /// Write the next piece of the body, or ask for the response once the body is out.
    fn write_next(&self) {
        if lock(&self.state).events.is_none() {
            return;
        }
        let piece = {
            let mut upload = lock(&self.upload);
            upload
                .next()
                .map(|more| more.then(|| (upload.out.as_ptr(), upload.out.len())))
        };
        match piece {
            Ok(Some((data, len))) => {
                // SAFETY: the piece stays in `Upload::out`, untouched, until STATUS_WRITE_COMPLETE;
                // a piece is at most `PIECE` bytes and framing.
                let ok = unsafe {
                    (self.api().write_data)(
                        self.request,
                        data.cast(),
                        len as DWORD,
                        std::ptr::null_mut(),
                    )
                };
                if ok == 0 {
                    self.fail(last_error());
                }
            }
            Ok(None) => self.receive_response(),
            Err(error) => self.fail(error),
        }
    }

    fn written(&self) {
        let (sent, total, body) = {
            let mut upload = lock(&self.upload);
            upload.sent += upload.out_body as u64;
            (upload.sent, upload.total, upload.out_body)
        };
        if body > 0 {
            self.emit(Event::Sent { sent, total });
        }
        self.write_next();
    }

    fn receive_response(&self) {
        // SAFETY: a live request handle whose request went out; the reserved argument is null.
        if unsafe { (self.api().receive)(self.request, std::ptr::null_mut()) } == 0 {
            self.fail(last_error());
        }
    }

    fn head(&self) {
        let (status, headers) = match read_head(self.api(), self.request) {
            Ok(head) => head,
            Err(error) => return self.fail(error),
        };
        // An encoded body's declared length is not the length the reader receives.
        let expected_length = match header(&headers, "content-encoding") {
            Some(encoding) if !encoding.eq_ignore_ascii_case("identity") => None,
            _ => header(&headers, "content-length").and_then(|v| v.trim().parse().ok()),
        };
        {
            let mut st = lock(&self.state);
            st.head = true;
            st.first_byte = Some(self.started.elapsed());
        }
        self.emit(Event::Head(Head {
            status,
            headers,
            url: self.url.clone(),
            expected_length,
        }));
        self.pump();
    }

    /// Start a read when the head is in, demand remains and none is in flight.
    fn pump(&self) {
        {
            let mut st = lock(&self.state);
            if st.events.is_none() || !st.head || st.reading || st.demand == 0 {
                return;
            }
            st.reading = true;
        }
        // SAFETY: `reading` keeps every other read, and every reader of the buffer, away until
        // this read completes.
        let ok = unsafe {
            (self.api().read_data)(
                self.request,
                (*self.buffer.get()).as_mut_ptr().cast(),
                PIECE as DWORD,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            self.fail(last_error());
        }
    }

    fn read(&self, n: usize) {
        if n == 0 {
            lock(&self.state).reading = false;
            return self.finish();
        }
        // SAFETY: the read that just completed wrote `n` bytes, and `reading` is still set.
        let buffer: &[u8] = unsafe { &*self.buffer.get() };
        let chunk = buffer[..n.min(PIECE)].to_vec();
        {
            let mut st = lock(&self.state);
            st.reading = false;
            st.demand = st.demand.saturating_sub(1);
            st.received += n as u64;
        }
        self.emit(Event::Chunk(chunk));
        self.pump();
    }

    fn finish(&self) {
        let (events, first_byte, received) = {
            let mut st = lock(&self.state);
            (st.events.take(), st.first_byte, st.received)
        };
        let Some(events) = events else {
            return;
        };
        let sent = lock(&self.upload).sent;
        events(Event::Metrics(Metrics {
            first_byte,
            total: Some(self.started.elapsed()),
            protocol: self.protocol(),
            bytes_sent: Some(sent),
            bytes_received: Some(received),
            ..Metrics::default()
        }));
        events(Event::End);
        self.close();
    }

    fn protocol(&self) -> Option<String> {
        let mut flags: DWORD = 0;
        let mut len = std::mem::size_of::<DWORD>() as DWORD;
        // SAFETY: a DWORD option query into a live local.
        let ok = unsafe {
            (self.api().query_option)(
                self.request,
                OPTION_HTTP_PROTOCOL_USED,
                (&mut flags as *mut DWORD).cast(),
                &mut len,
            )
        };
        (ok != 0).then(|| {
            if flags & PROTOCOL_FLAG_HTTP3 != 0 {
                "h3".to_string()
            } else if flags & PROTOCOL_FLAG_HTTP2 != 0 {
                "h2".to_string()
            } else {
                "http/1.1".to_string()
            }
        })
    }

    fn fail(&self, error: HttpError) {
        let events = lock(&self.state).events.take();
        if let Some(events) = events {
            events(Event::Failed(error));
        }
        self.close();
    }

    /// Close the request handle once. Its last notification frees the context.
    fn close(&self) {
        {
            let mut st = lock(&self.state);
            if st.closing {
                return;
            }
            st.closing = true;
        }
        // SAFETY: the request handle, closed exactly once; closing cancels whatever is pending.
        unsafe { (self.api().close)(self.request) };
    }

    /// STATUS_HANDLE_CLOSING: the request handle is gone.
    fn closed(&self) {
        let events = lock(&self.state).events.take();
        if let Some(events) = events {
            events(Event::Failed(HttpError::Io("the request closed".into())));
        }
        // SAFETY: the connection handle this exchange opened, whose request is gone.
        unsafe { (self.api().close)(self.connection) };
    }
}

impl Transfer for Exchange {
    fn demand(&self, chunks: u32) {
        {
            let mut st = lock(&self.state);
            st.demand = st.demand.saturating_add(chunks);
        }
        self.pump();
    }

    fn answer(&self, _id: QuestionId, _answer: Answer) {}

    fn cancel(&self) {
        let events = lock(&self.state).events.take();
        if let Some(events) = events {
            events(Event::Failed(HttpError::Cancelled));
        }
        self.close();
    }
}

// ---------------------------------------------------------------------------
// WebSockets
// ---------------------------------------------------------------------------

/// The longest close reason the protocol carries.
const CLOSE_REASON_MAX: usize = 123;

struct WsState {
    events: Option<WsEvents>,
    socket: HINTERNET,
    open: bool,
    request_closing: bool,
    socket_closing: bool,
    demand: u32,
    receiving: bool,
    message: Vec<u8>,
    queue: VecDeque<(u32, Vec<u8>, Completion)>,
    /// The message in flight, kept alive until STATUS_WRITE_COMPLETE, and its completion.
    sending: Vec<u8>,
    sending_done: Option<Completion>,
}

/// What a completed receive amounts to.
enum Received {
    More,
    Message(Message),
    Close,
}

/// One WebSocket: the request that upgrades, then the socket handle the upgrade returns.
struct WsConn {
    session: Arc<Session>,
    ws: &'static WsApi,
    _identity: Option<Arc<ClientCert>>,
    connection: HINTERNET,
    request: HINTERNET,
    /// Filled by the one receive in flight; `WsState::receiving` keeps everything else away.
    buffer: UnsafeCell<Box<[u8]>>,
    state: Mutex<WsState>,
}

// SAFETY: as for `Exchange`: WinHTTP handles are usable from any thread, and
// `WsState::receiving`, taken under the lock, admits one receive at a time into the buffer.
unsafe impl Send for WsConn {}
unsafe impl Sync for WsConn {}

fn refuse(waiting: Vec<Completion>) {
    for done in waiting {
        done(Err(HttpError::Io("the WebSocket is closed".into())));
    }
}

impl WsConn {
    fn begin(
        session: Arc<Session>,
        ws: &'static WsApi,
        identity: Option<Arc<ClientCert>>,
        prepared: Prepared,
        events: WsEvents,
    ) -> Result<Arc<WsConn>, HttpError> {
        let api = session.api;
        let mut headers = prepared.headers.clone();
        if !prepared.protocols.is_empty() {
            headers.push((
                "Sec-WebSocket-Protocol".into(),
                prepared.protocols.join(", "),
            ));
        }
        let opened = open_request(
            &session,
            identity.as_deref(),
            "GET",
            &prepared.url,
            &headers,
            prepared.timeout_idle,
        )?;
        // SAFETY: the upgrade option takes no buffer.
        let ok = unsafe {
            (api.set_option)(
                opened.request,
                OPTION_UPGRADE_TO_WEB_SOCKET,
                std::ptr::null(),
                0,
            )
        };
        if ok == 0 {
            let error = last_error();
            opened.close(api);
            return Err(error);
        }
        let conn = Arc::new(WsConn {
            session: session.clone(),
            ws,
            _identity: identity,
            connection: opened.connection,
            request: opened.request,
            buffer: UnsafeCell::new(vec![0u8; PIECE].into_boxed_slice()),
            state: Mutex::new(WsState {
                events: Some(events),
                socket: std::ptr::null_mut(),
                open: false,
                request_closing: false,
                socket_closing: false,
                demand: 0,
                receiving: false,
                message: Vec::new(),
                queue: VecDeque::new(),
                sending: Vec::new(),
                sending_done: None,
            }),
        });
        let context = match attach(api, conn.request, Context::Upgrade(conn.clone())) {
            Ok(context) => context,
            Err(error) => {
                opened.close(api);
                return Err(error);
            }
        };
        // SAFETY: a live request handle with its context, and no body.
        let ok = unsafe {
            (api.send)(
                conn.request,
                std::ptr::null(),
                0,
                std::ptr::null(),
                0,
                0,
                context,
            )
        };
        if ok == 0 {
            conn.fail(last_error());
        }
        Ok(conn)
    }

    fn receive_response(&self) {
        // SAFETY: a live request handle whose request went out; the reserved argument is null.
        if unsafe { (self.session.api.receive)(self.request, std::ptr::null_mut()) } == 0 {
            self.fail(last_error());
        }
    }

    /// The upgrade response: a 101 becomes the socket, anything else a failure.
    fn upgrade(self: &Arc<Self>) {
        let (status, headers) = match read_head(self.session.api, self.request) {
            Ok(head) => head,
            Err(error) => return self.fail(error),
        };
        if status != 101 {
            return self.fail(HttpError::Io(format!(
                "the server answered {status} to the upgrade"
            )));
        }
        let protocol = header(&headers, "sec-websocket-protocol")
            .map(str::to_string)
            .filter(|p| !p.is_empty());
        let raw = Context::Socket(self.clone()).into_raw();
        // SAFETY: a request whose upgrade response arrived; the box becomes the socket's context.
        let socket = unsafe { (self.ws.complete_upgrade)(self.request, raw) };
        if socket.is_null() {
            let error = last_error();
            // SAFETY: no socket handle exists, so no notification carries the box.
            drop(unsafe { Box::from_raw(raw as *mut Context) });
            return self.fail(error);
        }
        let events = {
            let mut st = lock(&self.state);
            st.socket = socket;
            st.open = true;
            st.events.clone()
        };
        // The request handle has done its work; the socket runs on its own.
        self.close_request();
        let Some(events) = events else {
            // Closed while the upgrade completed.
            return self.close_socket();
        };
        events(WsEvent::Open { protocol });
        self.flush();
        self.pump();
    }

    /// Start a receive while the socket is open, demand remains and none is in flight.
    fn pump(&self) {
        let socket = {
            let mut st = lock(&self.state);
            if st.events.is_none()
                || !st.open
                || st.socket_closing
                || st.receiving
                || st.demand == 0
            {
                return;
            }
            st.receiving = true;
            st.socket
        };
        // SAFETY: `receiving` keeps the buffer to this receive until it completes.
        let rc = unsafe {
            (self.ws.receive)(
                socket,
                (*self.buffer.get()).as_mut_ptr().cast(),
                PIECE as DWORD,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if rc != 0 {
            lock(&self.state).receiving = false;
            self.fail(map_error(rc));
        }
    }

    fn received(&self, n: usize, kind: u32) {
        let (events, got) = {
            let mut st = lock(&self.state);
            // SAFETY: the receive that just completed wrote `n` bytes, and `receiving` is still set.
            let buffer: &[u8] = unsafe { &*self.buffer.get() };
            let bytes = &buffer[..n.min(PIECE)];
            st.message.extend_from_slice(bytes);
            st.receiving = false;
            let got = match kind {
                WS_BINARY_MESSAGE => {
                    Received::Message(Message::Binary(std::mem::take(&mut st.message)))
                }
                WS_UTF8_MESSAGE => {
                    let text = std::mem::take(&mut st.message);
                    Received::Message(Message::Text(String::from_utf8_lossy(&text).into_owned()))
                }
                WS_CLOSE => {
                    st.message.clear();
                    Received::Close
                }
                // A fragment: the rest of the message follows.
                WS_BINARY_FRAGMENT | WS_UTF8_FRAGMENT => Received::More,
                _ => Received::More,
            };
            if matches!(got, Received::Message(_)) {
                st.demand = st.demand.saturating_sub(1);
            }
            (st.events.clone(), got)
        };
        match got {
            Received::More => self.pump(),
            Received::Message(message) => {
                if let Some(events) = events {
                    events(WsEvent::Message(message));
                }
                self.pump();
            }
            Received::Close => self.peer_closed(),
        }
    }

    /// The peer's close frame: report it, then answer it so the closing handshake completes.
    fn peer_closed(&self) {
        let socket = lock(&self.state).socket;
        let (code, reason) = self.close_status(socket);
        self.closed_with(code, reason, false);
        // 1005, 1006 and 1015 describe a close; none may be sent in one.
        let answer = if matches!(code, 1005 | 1006 | 1015) {
            1000
        } else {
            code
        };
        // SAFETY: the open socket, with no reason.
        let rc = unsafe { (self.ws.close)(socket, answer, std::ptr::null(), 0) };
        if rc != 0 {
            self.close_socket();
        }
    }

    fn close_complete(&self, socket: HINTERNET) {
        let (code, reason) = self.close_status(socket);
        self.closed_with(code, reason, true);
    }

    fn close_status(&self, socket: HINTERNET) -> (u16, String) {
        let mut code: u16 = 1005;
        let mut reason = [0u8; CLOSE_REASON_MAX];
        let mut len: DWORD = 0;
        // SAFETY: a buffer of the protocol's longest reason, and live locals for the outputs.
        let rc = unsafe {
            (self.ws.query_close_status)(
                socket,
                &mut code,
                reason.as_mut_ptr().cast(),
                CLOSE_REASON_MAX as DWORD,
                &mut len,
            )
        };
        if rc != 0 {
            return (1005, String::new());
        }
        let len = (len as usize).min(CLOSE_REASON_MAX);
        (code, String::from_utf8_lossy(&reason[..len]).into_owned())
    }

    /// Report the close, refuse what waits to go out, and close the socket handle when asked.
    fn closed_with(&self, code: u16, reason: String, close_socket: bool) {
        let (events, waiting) = self.terminal();
        if let Some(events) = events {
            events(WsEvent::Closed { code, reason });
        }
        refuse(waiting);
        if close_socket {
            self.close_socket();
        }
    }

    /// Take the events, so nothing follows, and every send still waiting.
    fn terminal(&self) -> (Option<WsEvents>, Vec<Completion>) {
        let mut st = lock(&self.state);
        let mut waiting: Vec<Completion> = st.queue.drain(..).map(|(_, _, done)| done).collect();
        waiting.extend(st.sending_done.take());
        (st.events.take(), waiting)
    }

    fn sent(&self) {
        let done = lock(&self.state).sending_done.take();
        if let Some(done) = done {
            done(Ok(()));
        }
        self.flush();
    }

    /// Send the next queued message when the socket is open and none is in flight.
    fn flush(&self) {
        loop {
            let (socket, kind, data, len) = {
                let mut st = lock(&self.state);
                if st.events.is_none() || !st.open || st.socket_closing || st.sending_done.is_some()
                {
                    return;
                }
                let Some((kind, bytes, done)) = st.queue.pop_front() else {
                    return;
                };
                let Ok(len) = DWORD::try_from(bytes.len()) else {
                    drop(st);
                    done(Err(HttpError::Io(
                        "the message is too large to send".into(),
                    )));
                    continue;
                };
                st.sending = bytes;
                st.sending_done = Some(done);
                (st.socket, kind, st.sending.as_ptr(), len)
            };
            // SAFETY: the message stays in `WsState::sending`, untouched, until
            // STATUS_WRITE_COMPLETE.
            let rc = unsafe { (self.ws.send)(socket, kind, data.cast(), len) };
            if rc == 0 {
                return;
            }
            let done = lock(&self.state).sending_done.take();
            if let Some(done) = done {
                done(Err(map_error(rc)));
            }
        }
    }

    fn fail(&self, error: HttpError) {
        let (events, waiting) = self.terminal();
        if let Some(events) = events {
            events(WsEvent::Failed(error));
        }
        refuse(waiting);
        self.close_request();
        self.close_socket();
    }

    fn close_request(&self) {
        {
            let mut st = lock(&self.state);
            if st.request_closing {
                return;
            }
            st.request_closing = true;
        }
        // SAFETY: the upgrade request handle, closed exactly once.
        unsafe { (self.session.api.close)(self.request) };
    }

    fn close_socket(&self) {
        let socket = {
            let mut st = lock(&self.state);
            if st.socket.is_null() || st.socket_closing {
                return;
            }
            st.socket_closing = true;
            st.socket
        };
        // SAFETY: the socket handle, closed exactly once.
        unsafe { (self.session.api.close)(socket) };
    }

    /// STATUS_HANDLE_CLOSING for the upgrade request.
    fn request_closed(&self) {
        if !lock(&self.state).socket.is_null() {
            return;
        }
        let (events, waiting) = self.terminal();
        if let Some(events) = events {
            events(WsEvent::Failed(HttpError::Io(
                "the WebSocket handshake ended".into(),
            )));
        }
        refuse(waiting);
        // SAFETY: the connection handle this socket opened; its request is gone and no socket
        // came of it.
        unsafe { (self.session.api.close)(self.connection) };
    }

    /// STATUS_HANDLE_CLOSING for the socket.
    fn socket_closed(&self) {
        let (events, waiting) = self.terminal();
        if let Some(events) = events {
            events(WsEvent::Closed {
                code: 1006,
                reason: String::new(),
            });
        }
        refuse(waiting);
        // SAFETY: the connection handle this socket opened; the socket is gone.
        unsafe { (self.session.api.close)(self.connection) };
    }
}

impl Socket for WsConn {
    fn send(&self, message: Message, done: Completion) {
        let (kind, bytes) = match message {
            Message::Text(text) => (WS_UTF8_MESSAGE, text.into_bytes()),
            Message::Binary(bytes) => (WS_BINARY_MESSAGE, bytes),
            Message::Close { code, reason } => {
                self.close(code, &reason);
                return done(Ok(()));
            }
        };
        {
            let mut st = lock(&self.state);
            if st.events.is_none() {
                drop(st);
                return done(Err(HttpError::Io("the WebSocket is closed".into())));
            }
            st.queue.push_back((kind, bytes, done));
        }
        self.flush();
    }

    fn ping(&self, done: Completion) {
        done(Err(HttpError::Unsupported));
    }

    fn close(&self, code: u16, reason: &str) {
        let socket = {
            let st = lock(&self.state);
            if st.events.is_none() {
                return;
            }
            st.open.then_some(st.socket)
        };
        let Some(socket) = socket else {
            // Before the upgrade: abandon the handshake.
            self.closed_with(code, reason.to_string(), false);
            return self.close_request();
        };
        let mut end = reason.len().min(CLOSE_REASON_MAX);
        while !reason.is_char_boundary(end) {
            end -= 1;
        }
        let data = if end == 0 {
            std::ptr::null()
        } else {
            reason.as_ptr().cast()
        };
        // SAFETY: the open socket and at most the protocol's longest reason, read for the call.
        let rc = unsafe { (self.ws.close)(socket, code, data, end as DWORD) };
        if rc != 0 {
            self.fail(map_error(rc));
        }
    }

    fn demand(&self, messages: u32) {
        {
            let mut st = lock(&self.state);
            st.demand = st.demand.saturating_add(messages);
        }
        self.pump();
    }
}
