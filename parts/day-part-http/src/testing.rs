// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! A local HTTP/1.1 and WebSocket server for tests and demonstrations (docs/http.md "Testing").
//!
//! It binds a loopback port and answers each connection on its own thread, with no network
//! needed. The crate's tests, the Showcase's Networking page and an app's own tests use it to
//! exercise redirects, challenges, cookies, caching, ranges, drip-fed bodies, uploads and a
//! WebSocket echo the same way on every platform that allows a listening socket.
//!
//! | route | answer |
//! |---|---|
//! | `/` | `day-http-ok` for GET and HEAD, `day-http-ok:<METHOD>` otherwise |
//! | `/headers` | the request headers, one per line |
//! | `/echo` | the request body, with `X-Day-Method` |
//! | `/status/<code>` | that status |
//! | `/delay/<ms>` | `waited <ms>` after the delay |
//! | `/redirect/<n>` | a chain of `n` 302s ending in `redirected` |
//! | `/redirect-to?url=<url>&status=<code>` | one redirect to `url` |
//! | `/basic-auth/<user>/<password>` | a Basic challenge, then `authenticated <user>` |
//! | `/digest-auth/<user>/<password>` | a Digest challenge (MD5, `qop=auth`), then `authenticated <user>` |
//! | `/bearer/<token>` | a Bearer challenge, then `authenticated` |
//! | `/cookies` | the `Cookie` header received, or `none` |
//! | `/cookies/set?<name>=<value>&…` | `Set-Cookie` for each pair, then a 302 to `/cookies` |
//! | `/cookies/delete?<name>&…` | each cookie expired, then a 302 to `/cookies` |
//! | `/cache/<seconds>` | `max-age=<seconds>` and `generated <n>`, where `n` counts real hits |
//! | `/drip?chunks=<n>&size=<bytes>&delay_ms=<ms>` | a chunked body fed over time |
//! | `/bytes/<n>?rate=<bytes per second>` | `n` deterministic bytes, honoring `Range` and `If-Range` |
//! | `/bytes/<n>/sha256` | the hex SHA-256 of those bytes |
//! | `/upload` | `<bytes> <hex SHA-256>` of the request body |
//! | `/ws/echo` | a WebSocket that echoes text and binary messages, answers pings, and echoes the close; the text `close:<code>:<reason>` makes the server close first |

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use sha2::{Digest, Sha256};

use crate::client::{base64_encode, basic_authorization, hex, sha256, verify_digest};

const REALM: &str = "Day test";
const HEAD_LIMIT: usize = 64 << 10;
const FRAME_LIMIT: u64 = 16 << 20;

/// A running test server. Dropping it stops accepting connections; connections already open
/// finish what they are doing.
pub struct Server {
    addr: SocketAddr,
    shared: Arc<Shared>,
}

struct Shared {
    stop: AtomicBool,
    nonce: String,
    hits: Mutex<HashMap<String, u64>>,
    digests: Mutex<HashMap<u64, String>>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server").field("addr", &self.addr).finish()
    }
}

impl Server {
    /// Bind a free loopback port and start serving.
    pub fn start() -> io::Result<Server> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let seed = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            nonce: hex(&sha256(format!("{addr}:{seed}").as_bytes())[..12]),
            hits: Mutex::new(HashMap::new()),
            digests: Mutex::new(HashMap::new()),
        });
        let accept = shared.clone();
        std::thread::Builder::new()
            .name("day-http-test-server".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    if accept.stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(stream) = stream else {
                        continue;
                    };
                    let conn = accept.clone();
                    let _ = std::thread::Builder::new()
                        .name("day-http-test-connection".into())
                        .spawn(move || {
                            let _ = serve(stream, &conn);
                        });
                }
            })?;
        Ok(Server { addr, shared })
    }

    /// The port the server listens on.
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// An `http://` URL for `path` on this server.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}/{}", self.addr, path.trim_start_matches('/'))
    }

    /// A `ws://` URL for `path` on this server.
    pub fn ws_url(&self, path: &str) -> String {
        format!("ws://{}/{}", self.addr, path.trim_start_matches('/'))
    }

    /// How many requests reached `path`, query excluded.
    pub fn hits(&self, path: &str) -> u64 {
        let key = format!("/{}", path.trim_start_matches('/'));
        lock(&self.shared.hits).get(&key).copied().unwrap_or(0)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(200));
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

