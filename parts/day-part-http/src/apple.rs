// ---------------------------------------------------------------------------
// macOS + iOS: NSURLSession — the system networking stack (proxies + PAC, VPN, Low Data Mode,
// the platform TLS + keychain trust store). ONE shared session with an EPHEMERAL configuration
// (no shared cookie jar / disk cache — stateless fetch semantics matching the other backends);
// the expensive/constrained knobs are per-request NSURLRequest properties (iOS 13+/macOS 10.15+,
// below every Day floor). NSURLSession is documented thread-safe, and its completion handlers
// run on the session's own delegate queue — never the main thread — so the sync bridge (an mpsc
// channel the block sends into) cannot deadlock the UI even when misused from it.
// ---------------------------------------------------------------------------

use std::path::Path;
use std::sync::OnceLock;
use std::sync::mpsc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{
    NSData, NSError, NSHTTPURLResponse, NSMutableURLRequest, NSString, NSURL, NSURLResponse,
    NSURLSession, NSURLSessionConfiguration,
};

use super::{Download, HttpError, Request, Response, Tier};

pub const TIER: Tier = Tier::NativeStack;

/// The shared ephemeral session. NSURLSession is thread-safe per Apple's documentation.
struct SharedSession(Retained<NSURLSession>);
// SAFETY: NSURLSession instances are documented thread-safe ("thread safe ... methods can be
// called from any thread"); the Retained is never mutated after creation.
unsafe impl Send for SharedSession {}
unsafe impl Sync for SharedSession {}

fn session() -> &'static NSURLSession {
    static SESSION: OnceLock<SharedSession> = OnceLock::new();
    &SESSION
        .get_or_init(|| {
            let config = NSURLSessionConfiguration::ephemeralSessionConfiguration();
            SharedSession(NSURLSession::sessionWithConfiguration(&config))
        })
        .0
}

/// Build the native request, or the portable BadUrl error.
fn build_request(req: &Request) -> Result<Retained<NSMutableURLRequest>, HttpError> {
    let url = NSURL::URLWithString(&NSString::from_str(&req.url))
        .filter(|u| u.scheme().is_some())
        .ok_or_else(|| HttpError::BadUrl(req.url.clone()))?;
    let native = NSMutableURLRequest::requestWithURL(&url);
    native.setHTTPMethod(&NSString::from_str(req.method.as_str()));
    native.setTimeoutInterval(req.timeout.as_secs_f64());
    native.setAllowsExpensiveNetworkAccess(req.allow_expensive);
    native.setAllowsConstrainedNetworkAccess(req.allow_constrained);
    for (k, v) in &req.headers {
        native.addValue_forHTTPHeaderField(&NSString::from_str(v), &NSString::from_str(k));
    }
    if let Some(body) = &req.body {
        native.setHTTPBody(Some(&NSData::from_vec(body.clone())));
    }
    Ok(native)
}

/// Map an NSURLErrorDomain error onto the portable taxonomy (docs/http.md).
fn map_error(err: &NSError) -> HttpError {
    const TIMED_OUT: isize = -1001;
    const CANNOT_FIND_HOST: isize = -1003;
    const DNS_FAILED: isize = -1006;
    const CANNOT_CONNECT: isize = -1004;
    const NOT_CONNECTED: isize = -1009;
    const BAD_URL: isize = -1000;
    const UNSUPPORTED_URL: isize = -1002;
    const CANCELLED: isize = -999;
    match err.code() {
        TIMED_OUT => HttpError::Timeout,
        CANNOT_FIND_HOST | DNS_FAILED => HttpError::Dns,
        CANNOT_CONNECT | NOT_CONNECTED => HttpError::Connect,
        CANCELLED => HttpError::Cancelled,
        c @ -1206..=-1200 => HttpError::Tls(format!("{} ({c})", err.localizedDescription())),
        BAD_URL | UNSUPPORTED_URL => HttpError::BadUrl(err.localizedDescription().to_string()),
        _ => HttpError::Io(err.localizedDescription().to_string()),
    }
}

