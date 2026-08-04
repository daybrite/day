# Location (headless capability crate)

> **Status: implemented** as `day-part-location` (in `parts/`, the headless counterpart of
> `pieces/`). It's a headless day-ecosystem crate (no UI Piece): the device's position, once or as a
> live stream, through each platform's own location service. Any Rust code can depend on it and call
> `day_part_location::watch(...)`. Host build/clippy/tests and the iOS-simulator, Android, wasm32 and
> HarmonyOS cross-compiles are verified; a real fix needs a device or a simulator with a location
> injected (Xcode's Features → Location, the Android emulator's extended controls), which has not yet
> been exercised.

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
| `is_available() -> bool` | whether this target has a location API Day can reach — a platform statement, not a permission one |
| `current(acc, cb)` / `current_future(acc)` | one fix |
| `watch(acc, cb) -> Watch` | a live stream; dropping the handle stops the platform's updates |

`Fix` carries `latitude`, `longitude`, and `Option`s for `altitude`, `accuracy_m`,
`vertical_accuracy_m`, `speed_mps`, `course_deg` and `timestamp_ms`. **A field the platform did not
measure is `None`, never a plausible-looking zero**: "we don't know the altitude" must not read as
"sea level". `Accuracy` is `Coarse` / `Balanced` / `Best`; ask for the least you need, since higher
accuracy costs battery and takes longer to acquire.

The crate has no cargo features: platform selection is purely `#[cfg(target_os)]`, since location
depends on the OS, not on which widget toolkit is in use.
`parts/day-part-location/examples/location.rs` is a plain `main` that uses it with no Day framework
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

| OS | API | dependency |
|---|---|---|
| iOS / macOS | `CLLocationManager` + a delegate defined with `objc2::define_class!` | `objc2`, `objc2-foundation`, `[package.metadata.day.ios].frameworks = ["CoreLocation"]` |
| Android | `LocationManager.requestLocationUpdates` via the crate's own Java shim | `day-android` + `[package.metadata.day.android]` |
| Web | `navigator.geolocation.watchPosition` through the day-dom shim | `web.rs` (wasm32; needs the day-dom host page) |
| HarmonyOS | none — see below | — |
| Linux | none — GeoClue2 would need a D-Bus dependency this tree does not have | — |
| Windows | none yet — `Windows.Devices.Geolocation` is the future impl | — |

An unsupported target is not silent: `is_available()` answers `false` and a `watch` reports
`LocationError::Unavailable` **once**, so an app waiting for its first fix is never left hanging.

### Why not FusedLocationProviderClient on Android

The fused provider is the usual Android recommendation and the wrong dependency here: it ships in
Google Play services, which AOSP images and many emulators do not have, and it would add a Gradle
coordinate to every app linking this part. The platform `LocationManager` is always present. The
shim prefers GPS at `Accuracy::Best` and the network provider otherwise, and seeds the first update
from `getLastKnownLocation` so a fix appears immediately instead of waiting for the radio.

### HarmonyOS is not implemented

Location on HarmonyOS is an ArkTS API (`@kit.LocationKit`) with no NDK C surface, and there is no
`[package.metadata.day.ohos]` mechanism for a crate to contribute ArkTS. Saying so beats shipping a
stub that looks like an oversight. It could later ride the same ArkTS seam
[`day-part-permissions`](permissions.md) needs for its request path.

### Apple: fixes need a run loop

CoreLocation delivers to the run loop of the thread the manager was created on. In a Day app that is
the UI thread, and everything works. In a plain `main` or under `cargo test` there is no run loop, so
no fix is ever delivered; `is_available()` still answers `true`, because CoreLocation exists. The
example file says so at the top rather than looking broken.

Apple reports "not measured" as a NEGATIVE accuracy and `-1` for speed and course; those become
`None` rather than nonsense numbers. Android reports it with a `hasXxx()` companion, which the shim
passes across alongside the value.

## What it shows about the extension system

The crate registers nothing in any `RENDERERS` slice. It contributes its Android Java shim through
`[package.metadata.day.android]` and its `CoreLocation` link through `[package.metadata.day.ios]`,
with no edits to any core Day crate. It is the first **part** to define an Objective-C class
(`objc2::define_class!`), which pieces have always done. Its `permissions = []` entry is deliberately
empty: the app declares location in its own `Day.toml`. See [extending.md](extending.md).
