# Sensors (headless capability crate)

> **Status: implemented** as `day-part-sensors` (in `parts/`, the headless counterpart of `pieces/`) — a **headless**
> day-ecosystem crate (no UI Piece): a shared cross-platform API for polling the device's motion
> sensors (accelerometer, gyroscope, magnetometer) through each platform's NATIVE API. Any Rust code
> can depend on it and call `day_part_sensors::read(kind)`. Host build/clippy/tests, iOS and Android
> clippy, and the HarmonyOS cross-compile (linked against the native `libohsensor.so`) are all
> verified; hardware readings need a real device (simulators/emulators report unavailable).

## Authoring

```rust
use day_part_sensors::SensorKind;

if day_part_sensors::is_available(SensorKind::Accelerometer) {
    if let Some(a) = day_part_sensors::read(SensorKind::Accelerometer) {
        println!("{:+.2} {:+.2} {:+.2} m/s²", a.x, a.y, a.z);
    }
}
```

`read(kind) -> Option<SensorReading>` returns the LATEST sample, `None` where the sensor (or a
platform API for it) doesn't exist. `SensorReading { x, y, z: f64 }` is in SI units per kind —
m/s² (`Accelerometer`, includes gravity), rad/s (`Gyroscope`), µT (`Magnetometer`) — the per-OS
impls normalize (e.g. iOS g → m/s²). Axis signs stay the platform's own convention (face-up is
`z ≈ +9.8` on Android, `z ≈ -9.8` on iOS). `is_available(kind) -> bool` checks for the hardware.

The API is a **poll**. Sensors are push-model on Android and HarmonyOS, so the first `read` lazily
registers a listener/subscription (kept for the process lifetime) that caches the newest event —
meaning the very first `read` may return `None` until the first event lands; poll again shortly.
iOS behaves the same (`startUpdates` + poll the data property); Linux sysfs is a true poll.

There are **no features** — platform selection is purely `#[cfg(target_os)]`, because a motion sensor
is an OS concern, not a widget-toolkit one. `parts/day-part-sensors/examples/sensors.rs` is a plain
`main` that uses it with no Day framework at all.

## Per-platform native realization

| OS | API | dependency |
|---|---|---|
| iOS | CoreMotion `CMMotionManager` (start updates, poll `…Data` properties) | `objc2-core-motion` |
| Android | `SensorManager` + a caching `SensorEventListener` via a Java shim | `day-android` + `[package.metadata.day.android]` |
| HarmonyOS | native `OH_Sensor_Subscribe` push API caching the latest sample (`libohsensor.so`) | raw FFI (SensorServiceKit) |
| Linux | Industrial I/O sysfs (`/sys/bus/iio/devices`, `in_accel_x_raw` × scale …) | std only |
| macOS | none — no public motion-sensor API | always `None` |
| Windows | stub for now (`Windows.Devices.Sensors` is the future impl) | always `None` |

iOS keeps a single `CMMotionManager` in a static (Apple's recommendation); it is not
`MainThreadOnly`, so reads work from any thread. Raw accelerometer/gyro/magnetometer access needs
**no** `NSMotionUsageDescription` (that key gates the Motion & Fitness APIs). The Simulator has no
sensors → unavailable → `None`.

Android sensors need no manifest permission at the shim's `SENSOR_DELAY_UI` rate. The shim
(`android/java/dev/daybrite/day/sensors/DaySensors.java`) registers one listener per sensor on first
read and caches `{x, y, z}` for Rust to poll via a `double[]` JNI round-trip.

HarmonyOS is `target_os = "linux"` but sandboxes `/sys` away, so it's gated on `target_env = "ohos"`
and uses the native SensorServiceKit C API instead — pure FFI, no Day runtime. **Permissions**: the
accelerometer requires `ohos.permission.ACCELEROMETER` and the gyroscope
`ohos.permission.GYROSCOPE` in the app's `module.json5` `requestPermissions`; the magnetometer needs
none. A failed subscribe (e.g. missing permission) is released and retried on a later read.

Linux computes `(raw + offset) × scale` from the first iio device exposing the channel triple
(magnetometer scale yields Gauss → ×100 for µT). Most desktops/CI runners have no motion sensors →
`None`; real coverage is laptops/tablets with rotation accelerometers.

## What it shows about the extension system

Like `day-part-battery`, this is a headless external crate — no UI Piece, nothing registered into any
backend's `RENDERERS` slice. It contributes its Android Java through `[package.metadata.day.android]`
exactly like the UI pieces but registers no renderer, and it adds a wrinkle battery didn't have:
adapting **push-model** platform APIs (Android listeners, HarmonyOS subscriptions) behind a poll API
by lazily subscribing on first use and caching the latest sample. On Android the crate rides on the
Day runtime (day-android's cached JVM + `DayBridge.ctx`); on every other platform it is fully
day-independent.
