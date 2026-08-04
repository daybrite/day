# Sensors (headless capability crate)

> **Status: implemented** as `day-part-sensors` (in `parts/`, the headless counterpart of `pieces/`).
> It's a headless day-ecosystem crate (no UI Piece): a shared cross-platform API that STREAMS the
> device's motion sensors (accelerometer, gyroscope, magnetometer) through each platform's native
> API. Any Rust code can depend on it and call `day_part_sensors::watch(kind, cb)`. Host
> build/clippy/tests, the iOS, Android, HarmonyOS and wasm32 cross-compiles, and an end-to-end
> browser check that dispatches a synthetic `devicemotion` and asserts the converted values
> (`scripts/ci/webdom-sensor-test.mjs`, in CI) are all verified; hardware readings need a real device
> (simulators/emulators report unavailable).

## Authoring

```rust
use day_part_sensors::SensorKind;

let watch = day_part_sensors::watch(SensorKind::Accelerometer, |a| {
    println!("{:+.2} {:+.2} {:+.2} m/s²", a.x, a.y, a.z);
});
// Samples arrive until `watch` is dropped.
```

`watch(kind, cb) -> Watch` subscribes; dropping the handle stops delivery, and stops the platform's
stream once the last watcher of that sensor is gone. `is_available(kind) -> bool` reports whether
the sensor exists at all, which is how an app tells "no such sensor" apart from "no sample yet". `SensorReading { x, y, z: f64 }` is in SI units per kind:
m/s² (`Accelerometer`, includes gravity), rad/s (`Gyroscope`), µT (`Magnetometer`). The per-OS
impls normalize (e.g. iOS g → m/s²). Axis signs stay the platform's own convention (face-up is
`z ≈ +9.8` on Android, `z ≈ -9.8` on iOS). `is_available(kind) -> bool` checks for the hardware.

The API is a poll. Sensors are push-model on Android and HarmonyOS, so the first `read` lazily
registers a listener/subscription (kept for the process lifetime) that caches the newest event.
That means the very first `read` may return `None` until the first event lands; poll again shortly.
iOS behaves the same (`startUpdates` + poll the data property); Linux sysfs is a true poll.

The crate has no cargo features: platform selection is purely `#[cfg(target_os)]`, since a motion
sensor depends on the OS, not on which widget toolkit is in use.
`parts/day-part-sensors/examples/sensors.rs` is a plain `main` that uses it with no Day framework
at all.

## Per-platform native realization

| OS | API | dependency |
|---|---|---|
| iOS | CoreMotion `CMMotionManager` (start updates, read the `…Data` properties) | `objc2-core-motion` |
| Android | `SensorManager` + a caching `SensorEventListener` via a Java shim | `day-android` + `[package.metadata.day.android]` |
| HarmonyOS | native `OH_Sensor_Subscribe` push API caching the latest sample (`libohsensor.so`) | raw FFI (SensorServiceKit) |
| Linux | Industrial I/O sysfs (`/sys/bus/iio/devices`, `in_accel_x_raw` × scale …) | std only |
| macOS | none (no public motion-sensor API) | always `None` |
| Windows | stub for now (`Windows.Devices.Sensors` is the future impl) | always `None` |
| Web | `DeviceMotionEvent` through the day-dom shim — accelerometer and gyroscope only | `web.rs` (wasm32; needs the day-dom host page) |

