// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// macOS + iOS: NSURLSession, the system networking stack (proxies and PAC, VPN, Low Data Mode,
// the platform TLS and keychain trust store), as the transport beneath the portable client.
//
// One session per client, with a Rust delegate. URLSession documents its sessions and tasks as
// thread-safe, and delivers every delegate message on the session's serial delegate queue, so
// each task's callbacks arrive in order and route to that task's transfer by identifier.
//
// - Redirects come back to the client: the delegate declines every automatic hop, so the 3xx is
//   the task's response and the client applies one redirect policy on every backend.
// - HTTP challenges (Basic, Digest, NTLM, Negotiate) and server trust become questions; the
//   stored completion handler answers them later, from whichever thread the client replies on.
// - A client identity is imported once with SecPKCS12Import and presented when asked.
// - The body flows under demand: when the reader's credit runs out the task is suspended, and
//   the next grant resumes it. Data already in URLSession's buffers still arrives after a
//   suspend, which the client's queue absorbs.
// - Uploads send bytes as the request body, files through an upload task that reads from disk,
//   and streams through a bound stream pair fed by a writer thread.
// - WebSockets ride URLSessionWebSocketTask, receiving one message per unit of demand.
// ---------------------------------------------------------------------------

use std::collections::HashMap;
use std::io::Read;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AllocAnyThread, ClassType, DefinedClass, define_class, msg_send};
use objc2_core_foundation::{CFData, CFDictionary, CFError, CFRetained, CFString};
use objc2_foundation::{
    NSData, NSDate, NSDictionary, NSError, NSHTTPCookie, NSHTTPCookieAcceptPolicy,
    NSHTTPCookieStorage, NSHTTPURLResponse, NSInputStream, NSMutableURLRequest, NSObject,
    NSObjectProtocol, NSOutputStream, NSStream, NSStreamStatus, NSString, NSURL,
    NSURLAuthenticationChallenge, NSURLAuthenticationMethodClientCertificate,
    NSURLAuthenticationMethodHTTPBasic, NSURLAuthenticationMethodHTTPDigest,
    NSURLAuthenticationMethodNTLM, NSURLAuthenticationMethodNegotiate,
    NSURLAuthenticationMethodServerTrust, NSURLCache, NSURLCredential, NSURLCredentialPersistence,
    NSURLRequest, NSURLRequestCachePolicy, NSURLResponse, NSURLSession,
    NSURLSessionAuthChallengeDisposition, NSURLSessionConfiguration, NSURLSessionDataDelegate,
    NSURLSessionDataTask, NSURLSessionDelegate, NSURLSessionResponseDisposition, NSURLSessionTask,
    NSURLSessionTaskDelegate, NSURLSessionTaskMetrics, NSURLSessionTaskMetricsResourceFetchType,
    NSURLSessionWebSocketCloseCode, NSURLSessionWebSocketDelegate, NSURLSessionWebSocketMessage,
    NSURLSessionWebSocketMessageType, NSURLSessionWebSocketTask,
};
use objc2_security::{
    SecCertificate, SecIdentity, SecPKCS12Import, SecTrust, kSecImportExportPassphrase,
    kSecImportItemIdentity,
};

use crate::client::{
    Answer, AuthQuestion, CachePolicy, Capabilities, Completion, Cookie, Event, Events, Head,
    Message, Metrics, Prepared, PreparedBody, Question, QuestionId, Scheme, ServerTrust, Socket,
    Transfer, Transport, TransportConfig, WsEvent, WsEvents,
};
use crate::{HttpError, Identity, Tier};

pub const TIER: Tier = Tier::NativeStack;

/// Everything URLSession offers the client.
pub(crate) const CAPABILITIES: Capabilities = Capabilities {
    streaming: true,
    upload_streaming: true,
    upload_progress: true,
    manual_redirects: true,
    auth_questions: true,
    native_auth_schemes: true,
    server_trust: true,
    client_identity: true,
    platform_cookies: true,
    platform_cache: true,
    metrics: true,
    websockets: true,
    websocket_ping: true,
    websocket_headers: true,
    wait_for_connectivity: true,
};

/// What URLSession offers, for [`crate::capabilities`].
pub(crate) fn capabilities() -> Capabilities {
    CAPABILITIES
}

const UPLOAD_BUFFER: usize = 64 << 10;
const MAX_MESSAGE: isize = 16 << 20;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// A Foundation or Security object carried across threads.
struct Shared<T>(T);
// SAFETY: every `Shared` here holds an object URLSession or Security documents as thread-safe:
// NSURLSession and its tasks ("thread safe"), NSHTTPCookieStorage and NSURLCache (shared
// instances used from any thread), NSOutputStream used from one writer thread at a time,
// SecTrust and SecIdentity (immutable after evaluation and import), and completion handler
// blocks, which URLSession accepts on any thread and which are called exactly once.
unsafe impl<T> Send for Shared<T> {}
unsafe impl<T> Sync for Shared<T> {}

/// The transport for one client.
pub(crate) fn transport(config: &TransportConfig) -> Arc<dyn Transport> {
    Arc::new(AppleTransport::new(config))
}

struct AppleTransport {
    session: Shared<Retained<NSURLSession>>,
    routes: Arc<Routes>,
    cookies: Option<Shared<Retained<NSHTTPCookieStorage>>>,
    cache: Option<Shared<Retained<NSURLCache>>>,
}

/// Where the delegate sends each task's messages.
struct Routes {
    config: TransportConfig,
    identity: Mutex<Option<Shared<Retained<NSURLCredential>>>>,
    transfers: Mutex<HashMap<usize, Arc<AppleTransfer>>>,
    sockets: Mutex<HashMap<usize, Arc<AppleSocket>>>,
}

