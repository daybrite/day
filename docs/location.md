---
title: "Location"
description: "One-shot and continuous location via day-part-location, with the permission flow attached."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Location

Find the device’s position once or follow it as it moves. `day-part-location` provides location
updates with selectable accuracy, so an app can balance precision and battery use. Request
[location permission](/docs/reference/permissions) separately before starting.

## Add to your app

Defined in [`day-part-location`](https://github.com/daybrite/day/tree/main/crates/day-part-location). Add this to your app’s `Cargo.toml`
`[dependencies]` table, at the same Day version as the rest of your app:

```toml
[dependencies]
day-part-location = "0.6"
```

The implementation is selected by the compilation target; no toolkit feature flags
or dependency entry in `Day.toml` are needed.

Declare the required capabilities and usage explanations in [Day.toml permissions](/docs/reference/permissions#declare-before-you-ask).

| Platforms / toolkits | Availability |
| --- | --- |
| macOS, iOS, Android, HarmonyOS, Web | Location APIs are present; permission and an available positioning source are required. Device delivery remains unverified. |
| Windows, Linux (GTK / Qt) | No location implementation. |

## Authoring

```rust
use day_part_location::Accuracy;

let watch = day_part_location::watch(Accuracy::Balanced, |fix| match fix {
    Ok(f) => println!("{:.5}, {:.5}", f.latitude, f.longitude),
    Err(e) => println!("location unavailable: {e}"),
});
// Updates arrive until `watch` is dropped.
```

| Function | Behavior |
|---|---|
| [`is_available() -> bool`](https://daybrite.dev/api/day_part_location/fn.is_available.html) | whether this target has a location API Day can reach (a platform statement, not a permission one) |
| [`current(acc, cb)`](https://daybrite.dev/api/day_part_location/fn.current.html) / [`current_future(acc)`](https://daybrite.dev/api/day_part_location/fn.current_future.html) | one fix |
| [`watch(acc, cb) -> Watch`](https://daybrite.dev/api/day_part_location/fn.watch.html) | a live stream; dropping the handle stops the platform's updates |

`Fix` carries `latitude`, `longitude`, and `Option`s for `altitude`, `accuracy_m`,
`vertical_accuracy_m`, `speed_mps`, `course_deg` and `timestamp_ms`. **A field the platform did not
measure is `None`, never a plausible-looking zero**: "we don't know the altitude" must not read as
"sea level". `Accuracy` is `Coarse` / `Balanced` / `Best`; ask for the least you need, since higher
accuracy costs battery and takes longer to acquire.

The crate has no cargo features: platform selection is purely `#[cfg(target_os)]`, since location
depends on the OS, not on which widget toolkit is in use.
`crates/day-part-location/examples/location.rs` is a plain `main` that uses it with no Day framework
at all.

Callbacks run on an unspecified thread (the platform's delivery thread natively, the sole browser
thread on the web), so deliver into UI state with a `day_reactive::Setter`. In a Day app, bind the
handle to the page's scope so the subscription (and the GPS) ends with it:

```rust
let watch = day_part_location::watch(Accuracy::Balanced, move |f| set.set(f));
day_reactive::Scope::current().on_cleanup(move || drop(watch));
```

## Permissions are a separate crate

This crate never prompts. A platform denial arrives as `LocationError::PermissionDenied`, and the
app asks through [`day-part-permissions`](permissions.md) (`Permission::Location`), so neither
crate depends on the other, and an app that already has permission pays nothing for the machinery
that requests it. Location also needs the build-time declaration `[permissions]` generates; without
it, iOS terminates the app on first use.

On the **web** there is no separate request at all: the first `watchPosition` call *is* the prompt,
so whichever of the two crates runs first shows it.

## Per-platform native realization

Platform guidance: [Apple: Core Location](https://developer.apple.com/documentation/corelocation); [Android: location permissions](https://developer.android.com/develop/sensors-and-location/location/permissions).

| OS | API | dependency |
|---|---|---|
| iOS / macOS | [`CLLocationManager`](https://developer.apple.com/documentation/corelocation/cllocationmanager) + a delegate defined with `objc2::define_class!` | `objc2`, `objc2-foundation`, `[package.metadata.day.ios].frameworks = ["CoreLocation"]` |
| Android | [`LocationManager.requestLocationUpdates`](https://developer.android.com/reference/android/location/LocationManager) via the crate's own Java shim | `day-toolkit-material` + `[package.metadata.day.android]` |
| Web | [`navigator.geolocation.watchPosition`](https://www.w3.org/TR/geolocation/#watchposition-method) through the day-toolkit-dom shim | `web.rs` (wasm32; needs the day-toolkit-dom host page) |
| HarmonyOS | [`geoLocationManager.on('locationChange')`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-location-kit/js-apis-geoLocationManager.md) (Location Kit) via an ArkTS arm; see below | [`day-bridge`](/contributing/backend-contracts/bridge) (`src/ohos.rs`) |
| Linux | none; GeoClue2 would need a D-Bus dependency this tree does not have | — |
| Windows | none yet; [`Windows.Devices.Geolocation`](https://learn.microsoft.com/en-us/uwp/api/windows.devices.geolocation) is the future impl | — |

An unsupported target reports itself: [`is_available()`](https://daybrite.dev/api/day_part_location/fn.is_available.html) answers `false` and a [`watch`](https://daybrite.dev/api/day_part_location/fn.watch.html) reports
`LocationError::Unavailable` **once**, so an app waiting for its first fix is never left hanging.

### Why not FusedLocationProviderClient on Android

The fused provider is the usual Android recommendation, but it ships in
Google Play services, which AOSP images and many emulators do not have, and it would add a Gradle
coordinate to every app linking this part. The platform [`LocationManager`](https://developer.android.com/reference/android/location/LocationManager) is always present. The
shim prefers GPS at `Accuracy::Best` and the network provider otherwise, and seeds the first update
from `getLastKnownLocation` so a fix appears immediately instead of waiting for the radio.

### HarmonyOS: an ArkTS arm

Location on HarmonyOS is an ArkTS API (`@kit.LocationKit`) with no NDK C surface, so the crate
reaches it through a daybridge arm ([docs/bridge](bridge.md) "Streams"): `watch_native` subscribes to
`locationChange` at the accuracy's priority (`LOW_POWER`, `FIRST_FIX`, `ACCURACY`) and streams each
fix back as one line, a field the platform left out as `NaN`. The switch being off answers
`LocationError::Disabled` at once rather than waiting for a fix that cannot come; error 201 is
`PermissionDenied`, and the app asks through [`day-part-permissions`](permissions.md)
(`Permission::Location`). The OpenHarmony emulator has no positioning source, so it reports
`Disabled` or times out; a real fix needs a device.

### Apple: fixes need a run loop

CoreLocation delivers to the run loop of the thread the manager was created on. In a Day app that is
the UI thread, and everything works. In a plain `main` or under `cargo test` there is no run loop, so
no fix is ever delivered; [`is_available()`](https://daybrite.dev/api/day_part_location/fn.is_available.html) still answers `true`, because CoreLocation exists. The
example file says so at the top.

Apple reports "not measured" as a negative accuracy and `-1` for speed and course; those become
`None` rather than nonsense numbers. Android reports it with a `hasXxx()` companion, which the shim
passes across alongside the value.

> [!WARNING]
> **Location delivery.**
> Device and simulator delivery has not been verified. Test updates, permission changes,
> and unavailable location services on each target you ship.

## What it shows about the extension system

The crate registers nothing in any `RENDERERS` slice. It contributes its Android Java shim through
`[package.metadata.day.android]` and its `CoreLocation` link through `[package.metadata.day.ios]`,
with no edits to any core Day crate. It is the first **part** to define an Objective-C class
(`objc2::define_class!`), which pieces have always done. Its `permissions = []` entry is empty
because the app declares location in its own `Day.toml`. See [extending](extending.md).
