// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Event-driven motion sensors. [`watch`] delivers new readings until its [`Watch`] is dropped.
//! iOS, Android, HarmonyOS, Windows and web use native callbacks; desktop Linux and macOS report
//! unavailable. Native delivery runs on a shared background dispatcher, web delivery on the browser
//! thread. Pass readings to UI state through a `day_reactive::Setter`.
//!
//! Sources request approximately 20 Hz where supported; actual rates depend on the device.
//! Pending native readings are coalesced to one per sensor when callbacks are slow. This API is for
//! live state, not lossless recording. It never repeats a cached reading on a timer.
//!
//! ```no_run
//! use day_part_sensors::SensorKind;
//! let watch = day_part_sensors::watch(SensorKind::Accelerometer, |r| {
//!     println!("{} {} {} m/s²", r.x, r.y, r.z);
//! });
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

/// Motion measurement to subscribe to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SensorKind {
    /// Acceleration including gravity, in m/s².
    Accelerometer,
    /// Angular velocity, in rad/s.
    Gyroscope,
    /// Magnetic field, in µT. iOS removes device bias through CoreMotion device motion;
    /// in the default mode samples arrive only after calibration. Nearby magnetic interference still affects them.
    Magnetometer,
}
impl SensorKind {
    /// Select the platform's bias-corrected or uncalibrated source explicitly.
    pub const fn with_calibration(self, calibration: Calibration) -> Sensor {
        Sensor {
            kind: self,
            calibration,
        }
    }
    const fn index(self) -> usize {
        match self {
            Self::Accelerometer => 0,
            Self::Gyroscope => 1,
            Self::Magnetometer => 2,
        }
    }
}

/// Sensor bias processing. Availability is queried for each mode independently.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Calibration {
    /// The normal platform sensor, with platform bias correction where provided.
    /// On iOS, gyroscope and magnetometer use CoreMotion's processed device motion.
    #[default]
    Calibrated,
    /// Omit runtime bias correction. Factory calibration and temperature compensation
    /// may still apply. Unsupported modes never silently fall back to another source.
    Uncalibrated,
}

/// A sensor kind and its processing mode. Pass this to [`watch`] and [`is_available`].
/// Passing a [`SensorKind`] alone selects [`Calibration::Calibrated`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Sensor {
    pub kind: SensorKind,
    pub calibration: Calibration,
}
impl From<SensorKind> for Sensor {
    fn from(kind: SensorKind) -> Self {
        kind.with_calibration(Calibration::Calibrated)
    }
}
impl Sensor {
    fn index(self) -> usize {
        self.kind.index()
            + if self.calibration == Calibration::Uncalibrated {
                3
            } else {
                0
            }
    }
    #[cfg(any(
        test,
        target_arch = "wasm32",
        target_os = "android",
        target_env = "ohos"
    ))]
    fn from_code(code: u32) -> Option<Self> {
        SOURCES.get(code as usize).copied()
    }
}
const SOURCES: [Sensor; 6] = [
    SensorKind::Accelerometer.with_calibration(Calibration::Calibrated),
    SensorKind::Gyroscope.with_calibration(Calibration::Calibrated),
    SensorKind::Magnetometer.with_calibration(Calibration::Calibrated),
    SensorKind::Accelerometer.with_calibration(Calibration::Uncalibrated),
    SensorKind::Gyroscope.with_calibration(Calibration::Uncalibrated),
    SensorKind::Magnetometer.with_calibration(Calibration::Uncalibrated),
];

/// A new measurement in SI units. Axis signs follow the platform's coordinate system.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SensorReading {
    /// Device x axis.
    pub x: f64,
    /// Device y axis.
    pub y: f64,
    /// Device z axis.
    pub z: f64,
}

/// Whether this platform exposes the sensor. Web checks API support, not hardware presence;
/// permission and hardware still determine whether events arrive. This never starts a sensor.
pub fn is_available(sensor: impl Into<Sensor>) -> bool {
    imp::is_available(sensor.into())
}