impl AppleTransport {
    fn new(config: &TransportConfig) -> AppleTransport {
        let conf = NSURLSessionConfiguration::defaultSessionConfiguration();
        // The client answers challenges itself; nothing is looked up or saved behind its back.
        conf.setURLCredentialStorage(None);
        conf.setTimeoutIntervalForRequest(config.timeout_idle.as_secs_f64());
        if let Some(total) = config.timeout_total {
            conf.setTimeoutIntervalForResource(total.as_secs_f64());
        }
        conf.setWaitsForConnectivity(config.wait_for_connectivity);
        if let Some(limit) = config.max_per_host {
            conf.setHTTPMaximumConnectionsPerHost(limit as isize);
        }
        let cookies = if config.platform_cookies {
            let storage = NSHTTPCookieStorage::sharedHTTPCookieStorage();
            conf.setHTTPCookieStorage(Some(&storage));
            conf.setHTTPShouldSetCookies(true);
            conf.setHTTPCookieAcceptPolicy(NSHTTPCookieAcceptPolicy::Always);
            Some(Shared(storage))
        } else {
            // The client's jar writes the Cookie header and reads Set-Cookie itself.
            conf.setHTTPCookieStorage(None);
            conf.setHTTPShouldSetCookies(false);
            None
        };
        let cache = match config.platform_cache {
            Some((memory, disk)) => {
                // One cache for the process, as URLSession's own default is: separate caches in
                // the default location would contend for the same database.
                let cache = NSURLCache::sharedURLCache();
                cache.setMemoryCapacity(cache.memoryCapacity().max(memory as usize));
                cache.setDiskCapacity(cache.diskCapacity().max(disk as usize));
                conf.setURLCache(Some(&cache));
                conf.setRequestCachePolicy(NSURLRequestCachePolicy::UseProtocolCachePolicy);
                Some(Shared(cache))
            }
            None => {
                conf.setURLCache(None);
                conf.setRequestCachePolicy(NSURLRequestCachePolicy::ReloadIgnoringLocalCacheData);
                None
            }
        };
        let routes = Arc::new(Routes {
            config: config.clone(),
            identity: Mutex::new(None),
            transfers: Mutex::new(HashMap::new()),
            sockets: Mutex::new(HashMap::new()),
        });
        let delegate = SessionDelegate::new(routes.clone());
        // SAFETY: a standard delegate session; the session retains its delegate until it is
        // invalidated, which Drop does. `None` gives the session its own serial delegate queue.
        let session = unsafe {
            NSURLSession::sessionWithConfiguration_delegate_delegateQueue(
                &conf,
                Some(ProtocolObject::from_ref(&*delegate)),
                None,
            )
        };
        AppleTransport {
            session: Shared(session),
            routes,
            cookies,
            cache,
        }
    }
}

impl Drop for AppleTransport {
    fn drop(&mut self) {
        // Tasks still running finish; then the session lets go of its delegate.
        self.session.0.finishTasksAndInvalidate();
    }
}

/// The native request for a prepared one.
fn native_request(prepared: &Prepared) -> Result<Retained<NSMutableURLRequest>, HttpError> {
    let url = NSURL::URLWithString(&NSString::from_str(&prepared.url))
        .filter(|u| u.scheme().is_some())
        .ok_or_else(|| HttpError::BadUrl(prepared.url.clone()))?;
    let request = NSMutableURLRequest::requestWithURL(&url);
    request.setHTTPMethod(&NSString::from_str(&prepared.method));
    request.setTimeoutInterval(prepared.timeout_idle.as_secs_f64());
    request.setAllowsExpensiveNetworkAccess(prepared.allow_expensive);
    request.setAllowsConstrainedNetworkAccess(prepared.allow_constrained);
    request.setCachePolicy(match prepared.cache {
        CachePolicy::Default => NSURLRequestCachePolicy::UseProtocolCachePolicy,
        CachePolicy::Reload => NSURLRequestCachePolicy::ReloadIgnoringLocalCacheData,
        CachePolicy::PreferCache => NSURLRequestCachePolicy::ReturnCacheDataElseLoad,
    });
    for (name, value) in &prepared.headers {
        request.addValue_forHTTPHeaderField(&NSString::from_str(value), &NSString::from_str(name));
    }
    Ok(request)
}

fn set_header(request: &NSMutableURLRequest, name: &str, value: &str) {
    request
        .setValue_forHTTPHeaderField(Some(&NSString::from_str(value)), &NSString::from_str(name));
}

/// A transfer that never started; its failure was already reported.
struct Declined;

impl Transfer for Declined {
    fn demand(&self, _chunks: u32) {}
    fn answer(&self, _id: QuestionId, _answer: Answer) {}
    fn cancel(&self) {}
}

impl Transport for AppleTransport {
    fn capabilities(&self) -> Capabilities {
        CAPABILITIES
    }

