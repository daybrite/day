// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Linux: the system's libcurl, loaded at run time, as the transport beneath the portable client.
// It brings the distribution's TLS stack and trust store, HTTP/2 where libcurl was built with
// nghttp2, and the proxy environment variables. Nothing links against it: the library opens with
// dlopen the first time a client needs it, and a system without one reports `Unavailable`.
//
// One driver thread owns a multi handle and every easy handle, because a curl handle belongs to
// one thread at a time. Other threads queue a command and wake the driver with
// curl_multi_wakeup: starting an exchange, demand, cancellation, a WebSocket send or close. The
// write callback pauses a transfer (CURL_WRITEFUNC_PAUSE) when its reader's demand runs out, and
// the next grant resumes it, so a slow reader slows the socket.
//
// WebSockets use libcurl's connect-only mode: after the upgrade the driver polls each open socket
// and reads a message only while its reader has demand. libcurl's WebSocket API is stable since
// 8.11.0; an older library, or one built without `ws`, reports WebSockets as unsupported.
// ---------------------------------------------------------------------------

use std::collections::{HashMap, VecDeque};
use std::ffi::{CStr, CString, c_char, c_int, c_long, c_short, c_uint, c_void};
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Once, OnceLock};
use std::time::Duration;

use crate::client::{
    Answer, Capabilities, Completion, Event, Events, Head, Message, Metrics, Prepared,
    PreparedBody, QuestionId, Socket, Transfer, Transport, TransportConfig, WsEvent, WsEvents,
};
use crate::{HttpError, Identity, Tier};

const OPT_WRITEDATA: c_int = 10_001;
const OPT_URL: c_int = 10_002;
const OPT_READDATA: c_int = 10_009;
const OPT_WRITEFUNCTION: c_int = 20_011;
const OPT_READFUNCTION: c_int = 20_012;
const OPT_LOW_SPEED_LIMIT: c_int = 19;
const OPT_LOW_SPEED_TIME: c_int = 20;
const OPT_HTTPHEADER: c_int = 10_023;
const OPT_KEYPASSWD: c_int = 10_026;
const OPT_HEADERDATA: c_int = 10_029;
const OPT_CUSTOMREQUEST: c_int = 10_036;
const OPT_NOPROGRESS: c_int = 43;
const OPT_NOBODY: c_int = 44;
const OPT_UPLOAD: c_int = 46;
const OPT_POST: c_int = 47;
const OPT_FOLLOWLOCATION: c_int = 52;
const OPT_XFERINFODATA: c_int = 10_057;
const OPT_HEADERFUNCTION: c_int = 20_079;
const OPT_HTTP_VERSION: c_int = 84;
const OPT_SSLCERTTYPE: c_int = 10_086;
const OPT_BUFFERSIZE: c_int = 98;
const OPT_NOSIGNAL: c_int = 99;
const OPT_INFILESIZE_LARGE: c_int = 30_115;
const OPT_POSTFIELDSIZE_LARGE: c_int = 30_120;
const OPT_CONNECT_ONLY: c_int = 141;
const OPT_TIMEOUT_MS: c_int = 155;
const OPT_CONNECTTIMEOUT_MS: c_int = 156;
const OPT_XFERINFOFUNCTION: c_int = 20_219;
const OPT_SSLCERT_BLOB: c_int = 40_291;

const INFO_EFFECTIVE_URL: c_int = 0x10_0000 + 1;
const INFO_SIZE_UPLOAD_T: c_int = 0x60_0000 + 7;
const INFO_SIZE_DOWNLOAD_T: c_int = 0x60_0000 + 8;
const INFO_NUM_CONNECTS: c_int = 0x20_0000 + 26;
const INFO_PRIMARY_IP: c_int = 0x10_0000 + 32;
const INFO_HTTP_VERSION: c_int = 0x20_0000 + 46;
const INFO_TOTAL_TIME_T: c_int = 0x60_0000 + 50;
const INFO_NAMELOOKUP_TIME_T: c_int = 0x60_0000 + 51;
const INFO_CONNECT_TIME_T: c_int = 0x60_0000 + 52;
const INFO_STARTTRANSFER_TIME_T: c_int = 0x60_0000 + 54;
const INFO_APPCONNECT_TIME_T: c_int = 0x60_0000 + 56;
const INFO_ACTIVESOCKET: c_int = 0x50_0000 + 44;

const WRITEFUNC_PAUSE: usize = 0x1000_0001;
const READFUNC_ABORT: usize = 0x1000_0000;
const PAUSE_CONT: c_int = 0;
const HTTP_VERSION_2TLS: c_long = 4;
const GLOBAL_ALL: c_long = 3;
const VERSION_FOURTH: c_int = 3;
const BLOB_COPY: c_uint = 1;
const MSG_DONE: c_int = 1;
const WAIT_POLLIN: c_short = 1;
const UPLOAD_BUFFER: c_long = 64 << 10;

const E_OK: c_int = 0;
const E_URL_MALFORMAT: c_int = 3;
const E_COULDNT_RESOLVE_PROXY: c_int = 5;
const E_COULDNT_RESOLVE_HOST: c_int = 6;
const E_COULDNT_CONNECT: c_int = 7;
const E_OPERATION_TIMEDOUT: c_int = 28;
const E_SSL_CONNECT_ERROR: c_int = 35;
const E_SSL_CERTPROBLEM: c_int = 58;
const E_SSL_CIPHER: c_int = 59;
const E_PEER_FAILED_VERIFICATION: c_int = 60;
const E_SSL_CACERT_BADFILE: c_int = 77;
const E_AGAIN: c_int = 81;
const E_SSL_ISSUER_ERROR: c_int = 83;
const E_SSL_PINNEDPUBKEYNOTMATCH: c_int = 90;
const E_SSL_INVALIDCERTSTATUS: c_int = 91;

const WS_TEXT: c_int = 1;
const WS_BINARY: c_int = 2;
const WS_CONT: c_int = 4;
const WS_CLOSE: c_int = 8;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

