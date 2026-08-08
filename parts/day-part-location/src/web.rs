// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The browser (`web-dom`): `navigator.geolocation.watchPosition` through the day-dom shim's
//! `day_dom_geo_*` imports (`crates/day-cli/resources/web/shim.js`).
//!
//! The cleanest arm of the three: the browser's API is already a subscription with an error channel,
//! so it maps onto this crate's shape almost exactly. The shim pushes each position into the
//! exported [`day_location_fix`] / [`day_location_error`].
//!
//! Two browser realities:
//!
//! - **A secure context is required** — geolocation is refused outside HTTPS/localhost, and reports
//!   as [`LocationError::PermissionDenied`], which is what the browser itself calls it.
//! - **The first `watchPosition` call IS the permission prompt.** There is no separate request, so
//!   on the web `day-part-permissions`' `Permission::Location` and this crate reach the same dialog;
//!   whichever runs first shows it.
//!
//! Using this crate on wasm outside a day-dom host page fails at instantiation (the imports are
//! unresolved) — the same contract as the tree's other web arms (docs/web.md).

use crate::{Accuracy, Fix, LocationError};

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    /// Start `watchPosition`; `high` selects `enableHighAccuracy`.
    fn day_dom_geo_start(high: u32);
    fn day_dom_geo_stop();
    fn day_dom_geo_available() -> u32;
}

pub fn is_available() -> bool {
    unsafe { day_dom_geo_available() != 0 }
}

pub fn start(acc: Accuracy) {
    let high = u32::from(acc == Accuracy::Best);
    unsafe { day_dom_geo_start(high) };
}

pub fn stop() {
    unsafe { day_dom_geo_stop() };
}

/// The shim's position callback. A field the browser did not measure arrives as NaN, which is how
/// `null` crosses the C ABI without a second parameter per field.
#[unsafe(no_mangle)]
pub extern "C" fn day_location_fix(
    latitude: f64,
    longitude: f64,
    altitude: f64,
    accuracy_m: f64,
    vertical_accuracy_m: f64,
    speed_mps: f64,
    course_deg: f64,
    timestamp_ms: f64,
) {
    fn some(v: f64) -> Option<f64> {
        v.is_finite().then_some(v)
    }
    crate::deliver(Ok(Fix {
        latitude,
        longitude,
        altitude: some(altitude),
        accuracy_m: some(accuracy_m),
        vertical_accuracy_m: some(vertical_accuracy_m),
        speed_mps: some(speed_mps),
        course_deg: some(course_deg),
        timestamp_ms: timestamp_ms.is_finite().then_some(timestamp_ms as i64),
    }));
}

/// The shim's error callback, carrying `GeolocationPositionError.code`.
#[unsafe(no_mangle)]
pub extern "C" fn day_location_error(code: u32) {
    crate::deliver(Err(match code {
        1 => LocationError::PermissionDenied, // PERMISSION_DENIED
        2 => LocationError::Disabled,         // POSITION_UNAVAILABLE
        3 => LocationError::Timeout,          // TIMEOUT
        _ => LocationError::Io(format!("geolocation error {code}")),
    }));
}