    fn start(&self, prepared: Prepared, events: Events) -> Arc<dyn Transfer> {
        let request = match native_request(&prepared) {
            Ok(r) => r,
            Err(e) => {
                events(Event::Failed(e));
                return Arc::new(Declined);
            }
        };
        let session = &self.session.0;
        let mut writer = None;
        let task: Retained<NSURLSessionTask> = match prepared.body {
            PreparedBody::Empty => Retained::into_super(session.dataTaskWithRequest(&request)),
            PreparedBody::Bytes(bytes) => {
                request.setHTTPBody(Some(&NSData::with_bytes(&bytes)));
                Retained::into_super(session.dataTaskWithRequest(&request))
            }
            PreparedBody::File { path, len } => {
                set_header(&request, "Content-Length", &len.to_string());
                let file = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
                Retained::into_super(Retained::into_super(
                    session.uploadTaskWithRequest_fromFile(&request, &file),
                ))
            }
            PreparedBody::Stream { reader, len } => {
                let Some(reader) = reader.take() else {
                    events(Event::Failed(HttpError::Io(
                        "the request body stream was already sent".into(),
                    )));
                    return Arc::new(Declined);
                };
                let mut input: Option<Retained<NSInputStream>> = None;
                let mut output: Option<Retained<NSOutputStream>> = None;
                NSStream::getBoundStreamsWithBufferSize_inputStream_outputStream(
                    UPLOAD_BUFFER,
                    Some(&mut input),
                    Some(&mut output),
                );
                let (Some(input), Some(output)) = (input, output) else {
                    events(Event::Failed(HttpError::Io(
                        "no stream pair for the upload".into(),
                    )));
                    return Arc::new(Declined);
                };
                request.setHTTPBodyStream(Some(&input));
                if let Some(len) = len {
                    set_header(&request, "Content-Length", &len.to_string());
                }
                writer = Some((Shared(output), reader));
                Retained::into_super(Retained::into_super(
                    session.uploadTaskWithStreamedRequest(&request),
                ))
            }
        };
        if let Some(priority) = prepared.priority {
            task.setPriority(priority);
        }
        let transfer = Arc::new(AppleTransfer {
            task: Shared(task.clone()),
            state: Mutex::new(TransferState {
                events: Some(events),
                demand: 0,
                suspended: false,
                questions: HashMap::new(),
                next_question: 0,
                refused_trust: false,
            }),
        });
        lock(&self.routes.transfers).insert(task.taskIdentifier(), transfer.clone());
        if let Some((output, reader)) = writer {
            spawn_writer(output, reader);
        }
        task.resume();
        transfer
    }

    fn websocket(&self, prepared: Prepared, events: WsEvents) -> Arc<dyn Socket> {
        let request = match native_request(&prepared) {
            Ok(r) => r,
            Err(e) => {
                events(WsEvent::Failed(e));
                return Arc::new(ClosedSocket);
            }
        };
        if !prepared.protocols.is_empty() {
            set_header(
                &request,
                "Sec-WebSocket-Protocol",
                &prepared.protocols.join(", "),
            );
        }
        let task = self.session.0.webSocketTaskWithRequest(&request);
        task.setMaximumMessageSize(MAX_MESSAGE);
        let socket = Arc::new_cyclic(|me| AppleSocket {
            me: me.clone(),
            task: Shared(task.clone()),
            state: Mutex::new(SocketState {
                events: Some(events),
                demand: 0,
                receiving: false,
                open: false,
            }),
        });
        lock(&self.routes.sockets).insert(task.taskIdentifier(), socket.clone());
        task.resume();
        socket
    }

    fn cookies(&self) -> Vec<Cookie> {
        let Some(storage) = &self.cookies else {
            return Vec::new();
        };
        storage
            .0
            .cookies()
            .map(|all| all.iter().map(|c| cookie_of(&c)).collect())
            .unwrap_or_default()
    }

    fn clear_cookies(&self) {
        if let Some(storage) = &self.cookies
            && let Some(all) = storage.0.cookies()
        {
            for cookie in all.iter() {
                storage.0.deleteCookie(&cookie);
            }
        }
    }

    fn clear_cache(&self) {
        if let Some(cache) = &self.cache {
            cache.0.removeAllCachedResponses();
        }
    }
}