struct Incoming {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    http11: bool,
}

impl Incoming {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn query(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn query_u64(&self, name: &str, default: u64) -> u64 {
        self.query(name)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }
}

fn serve(stream: TcpStream, shared: &Shared) -> io::Result<()> {
    let _ = stream.set_nodelay(true);
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut out = stream;
    loop {
        let Some(request) = read_head(&mut reader)? else {
            return Ok(());
        };
        *lock(&shared.hits).entry(request.path.clone()).or_insert(0) += 1;
        if request
            .header("expect")
            .is_some_and(|v| v.eq_ignore_ascii_case("100-continue"))
        {
            out.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
        }
        if request.path.starts_with("/ws/")
            && request
                .header("upgrade")
                .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
        {
            return websocket(reader, out, &request);
        }
        let keep = request.http11
            && !request
                .header("connection")
                .is_some_and(|v| v.eq_ignore_ascii_case("close"));
        route(&mut reader, &mut out, &request, shared, keep)?;
        out.flush()?;
        if !keep {
            let _ = out.shutdown(Shutdown::Both);
            return Ok(());
        }
    }
}

fn read_head(reader: &mut BufReader<TcpStream>) -> io::Result<Option<Incoming>> {
    let mut lines = Vec::new();
    let mut total = 0;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        total += n;
        if total > HEAD_LIMIT {
            return Err(io::Error::other("request head too large"));
        }
        let line = line.trim_end_matches(['\r', '\n']).to_string();
        if line.is_empty() {
            if lines.is_empty() {
                continue;
            }
            break;
        }
        lines.push(line);
    }
    let mut first = lines[0].split_whitespace();
    let method = first.next().unwrap_or("GET").to_string();
    let target = first.next().unwrap_or("/").to_string();
    let http11 = first.next() != Some("HTTP/1.0");
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (
            p.to_string(),
            url::form_urlencoded::parse(q.as_bytes())
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect(),
        ),
        None => (target, Vec::new()),
    };
    let headers = lines[1..]
        .iter()
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    Ok(Some(Incoming {
        method,
        path,
        query,
        headers,
        http11,
    }))
}

/// Read the request body block by block, by `Content-Length` or chunked framing.
fn read_body(
    reader: &mut BufReader<TcpStream>,
    request: &Incoming,
    mut block: impl FnMut(&[u8]),
) -> io::Result<()> {
    let mut buf = vec![0u8; 64 << 10];
    if request
        .header("transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"))
    {
        loop {
            let mut size_line = String::new();
            reader.read_line(&mut size_line)?;
            let size_text = size_line.trim().split(';').next().unwrap_or("0");
            let mut left = u64::from_str_radix(size_text, 16)
                .map_err(|_| io::Error::other("bad chunk size"))?;
            if left == 0 {
                let mut trailer = String::new();
                while reader.read_line(&mut trailer)? > 2 {
                    trailer.clear();
                }
                return Ok(());
            }
            while left > 0 {
                let want = left.min(buf.len() as u64) as usize;
                let n = reader.read(&mut buf[..want])?;
                if n == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                block(&buf[..n]);
                left -= n as u64;
            }
            let mut crlf = [0u8; 2];
            reader.read_exact(&mut crlf)?;
        }
    }
    let mut left: u64 = request
        .header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    while left > 0 {
        let want = left.min(buf.len() as u64) as usize;
        let n = reader.read(&mut buf[..want])?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        block(&buf[..n]);
        left -= n as u64;
    }
    Ok(())
}