#[repr(C)]
struct VersionInfo {
    age: c_int,
    version: *const c_char,
    version_num: c_uint,
    host: *const c_char,
    features: c_int,
    ssl_version: *const c_char,
    ssl_version_num: c_long,
    libz_version: *const c_char,
    protocols: *const *const c_char,
}

#[repr(C)]
struct CurlMsg {
    msg: c_int,
    easy: *mut c_void,
    data: *mut c_void,
}

#[repr(C)]
struct WaitFd {
    fd: c_int,
    events: c_short,
    revents: c_short,
}

#[repr(C)]
struct Blob {
    data: *mut c_void,
    len: usize,
    flags: c_uint,
}

#[repr(C)]
struct WsFrame {
    age: c_int,
    flags: c_int,
    offset: i64,
    bytesleft: i64,
    len: usize,
}

type Variadic = unsafe extern "C" fn(*mut c_void, c_int, ...) -> c_int;
type WsRecv =
    unsafe extern "C" fn(*mut c_void, *mut c_void, usize, *mut usize, *mut *const WsFrame) -> c_int;
type WsSend =
    unsafe extern "C" fn(*mut c_void, *const c_void, usize, *mut usize, i64, c_uint) -> c_int;

/// The loaded library's entry points, and what it offers.
struct Lib {
    easy_init: unsafe extern "C" fn() -> *mut c_void,
    easy_cleanup: unsafe extern "C" fn(*mut c_void),
    easy_setopt: Variadic,
    easy_getinfo: Variadic,
    easy_pause: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
    easy_strerror: unsafe extern "C" fn(c_int) -> *const c_char,
    multi_init: unsafe extern "C" fn() -> *mut c_void,
    multi_add_handle: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int,
    multi_remove_handle: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int,
    multi_perform: unsafe extern "C" fn(*mut c_void, *mut c_int) -> c_int,
    multi_poll: unsafe extern "C" fn(*mut c_void, *mut WaitFd, c_uint, c_int, *mut c_int) -> c_int,
    multi_wakeup: unsafe extern "C" fn(*mut c_void) -> c_int,
    multi_info_read: unsafe extern "C" fn(*mut c_void, *mut c_int) -> *mut CurlMsg,
    slist_append: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    slist_free_all: unsafe extern "C" fn(*mut c_void),
    ws_recv: Option<WsRecv>,
    ws_send: Option<WsSend>,
    caps: Capabilities,
}

// SAFETY: function pointers into a library that stays loaded for the life of the process.
// libcurl's global functions are thread-safe once curl_global_init has run, which loading does
// once; every easy and multi handle stays on the driver thread.
unsafe impl Send for Lib {}
unsafe impl Sync for Lib {}

/// Look a symbol up as a function pointer.
///
/// # Safety
/// `T` must be the pointer type matching the symbol's C signature.
unsafe fn symbol<T: Copy>(library: *mut c_void, name: &CStr) -> Option<T> {
    if std::mem::size_of::<T>() != std::mem::size_of::<*mut c_void>() {
        return None;
    }
    // SAFETY: `library` is a handle dlopen returned; `name` is NUL-terminated.
    let address = unsafe { libc::dlsym(library, name.as_ptr()) };
    if address.is_null() {
        return None;
    }
    // SAFETY: same size, and the caller names the symbol's true type.
    Some(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&address) })
}

fn library() -> Option<&'static Lib> {
    static LIB: OnceLock<Option<Lib>> = OnceLock::new();
    // SAFETY: loading and initializing libcurl happens once, here.
    LIB.get_or_init(|| unsafe { load() }).as_ref()
}

/// Open libcurl and read what it can do.
///
/// # Safety
/// Call once; curl_global_init is not thread-safe.
unsafe fn load() -> Option<Lib> {
    const NAMES: [&CStr; 4] = [
        c"libcurl.so.4",
        c"libcurl-gnutls.so.4",
        c"libcurl.so",
        c"libcurl.4.dylib",
    ];
    let library = NAMES
        .iter()
        // SAFETY: NUL-terminated names; a missing library returns null.
        .map(|name| unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) })
        .find(|handle| !handle.is_null())?;
    // SAFETY: each symbol is looked up as its documented C signature.
    unsafe {
        let global_init: unsafe extern "C" fn(c_long) -> c_int =
            symbol(library, c"curl_global_init")?;
        if global_init(GLOBAL_ALL) != E_OK {
            return None;
        }
        let version_info: unsafe extern "C" fn(c_int) -> *const VersionInfo =
            symbol(library, c"curl_version_info")?;
        let info = version_info(VERSION_FOURTH).as_ref()?;
        let mut protocols = Vec::new();
        let mut entry = info.protocols;
        while !entry.is_null() && !(*entry).is_null() {
            protocols.push(CStr::from_ptr(*entry).to_string_lossy().into_owned());
            entry = entry.add(1);
        }
        let ws_recv: Option<WsRecv> = symbol(library, c"curl_ws_recv");
        let ws_send: Option<WsSend> = symbol(library, c"curl_ws_send");
        let websockets = info.version_num >= 0x08_0b_00
            && protocols.iter().any(|p| p == "ws")
            && ws_recv.is_some()
            && ws_send.is_some();
        Some(Lib {
            easy_init: symbol(library, c"curl_easy_init")?,
            easy_cleanup: symbol(library, c"curl_easy_cleanup")?,
            easy_setopt: symbol(library, c"curl_easy_setopt")?,
            easy_getinfo: symbol(library, c"curl_easy_getinfo")?,
            easy_pause: symbol(library, c"curl_easy_pause")?,
            easy_strerror: symbol(library, c"curl_easy_strerror")?,
            multi_init: symbol(library, c"curl_multi_init")?,
            multi_add_handle: symbol(library, c"curl_multi_add_handle")?,
            multi_remove_handle: symbol(library, c"curl_multi_remove_handle")?,
            multi_perform: symbol(library, c"curl_multi_perform")?,
            // curl_multi_poll and curl_multi_wakeup arrived in 7.66 and 7.68.
            multi_poll: symbol(library, c"curl_multi_poll")?,
            multi_wakeup: symbol(library, c"curl_multi_wakeup")?,
            multi_info_read: symbol(library, c"curl_multi_info_read")?,
            slist_append: symbol(library, c"curl_slist_append")?,
            slist_free_all: symbol(library, c"curl_slist_free_all")?,
            ws_recv,
            ws_send,
            caps: Capabilities {
                streaming: true,
                upload_streaming: true,
                upload_progress: true,
                manual_redirects: true,
                auth_questions: false,
                native_auth_schemes: false,
                server_trust: false,
                // CURLOPT_SSLCERT_BLOB arrived in 7.71.0.
                client_identity: info.version_num >= 0x07_47_00,
                platform_cookies: false,
                platform_cache: false,
                metrics: true,
                websockets,
                websocket_ping: false,
                websocket_headers: websockets,
                wait_for_connectivity: false,
            },
        })
    }
}

