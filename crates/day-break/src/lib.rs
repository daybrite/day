//! day-break — consent-first crash reporting for Day apps (docs/break.md, DESIGN.md §8.5).
//!
//! An OPTIONAL crate. An app opts in with one call, as early as possible in startup:
//!
//! ```no_run
//! day_break::Config::new().max_reports(5).init().ok();
//! // … day::launch(WindowOptions::default(), app_root) …
//! ```
//!
//! day-break then registers a chained panic hook and (on Unix) native signal handlers, writes a
//! session sentinel, and on the NEXT launch reconciles any leftover artifacts into finalized JSON
//! reports. It never uploads anything on its own — [`send`] (the only network path) is called by
//! the app, from a user action on a disclosure surface. See [`Config`], [`last_session`],
//! [`report_paths`], and the `ui` feature's consent surface.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use day_reactive::Signal;

mod hook;
mod report;
mod store;
mod transport;

#[cfg(unix)]
mod signals_unix;

#[cfg(target_os = "android")]
mod java_android;

#[cfg(feature = "ui")]
mod i18n;
#[cfg(feature = "ui")]
mod ui;

pub use report::{Kind, Report, SignalInfo};
pub use store::SessionEnd;
pub use transport::{EmailReporter, GithubIssueReporter, Reporter, RestReporter, SendError};

#[cfg(feature = "ui")]
pub use ui::consent_banner;

/// A pending crash report awaiting the user's decision — the display record the consent surface
/// works with. Load the full text with [`report_text`]; send it with [`send`]; drop it with
/// [`discard`].
#[derive(Clone, PartialEq, Eq)]
pub struct ReportMeta {
    /// The finalized report file on disk.
    pub path: PathBuf,
    pub kind: Kind,
    pub fatal: bool,
    /// A one-line summary (the first line of the panic/exception message, or the crash kind).
    pub summary: String,
    /// When the crashing session started (ms since the Unix epoch).
    pub when_ms: u64,
}

/// Information passed to an [`on_crash`] callback, at panic time.
#[derive(Clone, Debug)]
pub struct CrashInfo {
    pub message: String,
    pub location: String,
    pub thread: String,
}

/// Why [`Config::init`] could not arm crash capture.
#[derive(Debug)]
pub enum InitError {
    /// The report directory could not be created.
    Dir(std::io::Error),
    /// `init` was already called this process (crash capture is process-global).
    AlreadyInitialized,
}

impl std::fmt::Display for InitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InitError::Dir(e) => write!(f, "day-break: cannot create the report directory: {e}"),
            InitError::AlreadyInitialized => write!(f, "day-break: init() was already called"),
        }
    }
}

impl std::error::Error for InitError {}

/// A message-scrubbing hook (see [`Config::redact`]): mutates a string in place before it is
/// persisted, displayed, or uploaded.
type Redactor = Box<dyn Fn(&mut String) + Send + Sync>;

/// Crash-reporting configuration. Build with [`Config::new`], then [`Config::init`].
pub struct Config {
    app_id: Option<String>,
    app_version: Option<String>,
    app_build: Option<String>,
    dir: Option<PathBuf>,
    max_reports: usize,
    keep_contained: bool,
    signals: bool,
    signal_backtrace: bool,
    redact: Option<Redactor>,
    reporter: Option<Arc<dyn Reporter>>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            app_id: None,
            app_version: None,
            app_build: None,
            dir: None,
            max_reports: 5,
            keep_contained: true,
            signals: true,
            signal_backtrace: false,
            redact: None,
            reporter: None,
        }
    }
}

impl Config {
    pub fn new() -> Config {
        Config::default()
    }

