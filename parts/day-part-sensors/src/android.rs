// Android: SensorManager, read via this crate's OWN Java shim (android/java/…/DaySensors.java) —
// staged into the app's Gradle build by `day build` through [package.metadata.day.android], exactly
// like the UI pieces, but registering NO renderer. Android sensors are push-only
// (SensorEventListener), so the shim lazily registers a listener per sensor on the first `read` and
// caches the newest event; Rust polls it. No manifest permission is needed for these three sensors
// at normal rates. The Java uses day-android's cached Context (DayBridge.ctx); Rust calls it through
// day-android's re-exported `jni`.

use day_android::DayEnv;
use day_android::jni::objects::{JDoubleArray, JValue};
use day_android::with_env;

use super::{SensorKind, SensorReading};

const SENSORS_CLASS: &str = "dev/daybrite/day/sensors/DaySensors";

/// The shim's sensor-kind code (DaySensors.java): 0 = accelerometer, 1 = gyroscope, 2 = magnetometer.
fn kind_code(kind: SensorKind) -> i32 {
    match kind {
        SensorKind::Accelerometer => 0,
        SensorKind::Gyroscope => 1,
        SensorKind::Magnetometer => 2,
    }
}

pub fn is_available(kind: SensorKind) -> bool {
    with_env(|env| {
        env.dcall_static(
            SENSORS_CLASS,
            "isAvailable",
            "(I)Z",
            &[JValue::Int(kind_code(kind))],
        )
        .ok()
        .and_then(|v| v.z().ok())
        .unwrap_or(false)
    })
}

pub fn read(kind: SensorKind) -> Option<SensorReading> {
    // `read()` returns the latest sample as a double[3] {x, y, z}, or null when the sensor is
    // missing or no event has arrived yet (the first call only registers the listener).
    with_env(|env| {
        let obj = env
            .dcall_static(
                SENSORS_CLASS,
                "read",
                "(I)[D",
                &[JValue::Int(kind_code(kind))],
            )
            .ok()?
            .l()
            .ok()?;
        if obj.is_null() {
            return None;
        }
        let arr: day_android::jni::objects::JDoubleArray = unsafe { std::mem::transmute(obj) };
        let mut xyz = [0.0f64; 3];
        arr.get_region(env, 0, &mut xyz).ok()?;
        Some(SensorReading {
            x: xyz[0],
            y: xyz[1],
            z: xyz[2],
        })
    })
}