/// What the loaded libcurl offers; nothing when there is none.
pub(crate) fn capabilities() -> Capabilities {
    library().map_or_else(Capabilities::default, |lib| lib.caps)
}

/// The native stack when libcurl loads.
pub(crate) fn tier() -> Tier {
    if library().is_some() {
        Tier::NativeStack
    } else {
        Tier::Unavailable
    }
}

/// The transport for one client.
pub(crate) fn transport(config: &TransportConfig) -> Arc<dyn Transport> {
    Arc::new(CurlTransport {
        config: config.clone(),
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Http,
    WebSocket,
}

/// Everything a new transfer needs, carried to the driver.
struct Job {
    id: u64,
    kind: Kind,
    prepared: Prepared,
    events: Option<Events>,
    ws_events: Option<WsEvents>,
    identity: Option<Identity>,
    timeout_total: Option<Duration>,
}

enum Command {
    Start(Box<Job>),
    Demand(u64, u32),
    Cancel(u64),
    WsSend(u64, Message, Completion),
    WsClose(u64, u16, String),
}

struct Driver {
    lib: &'static Lib,
    multi: usize,
    commands: Mutex<VecDeque<Command>>,
}

fn driver() -> Option<&'static Driver> {
    static DRIVER: OnceLock<Option<Driver>> = OnceLock::new();
    static STARTED: Once = Once::new();
    let driver = DRIVER
        .get_or_init(|| {
            let lib = library()?;
            // SAFETY: a loaded library's multi constructor.
            let multi = unsafe { (lib.multi_init)() };
            (!multi.is_null()).then(|| Driver {
                lib,
                multi: multi as usize,
                commands: Mutex::new(VecDeque::new()),
            })
        })
        .as_ref()?;
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("day-http-curl".into())
            .spawn(move || driver.run());
    });
    Some(driver)
}

enum Upload {
    None,
    Bytes { data: Arc<Vec<u8>>, at: usize },
    File(std::fs::File),
    Stream(Box<dyn Read + Send>),
}

/// One transfer, owned by the driver thread. Boxed, so the pointer its callbacks carry stays put.
struct Easy {
    lib: &'static Lib,
    kind: Kind,
    handle: *mut c_void,
    headers: *mut c_void,
    in_multi: bool,
    events: Option<Events>,
    ws_events: Option<WsEvents>,
    url: String,
    status: u16,
    head_headers: Vec<(String, String)>,
    head_done: bool,
    demand: u32,
    paused: bool,
    upload: Upload,
    last_sent: u64,
    protocol: Option<String>,
    connected: bool,
    socket: c_int,
    message: Vec<u8>,
    message_flags: c_int,
    sends: VecDeque<(Message, Completion)>,
}

impl Easy {
    fn emit(&self, event: Event) {
        if let Some(events) = &self.events {
            events(event);
        }
    }

    fn setopt_long(&self, option: c_int, value: c_long) -> c_int {
        // SAFETY: a LONG option takes a `long`.
        unsafe { (self.lib.easy_setopt)(self.handle, option, value) }
    }

    fn setopt_ptr(&self, option: c_int, value: *const c_void) -> c_int {
        // SAFETY: a pointer option takes a pointer that outlives the transfer or is copied.
        unsafe { (self.lib.easy_setopt)(self.handle, option, value) }
    }

    fn setopt_off(&self, option: c_int, value: i64) -> c_int {
        // SAFETY: an OFF_T option takes a `curl_off_t`.
        unsafe { (self.lib.easy_setopt)(self.handle, option, value) }
    }

    fn info_long(&self, info: c_int) -> Option<i64> {
        let mut value: c_long = 0;
        // SAFETY: a LONG info writes a `long`.
        let rc = unsafe { (self.lib.easy_getinfo)(self.handle, info, &mut value as *mut c_long) };
        // `long` is 32 bits on 32-bit Linux and 64 bits here.
        #[allow(clippy::useless_conversion)]
        (rc == E_OK).then_some(i64::from(value))
    }

    fn info_off(&self, info: c_int) -> Option<i64> {
        let mut value: i64 = 0;
        // SAFETY: an OFF_T info writes a `curl_off_t`.
        let rc = unsafe { (self.lib.easy_getinfo)(self.handle, info, &mut value as *mut i64) };
        (rc == E_OK).then_some(value)
    }

    fn info_string(&self, info: c_int) -> Option<String> {
        let mut value: *const c_char = std::ptr::null();
        // SAFETY: a STRING info writes a pointer owned by the handle.
        let rc =
            unsafe { (self.lib.easy_getinfo)(self.handle, info, &mut value as *mut *const c_char) };
        if rc != E_OK || value.is_null() {
            return None;
        }
        // SAFETY: a NUL-terminated string that lives as long as the handle.
        Some(
            unsafe { CStr::from_ptr(value) }
                .to_string_lossy()
                .into_owned(),
        )
    }