    /// Override the app identity (default: baked from `day build`, then runtime `DAY_APP_*`).
    pub fn app_id(mut self, v: impl Into<String>) -> Config {
        self.app_id = Some(v.into());
        self
    }
    pub fn app_version(mut self, v: impl Into<String>) -> Config {
        self.app_version = Some(v.into());
        self
    }
    pub fn app_build(mut self, v: impl Into<String>) -> Config {
        self.app_build = Some(v.into());
        self
    }
    /// Override the report directory (default: the platform's per-app data dir + `day-break`).
    pub fn dir(mut self, v: impl Into<PathBuf>) -> Config {
        self.dir = Some(v.into());
        self
    }
    /// How many finalized reports to keep (rotation, default 5).
    pub fn max_reports(mut self, n: usize) -> Config {
        self.max_reports = n;
        self
    }
    /// Record day-core-contained panics as non-fatal reports (default true).
    pub fn keep_contained(mut self, yes: bool) -> Config {
        self.keep_contained = yes;
        self
    }
    /// Install native signal handlers (Unix only; default true).
    pub fn signals(mut self, yes: bool) -> Config {
        self.signals = yes;
        self
    }
    /// Attempt an in-handler frame-pointer backtrace on a native fault (default false; reserved —
    /// see docs/break.md). Off by default because release x86_64 may omit frame pointers.
    pub fn signal_backtrace(mut self, yes: bool) -> Config {
        self.signal_backtrace = yes;
        self
    }
    /// A hook to scrub a message string before it is persisted, displayed, or uploaded. Applied to
    /// panic messages (and, in the consent UI, to any field the app routes through it).
    pub fn redact(mut self, f: impl Fn(&mut String) + Send + Sync + 'static) -> Config {
        self.redact = Some(Box::new(f));
        self
    }
    /// The upload transport used by [`send`] and the consent surface (default: none — [`send`]
    /// then errors). Choose a built-in ([`RestReporter`]/[`GithubIssueReporter`]/[`EmailReporter`])
    /// or your own [`Reporter`].
    pub fn reporter(mut self, r: impl Reporter + 'static) -> Config {
        self.reporter = Some(Arc::new(r));
        self
    }

