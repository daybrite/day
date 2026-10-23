---
title: "Sensors"
description: "Event-driven accelerometer, gyroscope, and magnetometer subscriptions."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Sensors

Use motion and magnetic-field measurements for interactive controls, measurements, and live
charts. `day-part-sensors` delivers new readings through callbacks and releases the sensor source
when its last subscription ends.

## Add to your app

Add [`day-part-sensors`](https://github.com/daybrite/day/tree/main/parts/day-part-sensors)
to your app's Cargo dependencies, using the same Day Git source as the rest of the app:

```toml
[dependencies]
day-part-sensors = { git = "https://github.com/daybrite/day.git" }
```

Platform selection follows the compilation target. There are no toolkit feature flags.

| Platform | Source |
| --- | --- |
| iOS / UIKit | CoreMotion handlers, where hardware is available. |
| Android | SensorManager events. |
| HarmonyOS / ArkUI | SensorServiceKit callbacks. |
| Windows / WinUI, GTK, Qt | WinRT sensor events, where hardware is available. |
| Web / DOM | DeviceMotionEvent; accelerometer and gyroscope only. |
| macOS / AppKit, GTK, Qt | Unavailable. |
| Linux / GTK, Qt | Unavailable; buffered IIO support is not implemented. |

## Subscribe

[`watch`](/api/day_part_sensors/fn.watch.html) takes a sensor kind and a callback.
Keep the returned [`Watch`](/api/day_part_sensors/struct.Watch.html) for as long
as you want updates. In a Day page, bind it to the reactive scope:

```rust
use day::prelude::*;
use day_part_sensors::{SensorKind, SensorReading};

let reading = Signal::new(None::<SensorReading>);
let set = reading.setter();
let watch = day_part_sensors::watch(SensorKind::Accelerometer, move |sample| {
    set.set(Some(sample));
});
Scope::current().on_cleanup(move || drop(watch));
```

Format localized readouts on the UI thread. The callback should pass raw measurements through
its setter, as described in [asynchronous work](async.md).

[`SensorReading`](/api/day_part_sensors/struct.SensorReading.html) contains
`x`, `y`, and `z` in SI units: acceleration including gravity in m/s², angular velocity in rad/s,
and magnetic field in µT. Axis signs follow each platform's coordinate system.

[`is_available`](/api/day_part_sensors/fn.is_available.html) probes support
without starting a sensor. Native availability does not imply that permission has been granted.
On the web it checks API support and a secure context; it cannot establish whether hardware exists.
An unavailable or denied source delivers no samples. Request access through
[permissions](permissions.md) before subscribing. If a native subscription fails, drop the watch
and create another after correcting access or service availability.

## Calibrated and uncalibrated readings

A `SensorKind` selects the normal platform sensor. Choose a processing mode explicitly with
`kind.with_calibration(Calibration::Uncalibrated)`; both `watch` and `is_available` accept it:

```rust
use day_part_sensors::{Calibration, SensorKind};

let sensor = SensorKind::Magnetometer.with_calibration(Calibration::Uncalibrated);
if day_part_sensors::is_available(sensor) {
    let watch = day_part_sensors::watch(sensor, |r| {
        println!("{} {} {} µT", r.x, r.y, r.z);
    });
    // Keep `watch` for as long as the app needs this stream.
}
```

`Calibration::Calibrated` requests the normal platform source, with platform bias correction
where provided. It does not certify accuracy or eliminate nearby magnetic interference.
`Uncalibrated` omits runtime bias correction; factory calibration and temperature compensation
may still apply. Acceleration includes gravity in both modes. Removing gravity is a different
operation from calibration.

| Platform | Selectable calibration modes |
| --- | --- |
| iOS | Gyroscope and magnetometer. No selectable accelerometer bias mode. |
| Android | All three kinds, if the device supplies the corresponding uncalibrated sensor; uncalibrated acceleration requires API 26+. |
| Windows, HarmonyOS, web | Normal source only. Explicit uncalibrated requests are unavailable. |
| macOS, Linux | No sensor sources. |

Query both modes before offering a calibration toggle. Unsupported modes deliver no samples and
never fall back silently. Calibrated and uncalibrated subscriptions can coexist: each has its own
listeners, delivery generations, and lifetime. To change modes, drop the old watch and subscribe
to the other mode. Clear the displayed history rather than joining readings with different bias
processing, and reject already queued UI updates from the old subscription.

## Delivery and lifetime

Subscriptions share one native source per sensor kind and calibration mode. Supported native sources request approximately
20 Hz, subject to the platform's minimum interval and hardware. Browsers control their event rate.
A new event with unchanged values is still a new sample. Day never repeats cached values on a timer.

Native callbacks run on one background dispatcher. Platform callbacks enqueue at most one pending
reading per sensor; newer readings replace it if an application callback is slow. Callbacks must
return promptly because they share the dispatcher. This API suits live state and charts, not
lossless recording. Web callbacks run on the browser thread.

Dropping a watch cancels its queued delivery. A callback already selected for delivery may finish.
The last watcher leaving schedules native teardown after the current callback, so a callback can
safely drop its watch or create another. Subscription generations reject late events from an old
source. The dispatcher waits for events and exits after all sources have stopped.

## Platform requirements

**UIKit:** [CMMotionManager](https://developer.apple.com/documentation/coremotion/cmmotionmanager)
delivers samples to an operation queue. Day keeps one manager, normalizes acceleration from g to
m/s², and calls the matching stop method on teardown. Magnetometer subscriptions use
[CMDeviceMotion.magneticField](https://developer.apple.com/documentation/coremotion/cmdevicemotion/magneticfield)
with `xArbitraryCorrectedZVertical` to remove device bias. The field remains in device coordinates
and µT. Raw `CMMagnetometerData` includes that bias and is unsuitable for a compass.
Uncalibrated samples are withheld; gently rotate the device if readings have not started.
Day does not show a system calibration dialog. Nearby metal still affects calibrated readings.
Calibrated gyroscope readings use
[CMDeviceMotion.rotationRate](https://developer.apple.com/documentation/coremotion/cmdevicemotion/rotationrate),
which removes gyroscope bias.
Both calibrated kinds share one device-motion subscription; it stops after both have unsubscribed.
When only gyroscope readings are requested, `xArbitraryZVertical` avoids requiring a magnetometer.
Changing that membership restarts device motion with the appropriate reference frame and rejects
callbacks from the previous stream. Raw gyroscope and magnetic-field requests use `CMGyroData`
and `CMMagnetometerData` independently. Sensor fusion may still use underlying hardware even
when the app pauses a separate subscription to that sensor.

Test on a physical device; simulators lack motion sensors. Declare motion access through
[permissions](permissions.md) for apps using motion and fitness capabilities.

**Android:** [SensorManager](https://developer.android.com/reference/android/hardware/SensorManager)
registers one listener per active kind. The Java callback forwards values through JNI; application
callbacks execute on the Rust dispatcher. Teardown unregisters the listener. The normal reporting
rate used here does not require the high-sampling-rate permission. Uncalibrated subscriptions use
`TYPE_ACCELEROMETER_UNCALIBRATED`, `TYPE_GYROSCOPE_UNCALIBRATED`, or
`TYPE_MAGNETIC_FIELD_UNCALIBRATED`. Day delivers the measured vector, without subtracting the
separate bias estimate supplied by Android.

**ArkUI:** [SensorServiceKit](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-sensor-service-kit/capi-oh-sensor-h.md)
uses `OH_Sensor_Subscribe` and `OH_Sensor_Unsubscribe`. Declare
`ohos.permission.ACCELEROMETER` and `ohos.permission.GYROSCOPE` in the application's
`module.json5` for those kinds. The magnetometer needs neither permission.

**Windows:** [ReadingChanged](https://learn.microsoft.com/en-us/uwp/api/windows.devices.sensors.accelerometer.readingchanged)
subscriptions retain the sensor and event token until teardown. Acceleration and angular velocity
are converted to SI units. Many desktop machines have no sensors.

**DOM:** The host uses one `devicemotion` listener for both supported kinds. It forwards
`accelerationIncludingGravity` unchanged and converts `rotationRate` from degrees/s to rad/s,
with beta → x, gamma → y, and alpha → z. The listener is removed when both kinds are unsubscribed.
Partial or nonfinite measurements are ignored.

Web delivery requires HTTPS or localhost. Cross-origin frames need the appropriate
`Permissions-Policy` delegation. Safari motion permission must be requested from a user gesture
through `Permission::Motion`; the subscription does not prompt. A browser with API support but
no sensor hardware remains waiting for its first event. There is no portable magnetometer source.
