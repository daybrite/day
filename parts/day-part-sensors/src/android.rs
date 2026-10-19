// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Android: SensorManager, read via this crate's Java shim (src/DaySensors.java), which is
// staged into the app's Gradle build by `day build` through [package.metadata.day.android], exactly
// like the UI pieces, but registering no renderer. Android sensors are push-only
// (SensorEventListener), so the shim lazily registers a listener per sensor on the first `read` and
// caches the newest event; Rust polls it. No manifest permission is needed for these three sensors
// at normal rates. The Java uses day-android's cached Context (DayBridge.ctx); Rust calls it through
// day-android's re-exported `jni`.

use day_android::java;

use super::{SensorKind, SensorReading};

// The shim (DaySensors.java, staged into the app's Gradle build by `day build`).
day_android::java_class! {
    DaySensors => dev.daybrite.day.sensors.DaySensors,
    methods {
        static fn isAvailable(kind: jint) -> jboolean,
        static fn read(kind: jint) -> jdouble[],
    },
}

/// The shim's sensor-kind code (DaySensors.java): 0 = accelerometer, 1 = gyroscope, 2 = magnetometer.
fn kind_code(kind: SensorKind) -> i32 {
    match kind {
        SensorKind::Accelerometer => 0,
        SensorKind::Gyroscope => 1,
        SensorKind::Magnetometer => 2,
    }
}

pub fn is_available(kind: SensorKind) -> bool {
    java(|env| DaySensors::isAvailable(env, kind_code(kind))).unwrap_or(false)
}

pub fn sample(kind: SensorKind) -> Option<SensorReading> {
    // The shim's `read()` returns the latest sample as a double[3] {x, y, z}, or null when the
    // sensor is missing or no event has arrived yet (the first call only registers the listener).
    java(|env| {
        let arr = DaySensors::read(env, kind_code(kind))?;
        if arr.is_null() {
            return Ok(None);
        }
        let mut xyz = [0.0f64; 3];
        arr.get_region(env, 0, &mut xyz)?;
        Ok(Some(SensorReading {
            x: xyz[0],
            y: xyz[1],
            z: xyz[2],
        }))
    })
    .flatten()
}