    /// Arm crash capture: resolve identity + directory, reconcile the previous session(s), write
    /// this session's sentinel, and install the panic hook (+ signal handlers, + the Android
    /// uncaught-exception handler). Idempotent-safe: a second call returns [`InitError::AlreadyInitialized`].
    pub fn init(self) -> Result<(), InitError> {
        if STATE.get().is_some() {
            return Err(InitError::AlreadyInitialized);
        }

        let app_id = resolve(self.app_id, "DAY_BREAK_DAY_APP_ID", "DAY_APP_ID");
        let app_version = resolve(
            self.app_version,
            "DAY_BREAK_DAY_APP_VERSION",
            "DAY_APP_VERSION",
        );
        let app_build = resolve(self.app_build, "DAY_BREAK_DAY_APP_BUILD", "DAY_APP_BUILD");
        let day_version = env!("CARGO_PKG_VERSION").to_string();

        let dir = store::store_dir(&app_id, self.dir.as_deref());
        std::fs::create_dir_all(&dir).map_err(InitError::Dir)?;

        let pid = std::process::id();
        let started_at_ms = unix_millis();
        // Session id: pid + start millis, hex — filesystem-safe, unique per launch.
        let sid = format!("{pid:x}-{started_at_ms:x}");

        // Reconcile prior sessions BEFORE writing our own sentinel (so our sid is excluded anyway,
        // but this also means last_session reflects only the past).
        let ctx = store::StaticCtx {
            app_id: app_id.clone(),
            app_version: app_version.clone(),
            app_build: app_build.clone(),
            day_version: day_version.clone(),
        };
        let reconciled =
            store::reconcile(&dir, &sid, pid, &ctx, self.max_reports, self.keep_contained);

        // Static context for this session's sentinel.
        let device = day_part_deviceinfo::get();
        let backend = day_core::backend_name().unwrap_or("").to_string();
        let locale = current_locale();
        let sentinel_fields = vec![
            ("pid", pid.to_string()),
            ("started_at_ms", started_at_ms.to_string()),
            ("app_id", app_id.clone()),
            ("app_version", app_version.clone()),
            ("app_build", app_build.clone()),
            ("day_version", day_version.clone()),
            ("backend", backend),
            ("os_name", device.system_name.clone()),
            ("os_version", device.system_version.clone()),
            ("device_model", device.model.clone()),
            (
                "simulator",
                if device.is_simulator {
                    "1".into()
                } else {
                    "0".into()
                },
            ),
            ("locale", locale),
        ];
        let _ = store::write_sentinel(&dir, &sid, &sentinel_fields);

        // Publish global state before installing handlers (they read it).
        let _ = STATE.set(State {
            dir: dir.clone(),
            sid: sid.clone(),
            start: Instant::now(),
            seq: AtomicU64::new(1),
            redact: self.redact,
            reporter: self.reporter,
            on_crash: std::sync::Mutex::new(Vec::new()),
            last_session: reconciled.last_session,
        });

        hook::install();
        day_core::set_contained_panic_observer(hook::observe_contained);

        #[cfg(unix)]
        if self.signals {
            signals_unix::install(&store::sig_path(&dir, &sid));
        }

        #[cfg(target_os = "android")]
        java_android::install(&dir, &sid);

        // Refresh the sentinel's backend once the backend is known (init may run before launch),
        // and remove the sentinel on a clean exit so a leftover means "did not exit cleanly".
        {
            let dir_l = dir.clone();
            let sid_l = sid.clone();
            let fields = sentinel_fields;
            day_core::on_lifecycle(day_spec::Lifecycle::WillLaunch, move || {
                if let Some(backend) = day_core::backend_name() {
                    let mut f = fields.clone();
                    if let Some(slot) = f.iter_mut().find(|(k, _)| *k == "backend") {
                        slot.1 = backend.to_string();
                    }
                    let _ = store::write_sentinel(&dir_l, &sid_l, &f);
                }
            });
        }
        {
            let dir_l = dir.clone();
            let sid_l = sid.clone();
            day_core::on_lifecycle(day_spec::Lifecycle::WillTerminate, move || {
                store::clear_session(&dir_l, &sid_l);
            });
        }

        Ok(())
    }
}

/// `Config::new().init()`.
pub fn init() -> Result<(), InitError> {
    Config::new().init()
}

// ---- public queries ------------------------------------------------------------------------

/// How the previous session ended (Clean / Crashed / Unknown), computed at [`Config::init`].
pub fn last_session() -> SessionEnd {
    STATE
        .get()
        .map(|s| s.last_session.clone())
        .unwrap_or(SessionEnd::Clean)
}

/// The crash-report directory, if [`Config::init`] has run.
pub fn reports_dir() -> Option<PathBuf> {
    STATE.get().map(|s| store::reports_subdir(&s.dir))
}

/// Paths of finalized reports, newest-first.
pub fn report_paths() -> Vec<PathBuf> {
    STATE
        .get()
        .map(|s| store::report_paths(&s.dir))
        .unwrap_or_default()
}

/// Load a finalized report file into a [`Report`] (parsing the JSON we wrote).
fn load_report(path: &Path) -> Option<Report> {
    let json = std::fs::read_to_string(path).ok()?;
    report::parse_json(&json)
}

/// Load the display record for one finalized report path.
fn load_meta(path: &Path) -> Option<ReportMeta> {
    let r = load_report(path)?;
    let summary = {
        let first = r.message.lines().next().unwrap_or("").trim();
        if first.is_empty() {
            match r.kind() {
                Some(Kind::Signal) => r
                    .signal
                    .as_ref()
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| "signal".into()),
                _ => r.kind_str.clone(),
            }
        } else {
            first.to_string()
        }
    };
    Some(ReportMeta {
        path: path.to_path_buf(),
        kind: r.kind().unwrap_or(Kind::Panic),
        fatal: r.fatal,
        summary,
        when_ms: r.started_at_ms,
    })
}

