# Device identity (headless capability crate)

> **Status: implemented** as `day-part-deviceinfo` (in `parts/`, the headless counterpart of
> `pieces/`). It's a **headless** day-ecosystem crate (no UI Piece): a shared cross-platform API for a
> snapshot of the device's identity (model, OS name/version, simulator/emulator flag) through each
> platform's native API. Any Rust code can depend on it and call `day_part_deviceinfo::get()`.
> Verified on macOS (real reading, e.g. `MacBookPro18,2 — macOS 26.5.1`); iOS-sim/Android
> clippy-clean, Linux/HarmonyOS cross-compile (HarmonyOS binds the native `libdeviceinfo_ndk.so`).

## Authoring

```rust
let d = day_part_deviceinfo::get();
println!("{} — {} {}{}", d.model, d.system_name, d.system_version,
    if d.is_simulator { " (simulator)" } else { "" });
```

`get() -> DeviceInfo` never panics and never returns an error: every platform yields something, and
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
| macOS | `ProcessInfo.operatingSystemVersion` + `sysctl hw.model` | `objc2-foundation` + `libc` |
| iOS | `UIDevice` `model` / `systemName` / `systemVersion` | `objc2` + `objc2-ui-kit` |
| Windows | `RtlGetVersion` (ntdll, resolved via `#[link]`) | raw FFI |
| Linux | `/etc/os-release` + `/sys/.../dmi/id/product_name` | std only |
| HarmonyOS | `OH_GetOSFullName` / `OH_GetDisplayVersion` / `OH_GetProductModel` (`libdeviceinfo_ndk.so`) | raw FFI (deviceinfo.h) |
| Android | `android.os.Build.*` via a Java shim | `day-android` + `[package.metadata.day.android]` |

## What each platform reports

Every field is best-effort; each platform reports what it can:

- **macOS**: `system_name` is a fixed `"macOS"`; `system_version` is `ProcessInfo.operatingSystemVersion`
  (the actual running version, unlike the deprecated Gestalt/`sw_vers` paths); `model` is the BSD
  `sysctl` node `hw.model` (e.g. `MacBookPro18,2`, `Macmini9,1`). There is no simulator concept, so
  `is_simulator` is always `false`.
- **iOS**: `UIDevice` gives `model` (the marketing class `"iPhone"` / `"iPad"`, not the hardware
  identifier), `systemName` (`"iOS"` / `"iPadOS"`) and `systemVersion`. `UIDevice` is main-thread-only, so
  off the main thread the OS fields fall back to `"Unknown"`. `is_simulator` is definitive from the `sim`
  target ABI, with the simulator's `SIMULATOR_UDID` / `SIMULATOR_DEVICE_NAME` env as a fallback.
- **Windows**: `RtlGetVersion` (ntdll) reports the actual running version (`major.minor.build`), unlike
  the Win32 `GetVersionExW`, which lies for Windows 8+ without a compatibility manifest. `system_name` is
  `"Windows"`; `model` is a best-effort `"PC"` (there is no cheap portable hardware-model source). Written
  blind, like the rest of the winui backend.
- **Linux**: no single portable API is guaranteed, so the crate reads the two files every desktop distro
  provides: `/etc/os-release` (`NAME` / `PRETTY_NAME` for `system_name`, `VERSION_ID` for
  `system_version`) and the DMI node `/sys/devices/virtual/dmi/id/product_name` for `model` (falling back
  to `"Linux"`). `is_simulator` is always `false`.
- **HarmonyOS**: the native `deviceinfo.h` C API: `OH_GetOSFullName()` (e.g. `"OpenHarmony-5.0.0.0"`,
  whose head becomes `system_name`), `OH_GetDisplayVersion()` (`system_version`, falling back to the
  version tail of the full name) and `OH_GetProductModel()` (`model`). No permission required.
- **Android**: `android.os.Build.*`, read through a Java shim (`model` from `MANUFACTURER` + `MODEL`,
  `system_name` `"Android"`, `system_version` `VERSION.RELEASE`). `is_simulator` is a heuristic over the
  standard emulator fingerprints (`FINGERPRINT` contains `generic`/`emulator`, `PRODUCT` contains `sdk`,
  `HARDWARE` is `goldfish`/`ranchu`, …). `Build.*` are static fields, so unlike the battery/network/
  clipboard parts, this one needs no `Context`, only the attached JVM. No permission required.

## What it shows about the extension system

Like `day-part-battery` and `day-part-network`, this is a headless external crate: it has no UI Piece
and registers nothing into any backend's `RENDERERS` slice. On Android it rides the Day runtime only
for its attached JNIEnv (via `day-android`); the Java shim is staged into the app's Gradle build by
`[package.metadata.day.android]` without touching any core day crate, and it contributes no manifest
permission. On every other platform the crate is fully day-independent: pure native FFI or std.