/// Status + headers out of the (HTTP) response object.
fn map_response(resp: &NSURLResponse) -> (u16, Vec<(String, String)>) {
    let Some(http) = resp.downcast_ref::<NSHTTPURLResponse>() else {
        return (0, Vec::new());
    };
    let status = http.statusCode().max(0) as u16;
    let mut headers = Vec::new();
    let dict = http.allHeaderFields();
    for key in dict.allKeys() {
        if let Some(value) = dict.objectForKey(&*key) {
            headers.push((obj_to_string(&key), obj_to_string(&value)));
        }
    }
    (status, headers)
}

fn obj_to_string(obj: &AnyObject) -> String {
    obj.downcast_ref::<NSString>()
        .map(|s| s.to_string())
        .unwrap_or_default()
}

type FetchResult = Result<Response, HttpError>;

/// Start a data task whose completion maps the native result and hands it to `deliver`.
/// Returns the task handle for cancellation; `None` = the request never started (`BadUrl`
/// already delivered). Dropping the returned `Retained` does NOT cancel — the session retains
/// its tasks (which is why the pre-cancellation callers could drop it for years).
fn start_data_task(
    req: &Request,
    deliver: impl Fn(FetchResult) + Send + 'static,
) -> Option<Retained<NSURLSessionDataTask>> {
    let native = match build_request(req) {
        Ok(n) => n,
        Err(e) => {
            deliver(Err(e));
            return None;
        }
    };
    let handler = RcBlock::new(
        move |data: *mut NSData, resp: *mut NSURLResponse, err: *mut NSError| {
            // SAFETY: URLSession hands valid (or null) object pointers, live for the call.
            let result = if let Some(err) = unsafe { err.as_ref() } {
                Err(map_error(err))
            } else {
                let (status, headers) = unsafe { resp.as_ref() }
                    .map(map_response)
                    .unwrap_or((0, Vec::new()));
                let body = unsafe { data.as_ref() }
                    .map(|d| d.to_vec())
                    .unwrap_or_default();
                Ok(Response {
                    status,
                    headers,
                    body,
                })
            };
            deliver(result);
        },
    );
    // SAFETY: the block is sendable (captures only Send values, required by the signature).
    let task = unsafe { session().dataTaskWithRequest_completionHandler(&native, &handler) };
    task.resume();
    Some(task)
}

pub fn fetch(req: &Request) -> Result<Response, HttpError> {
    let (tx, rx) = mpsc::channel::<FetchResult>();
    let _ = start_data_task(req, move |result| {
        let _ = tx.send(result);
    });
    // URLSession enforces the request timeout itself (timeoutInterval is an IDLE timer, so a
    // long transfer that keeps moving stays alive) and always calls the completion — including
    // on timeout (-1001). Waiting without a cap of our own keeps big slow downloads working.
    match rx.recv() {
        Ok(result) => result,
        Err(_) => Err(HttpError::Io("completion never delivered".into())),
    }
}

/// Natively async: the completion (and `on_done`) runs on the session's delegate queue.
pub fn fetch_async(req: Request, on_done: Box<dyn FnOnce(FetchResult) + Send>) {
    let _ = fetch_async_cancellable(req, on_done);
}

/// A cancel grip on an in-flight data task, callable from whichever thread drops a FetchFuture.
struct SendTask(Retained<NSURLSessionDataTask>);
// SAFETY: NSURLSessionTask is documented thread-safe — the SharedSession rationale above; the
// Retained is used only to call `cancel()` (and to release the reference afterwards).
unsafe impl Send for SendTask {}