    fn header_line(&mut self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        let line = text.trim_end_matches(['\r', '\n']);
        if line.starts_with("HTTP/") {
            self.status = line
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            self.head_headers.clear();
            return;
        }
        if !line.is_empty() {
            if !self.head_done
                && let Some((name, value)) = line.split_once(':')
            {
                self.head_headers
                    .push((name.trim().to_string(), value.trim().to_string()));
            }
            return;
        }
        // The blank line ends a head; an informational one (100 Continue) is not the response.
        if self.head_done || (self.status < 200 && self.status != 101) {
            return;
        }
        self.head_done = true;
        if self.kind == Kind::WebSocket {
            self.protocol = self
                .head_headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("sec-websocket-protocol"))
                .map(|(_, v)| v.clone())
                .filter(|p| !p.is_empty());
            return;
        }
        let head = Head {
            status: self.status,
            headers: std::mem::take(&mut self.head_headers),
            url: self
                .info_string(INFO_EFFECTIVE_URL)
                .unwrap_or_else(|| self.url.clone()),
            expected_length: None,
        };
        // libcurl records the length only after the head, so read the header itself; an encoded
        // body's declared length is not the length the reader receives.
        let header = |name: &str| {
            head.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        };
        let expected_length = match header("content-encoding") {
            Some(encoding) if !encoding.eq_ignore_ascii_case("identity") => None,
            _ => header("content-length").and_then(|v| v.trim().parse().ok()),
        };
        let head = Head {
            expected_length,
            ..head
        };
        self.emit(Event::Head(head));
    }

    fn metrics(&self) -> Metrics {
        let micros = |info| self.info_off(info).and_then(|v| u64::try_from(v).ok());
        let dns = micros(INFO_NAMELOOKUP_TIME_T);
        let connected = micros(INFO_CONNECT_TIME_T);
        let tls = micros(INFO_APPCONNECT_TIME_T).filter(|t| *t > 0);
        Metrics {
            dns: dns.map(Duration::from_micros),
            connect: connected
                .zip(dns)
                .map(|(c, d)| Duration::from_micros(c.saturating_sub(d))),
            tls: tls
                .zip(connected)
                .map(|(t, c)| Duration::from_micros(t.saturating_sub(c))),
            first_byte: micros(INFO_STARTTRANSFER_TIME_T).map(Duration::from_micros),
            total: micros(INFO_TOTAL_TIME_T).map(Duration::from_micros),
            protocol: match self.info_long(INFO_HTTP_VERSION) {
                Some(1) => Some("http/1.0".into()),
                Some(2) => Some("http/1.1".into()),
                Some(3) => Some("h2".into()),
                Some(30) => Some("h3".into()),
                _ => None,
            },
            reused_connection: self.info_long(INFO_NUM_CONNECTS).map(|n| n == 0),
            proxy: None,
            remote_address: self.info_string(INFO_PRIMARY_IP),
            tls_version: None,
            from_cache: false,
            bytes_sent: micros(INFO_SIZE_UPLOAD_T),
            bytes_received: micros(INFO_SIZE_DOWNLOAD_T),
            redirects: 0,
        }
    }

    fn error(&self, code: c_int) -> HttpError {
        // SAFETY: curl_easy_strerror returns a static string for any code.
        let text = unsafe { CStr::from_ptr((self.lib.easy_strerror)(code)) }
            .to_string_lossy()
            .into_owned();
        match code {
            E_OPERATION_TIMEDOUT => HttpError::Timeout,
            E_COULDNT_RESOLVE_HOST | E_COULDNT_RESOLVE_PROXY => HttpError::Dns,
            E_COULDNT_CONNECT => HttpError::Connect,
            E_URL_MALFORMAT => HttpError::BadUrl(text),
            E_SSL_CONNECT_ERROR
            | E_SSL_CERTPROBLEM
            | E_SSL_CIPHER
            | E_PEER_FAILED_VERIFICATION
            | E_SSL_CACERT_BADFILE
            | E_SSL_ISSUER_ERROR
            | E_SSL_PINNEDPUBKEYNOTMATCH
            | E_SSL_INVALIDCERTSTATUS => HttpError::Tls(text),
            _ => HttpError::Io(text),
        }
    }

    /// Send what waits to go out on a connected WebSocket.
    fn flush_sends(&mut self) {
        let Some(ws_send) = self.lib.ws_send else {
            return;
        };
        while let Some((message, done)) = self.sends.pop_front() {
            let (flags, payload) = match &message {
                Message::Text(text) => (WS_TEXT, text.as_bytes().to_vec()),
                Message::Binary(bytes) => (WS_BINARY, bytes.clone()),
                Message::Close { code, reason } => {
                    let mut payload = code.to_be_bytes().to_vec();
                    payload.extend_from_slice(reason.as_bytes());
                    (WS_CLOSE, payload)
                }
            };
            let mut sent = 0usize;
            // SAFETY: a connected WebSocket handle and `payload.len()` readable bytes.
            let rc = unsafe {
                ws_send(
                    self.handle,
                    payload.as_ptr().cast(),
                    payload.len(),
                    &mut sent,
                    0,
                    flags as c_uint,
                )
            };
            match rc {
                E_OK => done(Ok(())),
                E_AGAIN => {
                    self.sends.push_front((message, done));
                    return;
                }
                code => done(Err(self.error(code))),
            }
        }
    }

    /// Read WebSocket messages while the reader has demand. `false` when the socket ended.
    fn pump_socket(&mut self) -> bool {
        let Some(ws_recv) = self.lib.ws_recv else {
            return false;
        };
        let mut buffer = vec![0u8; 64 << 10];
        while self.demand > 0 {
            let mut received = 0usize;
            let mut meta: *const WsFrame = std::ptr::null();
            // SAFETY: a connected WebSocket handle and a writable buffer.
            let rc = unsafe {
                ws_recv(
                    self.handle,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut received,
                    &mut meta,
                )
            };
            if rc == E_AGAIN {
                return true;
            }
            if rc != E_OK {
                self.close_with(WsEvent::Failed(self.error(rc)));
                return false;
            }
            // SAFETY: on success libcurl points `meta` at the frame it just read.
            let Some(frame) = (unsafe { meta.as_ref() }) else {
                continue;
            };
            if self.message.is_empty() {
                self.message_flags = frame.flags;
            }
            self.message.extend_from_slice(&buffer[..received]);
            if frame.bytesleft > 0 || frame.flags & WS_CONT != 0 {
                continue;
            }
            let data = std::mem::take(&mut self.message);
            let flags = self.message_flags;
            if flags & WS_CLOSE != 0 {
                let code = data
                    .get(..2)
                    .map_or(1005, |b| u16::from_be_bytes([b[0], b[1]]));
                let reason = String::from_utf8_lossy(data.get(2..).unwrap_or(&[])).into_owned();
                self.close_with(WsEvent::Closed { code, reason });
                return false;
            }
            let message = if flags & WS_TEXT != 0 {
                Message::Text(String::from_utf8_lossy(&data).into_owned())
            } else if flags & WS_BINARY != 0 {
                Message::Binary(data)
            } else {
                // Pings and pongs; libcurl answers pings itself.
                continue;
            };
            self.demand -= 1;
            if let Some(events) = &self.ws_events {
                events(WsEvent::Message(message));
            }
        }
        true
    }

    fn close_with(&mut self, event: WsEvent) {
        if let Some(events) = self.ws_events.take() {
            events(event);
        }
        for (_, done) in self.sends.drain(..) {
            done(Err(HttpError::Io("the WebSocket is closed".into())));
        }
    }
}