/// Feed a bound stream pair's output from `reader`, on a thread of its own: the output side
/// blocks, or reports no space, until URLSession reads.
fn spawn_writer(output: Shared<Retained<NSOutputStream>>, mut reader: Box<dyn Read + Send>) {
    let _ = std::thread::Builder::new()
        .name("day-http-upload".into())
        .spawn(move || {
            // Move the whole wrapper in: its Send is what makes the stream safe to carry here.
            let output = output;
            let out = &output.0;
            out.open();
            let mut buf = vec![0u8; UPLOAD_BUFFER];
            'read: loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let mut offset = 0;
                while offset < n {
                    let Some(ptr) = NonNull::new(buf[offset..n].as_mut_ptr()) else {
                        break 'read;
                    };
                    // SAFETY: `ptr` addresses `n - offset` initialized bytes of `buf`, alive for
                    // the call; the stream copies what it accepts.
                    let written = unsafe { out.write_maxLength(ptr, n - offset) };
                    if written > 0 {
                        offset += written as usize;
                        continue;
                    }
                    let status = out.streamStatus();
                    if written < 0
                        || status == NSStreamStatus::Error
                        || status == NSStreamStatus::Closed
                    {
                        break 'read;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            out.close();
        });
}

type ChallengeCompletion =
    RcBlock<dyn Fn(NSURLSessionAuthChallengeDisposition, *mut NSURLCredential)>;

/// A question URLSession waits on, with the completion handler that answers it.
enum Pending {
    Trust {
        completion: Shared<ChallengeCompletion>,
        trust: Shared<CFRetained<SecTrust>>,
    },
    Auth {
        completion: Shared<ChallengeCompletion>,
    },
}

impl Pending {
    fn resolve(self, answer: Answer) {
        use NSURLSessionAuthChallengeDisposition as D;
        let none = std::ptr::null_mut();
        match self {
            Pending::Trust { completion, trust } => match answer {
                Answer::Accept => {
                    let trust_ptr = CFRetained::as_ptr(&trust.0).as_ptr();
                    // SAFETY: `credentialForTrust:` takes the SecTrustRef URLSession handed us,
                    // kept alive by `trust` for the call.
                    let credential: Retained<NSURLCredential> = unsafe {
                        msg_send![NSURLCredential::class(), credentialForTrust: trust_ptr]
                    };
                    completion
                        .0
                        .call((D::UseCredential, Retained::as_ptr(&credential) as *mut _));
                }
                Answer::Reject | Answer::Cancel => {
                    completion.0.call((D::CancelAuthenticationChallenge, none))
                }
                _ => completion.0.call((D::PerformDefaultHandling, none)),
            },
            Pending::Auth { completion } => match answer {
                Answer::Credential { user, password } => {
                    let credential = NSURLCredential::credentialWithUser_password_persistence(
                        &NSString::from_str(&user),
                        &NSString::from_str(&password),
                        NSURLCredentialPersistence::ForSession,
                    );
                    completion
                        .0
                        .call((D::UseCredential, Retained::as_ptr(&credential) as *mut _));
                }
                Answer::Cancel => completion.0.call((D::CancelAuthenticationChallenge, none)),
                _ => completion.0.call((D::PerformDefaultHandling, none)),
            },
        }
    }
}

struct AppleTransfer {
    task: Shared<Retained<NSURLSessionTask>>,
    state: Mutex<TransferState>,
}

struct TransferState {
    events: Option<Events>,
    demand: u32,
    suspended: bool,
    questions: HashMap<QuestionId, Pending>,
    next_question: QuestionId,
    /// The client refused the server, so the task's cancellation is a TLS failure.
    refused_trust: bool,
}

impl AppleTransfer {
    fn emit(&self, event: Event) {
        let events = lock(&self.state).events.clone();
        if let Some(events) = events {
            events(event);
        }
    }

    fn data(&self, bytes: Vec<u8>) {
        let events = {
            let mut st = lock(&self.state);
            st.demand = st.demand.saturating_sub(1);
            if st.demand == 0 && !st.suspended && st.events.is_some() {
                st.suspended = true;
                self.task.0.suspend();
            }
            st.events.clone()
        };
        if let Some(events) = events {
            events(Event::Chunk(bytes));
        }
    }

    fn ask(&self, question: Question, pending: Pending) {
        let asked = {
            let mut st = lock(&self.state);
            match st.events.clone() {
                Some(events) => {
                    st.next_question += 1;
                    let id = st.next_question;
                    st.questions.insert(id, pending);
                    Ok((events, id))
                }
                None => Err(pending),
            }
        };
        match asked {
            Ok((events, id)) => events(Event::Question(id, question)),
            Err(pending) => pending.resolve(Answer::DefaultHandling),
        }
    }

    fn finish(&self, error: Option<&NSError>) {
        let (events, refused, pending) = {
            let mut st = lock(&self.state);
            (
                st.events.take(),
                st.refused_trust,
                std::mem::take(&mut st.questions),
            )
        };
        for (_, question) in pending {
            question.resolve(Answer::Cancel);
        }
        let Some(events) = events else {
            return;
        };
        match error {
            None => events(Event::End),
            Some(e) if refused && e.code() == CANCELLED => events(Event::Failed(HttpError::Tls(
                "the server is not trusted".into(),
            ))),
            Some(e) => events(Event::Failed(map_error(e))),
        }
    }
}

impl Transfer for AppleTransfer {
    fn demand(&self, chunks: u32) {
        let mut st = lock(&self.state);
        st.demand += chunks;
        if st.suspended && st.demand > 0 {
            st.suspended = false;
            self.task.0.resume();
        }
    }

    fn answer(&self, id: QuestionId, answer: Answer) {
        let pending = {
            let mut st = lock(&self.state);
            let pending = st.questions.remove(&id);
            if matches!(pending, Some(Pending::Trust { .. })) && answer == Answer::Reject {
                st.refused_trust = true;
            }
            pending
        };
        if let Some(pending) = pending {
            pending.resolve(answer);
        }
    }

    fn cancel(&self) {
        let pending = std::mem::take(&mut lock(&self.state).questions);
        for (_, question) in pending {
            question.resolve(Answer::Cancel);
        }
        self.task.0.cancel();
    }
}

/// A socket that never opened; its failure was already reported.
struct ClosedSocket;

impl Socket for ClosedSocket {
    fn send(&self, _message: Message, done: Completion) {
        done(Err(HttpError::Io("the WebSocket is closed".into())));
    }
    fn ping(&self, done: Completion) {
        done(Err(HttpError::Io("the WebSocket is closed".into())));
    }
    fn close(&self, _code: u16, _reason: &str) {}
    fn demand(&self, _messages: u32) {}
}

struct AppleSocket {
    me: Weak<AppleSocket>,
    task: Shared<Retained<NSURLSessionWebSocketTask>>,
    state: Mutex<SocketState>,
}

struct SocketState {
    events: Option<WsEvents>,
    demand: u32,
    receiving: bool,
    open: bool,
}

impl AppleSocket {
    fn opened(&self, protocol: Option<String>) {
        let events = {
            let mut st = lock(&self.state);
            st.open = true;
            st.events.clone()
        };
        if let Some(events) = events {
            events(WsEvent::Open { protocol });
        }
        self.receive();
    }

    /// Ask for the next message while demand remains and no receive is in flight.
    fn receive(&self) {
        {
            let mut st = lock(&self.state);
            if st.receiving || st.demand == 0 || !st.open || st.events.is_none() {
                return;
            }
            st.receiving = true;
        }
        let Some(me) = self.me.upgrade() else {
            return;
        };
        let handler = RcBlock::new(
            move |message: *mut NSURLSessionWebSocketMessage, _error: *mut NSError| {
                // SAFETY: URLSession passes a valid message or null for the call's duration.
                me.received(unsafe { message.as_ref() });
            },
        );
        // SAFETY: the handler captures only Send + Sync state and may run on any thread.
        unsafe { self.task.0.receiveMessageWithCompletionHandler(&handler) };
    }

    fn received(&self, message: Option<&NSURLSessionWebSocketMessage>) {
        let events = {
            let mut st = lock(&self.state);
            st.receiving = false;
            if message.is_some() {
                st.demand = st.demand.saturating_sub(1);
            }
            st.events.clone()
        };
        // A receive that fails means the connection ended; the close or completion message
        // reports how.
        let Some(message) = message else {
            return;
        };
        let message = if message.r#type() == NSURLSessionWebSocketMessageType::String {
            Message::Text(message.string().map(|s| s.to_string()).unwrap_or_default())
        } else {
            Message::Binary(message.data().map(|d| d.to_vec()).unwrap_or_default())
        };
        if let Some(events) = events {
            events(WsEvent::Message(message));
        }
        self.receive();
    }

    fn closed(&self, code: isize, reason: Option<&NSData>) {
        let events = lock(&self.state).events.take();
        if let Some(events) = events {
            events(WsEvent::Closed {
                code: code.clamp(0, u16::MAX as isize) as u16,
                reason: reason
                    .map(|r| String::from_utf8_lossy(&r.to_vec()).into_owned())
                    .unwrap_or_default(),
            });
        }
    }

    fn finish(&self, error: Option<&NSError>) {
        let code = self.task.0.closeCode().0;
        if code != NSURLSessionWebSocketCloseCode::Invalid.0 {
            return self.closed(code, self.task.0.closeReason().as_deref());
        }
        let events = lock(&self.state).events.take();
        if let Some(events) = events {
            events(match error {
                Some(e) => WsEvent::Failed(map_error(e)),
                None => WsEvent::Closed {
                    code: 1006,
                    reason: String::new(),
                },
            });
        }
    }
}

impl Socket for AppleSocket {
    fn send(&self, message: Message, done: Completion) {
        let native = match message {
            Message::Text(text) => NSURLSessionWebSocketMessage::initWithString(
                NSURLSessionWebSocketMessage::alloc(),
                &NSString::from_str(&text),
            ),
            Message::Binary(bytes) => NSURLSessionWebSocketMessage::initWithData(
                NSURLSessionWebSocketMessage::alloc(),
                &NSData::with_bytes(&bytes),
            ),
            Message::Close { code, reason } => {
                self.close(code, &reason);
                return done(Ok(()));
            }
        };
        let done = Mutex::new(Some(done));
        let handler = RcBlock::new(move |error: *mut NSError| {
            if let Some(done) = lock(&done).take() {
                // SAFETY: URLSession passes a valid error or null for the call's duration.
                done(match unsafe { error.as_ref() } {
                    None => Ok(()),
                    Some(e) => Err(map_error(e)),
                });
            }
        });
        // SAFETY: the handler captures only Send state and may run on any thread.
        unsafe { self.task.0.sendMessage_completionHandler(&native, &handler) };
    }

    fn ping(&self, done: Completion) {
        let done = Mutex::new(Some(done));
        let handler = RcBlock::new(move |error: *mut NSError| {
            if let Some(done) = lock(&done).take() {
                // SAFETY: as in `send`.
                done(match unsafe { error.as_ref() } {
                    None => Ok(()),
                    Some(e) => Err(map_error(e)),
                });
            }
        });
        // SAFETY: as in `send`.
        unsafe { self.task.0.sendPingWithPongReceiveHandler(&handler) };
    }

    fn close(&self, code: u16, reason: &str) {
        self.task.0.cancelWithCloseCode_reason(
            NSURLSessionWebSocketCloseCode(code as isize),
            Some(&NSData::with_bytes(reason.as_bytes())),
        );
    }

    fn demand(&self, messages: u32) {
        lock(&self.state).demand += messages;
        self.receive();
    }
}

impl Routes {
    fn transfer(&self, task: &NSURLSessionTask) -> Option<Arc<AppleTransfer>> {
        lock(&self.transfers).get(&task.taskIdentifier()).cloned()
    }

    fn socket(&self, task: &NSURLSessionTask) -> Option<Arc<AppleSocket>> {
        lock(&self.sockets).get(&task.taskIdentifier()).cloned()
    }

    fn challenge(
        &self,
        task: &NSURLSessionTask,
        challenge: &NSURLAuthenticationChallenge,
        completion: &block2::DynBlock<
            dyn Fn(NSURLSessionAuthChallengeDisposition, *mut NSURLCredential),
        >,
    ) {
        use NSURLSessionAuthChallengeDisposition as D;
        let default = || completion.call((D::PerformDefaultHandling, std::ptr::null_mut()));
        let space = challenge.protectionSpace();
        let method = space.authenticationMethod();
        let is = |constant: &NSString| method.isEqualToString(constant);

        // SAFETY (each constant below): Foundation's authentication method strings, immutable
        // for the life of the process.
        if is(unsafe { NSURLAuthenticationMethodServerTrust }) {
            let Some(transfer) = self.transfer(task) else {
                return default();
            };
            if !self.config.ask_trust {
                return default();
            }
            // SAFETY: a server trust protection space answers `serverTrust` with its SecTrustRef,
            // valid while the challenge lives; retaining it keeps it for the answer.
            let trust: *mut SecTrust = unsafe { msg_send![&*space, serverTrust] };
            let Some(trust) = NonNull::new(trust) else {
                return default();
            };
            let trust = unsafe { CFRetained::retain(trust) };
            let mut error: *mut CFError = std::ptr::null_mut();
            // SAFETY: `trust` is valid; `error` receives a +1 CFError or stays null.
            let trusted = unsafe { trust.evaluate_with_error(&mut error) };
            let system_error = NonNull::new(error).map(|e| {
                // SAFETY: SecTrustEvaluateWithError returns the error retained (Create rule).
                let e = unsafe { CFRetained::from_raw(e) };
                e.description()
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| "the certificate is not trusted".into())
            });
            let chain = certificate_chain(&trust);
            let question = Question::ServerTrust(ServerTrust {
                host: space.host().to_string(),
                chain,
                system_trusted: trusted,
                system_error,
            });
            return transfer.ask(
                question,
                Pending::Trust {
                    completion: Shared(completion.copy()),
                    trust: Shared(trust),
                },
            );
        }

        if is(unsafe { NSURLAuthenticationMethodClientCertificate }) {
            return match self.identity_credential() {
                Some(credential) => completion.call((
                    D::UseCredential,
                    Retained::as_ptr(&credential.0) as *mut NSURLCredential,
                )),
                None => default(),
            };
        }

        let scheme = if is(unsafe { NSURLAuthenticationMethodHTTPBasic }) {
            Scheme::Basic
        } else if is(unsafe { NSURLAuthenticationMethodHTTPDigest }) {
            Scheme::Digest
        } else if is(unsafe { NSURLAuthenticationMethodNTLM }) {
            Scheme::Ntlm
        } else if is(unsafe { NSURLAuthenticationMethodNegotiate }) {
            Scheme::Negotiate
        } else {
            return default();
        };
        let Some(transfer) = self.transfer(task) else {
            return default();
        };
        if !self.config.ask_auth {
            return default();
        }
        let question = Question::Auth(AuthQuestion {
            host: space.host().to_string(),
            port: space.port().clamp(0, u16::MAX as isize) as u16,
            realm: space.realm().map(|r| r.to_string()),
            scheme,
            proxy: space.isProxy(),
            previous_failures: challenge.previousFailureCount().max(0) as u32,
        });
        transfer.ask(
            question,
            Pending::Auth {
                completion: Shared(completion.copy()),
            },
        );
    }

    /// The configured identity as a credential, imported on first use.
    fn identity_credential(&self) -> Option<Shared<Retained<NSURLCredential>>> {
        let identity = self.config.identity.as_ref()?;
        let mut cached = lock(&self.identity);
        if cached.is_none() {
            *cached = import_identity(identity).map(Shared);
        }
        cached.as_ref().map(|c| Shared(c.0.clone()))
    }

    fn complete(&self, task: &NSURLSessionTask, error: Option<&NSError>) {
        let id = task.taskIdentifier();
        let transfer = lock(&self.transfers).remove(&id);
        if let Some(transfer) = transfer {
            return transfer.finish(error);
        }
        let socket = lock(&self.sockets).remove(&id);
        if let Some(socket) = socket {
            socket.finish(error);
        }
    }
}