/// [`fetch_async`] plus a cancel closure: invoking it cancels the in-flight task, and the
/// completion then arrives exactly once with `NSURLErrorCancelled` → [`HttpError::Cancelled`].
/// `None` = the request never started (`BadUrl` was already delivered to `on_done`).
pub fn fetch_async_cancellable(
    req: Request,
    on_done: Box<dyn FnOnce(FetchResult) + Send>,
) -> Option<Box<dyn FnOnce() + Send>> {
    // FnOnce → Fn bridge: URLSession calls a completion handler exactly once.
    let once = std::sync::Mutex::new(Some(on_done));
    let task = start_data_task(&req, move |result| {
        if let Some(f) = once.lock().ok().and_then(|mut g| g.take()) {
            f(result);
        }
    })?;
    let grip = SendTask(task);
    Some(Box::new(move || grip.0.cancel()))
}

pub fn fetch_to_file(req: &Request, dest: &Path) -> Result<Download, HttpError> {
    let native = build_request(req)?;
    let (tx, rx) = mpsc::channel::<Result<Download, HttpError>>();
    let dest = dest.to_path_buf();
    let handler = RcBlock::new(
        move |location: *mut NSURL, resp: *mut NSURLResponse, err: *mut NSError| {
            let result = if let Some(err) = unsafe { err.as_ref() } {
                Err(map_error(err))
            } else {
                let (status, headers) = unsafe { resp.as_ref() }
                    .map(map_response)
                    .unwrap_or((0, Vec::new()));
                // The temp file is DELETED when this block returns — move it now. rename first
                // (atomic, same volume), copy+remove as the cross-volume fallback.
                let moved = unsafe { location.as_ref() }
                    .and_then(|l| l.path())
                    .map(|p| std::path::PathBuf::from(p.to_string()))
                    .ok_or_else(|| HttpError::Io("download produced no file".into()))
                    .and_then(|tmp| {
                        std::fs::rename(&tmp, &dest)
                            .or_else(|_| {
                                std::fs::copy(&tmp, &dest)
                                    .map(|_| ())
                                    .and_then(|()| std::fs::remove_file(&tmp))
                            })
                            .map_err(|e| HttpError::Io(e.to_string()))?;
                        std::fs::metadata(&dest)
                            .map(|m| m.len())
                            .map_err(|e| HttpError::Io(e.to_string()))
                    });
                moved.map(|bytes_written| Download {
                    status,
                    headers,
                    bytes_written,
                })
            };
            let _ = tx.send(result);
        },
    );
    // SAFETY: sendable block; see `start_data_task`.
    let task = unsafe { session().downloadTaskWithRequest_completionHandler(&native, &handler) };
    task.resume();
    // Same waiting rule as `fetch`: URLSession's idle-based timeout owns the clock, so a
    // multi-minute download is never cut off by a total-time cap here.
    match rx.recv() {
        Ok(result) => result,
        Err(_) => Err(HttpError::Io("completion never delivered".into())),
    }
}

// ---------------------------------------------------------------------------
// Streaming (fetch_streamed): a data-task DELEGATE session — didReceiveData hands chunks over a
// channel as they arrive, so a large body is never buffered (progress/cancel/hash sinks,
// docs/http.md). One session per streamed request (the delegate is per-transfer state);
// finishTasksAndInvalidate releases it.
// ---------------------------------------------------------------------------

use objc2::runtime::ProtocolObject;
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
use objc2_foundation::{
    NSObject, NSObjectProtocol, NSURLSessionDataDelegate, NSURLSessionDataTask,
    NSURLSessionDelegate, NSURLSessionResponseDisposition, NSURLSessionTask,
    NSURLSessionTaskDelegate,
};

enum StreamMsg {
    Head(u16, Vec<(String, String)>),
    Chunk(Vec<u8>),
    Done(Option<HttpError>),
}