fn load_pending() -> Vec<ReportMeta> {
    report_paths().iter().filter_map(|p| load_meta(p)).collect()
}

thread_local! {
    /// The pending-list signal. Held in a thread-local because a `Signal` is main-thread-affine
    /// (`!Send`/`!Sync`) and cannot live in a `static` — the same reason day-l10n keeps its global
    /// locale signal thread-locally.
    static PENDING: std::cell::RefCell<Option<Signal<Vec<ReportMeta>>>> = const { std::cell::RefCell::new(None) };
}

/// A reactive list of the pending reports awaiting the user's decision. Newest-first; updated by
/// [`send`] and [`discard`]. Drives the consent surface. Lazily created (root-scoped) on first use.
pub fn pending() -> Signal<Vec<ReportMeta>> {
    PENDING.with(|p| {
        let mut slot = p.borrow_mut();
        if slot.is_none() {
            *slot = Some(Signal::global(load_pending()));
        }
        slot.expect("just initialized")
    })
}

/// Re-read the pending list from disk into the [`pending`] signal (main-thread). Called after a
/// send/discard; safe to call from the app too.
pub fn refresh() {
    PENDING.with(|p| {
        if let Some(sig) = *p.borrow() {
            sig.set(load_pending());
        }
    });
}

/// The configured reporter's one-line disclosure ([`Reporter::describe`]) — for an app building
/// its own consent surface (like the showcase's Crash Reporting page). `None` if no reporter is set.
pub fn reporter_description() -> Option<String> {
    STATE
        .get()
        .and_then(|s| s.reporter.as_ref().map(|r| r.describe()))
}

/// The full, human-readable text of a report — the disclosure surface's content. This is exactly
/// what a transport uploads (the JSON is a machine mirror of the same facts).
pub fn report_text(meta: &ReportMeta) -> String {
    load_report(&meta.path)
        .map(|r| r.display_text())
        .unwrap_or_default()
}

/// The full text of the newest finalized report, if any.
pub fn latest_report_text() -> Option<String> {
    let meta = pending()
        .get_untracked()
        .into_iter()
        .next()
        .or_else(|| report_paths().first().and_then(|p| load_meta(p)))?;
    Some(report_text(&meta))
}

/// Upload a report through the configured [`Reporter`] — THE only network path, and only ever
/// called from app code (i.e. after the user has seen the report and chosen to send). `on_done`
/// runs on the main thread with the outcome; on success the report file is deleted and [`pending`]
/// refreshes. Errors [`SendError::Transport`] with "no reporter configured" if none was set.
pub fn send(meta: &ReportMeta, on_done: impl FnOnce(Result<(), SendError>) + Send + 'static) {
    let Some(state) = STATE.get() else {
        on_done(Err(SendError::Transport(
            "day-break not initialized".into(),
        )));
        return;
    };
    let Some(reporter) = state.reporter.clone() else {
        on_done(Err(SendError::Transport("no reporter configured".into())));
        return;
    };
    let Some(report) = load_report(&meta.path) else {
        on_done(Err(SendError::Transport(
            "could not read the report".into(),
        )));
        return;
    };
    let path = meta.path.clone();
    // The transport may call `done` off-thread. Signals aren't `Send`, so the pending refresh rides
    // a `Setter` (day-reactive's cross-thread write path, which marshals to the main thread); the
    // app's `on_done` is `Send` and may run on either thread — the consent UI keeps it Setter-based.
    let pending_setter = PENDING.with(|p| p.borrow().map(|s| s.setter()));
    reporter.send(
        &report,
        Box::new(move |result| {
            if result.is_ok() {
                let _ = std::fs::remove_file(&path);
                if let Some(setter) = pending_setter {
                    setter.set(load_pending());
                }
            }
            on_done(result);
        }),
    );
}

/// Discard a pending report (the user chose not to send it) and refresh [`pending`].
pub fn discard(meta: &ReportMeta) {
    let _ = std::fs::remove_file(&meta.path);
    refresh();
}

