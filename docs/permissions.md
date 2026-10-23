---
navTitle: "Permissions"
title: "OS permissions"
description: "Declaring, requesting, and explaining OS permissions across platforms, and what happens when you skip a step."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# OS permissions

Check whether an app can use a protected capability and ask the user for access when needed.
`day-part-permissions` handles runtime permission checks and requests; `Day.toml` declares the
capabilities and explanations required by each operating system.

## Add to your app

Defined in [`day-part-permissions`](https://github.com/daybrite/day/tree/main/parts/day-part-permissions). Add this to your app’s `Cargo.toml`
`[dependencies]` table, using the same Day Git source as the rest of your app:

```toml
[dependencies]
day-part-permissions = { git = "https://github.com/daybrite/day.git" }
```

The implementation is selected by the compilation target; no toolkit feature flags
or dependency entry in `Day.toml` are needed.

Declare the required capabilities and usage explanations in [Day.toml permissions](/docs/reference/permissions#declare-before-you-ask).

| Platforms / toolkits | Availability |
| --- | --- |
| macOS, iOS, Android, HarmonyOS, Web | Permission checks and requests vary by capability. Device prompts remain unverified. |
| Windows, Linux (GTK / Qt) | Reports static gate results; does not open system permission prompts. |

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

### Localized reasons

The inline text above is the single-language shortcut. An app with more than one locale keeps
each reason in its catalogs instead, as a `permission_<name>` message beside the rest of its
copy, so translators see it where they work and the prompt speaks the user's language:

```toml
[permissions]
camera = true                 # declared; the text is the catalog's
location-when-in-use = true

[permissions.raw]
ios = { NSBluetoothAlwaysUsageDescription = true }
```

```ftl
# resource/locales/en/app.ftl
permission_camera = Attach photos to your notes.
permission_location_when_in_use = Show stations near you.
permission_NSBluetoothAlwaysUsageDescription = Find your bike's lock.
```

The message id is `permission_` plus the portable name with every non-alphanumeric character
as `_`, or plus the native key itself for a raw entry. The catalog's message wins over inline
text in the default locale; inline text still fills the default locale when the catalog has no
message, so a single-locale app never has to move anything. `day localize add` copies the
messages into a new locale with the rest of the catalog, and `day metadata --json` reports every
declaration's `reasons` by locale for tooling such as the project site.

## Authoring

```rust
use day_part_permissions::{Permission, Status, request, status};

if status(Permission::Camera) != Status::Granted {
    request(Permission::Camera, |s| println!("camera: {s}"));
}
```

| Function | Answers |
|---|---|
| [`gate(perm) -> Gate`](https://daybrite.dev/api/day_part_permissions/fn.gate.html) | whether this target gates the capability at all |
| [`status(perm) -> Status`](https://daybrite.dev/api/day_part_permissions/fn.status.html) | what the OS will do if you use it now (never blocks) |
| [`status_async`](https://daybrite.dev/api/day_part_permissions/fn.status_async.html) / [`status_future`](https://daybrite.dev/api/day_part_permissions/fn.status_future.html) | the same, but authoritative where the platform is async |
| [`can_prompt(perm) -> bool`](https://daybrite.dev/api/day_part_permissions/fn.can_prompt.html) | whether [`request`](https://daybrite.dev/api/day_part_permissions/fn.request.html) would show a dialog |
| [`should_show_rationale(perm)`](https://daybrite.dev/api/day_part_permissions/fn.should_show_rationale.html) | Android's "explain first" signal; `false` elsewhere |
| [`request`](https://daybrite.dev/api/day_part_permissions/fn.request.html) / [`request_future`](https://daybrite.dev/api/day_part_permissions/fn.request_future.html) | ask the OS |
| [`request_many`](https://daybrite.dev/api/day_part_permissions/fn.request_many.html) / [`request_many_future`](https://daybrite.dev/api/day_part_permissions/fn.request_many_future.html) | ask for several in one prompt sequence |
| [`open_settings(perm) -> bool`](https://daybrite.dev/api/day_part_permissions/fn.open_settings.html) | the remedy when the answer is already final |

The crate has no cargo features: platform selection is purely `#[cfg(target_os)]`, since consent
depends on the OS, not on which widget toolkit is in use.
`parts/day-part-permissions/examples/permissions.rs` is a plain `main` that uses it with no Day
framework at all.

## Two questions, two vocabularies

[`gate()`](https://daybrite.dev/api/day_part_permissions/fn.gate.html) answers a structural question and [`status()`](https://daybrite.dev/api/day_part_permissions/fn.status.html) a live one, and keeping them apart
lets an ungated platform answer accurately:

| `Gate` | meaning |
|---|---|
| `Prompts` | the OS keeps a consent record and can show a dialog |
| `Ungated` | the capability exists and nothing gates it (desktop Linux, Windows) |
| `Absent` | no such capability here at all |

| `Status` | meaning |
|---|---|
| `Granted` | go ahead |
| `Prompt` | nobody has decided; [`request`](https://daybrite.dev/api/day_part_permissions/fn.request.html) will show a dialog |
| `Denied` | the user said no; [`can_prompt`](https://daybrite.dev/api/day_part_permissions/fn.can_prompt.html) says whether asking again can help |
| `Restricted` | policy forbids it, or it is missing from the merged manifest; neither asking nor Settings helps |
| `Unsupported` | `Gate::Absent` |
| `Unknown` | the platform answers only asynchronously and hasn't yet (web, and Apple notifications) |

On desktop Linux the camera has no permission gate, so [`status`](https://daybrite.dev/api/day_part_permissions/fn.status.html) answers **`Granted`**: an app
asking "may I use the camera?" should proceed, and the real failure belongs at `open("/dev/video0")`.
The structural fact moves to [`gate() == Ungated`](https://daybrite.dev/api/day_part_permissions/fn.gate.html). Two invariants are unit-tested:
`Ungated ⟹ Granted`, and `Absent ⟹ Unsupported`.

`Granted` is not a promise that the hardware exists. A laptop with no camera still answers
`Granted`, because no permission stands in the way; ask the capability's own part (e.g.
[`day_part_sensors::is_available`](https://daybrite.dev/api/day_part_sensors/fn.is_available.html)) about hardware.

## Reasons are not a runtime parameter

[`request(perm, reason)`](https://daybrite.dev/api/day_part_permissions/fn.request.html) is the natural API guess, but no platform accepts a reason at request time.
iOS and macOS read `NS…UsageDescription` from `Info.plist`; `requestPermissions(String[], int)` and
`requestPermissionsFromUser(context, string[])` take no text, and neither does `getUserMedia` or
`Notification.requestPermission`. The reason therefore lives in the declaration, where it reaches
the OS, and the runtime hands your app the two bits it needs to draw its own priming UI:
[`should_show_rationale`](https://daybrite.dev/api/day_part_permissions/fn.should_show_rationale.html) and [`can_prompt`](https://daybrite.dev/api/day_part_permissions/fn.can_prompt.html).

As a consequence, no user-facing string crosses this crate's boundary, so the layering rule that
keeps `IntoText`/`LocalizedText` out of parts ([extension model](extending.md)) never has to be worked around.

## Per-platform native realization

Platform guidance: [Apple: privacy](https://developer.apple.com/design/human-interface-guidelines/privacy); [Android: requesting permissions](https://developer.android.com/training/permissions/requesting).

| OS | check | request | dependency |
|---|---|---|---|
| iOS | [`CLLocationManager`](https://developer.apple.com/documentation/corelocation/cllocationmanager), [`AVCaptureDevice`](https://developer.apple.com/documentation/avfoundation/avcapturedevice), [`PHPhotoLibrary`](https://developer.apple.com/documentation/photos/phphotolibrary), [`UNUserNotificationCenter`](https://developer.apple.com/documentation/usernotifications/unusernotificationcenter) (async-only), [`CMMotionActivityManager`](https://developer.apple.com/documentation/coremotion/cmmotionactivitymanager) | the matching block-based `request…` | `objc2` + `block2`, `[package.metadata.day.ios].frameworks` |
| macOS | the same TCC APIs where they exist; no CoreMotion | same | shared `apple.rs` |
| Android | [`Context.checkSelfPermission`](https://developer.android.com/reference/android/content/Context#checkSelfPermission(java.lang.String)) + `getPackageInfo(GET_PERMISSIONS)` | `requestPermissions` from a headless `Fragment` | `day-android` + the crate's own Java shim |
| HarmonyOS | [`OH_AT_CheckSelfPermission`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-ability-kit/capi-ability-access-control-h.md); for notifications, `notificationManager.isNotificationEnabledSync` | `requestPermissionsFromUser`, through the host page's `registerPermissions` seam in `day-arkui` (found in the app's own library by `dlsym`, so the part links no toolkit); for notifications, `requestEnableNotification` | raw FFI; the notification switch through an ArkTS daybridge arm |
| Web | [`navigator.permissions.query`](https://www.w3.org/TR/permissions/#dom-permissions-query) + a live `change` cache; `Notification.permission` is sync | the per-API call | the day-dom shim |
| Linux / Windows | constants | resolves immediately | — |

Two platform facts apply everywhere:

- **[`request`](https://daybrite.dev/api/day_part_permissions/fn.request.html) is callback-and-future only; there is no blocking form.** The OS prompt is drawn by
  the very thread a blocking call would park, so it would deadlock by construction on every platform.
- **Dropping a `StatusFuture` does not dismiss the prompt.** No platform can dismiss its own
  permission dialog programmatically. Dropping stops you listening; the user's answer is still
  recorded, so the next [`status()`](https://daybrite.dev/api/day_part_permissions/fn.status.html) is correct. Aborting a [`day::task`](https://daybrite.dev/api/day/fn.task.html) that awaits one therefore
  leaves the dialog on screen.

### Android cannot tell "never asked" from "permanently denied"

It cannot without app-side state, and **Day keeps none**. A denied-but-declared permission with no
rationale flag is reported as `Prompt` either way. That is safe (asking after a permanent refusal
shows no dialog and resolves `Denied` immediately), but if your app needs the distinction, record it
yourself when you call [`request`](https://daybrite.dev/api/day_part_permissions/fn.request.html):

```rust
day_part_permissions::request(perm, move |s| {
    day::prefs::set("asked.camera", "1");
    // …
});
```

### macOS: the desktop dev loop cannot exercise permissions

`day launch -t macos-appkit` runs the bare binary, not a bundle. TCC reads usage descriptions from a
bundle's `Info.plist`, so an unbundled process is denied (or killed) regardless of what Day.toml
says. Only `day pack -t macos-appkit` produces a bundle that can be granted anything. The crate
guards every [`UNUserNotificationCenter`](https://developer.apple.com/documentation/usernotifications/unusernotificationcenter) call behind a bundle check for the same reason: touching it
unbundled aborts the process.

## What the declaration pipeline generates

| portable name | Android | iOS `Info.plist` | macOS | HarmonyOS |
|---|---|---|---|---|
| `location-when-in-use` | `ACCESS_FINE_LOCATION`, `ACCESS_COARSE_LOCATION` | [`NSLocationWhenInUseUsageDescription`](https://developer.apple.com/documentation/bundleresources/information-property-list/nslocationwheninuseusagedescription) | + [`NSLocationUsageDescription`](https://developer.apple.com/documentation/bundleresources/information-property-list/nslocationusagedescription) | `APPROXIMATELY_LOCATION` + `LOCATION` |
| `location-always` | + `ACCESS_BACKGROUND_LOCATION` | that key **and** the when-in-use one (Apple suppresses the prompt without both) | + [`NSLocationUsageDescription`](https://developer.apple.com/documentation/bundleresources/information-property-list/nslocationusagedescription) | + `LOCATION_IN_BACKGROUND` |
| `camera` | `CAMERA` | [`NSCameraUsageDescription`](https://developer.apple.com/documentation/bundleresources/information-property-list/nscamerausagedescription) | same | `ohos.permission.CAMERA` |
| `microphone` | `RECORD_AUDIO` | [`NSMicrophoneUsageDescription`](https://developer.apple.com/documentation/bundleresources/information-property-list/nsmicrophoneusagedescription) | same | `ohos.permission.MICROPHONE` |
| `notifications` | `POST_NOTIFICATIONS` | none | none | none (a runtime call) |
| `photos` | `READ_MEDIA_IMAGES`, `READ_MEDIA_VIDEO`, `READ_EXTERNAL_STORAGE` capped at `maxSdkVersion=32` | [`NSPhotoLibraryUsageDescription`](https://developer.apple.com/documentation/bundleresources/information-property-list/nsphotolibraryusagedescription) | same | `READ_IMAGEVIDEO` ⚠ |
| `motion` | `ACTIVITY_RECOGNITION` | [`NSMotionUsageDescription`](https://developer.apple.com/documentation/bundleresources/information-property-list/nsmotionusagedescription) | none (CoreMotion is iOS-only) | `ACTIVITY_MOTION` |
| `screen-privacy` | none | none | none | `PRIVACY_WINDOW` (granted at install, no reason) |
| `biometrics` | `USE_BIOMETRIC` (install-time, no dialog) | [`NSFaceIDUsageDescription`](https://developer.apple.com/documentation/bundleresources/information-property-list/nsfaceidusagedescription) | none (Touch ID and the account password need no key) | `ACCESS_BIOMETRIC` (granted at install) |

`screen-privacy = true` is how a HarmonyOS app opts into content protection
([`WindowHandle::set_content_protected`](https://daybrite.dev/api/day_core/windows/struct.WindowHandle.html#method.set_content_protected), [docs/windows](windows.md)): the window's privacy mode needs this
permission, and `Cap::ContentProtection` answers `Native` there only when the app declared it. No
other platform needs a permission for it, and nothing prompts, so it takes no reason.

`biometrics = "…"` is what [`day-part-biometrics`](/docs/reference/biometrics) asks an app to declare,
and what [`day-part-keychain`](/docs/reference/keychain) needs for items that require the person
present: the reason is the sentence Face ID shows the first time, and the Android and HarmonyOS
permissions are granted at install with no dialog. Nothing requests it at run time through this
crate, so it has no `Permission` variant.

⚠ `ohos.permission.READ_IMAGEVIDEO` is a `system_basic` permission, which an app signed at the
default `normal` level cannot hold. Prefer `PhotoViewPicker`, which needs no permission at all.

The table lives in `day_build::permissions`, one source shared by the CLI's generators and this
crate's runtime, with a parity test pinning the Rust variant names so `day lint` can map a source
reference back to a declaration.

For anything outside the portable set, use the raw tables:

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
- **iOS/macOS** — the checked-in `platform/ios/Runner/Info.plist` and
  `platform/macos/Runner/Info.plist`, each from its platform's column of the table, edited in
  place (the macOS host since 2026-10; before that a macOS bundle carried no usage descriptions,
  and TCC terminated a sandboxed app at its first camera or location request). Day owns exactly
  the keys in the table above plus your `[permissions.raw]` keys; every other byte is preserved, so
  the diff shows only what changed, and a hand-added key Day doesn't model is never touched. Two
  consecutive builds produce a byte-identical file. This is an exception to "aggregation
  never mutates the scaffolds", because the alternative broke `⌘R` in Xcode.
  The plist carries the default locale's text; the translations go to `InfoPlist.xcstrings`
  beside it, the string catalog Xcode 15+ reads localized `Info.plist` values from, one
  `stringUnit` per key and locale in Xcode's own locale spelling (`zh-Hans`). The scaffold
  ships an empty catalog already wired into the target; an older project gets the file and its
  four project entries the first time a build has a translation to write. The macOS host gets its
  catalog the same way, and the generated Xcode project references it once it exists. Regenerated from the
  same plan as the plist and byte-stable, so open it in Xcode to read, not to edit.
- **HarmonyOS** — Day copies the native host into `build/day/harmony/project/` and merges
  permissions into that copy's `entry/src/main/module.json5`. The referenced
  `day_perm_reason_*` strings go into the staged `resources/base/element/string.json` and
  per-locale files such as `resources/fr/element/string.json` and
  `resources/zh_CN/element/string.json`. Hand-written strings survive the merge; obsolete
  generated translations are removed. The source host under `platform/harmony/` stays
  unchanged, so adding a locale requires no generated native files in git. `day prepare`
  prepares these files too, and `day open -t harmony-arkui` opens the staged project.

## Testing past the prompt

> [!WARNING]
> **Device permission prompts.**
> Runtime prompts have not been exercised on devices. Test granting, denying, and revoking
> each permission your app requests.

> [!WARNING]
> **HarmonyOS / ArkUI: permissions manifest.**
> Generated permissions have not been verified in a device build. Check the packaged
> manifest and permission behavior on your target device.

A dayscript drives Day's own views and nothing else; the consent dialog is the system's window,
out of its reach ([docs/testing](testing.md)), so a walkthrough that needs the camera or the
microphone stops at the first [`request`](https://daybrite.dev/api/day_part_permissions/fn.request.html). Rather than drive the prompt, avoid it:

```sh
day launch -t android-mdc --grant camera --script dayscript/capture.yaml
```

`--grant` takes a portable name (repeatable, or a comma-separated list) and marks it granted on
the device between the install and the start, with the platform's own tool: `pm grant` for
each Android runtime permission the name maps to (minus any whose `maxSdkVersion` the device
has passed, which the installed app never requested), and `simctl privacy grant` for the iOS
Simulator's service. The app's first [`status`](https://daybrite.dev/api/day_part_permissions/fn.status.html) then reads `Granted`, and a reinstall of the same
build keeps the marks.

Where no tool exists the flag is refused before anything is built, since a grant that quietly
did not happen would surface minutes later as a failed assertion with nothing to say about why:

| target | `--grant` |
|---|---|
| `android-mdc` | `pm grant`, every runtime permission in the table row |
| `ios-uikit` on the simulator | `simctl privacy grant`: `location-when-in-use`, `location-always`, `microphone`, `photos`, `motion`. The simulator has no camera, so no camera service, and no service for notifications |
| `ios-uikit` on a device (`--ios-device`) | refused: the consent record is the device's own |
| `harmony-arkui` | refused: HarmonyOS ships no tool that marks a `user_grant` permission from outside the app |
| `macos-*` | refused: `tccutil` resets TCC, it never grants (see above) |
| `web-dom` | refused: the driver's own grants stand ([docs/web](web.md)) |
| `linux-*`, `windows-*` | nothing gates the capability, so the launch notes that and carries on |

The shared CI workflow's `grant-permissions` input names the grants per target, so one call can
pre-grant the camera on the Android emulator and still assert the undecided state everywhere
else.

## `day lint`

| code | fires when |
|---|---|
| `day::lint::undeclared-permission` | code requests `Permission::X` that Day.toml doesn't declare |
| `day::lint::missing-reason` | a declared permission that needs a reason has none — or has inline text only while another locale's catalog lacks its `permission_<name>` message |
| `day::lint::duplicate-reason` | a reason given both inline and in the default catalog (the catalog's ships; the other can drift) |
| `day::lint::unused-permission` | declared, referenced by nothing (a warning — over-declaring gets apps rejected) |
| `day::lint::stale-manifest` | the checked-in `Info.plist` or `InfoPlist.xcstrings` disagrees with Day.toml and the catalogs; run `day build -t ios-uikit` |

## What it shows about the extension system

The crate registers nothing in any `RENDERERS` slice. Its Android half is bundled with it and folded
into the app's Gradle build by `[package.metadata.day.android]`, with **no edits to any core Day
crate**, including the permission-result callback, which lives in a headless `Fragment` the crate
attaches itself rather than in `day-android`'s `DayActivity`. Its `permissions = []` entry is
empty and must stay that way, because a permissions crate must never force a permission into an
app's manifest. See [extending](extending.md).
