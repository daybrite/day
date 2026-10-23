---
title: "Battery"
description: "Battery level and charging state as reactive signals, via the day-part-battery headless capability crate."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Battery

Read the battery level and charging state to show a charge indicator or adjust power-hungry work.
`day-part-battery` returns a snapshot through the platform battery API, without requiring a UI.

## Add to your app

Defined in [`day-part-battery`](https://github.com/daybrite/day/tree/main/parts/day-part-battery). Add this to your app’s `Cargo.toml`
`[dependencies]` table, using the same Day Git source as the rest of your app:

```toml
[dependencies]
day-part-battery = { git = "https://github.com/daybrite/day.git" }
```

The implementation is selected by the compilation target; no toolkit feature flags
or dependency entry in `Day.toml` are needed.

| Platforms / toolkits | Availability |
| --- | --- |
| macOS, iOS, Android, Windows, Linux (GTK / Qt), HarmonyOS | Battery information where hardware and OS reporting are available. |
| Web | Unavailable; [`status()`](https://daybrite.dev/api/day_part_battery/fn.status.html) returns `None`. |

## Authoring

```rust
if let Some(b) = day_part_battery::status() {
    println!("{:?} · {}%", b.state, b.percent().unwrap_or(0));
}
```

[`status() -> Option<BatteryStatus>`](https://daybrite.dev/api/day_part_battery/fn.status.html) returns `None` where there's no battery API (or no battery).
[`BatteryStatus { level: Option<f32>, state: BatteryState }`](https://daybrite.dev/api/day_part_battery/struct.BatteryStatus.html); `level` is `0.0..=1.0` ([`percent()`](https://daybrite.dev/api/day_part_battery/struct.BatteryStatus.html#method.percent) gives
`0..=100`); [`BatteryState`](https://daybrite.dev/api/day_part_battery/enum.BatteryState.html) is `Charging | Discharging | Full | NotCharging | Unknown`.

There are no cargo features; platform selection is purely `#[cfg(target_os)]`, because a battery is
an OS concern rather than a toolkit one. `parts/day-part-battery/examples/battery.rs` is a plain
`main` that uses it with no Day framework at all.

## Per-platform native realization

| OS | API | dependency |
|---|---|---|
| macOS | IOKit [`IOPowerSources`](https://developer.apple.com/documentation/iokit/iopowersources_h) (CoreFoundation) | `core-foundation`, IOKit framework |
| iOS | [`UIDevice.batteryLevel`](https://developer.apple.com/documentation/uikit/uidevice) / `batteryState` | `objc2-ui-kit` |
| Windows | [`GetSystemPowerStatus`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getsystempowerstatus) | raw FFI (kernel32) |
| Linux | [`/sys/class/power_supply`](https://docs.kernel.org/power/power_supply_class.html) | std only |
| HarmonyOS | native [`OH_BatteryInfo_GetCapacity`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-basic-services-kit/capi-oh-batteryinfo.md) / [`GetPluggedType`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-basic-services-kit/capi-oh-batteryinfo.md) (`libohbattery_info.so`) | raw FFI (BasicServicesKit) |
| Android | [`BatteryManager`](https://developer.android.com/reference/android/os/BatteryManager) via the inline Java arm in `src/android.rs` | `day-android` + [`day-bridge`](/contributing/backend-contracts/bridge) ([bridge](bridge.md)) |

iOS reads on the main thread ([`UIDevice`](https://developer.apple.com/documentation/uikit/uidevice) is `MainThreadOnly`); off it, [`status()`](https://daybrite.dev/api/day_part_battery/fn.status.html) returns `None`. The
simulator has no battery, so you get `level: None, state: Unknown` (the API path still runs).

HarmonyOS is `target_os = "linux"` but sandboxes `/sys` away, so it's gated on `target_env = "ohos"`
and uses the native BasicServicesKit C API instead. That's pure FFI: it needs neither a permission
nor the Day runtime (unlike Android). The native API exposes capacity + plugged type but no explicit
charge state, so the state is inferred from whether external power is connected. The showcase's
Device & sensors page shows a live readout ([docs/harmonyos](harmonyos.md)).

## What it shows about the extension system

This is the first headless external crate: it has no UI Piece and registers nothing into any
toolkit's `RENDERERS` slice. It demonstrates that the standalone-piece toolkit-contribution
mechanism (see [extending](extending.md)) already accommodates headless capability crates:
[`day-part-battery`](https://daybrite.dev/api/day_part_battery/index.html) contributes an Android implementation exactly like the UI pieces, but registers
no renderer; the staging is independent of rendering. On Android the crate rides on the Day runtime
(it uses day-android's cached JVM + `DayBridge.ctx`); on every other platform it is fully
day-independent.

The Android half is a **daybridge** arm ([bridge](bridge.md)): the Java that reads
[`BatteryManager`](https://developer.android.com/reference/android/os/BatteryManager) lives inline in `parts/day-part-battery/src/android.rs`, beside the declaration it
implements, and `day build` stages it into the app's Gradle build. It replaced a checked-in
`DayBattery.java`, a `[package.metadata.day.android] java = [...]` table, and a packed-`i64` wire
format that was written twice and kept in agreement by comment. [day-part-speech](https://github.com/daybrite/day-part-speech) is the
fuller demonstration, with six languages in one file, in a separate repository.
