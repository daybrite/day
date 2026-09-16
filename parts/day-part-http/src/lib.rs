// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-part-http — HEADLESS cross-platform HTTP(S) through each platform's NATIVE networking
//! stack (docs/http.md). No UI; any Rust code can depend on this crate.
//!
//! ```no_run
//! let resp = day_part_http::fetch(&day_part_http::Request::get("https://example.com"))?;
//! println!("{} {} bytes", resp.status, resp.body.len());
//! # Ok::<(), day_part_http::HttpError>(())
//! ```
//!
//! Why native stacks instead of a Rust HTTP crate: the request inherits the SYSTEM configuration —
//! proxies + PAC, per-network VPN routing, Low Data Mode ([`Request::allow_constrained`]),
//! enterprise/MDM certificate stores — and the binary carries no TLS library of its own. macOS
//! and iOS use URLSession, Android OkHttp through a Java bridge arm, Windows WinHTTP, Linux the
//! system's libcurl (opened at run time), HarmonyOS the Network Kit through an ArkTS arm, and the
//! web the browser's `fetch` and `WebSocket` through a JavaScript arm, where only the asynchronous
//! entry points exist and the blocking calls return [`HttpError::Unsupported`]. [`tier`] reports
//! whether a stack is present.
//!
//! **Two doors.** The functions at the crate root ([`fetch`], [`fetch_async`], [`fetch_future`],
//! [`fetch_streamed`]) send one request with no state kept between requests. A [`Client`] adds
//! the rest of a modern HTTP surface on the same stacks: bodies streamed in chunks with
//! backpressure, uploads from files, streams and multipart forms, redirect and authentication
//! callbacks, public-key pins and server trust decisions, client identities, cookies, caches,
//! transfer metrics and WebSockets. [`capabilities`] reports what the platform offers.
//!
//! **Threading.** [`fetch`] BLOCKS the calling thread — run it on your own thread, never the UI
//! thread. [`fetch_async`]'s completion runs on the transport's own thread (URLSession's delegate
//! queue, OkHttp's reader pool, the libcurl driver, WinHTTP's callbacks, the HarmonyOS JS thread,
//! or the browser's only thread); deliver results into the UI by capturing
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
//! Or await it: [`fetch_future`] wraps the same completion as a `Future` whose DROP cancels the
//! request where the platform can (docs/http.md's cancel matrix) — under `day::task` the
//! continuation runs on the UI thread, so results are plain signal writes (docs/async.md).
//!
//! Every option is **best-effort per platform** (docs/http.md has the matrix):
//! `allow_expensive`/`allow_constrained` are native on Apple and advisory elsewhere; HTTP 4xx/5xx
//! are `Ok` responses (check [`Response::status`]), not errors.

use std::borrow::Cow;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod client;

#[cfg(not(target_arch = "wasm32"))]
pub mod testing;

pub use client::{
    Body, Cache, CachePolicy, Capabilities, Challenge, ChallengeReply, Client, ClientBuilder,
    Collecting, Connecting, Cookie, CookieJar, Cookies, Fetching, Form, Head, Hop, Identity,
    InFlight, Message, Metrics, NextChunk, NextMessage, RedirectReply, Redirects, Scheme, Sending,
    ServerTrust, Streaming, Trust, TrustReply, WebSocket, WsSender,
};
pub(crate) use client::{Payload, ProgressFn};

/// The contract a backend implements (docs/http.md "Transports"). Apps reach for it to install
/// a test double with [`ClientBuilder::transport`], or to route a client through their own stack.
pub mod transport {
    pub use crate::client::{
        Answer, AuthQuestion, Completion, Event, Events, Prepared, PreparedBody, Question,
        QuestionId, SharedReader, Socket, Transfer, Transport, TransportConfig, WsEvent, WsEvents,
    };
}

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

    /// The method a token names, ASCII case-insensitive.
    pub fn parse(token: &str) -> Option<Method> {
        [
            Method::Get,
            Method::Post,
            Method::Put,
            Method::Delete,
            Method::Patch,
            Method::Head,
        ]
        .into_iter()
        .find(|m| m.as_str().eq_ignore_ascii_case(token))
    }
}

