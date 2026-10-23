// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Android: `LocationManager.requestLocationUpdates` through this crate's Java shim
//! (`src/DayLocation.java`), staged into the app's Gradle build by `day build`.
//!
//! Not `FusedLocationProviderClient`: that lives in Google Play services, which AOSP
//! images and many emulators lack, and it would add a Gradle coordinate to every app linking this
//! part. The platform `LocationManager` is always there.
//!
//! Android reports "this field was not measured" with a `hasXxx()` companion rather than a sentinel,
//! so the shim passes both the value and the flag and this file rebuilds the `Option`s.

use day_android_runtime::jni::Env;
use day_android_runtime::jni::objects::JClass;
use day_android_runtime::jni::sys::{jdouble, jint, jlong};
use day_android_runtime::{java, vm_ready};

use crate::{Accuracy, Fix, LocationError};

day_android_runtime::java_class! {
    DayLocation => dev.daybrite.day.location.DayLocation,
    methods {
        static fn isAvailable() -> jboolean,
        static fn start(best: jboolean),
        static fn stop(),
    },
    native_methods {
        static fn nativeFix(
            latitude: jdouble, longitude: jdouble, altitude: jdouble, accuracy: jdouble,
            vertical_accuracy: jdouble, speed: jdouble, course: jdouble, timestamp_ms: jlong,
            has_altitude: jint, has_accuracy: jint, has_vertical_accuracy: jint, has_speed: jint,
            has_course: jint,
        ),
        static fn nativeError(code: jint),
    },
}

pub fn is_available() -> bool {
    vm_ready() && java(|env| DayLocation::isAvailable(env)).unwrap_or(false)
}

pub fn start(acc: Accuracy) {
    java(|env| DayLocation::start(env, acc == Accuracy::Best));
}

pub fn stop() {
    java(|env| DayLocation::stop(env));
}

impl DayLocationNativeInterface for DayLocationAPI {
    type Error = day_android_runtime::jni::errors::Error;

    /// The shim's position callback.
    fn nativeFix<'l>(
        _env: &mut Env<'l>,
        _class: JClass<'l>,
        latitude: jdouble,
        longitude: jdouble,
        altitude: jdouble,
        accuracy: jdouble,
        vertical_accuracy: jdouble,
        speed: jdouble,
        course: jdouble,
        timestamp_ms: jlong,
        has_altitude: jint,
        has_accuracy: jint,
        has_vertical_accuracy: jint,
        has_speed: jint,
        has_course: jint,
    ) -> Result<(), Self::Error> {
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
        Ok(())
    }

    /// The shim's error callback. Codes are DayLocation.java's, not Android's; the platform
    /// reports these as exceptions and provider callbacks rather than a single error enum.
    fn nativeError<'l>(
        _env: &mut Env<'l>,
        _class: JClass<'l>,
        code: jint,
    ) -> Result<(), Self::Error> {
        crate::deliver(Err(match code {
            1 => LocationError::PermissionDenied,
            2 => LocationError::Disabled,
            4 => LocationError::Unavailable,
            _ => LocationError::Io(format!("location error {code}")),
        }));
        Ok(())
    }
}
