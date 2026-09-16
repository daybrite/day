// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The portable client (docs/http.md "Client").
//!
//! A [`Client`] owns the policy that must behave the same on every backend: following redirects,
//! answering 401 and 407 challenges, the cookie jar, public-key pins, multipart bodies, total-time
//! limits, and backpressure on response bodies. Beneath it a [`Transport`] performs one exchange
//! at a time through the platform's own stack, which keeps TLS, trust stores, proxies, HTTP/2 and
//! 3, and NTLM with the OS.
//!
//! Every asynchronous operation has a callback form (`*_async`) and a future form (`*_future`),
//! per docs/async.md rule 3. Callbacks run on the thread the platform reports on; dropping a
//! future, a [`Body`] or a [`WebSocket`] cancels what it stands for.

use std::collections::VecDeque;
use std::future::Future;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use day_async::TimerId;
use sha2::{Digest, Sha256};

use crate::{HttpError, Method, Request, Response};

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Body chunks (or WebSocket messages) a reader keeps in hand, queued plus granted, before it
/// stops asking the transport for more. A slow reader therefore slows the socket.
const WINDOW: u32 = 4;
/// The idle bound when neither the request nor the client sets one.
pub(crate) const DEFAULT_IDLE: Duration = Duration::from_secs(30);
const DEFAULT_QUESTION_TIMEOUT: Duration = Duration::from_secs(120);
/// Credentials offered to one request before its 401 is delivered as the response.
const MAX_AUTH_ATTEMPTS: u32 = 3;

/// Lock, riding out poisoning: every state here is plain data whose invariants a panicking
/// holder cannot break halfway.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// A request body, as the builder recorded it.
#[derive(Clone, Debug, Default)]
pub(crate) enum Payload {
    #[default]
    Empty,
    Bytes(Arc<Vec<u8>>),
    File(PathBuf),
    Stream(SharedReader, Option<u64>),
    Form(Form),
}

/// An upload progress observer: `(bytes sent, total)`.
#[derive(Clone)]
pub(crate) struct ProgressFn(pub(crate) Arc<dyn Fn(u64, Option<u64>) + Send + Sync>);

impl std::fmt::Debug for ProgressFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProgressFn")
    }
}

/// Whether a client follows redirects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Redirects {
    /// Follow at most this many, then fail with [`HttpError::TooManyRedirects`].
    Follow(u32),
    /// Deliver each redirect response as the response.
    Never,
}

impl Default for Redirects {
    fn default() -> Self {
        Redirects::Follow(10)
    }
}

/// A redirect the client is about to follow, as [`ClientBuilder::on_redirect`] sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hop {
    pub status: u16,
    pub from: String,
    pub to: String,
    /// The method the next request uses: a 303 turns anything but HEAD into GET, and a 301 or
    /// 302 turns a POST into a GET.
    pub method: String,
    /// Redirects already followed on the way here.
    pub followed: u32,
}

/// An HTTP authentication challenge, as [`ClientBuilder::on_challenge`] sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Challenge {
    pub url: String,
    pub host: String,
    pub port: u16,
    pub realm: Option<String>,
    pub scheme: Scheme,
    /// A proxy asked (407), not the origin server.
    pub proxy: bool,
    /// Credentials this request already offered that the server refused.
    pub previous_failures: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Resolution {
    Default,
    Follow,
    Stop,
    Credential { user: String, password: String },
    Bearer(String),
    Cancel,
    Accept,
    Reject,
}

/// The half of a question a handler answers. Dropping it unanswered takes the default.
struct ReplyGrip {
    flight: Weak<Flight>,
    ask: u64,
    answered: bool,
}

impl ReplyGrip {
    fn answer(mut self, resolution: Resolution) {
        self.answered = true;
        if let Some(flight) = self.flight.upgrade() {
            flight.resolve(self.ask, resolution);
        }
    }
}

impl Drop for ReplyGrip {
    fn drop(&mut self) {
        if let (false, Some(flight)) = (self.answered, self.flight.upgrade()) {
            flight.resolve(self.ask, Resolution::Default);
        }
    }
}

/// Answers a redirect question. Dropping it unanswered follows the redirect. It is `Send`, so a
/// handler may keep it and answer later from any thread; the exchange waits meanwhile, up to the
/// client's question timeout.
pub struct RedirectReply(ReplyGrip);

impl RedirectReply {
    /// Follow the redirect.
    pub fn follow(self) {
        self.0.answer(Resolution::Follow)
    }

    /// Do not follow: the redirect response is the response.
    pub fn stop(self) {
        self.0.answer(Resolution::Stop)
    }

    /// Abandon the request; it fails with [`HttpError::Cancelled`].
    pub fn cancel(self) {
        self.0.answer(Resolution::Cancel)
    }
}

impl std::fmt::Debug for RedirectReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RedirectReply")
    }
}

/// Answers a challenge. Dropping it unanswered delivers the 401 (or 407) as the response.
pub struct ChallengeReply(ReplyGrip);

impl ChallengeReply {
    /// Answer with a user name and password. The client writes Basic or Digest to match the
    /// challenge; a platform stack answering NTLM or Negotiate itself takes them as they are.
    pub fn credential(self, user: impl Into<String>, password: impl Into<String>) {
        self.0.answer(Resolution::Credential {
            user: user.into(),
            password: password.into(),
        })
    }

    /// Answer with a bearer token.
    pub fn bearer(self, token: impl Into<String>) {
        self.0.answer(Resolution::Bearer(token.into()))
    }

    /// Offer nothing: the challenge response is the response.
    pub fn default_handling(self) {
        self.0.answer(Resolution::Default)
    }

    /// Abandon the request; it fails with [`HttpError::Cancelled`].
    pub fn cancel(self) {
        self.0.answer(Resolution::Cancel)
    }
}

impl std::fmt::Debug for ChallengeReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ChallengeReply")
    }
}

/// Answers a server trust question. Dropping it unanswered keeps the platform's verdict.
pub struct TrustReply(ReplyGrip);

impl TrustReply {
    /// Trust the server, whatever the platform concluded.
    pub fn accept(self) {
        self.0.answer(Resolution::Accept)
    }

    /// Refuse the server; the request fails with [`HttpError::Tls`].
    pub fn reject(self) {
        self.0.answer(Resolution::Reject)
    }

    /// Keep the platform's verdict.
    pub fn default_handling(self) {
        self.0.answer(Resolution::Default)
    }
}

impl std::fmt::Debug for TrustReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TrustReply")
    }
}

type RedirectHandler = dyn Fn(&Hop, RedirectReply) + Send + Sync;
type ChallengeHandler = dyn Fn(&Challenge, ChallengeReply) + Send + Sync;
type TrustHandler = dyn Fn(&ServerTrust, TrustReply) + Send + Sync;

/// Which servers a client trusts beyond the platform's own evaluation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Trust {
    pins: Vec<(String, Option<[u8; 32]>)>,
}

impl Trust {
    /// The platform's evaluation alone.
    pub fn system() -> Trust {
        Trust::default()
    }

    /// Require `host`'s certificate chain to hold a key whose SHA-256 is `pin`, written
    /// `sha256/<base64>` as [`ServerTrust::pins`] prints it. `*.example.com` covers every
    /// subdomain. Several pins for one host accept any of them, which is how a key rotation
    /// ships. A malformed pin matches nothing, so its host fails closed.
    pub fn pin(mut self, host: impl Into<String>, pin: &str) -> Trust {
        self.pins
            .push((host.into().to_ascii_lowercase(), parse_pin(pin)));
        self
    }

    /// Whether any pin is configured.
    pub fn is_empty(&self) -> bool {
        self.pins.is_empty()
    }

    fn pins_for(&self, host: &str) -> Option<Vec<Option<[u8; 32]>>> {
        let host = host.to_ascii_lowercase();
        let pins: Vec<_> = self
            .pins
            .iter()
            .filter(|(pattern, _)| host_matches(pattern, &host))
            .map(|(_, pin)| *pin)
            .collect();
        (!pins.is_empty()).then_some(pins)
    }
}

fn host_matches(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(base) => {
            host.len() > base.len() + 1
                && host.ends_with(base)
                && host.as_bytes()[host.len() - base.len() - 1] == b'.'
        }
        None => pattern == host,
    }
}

/// A client certificate identity, presented when a server asks for one.
#[derive(Clone)]
pub struct Identity {
    pub(crate) pkcs12: Arc<Vec<u8>>,
    pub(crate) password: String,
}

impl Identity {
    /// A PKCS#12 archive (`.p12`, `.pfx`) holding the certificate and its private key.
    pub fn pkcs12(der: Vec<u8>, password: impl Into<String>) -> Identity {
        Identity {
            pkcs12: Arc::new(der),
            password: password.into(),
        }
    }
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("pkcs12_bytes", &self.pkcs12.len())
            .finish_non_exhaustive()
    }
}

/// Where a client keeps cookies.
#[derive(Clone, Debug)]
pub enum Cookies {
    /// The platform's store: `HTTPCookieStorage` on Apple, the browser's jar on the web. Where
    /// the platform offers none to share ([`Capabilities::platform_cookies`]), an in-memory
    /// [`CookieJar`].
    Platform,
    /// This jar, which the client reads and writes itself.
    Jar(Arc<CookieJar>),
    /// Send none and keep none.
    Off,
}

impl Cookies {
    /// A fresh in-memory jar.
    pub fn jar() -> Cookies {
        Cookies::Jar(Arc::new(CookieJar::new()))
    }

    /// A jar persisted to `file`: cookies with an expiry survive a relaunch.
    pub fn jar_at(file: impl Into<PathBuf>) -> Cookies {
        Cookies::Jar(Arc::new(CookieJar::open(file)))
    }
}

impl Default for Cookies {
    /// The platform store on Apple and the web, an in-memory jar elsewhere.
    fn default() -> Self {
        if cfg!(any(
            target_os = "macos",
            target_os = "ios",
            target_arch = "wasm32"
        )) {
            Cookies::Platform
        } else {
            Cookies::jar()
        }
    }
}

/// Whether a client keeps an HTTP cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cache {
    /// The platform's cache with these capacities in bytes, where it has one
    /// ([`Capabilities::platform_cache`]).
    Platform { memory: u64, disk: u64 },
    /// No cache.
    Off,
}

impl Cache {
    /// The platform cache at 8 MiB in memory and 64 MiB on disk.
    pub fn platform() -> Cache {
        Cache::Platform {
            memory: 8 << 20,
            disk: 64 << 20,
        }
    }
}

impl Default for Cache {
    fn default() -> Self {
        Cache::platform()
    }
}

/// Configures a [`Client`].
pub struct ClientBuilder {
    timeout_idle: Duration,
    timeout_total: Option<Duration>,
    redirects: Redirects,
    on_redirect: Option<Arc<RedirectHandler>>,
    on_challenge: Option<Arc<ChallengeHandler>>,
    on_server_trust: Option<Arc<TrustHandler>>,
    trust: Trust,
    identity: Option<Identity>,
    cookies: Cookies,
    cache: Cache,
    headers: Vec<(String, String)>,
    max_per_host: Option<u32>,
    wait_for_connectivity: bool,
    question_timeout: Duration,
    transport: Option<Arc<dyn Transport>>,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        ClientBuilder {
            timeout_idle: DEFAULT_IDLE,
            timeout_total: None,
            redirects: Redirects::default(),
            on_redirect: None,
            on_challenge: None,
            on_server_trust: None,
            trust: Trust::system(),
            identity: None,
            cookies: Cookies::default(),
            cache: Cache::default(),
            headers: Vec::new(),
            max_per_host: None,
            wait_for_connectivity: false,
            question_timeout: DEFAULT_QUESTION_TIMEOUT,
            transport: None,
        }
    }
}

impl std::fmt::Debug for ClientBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientBuilder")
            .field("redirects", &self.redirects)
            .field("cookies", &self.cookies)
            .field("cache", &self.cache)
            .finish_non_exhaustive()
    }
}

impl ClientBuilder {
    /// How long an exchange may sit without progress, for requests that set no
    /// [`Request::timeout`]. Default 30 s.
    pub fn timeout_idle(mut self, limit: Duration) -> Self {
        self.timeout_idle = limit;
        self
    }

    /// How long a whole request may take, redirects, challenges and body included. Default
    /// none. A request's own [`Request::timeout_total`] wins.
    pub fn timeout_total(mut self, limit: Duration) -> Self {
        self.timeout_total = Some(limit);
        self
    }

    /// Whether to follow redirects. Default [`Redirects::Follow`]`(10)`.
    pub fn redirects(mut self, redirects: Redirects) -> Self {
        self.redirects = redirects;
        self
    }

    /// Ask before following each redirect. Needs [`Capabilities::manual_redirects`]; where the
    /// platform follows redirects itself, requests from this client fail with
    /// [`HttpError::Unsupported`].
    pub fn on_redirect(
        mut self,
        handler: impl Fn(&Hop, RedirectReply) + Send + Sync + 'static,
    ) -> Self {
        self.on_redirect = Some(Arc::new(handler));
        self
    }

    /// Answer authentication challenges. Without a handler a 401 or 407 is delivered as the
    /// response.
    pub fn on_challenge(
        mut self,
        handler: impl Fn(&Challenge, ChallengeReply) + Send + Sync + 'static,
    ) -> Self {
        self.on_challenge = Some(Arc::new(handler));
        self
    }

    /// Public-key pins, checked on top of the platform's evaluation.
    pub fn trust(mut self, trust: Trust) -> Self {
        self.trust = trust;
        self
    }

    /// Decide on servers the platform does not trust, or confirm the ones it does. Needs
    /// [`Capabilities::server_trust`].
    pub fn on_server_trust(
        mut self,
        handler: impl Fn(&ServerTrust, TrustReply) + Send + Sync + 'static,
    ) -> Self {
        self.on_server_trust = Some(Arc::new(handler));
        self
    }

    /// Present this identity when a server asks for a client certificate. Needs
    /// [`Capabilities::client_identity`].
    pub fn identity(mut self, identity: Identity) -> Self {
        self.identity = Some(identity);
        self
    }

    /// Where to keep cookies. Default [`Cookies::default`].
    pub fn cookies(mut self, cookies: Cookies) -> Self {
        self.cookies = cookies;
        self
    }

    /// Whether to keep an HTTP cache. Default [`Cache::platform`].
    pub fn cache(mut self, cache: Cache) -> Self {
        self.cache = cache;
        self
    }

    /// A header sent with every request that does not set it itself.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// The `User-Agent` sent with every request.
    pub fn user_agent(self, agent: &str) -> Self {
        self.header("User-Agent", agent)
    }

    /// At most this many connections to one host, where the platform lets an app choose.
    pub fn max_per_host(mut self, limit: u32) -> Self {
        self.max_per_host = Some(limit.max(1));
        self
    }

    /// Wait for a usable network instead of failing at once, where the platform can
    /// ([`Capabilities::wait_for_connectivity`]).
    pub fn wait_for_connectivity(mut self, wait: bool) -> Self {
        self.wait_for_connectivity = wait;
        self
    }

    /// How long a handler may hold a question before its default answer is taken. Default two
    /// minutes.
    pub fn question_timeout(mut self, limit: Duration) -> Self {
        self.question_timeout = limit;
        self
    }

    /// Perform exchanges through this transport instead of the platform's: a test double, or
    /// an app's own stack.
    pub fn transport(mut self, transport: impl Transport) -> Self {
        self.transport = Some(Arc::new(transport));
        self
    }

    /// The client.
    pub fn build(self) -> Client {
        let caps = match &self.transport {
            Some(t) => t.capabilities(),
            None => crate::platform_capabilities(),
        };
        let cookies = match self.cookies {
            Cookies::Platform if !caps.platform_cookies => Cookies::jar(),
            other => other,
        };
        let config = TransportConfig {
            timeout_idle: self.timeout_idle,
            timeout_total: self.timeout_total,
            platform_cookies: matches!(cookies, Cookies::Platform),
            platform_cache: match self.cache {
                Cache::Platform { memory, disk } => Some((memory, disk)),
                Cache::Off => None,
            },
            redirects: self.redirects,
            ask_auth: self.on_challenge.is_some(),
            ask_trust: self.on_server_trust.is_some() || !self.trust.is_empty(),
            identity: self.identity,
            max_per_host: self.max_per_host,
            wait_for_connectivity: self.wait_for_connectivity,
        };
        let transport = match self.transport {
            Some(t) => t,
            None => crate::platform_transport(&config),
        };
        Client {
            inner: Arc::new(ClientInner {
                caps: transport.capabilities(),
                transport,
                timeout_idle: self.timeout_idle,
                timeout_total: self.timeout_total,
                redirects: self.redirects,
                on_redirect: self.on_redirect,
                on_challenge: self.on_challenge,
                on_server_trust: self.on_server_trust,
                trust: self.trust,
                cookies,
                headers: self.headers,
                question_timeout: self.question_timeout,
                credentials: Mutex::new(Vec::new()),
            }),
        }
    }
}

