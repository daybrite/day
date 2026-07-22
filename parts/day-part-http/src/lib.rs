//! day-part-http — HEADLESS cross-platform HTTP(S) through each platform's NATIVE networking
//! stack (docs/http.md). No UI; any Rust code can depend on this crate and [`fetch`].
//!
//! ```no_run
//! let resp = day_part_http::fetch(&day_part_http::Request::get("https://example.com"))?;
//! println!("{} {} bytes", resp.status, resp.body.len());
//! # Ok::<(), day_part_http::HttpError>(())
//! ```
//!
//! Why native stacks instead of a Rust HTTP crate: the request inherits the SYSTEM configuration —
//! proxies + PAC, per-network VPN routing, Low Data Mode ([`Request::allow_constrained`]),
//! enterprise/MDM certificate stores — and the binary carries no bundled TLS on the native
//! targets. macOS/iOS use `NSURLSession`, Android `java.net.HttpURLConnection` (via this crate's
//! Java shim), Windows `WinHTTP`; Linux and HarmonyOS (whose OSS NDK has no HTTP C API) use a
//! cfg-gated Rust fallback (`ureq` + rustls). [`tier`] reports which one an app got.
//!
//! **Threading.** [`fetch`] BLOCKS the calling thread — run it on your own thread, never the UI
//! thread. [`fetch_async`]'s completion runs on an unspecified BACKGROUND thread (NSURLSession's
//! delegate queue on Apple; a spawned thread elsewhere); deliver results into the UI by capturing
//! a [`day_reactive::Signal::setter`]-style setter in the callback — setters hop to the UI thread
//! themselves and silently no-op after disposal, so late completions are harmless (DESIGN §4.5):
//!
//! ```ignore
//! let done = body_signal.setter();
//! day_part_http::fetch_async(Request::get(url), move |result| {
//!     if let Ok(resp) = result { done.set(Some(Arc::new(resp.body))); }
//! });
//! ```
//!
//! Every field and option is **best-effort per platform** (docs/http.md has the honest matrix):
//! `allow_expensive`/`allow_constrained` are native on Apple and advisory elsewhere; redirects
//! are always followed in v1; HTTP 4xx/5xx are `Ok` responses (check [`Response::status`]), not
//! errors.

use std::borrow::Cow;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// An HTTP request method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
}

impl Method {
    /// The RFC 9110 token (`"GET"`, …).
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
            Method::Patch => "PATCH",
            Method::Head => "HEAD",
        }
    }
}

/// A request under construction. Build with [`Request::get`] (and friends), then [`fetch`].
#[derive(Clone, Debug)]
pub struct Request {
    pub(crate) method: Method,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Option<Vec<u8>>,
    pub(crate) timeout: Duration,
    pub(crate) allow_expensive: bool,
    pub(crate) allow_constrained: bool,
}

impl Request {
    fn new(method: Method, url: impl Into<String>) -> Request {
        Request {
            method,
            url: url.into(),
            headers: Vec::new(),
            body: None,
            timeout: Duration::from_secs(30),
            allow_expensive: true,
            allow_constrained: true,
        }
    }

    pub fn get(url: impl Into<String>) -> Request {
        Self::new(Method::Get, url)
    }
    pub fn post(url: impl Into<String>, body: Vec<u8>) -> Request {
        Self::new(Method::Post, url).body(body)
    }
    pub fn put(url: impl Into<String>, body: Vec<u8>) -> Request {
        Self::new(Method::Put, url).body(body)
    }
    pub fn delete(url: impl Into<String>) -> Request {
        Self::new(Method::Delete, url)
    }
    pub fn patch(url: impl Into<String>, body: Vec<u8>) -> Request {
        Self::new(Method::Patch, url).body(body)
    }
    pub fn head(url: impl Into<String>) -> Request {
        Self::new(Method::Head, url)
    }

    /// Append a request header (duplicates allowed, sent in order).
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// The request body (also settable via the [`Request::post`]/[`Request::put`]/[`Request::patch`]
    /// constructors).
    pub fn body(mut self, bytes: Vec<u8>) -> Self {
        self.body = Some(bytes);
        self
    }

    /// How long the request may sit without progress. Default **30 s**. This bounds connecting,
    /// awaiting the response head, and idle gaps in the body — NOT the total transfer time, so a
    /// long download that keeps moving is never cut off (per-platform mapping: docs/http.md).
    pub fn timeout(mut self, d: Duration) -> Self {
        self.timeout = d;
        self
    }