/// A request under construction. Build with [`Request::get`] (and friends), then [`fetch`] or a
/// [`Client`].
#[derive(Clone, Debug)]
pub struct Request {
    pub(crate) method: Method,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) payload: Payload,
    pub(crate) timeout: Option<Duration>,
    pub(crate) timeout_total: Option<Duration>,
    pub(crate) allow_expensive: bool,
    pub(crate) allow_constrained: bool,
    pub(crate) cache: CachePolicy,
    pub(crate) priority: Option<f32>,
    pub(crate) protocols: Vec<String>,
    pub(crate) upload_progress: Option<ProgressFn>,
}

impl Request {
    /// A request with this method.
    pub fn new(method: Method, url: impl Into<String>) -> Request {
        Request {
            method,
            url: url.into(),
            headers: Vec::new(),
            payload: Payload::Empty,
            timeout: None,
            timeout_total: None,
            allow_expensive: true,
            allow_constrained: true,
            cache: CachePolicy::Default,
            priority: None,
            protocols: Vec::new(),
            upload_progress: None,
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

    /// The method.
    pub fn method(&self) -> Method {
        self.method
    }

    /// The URL, as given.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The request headers, in the order they were added.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// Append a request header (duplicates allowed, sent in order).
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// `Authorization: Bearer <token>`, sent up front.
    pub fn bearer(self, token: &str) -> Self {
        self.header("Authorization", &format!("Bearer {token}"))
    }

    /// `Authorization: Basic …`, sent up front instead of waiting for a challenge.
    pub fn basic_auth(self, user: &str, password: &str) -> Self {
        let value = client::basic_authorization(user, password);
        self.header("Authorization", &value)
    }

    /// The request body (also settable via the [`Request::post`]/[`Request::put`]/[`Request::patch`]
    /// constructors).
    pub fn body(mut self, bytes: Vec<u8>) -> Self {
        self.payload = Payload::Bytes(Arc::new(bytes));
        self
    }

    /// Send a file as the body, read while it is sent. A [`Client`] only.
    pub fn body_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.payload = Payload::File(path.into());
        self
    }

    /// Send what `reader` yields as the body, read while it is sent. `len` becomes
    /// `Content-Length`; without it the body goes out chunked where the platform allows. A
    /// stream can be sent once, so a redirect or challenge that needs the body again fails the
    /// request. A [`Client`] only.
    pub fn body_stream(
        mut self,
        reader: impl std::io::Read + Send + 'static,
        len: Option<u64>,
    ) -> Self {
        self.payload = Payload::Stream(transport::SharedReader::new(reader), len);
        self
    }

    /// Send a `multipart/form-data` body; its `Content-Type` is set unless the request sets one.
    /// A [`Client`] only.
    pub fn form(mut self, form: Form) -> Self {
        self.payload = Payload::Form(form);
        self
    }

    /// How long the request may sit without progress. Default **30 s**. This bounds connecting,
    /// awaiting the response head, and idle gaps in the body — not the total transfer time, so a
    /// long download that keeps moving is never cut off (per-platform mapping: docs/http.md).
    pub fn timeout(mut self, d: Duration) -> Self {
        self.timeout = Some(d);
        self
    }

    /// How long the whole request may take, redirects, challenges and body included. A
    /// [`Client`] only; it overrides [`ClientBuilder::timeout_total`].
    pub fn timeout_total(mut self, d: Duration) -> Self {
        self.timeout_total = Some(d);
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

    /// How the request may use the client's cache. Default [`CachePolicy::Default`].
    pub fn cache(mut self, policy: CachePolicy) -> Self {
        self.cache = policy;
        self
    }

    /// A scheduling hint from 0.0 (lowest) to 1.0 (highest), where the platform takes one.
    pub fn priority(mut self, priority: f32) -> Self {
        self.priority = Some(priority.clamp(0.0, 1.0));
        self
    }

    /// WebSocket subprotocols to offer, in preference order.
    pub fn protocols<S: Into<String>>(mut self, protocols: impl IntoIterator<Item = S>) -> Self {
        self.protocols = protocols.into_iter().map(Into::into).collect();
        self
    }

    /// Observe the body going out: `(bytes sent, total)`, on the thread the platform reports on.
    /// Needs [`Capabilities::upload_progress`]. A [`Client`] only.
    pub fn upload_progress(
        mut self,
        observe: impl Fn(u64, Option<u64>) + Send + Sync + 'static,
    ) -> Self {
        self.upload_progress = Some(ProgressFn(Arc::new(observe)));
        self
    }
}

/// A complete HTTP response, body buffered in memory. For large downloads use [`fetch_to_file`],
/// which streams to disk instead, or read a [`Client`]'s [`Body`] in chunks.
#[derive(Clone, Debug)]
pub struct Response {
    /// The HTTP status code. **4xx/5xx are delivered here, not as [`HttpError`]** — only
    /// transport-level failures error.
    pub status: u16,
    /// Response headers in arrival order (duplicates preserved).
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// The URL that answered, after redirects; empty where the platform does not report it.
    pub url: String,
    /// Transfer metrics, where the platform reports them.
    pub metrics: Option<Metrics>,
}

impl Response {
    /// A response from its parts, with no URL and no metrics.
    pub fn new(status: u16, headers: Vec<(String, String)>, body: Vec<u8>) -> Response {
        Response {
            status,
            headers,
            body,
            url: String::new(),
            metrics: None,
        }
    }

    pub(crate) fn assemble(head: Head, body: Vec<u8>, metrics: Option<Metrics>) -> Response {
        Response {
            status: head.status,
            headers: head.headers,
            body,
            url: head.url,
            metrics,
        }
    }

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
    /// The request exceeded [`Request::timeout`] or its total limit.
    Timeout,
    /// Host name resolution failed.
    Dns,
    /// The connection could not be established (refused, unreachable, reset mid-handshake).
    Connect,
    /// TLS handshake / certificate failure, including a server a pin or a trust handler refused.
    Tls(String),
    /// Everything else the platform reported (message passed through).
    Io(String),
    /// The request was cancelled — its [`FetchFuture`] dropped, or a platform-side cancel.
    Cancelled,
    /// No HTTP capability on this platform ([`Tier::Unavailable`]), or an entry point or option
    /// that cannot exist on it (the blocking calls on the web's single thread — docs/http.md).
    Unsupported,
    /// More redirects than [`Redirects::Follow`] allows.
    TooManyRedirects,
    /// A status the caller could not accept, such as a download manager's unexpected `416`.
    Status(u16),
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
            HttpError::Unsupported => write!(f, "not supported on this platform"),
            HttpError::TooManyRedirects => write!(f, "too many redirects"),
            HttpError::Status(s) => write!(f, "unexpected status {s}"),
        }
    }
}