/// An HTTP client: policy shared by every request it sends, over the platform's transport.
/// Cloning is cheap and shares the policy, the cookies and the connections.
#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    transport: Arc<dyn Transport>,
    caps: Capabilities,
    timeout_idle: Duration,
    timeout_total: Option<Duration>,
    redirects: Redirects,
    on_redirect: Option<Arc<RedirectHandler>>,
    on_challenge: Option<Arc<ChallengeHandler>>,
    on_server_trust: Option<Arc<TrustHandler>>,
    trust: Trust,
    cookies: Cookies,
    headers: Vec<(String, String)>,
    question_timeout: Duration,
    /// Credentials that worked, by origin and realm, offered again without asking.
    credentials: Mutex<Vec<(String, Option<String>, Resolution)>>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("capabilities", &self.inner.caps)
            .field("redirects", &self.inner.redirects)
            .field("cookies", &self.inner.cookies)
            .finish_non_exhaustive()
    }
}

impl Default for Client {
    fn default() -> Self {
        Client::new()
    }
}

impl Client {
    /// A client with the default policy over the platform's transport.
    pub fn new() -> Client {
        ClientBuilder::default().build()
    }

    /// Configure a client.
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    /// What this client's transport can do.
    pub fn capabilities(&self) -> Capabilities {
        self.inner.caps
    }

    /// Send `request`; `on_head` receives the response head with its unread [`Body`], or the
    /// failure. It runs on the thread the platform reports on.
    pub fn send_async(
        &self,
        request: Request,
        on_head: impl FnOnce(Result<Streaming, HttpError>) + Send + 'static,
    ) -> InFlight {
        InFlight(Arc::downgrade(&Flight::start(
            &self.inner,
            request,
            Box::new(on_head),
        )))
    }

    /// Send `request` and await the response head. Dropping the future cancels the request.
    pub fn send_future(&self, request: Request) -> Sending {
        let (tx, rx) = day_async::oneshot();
        let flight = Flight::start(&self.inner, request, Box::new(move |r| tx.send(r)));
        Sending {
            flight,
            rx,
            done: false,
        }
    }

    /// Send `request` and read its whole body into memory. For large bodies use
    /// [`Client::send_async`] and read the [`Body`] in chunks.
    pub fn fetch_async(
        &self,
        request: Request,
        on_done: impl FnOnce(Result<Response, HttpError>) + Send + 'static,
    ) -> InFlight {
        InFlight(Arc::downgrade(&Flight::start(
            &self.inner,
            request,
            Box::new(move |head| match head {
                Ok(streaming) => streaming.collect_async(on_done),
                Err(e) => on_done(Err(e)),
            }),
        )))
    }

    /// Send `request` and await the whole response. Dropping the future cancels the request.
    pub fn fetch_future(&self, request: Request) -> Fetching {
        let (tx, rx) = day_async::oneshot();
        let flight = Flight::start(
            &self.inner,
            request,
            Box::new(move |head| match head {
                Ok(streaming) => streaming.collect_async(move |r| tx.send(r)),
                Err(e) => tx.send(Err(e)),
            }),
        );
        Fetching {
            flight,
            rx,
            done: false,
        }
    }

    /// [`Client::fetch_async`], with a closure that cancels the request.
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub(crate) fn fetch_cancellable(
        &self,
        request: Request,
        on_done: impl FnOnce(Result<Response, HttpError>) + Send + 'static,
    ) -> Box<dyn FnOnce() + Send> {
        let flight = Flight::start(
            &self.inner,
            request,
            Box::new(move |head| match head {
                Ok(streaming) => streaming.collect_async(on_done),
                Err(e) => on_done(Err(e)),
            }),
        );
        Box::new(move || flight.cancel())
    }

    /// Open a WebSocket (`ws://` or `wss://`); `on_open` receives it after the handshake, or the
    /// failure.
    pub fn websocket_async(
        &self,
        request: Request,
        on_open: impl FnOnce(Result<WebSocket, HttpError>) + Send + 'static,
    ) {
        WsShared::open(&self.inner, request, Box::new(on_open));
    }

    /// Open a WebSocket and await the handshake. Dropping the future abandons the connection.
    pub fn websocket_future(&self, request: Request) -> Connecting {
        let (tx, rx) = day_async::oneshot();
        let shared = WsShared::open(&self.inner, request, Box::new(move |r| tx.send(r)));
        Connecting {
            shared,
            rx,
            done: false,
        }
    }

    /// The cookies this client would send, from its jar or the platform's store.
    pub fn cookies(&self) -> Vec<Cookie> {
        match &self.inner.cookies {
            Cookies::Jar(jar) => jar.cookies(),
            Cookies::Platform => self.inner.transport.cookies(),
            Cookies::Off => Vec::new(),
        }
    }

    /// Forget every cookie in this client's jar or the platform's store.
    pub fn clear_cookies(&self) {
        match &self.inner.cookies {
            Cookies::Jar(jar) => jar.clear(),
            Cookies::Platform => self.inner.transport.clear_cookies(),
            Cookies::Off => {}
        }
    }

    /// Empty the platform cache this client uses.
    pub fn clear_cache(&self) {
        self.inner.transport.clear_cache();
    }
}

impl ClientInner {
    fn prepare(&self, request: &Request, websocket: bool) -> Result<Prepared, HttpError> {
        let bad = || HttpError::BadUrl(request.url.clone());
        let written = request.url.trim();
        let url = match url::Url::parse(written) {
            Ok(url) => Some(url),
            // A page resolves a relative reference against its own address, which only the
            // browser knows, so the request goes out as written.
            #[cfg(target_arch = "wasm32")]
            Err(url::ParseError::RelativeUrlWithoutBase) if !written.is_empty() => None,
            Err(_) => return Err(bad()),
        };
        if let Some(url) = &url {
            let scheme_ok = if websocket {
                matches!(url.scheme(), "ws" | "wss")
            } else {
                matches!(url.scheme(), "http" | "https")
            };
            if !scheme_ok || url.host_str().is_none() {
                return Err(bad());
            }
        }
        let mut headers: Vec<(String, String)> = self
            .headers
            .iter()
            .filter(|(k, _)| !has_header(&request.headers, k))
            .cloned()
            .collect();
        headers.extend(request.headers.iter().cloned());
        if let Cookies::Jar(jar) = &self.cookies
            && let Some(url) = &url
            && !has_header(&headers, "cookie")
            && let Some(cookie) = jar.header_for(url)
        {
            headers.push(("Cookie".into(), cookie));
        }
        let body = prepare_body(&request.payload, &mut headers)?;
        Ok(Prepared {
            method: request.method.as_str().to_string(),
            url: url.map_or_else(|| written.to_string(), |url| url.to_string()),
            headers,
            body,
            timeout_idle: request.timeout.unwrap_or(self.timeout_idle),
            allow_expensive: request.allow_expensive,
            allow_constrained: request.allow_constrained,
            cache: request.cache,
            priority: request.priority,
            protocols: request.protocols.clone(),
        })
    }

    fn cached_credential(&self, origin: &str, realm: &Option<String>) -> Option<Resolution> {
        lock(&self.credentials)
            .iter()
            .find(|(o, r, _)| o == origin && r == realm)
            .map(|(_, _, c)| c.clone())
    }
}

fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
}

fn prepare_body(
    payload: &Payload,
    headers: &mut Vec<(String, String)>,
) -> Result<PreparedBody, HttpError> {
    let io = |e: std::io::Error| HttpError::Io(e.to_string());
    Ok(match payload {
        Payload::Empty => PreparedBody::Empty,
        Payload::Bytes(bytes) => PreparedBody::Bytes(bytes.clone()),
        Payload::File(path) => PreparedBody::File {
            len: std::fs::metadata(path).map_err(io)?.len(),
            path: path.clone(),
        },
        Payload::Stream(reader, len) => PreparedBody::Stream {
            reader: reader.clone(),
            len: *len,
        },
        Payload::Form(form) => {
            let (reader, len) = form.reader().map_err(io)?;
            if !has_header(headers, "content-type") {
                headers.push(("Content-Type".into(), form.content_type()));
            }
            PreparedBody::Stream {
                reader: SharedReader::new(reader),
                len: Some(len),
            }
        }
    })
}

/// The request that follows a redirect (RFC 9110 §15.4): the method rewrite, and no credentials
/// or cookies carried to another origin.
fn redirect_request(previous: &Request, status: u16, from: &url::Url, to: &url::Url) -> Request {
    let mut next = previous.clone();
    next.url = to.to_string();
    let rewrite = (status == 303 && previous.method != Method::Head)
        || (matches!(status, 301 | 302) && previous.method == Method::Post);
    if rewrite {
        next.method = Method::Get;
        next.payload = Payload::Empty;
        next.headers.retain(|(k, _)| {
            !k.eq_ignore_ascii_case("content-type") && !k.eq_ignore_ascii_case("content-length")
        });
    }
    if from.origin() != to.origin() {
        next.headers.retain(|(k, _)| {
            !k.eq_ignore_ascii_case("authorization")
                && !k.eq_ignore_ascii_case("proxy-authorization")
                && !k.eq_ignore_ascii_case("cookie")
        });
    }
    next
}

fn origin_of(url: &url::Url) -> String {
    url.origin().ascii_serialization()
}

type HeadCallback = Box<dyn FnOnce(Result<Streaming, HttpError>) + Send>;
type ItemCallback<T> = Box<dyn FnMut(Option<Result<T, HttpError>>) -> bool + Send>;

/// A queue between a transport and one reader, granting demand as the reader takes items.
struct Inbox<T> {
    queue: VecDeque<T>,
    /// Demand granted and not yet used.
    outstanding: u32,
    /// The terminal outcome, delivered once the queue is empty.
    finished: Option<Result<(), HttpError>>,
    /// The terminal outcome was delivered.
    taken: bool,
    paused: bool,
    waker: Option<Waker>,
    callback: Option<ItemCallback<T>>,
    /// A thread holds the callback and is delivering.
    draining: bool,
}

enum Notify {
    Wake(Waker),
    Drain,
}

impl<T> Inbox<T> {
    fn new() -> Self {
        Inbox {
            queue: VecDeque::new(),
            outstanding: 0,
            finished: None,
            taken: false,
            paused: false,
            waker: None,
            callback: None,
            draining: false,
        }
    }

    fn notify(&mut self) -> Option<Notify> {
        if self.callback.is_some() {
            Some(Notify::Drain)
        } else {
            self.waker.take().map(Notify::Wake)
        }
    }

    fn push(&mut self, item: T) -> Option<Notify> {
        self.outstanding = self.outstanding.saturating_sub(1);
        self.queue.push_back(item);
        self.notify()
    }

    fn finish(&mut self, outcome: Result<(), HttpError>) -> Option<Notify> {
        if self.finished.is_none() && !self.taken {
            self.finished = Some(outcome);
        }
        self.notify()
    }

    /// The next item to deliver: `Some(None)` is the clean end, `None` is nothing yet.
    fn take(&mut self) -> Option<Option<Result<T, HttpError>>> {
        if let Some(item) = self.queue.pop_front() {
            return Some(Some(Ok(item)));
        }
        if let Some(outcome) = self.finished.take() {
            self.taken = true;
            return Some(outcome.err().map(Err));
        }
        if self.taken {
            return Some(None);
        }
        None
    }

    /// More demand to grant, when the reader's hand runs low.
    fn refill(&mut self) -> Option<u32> {
        if self.paused || self.finished.is_some() || self.taken {
            return None;
        }
        let held = self.queue.len() as u32 + self.outstanding;
        if held > WINDOW / 2 {
            return None;
        }
        let more = WINDOW - held;
        self.outstanding += more;
        Some(more)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Head,
    Body,
    Done,
}

enum Asked {
    Redirect {
        head: Head,
        next: Request,
    },
    Challenge {
        head: Head,
        parsed: ParsedChallenge,
        challenge: Challenge,
        url: url::Url,
    },
    NativeAuth(QuestionId),
    Trust(QuestionId),
}

enum PendingOp {
    Demand(u32),
    Answer(QuestionId, Answer),
}

enum Action {
    Demand(Arc<dyn Transfer>, u32),
    Answer(Arc<dyn Transfer>, QuestionId, Answer),
    Cancel(Arc<dyn Transfer>),
    StartHop,
    DeliverHead(HeadCallback, Box<Head>),
    HeadFailed(HeadCallback, HttpError),
    Wake(Waker),
    Drain,
    AskRedirect(Arc<RedirectHandler>, Hop, u64),
    AskChallenge(Arc<ChallengeHandler>, Challenge, u64),
    AskTrust(Arc<TrustHandler>, ServerTrust, u64),
    CancelTimer(TimerId),
    Progress(ProgressFn, u64, Option<u64>),
}

impl From<Notify> for Action {
    fn from(n: Notify) -> Action {
        match n {
            Notify::Wake(w) => Action::Wake(w),
            Notify::Drain => Action::Drain,
        }
    }
}

/// One request across its redirects and challenges.
struct Flight {
    client: Arc<ClientInner>,
    state: Mutex<FlightState>,
}

struct FlightState {
    request: Request,
    stage: Stage,
    /// Bumped for each new exchange, so late events from an abandoned one are ignored.
    generation: u64,
    transfer: Option<Arc<dyn Transfer>>,
    /// The current exchange reported its terminal event.
    exchange_over: bool,
    pending: Vec<PendingOp>,
    on_head: Option<HeadCallback>,
    inbox: Inbox<Vec<u8>>,
    received: u64,
    expected: Option<u64>,
    metrics: Option<Metrics>,
    redirects_followed: u32,
    auth_failures: u32,
    /// The handler declined a challenge the transport raised, so its 401 is the response.
    native_auth_declined: bool,
    /// Credentials offered on the current exchange, remembered once they work.
    offered: Option<(String, Option<String>, Resolution)>,
    asks: Vec<(u64, Asked, Option<TimerId>)>,
    next_ask: u64,
    total_timer: Option<TimerId>,
}

impl Flight {
    fn start(client: &Arc<ClientInner>, request: Request, on_head: HeadCallback) -> Arc<Flight> {
        let total = request.timeout_total.or(client.timeout_total);
        // A handler the stack cannot consult, or pins it cannot check, must not be silently
        // skipped: such a request fails instead.
        let unsupported = (client.on_redirect.is_some()
            && !client.caps.manual_redirects
            && client.redirects != Redirects::Never)
            || (!client.trust.is_empty() && !client.caps.server_trust);
        let flight = Arc::new(Flight {
            client: client.clone(),
            state: Mutex::new(FlightState {
                request,
                stage: Stage::Head,
                generation: 0,
                transfer: None,
                exchange_over: false,
                pending: Vec::new(),
                on_head: Some(on_head),
                inbox: Inbox::new(),
                received: 0,
                expected: None,
                metrics: None,
                redirects_followed: 0,
                auth_failures: 0,
                native_auth_declined: false,
                offered: None,
                asks: Vec::new(),
                next_ask: 1,
                total_timer: None,
            }),
        });
        if unsupported {
            flight.fail_now(HttpError::Unsupported);
            return flight;
        }
        if let Some(total) = total {
            let weak = Arc::downgrade(&flight);
            let timer = day_async::schedule(total, move || {
                if let Some(flight) = weak.upgrade() {
                    flight.fail_now(HttpError::Timeout);
                }
            });
            lock(&flight.state).total_timer = Some(timer);
        }
        flight.start_exchange();
        flight
    }

    fn start_exchange(self: &Arc<Self>) {
        let (request, generation) = {
            let st = lock(&self.state);
            if st.stage == Stage::Done {
                return;
            }
            (st.request.clone(), st.generation)
        };
        let prepared = match self.client.prepare(&request, false) {
            Ok(p) => p,
            Err(e) => return self.fail_now(e),
        };
        let this = self.clone();
        let events: Events = Arc::new(move |event| this.on_event(generation, event));
        let transfer = self.client.transport.start(prepared, events);
        let mut actions = Vec::new();
        {
            let mut st = lock(&self.state);
            if st.generation == generation && !st.exchange_over && st.stage != Stage::Done {
                for op in std::mem::take(&mut st.pending) {
                    actions.push(match op {
                        PendingOp::Demand(n) => Action::Demand(transfer.clone(), n),
                        PendingOp::Answer(id, a) => Action::Answer(transfer.clone(), id, a),
                    });
                }
                st.transfer = Some(transfer);
            } else {
                actions.push(Action::Cancel(transfer));
            }
        }
        self.run(actions);
    }

