// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Android: `LocationManager.requestLocationUpdates` through this crate's OWN Java shim
//! (`android/java/…/DayLocation.java`), staged into the app's Gradle build by `day build`.
//!
//! Deliberately NOT `FusedLocationProviderClient`: that lives in Google Play services, which AOSP
//! images and many emulators lack, and it would add a Gradle coordinate to every app linking this
//! part. The platform `LocationManager` is always there.
//!
//! Android reports "this field was not measured" with a `hasXxx()` companion rather than a sentinel,
//! so the shim passes both the value and the flag and this file rebuilds the `Option`s.

use day_android::DayEnv;
use day_android::jni::objects::JValue;
use day_android::with_env;

use crate::{Accuracy, Fix, LocationError};

const CLASS: &str = "dev/daybrite/day/location/DayLocation";

pub fn is_available() -> bool {
    with_env(|env| {
        env.dcall_static(CLASS, "isAvailable", "()Z", &[])
            .ok()
            .and_then(|v| v.z().ok())
            .unwrap_or(false)
    })
}

pub fn start(acc: Accuracy) {
    let best = acc == Accuracy::Best;
    with_env(|env| {
        let _ = env.dcall_static(CLASS, "start", "(Z)V", &[JValue::Bool(best)]);
    });
}

pub fn stop() {
    with_env(|env| {
        let _ = env.dcall_static(CLASS, "stop", "()V", &[]);
    });
}

/// The shim's position callback.
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_daybrite_day_location_DayLocation_nativeFix(
    _env: day_android::jni::EnvUnowned<'_>,
    _class: day_android::jni::objects::JClass<'_>,
    latitude: day_android::jni::sys::jdouble,
    longitude: day_android::jni::sys::jdouble,
    altitude: day_android::jni::sys::jdouble,
    accuracy: day_android::jni::sys::jdouble,
    vertical_accuracy: day_android::jni::sys::jdouble,
    speed: day_android::jni::sys::jdouble,
    course: day_android::jni::sys::jdouble,
    timestamp_ms: day_android::jni::sys::jlong,
    has_altitude: day_android::jni::sys::jint,
    has_accuracy: day_android::jni::sys::jint,
    has_vertical_accuracy: day_android::jni::sys::jint,
    has_speed: day_android::jni::sys::jint,
    has_course: day_android::jni::sys::jint,
) {
    let opt = |flag: i32, v: f64| (flag != 0).then_some(v);
    crate::deliver(Ok(Fix {
        latitude,
        longitude,
        altitude: opt(has_altitude, altitude),
        accuracy_m: opt(has_accuracy, accuracy),
        vertical_accuracy_m: opt(has_vertical_accuracy, vertical_accuracy),
        speed_mps: opt(has_speed, speed),
        course_deg: opt(has_course, course),
        timestamp_ms: (timestamp_ms > 0).then_some(timestamp_ms),
    }));
}

/// The shim's error callback. Codes are DayLocation.java's, not Android's — the platform reports
/// these as exceptions and provider callbacks rather than a single error enum.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_daybrite_day_location_DayLocation_nativeError(
    _env: day_android::jni::EnvUnowned<'_>,
    _class: day_android::jni::objects::JClass<'_>,
    code: day_android::jni::sys::jint,
) {
    crate::deliver(Err(match code {
        1 => LocationError::PermissionDenied,
        2 => LocationError::Disabled,
        4 => LocationError::Unavailable,
        _ => LocationError::Io(format!("location error {code}")),
    }));
}