    /// Whether the request may use "expensive" paths (cellular / personal hotspot). Default
    /// `true`. Native on Apple (`allowsExpensiveNetworkAccess`); advisory elsewhere — combine
    /// with `day_part_network::status().expensive` for app-side policy.
    pub fn allow_expensive(mut self, allowed: bool) -> Self {
        self.allow_expensive = allowed;
        self
    }

    /// Whether the request may run under Low Data Mode. Default `true`. Native on Apple
    /// (`allowsConstrainedNetworkAccess`); advisory elsewhere.
    pub fn allow_constrained(mut self, allowed: bool) -> Self {
        self.allow_constrained = allowed;
        self
    }
}

/// A complete HTTP response, body buffered in memory. For large downloads use [`fetch_to_file`],
/// which streams to disk instead.
#[derive(Clone, Debug)]
pub struct Response {
    /// The HTTP status code. **4xx/5xx are delivered here, not as [`HttpError`]** — only
    /// transport-level failures error.
    pub status: u16,
    /// Response headers in arrival order (duplicates preserved).
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    /// The body as (lossily-decoded) UTF-8 text.
    pub fn text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.body)
    }

    /// The first header with this name (ASCII case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// The result of a [`fetch_to_file`] download (the body went to disk, not memory).
#[derive(Clone, Debug)]
pub struct Download {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub bytes_written: u64,
}

/// A transport-level failure. HTTP error STATUSES (4xx/5xx) are not here — they arrive as
/// [`Response::status`]. The portable core maps from each platform's taxonomy (docs/http.md).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpError {
    /// The URL failed to parse (or uses an unsupported scheme).
    BadUrl(String),
    /// The request exceeded [`Request::timeout`].
    Timeout,
    /// Host name resolution failed.
    Dns,
    /// The connection could not be established (refused, unreachable, reset mid-handshake).
    Connect,
    /// TLS handshake / certificate failure.
    Tls(String),
    /// Everything else the platform reported (message passed through).
    Io(String),
    /// The request was cancelled — its [`FetchFuture`] dropped, or a platform-side cancel.
    Cancelled,
    /// No HTTP capability on this platform ([`Tier::Unavailable`]).
    Unsupported,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::BadUrl(u) => write!(f, "bad url: {u}"),
            HttpError::Timeout => write!(f, "request timed out"),
            HttpError::Dns => write!(f, "host name resolution failed"),
            HttpError::Connect => write!(f, "connection failed"),
            HttpError::Tls(m) => write!(f, "TLS failure: {m}"),
            HttpError::Io(m) => write!(f, "{m}"),
            HttpError::Cancelled => write!(f, "request cancelled"),
            HttpError::Unsupported => write!(f, "no HTTP capability on this platform"),
        }
    }
}

impl std::error::Error for HttpError {}

/// How requests are realized on the compiled target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// The platform's own networking stack (URLSession / HttpURLConnection / WinHTTP): system
    /// proxy + PAC, VPN routing, platform TLS + certificate stores.
    NativeStack,
    /// The bundled Rust client (ureq + rustls): correct HTTP(S), but system awareness is limited
    /// to `http_proxy`/`https_proxy`/`no_proxy` environment variables.
    RustFallback,
    /// No HTTP capability — every call returns [`HttpError::Unsupported`].
    Unavailable,
}

impl Tier {
    /// A short display label (`"native"` / `"fallback"` / `"unavailable"`).
    pub fn label(self) -> &'static str {
        match self {
            Tier::NativeStack => "native",
            Tier::RustFallback => "fallback",
            Tier::Unavailable => "unavailable",
        }
    }
}

/// How this target realizes requests (fixed at compile time).
pub fn tier() -> Tier {
    imp::TIER
}

/// Perform the request, BLOCKING the calling thread until the response (or [`Request::timeout`]).
/// Run it on your own thread — calling this on the UI thread stalls the app (docs/http.md).
pub fn fetch(req: &Request) -> Result<Response, HttpError> {
    imp::fetch(req)
}