impl Drop for Easy {
    fn drop(&mut self) {
        // SAFETY: the handle and header list belong to this transfer alone, and the driver removed
        // the handle from its multi before dropping it.
        unsafe {
            if !self.headers.is_null() {
                (self.lib.slist_free_all)(self.headers);
            }
            (self.lib.easy_cleanup)(self.handle);
        }
    }
}

/// libcurl's view of one transfer, from the pointer its callbacks carry.
///
/// # Safety
/// `data` must be the `Easy` pointer the driver registered, called on the driver thread.
unsafe fn easy<'a>(data: *mut c_void) -> &'a mut Easy {
    // SAFETY: per the contract above.
    unsafe { &mut *(data as *mut Easy) }
}

extern "C" fn on_header(
    buffer: *mut c_char,
    size: usize,
    items: usize,
    data: *mut c_void,
) -> usize {
    let len = size.saturating_mul(items);
    // SAFETY: libcurl passes the registered Easy and `len` readable bytes, on the driver thread.
    let (easy, bytes) = unsafe {
        (
            easy(data),
            std::slice::from_raw_parts(buffer as *const u8, len),
        )
    };
    easy.header_line(bytes);
    len
}

extern "C" fn on_write(buffer: *mut c_char, size: usize, items: usize, data: *mut c_void) -> usize {
    let len = size.saturating_mul(items);
    // SAFETY: as in `on_header`.
    let easy = unsafe { easy(data) };
    if easy.demand == 0 {
        easy.paused = true;
        return WRITEFUNC_PAUSE;
    }
    easy.demand -= 1;
    // SAFETY: libcurl passes `len` readable bytes.
    let bytes = unsafe { std::slice::from_raw_parts(buffer as *const u8, len) }.to_vec();
    easy.emit(Event::Chunk(bytes));
    len
}

extern "C" fn on_read(buffer: *mut c_char, size: usize, items: usize, data: *mut c_void) -> usize {
    let len = size.saturating_mul(items);
    // SAFETY: libcurl passes the registered Easy and `len` writable bytes, on the driver thread.
    let (easy, out) = unsafe {
        (
            easy(data),
            std::slice::from_raw_parts_mut(buffer as *mut u8, len),
        )
    };
    let read = match &mut easy.upload {
        Upload::None => Ok(0),
        Upload::Bytes { data, at } => {
            let n = (data.len() - *at).min(out.len());
            out[..n].copy_from_slice(&data[*at..*at + n]);
            *at += n;
            Ok(n)
        }
        Upload::File(file) => file.read(out),
        Upload::Stream(reader) => reader.read(out),
    };
    read.unwrap_or(READFUNC_ABORT)
}

extern "C" fn on_progress(
    data: *mut c_void,
    _download_total: i64,
    _downloaded: i64,
    upload_total: i64,
    uploaded: i64,
) -> c_int {
    // SAFETY: as in `on_header`.
    let easy = unsafe { easy(data) };
    if let Ok(sent) = u64::try_from(uploaded)
        && sent > easy.last_sent
    {
        easy.last_sent = sent;
        easy.emit(Event::Sent {
            sent,
            total: u64::try_from(upload_total).ok().filter(|t| *t > 0),
        });
    }
    0
}

impl Driver {
    fn send(&self, command: Command) {
        lock(&self.commands).push_back(command);
        // SAFETY: curl_multi_wakeup is documented safe to call from any thread.
        unsafe { (self.lib.multi_wakeup)(self.multi as *mut c_void) };
    }

    fn run(&'static self) {
        let multi = self.multi as *mut c_void;
        let mut easies: HashMap<u64, Box<Easy>> = HashMap::new();
        let mut handles: HashMap<usize, u64> = HashMap::new();
        let mut fds: Vec<WaitFd> = Vec::new();
        loop {
            let commands: Vec<Command> = lock(&self.commands).drain(..).collect();
            for command in commands {
                self.apply(command, &mut easies, &mut handles);
            }
            let mut running: c_int = 0;
            // SAFETY: the multi handle is this thread's alone.
            unsafe { (self.lib.multi_perform)(multi, &mut running) };
            loop {
                let mut left: c_int = 0;
                // SAFETY: as above; the message is valid until the next multi call.
                let message = unsafe { (self.lib.multi_info_read)(multi, &mut left) };
                // SAFETY: a non-null message from curl_multi_info_read.
                let Some(message) = (unsafe { message.as_ref() }) else {
                    break;
                };
                if message.msg != MSG_DONE {
                    continue;
                }
                // `data` is a union whose `CURLcode` sits in the low bits on these targets.
                let code = (message.data as usize & 0xFFFF_FFFF) as c_int;
                if let Some(id) = handles.remove(&(message.easy as usize)) {
                    self.finish(id, code, &mut easies);
                }
            }
            let mut ended = Vec::new();
            fds.clear();
            for (id, easy) in easies.iter_mut() {
                if !easy.connected {
                    continue;
                }
                easy.flush_sends();
                if !easy.pump_socket() {
                    ended.push(*id);
                } else if easy.socket >= 0 {
                    fds.push(WaitFd {
                        fd: easy.socket,
                        events: WAIT_POLLIN,
                        revents: 0,
                    });
                }
            }
            for id in ended {
                easies.remove(&id);
            }
            // SAFETY: the multi handle and a valid (possibly empty) array of descriptors.
            unsafe {
                (self.lib.multi_poll)(
                    multi,
                    fds.as_mut_ptr(),
                    fds.len() as c_uint,
                    1000,
                    std::ptr::null_mut(),
                )
            };
        }
    }

