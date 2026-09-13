// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-part-downloads — a download manager over day-part-http (docs/downloads.md).
//!
//! [`Downloads`] queues transfers and runs a few at a time. Each download writes to a partial
//! file beside a journal, so a pause, a transport failure or a relaunch resumes where it stopped:
//! the next attempt asks for the rest with `Range`, guarded by `If-Range` with the validator the
//! server first sent, and starts over when the file changed on the server. Transient failures (a
//! dropped connection, 408, 429, 5xx) retry with capped exponential backoff that honors
//! `Retry-After`. A finished file is checked against its expected length and SHA-256, then moved
//! to its destination, which never holds a partial file.
//!
//! ```no_run
//! use day_part_downloads::{Download, Downloads};
//! use day_part_http::Request;
//!
//! let downloads = Downloads::open("/path/to/app/data/downloads")?;
//! let id = downloads.enqueue(
//!     Download::new(Request::get("https://example.com/big.bin"), "/path/to/big.bin")
//!         .expect_sha256("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"),
//! )?;
//! let _watch = downloads.watch(move |progress| {
//!     if progress.id == id {
//!         println!("{} {} bytes", progress.state.label(), progress.received);
//!     }
//! });
//! # Ok::<(), day_part_downloads::DownloadError>(())
//! ```
//!
//! **The system tier.** [`Download::in_background`] asks the OS to run a transfer, so it continues
//! while the app is suspended or gone: a background `URLSession` on macOS and iOS, `DownloadManager`
//! on Android, the request agent on HarmonyOS and the Background Intelligent Transfer Service on
//! Windows. The manager keeps the same journal and checks for such a download, and the OS keeps the
//! bytes until the body is whole. Elsewhere the download runs in the app.
//!
//! **Threads.** Nothing waits on a thread of its own: bytes arrive on the client's transport and
//! backoff waits on day-async's timer. Watch callbacks run on whichever thread reported the
//! change; capture a reactive `Setter` to deliver progress into UI state (docs/async.md).

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use day_part_http::{HttpError, Request};

/// Identifies a download for the life of its journal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DownloadId(u64);

impl DownloadId {
    /// The number the journal records.
    pub fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for DownloadId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A download to enqueue: the request, where the finished file goes, and what it must be.
#[derive(Clone, Debug)]
// The web's manager opens nothing, so it reads no download.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub struct Download {
    request: Request,
    dest: PathBuf,
    expected_len: Option<u64>,
    sha256: Option<String>,
    retries: u32,
    retry_delay: Duration,
    background: bool,
}

impl Download {
    /// Download `request`'s response body to `dest`.
    pub fn new(request: Request, dest: impl Into<PathBuf>) -> Download {
        Download {
            request,
            dest: dest.into(),
            expected_len: None,
            sha256: None,
            retries: 5,
            retry_delay: Duration::from_secs(1),
            background: false,
        }
    }

    /// The length the finished file must have. It also lets the manager check free space before
    /// the transfer starts.
    pub fn expect_len(mut self, len: u64) -> Self {
        self.expected_len = Some(len);
        self
    }

    /// The SHA-256 the finished file must have, as hex.
    pub fn expect_sha256(mut self, hex: &str) -> Self {
        self.sha256 = Some(hex.trim().to_ascii_lowercase());
        self
    }

    /// How many times a transient failure is retried before the download fails. Default 5.
    pub fn retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    /// The first backoff; each retry doubles it, up to a minute. Default one second. A server's
    /// `Retry-After` wins.
    pub fn retry_delay(mut self, delay: Duration) -> Self {
        self.retry_delay = delay;
        self
    }

    /// Ask the OS to run the transfer, so it can outlive the app, where
    /// [`Downloads::system_tier`] offers one. Elsewhere the download runs in the app and its
    /// progress reports [`Tier::InApp`].
    pub fn in_background(mut self, background: bool) -> Self {
        self.background = background;
        self
    }
}

/// Where a download stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum State {
    /// Waiting for a free slot.
    Queued,
    /// Transferring.
    Running,
    /// Stopped by the app; its partial file waits for [`Downloads::resume`].
    Paused,
    /// Waiting out a backoff after a transient failure.
    Retrying,
    /// Checking the finished file.
    Verifying,
    /// Verified and moved to its destination.
    Done,
    /// Given up; [`Progress::error`] says why. [`Downloads::resume`] tries again.
    Failed,
    /// Stopped for good; its partial file is gone.
    Cancelled,
}

impl State {
    /// No further progress comes without a call from the app.
    pub fn is_settled(self) -> bool {
        matches!(
            self,
            State::Paused | State::Done | State::Failed | State::Cancelled
        )
    }

    /// A short lowercase name: `queued`, `running`, `paused`, `retrying`, `verifying`, `done`,
    /// `failed`, `cancelled`.
    pub fn label(self) -> &'static str {
        match self {
            State::Queued => "queued",
            State::Running => "running",
            State::Paused => "paused",
            State::Retrying => "retrying",
            State::Verifying => "verifying",
            State::Done => "done",
            State::Failed => "failed",
            State::Cancelled => "cancelled",
        }
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn parse(label: &str) -> Option<State> {
        [
            State::Queued,
            State::Running,
            State::Paused,
            State::Retrying,
            State::Verifying,
            State::Done,
            State::Failed,
            State::Cancelled,
        ]
        .into_iter()
        .find(|s| s.label() == label)
    }
}

/// Who runs a transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tier {
    /// The app, through its HTTP client: every callback, header and check applies.
    InApp,
    /// The OS, so the transfer outlives the app.
    System,
}

/// A download's state and numbers, as a watch sees them.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub id: DownloadId,
    pub url: String,
    pub dest: PathBuf,
    pub state: State,
    /// Bytes in the partial (or finished) file.
    pub received: u64,
    /// The whole length, once the server or the download said.
    pub total: Option<u64>,
    /// Measured over the last few seconds.
    pub bytes_per_second: f64,
    /// At the current rate.
    pub remaining: Option<Duration>,
    /// Failed attempts so far.
    pub attempt: u32,
    /// Where the current or last attempt started, in bytes: more than zero means it resumed.
    pub resumed_from: u64,
    /// Why the download is retrying or failed.
    pub error: Option<String>,
    /// The finished file's SHA-256, as hex.
    pub sha256: Option<String>,
    pub tier: Tier,
}

impl Progress {
    /// `received / total`, from 0.0 to 1.0, once the total is known.
    pub fn fraction(&self) -> Option<f64> {
        self.total
            .filter(|t| *t > 0)
            .map(|t| (self.received as f64 / t as f64).min(1.0))
    }
}