fn reason(status: u16) -> &'static str {
    match status {
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        407 => "Proxy Authentication Required",
        416 => "Range Not Satisfiable",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

fn write_head(
    out: &mut TcpStream,
    status: u16,
    headers: &[(&str, String)],
    len: Option<u64>,
    keep: bool,
) -> io::Result<()> {
    let mut head = format!("HTTP/1.1 {status} {}\r\n", reason(status));
    head.push_str(&format!(
        "Date: {}\r\nServer: day-part-http test server\r\n",
        httpdate::fmt_http_date(SystemTime::now())
    ));
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    match len {
        Some(n) => head.push_str(&format!("Content-Length: {n}\r\n")),
        None => head.push_str("Transfer-Encoding: chunked\r\n"),
    }
    if !keep {
        head.push_str("Connection: close\r\n");
    }
    head.push_str("\r\n");
    out.write_all(head.as_bytes())
}

fn respond(
    out: &mut TcpStream,
    request: &Incoming,
    status: u16,
    headers: &[(&str, String)],
    body: &[u8],
    keep: bool,
) -> io::Result<()> {
    write_head(out, status, headers, Some(body.len() as u64), keep)?;
    if request.method != "HEAD" {
        out.write_all(body)?;
    }
    Ok(())
}

fn text() -> (&'static str, String) {
    ("Content-Type", "text/plain; charset=utf-8".to_string())
}

fn route(
    reader: &mut BufReader<TcpStream>,
    out: &mut TcpStream,
    request: &Incoming,
    shared: &Shared,
    keep: bool,
) -> io::Result<()> {
    let segments: Vec<&str> = request.path.trim_start_matches('/').split('/').collect();
    if request.path == "/upload" {
        let mut hasher = Sha256::new();
        let mut len = 0u64;
        read_body(reader, request, |block| {
            hasher.update(block);
            len += block.len() as u64;
        })?;
        let body = format!("{len} {}", hex(hasher.finalize().as_slice()));
        return respond(out, request, 200, &[text()], body.as_bytes(), keep);
    }
    let mut body = Vec::new();
    read_body(reader, request, |block| body.extend_from_slice(block))?;

    match segments.as_slice() {
        [""] => {
            let answer = if matches!(request.method.as_str(), "GET" | "HEAD") {
                "day-http-ok".to_string()
            } else {
                format!("day-http-ok:{}", request.method)
            };
            respond(out, request, 200, &[text()], answer.as_bytes(), keep)
        }
        ["headers"] => {
            let mut lines = String::new();
            for (k, v) in &request.headers {
                lines.push_str(&format!("{k}: {v}\n"));
            }
            respond(out, request, 200, &[text()], lines.as_bytes(), keep)
        }
        ["echo"] => {
            let mut headers = vec![("X-Day-Method", request.method.clone())];
            if let Some(ct) = request.header("content-type") {
                headers.push(("Content-Type", ct.to_string()));
            }
            respond(out, request, 200, &headers, &body, keep)
        }
        ["status", code] => {
            let status = code
                .parse::<u16>()
                .ok()
                .filter(|s| (200..=599).contains(s))
                .unwrap_or(400);
            let answer = format!("status {status}");
            respond(out, request, status, &[text()], answer.as_bytes(), keep)
        }
        ["delay", ms] => {
            let ms = ms.parse::<u64>().unwrap_or(0).min(60_000);
            std::thread::sleep(Duration::from_millis(ms));
            let answer = format!("waited {ms}");
            respond(out, request, 200, &[text()], answer.as_bytes(), keep)
        }
        ["redirect", n] => match n.parse::<u32>() {
            Ok(0) => respond(out, request, 200, &[text()], b"redirected", keep),
            Ok(n) => respond(
                out,
                request,
                302,
                &[("Location", format!("/redirect/{}", n - 1))],
                b"",
                keep,
            ),
            Err(_) => respond(out, request, 400, &[text()], b"bad count", keep),
        },
        ["redirect-to"] => {
            let status = request
                .query("status")
                .and_then(|s| s.parse::<u16>().ok())
                .filter(|s| matches!(s, 301 | 302 | 303 | 307 | 308))
                .unwrap_or(302);
            let target = request.query("url").unwrap_or("/").to_string();
            respond(out, request, status, &[("Location", target)], b"", keep)
        }
        ["basic-auth", user, password] => {
            if request.header("authorization") == Some(basic_authorization(user, password).as_str())
            {
                let answer = format!("authenticated {user}");
                respond(out, request, 200, &[text()], answer.as_bytes(), keep)
            } else {
                let challenge = format!("Basic realm=\"{REALM}\"");
                respond(
                    out,
                    request,
                    401,
                    &[("WWW-Authenticate", challenge), text()],
                    b"credentials required",
                    keep,
                )
            }
        }
        ["digest-auth", user, password] => {
            let nonce = &shared.nonce;
            let ok = request.header("authorization").is_some_and(|a| {
                a.contains(&format!("nonce=\"{nonce}\""))
                    && verify_digest(a, user, password, &request.method)
            });
            if ok {
                let answer = format!("authenticated {user}");
                respond(out, request, 200, &[text()], answer.as_bytes(), keep)
            } else {
                let challenge = format!(
                    "Digest realm=\"{REALM}\", qop=\"auth\", algorithm=MD5, nonce=\"{nonce}\", \
                     opaque=\"day\""
                );
                respond(
                    out,
                    request,
                    401,
                    &[("WWW-Authenticate", challenge), text()],
                    b"credentials required",
                    keep,
                )
            }
        }
        ["bearer", token] => {
            if request.header("authorization") == Some(format!("Bearer {token}").as_str()) {
                respond(out, request, 200, &[text()], b"authenticated", keep)
            } else {
                let challenge = format!("Bearer realm=\"{REALM}\"");
                respond(
                    out,
                    request,
                    401,
                    &[("WWW-Authenticate", challenge), text()],
                    b"token required",
                    keep,
                )
            }
        }
        ["cookies"] => {
            let answer = request.header("cookie").unwrap_or("none").to_string();
            respond(out, request, 200, &[text()], answer.as_bytes(), keep)
        }
        ["cookies", "set"] | ["cookies", "delete"] => {
            let delete = segments[1] == "delete";
            let mut headers: Vec<(&str, String)> = request
                .query
                .iter()
                .map(|(k, v)| {
                    let cookie = if delete {
                        format!("{k}=; Path=/; Max-Age=0")
                    } else {
                        format!("{k}={v}; Path=/")
                    };
                    ("Set-Cookie", cookie)
                })
                .collect();
            headers.push(("Location", "/cookies".to_string()));
            respond(out, request, 302, &headers, b"", keep)
        }
        ["cache", seconds] => {
            let seconds = seconds.parse::<u64>().unwrap_or(60);
            let etag = format!("\"day-cache-{seconds}\"");
            if request.header("if-none-match") == Some(etag.as_str()) {
                return respond(out, request, 304, &[("ETag", etag)], b"", keep);
            }
            let generated = {
                let mut hits = lock(&shared.hits);
                let n = hits
                    .entry(format!("generated:{}", request.path))
                    .or_insert(0);
                *n += 1;
                *n
            };
            let answer = format!("generated {generated}");
            respond(
                out,
                request,
                200,
                &[
                    ("Cache-Control", format!("public, max-age={seconds}")),
                    ("ETag", etag),
                    text(),
                ],
                answer.as_bytes(),
                keep,
            )
        }
        ["drip"] => {
            let chunks = request.query_u64("chunks", 10).min(10_000);
            let size = request.query_u64("size", 1024).clamp(1, 1 << 20) as usize;
            let delay = Duration::from_millis(request.query_u64("delay_ms", 100).min(10_000));
            write_head(
                out,
                200,
                &[("Content-Type", "application/octet-stream".into())],
                None,
                keep,
            )?;
            if request.method == "HEAD" {
                return Ok(());
            }
            for i in 0..chunks {
                if i > 0 {
                    std::thread::sleep(delay);
                }
                let block = vec![b'a' + (i % 26) as u8; size];
                out.write_all(format!("{size:x}\r\n").as_bytes())?;
                out.write_all(&block)?;
                out.write_all(b"\r\n")?;
                out.flush()?;
            }
            out.write_all(b"0\r\n\r\n")
        }
        ["bytes", n, "sha256"] => {
            let n = n.parse::<u64>().unwrap_or(0);
            let digest = lock(&shared.digests).get(&n).cloned();
            let digest = digest.unwrap_or_else(|| {
                let d = pattern_sha256(n);
                lock(&shared.digests).insert(n, d.clone());
                d
            });
            respond(out, request, 200, &[text()], digest.as_bytes(), keep)
        }
        ["bytes", n] => {
            let total = n.parse::<u64>().unwrap_or(0).min(8 << 30);
            bytes(out, request, total, keep)
        }
        _ => respond(out, request, 404, &[text()], b"not found", keep),
    }
}

/// The byte at `offset` in every `/bytes` body.
pub fn pattern_byte(offset: u64) -> u8 {
    (offset.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 56) as u8
}

fn fill_pattern(start: u64, buf: &mut [u8]) {
    for (i, b) in buf.iter_mut().enumerate() {
        *b = pattern_byte(start + i as u64);
    }
}

/// The hex SHA-256 of the first `len` pattern bytes.
pub fn pattern_sha256(len: u64) -> String {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 << 10];
    let mut offset = 0;
    while offset < len {
        let n = (len - offset).min(buf.len() as u64) as usize;
        fill_pattern(offset, &mut buf[..n]);
        hasher.update(&buf[..n]);
        offset += n as u64;
    }
    hex(hasher.finalize().as_slice())
}

