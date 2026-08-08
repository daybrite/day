// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-part-location — a HEADLESS cross-platform location API. No UI; any Rust code can depend on
//! this crate and ask the platform's own location service for a fix, once or as a live stream.
//!
//! ```no_run
//! use day_part_location::Accuracy;
//! let watch = day_part_location::watch(Accuracy::Balanced, |fix| match fix {
//!     Ok(f) => println!("{:.5}, {:.5}", f.latitude, f.longitude),
//!     Err(e) => println!("location unavailable: {e}"),
//! });
//! // Updates arrive until `watch` is dropped.
//! ```
//!
//! Platform selection is purely `#[cfg(target_os)]` (location is an OS concern, not a
//! widget-toolkit one): Apple platforms use CoreLocation, Android `LocationManager` through a Java
//! shim staged by `day build`, and the web `navigator.geolocation`. HarmonyOS, desktop Linux and
//! Windows report [`LocationError::Unavailable`] — see the table in docs/location.md, which says
//! why rather than pretending.
//!
//! # Permissions are a separate concern
//!
//! This crate never prompts. A platform denial arrives as [`LocationError::PermissionDenied`], and
//! the app asks for access through `day-part-permissions` (`Permission::Location`) — so neither
//! crate depends on the other, and an app that already has permission pays nothing for the machinery
//! that requests it. Every mobile OS ALSO needs a build-time declaration, which `[permissions]` in
//! Day.toml generates (docs/permissions.md).
//!
//! # Threading
//!
//! Callbacks run on an unspecified thread — the platform's delivery thread natively, the sole
//! browser thread on the web — so deliver into UI state with a `day_reactive::Setter`, exactly as
//! `day-part-http` and `day-part-sensors` document.

use std::sync::{Arc, Mutex, MutexGuard};

/// One position fix. Fields the platform did not report stay `None` — never faked, and never
/// silently zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fix {
    /// Degrees north of the equator (WGS-84).
    pub latitude: f64,
    /// Degrees east of the prime meridian (WGS-84).
    pub longitude: f64,
    /// Metres above the reference surface. Apple reports height above the WGS-84 ellipsoid, Android
    /// above the WGS-84 ellipsoid too, and the browser whatever its provider gives — treat it as
    /// approximate.
    pub altitude: Option<f64>,
    /// Horizontal accuracy radius in metres: the true position is within this distance with ~68%
    /// confidence. `None` where the platform declines to say.
    pub accuracy_m: Option<f64>,
    /// Vertical accuracy in metres, when reported.
    pub vertical_accuracy_m: Option<f64>,
    /// Ground speed in metres per second, when reported.
    pub speed_mps: Option<f64>,
    /// Direction of travel in degrees clockwise from true north, when reported. Meaningless when
    /// stationary, and platforms differ on whether they say so — treat a `Some` at zero speed with
    /// suspicion.
    pub course_deg: Option<f64>,
    /// When the platform timestamped the fix, in milliseconds since the Unix epoch.
    pub timestamp_ms: Option<i64>,
}

/// Why a fix could not be produced.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocationError {
    /// The app has not been granted location access. Ask through `day-part-permissions`
    /// (`Permission::Location`); this crate never prompts.
    PermissionDenied,
    /// Location services are switched off device-wide, or every provider is disabled. Only the user
    /// can change that — from Settings, not from a prompt.
    Disabled,
    /// No fix arrived in time. Indoors and on a cold start this is ordinary; try again.
    Timeout,
    /// This target has no location API Day can reach (docs/location.md).
    Unavailable,
    /// Anything else the platform reported, message passed through.
    Io(String),
}

impl std::fmt::Display for LocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LocationError::PermissionDenied => write!(f, "location permission denied"),
            LocationError::Disabled => write!(f, "location services are off"),
            LocationError::Timeout => write!(f, "no location fix in time"),
            LocationError::Unavailable => write!(f, "no location capability on this platform"),
            LocationError::Io(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for LocationError {}

/// How precise a fix to ask for. Higher accuracy costs battery and takes longer to acquire, so ask
/// for the least you need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accuracy {
    /// City-level, cheapest — network and cell positioning.
    Coarse,
    /// Roughly a city block. The sensible default.
    Balanced,
    /// The best the device can do, GPS included.
    Best,
}

/// Whether this target has a location API Day can reach at all. `false` is a platform statement, not
/// a permission one — a denied permission still answers `true` here.
pub fn is_available() -> bool {
    imp::is_available()
}

/// Ask for a single fix. `on_done` runs on an unspecified thread.
pub fn current(acc: Accuracy, on_done: impl FnOnce(Result<Fix, LocationError>) + Send + 'static) {
    if !is_available() {
        on_done(Err(LocationError::Unavailable));
        return;
    }
    // A one-shot is a watch that stops after the first answer — which is also how CoreLocation and
    // the browser model it, so nothing platform-specific is needed here.
    let holder: Arc<Mutex<Option<Watch>>> = Arc::new(Mutex::new(None));
    let sink = holder.clone();
    let done = std::sync::Mutex::new(Some(on_done));
    let watch = watch(acc, move |fix| {
        let Some(cb) = (match done.lock() {
            Ok(mut g) => g.take(),
            Err(p) => p.into_inner().take(),
        }) else {
            return;
        };
        cb(fix);
        // Dropping the handle inside its own callback would deadlock on the watcher list, so hand
        // it to a short-lived thread instead.
        let holder = sink.clone();
        std::thread::spawn(move || {
            let taken = match holder.lock() {
                Ok(mut g) => g.take(),
                Err(p) => p.into_inner().take(),
            };
            drop(taken);
        });
    });
    *lock(&holder) = Some(watch);
}