struct StreamIvars {
    // Sender is Send but not Sync; the delegate must be Sync (NSURLSessionDelegate supertrait).
    tx: std::sync::Mutex<mpsc::Sender<StreamMsg>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = AllocAnyThread]
    #[name = "DayHttpStreamDelegate"]
    #[ivars = StreamIvars]
    struct StreamDelegate;

    unsafe impl NSObjectProtocol for StreamDelegate {}
    unsafe impl NSURLSessionDelegate for StreamDelegate {}

    unsafe impl NSURLSessionTaskDelegate for StreamDelegate {
        #[unsafe(method(URLSession:task:didCompleteWithError:))]
        fn did_complete(
            &self,
            _session: &NSURLSession,
            _task: &NSURLSessionTask,
            error: Option<&NSError>,
        ) {
            self.send(StreamMsg::Done(error.map(map_error)));
        }
    }

    unsafe impl NSURLSessionDataDelegate for StreamDelegate {
        #[unsafe(method(URLSession:dataTask:didReceiveResponse:completionHandler:))]
        fn did_receive_response(
            &self,
            _session: &NSURLSession,
            _task: &NSURLSessionDataTask,
            response: &NSURLResponse,
            completion: &block2::DynBlock<dyn Fn(NSURLSessionResponseDisposition)>,
        ) {
            let (status, headers) = map_response(response);
            self.send(StreamMsg::Head(status, headers));
            completion.call((NSURLSessionResponseDisposition::Allow,));
        }

        #[unsafe(method(URLSession:dataTask:didReceiveData:))]
        fn did_receive_data(
            &self,
            _session: &NSURLSession,
            _task: &NSURLSessionDataTask,
            data: &NSData,
        ) {
            self.send(StreamMsg::Chunk(data.to_vec()));
        }
    }
);

impl StreamDelegate {
    fn new(tx: mpsc::Sender<StreamMsg>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(StreamIvars {
            tx: std::sync::Mutex::new(tx),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn send(&self, msg: StreamMsg) {
        if let Ok(tx) = self.ivars().tx.lock() {
            let _ = tx.send(msg);
        }
    }
}

pub fn fetch_streamed(
    req: &Request,
    sink: &mut dyn super::StreamSink,
) -> Result<Download, HttpError> {
    let native = build_request(req)?;
    let (tx, rx) = mpsc::channel();
    let delegate = StreamDelegate::new(tx);
    let config = NSURLSessionConfiguration::ephemeralSessionConfiguration();
    // SAFETY: standard delegate-session construction; the delegate outlives the session (retained
    // by it until finishTasksAndInvalidate).
    let session = unsafe {
        NSURLSession::sessionWithConfiguration_delegate_delegateQueue(
            &config,
            Some(ProtocolObject::from_ref(&*delegate)),
            None,
        )
    };
    let task = session.dataTaskWithRequest(&native);
    task.resume();

    // The request timeout bounds IDLE gaps between delegate messages (URLSession's own
    // timeoutInterval covers the wire; the grace covers pathological delegate loss).
    let grace = req.timeout + std::time::Duration::from_secs(5);
    let abort = |e: HttpError| -> Result<Download, HttpError> {
        task.cancel();
        session.finishTasksAndInvalidate();
        Err(e)
    };
    let (mut status, mut headers) = (0u16, Vec::new());
    let mut bytes_written = 0u64;
    loop {
        match rx.recv_timeout(grace) {
            Ok(StreamMsg::Head(s, h)) => {
                if !sink.head(s, &h) {
                    return abort(HttpError::Io("aborted".into()));
                }
                status = s;
                headers = h;
            }
            Ok(StreamMsg::Chunk(c)) => {
                if let Err(e) = sink.chunk(&c) {
                    return abort(e);
                }
                bytes_written += c.len() as u64;
            }
            Ok(StreamMsg::Done(Some(e))) => {
                session.finishTasksAndInvalidate();
                return Err(e);
            }
            Ok(StreamMsg::Done(None)) => break,
            Err(_) => return abort(HttpError::Timeout),
        }
    }
    session.finishTasksAndInvalidate();
    Ok(Download {
        status,
        headers,
        bytes_written,
    })
}
