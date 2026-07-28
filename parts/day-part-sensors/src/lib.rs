//! day-part-sensors — a HEADLESS cross-platform motion-sensor API. No UI; any Rust code can
//! depend on this crate and [`watch`] the device's motion sensors through the platform's NATIVE API.
//!
//! ```no_run
//! use day_part_sensors::SensorKind;
//! let watch = day_part_sensors::watch(SensorKind::Accelerometer, |a| {
//!     println!("acceleration: {:.2} {:.2} {:.2} m/s²", a.x, a.y, a.z);
//! });
//! // Samples arrive until `watch` is dropped.
//! ```
//!
//! Platform selection is purely `#[cfg(target_os)]`/`#[cfg(target_env)]` (a sensor is an OS concern,
//! not a widget-toolkit one): iOS uses CoreMotion, Android `SensorManager` (via a Java shim staged by
//! `day build`), HarmonyOS the native `libohsensor.so`, Linux the Industrial I/O sysfs tree, and the
//! web `DeviceMotionEvent`. macOS has no public motion-sensor API and Windows is a stub for now —
//! both report no sensors at all.
//!
//! # Why a stream
//!
//! Every platform's sensor API is already PUSH — `SensorEventListener`, CoreMotion handlers,
//! `OH_Sensor_Subscribe`, `devicemotion` — so the older `read()` poll was an adapter in the wrong
//! direction: each per-OS arm had to cache the newest event purely so a caller could ask for it.
//! [`watch`] removes that inversion, and "no sample yet" stops being a poll artifact.
//!
//! Delivery rate: the arms that cache a natively-pushed event are sampled at [`SAMPLE_MS`]; the
//! pull-only arms (Linux sysfs, Windows) are read at the same cadence. A `watch` therefore delivers
//! at a steady ~20 Hz rather than at the sensor's own rate — plenty for a readout or a chart, and it
//! keeps a fast sensor from flooding an app's UI thread.
//!
//! `on_sample` runs on an unspecified BACKGROUND thread (never the UI thread), so deliver into UI
//! state with a `day_reactive::Setter`, exactly as `day-part-http` documents for its completions.

/// Which motion sensor to query.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SensorKind {
    /// Acceleration (including gravity) along the device's x/y/z axes, in m/s².
    Accelerometer,
    /// Rotation rate around the device's x/y/z axes, in rad/s.
    Gyroscope,
    /// Ambient magnetic field along the device's x/y/z axes, in µT.
    Magnetometer,
}

/// One motion-sensor sample. Units are SI and depend on the [`SensorKind`]: m/s² for the
/// accelerometer, rad/s for the gyroscope, µT for the magnetometer (iOS g's and any platform quirks
/// are normalized by the per-OS impls). Axis sign conventions are the platform's own — e.g. a device
/// lying face-up reads `z ≈ +9.8` on Android but `z ≈ -9.8` on iOS.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SensorReading {
    /// Value along the device's x axis.
    pub x: f64,
    /// Value along the device's y axis.
    pub y: f64,
    /// Value along the device's z axis.
    pub z: f64,
}

/// How often a [`watch`] delivers, in milliseconds.
pub const SAMPLE_MS: u64 = 50;

/// Whether the device has the given sensor (and the platform an API for it). `false` also covers
/// simulators/emulators without sensor passthrough and desktops without motion hardware.
pub fn is_available(kind: SensorKind) -> bool {
    imp::is_available(kind)
}

/// Subscribe to a sensor. Samples arrive on an unspecified BACKGROUND thread roughly every
/// [`SAMPLE_MS`] until the returned [`Watch`] is dropped.
///
/// Nothing is delivered before the platform's first event, so a device that never reports (no such
/// sensor, an emulator without passthrough) simply produces no samples — ask [`is_available`] to
/// tell that apart from "not yet".
///
/// In a Day app, bind the handle to the page's scope so the subscription ends with it:
///
/// ```ignore
/// let watch = day_part_sensors::watch(SensorKind::Accelerometer, move |r| latest.set(r));
/// day_reactive::Scope::current().on_cleanup(move || drop(watch));
/// ```
pub fn watch(kind: SensorKind, on_sample: impl FnMut(SensorReading) + Send + 'static) -> Watch {
    subscribe(kind, Box::new(on_sample))
}

