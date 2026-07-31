//! day-part-fs — app-local file storage with one API on every target (docs/fs.md). No UI; any
//! Rust code can depend on this crate and call [`read`]/[`write`]/[`list`]/[`remove`].
//!
//! ```no_run
//! day_part_fs::write("notes/today.txt", b"rain later")?;
//! let bytes = day_part_fs::read("notes/today.txt")?;
//! # Ok::<(), day_part_fs::FsError>(())
//! ```
//!
//! Files live under a private per-app root: a real directory on the native targets (the mobile
//! hosts pass it as `DAY_DATA_DIR`; desktops use the platform's data-dir convention), and the
//! browser's Origin Private File System on web-dom. Paths are **relative and sandboxed** — an
//! absolute path or a `.`/`..` segment is [`FsError::BadPath`] — and `write` creates missing
//! parent directories.
//!
//! **Threading.** The blocking calls run where you call them — keep them off the UI thread for
//! large files, and on web they return [`FsError::Unsupported`] (one thread, no blocking waits
//! — the day-part-http rule). The `*_async` twins and futures work on every target; completions
//! arrive on an unspecified background thread natively and on the sole browser thread on web,
//! so deliver into UI state with a `Setter` or await under `day::task` (docs/async.md).

use std::sync::{Arc, Mutex};

/// A storage failure. The web tier collapses provider detail into [`FsError::Io`], like
/// day-part-http's error taxonomy does for the browser.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FsError {
    /// The path does not exist.
    NotFound,
    /// The path is absolute or contains a `.`/`..`/empty segment.
    BadPath,
    /// Everything else the platform reported.
    Io(String),
    /// No storage on this target, or a blocking entry point on web (docs/fs.md).
    Unsupported,
}

impl std::fmt::Display for FsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FsError::NotFound => write!(f, "not found"),
            FsError::BadPath => write!(f, "bad path (relative, no `.`/`..` segments)"),
            FsError::Io(m) => write!(f, "{m}"),
            FsError::Unsupported => write!(f, "no file storage on this target"),
        }
    }
}

impl std::error::Error for FsError {}

/// Reject absolute paths and `.`/`..`/empty segments — the same rule the day CLI's web server
/// applies to request paths. Every entry point funnels through this before touching a backend.
fn check_path(path: &str) -> Result<(), FsError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.contains(':')
        || path
            .split(['/', '\\'])
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return Err(FsError::BadPath);
    }
    Ok(())
}

/// Read a file's bytes. BLOCKING; [`FsError::Unsupported`] on web — use [`read_async`] or
/// [`read_future`] there (they work everywhere).
pub fn read(path: &str) -> Result<Vec<u8>, FsError> {
    check_path(path)?;
    imp::read(path)
}

/// Write `bytes` to `path` (create or truncate), creating missing parent directories. BLOCKING;
/// [`FsError::Unsupported`] on web.
pub fn write(path: &str, bytes: &[u8]) -> Result<(), FsError> {
    check_path(path)?;
    imp::write(path, bytes)
}

/// Remove a file (or an empty directory). BLOCKING; [`FsError::Unsupported`] on web. Removing a
/// missing path is [`FsError::NotFound`].
pub fn remove(path: &str) -> Result<(), FsError> {
    check_path(path)?;
    imp::remove(path)
}

/// List the entry names directly under `dir` (`""` = the storage root), sorted. Directories are
/// listed with a trailing `/`. BLOCKING; [`FsError::Unsupported`] on web.
pub fn list(dir: &str) -> Result<Vec<String>, FsError> {
    if !dir.is_empty() {
        check_path(dir)?;
    }
    imp::list(dir)
}

type BytesResult = Result<Vec<u8>, FsError>;
type UnitResult = Result<(), FsError>;
type ListResult = Result<Vec<String>, FsError>;

/// [`read`] without blocking; `on_done` runs on an unspecified background thread (the sole
/// browser thread on web).
pub fn read_async(path: &str, on_done: impl FnOnce(BytesResult) + Send + 'static) {
    if let Err(e) = check_path(path) {
        on_done(Err(e));
        return;
    }
    imp::read_async(path.to_string(), Box::new(on_done));
}

/// [`write`] without blocking.
pub fn write_async(path: &str, bytes: Vec<u8>, on_done: impl FnOnce(UnitResult) + Send + 'static) {
    if let Err(e) = check_path(path) {
        on_done(Err(e));
        return;
    }
    imp::write_async(path.to_string(), bytes, Box::new(on_done));
}

/// [`remove`] without blocking.
pub fn remove_async(path: &str, on_done: impl FnOnce(UnitResult) + Send + 'static) {
    if let Err(e) = check_path(path) {
        on_done(Err(e));
        return;
    }
    imp::remove_async(path.to_string(), Box::new(on_done));
}