    fn apply(
        &self,
        command: Command,
        easies: &mut HashMap<u64, Box<Easy>>,
        handles: &mut HashMap<usize, u64>,
    ) {
        let multi = self.multi as *mut c_void;
        match command {
            Command::Start(job) => {
                let (id, events, ws_events) = (job.id, job.events.clone(), job.ws_events.clone());
                match self.setup(*job) {
                    Ok(mut easy) => {
                        // SAFETY: a fresh easy handle joins this thread's multi handle.
                        let rc = unsafe { (self.lib.multi_add_handle)(multi, easy.handle) };
                        if rc != E_OK {
                            let error =
                                HttpError::Io(format!("libcurl refused the transfer ({rc})"));
                            match easy.kind {
                                Kind::Http => easy.emit(Event::Failed(error)),
                                Kind::WebSocket => easy.close_with(WsEvent::Failed(error)),
                            }
                            return;
                        }
                        easy.in_multi = true;
                        handles.insert(easy.handle as usize, id);
                        easies.insert(id, easy);
                    }
                    Err(error) => {
                        if let Some(events) = events {
                            events(Event::Failed(error.clone()));
                        }
                        if let Some(events) = ws_events {
                            events(WsEvent::Failed(error));
                        }
                    }
                }
            }
            Command::Demand(id, chunks) => {
                if let Some(easy) = easies.get_mut(&id) {
                    easy.demand += chunks;
                    if easy.paused {
                        easy.paused = false;
                        // SAFETY: unpausing this thread's handle; libcurl may deliver the held
                        // chunk through the write callback before returning.
                        unsafe { (self.lib.easy_pause)(easy.handle, PAUSE_CONT) };
                    }
                }
            }
            Command::Cancel(id) => {
                if let Some(mut easy) = easies.remove(&id) {
                    if easy.in_multi {
                        handles.remove(&(easy.handle as usize));
                        // SAFETY: removing this thread's handle before it is dropped.
                        unsafe { (self.lib.multi_remove_handle)(multi, easy.handle) };
                    }
                    easy.emit(Event::Failed(HttpError::Cancelled));
                    easy.events = None;
                    easy.close_with(WsEvent::Closed {
                        code: 1000,
                        reason: String::new(),
                    });
                }
            }
            Command::WsSend(id, message, done) => match easies.get_mut(&id) {
                Some(easy) => {
                    easy.sends.push_back((message, done));
                    if easy.connected {
                        easy.flush_sends();
                    }
                }
                None => done(Err(HttpError::Io("the WebSocket is closed".into()))),
            },
            Command::WsClose(id, code, reason) => {
                if let Some(easy) = easies.get_mut(&id) {
                    easy.sends
                        .push_back((Message::Close { code, reason }, Box::new(|_| {})));
                    if easy.connected {
                        easy.flush_sends();
                    }
                }
            }
        }
    }

    fn finish(&self, id: u64, code: c_int, easies: &mut HashMap<u64, Box<Easy>>) {
        let multi = self.multi as *mut c_void;
        let Some(mut easy) = easies.remove(&id) else {
            return;
        };
        // SAFETY: the transfer is done; its handle leaves this thread's multi handle.
        unsafe { (self.lib.multi_remove_handle)(multi, easy.handle) };
        easy.in_multi = false;
        match easy.kind {
            Kind::Http => {
                if code != E_OK {
                    let error = easy.error(code);
                    return easy.emit(Event::Failed(error));
                }
                if !easy.head_done {
                    return easy.emit(Event::Failed(HttpError::Io(
                        "the server sent no response".into(),
                    )));
                }
                easy.emit(Event::Metrics(easy.metrics()));
                easy.emit(Event::End);
            }
            Kind::WebSocket => {
                if code != E_OK || easy.status != 101 {
                    let error = if code != E_OK {
                        easy.error(code)
                    } else {
                        HttpError::Io(format!(
                            "the server answered {} to the upgrade",
                            easy.status
                        ))
                    };
                    return easy.close_with(WsEvent::Failed(error));
                }
                let mut socket: c_int = -1;
                // SAFETY: ACTIVESOCKET writes a `curl_socket_t`, an int on these targets.
                unsafe {
                    (self.lib.easy_getinfo)(
                        easy.handle,
                        INFO_ACTIVESOCKET,
                        &mut socket as *mut c_int,
                    )
                };
                easy.socket = socket;
                easy.connected = true;
                if let Some(events) = &easy.ws_events {
                    events(WsEvent::Open {
                        protocol: easy.protocol.clone(),
                    });
                }
                easy.flush_sends();
                easies.insert(id, easy);
            }
        }
    }

