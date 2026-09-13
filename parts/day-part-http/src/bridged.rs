// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// The bridged transports (src/bridge.rs): Android's Java arm over OkHttp, HarmonyOS's ArkTS arm
// over the Network Kit, and the web's JavaScript arm over fetch and WebSocket. Each arm speaks
// the same tagged frames over one bridge stream per exchange; this file turns them into transport
// events, and turns the client's demand, answers and cancellation into plain bridge calls. What
// each arm can do differs, and `CAPABILITIES` says so per platform.
//
// Frame layout, big-endian: a tag byte, the stream's token (i64), then the fields. Strings are an
// i32 length and UTF-8 bytes.
//
//   1 head       status i32, final url, header block, content length i64 (−1 unknown)
//   2 chunk      the bytes of one read
//   3 sent       bytes sent i64, total i64 (−1 unknown)
//   4 question   id i32, kind u8 (1 server trust), host, trusted u8, error, count i32,
//                then each certificate as i32 length and DER bytes
//   5 metrics    dns, connect, tls, first byte, total (i64 microseconds, −1 unknown), protocol,
//                reused u8 (2 unknown), remote address, TLS version, from cache u8,
//                bytes sent i64, bytes received i64
//   6 end
//   7 failed     sentinel i32, message
//   8 need body  the most bytes the arm wants now, i32
//  16 open       protocol
//  17 text       text
//  18 binary     the bytes
//  19 closed     code i32, reason
//  20 ws failed  sentinel i32, message
// ---------------------------------------------------------------------------

use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use day_bridge::Item;

use crate::bridge;
use crate::client::{
    Answer, CachePolicy, Capabilities, Completion, Event, Events, Head, Message, Metrics, Prepared,
    PreparedBody, Question, QuestionId, ServerTrust, Socket, Transfer, Transport, TransportConfig,
    WsEvent, WsEvents,
};
use crate::{HttpError, Tier};

/// HarmonyOS reaches its arm only when `day build` staged it; a plain cargo build of this crate for
/// the target compiles the fallback arm, which reports no HTTP capability rather than a broken one.
#[cfg(all(target_os = "linux", target_env = "ohos"))]
pub const TIER: Tier = if cfg!(day_bridge_staged) {
    Tier::NativeStack
} else {
    Tier::Unavailable
};

#[cfg(not(all(target_os = "linux", target_env = "ohos")))]
pub const TIER: Tier = Tier::NativeStack;

/// What the OkHttp arm offers the client. Challenges run through the client's own loop (OkHttp
/// has no NTLM or Negotiate), cookies through its jar, and a WebSocket has no manual ping.
#[cfg(target_os = "android")]
pub(crate) const CAPABILITIES: Capabilities = Capabilities {
    streaming: true,
    upload_streaming: true,
    upload_progress: true,
    manual_redirects: true,
    auth_questions: false,
    native_auth_schemes: false,
    server_trust: true,
    client_identity: true,
    platform_cookies: false,
    platform_cache: true,
    metrics: true,
    websockets: true,
    websocket_ping: false,
    websocket_headers: true,
    wait_for_connectivity: false,
};

/// What the Network Kit arm offers. `requestInStream` delivers the body as it arrives, with no way
/// to hold it back, and follows redirects itself; a request body is sent from memory.
#[cfg(all(target_os = "linux", target_env = "ohos"))]
pub(crate) const CAPABILITIES: Capabilities = Capabilities {
    streaming: true,
    upload_streaming: false,
    upload_progress: true,
    manual_redirects: false,
    auth_questions: false,
    native_auth_schemes: false,
    server_trust: false,
    client_identity: false,
    platform_cookies: false,
    platform_cache: true,
    metrics: false,
    websockets: true,
    websocket_ping: false,
    websocket_headers: true,
    wait_for_connectivity: false,
};

/// What the browser offers. fetch reads the body as a stream under demand, follows redirects
/// itself, and keeps cookies and a cache of its own; a request body is sent from memory, and a
/// WebSocket takes no request headers.
#[cfg(target_arch = "wasm32")]
pub(crate) const CAPABILITIES: Capabilities = Capabilities {
    streaming: true,
    upload_streaming: false,
    upload_progress: false,
    manual_redirects: false,
    auth_questions: false,
    native_auth_schemes: false,
    server_trust: false,
    client_identity: false,
    platform_cookies: true,
    platform_cache: true,
    metrics: false,
    websockets: true,
    websocket_ping: false,
    websocket_headers: false,
    wait_for_connectivity: false,
};

