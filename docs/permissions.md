---
title: "OS permissions"
description: "Declaring, requesting, and explaining OS permissions across platforms, and what happens when you skip a step."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# OS permissions (headless capability crate)

> **Status: implemented** as `day-part-permissions` (in `parts/`, the headless counterpart of
> `pieces/`), plus the build-time declaration pipeline in the CLI. It's a headless day-ecosystem
> crate (no UI Piece): ask what the OS will do about a permission, and ask the OS itself. Any Rust
> code can depend on it and call `day_part_permissions::status(perm)`. Host build/clippy/tests, the
> iOS-device and iOS-simulator cross-compiles, the Android cross-compile with its Java shim compiled
> and dexed into a real APK, and the generated declarations for Android (verified in a built APK,
> `maxSdkVersion` included) and iOS (verified in a built `.app`, idempotent across builds) are all
> verified. The HarmonyOS writer is unit-tested against the real `module.json5` but has not yet run
> through a device build, and no runtime prompt has been exercised on a device.

## Declare before you ask

Every mobile OS requires a **build-time declaration** in addition to the runtime request,
and the failure modes are not symmetric:

- **iOS and macOS terminate the process** when it touches a gated API without the matching
  `NS…UsageDescription` key. There is no exception to catch: TCC terminates the process.
- **Android** reports an undeclared permission as [`Status::Restricted`]: a request returns denied
  in the same frame, with no dialog, and Settings offers nothing.
- **HarmonyOS** refuses the request outright.

So declare what you use in `Day.toml`, and `day build` generates each platform's manifest entry:

```toml
[permissions]
camera               = "Attach photos to your notes."
location-when-in-use = "Show stations near you."
notifications        = true          # needs no reason on any platform
```

The reason text is what the OS shows the user in its own prompt. `day lint` flags a permission your
code requests but Day.toml doesn't declare, so this is caught in CI rather than on a device.

## Authoring

```rust
use day_part_permissions::{Permission, Status, request, status};

if status(Permission::Camera) != Status::Granted {
    request(Permission::Camera, |s| println!("camera: {s}"));
}
```

| Function | Answers |
|---|---|
| `gate(perm) -> Gate` | whether this target gates the capability at all |
| `status(perm) -> Status` | what the OS will do if you use it now (never blocks) |
| `status_async` / `status_future` | the same, but authoritative where the platform is async |
| `can_prompt(perm) -> bool` | whether `request` would show a dialog |
| `should_show_rationale(perm)` | Android's "explain first" signal; `false` elsewhere |
| `request` / `request_future` | ask the OS |
| `request_many` / `request_many_future` | ask for several in one prompt sequence |
| `open_settings(perm) -> bool` | the remedy when the answer is already final |

The crate has no cargo features: platform selection is purely `#[cfg(target_os)]`, since consent
depends on the OS, not on which widget toolkit is in use.
`parts/day-part-permissions/examples/permissions.rs` is a plain `main` that uses it with no Day
framework at all.

## Two questions, two vocabularies

`gate()` answers a structural question and `status()` a live one, and keeping them apart
lets an ungated platform answer accurately:

| `Gate` | meaning |
|---|---|
| `Prompts` | the OS keeps a consent record and can show a dialog |
| `Ungated` | the capability exists and nothing gates it (desktop Linux, Windows) |
| `Absent` | no such capability here at all |

| `Status` | meaning |
|---|---|
| `Granted` | go ahead |
| `Prompt` | nobody has decided; `request` will show a dialog |
| `Denied` | the user said no; `can_prompt` says whether asking again can help |
| `Restricted` | policy forbids it, or it is missing from the merged manifest; neither asking nor Settings helps |
| `Unsupported` | `Gate::Absent` |
| `Unknown` | the platform answers only asynchronously and hasn't yet (web, and Apple notifications) |

On desktop Linux the camera has no permission gate, so `status` answers **`Granted`**: an app
asking "may I use the camera?" should proceed, and the real failure belongs at `open("/dev/video0")`.
The structural fact moves to `gate() == Ungated`. Two invariants are unit-tested:
`Ungated ⟹ Granted`, and `Absent ⟹ Unsupported`.