fn certificate_chain(trust: &SecTrust) -> Vec<Vec<u8>> {
    // SAFETY: `trust` was evaluated; the copied chain is ours to read.
    let Some(chain) = (unsafe { trust.certificate_chain() }) else {
        return Vec::new();
    };
    // SAFETY: SecTrustCopyCertificateChain documents an array of SecCertificateRef.
    let chain = unsafe { chain.cast_unchecked::<SecCertificate>() };
    (0..chain.len())
        .filter_map(|i| chain.get(i))
        // SAFETY: a valid certificate from the chain.
        .map(|certificate| unsafe { certificate.data() }.to_vec())
        .collect()
}

/// Import a PKCS#12 identity as a session credential.
fn import_identity(identity: &Identity) -> Option<Retained<NSURLCredential>> {
    let data = CFData::from_bytes(&identity.pkcs12);
    let password = CFString::from_str(&identity.password);
    // SAFETY: Security's import option key, immutable for the process.
    let key: &CFString = unsafe { kSecImportExportPassphrase };
    let options = CFDictionary::<CFString, CFString>::from_slices(&[key], &[&password]);
    let mut items: *const objc2_core_foundation::CFArray = std::ptr::null();
    // SAFETY: valid data and options; `items` receives a +1 array on success.
    let status = unsafe { SecPKCS12Import(&data, options.as_opaque(), NonNull::from(&mut items)) };
    let items = NonNull::new(items as *mut objc2_core_foundation::CFArray)?;
    // SAFETY: SecPKCS12Import returns the array retained (Create rule).
    let items = unsafe { CFRetained::from_raw(items) };
    if status != 0 {
        return None;
    }
    // SAFETY: SecPKCS12Import documents an array of dictionaries.
    let items = unsafe { items.cast_unchecked::<CFDictionary>() };
    let first = items.get(0)?;
    // SAFETY: Security's item key, immutable for the process.
    let identity_key: &CFString = unsafe { kSecImportItemIdentity };
    let key_ptr: *const CFString = identity_key;
    // SAFETY: a CFDictionary lookup by a CFString key; the value is a SecIdentityRef owned by
    // the dictionary, which `items` keeps alive through the credential's creation.
    let value = unsafe { first.value(key_ptr.cast()) };
    let identity = NonNull::new(value as *mut SecIdentity)?;
    // SAFETY: `credentialWithIdentity:certificates:persistence:` takes a SecIdentityRef and an
    // optional certificate array; the credential retains the identity.
    let credential: Retained<NSURLCredential> = unsafe {
        msg_send![
            NSURLCredential::class(),
            credentialWithIdentity: identity.as_ptr(),
            certificates: std::ptr::null::<AnyObject>(),
            persistence: NSURLCredentialPersistence::ForSession
        ]
    };
    Some(credential)
}