    fn on_event(self: &Arc<Self>, generation: u64, event: Event) {
        let mut actions = Vec::new();
        {
            let mut st = lock(&self.state);
            if generation != st.generation || st.stage == Stage::Done {
                return;
            }
            match event {
                Event::Head(head) => self.head_arrived(&mut st, head, &mut actions),
                Event::Chunk(bytes) => {
                    // A head waiting on a redirect or challenge answer keeps its body too, in
                    // case the answer makes that response the response.
                    if st.stage == Stage::Body || parked(&st) {
                        st.received += bytes.len() as u64;
                        if let Some(n) = st.inbox.push(bytes) {
                            actions.push(n.into());
                        }
                    }
                }
                Event::Sent { sent, total } => {
                    if let Some(progress) = st.request.upload_progress.clone() {
                        actions.push(Action::Progress(progress, sent, total));
                    }
                }
                Event::Question(id, question) => self.question(&mut st, id, question, &mut actions),
                Event::Metrics(mut metrics) => {
                    metrics.redirects = st.redirects_followed;
                    st.metrics = Some(metrics);
                }
                Event::End => {
                    st.exchange_over = true;
                    st.transfer = None;
                    match st.stage {
                        Stage::Body => {
                            if let Some(n) = st.inbox.finish(Ok(())) {
                                actions.push(n.into());
                            }
                        }
                        // An empty body can end while its head waits on an answer.
                        Stage::Head if parked(&st) => {}
                        Stage::Head => fail(
                            &mut st,
                            HttpError::Io("the exchange ended without a response".into()),
                            &mut actions,
                        ),
                        Stage::Done => {}
                    }
                }
                Event::Failed(error) => {
                    st.exchange_over = true;
                    st.transfer = None;
                    fail(&mut st, error, &mut actions);
                }
            }
        }
        self.run(actions);
    }

    fn head_arrived(&self, st: &mut FlightState, head: Head, actions: &mut Vec<Action>) {
        let client = &self.client;
        let base = url::Url::parse(&head.url)
            .or_else(|_| url::Url::parse(&st.request.url))
            .ok();
        if let (Cookies::Jar(jar), Some(url)) = (&client.cookies, &base) {
            jar.store_all(url, head.headers_named("set-cookie"));
        }
        let status = head.status;

        if client.caps.manual_redirects
            && matches!(status, 301 | 302 | 303 | 307 | 308)
            && let Redirects::Follow(max) = client.redirects
            && let Some(from) = &base
            && let Some(to) = head.header("location").and_then(|l| from.join(l).ok())
            && matches!(to.scheme(), "http" | "https")
        {
            if st.redirects_followed >= max {
                return fail(st, HttpError::TooManyRedirects, actions);
            }
            let next = redirect_request(&st.request, status, from, &to);
            if let Some(handler) = client.on_redirect.clone() {
                let hop = Hop {
                    status,
                    from: from.to_string(),
                    to: to.to_string(),
                    method: next.method.as_str().to_string(),
                    followed: st.redirects_followed,
                };
                let ask = register(st, Asked::Redirect { head, next });
                actions.push(Action::AskRedirect(handler, hop, ask));
                return;
            }
            st.redirects_followed += 1;
            return replace_exchange(st, next, actions);
        }

        if matches!(status, 401 | 407)
            && !st.native_auth_declined
            && st.auth_failures < MAX_AUTH_ATTEMPTS
            && let Some(url) = base.clone()
        {
            let proxy = status == 407;
            let native = client.caps.auth_questions;
            let chosen = parse_challenges(&head.headers, proxy)
                .into_iter()
                .filter(|c| !(native && c.scheme.raised_natively()))
                .max_by_key(|c| match c.scheme {
                    Scheme::Digest => 3,
                    Scheme::Basic => 2,
                    Scheme::Bearer => 1,
                    _ => 0,
                });
            if let Some(parsed) = chosen {
                let realm = parsed.param("realm").map(str::to_string);
                let challenge = Challenge {
                    url: url.to_string(),
                    host: url.host_str().unwrap_or_default().to_string(),
                    port: url.port_or_known_default().unwrap_or(0),
                    realm: realm.clone(),
                    scheme: parsed.scheme.clone(),
                    proxy,
                    previous_failures: st.auth_failures,
                };
                if st.auth_failures == 0
                    && let Some(known) = client.cached_credential(&origin_of(&url), &realm)
                {
                    return answer_challenge(st, &parsed, &challenge, &url, known, head, actions);
                }
                if let Some(handler) = client.on_challenge.clone() {
                    let ask = register(
                        st,
                        Asked::Challenge {
                            head,
                            parsed,
                            challenge: challenge.clone(),
                            url,
                        },
                    );
                    actions.push(Action::AskChallenge(handler, challenge, ask));
                    return;
                }
            }
        }

        if let Some(offered) = st.offered.take()
            && !matches!(status, 401 | 407)
        {
            let mut known = lock(&client.credentials);
            known.retain(|(o, r, _)| !(o == &offered.0 && r == &offered.1));
            known.push(offered);
        }
        begin_body(st, head, actions);
    }

    fn question(
        &self,
        st: &mut FlightState,
        id: QuestionId,
        question: Question,
        actions: &mut Vec<Action>,
    ) {
        let client = &self.client;
        match question {
            Question::Auth(asked) => match client.on_challenge.clone() {
                Some(handler) if asked.previous_failures < MAX_AUTH_ATTEMPTS => {
                    let challenge = Challenge {
                        url: st.request.url.clone(),
                        host: asked.host,
                        port: asked.port,
                        realm: asked.realm,
                        scheme: asked.scheme,
                        proxy: asked.proxy,
                        previous_failures: asked.previous_failures,
                    };
                    let ask = register(st, Asked::NativeAuth(id));
                    actions.push(Action::AskChallenge(handler, challenge, ask));
                }
                _ => {
                    st.native_auth_declined = true;
                    answer(st, id, Answer::DefaultHandling, actions);
                }
            },
            Question::ServerTrust(trust) => {
                if let Some(pins) = client.trust.pins_for(&trust.host) {
                    let chain: Vec<[u8; 32]> =
                        trust.chain.iter().filter_map(|c| spki_sha256(c)).collect();
                    if !pins.iter().flatten().any(|p| chain.contains(p)) {
                        return answer(st, id, Answer::Reject, actions);
                    }
                    if trust.system_trusted {
                        return answer(st, id, Answer::Accept, actions);
                    }
                }
                match client.on_server_trust.clone() {
                    Some(handler) => {
                        let ask = register(st, Asked::Trust(id));
                        actions.push(Action::AskTrust(handler, trust, ask));
                    }
                    None => answer(st, id, Answer::DefaultHandling, actions),
                }
            }
        }
    }

    fn resolve(self: &Arc<Self>, ask: u64, resolution: Resolution) {
        let mut actions = Vec::new();
        {
            let mut st = lock(&self.state);
            let Some(index) = st.asks.iter().position(|(a, _, _)| *a == ask) else {
                return;
            };
            let (_, asked, timer) = st.asks.remove(index);
            if let Some(timer) = timer {
                actions.push(Action::CancelTimer(timer));
            }
            if st.stage != Stage::Done {
                self.apply(&mut st, asked, resolution, &mut actions);
            }
        }
        self.run(actions);
    }

    fn apply(
        &self,
        st: &mut FlightState,
        asked: Asked,
        resolution: Resolution,
        actions: &mut Vec<Action>,
    ) {
        match asked {
            Asked::Redirect { head, next } => match resolution {
                Resolution::Stop => begin_body(st, head, actions),
                Resolution::Cancel => fail(st, HttpError::Cancelled, actions),
                _ => {
                    st.redirects_followed += 1;
                    replace_exchange(st, next, actions);
                }
            },
            Asked::Challenge {
                head,
                parsed,
                challenge,
                url,
            } => answer_challenge(st, &parsed, &challenge, &url, resolution, head, actions),
            Asked::NativeAuth(id) => {
                let answer_with = match resolution {
                    Resolution::Credential { user, password } => {
                        Answer::Credential { user, password }
                    }
                    Resolution::Cancel => Answer::Cancel,
                    _ => {
                        st.native_auth_declined = true;
                        Answer::DefaultHandling
                    }
                };
                answer(st, id, answer_with, actions);
            }
            Asked::Trust(id) => {
                let answer_with = match resolution {
                    Resolution::Accept => Answer::Accept,
                    Resolution::Reject | Resolution::Cancel => Answer::Reject,
                    _ => Answer::DefaultHandling,
                };
                answer(st, id, answer_with, actions);
            }
        }
    }

    fn fail_now(self: &Arc<Self>, error: HttpError) {
        let mut actions = Vec::new();
        fail(&mut lock(&self.state), error, &mut actions);
        self.run(actions);
    }

    /// Abandon the request from the reader's side: nothing more is delivered.
    fn cancel(self: &Arc<Self>) {
        let mut actions = Vec::new();
        let (on_head, callback) = {
            let mut st = lock(&self.state);
            if st.stage == Stage::Done {
                return;
            }
            st.stage = Stage::Done;
            st.inbox.taken = true;
            st.inbox.finished = None;
            st.inbox.queue.clear();
            st.inbox.waker = None;
            teardown(&mut st, &mut actions);
            (st.on_head.take(), st.inbox.callback.take())
        };
        drop((on_head, callback));
        self.run(actions);
    }

    fn register_timer(self: &Arc<Self>, ask: u64) {
        let weak = Arc::downgrade(self);
        let timer = day_async::schedule(self.client.question_timeout, move || {
            if let Some(flight) = weak.upgrade() {
                flight.resolve(ask, Resolution::Default);
            }
        });
        let mut st = lock(&self.state);
        match st.asks.iter_mut().find(|(a, _, _)| *a == ask) {
            Some(entry) => entry.2 = Some(timer),
            None => day_async::unschedule(timer),
        }
    }

    fn grip(self: &Arc<Self>, ask: u64) -> ReplyGrip {
        ReplyGrip {
            flight: Arc::downgrade(self),
            ask,
            answered: false,
        }
    }

    fn run(self: &Arc<Self>, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Demand(transfer, n) => transfer.demand(n),
                Action::Answer(transfer, id, answer) => transfer.answer(id, answer),
                Action::Cancel(transfer) => transfer.cancel(),
                Action::StartHop => self.start_exchange(),
                Action::DeliverHead(on_head, head) => on_head(Ok(Streaming {
                    head: *head,
                    body: Body {
                        flight: self.clone(),
                        handed_off: false,
                    },
                })),
                Action::HeadFailed(on_head, error) => on_head(Err(error)),
                Action::Wake(waker) => waker.wake(),
                Action::Drain => self.drain(),
                Action::AskRedirect(handler, hop, ask) => {
                    self.register_timer(ask);
                    handler(&hop, RedirectReply(self.grip(ask)));
                }
                Action::AskChallenge(handler, challenge, ask) => {
                    self.register_timer(ask);
                    handler(&challenge, ChallengeReply(self.grip(ask)));
                }
                Action::AskTrust(handler, trust, ask) => {
                    self.register_timer(ask);
                    handler(&trust, TrustReply(self.grip(ask)));
                }
                Action::CancelTimer(timer) => day_async::unschedule(timer),
                Action::Progress(progress, sent, total) => (progress.0)(sent, total),
            }
        }
    }

    /// Deliver queued chunks to the body's callback, one thread at a time and in order.
    fn drain(self: &Arc<Self>) {
        loop {
            let mut actions = Vec::new();
            let (item, mut callback) = {
                let mut st = lock(&self.state);
                if st.inbox.draining {
                    return;
                }
                let Some(callback) = st.inbox.callback.take() else {
                    return;
                };
                let Some(item) = st.inbox.take() else {
                    st.inbox.callback = Some(callback);
                    return;
                };
                if let Some(n) = st.inbox.refill() {
                    demand(&mut st, n, &mut actions);
                }
                st.inbox.draining = true;
                (item, callback)
            };
            self.run(actions);
            let terminal = !matches!(item, Some(Ok(_)));
            if terminal {
                self.finish_body();
            }
            let keep = callback(item);
            let mut st = lock(&self.state);
            st.inbox.draining = false;
            if terminal {
                return;
            }
            if keep {
                st.inbox.callback = Some(callback);
            } else {
                drop(st);
                drop(callback);
                return self.cancel();
            }
        }
    }

    /// The body's terminal outcome went to the reader: stop the clocks.
    fn finish_body(self: &Arc<Self>) {
        let mut actions = Vec::new();
        {
            let mut st = lock(&self.state);
            if st.stage != Stage::Done {
                st.stage = Stage::Done;
                teardown(&mut st, &mut actions);
            }
        }
        self.run(actions);
    }
}

/// A response head is waiting on a redirect or challenge answer.
fn parked(st: &FlightState) -> bool {
    st.asks
        .iter()
        .any(|(_, asked, _)| matches!(asked, Asked::Redirect { .. } | Asked::Challenge { .. }))
}

fn register(st: &mut FlightState, asked: Asked) -> u64 {
    let ask = st.next_ask;
    st.next_ask += 1;
    st.asks.push((ask, asked, None));
    ask
}

fn demand(st: &mut FlightState, n: u32, actions: &mut Vec<Action>) {
    match &st.transfer {
        Some(t) => actions.push(Action::Demand(t.clone(), n)),
        None if !st.exchange_over => st.pending.push(PendingOp::Demand(n)),
        None => {}
    }
}

fn answer(st: &mut FlightState, id: QuestionId, answer: Answer, actions: &mut Vec<Action>) {
    match &st.transfer {
        Some(t) => actions.push(Action::Answer(t.clone(), id, answer)),
        None if !st.exchange_over => st.pending.push(PendingOp::Answer(id, answer)),
        None => {}
    }
}

/// Stop the current exchange and send `next` in its place.
fn replace_exchange(st: &mut FlightState, next: Request, actions: &mut Vec<Action>) {
    if let Some(t) = st.transfer.take() {
        actions.push(Action::Cancel(t));
    }
    st.generation += 1;
    st.exchange_over = false;
    st.pending.clear();
    st.inbox.queue.clear();
    st.inbox.outstanding = 0;
    st.inbox.finished = None;
    st.received = 0;
    st.request = next;
    st.stage = Stage::Head;
    actions.push(Action::StartHop);
}

fn answer_challenge(
    st: &mut FlightState,
    parsed: &ParsedChallenge,
    challenge: &Challenge,
    url: &url::Url,
    resolution: Resolution,
    head: Head,
    actions: &mut Vec<Action>,
) {
    let value = match &resolution {
        Resolution::Credential { user, password } => match parsed.scheme {
            Scheme::Digest => {
                let mut uri = url.path().to_string();
                if let Some(q) = url.query() {
                    uri.push('?');
                    uri.push_str(q);
                }
                digest_authorization(parsed, user, password, st.request.method.as_str(), &uri)
            }
            _ => Some(basic_authorization(user, password)),
        },
        Resolution::Bearer(token) => Some(format!("Bearer {token}")),
        Resolution::Cancel => return fail(st, HttpError::Cancelled, actions),
        _ => None,
    };
    let Some(value) = value else {
        return begin_body(st, head, actions);
    };
    let name = if challenge.proxy {
        "Proxy-Authorization"
    } else {
        "Authorization"
    };
    let mut next = st.request.clone();
    next.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    next.headers.push((name.to_string(), value));
    st.auth_failures += 1;
    st.offered = Some((origin_of(url), challenge.realm.clone(), resolution));
    replace_exchange(st, next, actions);
}

/// The final response head: hand it to the reader and start granting demand.
fn begin_body(st: &mut FlightState, head: Head, actions: &mut Vec<Action>) {
    st.stage = Stage::Body;
    st.expected = head.expected_length;
    if let Some(on_head) = st.on_head.take() {
        actions.push(Action::DeliverHead(on_head, Box::new(head)));
    }
    if let Some(n) = st.inbox.refill() {
        demand(st, n, actions);
    }
    // The exchange already ended while its head waited on an answer.
    if st.exchange_over
        && let Some(n) = st.inbox.finish(Ok(()))
    {
        actions.push(n.into());
    }
}