/// What this platform's arm offers, for [`crate::capabilities`].
pub(crate) fn capabilities() -> Capabilities {
    CAPABILITIES
}

const HEAD: u8 = 1;
const CHUNK: u8 = 2;
const SENT: u8 = 3;
const QUESTION: u8 = 4;
const METRICS: u8 = 5;
const END: u8 = 6;
const FAILED: u8 = 7;
const NEED_BODY: u8 = 8;
const WS_OPEN: u8 = 16;
const WS_TEXT: u8 = 17;
const WS_BINARY: u8 = 18;
const WS_CLOSED: u8 = 19;
const WS_FAILED: u8 = 20;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Reads one frame's fields in order; a short frame reads as zeros and empty strings.
struct Fields<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Fields<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Fields { bytes, at: 0 }
    }

    fn take(&mut self, n: usize) -> &'a [u8] {
        let end = self.at.saturating_add(n).min(self.bytes.len());
        let out = &self.bytes[self.at.min(end)..end];
        self.at = end;
        out
    }

    fn u8(&mut self) -> u8 {
        self.take(1).first().copied().unwrap_or(0)
    }

    fn i32(&mut self) -> i32 {
        self.take(4).try_into().map(i32::from_be_bytes).unwrap_or(0)
    }

    fn i64(&mut self) -> i64 {
        self.take(8).try_into().map(i64::from_be_bytes).unwrap_or(0)
    }

    fn str(&mut self) -> String {
        let len = usize::try_from(self.i32()).unwrap_or(0);
        String::from_utf8_lossy(self.take(len)).into_owned()
    }

    fn bytes(&mut self) -> Vec<u8> {
        let len = usize::try_from(self.i32()).unwrap_or(0);
        self.take(len).to_vec()
    }

    fn rest(&mut self) -> &'a [u8] {
        self.take(self.bytes.len())
    }
}

/// A transport sentinel as this crate's error.
fn sentinel_error(sentinel: i32, message: String) -> HttpError {
    match sentinel {
        -1 => HttpError::Timeout,
        -2 => HttpError::Dns,
        -3 => HttpError::Tls(message),
        -4 => HttpError::Connect,
        -6 => HttpError::BadUrl(message),
        -7 => HttpError::Cancelled,
        _ => HttpError::Io(message),
    }
}

fn header_pairs(block: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = block.split('\n').collect();
    lines
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[name, value]| (name.to_string(), value.to_string()))
        .collect()
}

fn millis(d: Duration) -> i32 {
    i32::try_from(d.as_millis()).unwrap_or(i32::MAX)
}

/// The transport for one client.
pub(crate) fn transport(config: &TransportConfig) -> Arc<dyn Transport> {
    Arc::new(AndroidTransport::new(config))
}

struct AndroidTransport {
    client: Result<i64, HttpError>,
}

impl AndroidTransport {
    fn new(config: &TransportConfig) -> AndroidTransport {
        let (identity, password) = config
            .identity
            .as_ref()
            .map(|i| (i.pkcs12.as_slice(), i.password.as_str()))
            .unwrap_or((&[], ""));
        let client = bridge::client(
            millis(config.timeout_idle),
            config.timeout_total.map_or(0, millis),
            config
                .platform_cache
                .map_or(0, |(_, disk)| i64::try_from(disk).unwrap_or(i64::MAX)),
            config
                .max_per_host
                .map_or(0, |n| i32::try_from(n).unwrap_or(i32::MAX)),
            config.ask_trust,
            identity,
            password,
        )
        .map_err(bridge::bridge_error);
        AndroidTransport { client }
    }
}

impl Drop for AndroidTransport {
    fn drop(&mut self) {
        if let Ok(client) = self.client {
            bridge::client_release(client);
        }
    }
}

struct AndroidTransfer {
    token: AtomicU64,
    state: Mutex<ExchangeState>,
}

#[derive(Default)]
struct ExchangeState {
    events: Option<Events>,
    reader: Option<Box<dyn Read + Send>>,
    pending_demand: u32,
    pending_answers: Vec<(QuestionId, i32)>,
    cancelled: bool,
}

