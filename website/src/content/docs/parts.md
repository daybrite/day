---
title: Device capabilities (parts)
description: "Headless platform capabilities — battery, clipboard, preferences, sensors, network, permissions, location — as ordinary crates with per-OS implementations."
order: 25
section: Guides
---

A **part** is Day's name for a headless platform capability: no UI, just functions whose
implementation differs per operating system. Battery level, the clipboard, preference storage,
sensors: the things every cross-platform app eventually needs and every platform spells
differently.

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

## Using parts

The APIs are small on purpose. Some examples, verbatim from the crates:

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
each function's reference lists per-platform support rather than implying uniform coverage.

## Writing your own

When you need a platform API Day doesn't cover (Bluetooth, a payment SDK, notification badges),
you write a part. The pattern scales from trivial to involved:

- Pure-Rust platforms are a `#[cfg]` branch and a system crate (`objc2` on Apple, `windows` on
  Windows, sysfs/D-Bus on Linux).
- Android usually needs a small Java shim; a part can carry its own Java sources, Android
  resources, Gradle dependencies, ProGuard keep rules, and even manifest components (a
  `BroadcastReceiver` for a scheduled notification), all declared in Cargo metadata and
  aggregated into the app's Gradle project by `day build` — no manual scaffold edits.
- The same channel covers the other platforms: system frameworks and Swift for iOS and macOS
  (`[package.metadata.day.ios]` / `[package.metadata.day.macos]`), ArkTS sources for HarmonyOS.
- Permissions a part needs (say, vibration) are declared in the part's metadata and merged into
  each platform's manifest the same way.

`day new part my-part` scaffolds the whole shape with per-OS stubs. The
[part tutorial](/docs/tutorial-part) walks through a complete real example (a battery part with
six platform implementations) and is the best template for your own.

One boundary worth respecting: parts are for *headless* capabilities. The moment your capability
needs to render something, it's a [piece](/docs/extending), and a different set of tools applies.
