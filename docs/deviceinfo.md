---
title: "Device info"
description: "Device identity (OS, version, model, form factor) as a headless capability crate."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Device identity

Device information describes the hardware and operating system running an app. It gives an About
screen or diagnostic report the context needed to identify a device and investigate a problem.

`day-part-device` returns a snapshot of the device model, operating-system version, and simulator
flag. It uses a fallback when a platform cannot report a field.

## Add to your app

Add [`day-part-device`](https://github.com/daybrite/day/tree/main/crates/day-part-device) to your app:

```sh
cargo add 'day-part-device@0.*'
```

The implementation is selected by the compilation target; no toolkit feature flags
or dependency entry in `Day.toml` are needed.

| Platforms / toolkits | Availability |
| --- | --- |
| [macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit"), [iOS](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit"), [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material"), [Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui"), [Linux](https://daybrite.dev/docs/reference/platforms "toolkit:linux-gtk,linux-qt") ([GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") / [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt")), [HarmonyOS](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | OS and device information; individual fields can be unknown. |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | Fallback values only; model and system version are unknown. |

## Authoring

```rust
let d = day_part_device::info::get();
println!("{} — {} {}{}", d.model, d.system_name, d.system_version,
    if d.is_simulator { " (simulator)" } else { "" });
```

[`get() -> DeviceInfo`](https://docs.rs/day-part-device/latest/day_part_device/info/fn.get.html) never panics and never returns an error: every platform yields something, and
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
`crates/day-part-device/examples/deviceinfo.rs` is a plain `main` that uses it with no Day
framework at all.

## Platform implementations

| OS | API | dependency |
|---|---|---|
| [macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | [`ProcessInfo.operatingSystemVersion`](https://developer.apple.com/documentation/foundation/processinfo/operatingsystemversion) + `sysctl hw.model` | `objc2-foundation` + `libc` |
| [iOS](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | [`UIDevice`](https://developer.apple.com/documentation/uikit/uidevice) `model` / `systemName` / `systemVersion` | `objc2` + `objc2-ui-kit` |
| [Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | [`RtlGetVersion`](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-rtlgetversion) (ntdll, resolved via `#[link]`) | raw FFI |
| [Linux](https://daybrite.dev/docs/reference/platforms "toolkit:linux-gtk,linux-qt") | [`/etc/os-release`](https://www.freedesktop.org/software/systemd/man/latest/os-release.html) + `/sys/.../dmi/id/product_name` | std only |
| [HarmonyOS](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | [`OH_GetOSFullName`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-basic-services-kit/capi-deviceinfo.md) / [`OH_GetDisplayVersion`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-basic-services-kit/capi-deviceinfo.md) / [`OH_GetProductModel`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-basic-services-kit/capi-deviceinfo.md) (`libdeviceinfo_ndk.so`) | raw FFI (deviceinfo.h) |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | [`android.os.Build.*`](https://developer.android.com/reference/android/os/Build) via a Java shim | `day-toolkit-material` + `[package.metadata.day.android]` |

## What each platform reports

Every field is best-effort; each platform reports what it can:

- **[macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit")**: `system_name` is a fixed `"macOS"`; `system_version` is [`ProcessInfo.operatingSystemVersion`](https://developer.apple.com/documentation/foundation/processinfo/operatingsystemversion)
  (the actual running version, unlike the deprecated Gestalt/`sw_vers` paths); `model` is the BSD
  `sysctl` node `hw.model` (e.g. `MacBookPro18,2`, `Macmini9,1`). There is no simulator concept, so
  `is_simulator` is always `false`.
- **[iOS](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit")**: [`UIDevice`](https://developer.apple.com/documentation/uikit/uidevice) gives `model` (the marketing class `"iPhone"` / `"iPad"`, not the hardware
  identifier), `systemName` (`"iOS"` / `"iPadOS"`) and `systemVersion`. [`UIDevice`](https://developer.apple.com/documentation/uikit/uidevice) is main-thread-only, so
  off the main thread the OS fields fall back to `"Unknown"`. `is_simulator` is definitive from the `sim`
  target ABI, with the simulator's `SIMULATOR_UDID` / `SIMULATOR_DEVICE_NAME` env as a fallback.
- **[Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui")**: [`RtlGetVersion`](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/nf-wdm-rtlgetversion) (ntdll) reports the actual running version (`major.minor.build`), unlike
  the Win32 `GetVersionExW`, which reports an older version on Windows 8+ without a compatibility
  manifest. `system_name` is
  `"Windows"`; `model` is a best-effort `"PC"` (there is no cheap portable hardware-model source). Written
  blind, like the rest of the WinUI toolkit.
- **[Linux](https://daybrite.dev/docs/reference/platforms "toolkit:linux-gtk,linux-qt")**: no single portable API is guaranteed, so the crate reads the two files every desktop distro
  provides: [`/etc/os-release`](https://www.freedesktop.org/software/systemd/man/latest/os-release.html) (`NAME` / `PRETTY_NAME` for `system_name`, `VERSION_ID` for
  `system_version`) and the DMI node `/sys/devices/virtual/dmi/id/product_name` for `model` (falling back
  to `"Linux"`). `is_simulator` is always `false`.
- **[HarmonyOS](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui")**: the native `deviceinfo.h` C API: `OH_GetOSFullName()` (e.g. `"OpenHarmony-5.0.0.0"`,
  whose head becomes `system_name`), `OH_GetDisplayVersion()` (`system_version`, falling back to the
  version tail of the full name) and `OH_GetProductModel()` (`model`). No permission required.
- **[Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material")**: [`android.os.Build.*`](https://developer.android.com/reference/android/os/Build), read through a Java shim (`model` from `MANUFACTURER` + `MODEL`,
  `system_name` `"Android"`, `system_version` `VERSION.RELEASE`). `is_simulator` is a heuristic over the
  standard emulator fingerprints (`FINGERPRINT` contains `generic`/`emulator`, `PRODUCT` contains `sdk`,
  `HARDWARE` is `goldfish`/`ranchu`, …). [`Build.*`](https://developer.android.com/reference/android/os/Build) are static fields, so unlike the battery/network/
  clipboard parts, this one needs no `Context`, only the attached JVM. No permission required.

## Extension integration

Like [`day-part-device`](/docs/reference/battery) and [`day-part-connectivity`](/docs/reference/network), this is a headless external crate: it has no UI Piece
and registers nothing into any toolkit's `RENDERERS` slice. On Android it rides the Day runtime only
for its attached JNIEnv (via `day-toolkit-material`); the Java shim is staged into the app's Gradle build by
`[package.metadata.day.android]` without touching any core day crate, and it contributes no manifest
permission. On every other platform the crate is fully day-independent: pure native FFI or std.