fn fail(st: &mut FlightState, error: HttpError, actions: &mut Vec<Action>) {
    match st.stage {
        Stage::Head => {
            st.stage = Stage::Done;
            // Tear the exchange down before the caller hears of the failure, as the body arm
            // does: a caller told "timed out" who retries at once must not have the old exchange
            // still running beside the new one. The `Failed(Cancelled)` a transport may raise
            // from inside `cancel` finds the flight `Done` and is dropped.
            teardown(st, actions);
            if let Some(on_head) = st.on_head.take() {
                actions.push(Action::HeadFailed(on_head, error));
            }
        }
        Stage::Body => {
            if let Some(t) = st.transfer.take() {
                actions.push(Action::Cancel(t));
            }
            if let Some(n) = st.inbox.finish(Err(error)) {
                actions.push(n.into());
            }
        }
        Stage::Done => {}
    }
}

fn teardown(st: &mut FlightState, actions: &mut Vec<Action>) {
    if let Some(t) = st.transfer.take() {
        actions.push(Action::Cancel(t));
    }
    st.pending.clear();
    for (_, _, timer) in st.asks.drain(..) {
        if let Some(timer) = timer {
            actions.push(Action::CancelTimer(timer));
        }
    }
    if let Some(timer) = st.total_timer.take() {
        actions.push(Action::CancelTimer(timer));
    }
}

/// A response head with its unread body.
#[derive(Debug)]
pub struct Streaming {
    head: Head,
    body: Body,
}

impl Streaming {
    /// The status code.
    pub fn status(&self) -> u16 {
        self.head.status
    }

    /// The headers, in arrival order.
    pub fn headers(&self) -> &[(String, String)] {
        &self.head.headers
    }

    /// The first header with this name, ASCII case-insensitive.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.head.header(name)
    }

    /// The URL that answered, after redirects.
    pub fn url(&self) -> &str {
        &self.head.url
    }

    /// `Content-Length`, when the response declared one.
    pub fn expected_length(&self) -> Option<u64> {
        self.head.expected_length
    }

    /// The whole head.
    pub fn head(&self) -> &Head {
        &self.head
    }

    /// The body, unread.
    pub fn into_body(self) -> Body {
        self.body
    }

    /// The head and the body, apart.
    pub fn into_parts(self) -> (Head, Body) {
        (self.head, self.body)
    }

    /// Read the whole body into memory; `on_done` receives the response.
    pub fn collect_async(self, on_done: impl FnOnce(Result<Response, HttpError>) + Send + 'static) {
        let Streaming { head, body } = self;
        let flight = Arc::downgrade(&body.flight);
        let capacity = head.expected_length.unwrap_or(0).min(16 << 20) as usize;
        let mut bytes = Vec::with_capacity(capacity);
        let mut on_done = Some(on_done);
        let mut head = Some(head);
        body.read_async(move |item| {
            match item {
                Some(Ok(chunk)) => {
                    bytes.extend_from_slice(&chunk);
                    return true;
                }
                Some(Err(e)) => {
                    if let Some(f) = on_done.take() {
                        f(Err(e));
                    }
                }
                None => {
                    if let (Some(f), Some(h)) = (on_done.take(), head.take()) {
                        let metrics = flight
                            .upgrade()
                            .and_then(|f| lock(&f.state).metrics.clone());
                        f(Ok(Response::assemble(
                            h,
                            std::mem::take(&mut bytes),
                            metrics,
                        )));
                    }
                }
            }
            false
        });
    }

    /// Read the whole body into memory and await the response.
    pub fn collect_future(self) -> Collecting {
        let flight = self.body.flight.clone();
        let (tx, rx) = day_async::oneshot();
        self.collect_async(move |r| tx.send(r));
        Collecting {
            flight,
            rx,
            done: false,
        }
    }
}

/// A response body, read in the chunks the platform delivers. The body pulls from the network
/// only while its reader keeps up. Dropping it before the end cancels the request.
pub struct Body {
    flight: Arc<Flight>,
    handed_off: bool,
}

impl std::fmt::Debug for Body {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Body")
            .field("received", &self.received())
            .finish_non_exhaustive()
    }
}

impl Body {
    /// The next chunk; `None` after the last.
    // Awaited (`body.next().await`), so it is not `Iterator::next`.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> NextChunk<'_> {
        NextChunk { body: self }
    }

    /// Wait for the next chunk on this thread. Never call it on the UI thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn next_blocking(&mut self) -> Option<Result<Vec<u8>, HttpError>> {
        wait(self.next())
    }

    /// A grip that cancels this body's request from any thread, also after
    /// [`Body::read_async`] took the body.
    pub fn in_flight(&self) -> InFlight {
        InFlight(Arc::downgrade(&self.flight))
    }

    /// Bytes received so far, including chunks not read yet.
    pub fn received(&self) -> u64 {
        lock(&self.flight.state).received
    }

    /// `Content-Length`, when the response declared one.
    pub fn expected_length(&self) -> Option<u64> {
        lock(&self.flight.state).expected
    }

    /// Transfer metrics, once the platform reported them (usually just before the end).
    pub fn metrics(&self) -> Option<Metrics> {
        lock(&self.flight.state).metrics.clone()
    }

    /// Stop pulling from the network. Chunks already on their way still arrive.
    pub fn pause(&self) {
        lock(&self.flight.state).inbox.paused = true;
    }

    /// Pull from the network again after [`Body::pause`].
    pub fn resume(&self) {
        let mut actions = Vec::new();
        {
            let mut st = lock(&self.flight.state);
            st.inbox.paused = false;
            if let Some(n) = st.inbox.refill() {
                demand(&mut st, n, &mut actions);
            }
            if let Some(n) = st.inbox.notify() {
                actions.push(n.into());
            }
        }
        self.flight.run(actions);
    }

    /// Hand each chunk to `on_chunk` as it arrives, then `None` at the end or `Some(Err)` on
    /// failure. The body pulls the next chunk only after `on_chunk` returns; returning `false`
    /// cancels the request.
    pub fn read_async(
        mut self,
        on_chunk: impl FnMut(Option<Result<Vec<u8>, HttpError>>) -> bool + Send + 'static,
    ) {
        self.handed_off = true;
        lock(&self.flight.state).inbox.callback = Some(Box::new(on_chunk));
        self.flight.drain();
    }
}

impl Drop for Body {
    fn drop(&mut self) {
        if !self.handed_off {
            self.flight.cancel();
        }
    }
}

/// The future [`Body::next`] returns.
pub struct NextChunk<'a> {
    body: &'a mut Body,
}

impl Future for NextChunk<'_> {
    type Output = Option<Result<Vec<u8>, HttpError>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let flight = self.body.flight.clone();
        let mut actions = Vec::new();
        let (out, terminal) = {
            let mut st = lock(&flight.state);
            let item = st.inbox.take();
            if let Some(n) = st.inbox.refill() {
                demand(&mut st, n, &mut actions);
            }
            match item {
                Some(item) => {
                    let terminal = !matches!(item, Some(Ok(_)));
                    (Poll::Ready(item), terminal)
                }
                None => {
                    st.inbox.waker = Some(cx.waker().clone());
                    (Poll::Pending, false)
                }
            }
        };
        flight.run(actions);
        if terminal {
            flight.finish_body();
        }
        out
    }
}

/// A grip on a request started with a callback: cancel it from any thread, at any stage. Holding
/// it does not keep the request alive, and cancelling a finished request does nothing.
#[derive(Clone, Debug)]
pub struct InFlight(Weak<Flight>);

impl InFlight {
    /// Cancel the request. Its callback is not called again.
    pub fn cancel(&self) {
        if let Some(flight) = self.0.upgrade() {
            flight.cancel();
        }
    }
}

impl std::fmt::Debug for Flight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Flight")
    }
}

/// The future [`Client::send_future`] returns.
pub struct Sending {
    flight: Arc<Flight>,
    rx: day_async::Oneshot<Result<Streaming, HttpError>>,
    done: bool,
}

