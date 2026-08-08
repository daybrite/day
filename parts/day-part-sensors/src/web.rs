// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The browser (`web-dom`): `DeviceMotionEvent` through the day-dom shim's `day_dom_sensor_*`
//! imports (`crates/day-cli/resources/web/shim.js`).
//!
//! One `devicemotion` listener feeds both supported kinds, because the event carries acceleration
//! and rotation together:
//!
//! | kind | source | conversion |
//! |---|---|---|
//! | Accelerometer | `accelerationIncludingGravity` (m/s²) | none — day's contract already says "including gravity" |
//! | Gyroscope | `rotationRate` (deg/s) | × π/180; beta→x, gamma→y, alpha→z |
//! | Magnetometer | — | no cross-browser API exists, so always unavailable |
//!
//! Two browser realities the shim handles and this file documents:
//!
//! - **A secure context is required.** `devicemotion` fires only over HTTPS or on localhost — both
//!   `day launch`'s server and the hosted showcase qualify — and `Permissions-Policy` defaults
//!   `accelerometer`/`gyroscope` to `self`, so a cross-origin iframe embed needs delegation.
//! - **iOS Safari needs a user gesture**: `DeviceMotionEvent.requestPermission()` must be called
//!   from a user activation, which is `day-part-permissions`' `Permission::Motion` on the web —
//!   ask for it from inside a button's action, where the gesture is still live.
//!
//! Using this crate on wasm outside a day-dom host page fails at instantiation (the imports are
//! unresolved) — the same contract as `day-part-prefs` and `day-part-http` (docs/web.md).

use super::{SensorKind, SensorReading};

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    /// Attach the `devicemotion` listener (idempotent, and a no-op for the magnetometer).
    fn day_dom_sensor_start(kind: u32);
    /// Writes three `f64`s (x, y, z) at `out`; returns 1 when a sample was written.
    fn day_dom_sensor_read(kind: u32, out: *mut f64) -> u32;
    fn day_dom_sensor_available(kind: u32) -> u32;
    /// Start a timer that calls the exported `day_sensors_tick(kind)` every `ms`.
    fn day_dom_sensor_feed(kind: u32, ms: u32);
    fn day_dom_sensor_unfeed(kind: u32);
}

/// Drive this sensor's feed from the browser's timer — wasm has no threads to sample on.
pub fn start_feed(kind: SensorKind, ms: u64) {
    unsafe {
        day_dom_sensor_start(code(kind));
        day_dom_sensor_feed(code(kind), ms as u32);
    }
}

pub fn stop_feed(kind: SensorKind) {
    unsafe { day_dom_sensor_unfeed(code(kind)) };
}

/// The shim's kind codes, shared with `day_dom_sensor_*`: 0 accelerometer, 1 gyroscope,
/// 2 magnetometer.
fn code(kind: SensorKind) -> u32 {
    match kind {
        SensorKind::Accelerometer => 0,
        SensorKind::Gyroscope => 1,
        SensorKind::Magnetometer => 2,
    }
}

pub fn is_available(kind: SensorKind) -> bool {
    // Starting here as well as in `sample` matters: an app that asks before subscribing would
    // otherwise begin the grace period only once it subscribed, and a page that never subscribes
    // would report "available" forever.
    unsafe {
        day_dom_sensor_start(code(kind));
        day_dom_sensor_available(code(kind)) != 0
    }
}

pub fn sample(kind: SensorKind) -> Option<SensorReading> {
    let mut xyz = [0.0f64; 3];
    // SAFETY: the shim writes exactly three f64s at this pointer, and only when it returns 1.
    let ok = unsafe {
        day_dom_sensor_start(code(kind));
        day_dom_sensor_read(code(kind), xyz.as_mut_ptr()) != 0
    };
    ok.then_some(SensorReading {
        x: xyz[0],
        y: xyz[1],
        z: xyz[2],
    })
}
