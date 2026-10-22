<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-part-sensors

Subscribe to accelerometer, gyroscope, and magnetometer events with `watch(kind, callback)`.
Dropping the last watch releases the native source. Native callbacks use a background dispatcher;
web callbacks run on the browser thread. Slow consumers receive the newest pending reading.
Pass `kind.with_calibration(Calibration::Uncalibrated)` to select an uncalibrated source, and
query `is_available` for that mode before offering it. Kind-only calls use the normal platform
source; calibrated and uncalibrated subscriptions have independent lifetimes.

Supported sources are CoreMotion on iOS, SensorManager on Android, SensorServiceKit on HarmonyOS,
WinRT on Windows, and DeviceMotionEvent on the web. Desktop Linux and macOS report unavailable.
Android needs Day's JVM host and web needs the day-dom host shim; the other implementations are
independent of Day UI toolkits. See the [sensor API](../../docs/sensors.md) for units and permissions.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's own widgets — AppKit, UIKit, Android's Material widgets, GTK 4, Qt 6,
XAML, and ArkUI — from one codebase. When you write `button("Save")`, macOS shows an
`NSButton` and Android shows a Material button. The framework also ships the tooling around
the app: the `day` CLI, a VS Code extension, GitHub CI workflows, localization,
accessibility, and dayscript automation.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