/// The response head URLSession delivered.
fn head_of(response: &NSURLResponse) -> Head {
    let url = response
        .URL()
        .and_then(|u| u.absoluteString())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let mut expected = u64::try_from(response.expectedContentLength()).ok();
    let Some(http) = response.downcast_ref::<NSHTTPURLResponse>() else {
        return Head {
            status: 0,
            headers: Vec::new(),
            url,
            expected_length: expected,
        };
    };
    let status = http.statusCode().clamp(0, u16::MAX as isize) as u16;
    let fields = http.allHeaderFields();
    let mut headers = Vec::new();
    let mut cookies = false;
    for key in fields.allKeys() {
        let Some(value) = fields.objectForKey(&*key) else {
            continue;
        };
        let (name, value) = (object_string(&key), object_string(&value));
        if name.eq_ignore_ascii_case("set-cookie") {
            cookies = true;
            continue;
        }
        // URLSession decodes compressed bodies itself, so the declared length is not the length
        // the reader receives.
        if name.eq_ignore_ascii_case("content-encoding") && !value.eq_ignore_ascii_case("identity")
        {
            expected = None;
        }
        headers.push((name, value));
    }
    if cookies && let Some(url) = response.URL() {
        // URLSession joins repeated Set-Cookie headers with commas, which cookie dates also
        // contain; Foundation's own parser splits them back into cookies.
        let fields_ptr: *const NSDictionary = &*fields;
        // SAFETY: allHeaderFields documents string keys and string values.
        let typed = unsafe { &*(fields_ptr as *const NSDictionary<NSString, NSString>) };
        for cookie in NSHTTPCookie::cookiesWithResponseHeaderFields_forURL(typed, &url).iter() {
            headers.push(("Set-Cookie".to_string(), set_cookie_line(&cookie)));
        }
    }
    Head {
        status,
        headers,
        url,
        expected_length: expected,
    }
}

