---
title: "Tutorial: A part with native platform code"
description: Build a headless capability crate (a battery monitor) with a shared Rust API and per-platform native implementations (Rust FFI, Android Java, and more), selected by cfg. Includes how to contribute a Java shim and iOS framework to the app build.
order: 43
section: Extend
---

Day apps are more than pixels. Sooner or later you want to read the battery, check network
connectivity, fire a haptic tap, or reach some other device capability, and every platform
exposes it through a different native API, often in a different language. A **part** is Day's answer:
a small headless crate that presents one flat cross-platform Rust API and picks the right
native implementation per target.

This tutorial builds `day-part-battery` from the ground up. It is a real crate in the Day workspace
at [`parts/day-part-battery/`](https://github.com/daybrite/day/tree/main/parts/day-part-battery). By the end you will have a `day_part_battery::status()` function that
reads the battery through IOKit on macOS, `UIDevice` on iOS, a Java `BatteryManager` shim on Android,
sysfs on Linux, `GetSystemPowerStatus` on Windows, and a native `.so` on HarmonyOS. Each uses
whatever language fits that platform, and all of them sit behind one signature.

## 1. What a part is (and when to build one)

A **part** is a *headless capability crate*. It has:

- **No UI.** It renders nothing, registers no renderer, and never touches a toolkit.
- **A flat cross-platform API.** One or two free functions like `status() -> Option<BatteryStatus>`.
- **Per-OS native implementations**, selected at compile time by `#[cfg(target_os = "…")]` rather
  than a Cargo feature, because a battery is an OS concern rather than a toolkit one.

Contrast that with a **piece**, which is a reusable UI widget (a `combo_box`, a `web_view`) that
*does* register a per-toolkit renderer. Pieces live in [`pieces/`](https://github.com/daybrite/day/tree/main/pieces); parts live in [`parts/`](https://github.com/daybrite/day/tree/main/parts), the non-UI
corollary. The rule of thumb:

- Building a **visible control** backed by a native widget? Write a **piece**; see
  [the piece tutorial](/docs/internal/extending) and [`pieces/day-piece-searchfield`](https://github.com/daybrite/day/tree/main/pieces/day-piece-searchfield).
- Exposing a **device service** with no UI of its own? Write a **part**.

A part reuses the same build-contribution channel pieces use (the `[package.metadata.day.*]` keys
that fold native assets into the app build) but registers nothing into any `RENDERERS` slice. The
mechanism that stages a piece's Android Java or iOS framework works the same way for a headless
crate. You get native-code contribution without touching any core Day crate.

## 2. Scaffold: the flat API and the cfg/path dispatch

Start with the scaffolder. `day new part` generates the whole layout below: `Cargo.toml`, a
`src/lib.rs` with the `#[cfg]`/`#[path]` dispatch already wired (including the mandatory
`None`-returning fallback), a stub `src/<os>.rs` per platform, an `examples/` runner, and, when you
target Android, the `android/java/.../Day<Name>.java` shim plus the `[package.metadata.day.android]`
block:

```bash
day new part day-part-battery --platforms macos,ios,android,linux,windows
```

Omit `--platforms` to get that same default set. As with pieces, the crate builds immediately against
a remote Day release; add `--local <path>` to point at a local Day checkout instead. The sections
below explain each generated file.

A part is an ordinary library crate. Here is the whole shape:

```
parts/day-part-battery/
├── Cargo.toml
├── android/java/dev/daybrite/day/battery/DayBattery.java   # Android backend (Java)
├── examples/battery.rs                                     # a plain `main`, no Day at all
└── src/
    ├── lib.rs        # the flat API + a #[cfg]/#[path] index of per-OS impls
    ├── macos.rs      # IOKit (Rust → C FFI)
    ├── ios.rs        # UIDevice (Rust → objc2)
    ├── android.rs    # calls the Java shim via JNI
    ├── linux.rs      # /sys/class/power_supply (pure std)
    ├── windows.rs    # GetSystemPowerStatus (Rust → C FFI)
    └── ohos.rs       # libohbattery_info.so (Rust → C FFI)
```

### The public surface

`lib.rs` defines a plain data struct and a single entry point. Nothing platform-specific leaks into
the API. Callers see the same types everywhere.

```rust
/// A snapshot of the device battery.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BatteryStatus {
    /// Charge fraction in `0.0..=1.0`, or `None` if the level is unknown (e.g. a simulator).
    pub level: Option<f32>,
    /// Charging / discharging / …
    pub state: BatteryState,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BatteryState {
    Charging,
    Discharging,
    Full,
    NotCharging,
    #[default]
    Unknown,
}

impl BatteryStatus {
    /// The charge level as a whole percentage `0..=100`, if known.
    pub fn percent(&self) -> Option<u8> {
        self.level.map(|l| (l.clamp(0.0, 1.0) * 100.0).round() as u8)
    }
    pub fn is_charging(&self) -> bool {
        matches!(self.state, BatteryState::Charging)
    }
}

/// Read the current battery status via the platform's native API. Returns `None` when there is no
/// battery API for the platform, or the device has no battery.
pub fn status() -> Option<BatteryStatus> {
    imp::status()
}
```

The public `status()` is a one-liner that forwards to `imp::status()`. `imp` is a different module
on every platform; that indirection is the whole trick.

### The cfg/path dispatch (and the mandatory fallback)

Below the API, `lib.rs` binds `imp` to exactly one file per target with `#[cfg]` + `#[path]`. Each
file exposes the same private `fn status() -> Option<BatteryStatus>`:

```rust
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;

#[cfg(target_os = "ios")]
#[path = "ios.rs"]
mod imp;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;

// Desktop/embedded Linux reads sysfs; HarmonyOS (also `target_os = "linux"`) sandboxes that away,
// so it uses its own native battery API instead.
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
#[path = "linux.rs"]
mod imp;

#[cfg(all(target_os = "linux", target_env = "ohos"))]
#[path = "ohos.rs"]
mod imp;

#[cfg(target_os = "android")]
#[path = "android.rs"]
mod imp;

// Any other platform: no native battery API. THIS ARM IS MANDATORY.
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "android"
)))]
mod imp {
    pub fn status() -> Option<super::BatteryStatus> {
        None
    }
}
```

Two details make this reliable:

- **HarmonyOS is `target_os = "linux"`** but sandboxes `/sys` away, so it is disambiguated with
  `target_env = "ohos"`: one arm for desktop Linux, one for OpenHarmony. This `target_os` +
  `target_env` pattern is the standard way to split a shared OS.
- **The catch-all `#[cfg(not(any(...)))]` fallback is not optional.** Without it, `status()` would
  fail to compile on any target you did not enumerate (a WASM build, a BSD, a bare `cargo check` on an
  exotic host). The fallback module returns `None` so the crate compiles everywhere and simply
  reports "no battery API here." A part that can panic or fail to build on an unexpected target is a
  broken part; the fallback is what makes the API's `Option` promise true.

Every arm is mutually exclusive, so exactly one `imp` is compiled into any given binary. There is no
runtime dispatch and no dead code: the AppKit build contains only the IOKit path, and the Android
build only the JNI path.

### Cargo.toml: target-gated dependencies

Because each platform needs different crates (and most need none), dependencies are declared
per-target so they are only pulled in where they compile:

```toml
[package]
name = "day-part-battery"
version.workspace = true
edition.workspace = true

# No shared [dependencies]; the flat API is pure std.
[dependencies]

[target.'cfg(target_os = "macos")'.dependencies]
core-foundation = "0.10"
core-foundation-sys = "0.8"

[target.'cfg(target_os = "ios")'.dependencies]
objc2 = "0.6"
objc2-ui-kit = { version = "0.3", features = ["UIDevice"] }

# Android reads BatteryManager through a Java shim + day-android's cached JVM/Context. This is the one
# platform where the headless crate rides on the Day runtime (like the pieces' Android backends).
[target.'cfg(target_os = "android")'.dependencies]
day-android = { workspace = true }
```

Note there is no `[features]` table. A part has no backends to toggle; the target is the
selector. (Linux and Windows need no crates at all: they use pure std or raw FFI.)

## 3. A native implementation per platform

This is where a part earns its keep. Every `imp::status()` has the same signature, but behind it each
platform speaks its own language and its own API. Here are four of them in detail.

### macOS: Rust calling C (IOKit, via `#[link]`)

macOS has no crate wrapping the power API, so `macos.rs` declares the three IOKit functions it needs
with a plain `extern "C"` block and force-links the framework with `#[link(name = "IOKit", kind =
"framework")]`. CoreFoundation (via the `core-foundation` crate) handles the dictionary access:

```rust
use super::{BatteryState, BatteryStatus};
use core_foundation_sys::base::{CFRelease, CFTypeRef};
use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation_sys::dictionary::CFDictionaryRef;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
    fn IOPSCopyPowerSourcesList(blob: CFTypeRef) -> CFArrayRef;
    fn IOPSGetPowerSourceDescription(blob: CFTypeRef, ps: CFTypeRef) -> CFDictionaryRef;
}

pub fn status() -> Option<BatteryStatus> {
    unsafe {
        let blob = IOPSCopyPowerSourcesInfo();
        if blob.is_null() { return None; }
        // walk IOPSCopyPowerSourcesList(blob), read "Current Capacity" / "Max Capacity" /
        // "Is Charging" / "Power Source State" out of each source's CFDictionary …
        // then CFRelease(blob); Copy-rule ownership means we release what we copied.
        unimplemented!()
    }
}
```

`day-part-network` shows the same plain-C style even more minimally: its shared `src/apple.rs`
declares SystemConfiguration's reachability API with two `extern "C"` functions and a locally-declared
`sockaddr_in`, with no crates at all:

```rust
#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCNetworkReachabilityCreateWithAddress(
        allocator: *const c_void,
        address: *const SockaddrIn,
    ) -> *const c_void;
    fn SCNetworkReachabilityGetFlags(target: *const c_void, flags: *mut u32) -> u8;
}
```

### iOS: Rust calling Objective-C (UIDevice, via objc2)

iOS has ready-made bindings, so `ios.rs` uses the `objc2-ui-kit` crate directly; there is no
hand-rolled FFI. `UIDevice` is main-thread-only, which the `MainThreadMarker` encodes at the type
level:

```rust
use super::{BatteryState, BatteryStatus};
use objc2::MainThreadMarker;
use objc2_ui_kit::{UIDevice, UIDeviceBatteryState};

pub fn status() -> Option<BatteryStatus> {
    let mtm = MainThreadMarker::new()?;          // None off the main thread → status() is None
    let device = UIDevice::currentDevice(mtm);
    device.setBatteryMonitoringEnabled(true);
    let raw = device.batteryLevel();             // 0.0–1.0, or -1 when unknown
    let level = if raw < 0.0 { None } else { Some(raw) };
    let state = match device.batteryState() {
        UIDeviceBatteryState::Charging => BatteryState::Charging,
        UIDeviceBatteryState::Unplugged => BatteryState::Discharging,
        UIDeviceBatteryState::Full => BatteryState::Full,
        _ => BatteryState::Unknown,
    };
    Some(BatteryStatus { level, state })
}
```

The macOS and iOS impls look nothing alike (one is raw C FFI, the other a typed Objective-C
binding), yet both satisfy `fn status() -> Option<BatteryStatus>`. That is the point: the
caller never knows or cares.

### Android: Rust calling Java (a `BatteryManager` shim over JNI)

Android is special: reading `BatteryManager` cleanly wants a `Context` and a sticky broadcast, which is
far easier in Java than through raw JNI. So the part carries its own small Java class and calls it
over the bridge. This is the one platform where a part rides on the Day runtime; it borrows the JVM
and `Context` that `day-android` already caches.

The Java shim reads the sticky `ACTION_BATTERY_CHANGED` intent and packs the reading into a `long` so
it crosses the JNI boundary as a single primitive (no object marshalling):

```java
package dev.daybrite.day.battery;

import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.os.BatteryManager;

import dev.daybrite.day.bridge.DayBridge;   // day-android's public surface

public final class DayBattery {
    private DayBattery() {}

    /** Packs (state << 8) | levelByte; levelByte 0..100 or 255 = unknown. */
    public static long read() {
        Context ctx = DayBridge.ctx;                 // the cached app Context
        int level = -1, state = 0;
        if (ctx != null) {
            IntentFilter filter = new IntentFilter(Intent.ACTION_BATTERY_CHANGED);
            Intent intent = ctx.registerReceiver(null, filter);   // sticky broadcast, no receiver
            if (intent != null) {
                int lvl = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1);
                int scale = intent.getIntExtra(BatteryManager.EXTRA_SCALE, -1);
                if (lvl >= 0 && scale > 0) level = Math.round(lvl * 100f / scale);
                switch (intent.getIntExtra(BatteryManager.EXTRA_STATUS,
                        BatteryManager.BATTERY_STATUS_UNKNOWN)) {
                    case BatteryManager.BATTERY_STATUS_CHARGING:     state = 1; break;
                    case BatteryManager.BATTERY_STATUS_DISCHARGING:  state = 2; break;
                    case BatteryManager.BATTERY_STATUS_FULL:         state = 3; break;
                    case BatteryManager.BATTERY_STATUS_NOT_CHARGING: state = 4; break;
                    default:                                         state = 0;
                }
            }
        }
        long levelByte = (level < 0) ? 255 : Math.min(100, level);
        return ((long) state << 8) | levelByte;
    }
}
```

The Java uses only `day-android`'s public surface (`DayBridge.ctx`). The Rust side calls the static
method through `day-android`'s re-exported `jni`, using `with_env` to grab the attached `JNIEnv`, then
unpacks the `long`:

```rust
use super::{BatteryState, BatteryStatus};
use day_android::with_env;

const BATTERY_CLASS: &str = "dev/daybrite/day/battery/DayBattery";

pub fn status() -> Option<BatteryStatus> {
    let packed: i64 = with_env(|env| {
        env.call_static_method(BATTERY_CLASS, "read", "()J", &[])   // ()J = () -> long
            .ok()
            .and_then(|v| v.j().ok())
    })?;

    let level_byte = (packed & 0xFF) as u8;
    let level = if level_byte == 255 { None } else { Some(level_byte as f32 / 100.0) };
    let state = match (packed >> 8) & 0xFF {
        1 => BatteryState::Charging,
        2 => BatteryState::Discharging,
        3 => BatteryState::Full,
        4 => BatteryState::NotCharging,
        _ => BatteryState::Unknown,
    };
    Some(BatteryStatus { level, state })
}
```

### Linux: pure Rust std (sysfs)

There is no FFI and no crate here. The kernel publishes power supplies under
`/sys/class/power_supply/<name>/`, so `linux.rs` reads a few files with `std::fs`:

```rust
use super::{BatteryState, BatteryStatus};
use std::{fs, path::Path};

pub fn status() -> Option<BatteryStatus> {
    for entry in fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let dir = entry.path();
        if fs::read_to_string(dir.join("type")).unwrap_or_default().trim() != "Battery" {
            continue;
        }
        let level = fs::read_to_string(dir.join("capacity")).ok()
            .and_then(|s| s.trim().parse::<f32>().ok())
            .map(|c| (c / 100.0).clamp(0.0, 1.0));
        let state = match fs::read_to_string(dir.join("status")).unwrap_or_default().trim() {
            "Charging" => BatteryState::Charging,
            "Discharging" => BatteryState::Discharging,
            "Full" => BatteryState::Full,
            "Not charging" => BatteryState::NotCharging,
            _ => BatteryState::Unknown,
        };
        return Some(BatteryStatus { level, state });
    }
    None
}
```

### Windows & HarmonyOS: more C FFI

For completeness, the two remaining targets are both raw C-ABI FFI, in the same style as macOS:

- **Windows** (`windows.rs`) links `kernel32` and calls `GetSystemPowerStatus`, filling a `#[repr(C)]
  SYSTEM_POWER_STATUS` struct. It uses no crate, was written blind against the Win32 docs, and is
  compiled only on the Windows target.

  ```rust
  #[link(name = "kernel32")]
  unsafe extern "system" {
      fn GetSystemPowerStatus(status: *mut SystemPowerStatus) -> c_int;
  }
  ```

- **HarmonyOS** (`ohos.rs`, gated on `target_env = "ohos"`) links the native BasicServicesKit library
  and calls its two C functions directly; there is no ArkTS bridge and no Day runtime involved:

  ```rust
  #[link(name = "ohbattery_info")]
  unsafe extern "C" {
      fn OH_BatteryInfo_GetCapacity() -> c_int;
      fn OH_BatteryInfo_GetPluggedType() -> c_int;
  }
  ```

Six platforms, three different interop styles (raw C FFI, typed objc2, JNI-to-Java), and one
pure-std path, all funnelling into the same `Option<BatteryStatus>`.

## 4. Contribute native artifacts to the app build

The Rust FFI paths (macOS, Windows, HarmonyOS, iOS-objc2, Linux) need nothing extra; `cargo` links
them. But two platforms need assets folded into the app's native build: Android needs the `.java`
file compiled and (for some parts) a manifest permission; iOS needs certain system frameworks linked.
A part declares both in its own `Cargo.toml`, and `day build` merges them into the app with no
edits to any core Day crate, the CLI, or the app scaffold.

### Android: staging the Java shim and a permission

```toml
[package.metadata.day.android]
java = ["android/java"]                                   # → Gradle java srcDirs
permissions = ["android.permission.ACCESS_NETWORK_STATE"] # → <uses-permission> overlay (if needed)
```

`day-part-battery` needs no permission (the sticky battery broadcast is unrestricted), so it declares
only `java = ["android/java"]`. `day-part-network`, whose `ConnectivityManager` call *does* require
`ACCESS_NETWORK_STATE`, adds the `permissions` line above.

When you run `day build -p android-widget`, the CLI runs `cargo metadata`, walks the app's entire
dependency closure, and collects every part's and piece's `[package.metadata.day.android]` blocks into
`build/day/android/day-pieces.json`, plus a generated overlay manifest for the permissions. The app's
checked-in Gradle scaffold reads that file generically (a loop over the JSON, with no per-part
entries) and adds each Java source dir and each `<uses-permission>`. Add a part to your `Cargo.toml`
and its Java appears in the build; there is nothing else to wire.

### iOS: linking a system framework

```toml
[package.metadata.day.ios]
frameworks = ["SystemConfiguration"]
```

This is what `day-part-network` declares (its `apple.rs` drives SystemConfiguration). Why is it needed
when the Rust source already has `#[link(name = "SystemConfiguration", kind = "framework")]`? Because
that Rust link directive is only honored when cargo drives the final link, i.e. on the macOS
desktop build. On iOS, `xcodebuild` links the Rust staticlib and does not read Rust link metadata, so
the app itself must link the framework. `day build -p ios-uikit` generates a local SwiftPM package
(`build/day/ios/DayPieces`) whose `linkerSettings` list every part's declared frameworks; the app's
one checked-in `.xcodeproj` depends on that package. So an iOS framework dependency is, again, pure
`Cargo.toml` data. You never edit the `.xcodeproj`.

`day-part-battery` itself declares no `[package.metadata.day.ios]`: it uses `objc2-ui-kit`, and UIKit
is auto-linked by the iOS SDK. You only need the `frameworks` key for a system framework that is not
linked by default (SystemConfiguration, WebKit, …).

This is the same contribution channel [`pieces/day-piece-searchfield`](https://github.com/daybrite/day/tree/main/pieces/day-piece-searchfield) (Android Java + Gradle deps) and
[`pieces/day-piece-webview`](https://github.com/daybrite/day/tree/main/pieces/day-piece-webview) (a framework + a permission) use. A part is just a piece that skips the
renderer.

## 5. Use it

Any Rust code (inside a Day app or a plain binary) depends on the crate and calls the function:

```rust
fn main() {
    match day_part_battery::status() {
        Some(b) => println!(
            "battery: {:?}, {} charging={}",
            b.state,
            b.percent().map(|p| format!("{p}%")).unwrap_or("?".into()),
            b.is_charging(),
        ),
        None => println!("no battery API (or no battery) on this platform"),
    }
}
```

That is [`parts/day-part-battery/examples/battery.rs`](https://github.com/daybrite/day/blob/main/parts/day-part-battery/examples/battery.rs) verbatim: a `main` that uses no Day framework at
all, provable with `cargo run -p day-part-battery --example battery`. Inside a Day app you would bind
the reading into a `Signal` and drive a `label` or a `canvas` gauge with it, but the part itself knows
nothing about UI.

The safety contract is what makes a part pleasant to consume: on a target with no battery API
(or a device with no battery, or the iOS simulator), `status()` returns `None`. It never panics and
it always compiles, because of the mandatory fallback module from step 2. A test in the crate
enforces exactly this:

```rust
#[test]
fn status_does_not_panic() {
    let s = super::status();
    if let Some(b) = s {
        assert!(b.percent().is_none_or(|p| p <= 100));
    }
}
```

## 6. A practical note: let an LLM draft the language-specific shims

Covering six platforms sounds daunting because it means writing Objective-C, Java, and Win32 C
interop on top of Rust. In practice the hardest part is knowing which API to call. Once you do,
the shim is small and mechanical, exactly the kind of thing an LLM drafts well.

The recommended workflow:

1. **Write the Rust `imp` signature first.** `fn status() -> Option<BatteryStatus>` is the contract
   every platform must satisfy. Decide the wire format up front (the packed-`long` trick the Android
   shim uses is a good pattern: it keeps the JNI/FFI boundary to a single primitive).
2. **Ask an LLM to draft the native side** from a one-line description of the platform API: "a Java
   method that reads `BatteryManager` from a `Context` and returns `(state << 8) | level` as a
   `long`", or "a C call to `GetSystemPowerStatus` filling `SYSTEM_POWER_STATUS`", or "the
   Objective-C to read `UIDevice.batteryLevel`". These snippets are well represented in training data
   and models produce them reliably. The Windows and HarmonyOS impls in this crate were written
   blind (no Windows or Harmony host) precisely because the API call is small and well specified.
3. **Wire the FFI yourself.** Declare the `extern` block or the JNI `call_static_method`, unpack the
   value, and map it to your enum. This is the part where types and ownership rules matter, and where
   you want to read carefully, but it is short.

Split this way, "support one more platform" becomes: draft a ~30-line shim, add one `#[cfg]/#[path]`
arm, and (if it is Android or iOS) one line of `Cargo.toml` metadata. The per-platform sprawl that
makes cross-platform capability code intimidating is mostly boilerplate an LLM is good at, leaving you
to own the small, load-bearing FFI seam.

---

For the full source, see [`parts/day-part-battery/`](https://github.com/daybrite/day/tree/main/parts/day-part-battery) in the Day repo, and
[extending.md](/docs/internal/extending) for the shared contribution mechanism that parts and pieces both ride.
[`parts/day-part-network`](https://github.com/daybrite/day/tree/main/parts/day-part-network) (SystemConfiguration + an iOS `frameworks` link + an Android permission) and
[`parts/day-part-haptics`](https://github.com/daybrite/day/tree/main/parts/day-part-haptics) (objc2 feedback generators + an Android `Vibrator` shim) are two more parts
to read as templates.
