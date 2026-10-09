// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Payload accounting at the transport boundary. No headers, TLS or TCP overhead is counted.
use crate::{Capabilities, Message, session::lock, transport::*};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

pub(crate) fn now() -> Duration {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static START: OnceLock<std::time::Instant> = OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed()
    }
    #[cfg(target_arch = "wasm32")]
    {
        Duration::from_secs_f64(crate::bridge::simulation_now() / 1000.0)
    }
}
/// Application payload totals. Rates divide bytes by elapsed scope time, including idle time.
#[derive(Clone, Debug, Default)]
pub struct Statistics {
    pub started: u64,
    pub active: u64,
    pub completed: u64,
    pub failed: u64,
    pub uploaded_bytes: u64,
    pub downloaded_bytes: u64,
    pub simulated_uploaded_bytes: u64,
    pub simulated_downloaded_bytes: u64,
    pub elapsed: Duration,
}
impl Statistics {
    pub fn upload_bytes_per_second(&self) -> f64 {
        self.uploaded_bytes as f64 / self.elapsed.as_secs_f64().max(0.001)
    }
    pub fn download_bytes_per_second(&self) -> f64 {
        self.downloaded_bytes as f64 / self.elapsed.as_secs_f64().max(0.001)
    }
    pub fn total_bytes(&self) -> u64 {
        self.uploaded_bytes + self.downloaded_bytes
    }
    /// All sessions, including explicit custom transports. Only day-part-http traffic.
    pub fn global() -> Self {
        global().snapshot()
    }
}
#[derive(Clone, Debug)]
pub struct TransferStatistics {
    pub id: u64,
    pub method: String,
    pub url: String,
    pub simulated: bool,
    pub statistics: Statistics,
    pub error: Option<String>,
}
struct State {
    start: Duration,
    totals: Statistics,
    history: VecDeque<Arc<Mutex<Record>>>,
}
struct Record {
    start: Duration,
    value: TransferStatistics,
    ended: bool,
}
pub(crate) struct Store(Mutex<State>);
impl Default for Store {
    fn default() -> Self {
        Self(Mutex::new(State {
            start: now(),
            totals: Statistics::default(),
            history: VecDeque::new(),
        }))
    }
}
fn global() -> &'static Arc<Store> {
    static STORE: OnceLock<Arc<Store>> = OnceLock::new();
    STORE.get_or_init(|| Arc::new(Store::default()))
}
impl Store {
    pub fn snapshot(&self) -> Statistics {
        let s = lock(&self.0);
        let mut t = s.totals.clone();
        t.elapsed = now().saturating_sub(s.start);
        t
    }
    pub fn transfers(&self) -> Vec<TransferStatistics> {
        let records = lock(&self.0).history.iter().cloned().collect::<Vec<_>>();
        records
            .into_iter()
            .map(|r| {
                let r = lock(&r);
                let mut v = r.value.clone();
                if !r.ended {
                    v.statistics.elapsed = now().saturating_sub(r.start);
                }
                v
            })
            .collect()
    }
}
struct Meter {
    record: Arc<Mutex<Record>>,
    stores: [Arc<Store>; 2],
}
impl Meter {
    fn new(store: Arc<Store>, req: &Prepared, simulated: bool) -> Arc<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let record = Arc::new(Mutex::new(Record {
            start: now(),
            ended: false,
            value: TransferStatistics {
                id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                method: req.method.clone(),
                url: req.url.clone(),
                simulated,
                statistics: Statistics {
                    started: 1,
                    active: 1,
                    ..Default::default()
                },
                error: None,
            },
        }));
        let stores = [store, global().clone()];
        for s in &stores {
            let mut st = lock(&s.0);
            st.totals.started += 1;
            st.totals.active += 1;
            st.history.push_back(record.clone());
            if st.history.len() > 256 {
                st.history.pop_front();
            }
        }
        Arc::new(Self { record, stores })
    }
    fn bytes(&self, upload: u64, download: u64) {
        self.add_bytes(&mut lock(&self.record), upload, download);
    }
    fn add_bytes(&self, r: &mut Record, upload: u64, download: u64) {
        if r.ended {
            return;
        }
        let sim = r.value.simulated;
        fn add(s: &mut Statistics, u: u64, d: u64, sim: bool) {
            s.uploaded_bytes += u;
            s.downloaded_bytes += d;
            if sim {
                s.simulated_uploaded_bytes += u;
                s.simulated_downloaded_bytes += d;
            }
        }
        add(&mut r.value.statistics, upload, download, sim);
        for store in &self.stores {
            add(&mut lock(&store.0).totals, upload, download, sim);
        }
    }
    fn uploaded(&self, total: u64) {
        let mut r = lock(&self.record);
        let delta = total.saturating_sub(r.value.statistics.uploaded_bytes);
        self.add_bytes(&mut r, delta, 0);
    }
    fn end(&self, error: Option<String>) {
        let mut r = lock(&self.record);
        if r.ended {
            return;
        }
        r.ended = true;
        r.value.statistics.elapsed = now().saturating_sub(r.start);
        r.value.error = error.clone();
        fn finish(s: &mut Statistics, failed: bool) {
            s.active = s.active.saturating_sub(1);
            if failed {
                s.failed += 1;
            } else {
                s.completed += 1;
            }
        }
        finish(&mut r.value.statistics, error.is_some());
        for store in &self.stores {
            finish(&mut lock(&store.0).totals, error.is_some());
        }
    }
}
struct Observed {
    inner: Arc<dyn Transport>,
    store: Arc<Store>,
    simulated: bool,
}
pub(crate) fn observe(
    inner: Arc<dyn Transport>,
    store: Arc<Store>,
    simulated: bool,
) -> Arc<dyn Transport> {
    Arc::new(Observed {
        inner,
        store,
        simulated,
    })
}
impl Transport for Observed {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn start(&self, req: Prepared, events: Events) -> Arc<dyn Transfer> {
        let meter = Meter::new(self.store.clone(), &req, self.simulated);
        let m = meter.clone();
        // Browser fetch does not report upload progress. A received response establishes that
        // its known request payload was submitted; partial failed uploads remain unknown.
        let fallback_upload = if self.inner.capabilities().upload_progress {
            0
        } else {
            match &req.body {
                PreparedBody::Empty => 0,
                PreparedBody::Bytes(bytes) => bytes.len() as u64,
                PreparedBody::File { len, .. } => *len,
                PreparedBody::Stream { len, .. } => len.unwrap_or(0),
            }
        };
        let inner = self.inner.start(
            req,
            Arc::new(move |e| {
                match &e {
                    Event::Head(_) => m.uploaded(fallback_upload),
                    Event::Chunk(b) => m.bytes(0, b.len() as u64),
                    Event::Sent { sent, .. } => m.uploaded(*sent),
                    Event::End => m.end(None),
                    Event::Failed(e) => m.end(Some(e.to_string())),
                    _ => {}
                }
                events(e);
            }),
        );
        Arc::new(ObservedTransfer { inner, meter })
    }
    fn websocket(&self, req: Prepared, events: WsEvents) -> Arc<dyn Socket> {
        let meter = Meter::new(self.store.clone(), &req, self.simulated);
        let m = meter.clone();
        let inner = self.inner.websocket(
            req,
            Arc::new(move |e| {
                match &e {
                    WsEvent::Message(msg) => m.bytes(0, message_len(msg)),
                    WsEvent::Closed { .. } => m.end(None),
                    WsEvent::Failed(e) => m.end(Some(e.to_string())),
                    _ => {}
                }
                events(e);
            }),
        );
        Arc::new(ObservedSocket { inner, meter })
    }
    fn cookies(&self) -> Vec<crate::Cookie> {
        self.inner.cookies()
    }
    fn clear_cookies(&self) {
        self.inner.clear_cookies()
    }
    fn clear_cache(&self) {
        self.inner.clear_cache()
    }
}
struct ObservedTransfer {
    inner: Arc<dyn Transfer>,
    meter: Arc<Meter>,
}
impl Transfer for ObservedTransfer {
    fn demand(&self, n: u32) {
        self.inner.demand(n)
    }
    fn answer(&self, id: QuestionId, a: Answer) {
        self.inner.answer(id, a)
    }
    fn cancel(&self) {
        self.meter.end(Some("cancelled".into()));
        self.inner.cancel()
    }
}
struct ObservedSocket {
    inner: Arc<dyn Socket>,
    meter: Arc<Meter>,
}
pub(crate) fn message_len(m: &Message) -> u64 {
    match m {
        Message::Text(t) => t.len() as u64,
        Message::Binary(b) => b.len() as u64,
        Message::Close { .. } => 0,
    }
}
impl Socket for ObservedSocket {
    fn send(&self, m: Message, done: Completion) {
        let n = message_len(&m);
        let meter = self.meter.clone();
        self.inner.send(
            m,
            Box::new(move |r| {
                if r.is_ok() {
                    meter.bytes(n, 0);
                }
                done(r)
            }),
        )
    }
    fn ping(&self, done: Completion) {
        self.inner.ping(done)
    }
    fn close(&self, code: u16, reason: &str) {
        self.meter.end(None);
        self.inner.close(code, reason)
    }
    fn demand(&self, n: u32) {
        self.inner.demand(n)
    }
}