/// [`current`] as a `Future`. Plain oneshot plumbing over the same completion — any executor can
/// await it, including a test's `block_on`.
pub fn current_future(acc: Accuracy) -> FixFuture {
    let shared = Arc::new(Mutex::new(FutureState::default()));
    let sink = shared.clone();
    current(acc, move |r| {
        let waker = {
            let mut st = lock(&sink);
            st.result = Some(r);
            st.waker.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
    });
    FixFuture {
        shared,
        done: false,
    }
}

/// Subscribe to position updates until the returned [`Watch`] is dropped.
///
/// In a Day app, bind the handle to the page's scope so the subscription ends with it:
///
/// ```ignore
/// let watch = day_part_location::watch(Accuracy::Balanced, move |f| set.set(f));
/// day_reactive::Scope::current().on_cleanup(move || drop(watch));
/// ```
pub fn watch(
    acc: Accuracy,
    on_fix: impl FnMut(Result<Fix, LocationError>) + Send + 'static,
) -> Watch {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    if !is_available() {
        // Still hand back a handle so callers need no special case; report once, then stay quiet.
        let mut cb = on_fix;
        cb(Err(LocationError::Unavailable));
        return Watch { id: 0 };
    }
    let start = {
        let mut w = lock(watchers());
        w.push((id, Box::new(on_fix)));
        w.len() == 1
    };
    if start {
        imp::start(acc);
    }
    Watch { id }
}

/// An active subscription. Dropping it stops delivery, and stops the platform's updates once the
/// last watcher is gone — which is what keeps the GPS from staying warm behind a closed page.
pub struct Watch {
    id: u64,
}

impl Drop for Watch {
    fn drop(&mut self) {
        if self.id == 0 {
            return;
        }
        let stop = {
            let mut w = lock(watchers());
            w.retain(|(i, _)| *i != self.id);
            w.is_empty()
        };
        if stop {
            imp::stop();
        }
    }
}

type Sink = Box<dyn FnMut(Result<Fix, LocationError>) + Send>;

fn watchers() -> &'static Mutex<Vec<(u64, Sink)>> {
    static WATCHERS: std::sync::OnceLock<Mutex<Vec<(u64, Sink)>>> = std::sync::OnceLock::new();
    WATCHERS.get_or_init(|| Mutex::new(Vec::new()))
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Deliver a platform update to every watcher. The per-OS arms call this.
///
/// `allow(dead_code)`: the targets with no location API compile the catch-all arm, which never
/// delivers — so this looks unused there while being the whole delivery path everywhere else.
#[allow(dead_code)]
pub(crate) fn deliver(update: Result<Fix, LocationError>) {
    let mut w = lock(watchers());
    for (_, cb) in w.iter_mut() {
        cb(update.clone());
    }
}

#[derive(Default)]
struct FutureState {
    result: Option<Result<Fix, LocationError>>,
    waker: Option<std::task::Waker>,
}

/// A pending [`current_future`].
pub struct FixFuture {
    shared: Arc<Mutex<FutureState>>,
    done: bool,
}

impl std::future::Future for FixFuture {
    type Output = Result<Fix, LocationError>;

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

impl Drop for FixFuture {
    fn drop(&mut self) {
        if !self.done {
            lock(&self.shared).waker = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Per-OS implementations. Each exposes `is_available()`, `start(Accuracy)` and `stop()`, and calls
// [`deliver`] as the platform reports.
// ---------------------------------------------------------------------------

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[path = "apple.rs"]
mod imp;

#[cfg(target_os = "android")]
#[path = "android.rs"]
mod imp;

#[cfg(target_arch = "wasm32")]
#[path = "web.rs"]
mod imp;

// Everything else — including HarmonyOS, desktop Linux and Windows — has no reachable location API
// yet. Answering honestly beats a stub that looks like an oversight (docs/location.md).
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "android",
    target_arch = "wasm32"
)))]
mod imp {
    pub fn is_available() -> bool {
        false
    }
    pub fn start(_acc: super::Accuracy) {}
    pub fn stop() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Must never panic on any host, with or without a location service.
    #[test]
    fn probing_never_panics() {
        let _ = is_available();
    }

    /// A target with no location API still has to answer — a watcher that is silently never called
    /// would hang an app waiting for its first fix.
    #[test]
    fn unavailable_targets_report_once() {
        if is_available() {
            return; // this host has CoreLocation; see the macOS-gated test below
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let watch = watch(Accuracy::Coarse, move |r| lock(&sink).push(r));
        drop(watch);
        let got = lock(&seen);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], Err(LocationError::Unavailable));
    }

    /// On a mac CoreLocation exists, so availability is true even when the app is unauthorized —
    /// `is_available` is a platform statement, not a permission one.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_has_a_location_api() {
        assert!(is_available());
    }
}