/// Watch new sensor readings. Callbacks are serialized and must return promptly. No samples are
/// produced when the source is unavailable or permission is denied; request permission first.
/// Recreate the watch after a native subscription failure. Native callbacks run on a background
/// dispatcher; web callbacks run on the browser thread. A callback may create or drop watches.
///
/// Bind the handle to a Day page with `Scope::current().on_cleanup(move || drop(watch))`.
pub fn watch(
    sensor: impl Into<Sensor>,
    on_sample: impl FnMut(SensorReading) + Send + 'static,
) -> Watch {
    let kind = sensor.into();
    let (watch, start) = hub().add(kind, Box::new(on_sample));
    #[cfg(not(target_arch = "wasm32"))]
    if start {
        let hub = hub().clone();
        std::thread::Builder::new()
            .name("day-sensors".into())
            .spawn(move || hub.run(imp::start))
            .expect("start sensor dispatcher");
    }
    #[cfg(target_arch = "wasm32")]
    if start {
        imp::start(kind, watch.generation);
    }
    watch
}

/// A subscription. Drop cancels queued delivery immediately. A callback already selected for delivery may
/// finish. Native teardown runs on the dispatcher, after that callback, so self-unsubscription
/// cannot deadlock. The last watcher releases the native sensor source.
pub struct Watch {
    hub: Arc<Hub>,
    kind: Sensor,
    listener: Arc<Listener>,
    #[cfg(target_arch = "wasm32")]
    generation: u32,
}
impl Drop for Watch {
    fn drop(&mut self) {
        self.listener.active.store(false, Ordering::Release);
        let mut state = lock(&self.hub.state);
        let feed = &mut state.feeds[self.kind.index()];
        feed.listeners.retain(|l| !Arc::ptr_eq(l, &self.listener));
        if feed.listeners.is_empty() {
            feed.pending = None;
        }
        #[cfg(target_arch = "wasm32")]
        let stop = feed.listeners.is_empty();
        drop(state);
        self.hub.wake();
        #[cfg(target_arch = "wasm32")]
        if stop {
            imp::stop(self.kind, self.generation);
        }
    }
}

