// ---------------------------------------------------------------------------
// Windows: WinHTTP (winhttp.dll) — the system HTTP stack: automatic proxy/PAC
// (WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY), schannel TLS + the Windows certificate stores
// (enterprise/AD roots included). Written blind (no Windows host), like the other parts'
// Windows halves: every symbol is resolved dynamically from winhttp.dll, so a missing DLL
// degrades to `Unsupported` instead of failing to load the process. Sync mode only — the
// caller owns the thread, per this crate's contract.
// ---------------------------------------------------------------------------

#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::ffi::c_void;
use std::path::Path;
use std::sync::OnceLock;

use super::{Download, HttpError, Request, Response, Tier};

pub const TIER: Tier = Tier::NativeStack;

type HINTERNET = *mut c_void;
type DWORD = u32;
type BOOL = i32;
type LPCWSTR = *const u16;

const WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY: DWORD = 4;
const WINHTTP_FLAG_SECURE: DWORD = 0x0080_0000;
const WINHTTP_ADDREQ_FLAG_ADD: DWORD = 0x2000_0000;
const WINHTTP_QUERY_STATUS_CODE: DWORD = 19;
const WINHTTP_QUERY_RAW_HEADERS_CRLF: DWORD = 22;
const WINHTTP_QUERY_FLAG_NUMBER: DWORD = 0x2000_0000;

// ERROR_WINHTTP_* (winhttp.h; 12000-base)
const E_TIMEOUT: DWORD = 12002;
const E_INVALID_URL: DWORD = 12005;
const E_UNRECOGNIZED_SCHEME: DWORD = 12006;
const E_NAME_NOT_RESOLVED: DWORD = 12007;
const E_CANNOT_CONNECT: DWORD = 12029;
const E_CONNECTION_ERROR: DWORD = 12030;
const SECURE_ERRORS: [DWORD; 7] = [12037, 12038, 12044, 12045, 12057, 12157, 12175];

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

        // Status code (numeric query).
        let mut status: DWORD = 0;
        let mut len = std::mem::size_of::<DWORD>() as DWORD;
        if (api.query_headers)(
            request.1,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            std::ptr::null(),
            (&mut status) as *mut DWORD as *mut c_void,
            &mut len,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(map_error(GetLastError()));
        }

        // Raw response headers ("HTTP/1.1 200 OK\r\nK: V\r\n…").
        let mut headers = Vec::new();
        let mut hlen: DWORD = 0;
        (api.query_headers)(
            request.1,
            WINHTTP_QUERY_RAW_HEADERS_CRLF,
            std::ptr::null(),
            std::ptr::null_mut(),
            &mut hlen,
            std::ptr::null_mut(),
        );
        if hlen > 0 {
            let mut buf = vec![0u16; (hlen as usize).div_ceil(2)];
            if (api.query_headers)(
                request.1,
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

        if !on_head(status as u16, &headers) {
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

        Ok((status as u16, headers, written))
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