impl AndroidTransfer {
    /// Learn the stream's token, from the call's return or from the first frame, and send what
    /// waited for it.
    fn adopt(&self, token: u64) {
        let (demand, answers, cancelled) = {
            let mut st = lock(&self.state);
            if token == 0
                || self
                    .token
                    .compare_exchange(0, token, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
            {
                return;
            }
            (
                std::mem::take(&mut st.pending_demand),
                std::mem::take(&mut st.pending_answers),
                st.cancelled,
            )
        };
        let token = token as i64;
        if cancelled {
            return bridge::exchange_cancel(token);
        }
        if demand > 0 {
            bridge::demand(token, i32::try_from(demand).unwrap_or(i32::MAX));
        }
        for (id, answer) in answers {
            bridge::answer(token, i32::try_from(id).unwrap_or(0), answer);
        }
    }

    fn events(&self) -> Option<Events> {
        lock(&self.state).events.clone()
    }

    fn finish(&self, event: Event) {
        let events = lock(&self.state).events.take();
        if let Some(events) = events {
            events(event);
        }
    }

    fn item(&self, item: Item<Vec<u8>>) {
        match item {
            Item::Value(frame) => self.frame(&frame),
            Item::End => {}
            Item::Failed(e) => self.finish(Event::Failed(bridge::bridge_error(e))),
        }
    }

    fn frame(&self, frame: &[u8]) {
        let mut f = Fields::new(frame);
        let tag = f.u8();
        let token = f.i64();
        self.adopt(token as u64);
        match tag {
            HEAD => {
                let status = u16::try_from(f.i32()).unwrap_or(0);
                let url = f.str();
                let headers = header_pairs(&f.str());
                let expected_length = u64::try_from(f.i64()).ok();
                if let Some(events) = self.events() {
                    events(Event::Head(Head {
                        status,
                        headers,
                        url,
                        expected_length,
                    }));
                }
            }
            CHUNK => {
                if let Some(events) = self.events() {
                    events(Event::Chunk(f.rest().to_vec()));
                }
            }
            SENT => {
                let sent = u64::try_from(f.i64()).unwrap_or(0);
                let total = u64::try_from(f.i64()).ok();
                if let Some(events) = self.events() {
                    events(Event::Sent { sent, total });
                }
            }
            QUESTION => {
                let id = QuestionId::try_from(f.i32()).unwrap_or(0);
                let _kind = f.u8();
                let host = f.str();
                let system_trusted = f.u8() != 0;
                let error = f.str();
                let count = usize::try_from(f.i32()).unwrap_or(0).min(32);
                let chain = (0..count).map(|_| f.bytes()).collect();
                let question = Question::ServerTrust(ServerTrust {
                    host,
                    chain,
                    system_trusted,
                    system_error: (!error.is_empty()).then_some(error),
                });
                match self.events() {
                    Some(events) => events(Event::Question(id, question)),
                    None => bridge::answer(token, i32::try_from(id).unwrap_or(0), 3),
                }
            }
            METRICS => {
                let span = |v: i64| u64::try_from(v).ok().map(Duration::from_micros);
                let (dns, connect, tls, first_byte, total) =
                    (f.i64(), f.i64(), f.i64(), f.i64(), f.i64());
                let protocol = f.str();
                let reused = f.u8();
                let remote = f.str();
                let tls_version = f.str();
                let from_cache = f.u8() != 0;
                let bytes_sent = u64::try_from(f.i64()).ok();
                let bytes_received = u64::try_from(f.i64()).ok();
                let metrics = Metrics {
                    dns: span(dns),
                    connect: span(connect),
                    tls: span(tls),
                    first_byte: span(first_byte),
                    total: span(total),
                    protocol: (!protocol.is_empty()).then_some(protocol),
                    reused_connection: match reused {
                        0 => Some(false),
                        1 => Some(true),
                        _ => None,
                    },
                    proxy: None,
                    remote_address: (!remote.is_empty()).then_some(remote),
                    tls_version: (!tls_version.is_empty()).then_some(tls_version),
                    from_cache,
                    bytes_sent,
                    bytes_received,
                    redirects: 0,
                };
                if let Some(events) = self.events() {
                    events(Event::Metrics(metrics));
                }
            }
            END => self.finish(Event::End),
            FAILED => {
                let sentinel = f.i32();
                let message = f.str();
                self.finish(Event::Failed(sentinel_error(sentinel, message)));
            }
            NEED_BODY => {
                let want = usize::try_from(f.i32()).unwrap_or(0).clamp(1, 1 << 20);
                let mut buf = vec![0u8; want];
                let reader = lock(&self.state).reader.take();
                let read = match reader {
                    Some(mut reader) => match reader.read(&mut buf) {
                        Ok(n) if n > 0 => {
                            lock(&self.state).reader = Some(reader);
                            n
                        }
                        _ => 0,
                    },
                    None => 0,
                };
                bridge::body(token, &buf[..read]);
            }
            _ => {}
        }
    }
}

impl Transfer for AndroidTransfer {
    fn demand(&self, chunks: u32) {
        let token = {
            let mut st = lock(&self.state);
            match self.token.load(Ordering::SeqCst) {
                0 => {
                    st.pending_demand += chunks;
                    return;
                }
                token => token as i64,
            }
        };
        bridge::demand(token, i32::try_from(chunks).unwrap_or(i32::MAX));
    }