/// [`list`] without blocking.
pub fn list_async(dir: &str, on_done: impl FnOnce(ListResult) + Send + 'static) {
    if !dir.is_empty()
        && let Err(e) = check_path(dir)
    {
        on_done(Err(e));
        return;
    }
    imp::list_async(dir.to_string(), Box::new(on_done));
}

// ---------------------------------------------------------------------------
// Futures: oneshot plumbing over the async completions, awaitable under any executor
// (`day::task`, or a test's block_on) — the day-part-http shape without the cancel grip
// (storage operations are short; v1 has no cancellation).
// ---------------------------------------------------------------------------

struct FutureState<T> {
    result: Option<T>,
    waker: Option<std::task::Waker>,
}

impl<T> Default for FutureState<T> {
    fn default() -> Self {
        FutureState {
            result: None,
            waker: None,
        }
    }
}

/// An in-flight storage operation; `.await` resolves it.
pub struct FsFuture<T> {
    shared: Arc<Mutex<FutureState<T>>>,
}

fn lock<T>(m: &Mutex<FutureState<T>>) -> std::sync::MutexGuard<'_, FutureState<T>> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl<T> std::future::Future for FsFuture<T> {
    type Output = T;
    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<T> {
        let mut st = lock(&self.shared);
        if let Some(r) = st.result.take() {
            return std::task::Poll::Ready(r);
        }
        st.waker = Some(cx.waker().clone());
        std::task::Poll::Pending
    }
}

fn deliver<T>(shared: &Arc<Mutex<FutureState<T>>>, result: T) {
    let waker = {
        let mut st = lock(shared);
        st.result = Some(result);
        st.waker.take()
    };
    if let Some(w) = waker {
        w.wake();
    }
}

macro_rules! future_over {
    ($shared:ident, $call:expr) => {{
        let $shared = Arc::new(Mutex::new(FutureState::default()));
        let sink = $shared.clone();
        $call;
        FsFuture { shared: sink }
    }};
}

/// [`read_async`] as a `Future`.
pub fn read_future(path: &str) -> FsFuture<BytesResult> {
    future_over!(shared, {
        let s = shared.clone();
        read_async(path, move |r| deliver(&s, r));
    })
}

/// [`write_async`] as a `Future`.
pub fn write_future(path: &str, bytes: Vec<u8>) -> FsFuture<UnitResult> {
    future_over!(shared, {
        let s = shared.clone();
        write_async(path, bytes, move |r| deliver(&s, r));
    })
}

/// [`remove_async`] as a `Future`.
pub fn remove_future(path: &str) -> FsFuture<UnitResult> {
    future_over!(shared, {
        let s = shared.clone();
        remove_async(path, move |r| deliver(&s, r));
    })
}

/// [`list_async`] as a `Future`.
pub fn list_future(dir: &str) -> FsFuture<ListResult> {
    future_over!(shared, {
        let s = shared.clone();
        list_async(dir, move |r| deliver(&s, r));
    })
}

// ---------------------------------------------------------------------------
// Per-target implementations. Native targets share one std::fs backend rooted at the
// per-platform app-data directory; web-dom rides the day-dom shim into OPFS.
// ---------------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
#[path = "native.rs"]
mod imp;

#[cfg(target_arch = "wasm32")]
#[path = "web.rs"]
mod imp;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_rules() {
        for bad in [
            "", "/abs", "a//b", "./x", "../x", "a/../b", "c:win", "a/./b",
        ] {
            assert_eq!(check_path(bad), Err(FsError::BadPath), "{bad:?}");
        }
        for good in ["a", "a/b.txt", "notes/2026/07.md"] {
            assert_eq!(check_path(good), Ok(()), "{good:?}");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn round_trip_list_remove() {
        // Root under a scratch dir so the test never touches the real data dir.
        let scratch = std::env::temp_dir().join(format!("day-part-fs-{}", std::process::id()));
        // SAFETY: tests in this crate run single-threaded over this env var (no other test
        // reads it), and the var is test-scoped.
        unsafe { std::env::set_var("DAY_DATA_DIR", &scratch) };

        write("t/hello.txt", b"hi").unwrap();
        assert_eq!(read("t/hello.txt").unwrap(), b"hi");
        assert_eq!(list("t").unwrap(), vec!["hello.txt".to_string()]);
        assert_eq!(read("t/missing.txt"), Err(FsError::NotFound));
        remove("t/hello.txt").unwrap();
        assert_eq!(remove("t/hello.txt"), Err(FsError::NotFound));
        assert_eq!(list("t").unwrap(), Vec::<String>::new());
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