iOS keeps a single `CMMotionManager` in a static (Apple's recommendation); it is not
`MainThreadOnly`, so reads work from any thread. Raw accelerometer/gyro/magnetometer access needs
no `NSMotionUsageDescription` (that key gates the Motion & Fitness APIs). The Simulator has no
sensors → unavailable → `None`.

Android sensors need no manifest permission at the shim's `SENSOR_DELAY_UI` rate. The shim
(`android/java/dev/daybrite/day/sensors/DaySensors.java`) registers one listener per sensor on first
read and caches `{x, y, z}` for Rust to poll via a `double[]` JNI round-trip.

HarmonyOS is `target_os = "linux"` but sandboxes `/sys` away, so it's gated on `target_env = "ohos"`
and uses the native SensorServiceKit C API instead: pure FFI, no Day runtime. **Permissions**: the
accelerometer requires `ohos.permission.ACCELEROMETER` and the gyroscope
`ohos.permission.GYROSCOPE` in the app's `module.json5` `requestPermissions`; the magnetometer needs
none. A failed subscribe (e.g. missing permission) is released and retried on a later read.

## Why a stream, and what the rate is

Every platform's sensor API is already PUSH (`SensorEventListener`, CoreMotion handlers,
`OH_Sensor_Subscribe`, `devicemotion`), so the older `read()` poll was an adapter pointing the wrong
way: each per-OS arm had to cache the newest event purely so a caller could ask for it. `watch`
removes that inversion, and "no sample yet" stops being a poll artifact.

Samples are delivered at a steady ~20 Hz (`day_part_sensors::SAMPLE_MS`), not at the sensor's own
rate: enough for a readout or a chart, and it keeps a fast sensor from flooding an app's UI thread.
Natively, one thread per watched sensor drives that cadence. **On the web there is no thread to
sample on** (`std::thread::spawn` panics on wasm32), so the day-dom shim drives the feed with a
timer that calls back into the module. `on_sample` therefore runs on an unspecified background
thread natively and on the sole browser thread on the web; deliver into UI state with a
`day_reactive::Setter` either way.

## The web arm

`DeviceMotionEvent` carries acceleration and rotation together, so one listener feeds both kinds:

| kind | source | conversion |
|---|---|---|
| `Accelerometer` | `accelerationIncludingGravity` (m/s²) | none — day's contract already says "including gravity" |
| `Gyroscope` | `rotationRate` (deg/s) | × π/180, with beta→x, gamma→y, alpha→z |
| `Magnetometer` | — | **no cross-browser API exists**; Chromium's Generic Sensor `Magnetometer` is flag-gated and absent from Safari and Firefox, so this reports unavailable rather than pretending |

Three browser realities:

- **A secure context is required.** `devicemotion` fires only over HTTPS or on localhost (both
  `day launch`'s server and the hosted showcase qualify), and `Permissions-Policy` defaults
  `accelerometer`/`gyroscope` to `self`, so a cross-origin iframe embed needs delegation.
- **iOS Safari needs a user gesture.** `DeviceMotionEvent.requestPermission()` must be called from a
  live user activation; that is `Permission::Motion` in [day-part-permissions](permissions.md), and
  it must be requested from inside a button's action, where the gesture is still live. Calling it
  after an `.await` does not work.
- **Availability is only knowable in retrospect.** `'DeviceMotionEvent' in window` is true on a
  desktop browser with no hardware, so the shim reports available until a short grace period passes
  with no event, and unavailable after. That is the right answer for a laptop.

Because headless WebKit has no motion hardware, CI cannot prove any of this from the walkthrough
alone. `scripts/ci/webdom-sensor-test.mjs` dispatches a synthetic `devicemotion` with known values
and asserts the converted numbers, which pins the unit conversion and the axis mapping: the two
things a browser sensor arm actually gets wrong.

Linux computes `(raw + offset) × scale` from the first iio device exposing the channel triple
(magnetometer scale yields Gauss → ×100 for µT). Most desktops/CI runners have no motion sensors →
`None`; real coverage is laptops/tablets with rotation accelerometers.

## What it shows about the extension system

Like `day-part-battery`, this is a headless external crate: it has no UI Piece and registers nothing
in any backend's `RENDERERS` slice. It contributes its Android Java through
`[package.metadata.day.android]` just like the UI pieces but registers no renderer. It also adds a
wrinkle battery didn't have: adapting push-model platform APIs (Android listeners, HarmonyOS
subscriptions) behind a poll API by lazily subscribing on first use and caching the latest sample.
On Android the crate rides on the Day runtime (day-android's cached JVM + `DayBridge.ctx`); on every
other platform it is fully day-independent.
