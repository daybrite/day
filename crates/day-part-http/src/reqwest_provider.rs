// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Optional async reqwest HTTP + tungstenite WebSocket provider, on a shared Tokio runtime.
use crate::{Capabilities, Head, HttpError, Message, session::lock, transport::*};
use futures_util::{SinkExt, StreamExt};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Semaphore, mpsc};

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("HTTP provider runtime")
    })
}
fn tls_config() -> Arc<rustls::ClientConfig> {
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    Arc::new(
        rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("ring TLS protocols")
        .with_root_certificates(roots)
        .with_no_client_auth(),
    )
}
fn error(e: reqwest::Error) -> HttpError {
    if e.is_timeout() {
        HttpError::Timeout
    } else {
        HttpError::Io(e.to_string())
    }
}
pub(crate) struct Reqwest {
    client: Result<reqwest::Client, String>,
}
impl Reqwest {
    pub fn new(config: &TransportConfig) -> Self {
        // Redirect/auth policy remains in Day's portable Client. TLS uses rustls/webpki roots;
        // native proxy/PAC, platform caches and client identities are not claimed here.
        let builder = reqwest::Client::builder()
            .tls_backend_preconfigured((*tls_config()).clone())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(config.timeout_idle)
            .read_timeout(config.timeout_idle);
        Self {
            client: builder.build().map_err(|e| e.to_string()),
        }
    }
}
struct HttpTransfer {
    events: Events,
    demand: Semaphore,
    over: AtomicBool,
    task: Mutex<Option<tokio::task::AbortHandle>>,
}
impl HttpTransfer {
    fn emit(&self, e: Event) {
        if matches!(e, Event::End | Event::Failed(_)) {
            if !self.over.swap(true, Ordering::AcqRel) {
                (self.events)(e);
            }
        } else if !self.over.load(Ordering::Acquire) {
            (self.events)(e);
        }
    }
}
impl Transfer for HttpTransfer {
    fn demand(&self, n: u32) {
        self.demand.add_permits(n as usize)
    }
    fn answer(&self, _: QuestionId, _: Answer) {}
    fn cancel(&self) {
        self.emit(Event::Failed(HttpError::Cancelled));
        if let Some(t) = lock(&self.task).take() {
            t.abort();
        }
    }
}
fn body_stream(
    reader: Box<dyn std::io::Read + Send>,
    transfer: Arc<HttpTransfer>,
    len: Option<u64>,
) -> reqwest::Body {
    let stream = futures_util::stream::unfold((Some(reader), 0u64), move |(reader, sent)| {
        let t = transfer.clone();
        async move {
            let mut reader = reader?;
            let result = tokio::task::spawn_blocking(move || {
                let mut b = vec![0; 16 * 1024];
                let result = reader.read(&mut b).map(|n| {
                    b.truncate(n);
                    b
                });
                (reader, result)
            })
            .await;
            match result {
                Ok((reader, Ok(b))) if !b.is_empty() => {
                    let total = sent + b.len() as u64;
                    t.emit(Event::Sent {
                        sent: total,
                        total: len,
                    });
                    Some((Ok::<_, std::io::Error>(b), (Some(reader), total)))
                }
                Ok((_, Ok(_))) => None,
                Ok((_, Err(e))) => Some((Err(e), (None, sent))),
                Err(e) => Some((Err(std::io::Error::other(e)), (None, sent))),
            }
        }
    });
    reqwest::Body::wrap_stream(stream)
}
impl Transport for Reqwest {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            streaming: true,
            upload_streaming: true,
            upload_progress: true,
            manual_redirects: true,
            websockets: true,
            websocket_ping: true,
            websocket_headers: true,
            ..Default::default()
        }
    }
    fn start(&self, req: Prepared, events: Events) -> Arc<dyn Transfer> {
        let t = Arc::new(HttpTransfer {
            events,
            demand: Semaphore::new(0),
            over: AtomicBool::new(false),
            task: Mutex::new(None),
        });
        let transfer = t.clone();
        let client = self.client.clone();
        let task = runtime().spawn(async move {
            let result = async {
                let client = client.map_err(HttpError::Io)?;
                let method = reqwest::Method::from_bytes(req.method.as_bytes())
                    .map_err(|e| HttpError::Io(e.to_string()))?;
                let mut builder = client.request(method, &req.url);
                for (k, v) in req.headers {
                    builder = builder.header(k, v);
                }
                let (reader, len): (Option<Box<dyn std::io::Read + Send>>, Option<u64>) = match req
                    .body
                {
                    PreparedBody::Empty => (None, Some(0)),
                    PreparedBody::Bytes(b) => {
                        let len = b.len() as u64;
                        (
                            Some(Box::new(std::io::Cursor::new((*b).clone()))),
                            Some(len),
                        )
                    }
                    PreparedBody::File { path, len } => (
                        Some(Box::new(
                            std::fs::File::open(path).map_err(|e| HttpError::Io(e.to_string()))?,
                        )),
                        Some(len),
                    ),
                    PreparedBody::Stream { reader, len } => (
                        Some(reader.take().ok_or_else(|| {
                            HttpError::Io("request body already consumed".into())
                        })?),
                        len,
                    ),
                };
                if let Some(reader) = reader {
                    if let Some(n) = len {
                        builder = builder.header(reqwest::header::CONTENT_LENGTH, n);
                    }
                    builder = builder.body(body_stream(reader, transfer.clone(), len));
                }
                let response = tokio::time::timeout(req.timeout_idle, builder.send())
                    .await
                    .map_err(|_| HttpError::Timeout)?
                    .map_err(error)?;
                let head = Head {
                    status: response.status().as_u16(),
                    headers: response
                        .headers()
                        .iter()
                        .map(|(k, v)| {
                            (
                                k.to_string(),
                                String::from_utf8_lossy(v.as_bytes()).into_owned(),
                            )
                        })
                        .collect(),
                    url: response.url().to_string(),
                    expected_length: response.content_length(),
                };
                transfer.emit(Event::Head(head));
                let mut stream = response.bytes_stream();
                loop {
                    let permit = transfer
                        .demand
                        .acquire()
                        .await
                        .map_err(|_| HttpError::Cancelled)?;
                    permit.forget();
                    match tokio::time::timeout(req.timeout_idle, stream.next())
                        .await
                        .map_err(|_| HttpError::Timeout)?
                    {
                        Some(Ok(b)) => transfer.emit(Event::Chunk(b.to_vec())),
                        Some(Err(e)) => return Err(error(e)),
                        None => break,
                    }
                }
                Ok(())
            }
            .await;
            transfer.emit(match result {
                Ok(()) => Event::End,
                Err(e) => Event::Failed(e),
            });
        });
        *lock(&t.task) = Some(task.abort_handle());
        if t.over.load(Ordering::Acquire) {
            task.abort();
        }
        t
    }
    fn websocket(&self, req: Prepared, events: WsEvents) -> Arc<dyn Socket> {
        let (tx, mut rx) = mpsc::channel(64);
        let socket = Arc::new(WsSocket {
            tx,
            events,
            demand: Semaphore::new(0),
            over: AtomicBool::new(false),
            task: Mutex::new(None),
        });
        let s = socket.clone();
        let task=runtime().spawn(async move{
            use tokio_tungstenite::tungstenite::{Message as T, client::IntoClientRequest};
            let result=async {
                let mut request=req.url.into_client_request().map_err(|e|HttpError::Io(e.to_string()))?;
                for(k,v)in req.headers{request.headers_mut().append(k.parse::<tokio_tungstenite::tungstenite::http::HeaderName>().map_err(|e|HttpError::Io(e.to_string()))?,v.parse().map_err(|e:tokio_tungstenite::tungstenite::http::header::InvalidHeaderValue|HttpError::Io(e.to_string()))?);}
                if !req.protocols.is_empty(){request.headers_mut().insert("Sec-WebSocket-Protocol",req.protocols.join(", ").parse().map_err(|e:tokio_tungstenite::tungstenite::http::header::InvalidHeaderValue|HttpError::Io(e.to_string()))?);}
                let (mut ws,response)=tokio::time::timeout(req.timeout_idle,tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(tokio_tungstenite::Connector::Rustls(tls_config())))).await.map_err(|_|HttpError::Timeout)?.map_err(|e|HttpError::Io(e.to_string()))?;
                s.emit(WsEvent::Open{protocol:response.headers().get("Sec-WebSocket-Protocol").and_then(|v|v.to_str().ok()).map(str::to_owned)});
                let mut demand=0usize;
                loop{tokio::select!{
                    cmd=rx.recv()=>match cmd{
                        Some(Command::Send(message,done))=>{let m=match message{Message::Text(t)=>T::Text(t.into()),Message::Binary(b)=>T::Binary(b.into()),Message::Close{code,reason}=>T::Close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame{code:code.into(),reason:reason.into()}))};let r=ws.send(m).await.map_err(|e|HttpError::Io(e.to_string()));done(r.clone());r?;},
                        Some(Command::Ping(done))=>{done(ws.send(T::Ping(Vec::new().into())).await.map_err(|e|HttpError::Io(e.to_string())));},
                        Some(Command::Close(code,reason))=>{let _=ws.close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame{code:code.into(),reason:reason.clone().into()})).await;s.emit(WsEvent::Closed{code,reason});return Ok(());},
                        None=>return Ok(()),
                    },
                    p=s.demand.acquire(),if demand==0=>{p.map_err(|_|HttpError::Cancelled)?.forget();demand+=1;},
                    m=ws.next(),if demand>0=>match m{
                        Some(Ok(T::Text(t)))=>{demand-=1;s.emit(WsEvent::Message(Message::Text(t.to_string())));},
                        Some(Ok(T::Binary(b)))=>{demand-=1;s.emit(WsEvent::Message(Message::Binary(b.to_vec())));},
                        Some(Ok(T::Close(frame)))=>{let(code,reason)=frame.map(|f|(f.code.into(),f.reason.to_string())).unwrap_or((1000,String::new()));s.emit(WsEvent::Closed{code,reason});return Ok(());},
                        Some(Err(e))=>return Err(HttpError::Io(e.to_string())),None=>return Ok(()),_=>{},
                    }
                }}
            }.await;
            match result{Err(e)=>s.emit(WsEvent::Failed(e)),Ok(())=>s.emit(WsEvent::Closed{code:1000,reason:String::new()})}
        });
        *lock(&socket.task) = Some(task.abort_handle());
        socket
    }
}
enum Command {
    Send(Message, Completion),
    Ping(Completion),
    Close(u16, String),
}
struct WsSocket {
    tx: mpsc::Sender<Command>,
    events: WsEvents,
    demand: Semaphore,
    over: AtomicBool,
    task: Mutex<Option<tokio::task::AbortHandle>>,
}
impl WsSocket {
    fn emit(&self, e: WsEvent) {
        if matches!(e, WsEvent::Closed { .. } | WsEvent::Failed(_)) {
            if !self.over.swap(true, Ordering::AcqRel) {
                (self.events)(e);
            }
        } else if !self.over.load(Ordering::Acquire) {
            (self.events)(e);
        }
    }
    fn command(&self, c: Command) {
        if let Err(e) = self.tx.try_send(c) {
            match e.into_inner() {
                Command::Send(_, done) | Command::Ping(done) => done(Err(HttpError::Io(
                    "WebSocket closed or send queue full".into(),
                ))),
                Command::Close(code, reason) => {
                    self.emit(WsEvent::Closed { code, reason });
                    if let Some(t) = lock(&self.task).take() {
                        t.abort();
                    }
                }
            }
        }
    }
}
impl Socket for WsSocket {
    fn send(&self, m: Message, done: Completion) {
        self.command(Command::Send(m, done))
    }
    fn ping(&self, done: Completion) {
        self.command(Command::Ping(done))
    }
    fn close(&self, code: u16, reason: &str) {
        self.emit(WsEvent::Closed {
            code,
            reason: reason.into(),
        });
        self.command(Command::Close(code, reason.into()));
        // A stalled handshake/send must not hold a cancelled socket indefinitely. Give an
        // established connection a short opportunity to send its close frame, then drop it.
        if let Some(task) = lock(&self.task).take() {
            day_async::schedule(std::time::Duration::from_millis(250), move || task.abort());
        }
    }
    fn demand(&self, n: u32) {
        self.demand.add_permits(n as usize)
    }
}
