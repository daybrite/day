---
title: "Device info"
description: "Device identity (OS, version, model, form factor) as a headless capability crate."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Device identity

Read the device model, operating-system version, and simulator flag for an About screen or
a diagnostic report. `day-part-deviceinfo` returns a snapshot and uses a fallback when a platform
cannot report a field.

## Add to your app

Defined in [`day-part-deviceinfo`](https://github.com/daybrite/day/tree/main/parts/day-part-deviceinfo). Add this to your app’s `Cargo.toml`
`[dependencies]` table, using the same Day Git source as the rest of your app:

```toml
[dependencies]
day-part-deviceinfo = { git = "https://github.com/daybrite/day.git" }
```

The implementation is selected by the compilation target; no toolkit feature flags
or dependency entry in `Day.toml` are needed.

| Platforms / toolkits | Availability |
| --- | --- |
| macOS, iOS, Android, Windows, Linux (GTK / Qt), HarmonyOS | OS and device information; individual fields can be unknown. |
| Web | Fallback values only; model and system version are unknown. |

## Authoring

```rust
let d = day_part_deviceinfo::get();
println!("{} — {} {}{}", d.model, d.system_name, d.system_version,
    if d.is_simulator { " (simulator)" } else { "" });
```

[`get() -> DeviceInfo`](/api/day_part_deviceinfo/fn.get.html) never panics and never returns an error: every platform yields something, and
fields a platform cannot report fall back to `"Unknown"` (or, for `system_name`, the OS family name). It
is a point-in-time snapshot; device identity does not change at runtime, so there is no notification rail.

```rust
pub struct DeviceInfo {
    pub model: String,          // "MacBookPro18,2", "iPhone", "Pixel 7", DMI product_name, …
    pub system_name: String,    // "macOS", "iOS", "Windows", the Linux distro NAME, "OpenHarmony", "Android"
    pub system_version: String, // "26.5.1", "17.5", VERSION_ID, Build.VERSION.RELEASE, …
    pub is_simulator: bool,     // true on the iOS Simulator / an Android emulator; false on desktop
}
```

There are no cargo features; platform selection is purely `#[cfg(target_os)]`/`#[cfg(target_env)]`,
because device identity is an OS concern rather than a toolkit one.
`parts/day-part-deviceinfo/examples/deviceinfo.rs` is a plain `main` that uses it with no Day
framework at all.

## Per-platform native realization

| OS | API | dependency |
|---|---|---|
| macOS | [`ProcessInfo.operatingSystemVersion`](https://developer.apple.com/documentation/foundation/processinfo/operatingsystemversion) + `sysctl hw.model` | `objc2-foundation` + `libc` |
| iOS | [`UIDevice`](https://developer.apple.com/documentation/uikit/uidevice) `model` / `systemName` / `systemVersion` | `objc2` + `objc2-ui-kit` |
| Windows | [`RtlGetVersion`](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-rtlgetversion) (ntdll, resolved via `#[link]`) | raw FFI |
| Linux | [`/etc/os-release`](https://www.freedesktop.org/software/systemd/man/latest/os-release.html) + `/sys/.../dmi/id/product_name` | std only |
| HarmonyOS | [`OH_GetOSFullName`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-basic-services-kit/capi-deviceinfo.md) / [`OH_GetDisplayVersion`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-basic-services-kit/capi-deviceinfo.md) / [`OH_GetProductModel`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-basic-services-kit/capi-deviceinfo.md) (`libdeviceinfo_ndk.so`) | raw FFI (deviceinfo.h) |
| Android | [`android.os.Build.*`](https://developer.android.com/reference/android/os/Build) via a Java shim | `day-android` + `[package.metadata.day.android]` |

## What each platform reports

Every field is best-effort; each platform reports what it can:

- **macOS**: `system_name` is a fixed `"macOS"`; `system_version` is [`ProcessInfo.operatingSystemVersion`](https://developer.apple.com/documentation/foundation/processinfo/operatingsystemversion)
  (the actual running version, unlike the deprecated Gestalt/`sw_vers` paths); `model` is the BSD
  `sysctl` node `hw.model` (e.g. `MacBookPro18,2`, `Macmini9,1`). There is no simulator concept, so
  `is_simulator` is always `false`.
- **iOS**: [`UIDevice`](https://developer.apple.com/documentation/uikit/uidevice) gives `model` (the marketing class `"iPhone"` / `"iPad"`, not the hardware
  identifier), `systemName` (`"iOS"` / `"iPadOS"`) and `systemVersion`. [`UIDevice`](https://developer.apple.com/documentation/uikit/uidevice) is main-thread-only, so
  off the main thread the OS fields fall back to `"Unknown"`. `is_simulator` is definitive from the `sim`
  target ABI, with the simulator's `SIMULATOR_UDID` / `SIMULATOR_DEVICE_NAME` env as a fallback.
- **Windows**: [`RtlGetVersion`](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-rtlgetversion) (ntdll) reports the actual running version (`major.minor.build`), unlike
  the Win32 `GetVersionExW`, which reports an older version on Windows 8+ without a compatibility
  manifest. `system_name` is
  `"Windows"`; `model` is a best-effort `"PC"` (there is no cheap portable hardware-model source). Written
  blind, like the rest of the xaml toolkit.
- **Linux**: no single portable API is guaranteed, so the crate reads the two files every desktop distro
  provides: [`/etc/os-release`](https://www.freedesktop.org/software/systemd/man/latest/os-release.html) (`NAME` / `PRETTY_NAME` for `system_name`, `VERSION_ID` for
  `system_version`) and the DMI node `/sys/devices/virtual/dmi/id/product_name` for `model` (falling back
  to `"Linux"`). `is_simulator` is always `false`.
- **HarmonyOS**: the native `deviceinfo.h` C API: `OH_GetOSFullName()` (e.g. `"OpenHarmony-5.0.0.0"`,
  whose head becomes `system_name`), `OH_GetDisplayVersion()` (`system_version`, falling back to the
  version tail of the full name) and `OH_GetProductModel()` (`model`). No permission required.
- **Android**: [`android.os.Build.*`](https://developer.android.com/reference/android/os/Build), read through a Java shim (`model` from `MANUFACTURER` + `MODEL`,
  `system_name` `"Android"`, `system_version` `VERSION.RELEASE`). `is_simulator` is a heuristic over the
  standard emulator fingerprints (`FINGERPRINT` contains `generic`/`emulator`, `PRODUCT` contains `sdk`,
  `HARDWARE` is `goldfish`/`ranchu`, …). [`Build.*`](https://developer.android.com/reference/android/os/Build) are static fields, so unlike the battery/network/
  clipboard parts, this one needs no `Context`, only the attached JVM. No permission required.

## What it shows about the extension system

Like [`day-part-battery`](/docs/reference/battery) and [`day-part-network`](/docs/reference/network), this is a headless external crate: it has no UI Piece
and registers nothing into any toolkit's `RENDERERS` slice. On Android it rides the Day runtime only
for its attached JNIEnv (via `day-android`); the Java shim is staged into the app's Gradle build by
`[package.metadata.day.android]` without touching any core day crate, and it contributes no manifest
permission. On every other platform the crate is fully day-independent: pure native FFI or std.