`Granted` is not a promise that the hardware exists. A laptop with no camera still answers
`Granted`, because no permission stands in the way; ask the capability's own part (e.g.
`day_part_sensors::is_available`) about hardware.

## Reasons are not a runtime parameter

`request(perm, reason)` is the natural API guess, but no platform accepts a reason at request time.
iOS and macOS read `NS…UsageDescription` from `Info.plist`; `requestPermissions(String[], int)` and
`requestPermissionsFromUser(context, string[])` take no text, and neither does `getUserMedia` or
`Notification.requestPermission`. The reason therefore lives in the declaration, where it reaches
the OS, and the runtime hands your app the two bits it needs to draw its own priming UI:
`should_show_rationale` and `can_prompt`.

As a consequence, no user-facing string crosses this crate's boundary, so the layering rule that
keeps `IntoText`/`LocalizedText` out of parts ([docs/extending.md](extending.md) §4) never has to be worked around.

## Per-platform native realization

| OS | check | request | dependency |
|---|---|---|---|
| iOS | `CLLocationManager`, `AVCaptureDevice`, `PHPhotoLibrary`, `UNUserNotificationCenter` (async-only), `CMMotionActivityManager` | the matching block-based `request…` | `objc2` + `block2`, `[package.metadata.day.ios].frameworks` |
| macOS | the same TCC APIs where they exist; no CoreMotion | same | shared `apple.rs` |
| Android | `Context.checkSelfPermission` + `getPackageInfo(GET_PERMISSIONS)` | `requestPermissions` from a headless `Fragment` | `day-android` + the crate's own Java shim |
| HarmonyOS | `OH_AT_CheckSelfPermission` | needs an ArkTS bridge, not yet built, so `can_prompt` is `false` | raw FFI |
| Web | `navigator.permissions.query` + a live `change` cache; `Notification.permission` is sync | the per-API call | the day-dom shim |
| Linux / Windows | constants | resolves immediately | — |

Two platform facts apply everywhere:

- **`request` is callback-and-future only; there is no blocking form.** The OS prompt is drawn by
  the very thread a blocking call would park, so it would deadlock by construction on every platform.
- **Dropping a `StatusFuture` does not dismiss the prompt.** No platform can dismiss its own
  permission dialog programmatically. Dropping stops you listening; the user's answer is still
  recorded, so the next `status()` is correct. Aborting a `day::task` that awaits one therefore
  leaves the dialog on screen.

### Android cannot tell "never asked" from "permanently denied"

It cannot without app-side state, and **Day keeps none**. A denied-but-declared permission with no
rationale flag is reported as `Prompt` either way. That is safe (asking after a permanent refusal
shows no dialog and resolves `Denied` immediately), but if your app needs the distinction, record it
yourself when you call `request`:

```rust
day_part_permissions::request(perm, move |s| {
    day::prefs::set("asked.camera", "1");
    // …
});
```

### macOS: the desktop dev loop cannot exercise permissions

`day launch -p macos-appkit` runs the bare binary, not a bundle. TCC reads usage descriptions from a
bundle's `Info.plist`, so an unbundled process is denied (or killed) regardless of what Day.toml
says. Only `day pack -p macos-appkit` produces a bundle that can be granted anything. The crate
guards every `UNUserNotificationCenter` call behind a bundle check for the same reason: touching it
unbundled aborts the process.

## What the declaration pipeline generates