    fn answer(&self, id: QuestionId, answer: Answer) {
        let code = match answer {
            Answer::Accept => 1,
            Answer::Reject => 2,
            Answer::Cancel => 3,
            _ => 0,
        };
        let token = {
            let mut st = lock(&self.state);
            match self.token.load(Ordering::SeqCst) {
                0 => {
                    st.pending_answers.push((id, code));
                    return;
                }
                token => token as i64,
            }
        };
        bridge::answer(token, i32::try_from(id).unwrap_or(0), code);
    }

    fn cancel(&self) {
        let (token, events) = {
            let mut st = lock(&self.state);
            st.cancelled = true;
            (self.token.load(Ordering::SeqCst), st.events.take())
        };
        drop(events);
        if token != 0 {
            bridge::exchange_cancel(token as i64);
        }
    }
}

impl Transport for AndroidTransport {
    fn capabilities(&self) -> Capabilities {
        CAPABILITIES
    }

    fn start(&self, prepared: Prepared, events: Events) -> Arc<dyn Transfer> {
        let transfer = Arc::new(AndroidTransfer {
            token: AtomicU64::new(0),
            state: Mutex::new(ExchangeState {
                events: Some(events),
                ..ExchangeState::default()
            }),
        });
        let client = match &self.client {
            Ok(client) => *client,
            Err(e) => {
                transfer.finish(Event::Failed(e.clone()));
                return transfer;
            }
        };
        let empty: &[u8] = &[];
        // An arm that sends a body from memory gets one: read the file or stream here first.
        let buffered;
        let body = if CAPABILITIES.upload_streaming {
            &prepared.body
        } else {
            let bytes = match &prepared.body {
                PreparedBody::File { path, .. } => Some(std::fs::read(path)),
                PreparedBody::Stream { reader, .. } => reader.take().map(|mut reader| {
                    let mut bytes = Vec::new();
                    reader.read_to_end(&mut bytes).map(|_| bytes)
                }),
                _ => None,
            };
            match bytes {
                Some(Ok(bytes)) => {
                    buffered = PreparedBody::Bytes(Arc::new(bytes));
                    &buffered
                }
                Some(Err(e)) => {
                    transfer.finish(Event::Failed(HttpError::Io(e.to_string())));
                    return transfer;
                }
                None => &prepared.body,
            }
        };
        let (kind, bytes, path, len) = match body {
            PreparedBody::Empty => (0, empty, String::new(), -1),
            PreparedBody::Bytes(b) => (1, b.as_slice(), String::new(), b.len() as i64),
            PreparedBody::File { path, len } => (
                2,
                empty,
                path.to_string_lossy().into_owned(),
                i64::try_from(*len).unwrap_or(-1),
            ),
            PreparedBody::Stream { reader, len } => match reader.take() {
                Some(reader) => {
                    lock(&transfer.state).reader = Some(reader);
                    (
                        3,
                        empty,
                        String::new(),
                        len.and_then(|l| i64::try_from(l).ok()).unwrap_or(-1),
                    )
                }
                None => {
                    transfer.finish(Event::Failed(HttpError::Io(
                        "the request body stream was already sent".into(),
                    )));
                    return transfer;
                }
            },
        };
        let grip = transfer.clone();
        let started = bridge::exchange_native_stream(
            client,
            &prepared.method,
            &prepared.url,
            &bridge::header_block(&prepared.headers),
            kind,
            bytes,
            &path,
            len,
            millis(prepared.timeout_idle),
            match prepared.cache {
                CachePolicy::Default => 0,
                CachePolicy::Reload => 1,
                CachePolicy::PreferCache => 2,
            },
            move |item| grip.item(item),
        );
        match started {
            Ok(token) => transfer.adopt(token),
            Err(e) => transfer.finish(Event::Failed(bridge::bridge_error(e))),
        }
        transfer
    }