impl Future for Sending {
    type Output = Result<Streaming, HttpError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.rx).poll(cx) {
            Poll::Ready(result) => {
                self.done = true;
                Poll::Ready(result.unwrap_or(Err(HttpError::Cancelled)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for Sending {
    fn drop(&mut self) {
        if !self.done {
            self.flight.cancel();
        }
    }
}

/// The future [`Client::fetch_future`] returns.
pub struct Fetching {
    flight: Arc<Flight>,
    rx: day_async::Oneshot<Result<Response, HttpError>>,
    done: bool,
}

impl Future for Fetching {
    type Output = Result<Response, HttpError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.rx).poll(cx) {
            Poll::Ready(result) => {
                self.done = true;
                Poll::Ready(result.unwrap_or(Err(HttpError::Cancelled)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for Fetching {
    fn drop(&mut self) {
        if !self.done {
            self.flight.cancel();
        }
    }
}

/// The future [`Streaming::collect_future`] returns.
pub struct Collecting {
    flight: Arc<Flight>,
    rx: day_async::Oneshot<Result<Response, HttpError>>,
    done: bool,
}

impl Future for Collecting {
    type Output = Result<Response, HttpError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.rx).poll(cx) {
            Poll::Ready(result) => {
                self.done = true;
                Poll::Ready(result.unwrap_or(Err(HttpError::Cancelled)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for Collecting {
    fn drop(&mut self) {
        if !self.done {
            self.flight.cancel();
        }
    }
}

/// Run a future to completion on this thread, parking between polls.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn wait<F: Future>(future: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        std::thread::park();
    }
}

// ---------------------------------------------------------------------------
// WebSockets
// ---------------------------------------------------------------------------

type OpenCallback = Box<dyn FnOnce(Result<WebSocket, HttpError>) + Send>;

struct WsShared {
    state: Mutex<WsState>,
}

struct WsState {
    socket: Option<Arc<dyn Socket>>,
    open: bool,
    over: bool,
    on_open: Option<OpenCallback>,
    protocol: Option<String>,
    inbox: Inbox<Message>,
    pending_demand: u32,
    /// Sends and a close requested before the transport handed back its socket.
    pending_sends: Vec<(Message, Completion)>,
    pending_close: Option<(u16, String)>,
}

impl WsShared {
    fn open(client: &Arc<ClientInner>, request: Request, on_open: OpenCallback) -> Arc<WsShared> {
        let shared = Arc::new(WsShared {
            state: Mutex::new(WsState {
                socket: None,
                open: false,
                over: false,
                on_open: None,
                protocol: None,
                inbox: Inbox::new(),
                pending_demand: 0,
                pending_sends: Vec::new(),
                pending_close: None,
            }),
        });
        // Pins are checked through a transport's trust questions, which a WebSocket handshake
        // does not raise: a pinned client fails closed rather than connecting unpinned.
        let prepared = match client.prepare(&request, true).and_then(|p| {
            if client.trust.is_empty() {
                Ok(p)
            } else {
                Err(HttpError::Unsupported)
            }
        }) {
            Ok(p) => p,
            Err(e) => {
                lock(&shared.state).over = true;
                on_open(Err(e));
                return shared;
            }
        };
        lock(&shared.state).on_open = Some(on_open);
        let this = shared.clone();
        let events: WsEvents = Arc::new(move |event| this.on_event(event));
        let socket = client.transport.websocket(prepared, events);
        let (keep, demand, sends, close) = {
            let mut st = lock(&shared.state);
            let keep = !st.over;
            if keep {
                st.socket = Some(socket.clone());
            }
            (
                keep,
                std::mem::take(&mut st.pending_demand),
                std::mem::take(&mut st.pending_sends),
                st.pending_close.take(),
            )
        };
        if !keep {
            for (_, done) in sends {
                done(Err(HttpError::Io("the WebSocket is closed".into())));
            }
            let (code, reason) = close.unwrap_or((1000, String::new()));
            socket.close(code, &reason);
            return shared;
        }
        if demand > 0 {
            socket.demand(demand);
        }
        for (message, done) in sends {
            socket.send(message, done);
        }
        if let Some((code, reason)) = close {
            socket.close(code, &reason);
        }
        shared
    }

    fn on_event(self: &Arc<Self>, event: WsEvent) {
        let mut notify = None;
        let mut opened = None;
        let mut failed_open = None;
        let mut grant = None;
        {
            let mut st = lock(&self.state);
            if st.over && !matches!(event, WsEvent::Closed { .. }) {
                return;
            }
            match event {
                WsEvent::Open { protocol } => {
                    st.open = true;
                    st.protocol = protocol;
                    opened = st.on_open.take();
                    grant = st.inbox.refill().map(|n| ws_demand(&mut st, n));
                }
                WsEvent::Message(message) => notify = st.inbox.push(message),
                WsEvent::Closed { code, reason } => {
                    if st.over {
                        return;
                    }
                    st.over = true;
                    st.socket = None;
                    if st.open {
                        st.inbox.queue.push_back(Message::Close { code, reason });
                        notify = st.inbox.finish(Ok(()));
                    } else {
                        failed_open = st.on_open.take().map(|f| {
                            (
                                f,
                                HttpError::Io(format!("closed during the handshake ({code})")),
                            )
                        });
                    }
                }
                WsEvent::Failed(error) => {
                    st.over = true;
                    st.socket = None;
                    if st.open {
                        notify = st.inbox.finish(Err(error));
                    } else {
                        failed_open = st.on_open.take().map(|f| (f, error));
                    }
                }
            }
        }
        if let Some(Some((socket, n))) = grant {
            socket.demand(n);
        }
        if let Some(on_open) = opened {
            on_open(Ok(WebSocket {
                shared: self.clone(),
                handed_off: false,
            }));
        }
        if let Some((on_open, error)) = failed_open {
            on_open(Err(error));
        }
        match notify {
            Some(Notify::Wake(w)) => w.wake(),
            Some(Notify::Drain) => self.drain(),
            None => {}
        }
    }

    fn socket(&self) -> Option<Arc<dyn Socket>> {
        lock(&self.state).socket.clone()
    }

    fn drain(self: &Arc<Self>) {
        loop {
            let mut grant = None;
            let (item, mut callback) = {
                let mut st = lock(&self.state);
                if st.inbox.draining {
                    return;
                }
                let Some(callback) = st.inbox.callback.take() else {
                    return;
                };
                let Some(item) = st.inbox.take() else {
                    st.inbox.callback = Some(callback);
                    return;
                };
                if let Some(n) = st.inbox.refill() {
                    grant = ws_demand(&mut st, n);
                }
                st.inbox.draining = true;
                (item, callback)
            };
            if let Some((socket, n)) = grant {
                socket.demand(n);
            }
            let terminal = !matches!(item, Some(Ok(_)));
            let keep = callback(item);
            let mut st = lock(&self.state);
            st.inbox.draining = false;
            if terminal {
                return;
            }
            if keep {
                st.inbox.callback = Some(callback);
            } else {
                let socket = st.socket.take();
                st.over = true;
                drop(st);
                drop(callback);
                if let Some(socket) = socket {
                    socket.close(1000, "");
                }
                return;
            }
        }
    }
}

fn ws_demand(st: &mut WsState, n: u32) -> Option<(Arc<dyn Socket>, u32)> {
    match &st.socket {
        Some(socket) => Some((socket.clone(), n)),
        None => {
            st.pending_demand += n;
            None
        }
    }
}

/// An open WebSocket. Read it with [`WebSocket::next`] or [`WebSocket::read_async`], and send
/// through it or a [`WsSender`]. Dropping it closes the connection with code 1000.
pub struct WebSocket {
    shared: Arc<WsShared>,
    handed_off: bool,
}

impl std::fmt::Debug for WebSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSocket")
            .field("protocol", &self.protocol())
            .finish_non_exhaustive()
    }
}

impl WebSocket {
    /// The subprotocol the server chose.
    pub fn protocol(&self) -> Option<String> {
        lock(&self.shared.state).protocol.clone()
    }

    /// A sending half to give to other code: it can send, ping and close.
    pub fn sender(&self) -> WsSender {
        WsSender {
            shared: self.shared.clone(),
        }
    }

    /// Send a message; `done` reports when the platform took it.
    pub fn send_async(
        &self,
        message: Message,
        done: impl FnOnce(Result<(), HttpError>) + Send + 'static,
    ) {
        self.sender().send_async(message, done)
    }

    /// Send a message and await the platform taking it.
    pub fn send_future(&self, message: Message) -> day_async::Oneshot<Result<(), HttpError>> {
        self.sender().send_future(message)
    }

    /// Close with `code` and `reason`.
    pub fn close(&self, code: u16, reason: &str) {
        self.sender().close(code, reason)
    }

    /// The next message; the last is a [`Message::Close`], then `None`.
    // Awaited (`socket.next().await`), so it is not `Iterator::next`.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> NextMessage<'_> {
        NextMessage { socket: self }
    }

    /// Hand each message to `on_message` as it arrives, then `None` at the end or `Some(Err)` on
    /// failure. Returning `false` closes the connection.
    pub fn read_async(
        mut self,
        on_message: impl FnMut(Option<Result<Message, HttpError>>) -> bool + Send + 'static,
    ) {
        self.handed_off = true;
        lock(&self.shared.state).inbox.callback = Some(Box::new(on_message));
        self.shared.drain();
    }
}

impl Drop for WebSocket {
    fn drop(&mut self) {
        if !self.handed_off {
            let socket = {
                let mut st = lock(&self.shared.state);
                st.over = true;
                st.socket.take()
            };
            if let Some(socket) = socket {
                socket.close(1000, "");
            }
        }
    }
}

/// The sending half of a [`WebSocket`].
#[derive(Clone)]
pub struct WsSender {
    shared: Arc<WsShared>,
}

impl std::fmt::Debug for WsSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WsSender")
    }
}

impl WsSender {
    /// Send a message; `done` reports when the platform took it.
    pub fn send_async(
        &self,
        message: Message,
        done: impl FnOnce(Result<(), HttpError>) + Send + 'static,
    ) {
        let socket = {
            let mut st = lock(&self.shared.state);
            if let Some(socket) = st.socket.clone() {
                Some(socket)
            } else if !st.over {
                st.pending_sends.push((message, Box::new(done)));
                return;
            } else {
                None
            }
        };
        match socket {
            Some(socket) => socket.send(message, Box::new(done)),
            None => done(Err(HttpError::Io("the WebSocket is closed".into()))),
        }
    }

    /// Send a message and await the platform taking it.
    pub fn send_future(&self, message: Message) -> day_async::Oneshot<Result<(), HttpError>> {
        let (tx, rx) = day_async::oneshot();
        self.send_async(message, move |r| tx.send(r));
        rx
    }

    /// Send a ping; `done` reports the pong. Needs [`Capabilities::websocket_ping`].
    pub fn ping_async(&self, done: impl FnOnce(Result<(), HttpError>) + Send + 'static) {
        match self.shared.socket() {
            Some(socket) => socket.ping(Box::new(done)),
            None => done(Err(HttpError::Io("the WebSocket is closed".into()))),
        }
    }

    /// Send a ping and await the pong.
    pub fn ping_future(&self) -> day_async::Oneshot<Result<(), HttpError>> {
        let (tx, rx) = day_async::oneshot();
        self.ping_async(move |r| tx.send(r));
        rx
    }

    /// Close with `code` and `reason`. The peer's reply arrives as the last message.
    pub fn close(&self, code: u16, reason: &str) {
        let socket = {
            let mut st = lock(&self.shared.state);
            if st.socket.is_none() && !st.over {
                st.pending_close = Some((code, reason.to_string()));
            }
            st.socket.clone()
        };
        if let Some(socket) = socket {
            socket.close(code, reason);
        }
    }
}

/// The future [`WebSocket::next`] returns.
pub struct NextMessage<'a> {
    socket: &'a mut WebSocket,
}

impl Future for NextMessage<'_> {
    type Output = Option<Result<Message, HttpError>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let shared = self.socket.shared.clone();
        let mut grant = None;
        let out = {
            let mut st = lock(&shared.state);
            let item = st.inbox.take();
            if let Some(n) = st.inbox.refill() {
                grant = ws_demand(&mut st, n);
            }
            match item {
                Some(item) => Poll::Ready(item),
                None => {
                    st.inbox.waker = Some(cx.waker().clone());
                    Poll::Pending
                }
            }
        };
        if let Some((socket, n)) = grant {
            socket.demand(n);
        }
        out
    }
}

/// The future [`Client::websocket_future`] returns.
pub struct Connecting {
    shared: Arc<WsShared>,
    rx: day_async::Oneshot<Result<WebSocket, HttpError>>,
    done: bool,
}

impl Future for Connecting {
    type Output = Result<WebSocket, HttpError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.rx).poll(cx) {
            Poll::Ready(result) => {
                self.done = true;
                Poll::Ready(result.unwrap_or(Err(HttpError::Cancelled)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for Connecting {
    fn drop(&mut self) {
        if !self.done {
            let socket = {
                let mut st = lock(&self.shared.state);
                st.over = true;
                st.on_open = None;
                st.socket.take()
            };
            if let Some(socket) = socket {
                socket.close(1000, "");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Cookies (RFC 6265)
// ---------------------------------------------------------------------------

/// A cookie, as a jar or the platform's store holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    /// `None` for a session cookie.
    pub expires: Option<SystemTime>,
    pub secure: bool,
    pub http_only: bool,
    /// Sent only to the host that set it, which named no `Domain`.
    pub host_only: bool,
}

/// A cookie store the client reads and writes itself: parsing, matching and expiry follow RFC
/// 6265, and a `Domain` that names a public suffix is refused.
#[derive(Debug, Default)]
pub struct CookieJar {
    cookies: Mutex<Vec<(Cookie, u64)>>,
    created: AtomicU64,
    file: Option<PathBuf>,
}

impl CookieJar {
    /// An empty jar in memory.
    pub fn new() -> CookieJar {
        CookieJar::default()
    }

    /// A jar persisted to `file`, loading what it holds. Session cookies are never written.
    pub fn open(file: impl Into<PathBuf>) -> CookieJar {
        let file = file.into();
        let jar = CookieJar {
            file: Some(file.clone()),
            ..CookieJar::default()
        };
        if let Ok(text) = std::fs::read_to_string(&file) {
            let now = now();
            let mut cookies = lock(&jar.cookies);
            for line in text.lines() {
                if let Some(cookie) = decode_cookie_line(line)
                    && cookie.expires.is_some_and(|e| e > now)
                {
                    let seq = jar.created.fetch_add(1, Ordering::Relaxed);
                    cookies.push((cookie, seq));
                }
            }
        }
        jar
    }

    /// Every unexpired cookie.
    pub fn cookies(&self) -> Vec<Cookie> {
        let now = now();
        lock(&self.cookies)
            .iter()
            .filter(|(c, _)| c.expires.is_none_or(|e| e > now))
            .map(|(c, _)| c.clone())
            .collect()
    }

    /// Forget every cookie.
    pub fn clear(&self) {
        lock(&self.cookies).clear();
        self.save();
    }

    /// Store the `Set-Cookie` values a response from `url` carried.
    pub fn store<'a>(&self, url: &str, set_cookie: impl IntoIterator<Item = &'a str>) {
        if let Ok(url) = url::Url::parse(url) {
            self.store_all(&url, set_cookie);
        }
    }

    /// The `Cookie` header for a request to `url`.
    pub fn header(&self, url: &str) -> Option<String> {
        url::Url::parse(url).ok().and_then(|u| self.header_for(&u))
    }

    fn store_all<'a>(&self, url: &url::Url, set_cookie: impl IntoIterator<Item = &'a str>) {
        let now = now();
        let mut changed = false;
        {
            let mut cookies = lock(&self.cookies);
            for value in set_cookie {
                let Some(cookie) = parse_set_cookie(url, value, now) else {
                    continue;
                };
                let existing = cookies.iter().position(|(c, _)| {
                    c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path
                });
                let seq = match existing {
                    Some(i) => cookies.remove(i).1,
                    None => self.created.fetch_add(1, Ordering::Relaxed),
                };
                if cookie.expires.is_none_or(|e| e > now) {
                    cookies.push((cookie, seq));
                }
                changed = true;
            }
            cookies.retain(|(c, _)| c.expires.is_none_or(|e| e > now));
        }
        if changed {
            self.save();
        }
    }

    fn header_for(&self, url: &url::Url) -> Option<String> {
        let host = url.host_str()?.to_ascii_lowercase();
        let secure = matches!(url.scheme(), "https" | "wss");
        let path = url.path();
        let now = now();
        let cookies = lock(&self.cookies);
        let mut matching: Vec<&(Cookie, u64)> = cookies
            .iter()
            .filter(|(c, _)| {
                c.expires.is_none_or(|e| e > now)
                    && (!c.secure || secure)
                    && if c.host_only {
                        c.domain == host
                    } else {
                        domain_matches(&host, &c.domain)
                    }
                    && path_matches(path, &c.path)
            })
            .collect();
        if matching.is_empty() {
            return None;
        }
        matching.sort_by(|(a, sa), (b, sb)| b.path.len().cmp(&a.path.len()).then(sa.cmp(sb)));
        Some(
            matching
                .iter()
                .map(|(c, _)| format!("{}={}", c.name, c.value))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }

    fn save(&self) {
        let Some(file) = &self.file else {
            return;
        };
        let mut text = String::new();
        for (cookie, _) in lock(&self.cookies).iter() {
            if let Some(line) = encode_cookie_line(cookie) {
                text.push_str(&line);
                text.push('\n');
            }
        }
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let partial = file.with_extension("partial");
        if std::fs::write(&partial, text).is_ok() {
            let _ = std::fs::rename(&partial, file);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn now() -> SystemTime {
    SystemTime::now()
}

/// The browser keeps its own cookies, and `SystemTime::now` does not exist on wasm32.
#[cfg(target_arch = "wasm32")]
fn now() -> SystemTime {
    UNIX_EPOCH
}

fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain
        || (host.len() > domain.len()
            && host.ends_with(domain)
            && host.as_bytes()[host.len() - domain.len() - 1] == b'.'
            && host.parse::<std::net::IpAddr>().is_err())
}

fn path_matches(request: &str, cookie: &str) -> bool {
    request == cookie
        || (request.starts_with(cookie)
            && (cookie.ends_with('/') || request.as_bytes().get(cookie.len()) == Some(&b'/')))
}

fn default_cookie_path(url: &url::Url) -> String {
    let path = url.path();
    match path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => path[..i].to_string(),
    }
}

fn parse_cookie_date(text: &str) -> Option<SystemTime> {
    httpdate::parse_http_date(text)
        .ok()
        .or_else(|| httpdate::parse_http_date(&text.replace('-', " ")).ok())
}

fn parse_set_cookie(url: &url::Url, header: &str, now: SystemTime) -> Option<Cookie> {
    let mut parts = header.split(';');
    let (name, value) = parts.next()?.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let mut cookie = Cookie {
        name: name.to_string(),
        value: value.trim().to_string(),
        domain: host.clone(),
        path: default_cookie_path(url),
        expires: None,
        secure: false,
        http_only: false,
        host_only: true,
    };
    let mut max_age = None;
    let mut expires = None;
    let mut domain = None;
    for attribute in parts {
        let (key, value) = match attribute.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => (attribute.trim(), ""),
        };
        match key.to_ascii_lowercase().as_str() {
            "expires" => expires = parse_cookie_date(value),
            "max-age" => max_age = value.parse::<i64>().ok(),
            "domain" => {
                let d = value.trim_start_matches('.').to_ascii_lowercase();
                if !d.is_empty() {
                    domain = Some(d);
                }
            }
            "path" if value.starts_with('/') => cookie.path = value.to_string(),
            "secure" => cookie.secure = true,
            "httponly" => cookie.http_only = true,
            _ => {}
        }
    }
    if let Some(domain) = domain {
        if domain != host && psl::suffix_str(&domain) == Some(domain.as_str()) {
            return None;
        }
        if !domain_matches(&host, &domain) {
            return None;
        }
        cookie.domain = domain;
        cookie.host_only = false;
    }
    cookie.expires = match max_age {
        Some(seconds) if seconds <= 0 => Some(UNIX_EPOCH),
        Some(seconds) => now.checked_add(Duration::from_secs(seconds as u64)),
        None => expires,
    };
    if cookie.secure && !matches!(url.scheme(), "https" | "wss") {
        return None;
    }
    Some(cookie)
}

fn encode_cookie_line(c: &Cookie) -> Option<String> {
    let expires = c.expires?.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let fields = [&c.name, &c.value, &c.domain, &c.path];
    if fields.iter().any(|f| f.contains(['\t', '\n', '\r'])) {
        return None;
    }
    let flags: String = [(c.secure, 's'), (c.http_only, 'h'), (c.host_only, 'o')]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, f)| *f)
        .collect();
    Some(format!(
        "{}\t{}\t{}\t{}\t{expires}\t{flags}",
        c.name, c.value, c.domain, c.path
    ))
}

fn decode_cookie_line(line: &str) -> Option<Cookie> {
    let mut fields = line.split('\t');
    let name = fields.next()?.to_string();
    let value = fields.next()?.to_string();
    let domain = fields.next()?.to_string();
    let path = fields.next()?.to_string();
    let expires = UNIX_EPOCH.checked_add(Duration::from_secs(fields.next()?.parse().ok()?))?;
    let flags = fields.next().unwrap_or("");
    Some(Cookie {
        name,
        value,
        domain,
        path,
        expires: Some(expires),
        secure: flags.contains('s'),
        http_only: flags.contains('h'),
        host_only: flags.contains('o'),
    })
}

// ---------------------------------------------------------------------------
// Transports
// ---------------------------------------------------------------------------

// The transport contract beneath [`crate::Client`] (docs/http.md "Transports").
//
// A transport performs one exchange: it sends a prepared request and reports what the platform
// stack saw, as [`Event`]s, in order. Policy (following a redirect, retrying a challenge,
// attaching cookies, deciding trust) belongs to the client above it, so every backend applies the
// same rules. Where a stack insists on deciding something itself (URLSession asks its delegate
// about redirects and challenges), the transport turns that into a [`Question`] and waits for the
// client's [`Answer`].
//
// Apps implement [`Transport`] only to install a test double through `ClientBuilder::transport`.

/// What a backend can do. The client consults it to decide what it must do itself, and apps can
/// read it (`day_part_http::capabilities()`) to say what a platform offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Response bodies arrive in chunks, pulled on demand.
    pub streaming: bool,
    /// A request body can be sent from a reader without buffering it first.
    pub upload_streaming: bool,
    /// Upload progress is reported while a body is sent.
    pub upload_progress: bool,
    /// The stack hands back redirect responses, so the client can follow and approve each hop.
    pub manual_redirects: bool,
    /// The stack asks about HTTP challenges itself ([`Question::Auth`]).
    pub auth_questions: bool,
    /// NTLM and Negotiate are answered by the stack.
    pub native_auth_schemes: bool,
    /// The stack reports each server certificate chain for evaluation ([`Question::ServerTrust`]).
    pub server_trust: bool,
    /// A client certificate identity can be presented.
    pub client_identity: bool,
    /// The platform's own cookie store is available.
    pub platform_cookies: bool,
    /// The platform's own HTTP cache is available.
    pub platform_cache: bool,
    /// Transfer metrics are reported.
    pub metrics: bool,
    /// WebSockets are available.
    pub websockets: bool,
    /// A WebSocket can send an explicit ping.
    pub websocket_ping: bool,
    /// A WebSocket handshake can carry custom request headers.
    pub websocket_headers: bool,
    /// Requests can wait for connectivity instead of failing at once.
    pub wait_for_connectivity: bool,
}

/// How a request may use the platform's cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CachePolicy {
    /// Follow the response's cache headers.
    #[default]
    Default,
    /// Always load from the network, ignoring cached data.
    Reload,
    /// Use cached data when present, whatever its age; load otherwise.
    PreferCache,
}

/// A request body reader shared between clones of a request. A stream can be sent once: a retry
/// or a 307 redirect that needs the body again finds it taken and fails.
#[derive(Clone)]
pub struct SharedReader(Arc<Mutex<Option<Box<dyn Read + Send>>>>);

impl SharedReader {
    /// Wrap a reader.
    pub fn new(reader: impl Read + Send + 'static) -> Self {
        Self(Arc::new(Mutex::new(Some(Box::new(reader)))))
    }

    /// Take the reader; `None` once it was taken.
    pub fn take(&self) -> Option<Box<dyn Read + Send>> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).take()
    }
}

impl std::fmt::Debug for SharedReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedReader")
    }
}

/// The body a transport sends.
#[derive(Debug)]
pub enum PreparedBody {
    Empty,
    Bytes(Arc<Vec<u8>>),
    File {
        path: PathBuf,
        len: u64,
    },
    Stream {
        reader: SharedReader,
        len: Option<u64>,
    },
}