impl std::error::Error for HttpError {}

/// How requests are realized on the compiled target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// The platform's own networking stack (URLSession, OkHttp, WinHTTP, libcurl, the Network
    /// Kit, the browser's fetch): system proxy + PAC, VPN routing, platform TLS + certificate
    /// stores. On the web this tier is async-only — the blocking entry points return
    /// [`HttpError::Unsupported`] (docs/http.md).
    NativeStack,
    /// No HTTP capability (no libcurl on Linux, a HarmonyOS build without its staged arm, an
    /// unknown target) — every call returns [`HttpError::Unsupported`].
    Unavailable,
}

impl Tier {
    /// A short display label (`"native"` / `"unavailable"`).
    pub fn label(self) -> &'static str {
        match self {
            Tier::NativeStack => "native",
            Tier::Unavailable => "unavailable",
        }
    }
}

/// How this target realizes requests (fixed at compile time).
pub fn tier() -> Tier {
    // libcurl loads at run time, so Linux knows its tier only then.
    #[cfg(all(target_os = "linux", not(target_env = "ohos")))]
    {
        imp::tier()
    }
    #[cfg(not(all(target_os = "linux", not(target_env = "ohos"))))]
    {
        imp::TIER
    }
}

/// What a [`Client`] on this platform can do.
pub fn capabilities() -> Capabilities {
    platform_capabilities()
}

/// The platform transport's capabilities, before any client exists.
pub(crate) fn platform_capabilities() -> Capabilities {
    imp::capabilities()
}

/// The platform transport, configured for one client.
pub(crate) fn platform_transport(
    config: &transport::TransportConfig,
) -> Arc<dyn transport::Transport> {
    imp::transport(config)
}