/// An active subscription. Dropping it stops delivery, and stops the underlying platform stream once
/// the last watcher of that sensor is gone.
pub struct Watch {
    kind: SensorKind,
    id: u64,
}

impl Drop for Watch {
    fn drop(&mut self) {
        unsubscribe(self.kind, self.id);
    }
}

type Handler = Box<dyn FnMut(SensorReading) + Send>;

/// Per-sensor state: the live watchers, and the flag their sampling thread checks.
#[derive(Default)]
struct Feed {
    watchers: Vec<(u64, Handler)>,
    running: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

fn feeds() -> &'static std::sync::Mutex<std::collections::HashMap<u8, Feed>> {
    static FEEDS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<u8, Feed>>> =
        std::sync::OnceLock::new();
    FEEDS.get_or_init(Default::default)
}

fn lock_feeds() -> std::sync::MutexGuard<'static, std::collections::HashMap<u8, Feed>> {
    match feeds().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn key(kind: SensorKind) -> u8 {
    match kind {
        SensorKind::Accelerometer => 0,
        SensorKind::Gyroscope => 1,
        SensorKind::Magnetometer => 2,
    }
}

fn subscribe(kind: SensorKind, handler: Handler) -> Watch {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let start = {
        let mut feeds = lock_feeds();
        let feed = feeds.entry(key(kind)).or_default();
        feed.watchers.push((id, handler));
        if feed.running.is_some() {
            None
        } else {
            let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
            feed.running = Some(flag.clone());
            Some(flag)
        }
    };

    // One feed per WATCHED SENSOR, not per watcher: the platform arm keeps a single native
    // subscription either way, so a second watcher costs nothing but a callback.
    if let Some(flag) = start {
        start_feed(kind, flag);
    }
    Watch { kind, id }
}

/// Push the platform's newest reading to everyone watching `kind`.
///
/// The two feed drivers — a thread natively, the browser's timer on wasm — deliver through this one
/// path, so the fan-out logic exists once.
pub(crate) fn deliver(kind: SensorKind) {
    let Some(reading) = imp::sample(kind) else {
        return;
    };
    // Hold the lock only to run the callbacks; a handler that blocks delays its own sensor's feed
    // and nothing else.
    let mut feeds = lock_feeds();
    if let Some(feed) = feeds.get_mut(&key(kind)) {
        for (_, h) in feed.watchers.iter_mut() {
            h(reading);
        }
    }
}

/// Native platforms: one sampling thread per watched sensor.
#[cfg(not(target_arch = "wasm32"))]
fn start_feed(kind: SensorKind, running: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    std::thread::spawn(move || {
        while running.load(std::sync::atomic::Ordering::Relaxed) {
            deliver(kind);
            std::thread::sleep(std::time::Duration::from_millis(SAMPLE_MS));
        }
    });
}

/// The browser has ONE thread — `std::thread::spawn` PANICS on wasm32 — so the feed is driven by a
/// timer inside the day-dom shim, which calls back into [`day_sensors_tick`].
#[cfg(target_arch = "wasm32")]
fn start_feed(kind: SensorKind, _running: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    imp::start_feed(kind, SAMPLE_MS);
}

#[cfg(not(target_arch = "wasm32"))]
fn stop_feed(_kind: SensorKind) {}

#[cfg(target_arch = "wasm32")]
fn stop_feed(kind: SensorKind) {
    imp::stop_feed(kind);
}

/// The day-dom shim's timer tick (wasm only): deliver one sample for `kind`.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn day_sensors_tick(kind: u32) {
    let kind = match kind {
        0 => SensorKind::Accelerometer,
        1 => SensorKind::Gyroscope,
        _ => SensorKind::Magnetometer,
    };
    deliver(kind);
}