    fn setup(&self, job: Job) -> Result<Box<Easy>, HttpError> {
        let lib = self.lib;
        let prepared = job.prepared;
        // SAFETY: the loaded library's easy constructor.
        let handle = unsafe { (lib.easy_init)() };
        if handle.is_null() {
            return Err(HttpError::Io("libcurl could not start a transfer".into()));
        }
        let body_len = match &prepared.body {
            PreparedBody::Empty => Some(0),
            PreparedBody::Bytes(data) => Some(data.len() as u64),
            PreparedBody::File { len, .. } => Some(*len),
            PreparedBody::Stream { len, .. } => *len,
        };
        let has_body = !matches!(prepared.body, PreparedBody::Empty);
        let upload = match prepared.body {
            PreparedBody::Empty => Upload::None,
            PreparedBody::Bytes(data) => Upload::Bytes { data, at: 0 },
            PreparedBody::File { ref path, .. } => {
                Upload::File(std::fs::File::open(path).map_err(|e| HttpError::Io(e.to_string()))?)
            }
            PreparedBody::Stream { ref reader, .. } => {
                Upload::Stream(reader.take().ok_or_else(|| {
                    HttpError::Io("the request body stream was already sent".into())
                })?)
            }
        };
        let mut easy = Box::new(Easy {
            lib,
            kind: job.kind,
            handle,
            headers: std::ptr::null_mut(),
            in_multi: false,
            events: job.events,
            ws_events: job.ws_events,
            url: prepared.url.clone(),
            status: 0,
            head_headers: Vec::new(),
            head_done: false,
            demand: 0,
            paused: false,
            upload,
            last_sent: 0,
            protocol: None,
            connected: false,
            socket: -1,
            message: Vec::new(),
            message_flags: 0,
            sends: VecDeque::new(),
        });
        let this = (&mut *easy as *mut Easy).cast::<c_void>();
        let url = CString::new(prepared.url.clone())
            .map_err(|_| HttpError::BadUrl(prepared.url.clone()))?;
        // libcurl copies string options, so the CStrings may go once they are set.
        easy.setopt_ptr(OPT_URL, url.as_ptr().cast());
        easy.setopt_long(OPT_NOSIGNAL, 1);
        easy.setopt_long(OPT_FOLLOWLOCATION, 0);
        easy.setopt_ptr(OPT_HEADERFUNCTION, on_header as *const () as *const c_void);
        easy.setopt_ptr(OPT_HEADERDATA, this);
        let idle_ms = c_long::try_from(prepared.timeout_idle.as_millis()).unwrap_or(c_long::MAX);
        easy.setopt_long(OPT_CONNECTTIMEOUT_MS, idle_ms);
        if let Some(total) = job.timeout_total {
            easy.setopt_long(
                OPT_TIMEOUT_MS,
                c_long::try_from(total.as_millis()).unwrap_or(c_long::MAX),
            );
        }
        let mut headers = prepared.headers.clone();
        if job.kind == Kind::WebSocket {
            if !prepared.protocols.is_empty() {
                headers.push((
                    "Sec-WebSocket-Protocol".into(),
                    prepared.protocols.join(", "),
                ));
            }
            easy.setopt_long(OPT_CONNECT_ONLY, 2);
        } else {
            easy.setopt_ptr(OPT_WRITEFUNCTION, on_write as *const () as *const c_void);
            easy.setopt_ptr(OPT_WRITEDATA, this);
            easy.setopt_long(OPT_BUFFERSIZE, UPLOAD_BUFFER);
            // The idle bound: under a byte a second for that long ends the transfer.
            easy.setopt_long(OPT_LOW_SPEED_LIMIT, 1);
            easy.setopt_long(OPT_LOW_SPEED_TIME, (idle_ms / 1000).max(1));
            easy.setopt_long(OPT_HTTP_VERSION, HTTP_VERSION_2TLS);
            easy.setopt_long(OPT_NOPROGRESS, 0);
            easy.setopt_ptr(
                OPT_XFERINFOFUNCTION,
                on_progress as *const () as *const c_void,
            );
            easy.setopt_ptr(OPT_XFERINFODATA, this);
            let method = prepared.method.as_str();
            let size = body_len.and_then(|n| i64::try_from(n).ok()).unwrap_or(-1);
            match method {
                "GET" if !has_body => {}
                "HEAD" => {
                    easy.setopt_long(OPT_NOBODY, 1);
                }
                "POST" => {
                    easy.setopt_long(OPT_POST, 1);
                    easy.setopt_ptr(OPT_READFUNCTION, on_read as *const () as *const c_void);
                    easy.setopt_ptr(OPT_READDATA, this);
                    easy.setopt_off(OPT_POSTFIELDSIZE_LARGE, size);
                }
                _ if has_body => {
                    easy.setopt_long(OPT_UPLOAD, 1);
                    easy.setopt_ptr(OPT_READFUNCTION, on_read as *const () as *const c_void);
                    easy.setopt_ptr(OPT_READDATA, this);
                    easy.setopt_off(OPT_INFILESIZE_LARGE, size);
                    if method != "PUT" {
                        let verb = CString::new(method).map_err(|_| HttpError::Unsupported)?;
                        easy.setopt_ptr(OPT_CUSTOMREQUEST, verb.as_ptr().cast());
                    }
                }
                _ => {
                    let verb = CString::new(method).map_err(|_| HttpError::Unsupported)?;
                    easy.setopt_ptr(OPT_CUSTOMREQUEST, verb.as_ptr().cast());
                }
            }
        }
        let mut list: *mut c_void = std::ptr::null_mut();
        for (name, value) in &headers {
            let Ok(line) = CString::new(format!("{name}: {value}")) else {
                continue;
            };
            // SAFETY: curl_slist_append copies the string.
            let next = unsafe { (lib.slist_append)(list, line.as_ptr()) };
            if !next.is_null() {
                list = next;
            }
        }
        easy.headers = list;
        if !list.is_null() {
            easy.setopt_ptr(OPT_HTTPHEADER, list.cast_const());
        }
        if let Some(identity) = &job.identity {
            let mut blob = Blob {
                data: identity.pkcs12.as_ptr() as *mut c_void,
                len: identity.pkcs12.len(),
                flags: BLOB_COPY,
            };
            // BLOB_COPY: libcurl copies the bytes before this returns.
            easy.setopt_ptr(
                OPT_SSLCERT_BLOB,
                (&mut blob as *mut Blob).cast_const().cast(),
            );
            easy.setopt_ptr(OPT_SSLCERTTYPE, c"P12".as_ptr().cast());
            if let Ok(password) = CString::new(identity.password.clone()) {
                easy.setopt_ptr(OPT_KEYPASSWD, password.as_ptr().cast());
            }
        }
        Ok(easy)
    }
}

