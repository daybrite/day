//! day-part-sensors — a HEADLESS cross-platform motion-sensor API. No UI; any Rust code can depend
//! on this crate and call [`read`] to sample the device's motion sensors through the platform's
//! NATIVE API.
//!
//! ```no_run
//! use day_part_sensors::SensorKind;
//! if let Some(a) = day_part_sensors::read(SensorKind::Accelerometer) {
//!     println!("acceleration: {:.2} {:.2} {:.2} m/s²", a.x, a.y, a.z);
//! }
//! ```
//!
//! Platform selection is purely `#[cfg(target_os)]`/`#[cfg(target_env)]` (a sensor is an OS concern,
//! not a widget-toolkit one): iOS uses CoreMotion, Android `SensorManager` (via a Java shim staged by
//! `day build`), HarmonyOS the native `libohsensor.so`, and Linux the Industrial I/O sysfs tree.
//! macOS has no public motion-sensor API and Windows is a stub for now — both always return `None`.
//!
//! The API is a poll: [`read`] returns the LATEST sample. On the push-model platforms (Android,
//! HarmonyOS) the first call lazily subscribes a listener that caches the newest event, so the very
//! first `read` after startup may return `None` until the first event lands — poll again shortly.
//! iOS behaves the same way (`startUpdates` + poll the data property).

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

/// Whether the device has the given sensor (and the platform an API for it). `false` also covers
/// simulators/emulators without sensor passthrough and desktops without motion hardware.
pub fn is_available(kind: SensorKind) -> bool {
    imp::is_available(kind)
}

/// The latest sample from the given sensor via the platform's native API. The first call lazily
/// starts the underlying updates/subscription (kept alive for the process); `None` means the sensor
/// is unavailable on this platform/device — or no sample has arrived yet (poll again).
pub fn read(kind: SensorKind) -> Option<SensorReading> {
    imp::read(kind)
}

// ---------------------------------------------------------------------------
// Per-OS implementations. Each exposes `fn is_available(SensorKind) -> bool` and
// `fn read(SensorKind) -> Option<SensorReading>`.
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

// Any other platform: no native sensor API.
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "android"
)))]
mod imp {
    pub fn is_available(_kind: super::SensorKind) -> bool {
        false
    }
    pub fn read(_kind: super::SensorKind) -> Option<super::SensorReading> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Querying must never panic, whether or not the host has sensors (dev machines and CI runners
    // typically don't — the mac host always answers false/None).
    #[test]
    fn read_does_not_panic() {
        for kind in [
            SensorKind::Accelerometer,
            SensorKind::Gyroscope,
            SensorKind::Magnetometer,
        ] {
            let _ = is_available(kind);
            if let Some(r) = read(kind) {
                assert!(r.x.is_finite() && r.y.is_finite() && r.z.is_finite());
            }
        }
    }
}