/// One request, ready for a transport: headers merged, cookies and credentials attached.
#[derive(Debug)]
pub struct Prepared {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: PreparedBody,
    /// The idle bound: connecting, awaiting the head, and gaps in the body.
    pub timeout_idle: Duration,
    pub allow_expensive: bool,
    pub allow_constrained: bool,
    pub cache: CachePolicy,
    pub priority: Option<f32>,
    /// WebSocket subprotocols, in preference order.
    pub protocols: Vec<String>,
}

/// Per-client settings a transport applies when it is created.
#[derive(Clone, Debug)]
pub struct TransportConfig {
    pub timeout_idle: Duration,
    pub timeout_total: Option<Duration>,
    pub platform_cookies: bool,
    /// `(memory bytes, disk bytes)` for the platform cache; `None` = no cache.
    pub platform_cache: Option<(u64, u64)>,
    /// A stack with [`Capabilities::manual_redirects`] hands back every redirect whatever this
    /// says, and the client applies it; the others follow natively up to the limit, or refuse.
    pub redirects: Redirects,
    /// The client wants challenges as questions.
    pub ask_auth: bool,
    /// The client evaluates server trust (pins or a handler).
    pub ask_trust: bool,
    pub identity: Option<Identity>,
    pub max_per_host: Option<u32>,
    pub wait_for_connectivity: bool,
}

/// The response head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Head {
    pub status: u16,
    /// In arrival order, duplicates preserved.
    pub headers: Vec<(String, String)>,
    /// The URL that answered, after any redirects the stack followed itself.
    pub url: String,
    /// `Content-Length`, when the response declared one.
    pub expected_length: Option<u64>,
}

impl Head {
    /// The first header with this name, ASCII case-insensitive.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Every header with this name, in order.
    pub fn headers_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.headers
            .iter()
            .filter(move |(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Timings and facts about one completed transfer. Every field a platform does not report is
/// `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metrics {
    /// Resolving the host name.
    pub dns: Option<Duration>,
    /// Establishing the connection, TLS included.
    pub connect: Option<Duration>,
    /// The TLS handshake.
    pub tls: Option<Duration>,
    /// From the start of the request to its first response byte.
    pub first_byte: Option<Duration>,
    /// From the start of the request to its last response byte.
    pub total: Option<Duration>,
    /// `http/1.1`, `h2`, `h3`.
    pub protocol: Option<String>,
    pub reused_connection: Option<bool>,
    pub proxy: Option<bool>,
    pub remote_address: Option<String>,
    pub tls_version: Option<String>,
    /// The response came from the local cache.
    pub from_cache: bool,
    pub bytes_sent: Option<u64>,
    pub bytes_received: Option<u64>,
    /// Redirects followed on the way to this response.
    pub redirects: u32,
}

/// An HTTP authentication scheme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scheme {
    Basic,
    Bearer,
    Digest,
    Ntlm,
    Negotiate,
    Other(String),
}

impl Scheme {
    pub(crate) fn parse(token: &str) -> Scheme {
        match token.to_ascii_lowercase().as_str() {
            "basic" => Scheme::Basic,
            "bearer" => Scheme::Bearer,
            "digest" => Scheme::Digest,
            "ntlm" => Scheme::Ntlm,
            "negotiate" => Scheme::Negotiate,
            _ => Scheme::Other(token.to_string()),
        }
    }

    /// Schemes a stack with native challenge handling (URLSession) raises as questions itself.
    pub(crate) fn raised_natively(&self) -> bool {
        matches!(
            self,
            Scheme::Basic | Scheme::Digest | Scheme::Ntlm | Scheme::Negotiate
        )
    }
}

/// The certificate chain a server presented, with the platform's own verdict on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerTrust {
    pub host: String,
    /// DER certificates, leaf first.
    pub chain: Vec<Vec<u8>>,
    /// The platform's trust evaluation passed.
    pub system_trusted: bool,
    /// Why it failed, when it did.
    pub system_error: Option<String>,
}

impl ServerTrust {
    /// Each certificate's public-key pin, `sha256/<base64>`, leaf first — the value
    /// `Trust::pin` takes.
    pub fn pins(&self) -> Vec<String> {
        self.chain
            .iter()
            .filter_map(|der| spki_sha256(der))
            .map(|h| pin_string(&h))
            .collect()
    }
}

/// A challenge a stack raised itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthQuestion {
    pub host: String,
    pub port: u16,
    pub realm: Option<String>,
    pub scheme: Scheme,
    pub proxy: bool,
    pub previous_failures: u32,
}

/// Something the stack waits on the client for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Question {
    Auth(AuthQuestion),
    ServerTrust(ServerTrust),
}

/// The identifier a transport pairs a question with its answer by.
pub type QuestionId = u64;

/// The client's answer to a [`Question`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Credential {
        user: String,
        password: String,
    },
    /// Let the stack decide: for a challenge, deliver the 401; for trust, the platform verdict.
    DefaultHandling,
    /// Abandon the request.
    Cancel,
    /// Trust the server despite the platform verdict.
    Accept,
    /// Refuse the server.
    Reject,
}

/// What a transport reports about one exchange, in order.
#[derive(Debug)]
pub enum Event {
    Head(Head),
    Chunk(Vec<u8>),
    Sent { sent: u64, total: Option<u64> },
    Question(QuestionId, Question),
    Metrics(Metrics),
    End,
    Failed(HttpError),
}

/// The sink a transport reports through.
pub type Events = Arc<dyn Fn(Event) + Send + Sync>;

/// The client's grip on one exchange.
pub trait Transfer: Send + Sync {
    /// Read up to `chunks` more body chunks.
    fn demand(&self, chunks: u32);
    /// Answer a question the transport raised.
    fn answer(&self, id: QuestionId, answer: Answer);
    /// Abandon the exchange. A transport still reports a terminal event afterwards.
    fn cancel(&self);
}

/// A WebSocket message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Text(String),
    Binary(Vec<u8>),
    /// The peer closed the connection; nothing follows.
    Close {
        code: u16,
        reason: String,
    },
}

/// What a transport reports about one WebSocket.
#[derive(Debug)]
pub enum WsEvent {
    Open { protocol: Option<String> },
    Message(Message),
    Closed { code: u16, reason: String },
    Failed(HttpError),
}

/// The sink a WebSocket transport reports through.
pub type WsEvents = Arc<dyn Fn(WsEvent) + Send + Sync>;

/// A completion callback.
pub type Completion = Box<dyn FnOnce(Result<(), HttpError>) + Send>;

/// The client's grip on one WebSocket.
pub trait Socket: Send + Sync {
    fn send(&self, message: Message, done: Completion);
    fn ping(&self, done: Completion);
    fn close(&self, code: u16, reason: &str);
    /// Receive up to `messages` more messages.
    fn demand(&self, messages: u32);
}

/// One backend.
pub trait Transport: Send + Sync + 'static {
    fn capabilities(&self) -> Capabilities;
    /// Start an exchange. Events may arrive before this returns.
    fn start(&self, request: Prepared, events: Events) -> Arc<dyn Transfer>;
    /// Open a WebSocket.
    fn websocket(&self, request: Prepared, events: WsEvents) -> Arc<dyn Socket> {
        let _ = request;
        events(WsEvent::Failed(HttpError::Unsupported));
        Arc::new(NoSocket)
    }
    /// The platform store's cookies, for [`Cookies::Platform`].
    fn cookies(&self) -> Vec<Cookie> {
        Vec::new()
    }
    /// Forget the platform store's cookies.
    fn clear_cookies(&self) {}
    /// Empty the platform cache.
    fn clear_cache(&self) {}
}

struct NoSocket;

impl Socket for NoSocket {
    fn send(&self, _message: Message, done: Completion) {
        done(Err(HttpError::Unsupported));
    }
    fn ping(&self, done: Completion) {
        done(Err(HttpError::Unsupported));
    }
    fn close(&self, _code: u16, _reason: &str) {}
    fn demand(&self, _messages: u32) {}
}

// ---------------------------------------------------------------------------
// Pins
// ---------------------------------------------------------------------------

// Public-key pins (docs/http.md "Trust"): the SHA-256 of a certificate's SubjectPublicKeyInfo,
// written `sha256/<base64>`. Pinning the key rather than the certificate survives a renewal that
// keeps the key. The DER walk here reads exactly as far as the SPKI and no further.

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Standard base64, padding optional, whitespace ignored. `None` on any other character.
pub(crate) fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut acc = 0u32;
    let mut bits = 0u32;
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(digest.as_slice());
    out
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// One DER element: `(tag, content, the whole element, what follows it)`.
type Tlv<'a> = (u8, &'a [u8], &'a [u8], &'a [u8]);

/// Read the DER element at the start of `data`.
fn read_tlv(data: &[u8]) -> Option<Tlv<'_>> {
    let tag = *data.first()?;
    let first = *data.get(1)?;
    let (len, header) = if first & 0x80 == 0 {
        (usize::from(first), 2)
    } else {
        let n = usize::from(first & 0x7f);
        if n == 0 || n > 4 {
            return None;
        }
        let mut len = 0usize;
        for i in 0..n {
            len = (len << 8) | usize::from(*data.get(2 + i)?);
        }
        (len, 2 + n)
    };
    let end = header.checked_add(len)?;
    if data.len() < end {
        return None;
    }
    Some((tag, &data[header..end], &data[..end], &data[end..]))
}

/// The SubjectPublicKeyInfo element of a DER certificate.
///
/// `Certificate ::= SEQUENCE { tbsCertificate, signatureAlgorithm, signature }` and
/// `TBSCertificate ::= SEQUENCE { [0] version OPTIONAL, serialNumber, signature, issuer, validity,
/// subject, subjectPublicKeyInfo, … }`.
pub(crate) fn spki(cert: &[u8]) -> Option<&[u8]> {
    const SEQUENCE: u8 = 0x30;
    let (tag, certificate, _, _) = read_tlv(cert)?;
    if tag != SEQUENCE {
        return None;
    }
    let (tag, tbs, _, _) = read_tlv(certificate)?;
    if tag != SEQUENCE {
        return None;
    }
    let mut rest = tbs;
    let (tag, _, _, after) = read_tlv(rest)?;
    if tag == 0xa0 {
        rest = after;
    }
    for _ in 0..5 {
        let (_, _, _, after) = read_tlv(rest)?;
        rest = after;
    }
    let (tag, _, whole, _) = read_tlv(rest)?;
    (tag == SEQUENCE).then_some(whole)
}

/// The SHA-256 of a certificate's SPKI.
pub(crate) fn spki_sha256(cert: &[u8]) -> Option<[u8; 32]> {
    spki(cert).map(sha256)
}

/// `sha256/<base64>`.
pub(crate) fn pin_string(hash: &[u8; 32]) -> String {
    format!("sha256/{}", base64_encode(hash))
}

/// Parse `sha256/<base64>` (the prefix optional).
pub(crate) fn parse_pin(pin: &str) -> Option<[u8; 32]> {
    let text = pin.trim();
    let text = text.strip_prefix("sha256/").unwrap_or(text);
    let bytes = base64_decode(text)?;
    <[u8; 32]>::try_from(bytes.as_slice()).ok()
}

#[cfg(test)]
mod pin_tests {
    use super::*;

    fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        if content.len() < 0x80 {
            out.push(content.len() as u8);
        } else {
            out.push(0x82);
            out.extend_from_slice(&(content.len() as u16).to_be_bytes());
        }
        out.extend_from_slice(content);
        out
    }

    #[test]
    fn base64_round_trips() {
        // RFC 4648's vectors: each prefix of "foobar".
        let full = b"foobar";
        let expected = [
            "", "Zg==", "Zm8=", "Zm9v", "Zm9vYg==", "Zm9vYmE=", "Zm9vYmFy",
        ];
        for (n, want) in expected.iter().enumerate() {
            let sample = &full[..n];
            assert_eq!(base64_encode(sample), *want);
            assert_eq!(base64_decode(want).as_deref(), Some(sample));
        }
    }

    #[test]
    fn the_spki_is_found_after_the_optional_version() {
        let spki_elem = tlv(0x30, &[0x30, 0x00, 0x03, 0x02, 0x00, 0xAA]);
        let mut tbs = Vec::new();
        tbs.extend(tlv(0xa0, &tlv(0x02, &[2])));
        tbs.extend(tlv(0x02, &[1]));
        for _ in 0..4 {
            tbs.extend(tlv(0x30, &[]));
        }
        tbs.extend(spki_elem.clone());
        tbs.extend(tlv(0xa3, &[0x30, 0x00]));
        let mut cert = tlv(0x30, &tbs);
        cert.extend(tlv(0x30, &[]));
        let cert = tlv(0x30, &cert);
        assert_eq!(spki(&cert), Some(spki_elem.as_slice()));
        let pin = pin_string(&spki_sha256(&cert).unwrap());
        assert_eq!(parse_pin(&pin), Some(sha256(&spki_elem)));
    }

    #[test]
    fn a_malformed_certificate_has_no_spki() {
        assert_eq!(spki(&[0x30, 0x05, 0x30]), None);
        assert_eq!(spki(&[]), None);
    }
}

// ---------------------------------------------------------------------------
// Challenges
// ---------------------------------------------------------------------------

// HTTP authentication the client performs itself (docs/http.md "Challenges"): parsing
// `WWW-Authenticate` and `Proxy-Authenticate`, and writing Basic, Bearer and Digest credentials.
// NTLM and Negotiate are handshakes only an OS stack performs; those arrive as questions from a
// transport that can answer them.

/// One challenge: its scheme and parameters (names lowercased).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParsedChallenge {
    pub scheme: Scheme,
    pub params: Vec<(String, String)>,
}

impl ParsedChallenge {
    pub(crate) fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Every challenge in the response's `WWW-Authenticate` (or `Proxy-Authenticate`) headers, in
/// order.
pub(crate) fn parse_challenges(headers: &[(String, String)], proxy: bool) -> Vec<ParsedChallenge> {
    let name = if proxy {
        "proxy-authenticate"
    } else {
        "www-authenticate"
    };
    let mut out = Vec::new();
    for (_, value) in headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case(name)) {
        parse_value(value, &mut out);
    }
    out
}

fn parse_value(value: &str, out: &mut Vec<ParsedChallenge>) {
    for item in split_outside_quotes(value) {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let (first, rest) = match item.find(char::is_whitespace) {
            Some(i) => (&item[..i], item[i..].trim()),
            None => (item, ""),
        };
        if first.contains('=') {
            push_param(out, item);
        } else {
            out.push(ParsedChallenge {
                scheme: Scheme::parse(first),
                params: Vec::new(),
            });
            if !rest.is_empty() {
                push_param(out, rest);
            }
        }
    }
}

fn push_param(out: &mut [ParsedChallenge], text: &str) {
    let Some(last) = out.last_mut() else {
        return;
    };
    match text.split_once('=') {
        Some((k, v)) if !k.trim().is_empty() => last
            .params
            .push((k.trim().to_ascii_lowercase(), unquote(v.trim()))),
        _ => last.params.push(("token68".into(), text.to_string())),
    }
}

fn split_outside_quotes(value: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for c in value.chars() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' if quoted => {
                current.push(c);
                escaped = true;
            }
            '"' => {
                quoted = !quoted;
                current.push(c);
            }
            ',' if !quoted => items.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    items.push(current);
    items
}

fn unquote(value: &str) -> String {
    let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
        return value.to_string();
    };
    let mut out = String::with_capacity(inner.len());
    let mut escaped = false;
    for c in inner.chars() {
        if escaped {
            out.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else {
            out.push(c);
        }
    }
    out
}

/// `Basic <base64(user:password)>`.
pub(crate) fn basic_authorization(user: &str, password: &str) -> String {
    format!(
        "Basic {}",
        base64_encode(format!("{user}:{password}").as_bytes())
    )
}

/// A fresh client nonce.
fn digest_cnonce() -> String {
    static CNONCES: AtomicU64 = AtomicU64::new(1);
    let n = CNONCES.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    hex(&sha256(format!("{n}:{now}").as_bytes())[..8])
}

fn md5_hex(text: &str) -> String {
    use md5::{Digest, Md5};
    let digest = Md5::digest(text.as_bytes());
    hex(digest.as_slice())
}