/// Perform the request without blocking; `on_done` runs on an unspecified BACKGROUND thread
/// (capture a reactive `Setter` to deliver into UI state — see the crate docs).
pub fn fetch_async(
    req: Request,
    on_done: impl FnOnce(Result<Response, HttpError>) + Send + 'static,
) {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        // Natively async: the NSURLSession completion handler invokes `on_done` on the session's
        // delegate queue — no extra thread.
        imp::fetch_async(req, Box::new(on_done));
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    {
        std::thread::spawn(move || on_done(imp::fetch(&req)));
    }
}

/// Download the response body straight to `dest` (create/truncate), never buffering it in memory.
/// Blocking, like [`fetch`]. On error a partial `dest` is removed best-effort; atomicity is not
/// promised.
pub fn fetch_to_file(req: &Request, dest: &Path) -> Result<Download, HttpError> {
    let out = imp::fetch_to_file(req, dest);
    if out.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    out
}

/// Start the request immediately and await the result. The future is the cancellation grip:
/// **dropping it cancels the request** where the platform supports it (docs/http.md's cancel
/// matrix) — Apple `NSURLSessionTask.cancel`, Android OkHttp `Call.cancel`; Windows and the
/// Rust fallback run the request to completion on their worker thread and discard the result.
/// A cancelled request that still completes resolves nothing; a platform-side cancel that beats
/// the drop surfaces as [`HttpError::Cancelled`].
///
/// Await it inside `day::task` (or any executor — the future is `Send`-agnostic plumbing over
/// [`fetch_async`]'s completion): `day::task(async move { let r = fetch_future(req).await; … })`.
pub fn fetch_future(req: Request) -> FetchFuture {
    let shared = Arc::new(Mutex::new(FutureState {
        result: None,
        waker: None,
        cancelled: false,
    }));
    let cancel = start_future(req, shared.clone());
    FetchFuture {
        shared,
        cancel,
        done: false,
    }
}

/// Shared state between a [`FetchFuture`] and its completion callback.
///
/// Locking protocol (the mutex is a LEAF lock — no platform or user code runs under it):
/// - `poll` checks `result` and stores the waker under ONE lock acquisition, closing the
///   lost-wakeup race (a completion between a check and a separate store would be missed).
/// - the completion stores `result`, takes the waker, UNLOCKS, then wakes — an inline waker
///   (tests) re-polls synchronously, which re-takes the lock.
/// - `Drop` sets `cancelled`, clears the waker, UNLOCKS, then runs the platform cancel —
///   Apple's `task.cancel()` can schedule the completion synchronously on the delegate queue,
///   which takes this lock. Nobody wakes on cancel: Drop is terminal (the future can never be
///   polled again); a late completion finds no waker and its stored result is never read.
struct FutureState {
    result: Option<Result<Response, HttpError>>,
    waker: Option<std::task::Waker>,
    cancelled: bool,
}

/// An in-flight [`fetch_future`] request; dropping it cancels (see the cancel matrix).
pub struct FetchFuture {
    shared: Arc<Mutex<FutureState>>,
    cancel: Option<Box<dyn FnOnce() + Send>>,
    done: bool,
}

/// Lock, riding out poisoning: a panic while holding the lock leaves plain data (no broken
/// invariants), so the poisoned value is still the truth.
fn lock(m: &Mutex<FutureState>) -> std::sync::MutexGuard<'_, FutureState> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl Future for FetchFuture {
    type Output = Result<Response, HttpError>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let mut st = lock(&self.shared);
        if let Some(r) = st.result.take() {
            drop(st);
            self.done = true;
            return std::task::Poll::Ready(r);
        }
        st.waker = Some(cx.waker().clone());
        std::task::Poll::Pending
    }
}

impl Drop for FetchFuture {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        {
            let mut st = lock(&self.shared);
            st.cancelled = true;
            st.waker = None;
        }
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}

/// Store a completion result and wake the future (see [`FutureState`]'s locking protocol).
fn deliver_future(shared: &Arc<Mutex<FutureState>>, result: Result<Response, HttpError>) {
    let waker = {
        let mut st = lock(shared);
        st.result = Some(result);
        st.waker.take()
    };
    if let Some(w) = waker {
        w.wake();
    }
}