/// The client the crate-root functions share: no cookies and no cache, so each call stands alone
/// as it always has.
fn stateless() -> &'static Client {
    static CLIENT: std::sync::OnceLock<Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .cookies(Cookies::Off)
            .cache(Cache::Off)
            .build()
    })
}

/// Stream `req` through `client` into `sink`, waiting on this thread.
#[cfg(not(target_arch = "wasm32"))]
fn streamed_through(
    client: &Client,
    req: &Request,
    sink: &mut dyn StreamSink,
) -> Result<Download, HttpError> {
    // On HarmonyOS the JS thread is the loop that delivers the answer: waiting there would starve it.
    #[cfg(all(target_os = "linux", target_env = "ohos"))]
    if day_bridge::arkts::on_js_thread() {
        return Err(HttpError::Unsupported);
    }
    let streaming = client::wait(client.send_future(req.clone()))?;
    let (status, headers) = (streaming.status(), streaming.headers().to_vec());
    if !sink.head(status, &headers) {
        return Err(HttpError::Io("aborted".into()));
    }
    let mut body = streaming.into_body();
    let mut bytes_written = 0u64;
    while let Some(chunk) = body.next_blocking() {
        let chunk = chunk?;
        sink.chunk(&chunk)?;
        bytes_written += chunk.len() as u64;
    }
    Ok(Download {
        status,
        headers,
        bytes_written,
    })
}

/// Perform the request, BLOCKING the calling thread until the response (or [`Request::timeout`]).
/// Run it on your own thread — calling this on the UI thread stalls the app (docs/http.md).
/// On the web (web-dom) blocking is impossible on the single browser thread: this returns
/// [`HttpError::Unsupported`] there — use [`fetch_async`] or [`fetch_future`].
pub fn fetch(req: &Request) -> Result<Response, HttpError> {
    // The browser has one thread and no blocking waits.
    #[cfg(target_arch = "wasm32")]
    {
        let _ = req;
        Err(HttpError::Unsupported)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // On HarmonyOS the JS thread is the loop that delivers the answer: waiting there would
        // starve it.
        #[cfg(all(target_os = "linux", target_env = "ohos"))]
        if day_bridge::arkts::on_js_thread() {
            return Err(HttpError::Unsupported);
        }
        client::wait(stateless().fetch_future(req.clone()))
    }
}

/// Perform the request without blocking; `on_done` runs on an unspecified BACKGROUND thread
/// (capture a reactive `Setter` to deliver into UI state — see the crate docs). On the web
/// the completion runs on the browser thread — the only one — which the Setter idiom absorbs
/// unchanged.
pub fn fetch_async(
    req: Request,
    on_done: impl FnOnce(Result<Response, HttpError>) + Send + 'static,
) {
    let _ = start_cancellable(req, Box::new(on_done));
}

/// Download the response body straight to `dest` (create/truncate), never buffering it in memory.
/// Blocking, like [`fetch`]. On error a partial `dest` is removed best-effort; atomicity is not
/// promised.
pub fn fetch_to_file(req: &Request, dest: &Path) -> Result<Download, HttpError> {
    // The browser has one thread and no filesystem for this.
    #[cfg(target_arch = "wasm32")]
    let out = {
        let _ = req;
        Err(HttpError::Unsupported)
    };
    #[cfg(not(target_arch = "wasm32"))]
    let out = {
        struct FileSink(std::fs::File);
        impl StreamSink for FileSink {
            fn chunk(&mut self, data: &[u8]) -> Result<(), HttpError> {
                use std::io::Write;
                self.0
                    .write_all(data)
                    .map_err(|e| HttpError::Io(e.to_string()))
            }
        }
        std::fs::File::create(dest)
            .map_err(|e| HttpError::Io(e.to_string()))
            .and_then(|file| streamed_through(stateless(), req, &mut FileSink(file)))
    };
    if out.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    out
}