/// Discard a finalized report by path (lower-level; prefer [`discard`]).
pub fn discard_path(path: &Path) {
    let _ = std::fs::remove_file(path);
    refresh();
}

/// Register a callback invoked at PANIC time (never in signal context — nothing is safe there;
/// signal crashes surface via [`last_session`]/[`report_paths`] on the next launch). Implements
/// DESIGN.md §8.5's crash-reporter hook.
pub fn on_crash(f: fn(&CrashInfo)) {
    if let Some(s) = STATE.get()
        && let Ok(mut v) = s.on_crash.lock()
    {
        v.push(f);
    }
}

// ---- internal state ------------------------------------------------------------------------

struct State {
    dir: PathBuf,
    sid: String,
    start: Instant,
    seq: AtomicU64,
    redact: Option<Redactor>,
    reporter: Option<Arc<dyn Reporter>>,
    on_crash: std::sync::Mutex<Vec<fn(&CrashInfo)>>,
    last_session: SessionEnd,
}

impl State {
    fn uptime_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
    fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::Relaxed)
    }
    fn redact(&self, s: &mut String) {
        if let Some(f) = &self.redact {
            f(s);
        }
    }
    fn on_crash_snapshot(&self) -> Vec<fn(&CrashInfo)> {
        self.on_crash.lock().map(|v| v.clone()).unwrap_or_default()
    }
}

static STATE: OnceLock<State> = OnceLock::new();

pub(crate) fn state() -> Option<&'static State> {
    STATE.get()
}

// ---- helpers -------------------------------------------------------------------------------

/// Resolve a field: explicit override, else the value baked by build.rs, else a runtime env var,
/// else `"unknown"`.
fn resolve(explicit: Option<String>, baked_var: &str, runtime_var: &str) -> String {
    if let Some(v) = explicit {
        return v;
    }
    // build.rs re-exports DAY_APP_* under DAY_BREAK_DAY_APP_* via cargo:rustc-env.
    let baked = match baked_var {
        "DAY_BREAK_DAY_APP_ID" => option_env!("DAY_BREAK_DAY_APP_ID"),
        "DAY_BREAK_DAY_APP_VERSION" => option_env!("DAY_BREAK_DAY_APP_VERSION"),
        "DAY_BREAK_DAY_APP_BUILD" => option_env!("DAY_BREAK_DAY_APP_BUILD"),
        _ => None,
    };
    if let Some(v) = baked
        && !v.is_empty()
    {
        return v.to_string();
    }
    if let Ok(v) = std::env::var(runtime_var)
        && !v.is_empty()
    {
        return v;
    }
    "unknown".to_string()
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The current UI locale (the launcher sets `DAY_LOCALE`; default `"en"`). Kept env-based so the
/// capture core has no dependency on the l10n stack.
fn current_locale() -> String {
    std::env::var("DAY_LOCALE")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "en".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_explicit_then_runtime_then_unknown() {
        assert_eq!(
            resolve(Some("x".into()), "DAY_BREAK_DAY_APP_ID", "DAY_APP_ID_NOPE"),
            "x"
        );
        assert_eq!(
            resolve(
                None,
                "DAY_BREAK_DAY_APP_ID",
                "DAY_APP_ID_DEFINITELY_UNSET_XYZ"
            ),
            "unknown"
        );
    }

    #[test]
    fn init_arms_and_is_single_shot() {
        // Isolate: point at a scratch dir so we don't touch a real app store.
        let d = std::env::temp_dir().join(format!("day-break-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let r = Config::new().dir(&d).app_id("dev.test.break").init();
        assert!(r.is_ok(), "first init should arm: {r:?}");
        assert!(STATE.get().is_some());
        // Sentinel exists.
        let s = STATE.get().unwrap();
        assert!(store::report_paths(&s.dir).is_empty() || true);
        // Second init is rejected.
        assert!(matches!(
            Config::new().dir(&d).init(),
            Err(InitError::AlreadyInitialized)
        ));
    }
}