type Handler = Box<dyn FnMut(SensorReading) + Send>;
struct Listener {
    active: AtomicBool,
    delivery: Mutex<Delivery>,
}
struct Delivery {
    handler: Option<Handler>,
    pending: Option<SensorReading>,
}
impl Listener {
    fn deliver(&self, reading: SensorReading) {
        let mut delivery = lock(&self.delivery);
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        let Some(mut handler) = delivery.handler.take() else {
            // A reentrant browser event replaces the single pending sample.
            delivery.pending = Some(reading);
            return;
        };
        drop(delivery);
        let mut next = Some(reading);
        while let Some(reading) = next {
            if self.active.load(Ordering::Acquire)
                && std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(reading)))
                    .is_err()
            {
                self.active.store(false, Ordering::Release);
            }
            let mut delivery = lock(&self.delivery);
            next = delivery.pending.take();
            if next.is_none() {
                delivery.handler = Some(handler);
                return;
            }
        }
    }
}
#[derive(Default)]
struct Feed {
    generation: u32,
    listeners: Vec<Arc<Listener>>,
    pending: Option<SensorReading>,
}
#[derive(Default)]
struct State {
    feeds: [Feed; 6],
    #[cfg(not(target_arch = "wasm32"))]
    running: bool,
}
#[derive(Default)]
struct Hub {
    state: Mutex<State>,
    #[cfg(not(target_arch = "wasm32"))]
    ready: std::sync::Condvar,
}
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
fn hub() -> &'static Arc<Hub> {
    static HUB: OnceLock<Arc<Hub>> = OnceLock::new();
    HUB.get_or_init(Default::default)
}
impl Hub {
    fn add(self: &Arc<Self>, kind: Sensor, handler: Handler) -> (Watch, bool) {
        let listener = Arc::new(Listener {
            active: AtomicBool::new(true),
            delivery: Mutex::new(Delivery {
                handler: Some(handler),
                pending: None,
            }),
        });
        let mut state = lock(&self.state);
        let feed = &mut state.feeds[kind.index()];
        let first = feed.listeners.is_empty();
        if first {
            feed.generation = feed
                .generation
                .checked_add(1)
                .expect("sensor generation exhausted");
        }
        #[cfg(target_arch = "wasm32")]
        let generation = feed.generation;
        feed.listeners.push(listener.clone());
        #[cfg(not(target_arch = "wasm32"))]
        let start = {
            let start = !state.running;
            state.running = true;
            start
        };
        #[cfg(target_arch = "wasm32")]
        let start = first;
        drop(state);
        self.wake();
        (
            Watch {
                hub: self.clone(),
                kind,
                listener,
                #[cfg(target_arch = "wasm32")]
                generation,
            },
            start,
        )
    }
    fn wake(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        self.ready.notify_one();
    }
    #[cfg(any(
        test,
        target_os = "ios",
        target_os = "android",
        target_os = "windows",
        target_env = "ohos",
        target_arch = "wasm32"
    ))]
    fn push(&self, kind: Sensor, generation: u32, reading: SensorReading) {
        if !reading.x.is_finite() || !reading.y.is_finite() || !reading.z.is_finite() {
            return;
        }
        let mut state = lock(&self.state);
        let feed = &mut state.feeds[kind.index()];
        if feed.generation != generation || feed.listeners.is_empty() {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            feed.pending = Some(reading);
            drop(state);
            self.wake();
        }
        #[cfg(target_arch = "wasm32")]
        {
            let listeners = feed.listeners.clone();
            drop(state);
            for listener in listeners {
                listener.deliver(reading);
            }
        }
    }

    // Start/stop and user callbacks share a worker. It sleeps until an event or subscription
    // change, never polls, and exits after every source has been released.
    #[cfg(not(target_arch = "wasm32"))]
    fn run<S>(&self, mut start: impl FnMut(Sensor, u32) -> Option<S>) {
        let mut sources: [Option<(u32, Option<S>)>; 6] = std::array::from_fn(|_| None);
        'drive: loop {
            let mut state = lock(&self.state);
            for (i, kind) in SOURCES.into_iter().enumerate() {
                let feed = &mut state.feeds[i];
                let desired = (!feed.listeners.is_empty()).then_some(feed.generation);
                if sources[i].as_ref().map(|s| s.0) != desired {
                    if let Some(source) = sources[i].take() {
                        drop(state);
                        drop(source);
                        continue 'drive;
                    }
                    if let Some(generation) = desired {
                        drop(state);
                        sources[i] = Some((generation, start(kind, generation)));
                        continue 'drive;
                    }
                }
            }
            // Take one sample per sensor so a busy source cannot starve the other kinds.
            let batch: Vec<_> = state
                .feeds
                .iter_mut()
                .filter_map(|feed| {
                    feed.pending
                        .take()
                        .map(|reading| (feed.listeners.clone(), reading))
                })
                .collect();
            if !batch.is_empty() {
                drop(state);
                for (listeners, reading) in batch {
                    for listener in listeners {
                        listener.deliver(reading);
                    }
                }
                continue;
            }
            if state.feeds.iter().all(|feed| feed.listeners.is_empty()) {
                state.running = false;
                return;
            }
            drop(self.ready.wait(state).unwrap_or_else(|e| e.into_inner()));
        }
    }
}

// Platform callbacks only enqueue bounded data. No user code runs on a native FFI stack.
#[cfg(any(
    target_os = "ios",
    target_os = "android",
    target_os = "windows",
    target_env = "ohos",
    target_arch = "wasm32"
))]
fn push(kind: Sensor, generation: u32, reading: SensorReading) {
    hub().push(kind, generation, reading);
}

#[cfg(target_os = "ios")]
#[path = "ios.rs"]
mod imp;
#[cfg(target_os = "android")]
#[path = "android.rs"]
mod imp;
#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;
#[cfg(target_env = "ohos")]
#[path = "ohos.rs"]
mod imp;
#[cfg(target_arch = "wasm32")]
#[path = "web.rs"]
mod imp;
#[cfg(not(any(
    target_os = "ios",
    target_os = "android",
    target_os = "windows",
    target_env = "ohos",
    target_arch = "wasm32"
)))]
mod imp {
    pub fn is_available(_: super::Sensor) -> bool {
        false
    }
    pub fn start(_: super::Sensor, _: u32) -> Option<()> {
        None
    }
}

#[cfg(test)]
mod tests;