struct CurlTransport {
    config: TransportConfig,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl Transport for CurlTransport {
    fn capabilities(&self) -> Capabilities {
        capabilities()
    }

    fn start(&self, prepared: Prepared, events: Events) -> Arc<dyn Transfer> {
        let Some(driver) = driver() else {
            events(Event::Failed(HttpError::Unsupported));
            return Arc::new(CurlTransfer {
                id: 0,
                driver: None,
            });
        };
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        driver.send(Command::Start(Box::new(Job {
            id,
            kind: Kind::Http,
            prepared,
            events: Some(events),
            ws_events: None,
            identity: self.config.identity.clone(),
            timeout_total: self.config.timeout_total,
        })));
        Arc::new(CurlTransfer {
            id,
            driver: Some(driver),
        })
    }

    fn websocket(&self, prepared: Prepared, events: WsEvents) -> Arc<dyn Socket> {
        let driver = driver().filter(|_| capabilities().websockets);
        let Some(driver) = driver else {
            events(WsEvent::Failed(HttpError::Unsupported));
            return Arc::new(CurlTransfer {
                id: 0,
                driver: None,
            });
        };
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        driver.send(Command::Start(Box::new(Job {
            id,
            kind: Kind::WebSocket,
            prepared,
            events: None,
            ws_events: Some(events),
            identity: self.config.identity.clone(),
            timeout_total: None,
        })));
        Arc::new(CurlTransfer {
            id,
            driver: Some(driver),
        })
    }
}

/// The client's grip on one transfer or WebSocket: commands for the driver.
struct CurlTransfer {
    id: u64,
    driver: Option<&'static Driver>,
}

impl Transfer for CurlTransfer {
    fn demand(&self, chunks: u32) {
        if let Some(driver) = self.driver {
            driver.send(Command::Demand(self.id, chunks));
        }
    }

    fn answer(&self, _id: QuestionId, _answer: Answer) {}

    fn cancel(&self) {
        if let Some(driver) = self.driver {
            driver.send(Command::Cancel(self.id));
        }
    }
}

impl Socket for CurlTransfer {
    fn send(&self, message: Message, done: Completion) {
        match self.driver {
            Some(driver) => driver.send(Command::WsSend(self.id, message, done)),
            None => done(Err(HttpError::Unsupported)),
        }
    }

    fn ping(&self, done: Completion) {
        done(Err(HttpError::Unsupported));
    }

    fn close(&self, code: u16, reason: &str) {
        if let Some(driver) = self.driver {
            driver.send(Command::WsClose(self.id, code, reason.to_string()));
        }
    }

    fn demand(&self, messages: u32) {
        if let Some(driver) = self.driver {
            driver.send(Command::Demand(self.id, messages));
        }
    }
}

#[cfg(test)]
mod curl_tests {
    use super::*;
    use crate::client::wait;
    use crate::testing::Server;
    use crate::{Cache, Client, Cookies, Redirects, Request};

    fn client() -> Option<Client> {
        library()?;
        let config = TransportConfig {
            timeout_idle: Duration::from_secs(30),
            timeout_total: None,
            platform_cookies: false,
            platform_cache: None,
            redirects: Redirects::default(),
            ask_auth: false,
            ask_trust: false,
            identity: None,
            max_per_host: None,
            wait_for_connectivity: false,
        };
        Some(
            Client::builder()
                .transport(CurlTransport { config })
                .cookies(Cookies::jar())
                .cache(Cache::Off)
                .build(),
        )
    }

    #[test]
    fn fetches_follows_redirects_and_streams_through_libcurl() {
        let Some(client) = client() else {
            return;
        };
        let server = Server::start().expect("server");
        let resp = wait(client.fetch_future(Request::get(server.url("/")))).expect("fetch");
        assert_eq!((resp.status, resp.text().as_ref()), (200, "day-http-ok"));
        assert!(resp.metrics.is_some());

        let resp =
            wait(client.fetch_future(Request::get(server.url("/redirect/2")))).expect("hops");
        assert_eq!(resp.text(), "redirected");
        assert!(resp.url.ends_with("/redirect/0"), "{}", resp.url);

        let streaming =
            wait(client.send_future(Request::get(server.url("/bytes/3000000")))).expect("head");
        assert_eq!(streaming.expected_length(), Some(3_000_000));
        let mut body = streaming.into_body();
        let (mut total, mut chunks) = (0u64, 0);
        while let Some(chunk) = wait(body.next()) {
            total += chunk.expect("chunk").len() as u64;
            chunks += 1;
        }
        assert_eq!(total, 3_000_000);
        assert!(chunks > 1);
    }

    #[test]
    fn uploads_and_challenges_go_through_libcurl() {
        let Some(_) = client() else {
            return;
        };
        let server = Server::start().expect("server");
        let config_client = Client::builder()
            .transport(CurlTransport {
                config: TransportConfig {
                    timeout_idle: Duration::from_secs(30),
                    timeout_total: None,
                    platform_cookies: false,
                    platform_cache: None,
                    redirects: Redirects::default(),
                    ask_auth: true,
                    ask_trust: false,
                    identity: None,
                    max_per_host: None,
                    wait_for_connectivity: false,
                },
            })
            .cookies(Cookies::jar())
            .on_challenge(|_, reply| reply.credential("day", "sunrise"))
            .build();
        let resp =
            wait(config_client.fetch_future(Request::get(server.url("/digest-auth/day/sunrise"))))
                .expect("digest");
        assert_eq!(resp.text(), "authenticated day");

        let payload: Vec<u8> = (0..200_000u32).map(|i| (i % 7) as u8).collect();
        let stream = Request::post(server.url("/upload"), Vec::new())
            .body_stream(std::io::Cursor::new(payload.clone()), None);
        let resp = wait(config_client.fetch_future(stream)).expect("upload");
        assert!(resp.text().starts_with("200000 "), "{}", resp.text());
        let put = Request::put(server.url("/upload"), payload);
        let resp = wait(config_client.fetch_future(put)).expect("put");
        assert!(resp.text().starts_with("200000 "), "{}", resp.text());
    }
}