/// Why an operation on the manager failed, or why a download did.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadError {
    /// A file or directory the manager needs could not be used.
    Io(String),
    /// No download has this id.
    Unknown(DownloadId),
    /// Not available on this platform: the web gives the manager no files to own, and Android's
    /// `DownloadManager` cannot pause a transfer.
    Unsupported,
    /// The request failed.
    Http(HttpError),
    /// The finished file's SHA-256 was not the one expected.
    Integrity { expected: String, actual: String },
    /// The finished file's length was not the one expected.
    Length { expected: u64, actual: u64 },
    /// The volume lacks room for the rest of the file.
    NoSpace { needed: u64, available: u64 },
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::Io(m) => write!(f, "{m}"),
            DownloadError::Unknown(id) => write!(f, "no download {id}"),
            DownloadError::Unsupported => write!(f, "not supported on this platform"),
            DownloadError::Http(e) => write!(f, "{e}"),
            DownloadError::Integrity { expected, actual } => {
                write!(f, "SHA-256 {actual} is not the expected {expected}")
            }
            DownloadError::Length { expected, actual } => {
                write!(f, "{actual} bytes is not the expected {expected}")
            }
            DownloadError::NoSpace { needed, available } => {
                write!(f, "{needed} bytes needed, {available} available")
            }
        }
    }
}

impl std::error::Error for DownloadError {}

/// Move `from` to `to`, copying when the two are on different volumes.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn move_file(from: &Path, to: &Path) -> Result<(), DownloadError> {
    let io = |e: std::io::Error| DownloadError::Io(e.to_string());
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    if std::fs::rename(from, to).is_err() {
        std::fs::copy(from, to).map_err(io)?;
        std::fs::remove_file(from).map_err(io)?;
    }
    Ok(())
}

/// What the OS reports about a transfer it runs for the system tier.
#[cfg(not(target_arch = "wasm32"))]
// Each backend reports the events its platform offers.
#[allow(dead_code)]
pub(crate) enum SystemEvent {
    /// The OS's name for the transfer, which the journal keeps so a later run finds it again.
    Started(String),
    /// Bytes so far, and the whole length once known.
    Progress { received: u64, total: Option<u64> },
    /// The transfer continued from this many bytes.
    Resumed(u64),
    /// The whole body is at the job's `part` path.
    Finished,
    /// The transfer stopped. A `transient` failure is retried like the in-app tier's, and a new
    /// transfer starts, from resume data where the platform kept some.
    Failed {
        error: DownloadError,
        transient: bool,
    },
}

/// Where a system-tier backend delivers its events, from any thread.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) type SystemEvents = std::sync::Arc<dyn Fn(SystemEvent) + Send + Sync>;

/// One transfer for the OS to run.
#[cfg(not(target_arch = "wasm32"))]
// Each backend reads what its platform needs.
#[allow(dead_code)]
pub(crate) struct SystemJob<'a> {
    pub(crate) id: u64,
    pub(crate) request: &'a Request,
    /// Where the body must be when the backend reports [`SystemEvent::Finished`].
    pub(crate) part: &'a Path,
    /// The manager's directory, where a backend may keep what it needs to continue a transfer.
    pub(crate) dir: &'a Path,
    /// A name for the OS's own progress UI.
    pub(crate) title: &'a str,
    /// The name an earlier [`SystemEvent::Started`] gave: follow or continue that transfer.
    pub(crate) reference: Option<&'a str>,
}

// The system tier's backends. Each offers `available()`, `start(job, events)`, `pause(dir, id,
// reference)` and `cancel(dir, id, reference)`.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[path = "apple.rs"]
mod system;

#[cfg(any(target_os = "android", all(target_os = "linux", target_env = "ohos")))]
mod bridge;
#[cfg(any(target_os = "android", all(target_os = "linux", target_env = "ohos")))]
use bridge as system;

#[cfg(windows)]
#[path = "windows.rs"]
mod system;

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "android",
    all(target_os = "linux", target_env = "ohos"),
    windows,
    target_arch = "wasm32"
)))]
mod system {
    //! No system download service: every download runs in the app.
    use std::path::Path;

    use crate::{DownloadError, SystemEvents, SystemJob};

    pub(crate) fn available() -> bool {
        false
    }

    pub(crate) fn start(_job: SystemJob<'_>, _events: SystemEvents) -> Result<(), DownloadError> {
        Err(DownloadError::Unsupported)
    }

    pub(crate) fn pause(
        _dir: &Path,
        _id: u64,
        _reference: Option<&str>,
    ) -> Result<(), DownloadError> {
        Err(DownloadError::Unsupported)
    }

    pub(crate) fn cancel(_dir: &Path, _id: u64, _reference: Option<&str>) {}
}