/// The `Authorization` value answering a Digest challenge (RFC 7616, `qop=auth`), or `None` when
/// the challenge lacks what it needs or names an algorithm this does not implement.
pub(crate) fn digest_authorization(
    challenge: &ParsedChallenge,
    user: &str,
    password: &str,
    method: &str,
    uri: &str,
) -> Option<String> {
    let realm = challenge.param("realm")?;
    let nonce = challenge.param("nonce")?;
    let algorithm = challenge.param("algorithm").unwrap_or("MD5");
    let hash = |text: String| -> Option<String> {
        match algorithm.to_ascii_uppercase().as_str() {
            "MD5" => Some(md5_hex(&text)),
            "SHA-256" => Some(hex(&sha256(text.as_bytes()))),
            _ => None,
        }
    };
    let ha1 = hash(format!("{user}:{realm}:{password}"))?;
    let ha2 = hash(format!("{method}:{uri}"))?;
    let auth_qop = challenge
        .param("qop")
        .is_some_and(|q| q.split(',').any(|t| t.trim() == "auth"));
    let mut value = format!(
        "Digest username=\"{user}\", realm=\"{realm}\", nonce=\"{nonce}\", uri=\"{uri}\", \
         algorithm={algorithm}"
    );
    if auth_qop {
        let nc = "00000001";
        let cnonce = digest_cnonce();
        let response = hash(format!("{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}"))?;
        value.push_str(&format!(
            ", response=\"{response}\", qop=auth, nc={nc}, cnonce=\"{cnonce}\""
        ));
    } else {
        let response = hash(format!("{ha1}:{nonce}:{ha2}"))?;
        value.push_str(&format!(", response=\"{response}\""));
    }
    if let Some(opaque) = challenge.param("opaque") {
        value.push_str(&format!(", opaque=\"{opaque}\""));
    }
    Some(value)
}

/// Check a Digest `Authorization` value against the expected credentials — what the test server
/// verifies with.
// The test server, its one caller, is native-only.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn verify_digest(authorization: &str, user: &str, password: &str, method: &str) -> bool {
    let Some(rest) = authorization.strip_prefix("Digest ") else {
        return false;
    };
    let mut parsed = Vec::new();
    parse_value(&format!("Digest {rest}"), &mut parsed);
    let Some(p) = parsed.first() else {
        return false;
    };
    let (Some(realm), Some(nonce), Some(uri), Some(response)) = (
        p.param("realm"),
        p.param("nonce"),
        p.param("uri"),
        p.param("response"),
    ) else {
        return false;
    };
    if p.param("username") != Some(user) {
        return false;
    }
    let ha1 = md5_hex(&format!("{user}:{realm}:{password}"));
    let ha2 = md5_hex(&format!("{method}:{uri}"));
    let expected = match (p.param("qop"), p.param("nc"), p.param("cnonce")) {
        (Some(qop), Some(nc), Some(cnonce)) => {
            md5_hex(&format!("{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}"))
        }
        _ => md5_hex(&format!("{ha1}:{nonce}:{ha2}")),
    };
    expected == response
}

#[cfg(test)]
mod auth_tests {
    use super::*;