    fn websocket(&self, prepared: Prepared, events: WsEvents) -> Arc<dyn Socket> {
        let socket = Arc::new(AndroidSocket {
            token: AtomicU64::new(0),
            events: Mutex::new(Some(events)),
        });
        let client = match &self.client {
            Ok(client) => *client,
            Err(e) => {
                socket.finish(WsEvent::Failed(e.clone()));
                return socket;
            }
        };
        let grip = socket.clone();
        let started = bridge::ws_open_native_stream(
            client,
            &prepared.url,
            &bridge::header_block(&prepared.headers),
            &prepared.protocols.join(", "),
            move |item| grip.item(item),
        );
        match started {
            Ok(token) => {
                let _ = socket
                    .token
                    .compare_exchange(0, token, Ordering::SeqCst, Ordering::SeqCst);
            }
            Err(e) => socket.finish(WsEvent::Failed(bridge::bridge_error(e))),
        }
        socket
    }

    fn clear_cache(&self) {
        if let Ok(client) = self.client {
            bridge::cache_clear(client);
        }
    }
}

struct AndroidSocket {
    token: AtomicU64,
    events: Mutex<Option<WsEvents>>,
}

impl AndroidSocket {
    fn finish(&self, event: WsEvent) {
        let events = lock(&self.events).take();
        if let Some(events) = events {
            events(event);
        }
    }

    fn item(&self, item: Item<Vec<u8>>) {
        let frame = match item {
            Item::Value(frame) => frame,
            Item::End => return,
            Item::Failed(e) => return self.finish(WsEvent::Failed(bridge::bridge_error(e))),
        };
        let mut f = Fields::new(&frame);
        let tag = f.u8();
        let token = f.i64();
        if token > 0 {
            let _ =
                self.token
                    .compare_exchange(0, token as u64, Ordering::SeqCst, Ordering::SeqCst);
        }
        let events = lock(&self.events).clone();
        let Some(events) = events else {
            return;
        };
        match tag {
            WS_OPEN => {
                let protocol = f.str();
                events(WsEvent::Open {
                    protocol: (!protocol.is_empty()).then_some(protocol),
                });
            }
            WS_TEXT => events(WsEvent::Message(Message::Text(f.str()))),
            WS_BINARY => events(WsEvent::Message(Message::Binary(f.rest().to_vec()))),
            WS_CLOSED => {
                let code = u16::try_from(f.i32()).unwrap_or(1006);
                let reason = f.str();
                self.finish(WsEvent::Closed { code, reason });
            }
            WS_FAILED => {
                let sentinel = f.i32();
                let message = f.str();
                self.finish(WsEvent::Failed(sentinel_error(sentinel, message)));
            }
            _ => {}
        }
    }

    fn token(&self) -> Option<i64> {
        match self.token.load(Ordering::SeqCst) {
            0 => None,
            token => Some(token as i64),
        }
    }
}

impl Socket for AndroidSocket {
    fn send(&self, message: Message, done: Completion) {
        let Some(token) = self.token() else {
            return done(Err(HttpError::Io("the WebSocket is not open".into())));
        };
        let (binary, data) = match message {
            Message::Text(text) => (false, text.into_bytes()),
            Message::Binary(bytes) => (true, bytes),
            Message::Close { code, reason } => {
                self.close(code, &reason);
                return done(Ok(()));
            }
        };
        done(match bridge::ws_send(token, binary, &data) {
            Ok(true) => Ok(()),
            Ok(false) => Err(HttpError::Io("the WebSocket is closed".into())),
            Err(e) => Err(bridge::bridge_error(e)),
        });
    }

    fn ping(&self, done: Completion) {
        done(Err(HttpError::Unsupported));
    }

    fn close(&self, code: u16, reason: &str) {
        if let Some(token) = self.token() {
            bridge::ws_close(token, i32::from(code), reason);
        }
    }

    /// OkHttp delivers every message as it arrives; the client's queue holds them.
    fn demand(&self, _messages: u32) {}
}