fn unsubscribe(kind: SensorKind, id: u64) {
    let mut feeds = lock_feeds();
    let Some(feed) = feeds.get_mut(&key(kind)) else {
        return;
    };
    feed.watchers.retain(|(w, _)| *w != id);
    if feed.watchers.is_empty()
        && let Some(flag) = feed.running.take()
    {
        // The thread notices on its next tick and exits; the platform arm keeps its native
        // subscription for the process lifetime, as it always has.
        flag.store(false, std::sync::atomic::Ordering::Relaxed);
        stop_feed(kind);
    }
}

// ---------------------------------------------------------------------------
// Per-OS implementations. Each exposes `fn is_available(SensorKind) -> bool` and
// `fn sample(SensorKind) -> Option<SensorReading>` — the newest reading the platform has, which
// the subscription loop above turns into a stream.
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;

#[cfg(target_os = "ios")]
#[path = "ios.rs"]
mod imp;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;

// Desktop/embedded Linux reads iio sysfs; HarmonyOS (also `target_os = "linux"`) sandboxes that
// away, so it uses its own native sensor API instead.
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
#[path = "linux.rs"]
mod imp;

#[cfg(all(target_os = "linux", target_env = "ohos"))]
#[path = "ohos.rs"]
mod imp;

#[cfg(target_os = "android")]
#[path = "android.rs"]
mod imp;

// The browser: `DeviceMotionEvent` through the day-dom shim (docs/web.md).
#[cfg(target_arch = "wasm32")]
#[path = "web.rs"]
mod imp;

// Any other platform: no native sensor API.
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "android",
    target_arch = "wasm32"
)))]
mod imp {
    pub fn is_available(_kind: super::SensorKind) -> bool {
        false
    }
    pub fn sample(_kind: super::SensorKind) -> Option<super::SensorReading> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The subscription registry is process-global, so the tests that inspect or perturb it must
    /// not run concurrently — cargo runs them on parallel threads by default.
    static REGISTRY: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn registry_guard() -> std::sync::MutexGuard<'static, ()> {
        match REGISTRY.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    const KINDS: [SensorKind; 3] = [
        SensorKind::Accelerometer,
        SensorKind::Gyroscope,
        SensorKind::Magnetometer,
    ];

    // Querying must never panic, whether or not the host has sensors (dev machines and CI runners
    // typically don't — the mac host always answers false/no samples).
    #[test]
    fn probing_never_panics() {
        for kind in KINDS {
            let _ = is_available(kind);
            if let Some(r) = imp::sample(kind) {
                assert!(r.x.is_finite() && r.y.is_finite() && r.z.is_finite());
            }
        }
    }

    /// Subscribing and dropping must work on a host with no sensors at all: the thread starts,
    /// finds nothing to deliver, and exits when the last watcher goes. Any sample that DOES arrive
    /// must be finite.
    #[test]
    fn watch_starts_and_stops_cleanly() {
        let _guard = registry_guard();
        for kind in KINDS {
            let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let sink = seen.clone();
            let watch = watch(kind, move |r| {
                if let Ok(mut v) = sink.lock() {
                    v.push(r);
                }
            });
            std::thread::sleep(std::time::Duration::from_millis(SAMPLE_MS * 2));
            drop(watch);
            for r in seen.lock().expect("samples").iter() {
                assert!(r.x.is_finite() && r.y.is_finite() && r.z.is_finite());
            }
        }
    }

    /// Two watchers on one sensor share a single feed, and dropping one leaves the other running.
    #[test]
    fn watchers_are_independent() {
        let _guard = registry_guard();
        let a = watch(SensorKind::Accelerometer, |_| {});
        let b = watch(SensorKind::Accelerometer, |_| {});
        assert_eq!(
            lock_feeds()
                .get(&key(SensorKind::Accelerometer))
                .map(|f| f.watchers.len()),
            Some(2)
        );
        drop(a);
        let feeds = lock_feeds();
        let feed = feeds.get(&key(SensorKind::Accelerometer)).expect("feed");
        assert_eq!(feed.watchers.len(), 1);
        assert!(
            feed.running.is_some(),
            "the feed must survive one watcher leaving"
        );
        drop(feeds);
        drop(b);
        assert!(
            lock_feeds()
                .get(&key(SensorKind::Accelerometer))
                .is_none_or(|f| f.running.is_none()),
            "the last watcher leaving must stop the feed"
        );
    }
}