/// Start the request immediately and await the result. The future is the cancellation grip:
/// **dropping it cancels the request** where the platform supports it (docs/http.md's cancel
/// matrix) — Apple `NSURLSessionTask.cancel`, Android OkHttp `Call.cancel`, the web the
/// fetch's `AbortController`; Windows closes the WinHTTP
/// request handle.
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
    let deliver = shared.clone();
    let cancel = start_cancellable(
        req,
        Box::new(move |result| deliver_future(&deliver, result)),
    );
    FetchFuture {
        shared,
        cancel,
        done: false,
    }
}

/// Shared state between a [`FetchFuture`] and its completion callback.
///
/// Locking protocol (the mutex is a LEAF lock — no platform or user code runs under it):
/// - `poll` checks `result` and stores the waker under one lock acquisition, closing the
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
        if st.cancelled {
            return;
        }
        st.result = Some(result);
        st.waker.take()
    };
    if let Some(w) = waker {
        w.wake();
    }
}

type Completion = Box<dyn FnOnce(Result<Response, HttpError>) + Send>;
type CancelFn = Box<dyn FnOnce() + Send>;

/// Start the request through the shared stateless client; returns the closure that cancels it. The
/// completion runs on the thread the platform reports on.
fn start_cancellable(req: Request, on_done: Completion) -> Option<CancelFn> {
    Some(stateless().fetch_cancellable(req, on_done))
}

/// [`fetch_to_file`] without blocking; `on_done` runs on an unspecified background thread.
pub fn fetch_to_file_async(
    req: Request,
    dest: PathBuf,
    on_done: impl FnOnce(Result<Download, HttpError>) + Send + 'static,
) {
    #[cfg(target_arch = "wasm32")]
    {
        // No filesystem in the browser sandbox: the Unsupported answer is immediate, so
        // complete inline (wasm32 has no threads to defer to).
        on_done(fetch_to_file(&req, &dest));
    }
    #[cfg(not(target_arch = "wasm32"))]
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
    // The browser has one thread and no blocking waits.
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (req, sink);
        Err(HttpError::Unsupported)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        streamed_through(stateless(), req, sink)
    }
}

// ---------------------------------------------------------------------------
// Per-OS implementations. Each exposes `TIER` (Linux: `tier()`), `capabilities()` and
// `transport()`, the platform transport beneath every `Client`.
// ---------------------------------------------------------------------------

// The bridged arms (docs/bridge.md "Streams"): Android, HarmonyOS and the web. The declarations and
// their fallback compile everywhere; only those targets reach an arm.
mod bridge;

// macOS + iOS share one NSURLSession impl.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[path = "apple.rs"]
mod imp;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;

// Desktop Linux: the system's libcurl, loaded at run time.
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
#[path = "linux.rs"]
mod imp;

// The same libcurl transport, tested on macOS against the system's libcurl.
#[cfg(all(test, target_os = "macos"))]
#[path = "linux.rs"]
#[allow(dead_code)]
mod curl;

// Android, HarmonyOS and the web: the bridged arms over one frame protocol (src/bridge.rs).
#[cfg(any(
    target_os = "android",
    all(target_os = "linux", target_env = "ohos"),
    target_arch = "wasm32"
))]
#[path = "bridged.rs"]
mod imp;

// Any other platform: no HTTP capability.
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "android",
    target_arch = "wasm32"
)))]
mod imp {
    use std::sync::Arc;

    use super::{HttpError, Tier};
    use crate::client::{
        Answer, Capabilities, Event, Events, Prepared, QuestionId, Transfer, Transport,
        TransportConfig,
    };

    pub const TIER: Tier = Tier::Unavailable;

    pub(crate) fn capabilities() -> Capabilities {
        Capabilities::default()
    }

    pub(crate) fn transport(_config: &TransportConfig) -> Arc<dyn Transport> {
        Arc::new(Unavailable)
    }

    /// Every exchange fails with `Unsupported`.
    struct Unavailable;

    impl Transport for Unavailable {
        fn capabilities(&self) -> Capabilities {
            capabilities()
        }

        fn start(&self, _request: Prepared, events: Events) -> Arc<dyn Transfer> {
            events(Event::Failed(HttpError::Unsupported));
            Arc::new(Unavailable)
        }
    }

    impl Transfer for Unavailable {
        fn demand(&self, _chunks: u32) {}
        fn answer(&self, _id: QuestionId, _answer: Answer) {}
        fn cancel(&self) {}
    }
}