/// Start the platform request for [`fetch_future`]; returns the platform cancel closure
/// (`None` on the discard tiers — Windows, the Rust fallback, and Android until its
/// cancel-token rail below).
fn start_future(req: Request, shared: Arc<Mutex<FutureState>>) -> Option<Box<dyn FnOnce() + Send>> {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        imp::fetch_async_cancellable(req, Box::new(move |result| deliver_future(&shared, result)))
    }
    #[cfg(target_os = "android")]
    {
        // The blocking Java call runs on a worker thread, registered under an OkHttp cancel
        // token; the cancel closure fires `Call.cancel()` from whichever thread drops the
        // future (the call then returns the `-7` sentinel). A drop that beats the worker's
        // registration degrades to a discard — the cancelled-flag check below catches the
        // drop-before-start case for free (docs/http.md's cancel matrix).
        let token = imp::next_token();
        std::thread::spawn(move || {
            if lock(&shared).cancelled {
                return;
            }
            let result = imp::fetch_with_token(&req, token);
            deliver_future(&shared, result);
        });
        Some(Box::new(move || imp::cancel(token)))
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "android")))]
    {
        // Windows / fallback / unavailable: blocking half on a worker thread; a drop before
        // the request started is a free discard, a drop mid-flight lets the request run out
        // and discards the result (no platform cancel on these tiers — docs/http.md).
        std::thread::spawn(move || {
            if lock(&shared).cancelled {
                return;
            }
            let result = imp::fetch(&req);
            deliver_future(&shared, result);
        });
        None
    }
}

/// [`fetch_to_file`] without blocking; `on_done` runs on an unspecified background thread.
pub fn fetch_to_file_async(
    req: Request,
    dest: PathBuf,
    on_done: impl FnOnce(Result<Download, HttpError>) + Send + 'static,
) {
    std::thread::spawn(move || on_done(fetch_to_file(&req, &dest)));
}

/// Receives a streamed response: the head first, then each body chunk as it arrives. Implement
/// this for progress reporting, cancellation, incremental hashing — anything that must observe a
/// large body without buffering it (an app store streaming an APK to disk, docs/http.md).
pub trait StreamSink {
    /// The status + headers, before any body. Return `false` to abort the transfer (e.g. an
    /// unexpected status for a `Range` resume) — [`fetch_streamed`] then returns
    /// [`HttpError::Io`]`("aborted")`.
    fn head(&mut self, _status: u16, _headers: &[(String, String)]) -> bool {
        true
    }
    /// One body chunk, in arrival order. Returning `Err` aborts the transfer and becomes
    /// [`fetch_streamed`]'s result (return `Io("cancelled")` for user cancellation).
    fn chunk(&mut self, data: &[u8]) -> Result<(), HttpError>;
}

/// Perform the request, streaming the body into `sink` chunk by chunk — nothing is buffered
/// beyond one chunk. Blocking, like [`fetch`]. `bytes_written` counts the bytes handed to the
/// sink.
pub fn fetch_streamed(req: &Request, sink: &mut dyn StreamSink) -> Result<Download, HttpError> {
    imp::fetch_streamed(req, sink)
}

// ---------------------------------------------------------------------------
// Per-OS implementations. Each exposes `TIER`, `fn fetch(&Request) -> Result<Response, HttpError>`
// and `fn fetch_to_file(&Request, &Path) -> Result<Download, HttpError>`; Apple additionally
// exposes the natively-async `fetch_async`.
// ---------------------------------------------------------------------------

// macOS + iOS share one NSURLSession impl.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[path = "apple.rs"]
mod imp;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;

// Desktop Linux AND HarmonyOS (also `target_os = "linux"`) use the Rust fallback: Linux has no
// OS-level HTTP service, and the OSS OpenHarmony NDK has no HTTP C API (only websocket).
#[cfg(target_os = "linux")]
#[path = "fallback.rs"]
mod imp;

#[cfg(target_os = "android")]
#[path = "android.rs"]
mod imp;

// Any other platform: no HTTP capability.
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "android"
)))]
mod imp {
    use super::{Download, HttpError, Request, Response, StreamSink, Tier};
    use std::path::Path;

    pub const TIER: Tier = Tier::Unavailable;
    pub fn fetch(_req: &Request) -> Result<Response, HttpError> {
        Err(HttpError::Unsupported)
    }
    pub fn fetch_to_file(_req: &Request, _dest: &Path) -> Result<Download, HttpError> {
        Err(HttpError::Unsupported)
    }
    pub fn fetch_streamed(
        _req: &Request,
        _sink: &mut dyn StreamSink,
    ) -> Result<Download, HttpError> {
        Err(HttpError::Unsupported)
    }
}