#[cfg(not(target_arch = "wasm32"))]
mod manager {
    // The in-app manager: queue, journal, attempts, verification.
    //
    // One mutex guards every entry. File writes happen under it, which is what keeps an abandoned
    // attempt from writing after a newer one opened the partial file: each attempt carries a
    // generation, and a callback from an older generation finds it bumped and stops.
    use std::collections::{BTreeMap, HashMap, VecDeque};
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, MutexGuard, Weak};
    use std::time::{Duration, Instant, SystemTime};

    use day_async::TimerId;
    use day_part_http::{Body, Client, HttpError, InFlight, Method, Request, Streaming};
    use sha2::{Digest, Sha256};

    use crate::{
        Download, DownloadError, DownloadId, Progress, State, SystemEvent, SystemEvents, SystemJob,
        Tier,
    };

    const JOURNAL: &str = "downloads.journal";
    const NOTICE_INTERVAL: Duration = Duration::from_millis(100);
    const RATE_WINDOW: Duration = Duration::from_secs(3);
    const MAX_BACKOFF: Duration = Duration::from_secs(60);

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn io(e: std::io::Error) -> DownloadError {
        DownloadError::Io(e.to_string())
    }

    type Watcher = Arc<dyn Fn(&Progress) + Send + Sync>;

    /// A download manager: a queue, a journal and the attempts it runs. Cloning shares it.
    #[derive(Clone)]
    pub struct Downloads {
        inner: Arc<Manager>,
    }

    impl std::fmt::Debug for Downloads {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Downloads")
                .field("dir", &self.inner.dir)
                .finish_non_exhaustive()
        }
    }

    /// Stops a watch when dropped.
    pub struct Watch {
        manager: Weak<Manager>,
        id: u64,
    }

    impl std::fmt::Debug for Watch {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Watch").field("id", &self.id).finish()
        }
    }

    impl Drop for Watch {
        fn drop(&mut self) {
            if let Some(manager) = self.manager.upgrade() {
                lock(&manager.state)
                    .watchers
                    .retain(|(id, _)| *id != self.id);
            }
        }
    }

    struct Manager {
        client: Client,
        dir: PathBuf,
        me: Weak<Manager>,
        state: Mutex<Shared>,
    }

    struct Shared {
        entries: BTreeMap<u64, Entry>,
        next_id: u64,
        limit_total: usize,
        limit_per_host: usize,
        watchers: Vec<(u64, Watcher)>,
        next_watcher: u64,
    }

    struct Entry {
        download: Download,
        host: String,
        state: State,
        received: u64,
        total: Option<u64>,
        validator: Option<String>,
        attempt: u32,
        error: Option<String>,
        sha256: Option<String>,
        resumed_from: u64,
        generation: u64,
        in_flight: Option<InFlight>,
        /// The digest of the bytes in the partial file, when this process has it.
        hasher: Option<Sha256>,
        file: Option<File>,
        timer: Option<TimerId>,
        samples: VecDeque<(Instant, u64)>,
        noticed: Option<Instant>,
        tier: Tier,
        /// The OS's name for a system-tier transfer.
        reference: Option<String>,
    }

    impl Entry {
        fn new(download: Download) -> Entry {
            Entry {
                host: host_of(download.request.url()),
                download,
                state: State::Queued,
                received: 0,
                total: None,
                validator: None,
                attempt: 0,
                error: None,
                sha256: None,
                resumed_from: 0,
                generation: 0,
                in_flight: None,
                hasher: None,
                file: None,
                timer: None,
                samples: VecDeque::new(),
                noticed: None,
                tier: Tier::InApp,
                reference: None,
            }
        }

        /// Record `received` for the rate, and say whether a watch is due a notice.
        fn sample(&mut self, received: u64) -> bool {
            self.received = received;
            let now = Instant::now();
            self.samples.push_back((now, received));
            while self
                .samples
                .front()
                .is_some_and(|(t, _)| now.duration_since(*t) > RATE_WINDOW)
            {
                self.samples.pop_front();
            }
            let due = self
                .noticed
                .is_none_or(|t| now.duration_since(t) >= NOTICE_INTERVAL);
            if due {
                self.noticed = Some(now);
            }
            due
        }

        fn progress(&self, id: u64) -> Progress {
            let bytes_per_second = match (self.state, self.samples.front(), self.samples.back()) {
                (State::Running, Some((t0, b0)), Some((t1, b1))) => {
                    let span = t1.duration_since(*t0).as_secs_f64();
                    if span >= 0.25 {
                        (b1 - b0) as f64 / span
                    } else {
                        0.0
                    }
                }
                _ => 0.0,
            };
            let remaining = match self.total {
                Some(total) if bytes_per_second > 0.0 => Some(Duration::from_secs_f64(
                    total.saturating_sub(self.received) as f64 / bytes_per_second,
                )),
                _ => None,
            };
            Progress {
                id: DownloadId(id),
                url: self.download.request.url().to_string(),
                dest: self.download.dest.clone(),
                state: self.state,
                received: self.received,
                total: self.total.or(self.download.expected_len),
                bytes_per_second,
                remaining,
                attempt: self.attempt,
                resumed_from: self.resumed_from,
                error: self.error.clone(),
                sha256: self.sha256.clone(),
                tier: self.tier,
            }
        }

        /// Stop the current attempt: the returned grip cancels its request once the lock is gone.
        fn stop(&mut self) -> (Option<InFlight>, Option<TimerId>) {
            self.generation += 1;
            self.file = None;
            self.samples.clear();
            (self.in_flight.take(), self.timer.take())
        }
    }

    fn host_of(url: &str) -> String {
        let rest = url.split_once("://").map_or(url, |(_, r)| r);
        rest.split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
    }

    /// Cancel what `Entry::stop` handed back, outside the lock.
    fn release((in_flight, timer): (Option<InFlight>, Option<TimerId>)) {
        if let Some(in_flight) = in_flight {
            in_flight.cancel();
        }
        if let Some(timer) = timer {
            day_async::unschedule(timer);
        }
    }

    impl Downloads {
        /// Open the manager whose journal and partial files live in `dir`, with a default
        /// [`Client`]. Downloads that were running when the app last stopped are queued again;
        /// paused ones stay paused.
        pub fn open(dir: impl Into<PathBuf>) -> Result<Downloads, DownloadError> {
            Downloads::with_client(dir, Client::new())
        }

        /// Open the manager with this client, whose policy (cookies, challenges, trust) every
        /// download uses.
        pub fn with_client(
            dir: impl Into<PathBuf>,
            client: Client,
        ) -> Result<Downloads, DownloadError> {
            let dir = dir.into();
            std::fs::create_dir_all(&dir).map_err(io)?;
            let entries = load_journal(&dir);
            let next_id = entries.keys().max().map_or(1, |m| m + 1);
            let inner = Arc::new_cyclic(|me| Manager {
                client,
                dir,
                me: me.clone(),
                state: Mutex::new(Shared {
                    entries,
                    next_id,
                    limit_total: 3,
                    limit_per_host: 2,
                    watchers: Vec::new(),
                    next_watcher: 1,
                }),
            });
            inner.pump();
            Ok(Downloads { inner })
        }

        /// Whether this platform can hand a download to the OS ([`Download::in_background`]).
        pub fn system_tier() -> bool {
            crate::system::available()
        }

        /// Run at most `total` downloads at once, and at most `per_host` from one host. Default 3
        /// and 2.
        pub fn limits(&self, total: usize, per_host: usize) {
            {
                let mut st = lock(&self.inner.state);
                st.limit_total = total.max(1);
                st.limit_per_host = per_host.max(1);
            }
            self.inner.pump();
        }

        /// Add a download to the queue.
        pub fn enqueue(&self, download: Download) -> Result<DownloadId, DownloadError> {
            let (id, progress) = {
                let mut st = lock(&self.inner.state);
                let id = st.next_id;
                st.next_id += 1;
                let background = download.background;
                let mut entry = Entry::new(download);
                if background && crate::system::available() {
                    entry.tier = Tier::System;
                }
                let progress = entry.progress(id);
                st.entries.insert(id, entry);
                self.inner.save(&st);
                (id, progress)
            };
            self.inner.notify(&progress);
            self.inner.pump();
            Ok(DownloadId(id))
        }

        /// Stop a download, keeping its partial file for [`Downloads::resume`]. A system-tier
        /// transfer the OS cannot pause reports [`DownloadError::Unsupported`] and keeps running.
        pub fn pause(&self, id: DownloadId) -> Result<(), DownloadError> {
            let system = {
                let st = lock(&self.inner.state);
                let entry = st.entries.get(&id.0).ok_or(DownloadError::Unknown(id))?;
                (entry.tier == Tier::System && entry.state == State::Running)
                    .then(|| entry.reference.clone())
            };
            if let Some(reference) = system {
                crate::system::pause(&self.inner.dir, id.0, reference.as_deref())?;
            }
            self.inner.settle(id, State::Paused)
        }

        /// Queue a paused or failed download again. It continues from its partial file.
        pub fn resume(&self, id: DownloadId) -> Result<(), DownloadError> {
            let progress = {
                let mut st = lock(&self.inner.state);
                let entry = st
                    .entries
                    .get_mut(&id.0)
                    .ok_or(DownloadError::Unknown(id))?;
                if !matches!(entry.state, State::Paused | State::Failed) {
                    return Ok(());
                }
                entry.state = State::Queued;
                entry.attempt = 0;
                entry.error = None;
                let progress = entry.progress(id.0);
                self.inner.save(&st);
                progress
            };
            self.inner.notify(&progress);
            self.inner.pump();
            Ok(())
        }

        /// Stop a download for good and delete its partial file.
        pub fn cancel(&self, id: DownloadId) -> Result<(), DownloadError> {
            self.inner.settle(id, State::Cancelled)
        }

        /// Forget a download, cancelling it first if it is still active.
        pub fn remove(&self, id: DownloadId) -> Result<(), DownloadError> {
            let (stopped, system) = {
                let mut st = lock(&self.inner.state);
                let mut entry = st.entries.remove(&id.0).ok_or(DownloadError::Unknown(id))?;
                self.inner.save(&st);
                let system = (entry.tier == Tier::System && entry.state != State::Done)
                    .then(|| entry.reference.clone());
                (entry.stop(), system)
            };
            release(stopped);
            if let Some(reference) = system {
                crate::system::cancel(&self.inner.dir, id.0, reference.as_deref());
            }
            let _ = std::fs::remove_file(self.inner.part_path(id.0));
            self.inner.pump();
            Ok(())
        }

        /// One download's progress.
        pub fn progress(&self, id: DownloadId) -> Option<Progress> {
            lock(&self.inner.state)
                .entries
                .get(&id.0)
                .map(|e| e.progress(id.0))
        }

        /// Every download's progress, oldest first.
        pub fn list(&self) -> Vec<Progress> {
            lock(&self.inner.state)
                .entries
                .iter()
                .map(|(id, e)| e.progress(*id))
                .collect()
        }

        /// Call `on_change` whenever a download changes state, and about ten times a second while
        /// bytes arrive. It runs on the thread that reported the change.
        pub fn watch(&self, on_change: impl Fn(&Progress) + Send + Sync + 'static) -> Watch {
            let mut st = lock(&self.inner.state);
            let id = st.next_watcher;
            st.next_watcher += 1;
            st.watchers.push((id, Arc::new(on_change)));
            Watch {
                manager: Arc::downgrade(&self.inner),
                id,
            }
        }
    }

    enum Next {
        Read,
        Restart,
        Complete,
        Retry(Option<Duration>),
        Fail(DownloadError),
    }

    impl Manager {
        fn part_path(&self, id: u64) -> PathBuf {
            self.dir.join(format!("{id}.part"))
        }

        fn notify(&self, progress: &Progress) {
            let watchers: Vec<Watcher> = lock(&self.state)
                .watchers
                .iter()
                .map(|(_, w)| w.clone())
                .collect();
            for watcher in watchers {
                watcher(progress);
            }
        }

        /// Pause or cancel.
        fn settle(&self, id: DownloadId, to: State) -> Result<(), DownloadError> {
            let settled = {
                let mut st = lock(&self.state);
                let entry = st
                    .entries
                    .get_mut(&id.0)
                    .ok_or(DownloadError::Unknown(id))?;
                let active = matches!(
                    entry.state,
                    State::Queued | State::Running | State::Retrying
                );
                let allowed = match to {
                    State::Paused => active,
                    _ => active || matches!(entry.state, State::Paused | State::Failed),
                };
                if !allowed {
                    return Ok(());
                }
                let stopped = entry.stop();
                entry.state = to;
                let mut system = None;
                if to == State::Cancelled {
                    entry.received = 0;
                    entry.hasher = None;
                    entry.validator = None;
                    if entry.tier == Tier::System {
                        system = Some(entry.reference.take());
                    }
                }
                let progress = entry.progress(id.0);
                self.save(&st);
                (stopped, progress, system)
            };
            let (stopped, progress, system) = settled;
            release(stopped);
            if let Some(reference) = system {
                crate::system::cancel(&self.dir, id.0, reference.as_deref());
            }
            if to == State::Cancelled {
                let _ = std::fs::remove_file(self.part_path(id.0));
            }
            self.notify(&progress);
            self.pump();
            Ok(())
        }

        /// Start queued downloads while the limits allow.
        fn pump(&self) {
            let mut starts = Vec::new();
            {
                let mut st = lock(&self.state);
                let mut running = 0;
                let mut per_host: HashMap<String, usize> = HashMap::new();
                for entry in st.entries.values() {
                    // The OS schedules the system tier's transfers itself.
                    if entry.tier == Tier::InApp
                        && matches!(entry.state, State::Running | State::Verifying)
                    {
                        running += 1;
                        *per_host.entry(entry.host.clone()).or_default() += 1;
                    }
                }
                let (limit_total, limit_per_host) = (st.limit_total, st.limit_per_host);
                for (id, entry) in st.entries.iter_mut() {
                    if entry.state != State::Queued {
                        continue;
                    }
                    if entry.tier == Tier::InApp {
                        if running >= limit_total {
                            continue;
                        }
                        let on_host = per_host.entry(entry.host.clone()).or_default();
                        if *on_host >= limit_per_host {
                            continue;
                        }
                        *on_host += 1;
                        running += 1;
                    }
                    entry.state = State::Running;
                    entry.generation += 1;
                    starts.push((*id, entry.generation, entry.tier));
                }
                if !starts.is_empty() {
                    self.save(&st);
                }
            }
            for (id, generation, tier) in starts {
                match tier {
                    Tier::InApp => self.start(id, generation),
                    Tier::System => self.start_system(id, generation),
                }
            }
        }

        /// Hand a download to the OS, or follow the transfer an earlier run handed it.
        fn start_system(&self, id: u64, generation: u64) {
            let part = self.part_path(id);
            let prepared = {
                let mut st = lock(&self.state);
                let Some(entry) = st.entries.get_mut(&id) else {
                    return;
                };
                if entry.generation != generation || entry.state != State::Running {
                    return;
                }
                entry.samples.clear();
                entry.hasher = None;
                entry.file = None;
                if entry.reference.is_none() {
                    // No transfer the OS still has wrote this partial file.
                    let _ = std::fs::remove_file(&part);
                }
                let title = entry.download.dest.file_name().map_or_else(
                    || entry.download.request.url().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                );
                (
                    entry.download.request.clone(),
                    entry.reference.clone(),
                    title,
                    entry.progress(id),
                )
            };
            let (request, reference, title, progress) = prepared;
            self.notify(&progress);
            if reference.is_some() && part.exists() {
                // The OS finished the body while the app was away.
                return self.finish(id, generation);
            }
            let me = self.me.clone();
            let events: SystemEvents = Arc::new(move |event| {
                if let Some(manager) = me.upgrade() {
                    manager.system_event(id, generation, event);
                }
            });
            let job = SystemJob {
                id,
                request: &request,
                part: &part,
                dir: &self.dir,
                title: &title,
                reference: reference.as_deref(),
            };
            if let Err(error) = crate::system::start(job, events) {
                self.fail(id, generation, error);
            }
        }

        fn system_event(&self, id: u64, generation: u64, event: SystemEvent) {
            match event {
                SystemEvent::Finished => self.finish(id, generation),
                SystemEvent::Failed { error, transient } => {
                    {
                        let mut st = lock(&self.state);
                        match st.entries.get_mut(&id) {
                            Some(entry) if entry.generation == generation => entry.reference = None,
                            _ => return,
                        }
                    }
                    if transient {
                        self.retry(id, generation, error.to_string(), None);
                    } else {
                        self.fail(id, generation, error);
                    }
                }
                event => {
                    let notice = {
                        let mut st = lock(&self.state);
                        let Some(entry) = st.entries.get_mut(&id) else {
                            return;
                        };
                        if entry.generation != generation || entry.state != State::Running {
                            return;
                        }
                        let (notice, save) = match event {
                            SystemEvent::Started(reference) => {
                                entry.reference = Some(reference);
                                (None, true)
                            }
                            SystemEvent::Resumed(offset) => {
                                entry.resumed_from = offset;
                                entry.received = entry.received.max(offset);
                                (Some(entry.progress(id)), false)
                            }
                            SystemEvent::Progress { received, total } => {
                                if total.is_some() {
                                    entry.total = total;
                                }
                                let due = entry.sample(received);
                                (due.then(|| entry.progress(id)), false)
                            }
                            SystemEvent::Finished | SystemEvent::Failed { .. } => (None, false),
                        };
                        if save {
                            self.save(&st);
                        }
                        notice
                    };
                    if let Some(progress) = notice {
                        self.notify(&progress);
                    }
                }
            }
        }

        /// Open the partial file and send the request, resuming when the file and a validator allow.
        fn start(&self, id: u64, generation: u64) {
            let part = self.part_path(id);
            let prepared = {
                let mut st = lock(&self.state);
                let Some(entry) = st.entries.get_mut(&id) else {
                    return;
                };
                if entry.generation != generation || entry.state != State::Running {
                    return;
                }
                let mut len = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
                if len > 0 && entry.validator.is_none() {
                    // Nothing proves the server still has the same file: start over.
                    len = 0;
                    entry.hasher = None;
                }
                let opened = if len == 0 {
                    File::create(&part)
                } else {
                    OpenOptions::new().append(true).open(&part)
                };
                match opened {
                    Ok(file) => entry.file = Some(file),
                    Err(e) => {
                        drop(st);
                        return self.fail(id, generation, io(e));
                    }
                }
                if let Some(total) = entry.total.or(entry.download.expected_len)
                    && let Some(available) = available_space(&self.dir)
                {
                    let needed = total.saturating_sub(len);
                    if available < needed {
                        drop(st);
                        return self.fail(
                            id,
                            generation,
                            DownloadError::NoSpace { needed, available },
                        );
                    }
                }
                entry.received = len;
                entry.resumed_from = len;
                entry.samples.clear();
                if len == 0 {
                    entry.hasher = Some(Sha256::new());
                }
                let rehash = len > 0 && entry.hasher.is_none();
                let mut request = entry.download.request.clone();
                if len > 0
                    && let Some(validator) = &entry.validator
                {
                    request = request
                        .header("Range", &format!("bytes={len}-"))
                        .header("If-Range", validator);
                }
                let progress = entry.progress(id);
                (request, rehash, len, progress)
            };
            let (request, rehash, len, progress) = prepared;
            self.notify(&progress);
            if !rehash {
                return self.send(id, generation, request);
            }
            // The bytes on disk came from an earlier run of the app: hash them before new ones land.
            let me = self.me.clone();
            let spawned = std::thread::Builder::new()
                .name("day-downloads-rehash".into())
                .spawn(move || {
                    let Some(manager) = me.upgrade() else {
                        return;
                    };
                    match hash_prefix(&manager.part_path(id), len) {
                        Ok(hasher) => {
                            {
                                let mut st = lock(&manager.state);
                                match st.entries.get_mut(&id) {
                                    Some(entry) if entry.generation == generation => {
                                        entry.hasher = Some(hasher)
                                    }
                                    _ => return,
                                }
                            }
                            manager.send(id, generation, request);
                        }
                        Err(e) => manager.fail(id, generation, e),
                    }
                });
            if let Err(e) = spawned {
                self.fail(id, generation, io(e));
            }
        }

        fn send(&self, id: u64, generation: u64, request: Request) {
            let me = self.me.clone();
            let in_flight = self.client.send_async(request, move |head| {
                if let Some(manager) = me.upgrade() {
                    manager.head(id, generation, head);
                }
            });
            let late = {
                let mut st = lock(&self.state);
                match st.entries.get_mut(&id) {
                    Some(entry)
                        if entry.generation == generation && entry.state == State::Running =>
                    {
                        entry.in_flight = Some(in_flight);
                        None
                    }
                    _ => Some(in_flight),
                }
            };
            if let Some(in_flight) = late {
                in_flight.cancel();
            }
        }

        fn head(&self, id: u64, generation: u64, head: Result<Streaming, HttpError>) {
            let streaming = match head {
                Ok(streaming) => streaming,
                Err(e) => return self.transport_failed(id, generation, e),
            };
            let status = streaming.status();
            let part = self.part_path(id);
            let (next, progress) = {
                let mut st = lock(&self.state);
                let Some(entry) = st.entries.get_mut(&id) else {
                    return;
                };
                if entry.generation != generation || entry.state != State::Running {
                    return;
                }
                let next = match status {
                    206 => match content_range(streaming.header("content-range")) {
                        Some((start, total)) if start == entry.received => {
                            entry.total = total.or(entry.total);
                            Next::Read
                        }
                        _ => Next::Restart,
                    },
                    200..=299 => {
                        if entry.received > 0 {
                            // The server sent the whole file: the one on the server changed, or it
                            // ignores ranges. Start the partial file over.
                            match File::create(&part) {
                                Ok(file) => entry.file = Some(file),
                                Err(e) => {
                                    let error = io(e);
                                    drop(st);
                                    return self.fail(id, generation, error);
                                }
                            }
                            entry.received = 0;
                            entry.resumed_from = 0;
                            entry.hasher = Some(Sha256::new());
                        }
                        entry.validator = validator_of(&streaming);
                        entry.total = streaming.expected_length().or(entry.download.expected_len);
                        Next::Read
                    }
                    416 if entry.received > 0 && entry.total == Some(entry.received) => {
                        Next::Complete
                    }
                    416 => Next::Restart,
                    408 | 429 | 500..=599 => {
                        Next::Retry(retry_after(streaming.header("retry-after")))
                    }
                    other => Next::Fail(DownloadError::Http(HttpError::Status(other))),
                };
                if matches!(next, Next::Read) {
                    self.save(&st);
                }
                let progress = st.entries.get(&id).map(|e| e.progress(id));
                (next, progress)
            };
            match next {
                Next::Read => {
                    if let Some(progress) = progress {
                        self.notify(&progress);
                    }
                    self.read(id, generation, streaming.into_body());
                }
                Next::Restart => {
                    drop(streaming);
                    let restart = {
                        let mut st = lock(&self.state);
                        match st.entries.get_mut(&id) {
                            Some(entry) if entry.generation == generation => {
                                entry.validator = None;
                                entry.hasher = None;
                                entry.generation += 1;
                                entry.file = None;
                                let _ = std::fs::remove_file(&part);
                                Some(entry.generation)
                            }
                            _ => None,
                        }
                    };
                    if let Some(generation) = restart {
                        self.start(id, generation);
                    }
                }
                Next::Complete => {
                    drop(streaming);
                    self.finish(id, generation);
                }
                Next::Retry(after) => {
                    drop(streaming);
                    self.retry(id, generation, format!("status {status}"), after);
                }
                Next::Fail(error) => {
                    drop(streaming);
                    self.fail(id, generation, error);
                }
            }
        }

        fn read(&self, id: u64, generation: u64, body: Body) {
            let me = self.me.clone();
            body.read_async(move |item| {
                let Some(manager) = me.upgrade() else {
                    return false;
                };
                match item {
                    Some(Ok(chunk)) => manager.chunk(id, generation, &chunk),
                    Some(Err(e)) => {
                        manager.transport_failed(id, generation, e);
                        false
                    }
                    None => {
                        manager.finish(id, generation);
                        false
                    }
                }
            });
        }

        fn chunk(&self, id: u64, generation: u64, bytes: &[u8]) -> bool {
            let (notice, failure) = {
                let mut st = lock(&self.state);
                let Some(entry) = st.entries.get_mut(&id) else {
                    return false;
                };
                if entry.generation != generation || entry.state != State::Running {
                    return false;
                }
                let Some(file) = entry.file.as_mut() else {
                    return false;
                };
                match file.write_all(bytes) {
                    Ok(()) => {
                        if let Some(hasher) = entry.hasher.as_mut() {
                            hasher.update(bytes);
                        }
                        let due = entry.sample(entry.received + bytes.len() as u64);
                        (due.then(|| entry.progress(id)), None)
                    }
                    Err(e) => (None, Some(io(e))),
                }
            };
            if let Some(error) = failure {
                self.fail(id, generation, error);
                return false;
            }
            if let Some(progress) = notice {
                self.notify(&progress);
            }
            true
        }

        /// The body ended: check the file and move it into place.
        fn finish(&self, id: u64, generation: u64) {
            let taken = {
                let mut st = lock(&self.state);
                let Some(entry) = st.entries.get_mut(&id) else {
                    return;
                };
                if entry.generation != generation || entry.state != State::Running {
                    return;
                }
                entry.state = State::Verifying;
                if let Some(mut file) = entry.file.take() {
                    let _ = file.flush();
                }
                entry.in_flight = None;
                entry.samples.clear();
                let taken = (
                    entry.download.dest.clone(),
                    entry.download.expected_len.or(entry.total),
                    entry.download.sha256.clone(),
                    entry.hasher.take(),
                    entry.progress(id),
                );
                self.save(&st);
                taken
            };
            let (dest, expected_len, expected_sha256, hasher, progress) = taken;
            self.notify(&progress);
            let part = self.part_path(id);
            let verified = verify(
                &part,
                &dest,
                expected_len,
                expected_sha256.as_deref(),
                hasher,
            );
            let progress = {
                let mut st = lock(&self.state);
                let Some(entry) = st.entries.get_mut(&id) else {
                    return;
                };
                if entry.generation != generation {
                    return;
                }
                match verified {
                    Ok((len, digest)) => {
                        entry.state = State::Done;
                        entry.received = len;
                        entry.total = Some(len);
                        entry.sha256 = Some(digest);
                        entry.error = None;
                    }
                    Err(error) => {
                        entry.state = State::Failed;
                        entry.error = Some(error.to_string());
                        // A system-tier body came whole from the OS, whose transfer is over: the
                        // next attempt starts a new one.
                        if matches!(error, DownloadError::Integrity { .. })
                            || entry.tier == Tier::System
                        {
                            entry.received = 0;
                            entry.validator = None;
                            entry.reference = None;
                            let _ = std::fs::remove_file(&part);
                        }
                    }
                }
                let progress = entry.progress(id);
                self.save(&st);
                progress
            };
            self.notify(&progress);
            self.pump();
        }

        fn transport_failed(&self, id: u64, generation: u64, error: HttpError) {
            match error {
                HttpError::Timeout
                | HttpError::Connect
                | HttpError::Dns
                | HttpError::Io(_)
                | HttpError::Cancelled => self.retry(id, generation, error.to_string(), None),
                other => self.fail(id, generation, DownloadError::Http(other)),
            }
        }

        fn retry(&self, id: u64, generation: u64, reason: String, after: Option<Duration>) {
            let (progress, timer_due) = {
                let mut st = lock(&self.state);
                let Some(entry) = st.entries.get_mut(&id) else {
                    return;
                };
                if entry.generation != generation || entry.state != State::Running {
                    return;
                }
                entry.file = None;
                entry.in_flight = None;
                entry.samples.clear();
                entry.attempt += 1;
                entry.error = Some(reason);
                let due = if entry.attempt > entry.download.retries {
                    entry.state = State::Failed;
                    None
                } else {
                    entry.state = State::Retrying;
                    Some(
                        after.unwrap_or_else(|| backoff(entry.download.retry_delay, entry.attempt)),
                    )
                };
                let progress = entry.progress(id);
                self.save(&st);
                (progress, due)
            };
            if let Some(delay) = timer_due {
                let me = self.me.clone();
                let timer = day_async::schedule(delay, move || {
                    if let Some(manager) = me.upgrade() {
                        manager.retry_due(id, generation);
                    }
                });
                let mut st = lock(&self.state);
                match st.entries.get_mut(&id) {
                    Some(entry)
                        if entry.generation == generation && entry.state == State::Retrying =>
                    {
                        entry.timer = Some(timer)
                    }
                    _ => day_async::unschedule(timer),
                }
            }
            self.notify(&progress);
            self.pump();
        }

        fn retry_due(&self, id: u64, generation: u64) {
            {
                let mut st = lock(&self.state);
                match st.entries.get_mut(&id) {
                    Some(entry)
                        if entry.generation == generation && entry.state == State::Retrying =>
                    {
                        entry.state = State::Queued;
                        entry.timer = None;
                    }
                    _ => return,
                }
            }
            self.pump();
        }

        fn fail(&self, id: u64, generation: u64, error: DownloadError) {
            let progress = {
                let mut st = lock(&self.state);
                let Some(entry) = st.entries.get_mut(&id) else {
                    return;
                };
                if entry.generation != generation {
                    return;
                }
                let stopped = entry.stop();
                entry.state = State::Failed;
                entry.error = Some(error.to_string());
                let progress = entry.progress(id);
                self.save(&st);
                drop(st);
                release(stopped);
                progress
            };
            self.notify(&progress);
            self.pump();
        }

        /// Write the journal: one line per download, replaced whole.
        fn save(&self, st: &Shared) {
            let mut text = String::new();
            for (id, entry) in &st.entries {
                let request = &entry.download.request;
                let headers: Vec<String> = request
                    .headers()
                    .iter()
                    .filter(|(k, _)| {
                        !k.eq_ignore_ascii_case("range") && !k.eq_ignore_ascii_case("if-range")
                    })
                    .map(|(k, v)| format!("{k}: {v}"))
                    .collect();
                let fields = [
                    id.to_string(),
                    entry.state.label().to_string(),
                    request.method().as_str().to_string(),
                    request.url().to_string(),
                    entry.download.dest.to_string_lossy().into_owned(),
                    optional(entry.total),
                    entry.validator.clone().unwrap_or_else(|| "-".into()),
                    entry.download.sha256.clone().unwrap_or_else(|| "-".into()),
                    entry.download.retries.to_string(),
                    entry.download.retry_delay.as_millis().to_string(),
                    headers.join("\n"),
                    optional(entry.download.expected_len),
                    entry.sha256.clone().unwrap_or_else(|| "-".into()),
                    entry.error.clone().unwrap_or_else(|| "-".into()),
                    match entry.tier {
                        Tier::InApp => "app".into(),
                        Tier::System => "system".into(),
                    },
                    entry.reference.clone().unwrap_or_else(|| "-".into()),
                ];
                let line: Vec<String> = fields.iter().map(|f| escape(f)).collect();
                text.push_str(&line.join("\t"));
                text.push('\n');
            }
            let path = self.dir.join(JOURNAL);
            let partial = self.dir.join(format!("{JOURNAL}.partial"));
            if std::fs::write(&partial, text).is_ok() {
                let _ = std::fs::rename(&partial, &path);
            }
        }
    }

    fn optional(value: Option<u64>) -> String {
        value.map_or_else(|| "-".into(), |v| v.to_string())
    }

    fn escape(field: &str) -> String {
        field
            .replace('%', "%25")
            .replace('\t', "%09")
            .replace('\n', "%0A")
            .replace('\r', "%0D")
    }

    fn unescape(field: &str) -> String {
        field
            .replace("%0D", "\r")
            .replace("%0A", "\n")
            .replace("%09", "\t")
            .replace("%25", "%")
    }

    fn load_journal(dir: &Path) -> BTreeMap<u64, Entry> {
        let mut entries = BTreeMap::new();
        let Ok(text) = std::fs::read_to_string(dir.join(JOURNAL)) else {
            return entries;
        };
        for line in text.lines() {
            let fields: Vec<String> = line.split('\t').map(unescape).collect();
            let Some(entry) = decode_entry(dir, &fields) else {
                continue;
            };
            entries.insert(entry.0, entry.1);
        }
        entries
    }

    fn decode_entry(dir: &Path, fields: &[String]) -> Option<(u64, Entry)> {
        let dash = |s: &String| (s != "-").then(|| s.clone());
        let id: u64 = fields.first()?.parse().ok()?;
        let state = State::parse(fields.get(1)?)?;
        let method = Method::parse(fields.get(2)?)?;
        let mut request = Request::new(method, fields.get(3)?.clone());
        for header in fields.get(10)?.split('\n').filter(|h| !h.is_empty()) {
            if let Some((k, v)) = header.split_once(": ") {
                request = request.header(k, v);
            }
        }
        let mut download = Download::new(request, PathBuf::from(fields.get(4)?))
            .retries(fields.get(8)?.parse().ok()?)
            .retry_delay(Duration::from_millis(fields.get(9)?.parse().ok()?));
        if let Some(len) = fields.get(11).and_then(dash).and_then(|s| s.parse().ok()) {
            download = download.expect_len(len);
        }
        if let Some(sha) = fields.get(7).and_then(dash) {
            download = download.expect_sha256(&sha);
        }
        let mut entry = Entry::new(download);
        entry.total = fields.get(5).and_then(dash).and_then(|s| s.parse().ok());
        entry.validator = fields.get(6).and_then(dash);
        entry.sha256 = fields.get(12).and_then(dash);
        entry.error = fields.get(13).and_then(dash);
        if fields.get(14).is_some_and(|t| t == "system") && crate::system::available() {
            entry.tier = Tier::System;
            entry.reference = fields.get(15).and_then(dash);
        }
        entry.state = match state {
            State::Running | State::Retrying | State::Verifying => State::Queued,
            other => other,
        };
        entry.received = match entry.state {
            State::Done => entry.total.unwrap_or(0),
            State::Cancelled => 0,
            _ => std::fs::metadata(dir.join(format!("{id}.part")))
                .map(|m| m.len())
                .unwrap_or(0),
        };
        Some((id, entry))
    }

    /// `(first byte, whole length)` from `Content-Range: bytes first-last/total`.
    fn content_range(header: Option<&str>) -> Option<(u64, Option<u64>)> {
        let spec = header?.trim().strip_prefix("bytes ")?;
        let (range, total) = spec.split_once('/')?;
        let (first, _) = range.split_once('-')?;
        Some((first.trim().parse().ok()?, total.trim().parse().ok()))
    }

    /// A validator `If-Range` accepts: a strong ETag, else `Last-Modified`.
    fn validator_of(streaming: &Streaming) -> Option<String> {
        streaming
            .header("etag")
            .filter(|e| !e.starts_with("W/"))
            .or_else(|| streaming.header("last-modified"))
            .map(str::to_string)
    }

    /// `Retry-After` as seconds or an HTTP date.
    fn retry_after(header: Option<&str>) -> Option<Duration> {
        let value = header?.trim();
        if let Ok(seconds) = value.parse::<u64>() {
            return Some(Duration::from_secs(seconds).min(MAX_BACKOFF));
        }
        let at = httpdate::parse_http_date(value).ok()?;
        Some(
            at.duration_since(SystemTime::now())
                .unwrap_or_default()
                .min(MAX_BACKOFF),
        )
    }

    /// `base × 2^(attempt − 1)`, capped at a minute, with a fifth either way of jitter.
    fn backoff(base: Duration, attempt: u32) -> Duration {
        let doubled = base.saturating_mul(1u32 << attempt.saturating_sub(1).min(16));
        let capped = doubled.min(MAX_BACKOFF);
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let jitter = 0.8 + f64::from(nanos % 400) / 1000.0;
        capped.mul_f64(jitter)
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn hash_prefix(path: &Path, len: u64) -> Result<Sha256, DownloadError> {
        let mut file = File::open(path).map_err(io)?.take(len);
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 << 10];
        loop {
            let n = file.read(&mut buf).map_err(io)?;
            if n == 0 {
                return Ok(hasher);
            }
            hasher.update(&buf[..n]);
        }
    }

    /// Check the partial file and move it to `dest`: `(length, hex SHA-256)`.
    fn verify(
        part: &Path,
        dest: &Path,
        expected_len: Option<u64>,
        expected_sha256: Option<&str>,
        hasher: Option<Sha256>,
    ) -> Result<(u64, String), DownloadError> {
        let len = std::fs::metadata(part).map_err(io)?.len();
        if let Some(expected) = expected_len
            && expected != len
        {
            return Err(DownloadError::Length {
                expected,
                actual: len,
            });
        }
        let hasher = match hasher {
            Some(hasher) => hasher,
            None => hash_prefix(part, len)?,
        };
        let digest = hex(hasher.finalize().as_slice());
        if let Some(expected) = expected_sha256
            && expected != digest
        {
            return Err(DownloadError::Integrity {
                expected: expected.to_string(),
                actual: digest,
            });
        }
        crate::move_file(part, dest)?;
        Ok((len, digest))
    }

    #[cfg(unix)]
    fn available_space(dir: &Path) -> Option<u64> {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        // SAFETY: an all-zero statvfs is a valid out-parameter for the call to fill.
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: a NUL-terminated path and a writable statvfs.
        let rc = unsafe { libc::statvfs(path.as_ptr(), &mut stat) };
        // The field widths differ per platform (u32 on Apple, u64 on Linux).
        #[allow(clippy::unnecessary_cast)]
        let available = stat.f_bavail as u64 * stat.f_frsize as u64;
        (rc == 0).then_some(available)
    }

    #[cfg(windows)]
    fn available_space(dir: &Path) -> Option<u64> {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetDiskFreeSpaceExW(
                directory: *const u16,
                free_to_caller: *mut u64,
                total: *mut u64,
                free: *mut u64,
            ) -> i32;
        }
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut available = 0u64;
        // SAFETY: a NUL-terminated wide path and one writable out-parameter; the others may be null.
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(available)
    }

    #[cfg(not(any(unix, windows)))]
    fn available_space(_dir: &Path) -> Option<u64> {
        None
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn content_ranges_and_retry_after_parse() {
            assert_eq!(
                content_range(Some("bytes 100-199/1000")),
                Some((100, Some(1000)))
            );
            assert_eq!(content_range(Some("bytes 5-9/*")), Some((5, None)));
            assert_eq!(content_range(Some("items 1-2/3")), None);
            assert_eq!(retry_after(Some("7")), Some(Duration::from_secs(7)));
            assert_eq!(retry_after(Some("99999")), Some(MAX_BACKOFF));
            assert_eq!(
                retry_after(Some("Wed, 21 Oct 2015 07:28:00 GMT")),
                Some(Duration::ZERO)
            );
        }

        #[test]
        fn backoff_doubles_and_caps() {
            let base = Duration::from_millis(100);
            let first = backoff(base, 1);
            let third = backoff(base, 3);
            assert!(first >= Duration::from_millis(80) && first <= Duration::from_millis(120));
            assert!(third >= Duration::from_millis(320) && third <= Duration::from_millis(480));
            assert!(backoff(base, 30) <= MAX_BACKOFF.mul_f64(1.2));
        }

        #[test]
        fn journal_fields_round_trip() {
            for field in ["plain", "tab\there", "line\nbreak", "100%", "%09 literal"] {
                assert_eq!(unescape(&escape(field)), field);
            }
            assert_eq!(host_of("https://Example.com:8443/a?b"), "example.com:8443");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use manager::{Downloads, Watch};

#[cfg(target_arch = "wasm32")]
mod web {
    use std::path::PathBuf;

    use day_part_http::Client;

    use super::{Download, DownloadError, DownloadId, Progress};

    /// A download manager. The web gives it no files to own, so opening one reports
    /// [`DownloadError::Unsupported`]; the rest of the API exists so shared code compiles.
    #[derive(Clone, Debug)]
    pub struct Downloads(());

    /// Stops a watch when dropped.
    #[derive(Debug)]
    pub struct Watch(());

    impl Downloads {
        pub fn open(_dir: impl Into<PathBuf>) -> Result<Downloads, DownloadError> {
            Err(DownloadError::Unsupported)
        }
        pub fn with_client(
            _dir: impl Into<PathBuf>,
            _client: Client,
        ) -> Result<Downloads, DownloadError> {
            Err(DownloadError::Unsupported)
        }
        pub fn system_tier() -> bool {
            false
        }
        pub fn limits(&self, _total: usize, _per_host: usize) {}
        pub fn enqueue(&self, _download: Download) -> Result<DownloadId, DownloadError> {
            Err(DownloadError::Unsupported)
        }
        pub fn pause(&self, _id: DownloadId) -> Result<(), DownloadError> {
            Err(DownloadError::Unsupported)
        }
        pub fn resume(&self, _id: DownloadId) -> Result<(), DownloadError> {
            Err(DownloadError::Unsupported)
        }
        pub fn cancel(&self, _id: DownloadId) -> Result<(), DownloadError> {
            Err(DownloadError::Unsupported)
        }
        pub fn remove(&self, _id: DownloadId) -> Result<(), DownloadError> {
            Err(DownloadError::Unsupported)
        }
        pub fn progress(&self, _id: DownloadId) -> Option<Progress> {
            None
        }
        pub fn list(&self) -> Vec<Progress> {
            Vec::new()
        }
        pub fn watch(&self, _on_change: impl Fn(&Progress) + Send + Sync + 'static) -> Watch {
            Watch(())
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::{Downloads, Watch};