    fn headers(values: &[&str]) -> Vec<(String, String)> {
        values
            .iter()
            .map(|v| ("WWW-Authenticate".to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn parses_several_challenges_with_quoted_commas() {
        let parsed = parse_challenges(
            &headers(&[
                r#"Basic realm="Day, test", charset="UTF-8""#,
                r#"Bearer realm="api", error="invalid_token""#,
            ]),
            false,
        );
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].scheme, Scheme::Basic);
        assert_eq!(parsed[0].param("realm"), Some("Day, test"));
        assert_eq!(parsed[0].param("charset"), Some("UTF-8"));
        assert_eq!(parsed[1].scheme, Scheme::Bearer);
        assert_eq!(parsed[1].param("error"), Some("invalid_token"));
    }

    #[test]
    fn parses_two_challenges_in_one_header() {
        let parsed = parse_challenges(
            &headers(&[r#"Negotiate, Digest realm="r", nonce="n", qop="auth""#]),
            false,
        );
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].scheme, Scheme::Negotiate);
        assert_eq!(parsed[1].scheme, Scheme::Digest);
        assert_eq!(parsed[1].param("nonce"), Some("n"));
    }

    #[test]
    fn basic_encodes_the_pair() {
        assert_eq!(
            basic_authorization("Aladdin", "open sesame"),
            "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
    }

    #[test]
    fn a_digest_answer_verifies() {
        let challenge = parse_challenges(
            &headers(&[r#"Digest realm="Day test", nonce="abc", qop="auth", opaque="xyz""#]),
            false,
        )
        .remove(0);
        let value = digest_authorization(
            &challenge,
            "day",
            "sunrise",
            "GET",
            "/digest-auth/day/sunrise",
        )
        .expect("answer");
        assert!(value.contains("opaque=\"xyz\""), "{value}");
        assert!(verify_digest(&value, "day", "sunrise", "GET"));
        assert!(!verify_digest(&value, "day", "sunset", "GET"));
    }
}

// ---------------------------------------------------------------------------
// Multipart forms
// ---------------------------------------------------------------------------

// `multipart/form-data` bodies (docs/http.md "Uploads"), encoded as a stream: a file part is read
// from disk while it is sent, so a form carrying a large file never sits in memory.

#[derive(Clone, Debug)]
enum PartBody {
    Bytes(Vec<u8>),
    File(PathBuf),
}

#[derive(Clone, Debug)]
struct Part {
    name: String,
    filename: Option<String>,
    content_type: Option<String>,
    body: PartBody,
}

/// A multipart form. Build it, then hand it to `Request::form`.
#[derive(Clone, Debug)]
pub struct Form {
    boundary: String,
    parts: Vec<Part>,
}

impl Default for Form {
    fn default() -> Self {
        Self::new()
    }
}

impl Form {
    /// An empty form with a fresh boundary.
    pub fn new() -> Self {
        static BOUNDARIES: AtomicU64 = AtomicU64::new(1);
        let n = BOUNDARIES.fetch_add(1, Ordering::Relaxed);
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let hash = sha256(format!("{n}:{seed}").as_bytes());
        Self {
            boundary: format!("day-{}", hex(&hash[..12])),
            parts: Vec::new(),
        }
    }

    /// A text field.
    pub fn text(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.parts.push(Part {
            name: name.into(),
            filename: None,
            content_type: None,
            body: PartBody::Bytes(value.into().into_bytes()),
        });
        self
    }

    /// A file part from bytes in memory.
    pub fn bytes(
        mut self,
        name: impl Into<String>,
        filename: impl Into<String>,
        content_type: impl Into<String>,
        bytes: Vec<u8>,
    ) -> Self {
        self.parts.push(Part {
            name: name.into(),
            filename: Some(filename.into()),
            content_type: Some(content_type.into()),
            body: PartBody::Bytes(bytes),
        });
        self
    }

    /// A file part read from disk while the form is sent.
    pub fn file(
        mut self,
        name: impl Into<String>,
        path: impl Into<PathBuf>,
        content_type: impl Into<String>,
    ) -> Self {
        let path = path.into();
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());
        self.parts.push(Part {
            name: name.into(),
            filename: Some(filename),
            content_type: Some(content_type.into()),
            body: PartBody::File(path),
        });
        self
    }

    /// The `Content-Type` header value, boundary included.
    pub fn content_type(&self) -> String {
        format!("multipart/form-data; boundary={}", self.boundary)
    }

    fn part_header(&self, part: &Part) -> Vec<u8> {
        let escape = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let mut head = format!(
            "--{}\r\nContent-Disposition: form-data; name=\"{}\"",
            self.boundary,
            escape(&part.name)
        );
        if let Some(filename) = &part.filename {
            head.push_str(&format!("; filename=\"{}\"", escape(filename)));
        }
        head.push_str("\r\n");
        if let Some(ct) = &part.content_type {
            head.push_str(&format!("Content-Type: {ct}\r\n"));
        }
        head.push_str("\r\n");
        head.into_bytes()
    }

    /// The encoded body as a reader, with its exact length.
    pub(crate) fn reader(&self) -> std::io::Result<(Box<dyn Read + Send>, u64)> {
        let mut pieces: VecDeque<Box<dyn Read + Send>> = VecDeque::new();
        let mut len = 0u64;
        for part in &self.parts {
            let head = self.part_header(part);
            len += head.len() as u64;
            pieces.push_back(Box::new(Cursor::new(head)));
            match &part.body {
                PartBody::Bytes(bytes) => {
                    len += bytes.len() as u64;
                    pieces.push_back(Box::new(Cursor::new(bytes.clone())));
                }
                PartBody::File(path) => {
                    let file = std::fs::File::open(path)?;
                    len += file.metadata()?.len();
                    pieces.push_back(Box::new(file));
                }
            }
            len += 2;
            pieces.push_back(Box::new(Cursor::new(b"\r\n".to_vec())));
        }
        let tail = format!("--{}--\r\n", self.boundary).into_bytes();
        len += tail.len() as u64;
        pieces.push_back(Box::new(Cursor::new(tail)));
        Ok((Box::new(Chain { pieces }), len))
    }
}

struct Chain {
    pieces: VecDeque<Box<dyn Read + Send>>,
}

impl Read for Chain {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while let Some(front) = self.pieces.front_mut() {
            let n = front.read(buf)?;
            if n > 0 {
                return Ok(n);
            }
            self.pieces.pop_front();
        }
        Ok(0)
    }
}

#[cfg(test)]
mod form_tests {
    use super::*;

    #[test]
    fn the_declared_length_is_the_encoded_length() {
        let form = Form::new().text("title", "Sunrise").bytes(
            "photo",
            "day.png",
            "image/png",
            vec![1, 2, 3, 4],
        );
        let (mut reader, len) = form.reader().expect("reader");
        let mut out = Vec::new();
        reader.read_to_end(&mut out).expect("read");
        assert_eq!(out.len() as u64, len);
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("name=\"title\"\r\n\r\nSunrise\r\n"), "{text}");
        assert!(
            text.contains("filename=\"day.png\"\r\nContent-Type: image/png"),
            "{text}"
        );
        assert!(text.ends_with(&format!("--{}--\r\n", form.boundary)));
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod client_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[derive(Clone, Debug)]
    struct Seen {
        method: String,
        url: String,
        headers: Vec<(String, String)>,
    }

    impl Seen {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
    }

    #[derive(Default)]
    struct Reply {
        status: u16,
        headers: Vec<(String, String)>,
        chunks: Vec<Vec<u8>>,
        question: Option<Question>,
        stall: bool,
    }

    impl Reply {
        fn new(status: u16) -> Reply {
            Reply {
                status,
                ..Reply::default()
            }
        }
        fn header(mut self, name: &str, value: &str) -> Reply {
            self.headers.push((name.into(), value.into()));
            self
        }
        fn body(mut self, text: &str) -> Reply {
            self.chunks = vec![text.as_bytes().to_vec()];
            self
        }
        fn chunks(mut self, n: u8) -> Reply {
            self.chunks = (0..n).map(|i| vec![i]).collect();
            self
        }
        fn question(mut self, question: Question) -> Reply {
            self.question = Some(question);
            self
        }
        fn stall(mut self) -> Reply {
            self.stall = true;
            self
        }
    }

    type Script = dyn Fn(&Seen) -> Reply + Send + Sync;

    /// A transport that answers each exchange from a script, pulled by demand.
    #[derive(Clone)]
    struct Scripted {
        caps: Capabilities,
        script: Arc<Script>,
        seen: Arc<Mutex<Vec<Seen>>>,
        granted: Arc<AtomicU64>,
        cancels: Arc<AtomicUsize>,
        answers: Arc<Mutex<Vec<Answer>>>,
    }

    impl Scripted {
        fn new(script: impl Fn(&Seen) -> Reply + Send + Sync + 'static) -> Scripted {
            Scripted {
                caps: Capabilities {
                    streaming: true,
                    manual_redirects: true,
                    server_trust: true,
                    websockets: true,
                    ..Capabilities::default()
                },
                script: Arc::new(script),
                seen: Arc::default(),
                granted: Arc::default(),
                cancels: Arc::default(),
                answers: Arc::default(),
            }
        }

        fn seen(&self) -> Vec<Seen> {
            lock(&self.seen).clone()
        }
    }

    struct ExchangeState {
        events: Option<Events>,
        head: Option<Head>,
        chunks: VecDeque<Vec<u8>>,
        demand: u32,
        waiting: bool,
        emitting: bool,
        /// Emits nothing but keeps its events until cancelled, like a real stalled exchange; that
        /// hold is what keeps a callback-started flight alive.
        stalled: bool,
    }

    struct ScriptedTransfer {
        owner: Scripted,
        state: Mutex<ExchangeState>,
    }

    impl ScriptedTransfer {
        fn pump(&self) {
            loop {
                let (events, event) = {
                    let mut st = lock(&self.state);
                    if st.emitting || st.waiting || st.stalled {
                        return;
                    }
                    let Some(events) = st.events.clone() else {
                        return;
                    };
                    let event = if let Some(head) = st.head.take() {
                        Event::Head(head)
                    } else if st.chunks.is_empty() {
                        st.events = None;
                        Event::End
                    } else if st.demand == 0 {
                        return;
                    } else {
                        st.demand -= 1;
                        Event::Chunk(st.chunks.pop_front().unwrap_or_default())
                    };
                    st.emitting = true;
                    (events, event)
                };
                events(event);
                lock(&self.state).emitting = false;
            }
        }
    }

    impl Transfer for ScriptedTransfer {
        fn demand(&self, chunks: u32) {
            self.owner
                .granted
                .fetch_add(u64::from(chunks), Ordering::SeqCst);
            lock(&self.state).demand += chunks;
            self.pump();
        }

        fn answer(&self, _id: QuestionId, answer: Answer) {
            lock(&self.owner.answers).push(answer.clone());
            let refused = {
                let mut st = lock(&self.state);
                st.waiting = false;
                if matches!(answer, Answer::Reject | Answer::Cancel) {
                    st.events.take()
                } else {
                    None
                }
            };
            match refused {
                Some(events) => events(Event::Failed(HttpError::Tls("refused".into()))),
                None => self.pump(),
            }
        }

        fn cancel(&self) {
            self.owner.cancels.fetch_add(1, Ordering::SeqCst);
            let events = lock(&self.state).events.take();
            if let Some(events) = events {
                events(Event::Failed(HttpError::Cancelled));
            }
        }
    }

    struct EchoState {
        events: Option<WsEvents>,
        queue: VecDeque<Message>,
        demand: u32,
    }

    struct EchoSocket(Mutex<EchoState>);

    impl EchoSocket {
        fn pump(&self) {
            loop {
                let (events, message) = {
                    let mut st = lock(&self.0);
                    if st.demand == 0 || st.queue.is_empty() {
                        return;
                    }
                    let Some(events) = st.events.clone() else {
                        return;
                    };
                    st.demand -= 1;
                    (events, st.queue.pop_front())
                };
                if let Some(message) = message {
                    events(WsEvent::Message(message));
                }
            }
        }
    }

    impl Socket for EchoSocket {
        fn send(&self, message: Message, done: Completion) {
            lock(&self.0).queue.push_back(message);
            done(Ok(()));
            self.pump();
        }
        fn ping(&self, done: Completion) {
            done(Ok(()));
        }
        fn close(&self, code: u16, reason: &str) {
            let events = lock(&self.0).events.take();
            if let Some(events) = events {
                events(WsEvent::Closed {
                    code,
                    reason: reason.to_string(),
                });
            }
        }
        fn demand(&self, messages: u32) {
            lock(&self.0).demand += messages;
            self.pump();
        }
    }

    impl Transport for Scripted {
        fn capabilities(&self) -> Capabilities {
            self.caps
        }

        fn start(&self, request: Prepared, events: Events) -> Arc<dyn Transfer> {
            let seen = Seen {
                method: request.method.clone(),
                url: request.url.clone(),
                headers: request.headers.clone(),
            };
            lock(&self.seen).push(seen.clone());
            let reply = (self.script)(&seen);
            let expected = reply.chunks.iter().map(|c| c.len() as u64).sum();
            let transfer = Arc::new(ScriptedTransfer {
                owner: self.clone(),
                state: Mutex::new(ExchangeState {
                    events: Some(events.clone()),
                    head: Some(Head {
                        status: reply.status,
                        headers: reply.headers,
                        url: request.url,
                        expected_length: Some(expected),
                    }),
                    chunks: reply.chunks.into(),
                    demand: 0,
                    waiting: reply.question.is_some(),
                    emitting: false,
                    stalled: reply.stall,
                }),
            });
            if let Some(question) = reply.question {
                events(Event::Question(7, question));
            }
            transfer.pump();
            transfer
        }

        fn websocket(&self, request: Prepared, events: WsEvents) -> Arc<dyn Socket> {
            let socket = Arc::new(EchoSocket(Mutex::new(EchoState {
                events: Some(events.clone()),
                queue: VecDeque::new(),
                demand: 0,
            })));
            events(WsEvent::Open {
                protocol: request.protocols.first().cloned(),
            });
            socket
        }
    }

    fn builder(t: &Scripted) -> ClientBuilder {
        Client::builder()
            .transport(t.clone())
            .cookies(Cookies::jar())
    }

    fn der(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut out = vec![tag, content.len() as u8];
        out.extend_from_slice(content);
        out
    }

    fn fake_certificate(key: u8) -> Vec<u8> {
        let mut tbs = der(0x02, &[1]);
        for _ in 0..4 {
            tbs.extend(der(0x30, &[]));
        }
        tbs.extend(der(0x30, &[0x03, 0x02, 0x00, key]));
        der(0x30, &der(0x30, &tbs))
    }

    #[test]
    fn a_303_is_followed_as_a_get() {
        let t = Scripted::new(|seen| match seen.url.as_str() {
            "http://day.test/a" => Reply::new(303).header("Location", "/b"),
            _ => Reply::new(200).body("done"),
        });
        let client = builder(&t).build();
        let resp = wait(client.fetch_future(Request::post("http://day.test/a", b"x".to_vec())))
            .expect("response");
        assert_eq!(resp.text(), "done");
        assert_eq!(resp.url, "http://day.test/b");
        let methods: Vec<String> = t.seen().into_iter().map(|s| s.method).collect();
        assert_eq!(methods, ["POST", "GET"]);
    }

    #[test]
    fn credentials_and_cookies_stay_behind_on_another_origin() {
        let t = Scripted::new(|seen| match seen.url.as_str() {
            "http://one.test/" => Reply::new(302).header("Location", "http://two.test/"),
            _ => Reply::new(200),
        });
        let jar = Arc::new(CookieJar::new());
        jar.store("http://one.test/", ["id=1"]);
        let client = Client::builder()
            .transport(t.clone())
            .cookies(Cookies::Jar(jar))
            .build();
        wait(client.fetch_future(Request::get("http://one.test/").bearer("secret")))
            .expect("response");
        let seen = t.seen();
        assert_eq!(seen[0].header("authorization"), Some("Bearer secret"));
        assert_eq!(seen[0].header("cookie"), Some("id=1"));
        assert_eq!(seen[1].header("authorization"), None);
        assert_eq!(seen[1].header("cookie"), None);
    }

    #[test]
    fn the_redirect_limit_fails_the_request() {
        let t = Scripted::new(|_| Reply::new(302).header("Location", "/again"));
        let client = builder(&t).redirects(Redirects::Follow(3)).build();
        let err = wait(client.fetch_future(Request::get("http://day.test/"))).unwrap_err();
        assert_eq!(err, HttpError::TooManyRedirects);
        assert_eq!(t.seen().len(), 4);
    }

    #[test]
    fn never_delivers_the_redirect_itself() {
        let t = Scripted::new(|_| Reply::new(301).header("Location", "/moved").body("moved"));
        let client = builder(&t).redirects(Redirects::Never).build();
        let resp = wait(client.fetch_future(Request::get("http://day.test/"))).expect("response");
        assert_eq!((resp.status, resp.text().as_ref()), (301, "moved"));
    }

    #[test]
    fn a_redirect_handler_decides_each_hop() {
        let t = Scripted::new(|seen| match seen.url.as_str() {
            "http://day.test/1" => Reply::new(302).header("Location", "/2"),
            "http://day.test/2" => Reply::new(302)
                .header("Location", "/3")
                .body("stopped here"),
            _ => Reply::new(200),
        });
        let hops = Arc::new(Mutex::new(Vec::new()));
        let log = hops.clone();
        let client = builder(&t)
            .on_redirect(move |hop, reply| {
                lock(&log).push(hop.to.clone());
                if hop.followed == 0 {
                    drop(reply);
                } else {
                    reply.stop();
                }
            })
            .build();
        let resp = wait(client.fetch_future(Request::get("http://day.test/1"))).expect("response");
        assert_eq!((resp.status, resp.text().as_ref()), (302, "stopped here"));
        assert_eq!(*lock(&hops), ["http://day.test/2", "http://day.test/3"]);
    }

    #[test]
    fn a_redirect_handler_needs_manual_redirects() {
        let mut t = Scripted::new(|_| Reply::new(200));
        t.caps.manual_redirects = false;
        let client = builder(&t).on_redirect(|_, reply| reply.follow()).build();
        let err = wait(client.fetch_future(Request::get("http://day.test/"))).unwrap_err();
        assert_eq!(err, HttpError::Unsupported);
    }

    #[test]
    fn a_basic_challenge_is_answered_once_and_remembered() {
        let t = Scripted::new(|seen| match seen.header("authorization") {
            Some(a) if a == basic_authorization("day", "sunrise") => Reply::new(200).body("in"),
            _ => Reply::new(401).header("WWW-Authenticate", "Basic realm=\"Day test\""),
        });
        let asked = Arc::new(AtomicUsize::new(0));
        let count = asked.clone();
        let client = builder(&t)
            .on_challenge(move |challenge, reply| {
                count.fetch_add(1, Ordering::SeqCst);
                assert_eq!(challenge.scheme, Scheme::Basic);
                assert_eq!(challenge.realm.as_deref(), Some("Day test"));
                reply.credential("day", "sunrise");
            })
            .build();
        for _ in 0..2 {
            let resp = wait(client.fetch_future(Request::get("http://day.test/private")))
                .expect("response");
            assert_eq!(resp.text(), "in");
        }
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        assert_eq!(t.seen().len(), 4);
    }

    #[test]
    fn a_digest_challenge_is_answered() {
        let t = Scripted::new(|seen| match seen.header("authorization") {
            Some(a) if verify_digest(a, "day", "sunrise", &seen.method) => {
                Reply::new(200).body("in")
            }
            _ => Reply::new(401).header(
                "WWW-Authenticate",
                "Digest realm=\"Day test\", nonce=\"n1\", qop=\"auth\"",
            ),
        });
        let client = builder(&t)
            .on_challenge(|_, reply| reply.credential("day", "sunrise"))
            .build();
        let resp = wait(client.fetch_future(Request::get("http://day.test/digest?x=1")))
            .expect("response");
        assert_eq!(resp.text(), "in");
    }

    #[test]
    fn refused_credentials_end_with_the_401() {
        let t = Scripted::new(|_| {
            Reply::new(401).header("WWW-Authenticate", "Basic realm=\"Day test\"")
        });
        let client = builder(&t)
            .on_challenge(|_, reply| reply.credential("day", "wrong"))
            .build();
        let resp = wait(client.fetch_future(Request::get("http://day.test/"))).expect("response");
        assert_eq!(resp.status, 401);
        assert_eq!(t.seen().len(), 1 + MAX_AUTH_ATTEMPTS as usize);
    }

    #[test]
    fn without_a_handler_the_401_is_the_response() {
        let t = Scripted::new(|_| Reply::new(401).header("WWW-Authenticate", "Bearer"));
        let client = builder(&t).build();
        let resp = wait(client.fetch_future(Request::get("http://day.test/"))).expect("response");
        assert_eq!(resp.status, 401);
        assert_eq!(t.seen().len(), 1);
    }

    #[test]
    fn cookies_set_during_a_redirect_reach_the_next_hop() {
        let t = Scripted::new(|seen| match seen.url.as_str() {
            "http://day.test/login" => Reply::new(302)
                .header("Location", "/home")
                .header("Set-Cookie", "session=abc; Path=/; HttpOnly"),
            _ => Reply::new(200),
        });
        let client = builder(&t).build();
        wait(client.fetch_future(Request::get("http://day.test/login"))).expect("response");
        assert_eq!(t.seen()[1].header("cookie"), Some("session=abc"));
        assert_eq!(client.cookies().len(), 1);
        client.clear_cookies();
        assert!(client.cookies().is_empty());
    }

    #[test]
    fn the_jar_follows_domain_path_and_secure_rules() {
        let jar = CookieJar::new();
        jar.store(
            "https://www.day.test/app/page",
            [
                "a=1",
                "b=2; Domain=day.test; Path=/",
                "c=3; Secure; Path=/",
                "d=4; Domain=test",
                "e=5; Max-Age=0",
                "f=6; Domain=other.test",
            ],
        );
        assert_eq!(
            jar.header("https://www.day.test/app/x").as_deref(),
            Some("a=1; b=2; c=3")
        );
        assert_eq!(
            jar.header("http://www.day.test/app/x").as_deref(),
            Some("a=1; b=2")
        );
        assert_eq!(jar.header("https://api.day.test/").as_deref(), Some("b=2"));
        assert_eq!(
            jar.header("https://www.day.test/other").as_deref(),
            Some("b=2; c=3")
        );
        jar.store(
            "https://www.day.test/",
            ["b=; Domain=day.test; Path=/; Max-Age=0"],
        );
        assert_eq!(
            jar.header("https://www.day.test/other").as_deref(),
            Some("c=3")
        );
    }

    #[test]
    fn a_persistent_jar_keeps_only_cookies_with_an_expiry() {
        let dir = std::env::temp_dir().join(format!("day-http-jar-{}", std::process::id()));
        let file = dir.join("cookies");
        let _ = std::fs::remove_file(&file);
        let jar = CookieJar::open(&file);
        jar.store(
            "https://day.test/",
            ["session=1", "remember=2; Max-Age=3600"],
        );
        let reopened = CookieJar::open(&file);
        assert_eq!(
            reopened.header("https://day.test/").as_deref(),
            Some("remember=2")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_slow_reader_holds_the_transport_to_its_window() {
        let t = Scripted::new(|_| Reply::new(200).chunks(100));
        let client = builder(&t).build();
        let streaming =
            wait(client.send_future(Request::get("http://day.test/big"))).expect("head");
        assert_eq!(streaming.expected_length(), Some(100));
        let mut body = streaming.into_body();
        assert_eq!(wait(body.next()), Some(Ok(vec![0])));
        let granted = t.granted.load(Ordering::SeqCst);
        assert!(granted <= u64::from(WINDOW), "granted {granted}");
        let mut all = vec![0u8];
        while let Some(chunk) = wait(body.next()) {
            all.extend(chunk.expect("chunk"));
        }
        assert_eq!(all, (0..100).collect::<Vec<u8>>());
        assert_eq!(body.received(), 100);
    }

    #[test]
    fn dropping_an_unread_body_cancels_the_exchange() {
        let t = Scripted::new(|_| Reply::new(200).chunks(100));
        let client = builder(&t).build();
        let streaming =
            wait(client.send_future(Request::get("http://day.test/big"))).expect("head");
        drop(streaming);
        assert_eq!(t.cancels.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn dropping_the_send_future_cancels_the_exchange() {
        let t = Scripted::new(|_| Reply::new(200).stall());
        let client = builder(&t).build();
        drop(client.send_future(Request::get("http://day.test/")));
        assert_eq!(t.cancels.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn read_async_hands_over_every_chunk_then_the_end() {
        let t = Scripted::new(|_| Reply::new(200).chunks(10));
        let client = builder(&t).build();
        let (tx, rx) = std::sync::mpsc::channel();
        client.send_async(Request::get("http://day.test/"), move |head| {
            let body = head.expect("head").into_body();
            body.read_async(move |item| {
                let more = matches!(item, Some(Ok(_)));
                let _ = tx.send(item.map(|r| r.map(|c| c[0])));
                more
            });
        });
        let items: Vec<_> = rx.iter().take(11).collect();
        let expected: Vec<_> = (0..10).map(|i| Some(Ok(i))).collect();
        assert_eq!(&items[..10], &expected[..]);
        assert_eq!(items[10], None);
    }

    #[test]
    fn a_pin_that_matches_accepts_and_one_that_does_not_rejects() {
        let cert = fake_certificate(0xAA);
        let good = pin_string(&spki_sha256(&cert).expect("spki"));
        let asking = |cert: Vec<u8>| {
            move |_: &Seen| {
                Reply::new(200)
                    .body("secure")
                    .question(Question::ServerTrust(ServerTrust {
                        host: "day.test".into(),
                        chain: vec![cert.clone()],
                        system_trusted: true,
                        system_error: None,
                    }))
            }
        };
        let t = Scripted::new(asking(cert.clone()));
        let client = builder(&t)
            .trust(Trust::system().pin("day.test", &good))
            .build();
        let resp = wait(client.fetch_future(Request::get("https://day.test/"))).expect("response");
        assert_eq!(resp.text(), "secure");
        assert_eq!(*lock(&t.answers), [Answer::Accept]);

        let t = Scripted::new(asking(cert));
        let wrong = "sha256/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let client = builder(&t)
            .trust(Trust::system().pin("*.test", wrong))
            .build();
        let err = wait(client.fetch_future(Request::get("https://day.test/"))).unwrap_err();
        assert!(matches!(err, HttpError::Tls(_)), "{err:?}");
        assert_eq!(*lock(&t.answers), [Answer::Reject]);
    }

    #[test]
    fn a_trust_handler_may_accept_what_the_platform_refused() {
        let t = Scripted::new(|_| {
            Reply::new(200).question(Question::ServerTrust(ServerTrust {
                host: "self-signed.test".into(),
                chain: Vec::new(),
                system_trusted: false,
                system_error: Some("self-signed".into()),
            }))
        });
        let client = builder(&t)
            .on_server_trust(|trust, reply| {
                if trust.host == "self-signed.test" {
                    reply.accept()
                } else {
                    reply.reject()
                }
            })
            .build();
        let resp =
            wait(client.fetch_future(Request::get("https://self-signed.test/"))).expect("response");
        assert_eq!(resp.status, 200);
        assert_eq!(*lock(&t.answers), [Answer::Accept]);
    }

    #[test]
    fn a_question_nobody_answers_takes_its_default_after_the_timeout() {
        static HELD: Mutex<Vec<ChallengeReply>> = Mutex::new(Vec::new());
        let t =
            Scripted::new(|_| Reply::new(401).header("WWW-Authenticate", "Bearer realm=\"api\""));
        let client = builder(&t)
            .question_timeout(Duration::from_millis(50))
            .on_challenge(|_, reply| lock(&HELD).push(reply))
            .build();
        let resp = wait(client.fetch_future(Request::get("http://day.test/"))).expect("response");
        assert_eq!(resp.status, 401);
        lock(&HELD).clear();
    }

    #[test]
    fn the_total_limit_fails_a_stalled_request() {
        let t = Scripted::new(|_| Reply::new(200).stall());
        let client = builder(&t).timeout_total(Duration::from_millis(50)).build();
        // The count is read inside the callback, where the caller first hears of the timeout:
        // the stalled exchange must be cancelled by then, not a moment later on the timer's
        // thread (reading it after a woken future raced that thread, and lost on Windows CI).
        let cancels = t.cancels.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let _flight = client.fetch_async(Request::get("http://day.test/"), move |result| {
            let _ = tx.send((result.err(), cancels.load(Ordering::SeqCst)));
        });
        let (err, cancelled) = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the total limit fires");
        assert_eq!(err, Some(HttpError::Timeout));
        assert_eq!(cancelled, 1);
    }

    #[test]
    fn a_bad_url_fails_before_any_exchange() {
        let t = Scripted::new(|_| Reply::new(200));
        let client = builder(&t).build();
        let err = wait(client.fetch_future(Request::get("ftp://day.test/"))).unwrap_err();
        assert!(matches!(err, HttpError::BadUrl(_)), "{err:?}");
        assert!(t.seen().is_empty());
    }

    #[test]
    fn a_websocket_echoes_then_closes_with_a_code() {
        let t = Scripted::new(|_| Reply::new(200));
        let client = builder(&t).build();
        let mut socket =
            wait(client.websocket_future(Request::get("ws://day.test/echo").protocols(["chat"])))
                .expect("open");
        assert_eq!(socket.protocol().as_deref(), Some("chat"));
        wait(socket.send_future(Message::Text("hello".into())))
            .expect("delivered")
            .expect("sent");
        assert_eq!(wait(socket.next()), Some(Ok(Message::Text("hello".into()))));
        socket.close(4000, "done");
        assert_eq!(
            wait(socket.next()),
            Some(Ok(Message::Close {
                code: 4000,
                reason: "done".into()
            }))
        );
        assert_eq!(wait(socket.next()), None);
    }
}