fn object_string(object: &AnyObject) -> String {
    object
        .downcast_ref::<NSString>()
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn system_time(date: &NSDate) -> Option<SystemTime> {
    let seconds = date.timeIntervalSince1970();
    if seconds >= 0.0 {
        UNIX_EPOCH.checked_add(Duration::from_secs_f64(seconds))
    } else {
        Some(UNIX_EPOCH)
    }
}

fn set_cookie_line(cookie: &NSHTTPCookie) -> String {
    let mut line = format!("{}={}", cookie.name(), cookie.value());
    let domain = cookie.domain().to_string();
    if let Some(domain) = domain.strip_prefix('.') {
        line.push_str(&format!("; Domain={domain}"));
    }
    line.push_str(&format!("; Path={}", cookie.path()));
    if let Some(expires) = cookie.expiresDate().as_deref().and_then(system_time) {
        line.push_str(&format!("; Expires={}", httpdate::fmt_http_date(expires)));
    }
    if cookie.isSecure() {
        line.push_str("; Secure");
    }
    if cookie.isHTTPOnly() {
        line.push_str("; HttpOnly");
    }
    line
}

fn cookie_of(cookie: &NSHTTPCookie) -> Cookie {
    let domain = cookie.domain().to_string();
    Cookie {
        name: cookie.name().to_string(),
        value: cookie.value().to_string(),
        host_only: !domain.starts_with('.'),
        domain: domain.trim_start_matches('.').to_string(),
        path: cookie.path().to_string(),
        expires: cookie.expiresDate().as_deref().and_then(system_time),
        secure: cookie.isSecure(),
        http_only: cookie.isHTTPOnly(),
    }
}

fn metrics_of(metrics: &NSURLSessionTaskMetrics) -> Metrics {
    let transactions = metrics.transactionMetrics();
    let Some(last) = transactions.lastObject() else {
        return Metrics::default();
    };
    let span = |start: Option<Retained<NSDate>>, end: Option<Retained<NSDate>>| match (start, end) {
        (Some(start), Some(end)) => Some(Duration::from_secs_f64(
            end.timeIntervalSinceDate(&start).max(0.0),
        )),
        _ => None,
    };
    let tls_version = last
        .negotiatedTLSProtocolVersion()
        .map(|v| match v.unsignedShortValue() {
            0x0304 => "TLS 1.3".to_string(),
            0x0303 => "TLS 1.2".to_string(),
            0x0302 => "TLS 1.1".to_string(),
            0x0301 => "TLS 1.0".to_string(),
            other => format!("0x{other:04x}"),
        });
    Metrics {
        dns: span(last.domainLookupStartDate(), last.domainLookupEndDate()),
        connect: span(last.connectStartDate(), last.connectEndDate()),
        tls: span(
            last.secureConnectionStartDate(),
            last.secureConnectionEndDate(),
        ),
        first_byte: span(last.fetchStartDate(), last.responseStartDate()),
        total: span(
            transactions
                .firstObject()
                .and_then(|first| first.fetchStartDate()),
            last.responseEndDate(),
        ),
        protocol: last.networkProtocolName().map(|p| p.to_string()),
        reused_connection: Some(last.isReusedConnection()),
        proxy: Some(last.isProxyConnection()),
        remote_address: last.remoteAddress().map(|a| a.to_string()),
        tls_version,
        from_cache: last.resourceFetchType()
            == NSURLSessionTaskMetricsResourceFetchType::LocalCache,
        bytes_sent: u64::try_from(
            last.countOfRequestHeaderBytesSent() + last.countOfRequestBodyBytesSent(),
        )
        .ok(),
        bytes_received: u64::try_from(
            last.countOfResponseHeaderBytesReceived() + last.countOfResponseBodyBytesReceived(),
        )
        .ok(),
        redirects: 0,
    }
}

const CANCELLED: isize = -999;

/// Map an NSURLErrorDomain error onto the portable taxonomy (docs/http.md).
fn map_error(err: &NSError) -> HttpError {
    const TIMED_OUT: isize = -1001;
    const CANNOT_FIND_HOST: isize = -1003;
    const DNS_FAILED: isize = -1006;
    const CANNOT_CONNECT: isize = -1004;
    const NOT_CONNECTED: isize = -1009;
    const BAD_URL: isize = -1000;
    const UNSUPPORTED_URL: isize = -1002;
    const TOO_MANY_REDIRECTS: isize = -1007;
    match err.code() {
        TIMED_OUT => HttpError::Timeout,
        CANNOT_FIND_HOST | DNS_FAILED => HttpError::Dns,
        CANNOT_CONNECT | NOT_CONNECTED => HttpError::Connect,
        CANCELLED => HttpError::Cancelled,
        TOO_MANY_REDIRECTS => HttpError::TooManyRedirects,
        c @ -1206..=-1200 => HttpError::Tls(format!("{} ({c})", err.localizedDescription())),
        BAD_URL | UNSUPPORTED_URL => HttpError::BadUrl(err.localizedDescription().to_string()),
        _ => HttpError::Io(err.localizedDescription().to_string()),
    }
}

struct DelegateIvars {
    routes: Arc<Routes>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = AllocAnyThread]
    #[name = "DayHttpSessionDelegate"]
    #[ivars = DelegateIvars]
    struct SessionDelegate;

    unsafe impl NSObjectProtocol for SessionDelegate {}
    unsafe impl NSURLSessionDelegate for SessionDelegate {}

    unsafe impl NSURLSessionTaskDelegate for SessionDelegate {
        #[unsafe(method(URLSession:task:willPerformHTTPRedirection:newRequest:completionHandler:))]
        fn will_redirect(
            &self,
            _session: &NSURLSession,
            _task: &NSURLSessionTask,
            _response: &NSHTTPURLResponse,
            _request: &NSURLRequest,
            completion: &block2::DynBlock<dyn Fn(*mut NSURLRequest)>,
        ) {
            // The client follows redirects itself: declining makes the 3xx the response.
            completion.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(URLSession:task:didReceiveChallenge:completionHandler:))]
        fn did_receive_challenge(
            &self,
            _session: &NSURLSession,
            task: &NSURLSessionTask,
            challenge: &NSURLAuthenticationChallenge,
            completion: &block2::DynBlock<
                dyn Fn(NSURLSessionAuthChallengeDisposition, *mut NSURLCredential),
            >,
        ) {
            self.ivars().routes.challenge(task, challenge, completion);
        }

        #[unsafe(method(URLSession:task:didSendBodyData:totalBytesSent:totalBytesExpectedToSend:))]
        fn did_send_body_data(
            &self,
            _session: &NSURLSession,
            task: &NSURLSessionTask,
            _bytes_sent: i64,
            total_sent: i64,
            total_expected: i64,
        ) {
            if let Some(transfer) = self.ivars().routes.transfer(task) {
                transfer.emit(Event::Sent {
                    sent: u64::try_from(total_sent).unwrap_or(0),
                    total: u64::try_from(total_expected).ok().filter(|t| *t > 0),
                });
            }
        }

        #[unsafe(method(URLSession:task:needNewBodyStream:))]
        fn need_new_body_stream(
            &self,
            _session: &NSURLSession,
            _task: &NSURLSessionTask,
            completion: &block2::DynBlock<dyn Fn(*mut NSInputStream)>,
        ) {
            // A stream body is sent once; the client re-sends only bodies it can rebuild.
            completion.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(URLSession:task:didFinishCollectingMetrics:))]
        fn did_finish_collecting_metrics(
            &self,
            _session: &NSURLSession,
            task: &NSURLSessionTask,
            metrics: &NSURLSessionTaskMetrics,
        ) {
            if let Some(transfer) = self.ivars().routes.transfer(task) {
                transfer.emit(Event::Metrics(metrics_of(metrics)));
            }
        }

        #[unsafe(method(URLSession:task:didCompleteWithError:))]
        fn did_complete(
            &self,
            _session: &NSURLSession,
            task: &NSURLSessionTask,
            error: Option<&NSError>,
        ) {
            self.ivars().routes.complete(task, error);
        }
    }

    unsafe impl NSURLSessionDataDelegate for SessionDelegate {
        #[unsafe(method(URLSession:dataTask:didReceiveResponse:completionHandler:))]
        fn did_receive_response(
            &self,
            _session: &NSURLSession,
            task: &NSURLSessionDataTask,
            response: &NSURLResponse,
            completion: &block2::DynBlock<dyn Fn(NSURLSessionResponseDisposition)>,
        ) {
            if let Some(transfer) = self.ivars().routes.transfer(task) {
                transfer.emit(Event::Head(head_of(response)));
            }
            completion.call((NSURLSessionResponseDisposition::Allow,));
        }

        #[unsafe(method(URLSession:dataTask:didReceiveData:))]
        fn did_receive_data(
            &self,
            _session: &NSURLSession,
            task: &NSURLSessionDataTask,
            data: &NSData,
        ) {
            if let Some(transfer) = self.ivars().routes.transfer(task) {
                transfer.data(data.to_vec());
            }
        }
    }

    unsafe impl NSURLSessionWebSocketDelegate for SessionDelegate {
        #[unsafe(method(URLSession:webSocketTask:didOpenWithProtocol:))]
        fn did_open(
            &self,
            _session: &NSURLSession,
            task: &NSURLSessionWebSocketTask,
            protocol: Option<&NSString>,
        ) {
            if let Some(socket) = self.ivars().routes.socket(task) {
                socket.opened(protocol.map(|p| p.to_string()).filter(|p| !p.is_empty()));
            }
        }

        #[unsafe(method(URLSession:webSocketTask:didCloseWithCode:reason:))]
        fn did_close(
            &self,
            _session: &NSURLSession,
            task: &NSURLSessionWebSocketTask,
            code: NSURLSessionWebSocketCloseCode,
            reason: Option<&NSData>,
        ) {
            if let Some(socket) = self.ivars().routes.socket(task) {
                socket.closed(code.0, reason);
            }
        }
    }
);

impl SessionDelegate {
    fn new(routes: Arc<Routes>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars { routes });
        // SAFETY: NSObject's designated initializer on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }
}