fn bytes(out: &mut TcpStream, request: &Incoming, total: u64, keep: bool) -> io::Result<()> {
    let etag = format!("\"day-bytes-{total}\"");
    let rate = request.query_u64("rate", 0);
    let if_range_ok = request.header("if-range").is_none_or(|v| v == etag);
    let range = request
        .header("range")
        .filter(|_| if_range_ok)
        .and_then(|r| parse_range(r, total));
    let mut headers = vec![
        ("Accept-Ranges", "bytes".to_string()),
        ("ETag", etag),
        ("Content-Type", "application/octet-stream".to_string()),
    ];
    let (status, start, end) = match range {
        Some(Ok((start, end))) => {
            headers.push(("Content-Range", format!("bytes {start}-{end}/{total}")));
            (206, start, end + 1)
        }
        Some(Err(())) => {
            headers.push(("Content-Range", format!("bytes */{total}")));
            return respond(out, request, 416, &headers, b"", keep);
        }
        None => (200, 0, total),
    };
    write_head(out, status, &headers, Some(end - start), keep)?;
    if request.method == "HEAD" {
        return Ok(());
    }
    let began = Instant::now();
    let mut buf = vec![0u8; 16 << 10];
    let mut offset = start;
    while offset < end {
        let n = (end - offset).min(buf.len() as u64) as usize;
        fill_pattern(offset, &mut buf[..n]);
        out.write_all(&buf[..n])?;
        offset += n as u64;
        if rate > 0 {
            let due = Duration::from_secs_f64((offset - start) as f64 / rate as f64);
            if let Some(ahead) = due.checked_sub(began.elapsed()) {
                std::thread::sleep(ahead);
            }
        }
    }
    Ok(())
}