| portable name | Android | iOS `Info.plist` | macOS | HarmonyOS |
|---|---|---|---|---|
| `location-when-in-use` | `ACCESS_FINE_LOCATION`, `ACCESS_COARSE_LOCATION` | `NSLocationWhenInUseUsageDescription` | + `NSLocationUsageDescription` | `APPROXIMATELY_LOCATION` + `LOCATION` |
| `location-always` | + `ACCESS_BACKGROUND_LOCATION` | that key **and** the when-in-use one (Apple suppresses the prompt without both) | + `NSLocationUsageDescription` | + `LOCATION_IN_BACKGROUND` |
| `camera` | `CAMERA` | `NSCameraUsageDescription` | same | `ohos.permission.CAMERA` |
| `microphone` | `RECORD_AUDIO` | `NSMicrophoneUsageDescription` | same | `ohos.permission.MICROPHONE` |
| `notifications` | `POST_NOTIFICATIONS` | none | none | none (a runtime call) |
| `photos` | `READ_MEDIA_IMAGES`, `READ_MEDIA_VIDEO`, `READ_EXTERNAL_STORAGE` capped at `maxSdkVersion=32` | `NSPhotoLibraryUsageDescription` | same | `READ_IMAGEVIDEO` ⚠ |
| `motion` | `ACTIVITY_RECOGNITION` | `NSMotionUsageDescription` | none (CoreMotion is iOS-only) | `ACTIVITY_MOTION` |

⚠ `ohos.permission.READ_IMAGEVIDEO` is a `system_basic` permission, which an app signed at the
default `normal` level cannot hold. Prefer `PhotoViewPicker`, which needs no permission at all.

The table lives in `day_build::permissions`, one source shared by the CLI's generators and this
crate's runtime, with a parity test pinning the Rust variant names so `day lint` can map a source
reference back to a declaration.

For anything outside the portable seven, use the raw tables:

```toml
[permissions.raw]
android = ["android.permission.READ_CONTACTS"]
ios     = { NSContactsUsageDescription = "Find friends who already use Day." }
ohos    = [{ name = "ohos.permission.READ_CONTACTS", reason = "Find friends.", when = "inuse" }]
```

A library can declare the machine-facing half itself (`[package.metadata.day.permissions] uses =
["camera"]`) but never the reason, which is app copy. A contribution with no reason in the app's
Day.toml is a **hard build error** on iOS and HarmonyOS, naming the crate and the lines to paste.

### Where each file is written, and what Day owns in it

- **Android** — `build/day/android/day-pieces-manifest.xml`, gitignored and regenerated, merged by
  AGP. That filename is a compatibility surface: it is baked into every scaffold `day new` has
  generated, and a source set has one manifest slot, so it is widened, never moved.
- **iOS/macOS** — the checked-in `platform/ios/Runner/Info.plist`, edited in place. Day owns exactly
  the keys in the table above plus your `[permissions.raw]` keys; every other byte is preserved, so
  the diff shows only what changed, and a hand-added key Day doesn't model is never touched. Two
  consecutive builds produce a byte-identical file. This is an exception to "aggregation
  never mutates the scaffolds" (DESIGN §15.2), because the alternative broke `⌘R` in Xcode.
- **HarmonyOS** — a marker region in `module.json5` (`// day:permissions-begin` … `-end`), inserted
  once on an older scaffold and replaced thereafter, plus `day_perm_reason_*` entries in
  `string.json`. Region editing rather than JSON5 parsing, because a round-trip would delete the
  file's comments.

## `day lint`

| code | fires when |
|---|---|
| `day::lint::undeclared-permission` | code requests `Permission::X` that Day.toml doesn't declare |
| `day::lint::missing-reason` | a declared permission that needs a reason has none |
| `day::lint::unused-permission` | declared, referenced by nothing (a warning — over-declaring gets apps rejected) |
| `day::lint::stale-manifest` | the checked-in `Info.plist` disagrees with Day.toml; run `day build -p ios-uikit` |

## What it shows about the extension system

The crate registers nothing in any `RENDERERS` slice. Its Android half is bundled with it and folded
into the app's Gradle build by `[package.metadata.day.android]`, with **no edits to any core Day
crate**, including the permission-result callback, which lives in a headless `Fragment` the crate
attaches itself rather than in `day-android`'s `DayActivity`. Its `permissions = []` entry is
empty and must stay that way, because a permissions crate must never force a permission into an
app's manifest. See [extending.md](extending.md).
