---
title: Device capabilities (parts)
description: "Headless platform capabilities (battery, clipboard, preferences, sensors, network, permissions, location) as ordinary crates with per-OS implementations."
order: 25
section: Guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

A **part** is Day's name for a headless platform capability: a set of functions with no UI
whose implementation differs per operating system. Battery level, the clipboard, preference
storage, and sensors are the things every cross-platform app eventually needs and every platform
spells differently.

Parts are ordinary crates. You add one to `Cargo.toml`, call plain functions, and the right
platform code runs because each function's body dispatches on `#[cfg(target_os)]`: IOKit on
macOS, `BatteryManager` over JNI on Android, sysfs on Linux, Win32 on Windows. There is no
plugin registry or runtime lookup; the target selects the implementation at compile time.

## The catalog

| Crate | What it does | Reference |
|---|---|---|
| `day-part-battery` | charge level and charging state | [battery](/docs/internal/battery) |
| `day-part-clipboard` | read/write the system clipboard (text) | [clipboard](/docs/internal/clipboard) |
| `day-part-prefs` | small key-value preference storage in the platform's conventional location | [prefs](/docs/internal/prefs) |
| `day-part-fs` | app-local file storage: read, write, remove, list (sync and async) | [fs](/docs/internal/fs) |
| `day-part-local-notify` | local notifications: post now or schedule, channels, tap-to-route | [notify](/docs/internal/notify) |
| `day-part-network` | connectivity status | [network](/docs/internal/network) |
| `day-part-deviceinfo` | device model, OS version | [deviceinfo](/docs/internal/deviceinfo) |
| `day-part-sensors` | accelerometer and friends, as a live stream | [sensors](/docs/internal/sensors) |
| `day-part-http` | HTTP through each platform's own networking stack | [http](/docs/internal/http) |
| `day-part-permissions` | ask the OS for the camera, location, notifications … and declare them at build time | [permissions](/docs/internal/permissions) |
| `day-part-location` | the device's position, once or as a live stream | [location](/docs/internal/location) |
| `day-part-haptics` | haptic feedback | [haptics](/docs/internal/haptics) |
| `day-part-timezone` | the wall clock (works on web too) and DST-correct IANA time-zone offsets | [timezone](/docs/internal/timezone) |
| `day-part-speech` | text to speech through each platform's own voice | [speech](/docs/internal/speech) |

## Using parts

The APIs are small. Here are some examples, verbatim from the crates:

```rust
// Battery
if let Some(b) = day_part_battery::status() {
    println!("{:?}, {:?}%", b.state, b.percent());   // Charging, Some(80)
}

// Clipboard
day_part_clipboard::set_text("hello");
let text = day_part_clipboard::get_text();           // Option<String>

// Preferences — strings in, strings out, stored where the platform expects
day_part_prefs::set("theme", "dark");
let theme = day_part_prefs::get("theme");            // Option<String>
```

Wiring a part into UI is the usual reactive pattern (read into a signal, bind the signal):

```rust
let battery = Signal::new(day_part_battery::status());

column((
    label(move || match battery.get() {
        Some(b) => format!("{}%", b.percent().unwrap_or(0)),
        None => tr("battery_unknown").format(),
    }),
    button(tr("refresh")).action(move || battery.set(day_part_battery::status())),
))
```

Returns are `Option`/`bool` rather than panics: a desktop without a battery reports `None`, a
denied clipboard read reports `None`, and your UI decides what that means. Check each part's
reference page for the per-platform support matrix; not every capability exists everywhere, and
each function's reference lists its per-platform support.

## Writing your own

When you need a platform API Day doesn't cover (Bluetooth, a payment SDK, notification badges),
you write a part. The pattern scales from trivial to involved:

- Pure-Rust platforms are a `#[cfg]` branch and a system crate (`objc2` on Apple, `windows` on
  Windows, sysfs/D-Bus on Linux).
- Android usually needs a small Java shim; a part can carry its own Java sources, Android
  resources, Gradle dependencies, ProGuard keep rules, and even manifest components (a
  `BroadcastReceiver` for a scheduled notification), all declared in Cargo metadata and
  aggregated into the app's Gradle project by `day build`, so the scaffold needs no manual edits.
- The same channel covers the other platforms: system frameworks and Swift for iOS and macOS
  (`[package.metadata.day.ios]` / `[package.metadata.day.macos]`), ArkTS sources for HarmonyOS.
- Or write the platform half **inline in your Rust file**, one arm per platform, and let the build
  generate both sides of the call (see below).
- Permissions a part needs (say, vibration) are declared in the part's metadata and merged into
  each platform's manifest the same way.

`day new part my-part` scaffolds the whole shape with per-OS stubs. The
[part tutorial](/docs/tutorial-part) walks through a complete real example (a battery part with
six platform implementations) and is the best template for your own.

### Foreign code, inline

A part whose platform half is a *function* rather than a directory of shims can declare it once in
Rust and implement it per platform in the language that platform speaks, in the same file:

```rust
day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        fn speak_native(text: &str) -> Result<(), day_bridge::Error>;
    }

    #[day_bridge::impl(swift, platforms = [ios, macos])]
    swift!(
        prelude = r#"
            import AVFoundation
        "#,
        body = r#"
            func speak_native(text: String) throws { … }
        "#,
    );

    #[day_bridge::impl(rust, platforms = [other])]
    fn speak_native(_text: &str) -> Result<(), day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }
}
```

The build generates the Swift adapter, the JNI binding, the ES module, or the C translation unit,
plus the Rust that calls it. The crate still compiles with plain `cargo test` on a machine
with none of those toolchains, because the last arm answers everywhere else.
`day-part-speech` carries six languages in one file this way; the
[bridge reference](/docs/internal/bridge) is the contract, and
[speech](/docs/internal/speech) is the worked example.

Parts are for *headless* capabilities only. The moment your capability needs to render
something, it's a [piece](/docs/extending), and a different set of tools applies.