/// One `bytes=` range within `total`: `Ok((first, last))` inclusive, `Err` when unsatisfiable,
/// `None` when the header is not a single byte range.
fn parse_range(header: &str, total: u64) -> Option<Result<(u64, u64), ()>> {
    let spec = header.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (first, last) = spec.split_once('-')?;
    let (first, last) = match (first.trim(), last.trim()) {
        ("", suffix) => {
            let n: u64 = suffix.parse().ok()?;
            if n == 0 || total == 0 {
                return Some(Err(()));
            }
            (total.saturating_sub(n), total - 1)
        }
        (a, "") => (a.parse().ok()?, total.saturating_sub(1)),
        (a, b) => (
            a.parse().ok()?,
            b.parse::<u64>().ok()?.min(total.saturating_sub(1)),
        ),
    };
    if total == 0 || first >= total || first > last {
        return Some(Err(()));
    }
    Some(Ok((first, last)))
}

fn websocket(
    mut reader: BufReader<TcpStream>,
    mut out: TcpStream,
    request: &Incoming,
) -> io::Result<()> {
    if request.path != "/ws/echo" {
        return respond(&mut out, request, 404, &[text()], b"not found", false);
    }
    let key = request.header("sec-websocket-key").unwrap_or("");
    let accept = base64_encode(&sha1(
        format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
    ));
    let mut head = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept}\r\n"
    );
    if let Some(protocol) = request
        .header("sec-websocket-protocol")
        .and_then(|p| p.split(',').next())
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        head.push_str(&format!("Sec-WebSocket-Protocol: {protocol}\r\n"));
    }
    head.push_str("\r\n");
    out.write_all(head.as_bytes())?;

    let mut partial: Option<(u8, Vec<u8>)> = None;
    loop {
        let (fin, opcode, payload) = read_frame(&mut reader)?;
        match opcode {
            0x0 => {
                if let Some((_, data)) = partial.as_mut() {
                    data.extend_from_slice(&payload);
                }
                if fin && let Some((op, data)) = partial.take() {
                    write_frame(&mut out, op, &data)?;
                }
            }
            0x1 if fin => {
                if let Some(command) = std::str::from_utf8(&payload)
                    .ok()
                    .and_then(|t| t.strip_prefix("close:"))
                {
                    let (code, reason) = command.split_once(':').unwrap_or((command, ""));
                    let code = code.parse::<u16>().unwrap_or(1000);
                    let mut close = code.to_be_bytes().to_vec();
                    close.extend_from_slice(reason.as_bytes());
                    write_frame(&mut out, 0x8, &close)?;
                } else {
                    write_frame(&mut out, 0x1, &payload)?;
                }
            }
            0x1 | 0x2 if !fin => partial = Some((opcode, payload)),
            0x2 => write_frame(&mut out, 0x2, &payload)?,
            0x8 => {
                write_frame(&mut out, 0x8, &payload)?;
                let _ = out.shutdown(Shutdown::Both);
                return Ok(());
            }
            0x9 => write_frame(&mut out, 0xA, &payload)?,
            0xA => {}
            _ => return Ok(()),
        }
    }
}

fn read_frame(reader: &mut impl Read) -> io::Result<(bool, u8, Vec<u8>)> {
    let mut two = [0u8; 2];
    reader.read_exact(&mut two)?;
    let fin = two[0] & 0x80 != 0;
    let opcode = two[0] & 0x0F;
    let masked = two[1] & 0x80 != 0;
    let len = match two[1] & 0x7F {
        126 => {
            let mut b = [0u8; 2];
            reader.read_exact(&mut b)?;
            u64::from(u16::from_be_bytes(b))
        }
        127 => {
            let mut b = [0u8; 8];
            reader.read_exact(&mut b)?;
            u64::from_be_bytes(b)
        }
        n => u64::from(n),
    };
    if len > FRAME_LIMIT {
        return Err(io::Error::other("frame too large"));
    }
    let mut mask = [0u8; 4];
    if masked {
        reader.read_exact(&mut mask)?;
    }
    let mut payload = vec![0u8; len as usize];
    reader.read_exact(&mut payload)?;
    if masked {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
    }
    Ok((fin, opcode, payload))
}

fn write_frame(out: &mut impl Write, opcode: u8, payload: &[u8]) -> io::Result<()> {
    let mut frame = vec![0x80 | opcode];
    match payload.len() {
        n if n < 126 => frame.push(n as u8),
        n if n <= 0xFFFF => {
            frame.push(126);
            frame.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            frame.push(127);
            frame.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(payload);
    out.write_all(&frame)?;
    out.flush()
}

/// SHA-1, for the WebSocket handshake's accept key only (RFC 6455 §4.2.2).
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in message.as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for (slot, word) in w.iter_mut().zip(block.as_chunks::<4>().0) {
            *slot = u32::from_be_bytes(*word);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e]) {
            *slot = slot.wrapping_add(v);
        }
    }
    let mut out = [0u8; 20];
    for (chunk, v) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
        *chunk = v.to_be_bytes();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_matches_the_reference_vectors() {
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        let accept = base64_encode(&sha1(
            b"dGhlIHNhbXBsZSBub25jZQ==258EAFA5-E914-47DA-95CA-C5AB0DC85B11",
        ));
        assert_eq!(accept, "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn ranges_parse_within_the_total() {
        assert_eq!(parse_range("bytes=0-9", 100), Some(Ok((0, 9))));
        assert_eq!(parse_range("bytes=90-", 100), Some(Ok((90, 99))));
        assert_eq!(parse_range("bytes=-10", 100), Some(Ok((90, 99))));
        assert_eq!(parse_range("bytes=95-200", 100), Some(Ok((95, 99))));
        assert_eq!(parse_range("bytes=100-", 100), Some(Err(())));
        assert_eq!(parse_range("bytes=0-1,5-6", 100), None);
    }
}
