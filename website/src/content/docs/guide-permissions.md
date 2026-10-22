---
title: Permissions
description: "Build-time permission declarations and runtime access requests."
order: 31
section: How-to guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Access to the camera, location, and other protected features may require the user’s permission.
Day handles this in two stages: a declaration in `Day.toml` supplies the platform metadata,
and [`day-part-permissions`](/docs/reference/permissions) checks or requests access while the app is running.

Request a permission from an explicit user action. Check its status first so a granted,
restricted, or unsupported capability does not send the user through an irrelevant request
or Settings action.

Permission behavior differs by platform. Apple platforms, Android, and web can prompt where
the capability supports it. Day can check HarmonyOS permissions but cannot yet prompt there.
On desktop Linux and Windows, this API reports `Ungated` and `Granted`; that does not guarantee
the device or service is available. See [platform behavior](#4-what-each-platform-does-with-a-request)
for details.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/examples/catalog) and [catalog setup](/docs/how-to/localization).

## Check an authoritative status

Start with [`day-part-permissions`](/docs/reference/permissions) at the same revision as `day` and the manifest declaration
below. Use `status_future` where a cached `Unknown` is insufficient. This module asks only
when the current answer is `Prompt`, or is `Denied` and the backend allows another prompt; it does not open Settings for a granted or unsupported
permission. Request consent from a user action, and offer a separate Settings action after denial.

Complete module for `src/permissions.rs` in the [technical recipe harness](/docs/reference/examples/catalog#technical-recipes).
The harness compiles this exact block; it is not a standalone app. Add the catalog entries below
to `resource/locales/en/app.ftl` and declare the module in `src/lib.rs`.

<!-- technical-example: permissions -->
```rust
use crate::res;
use day::prelude::*;
use day_part_permissions::{
    Permission, Status, can_prompt, open_settings, request_future, status_future,
};

pub fn should_request(current: Status, prompt_available: bool) -> bool {
    prompt_available && matches!(current, Status::Prompt | Status::Denied)
}

pub async fn notification_access() -> Status {
    let permission = Permission::Notifications;
    let current = status_future(permission).await;
    if should_request(current, can_prompt(permission)) {
        request_future(permission).await
    } else {
        current
    }
}

pub fn notification_controls() -> impl Piece {
    let status = Signal::new(Status::Unknown);
    column((
        button(res::str::permission_check())
            .id(id!(permission_check))
            .action(move || {
                day::task(async move {
                    status.set(notification_access().await);
                });
            }),
        label(move || match status.get() {
            Status::Granted => res::str::permission_granted().format(),
            Status::Denied => res::str::permission_denied().format(),
            Status::Restricted => res::str::permission_restricted().format(),
            Status::Unsupported => res::str::permission_unsupported().format(),
            _ => res::str::permission_pending().format(),
        })
        .id(id!(permission_status)),
        button(res::str::permission_settings())
            .id(id!(permission_settings))
            .enabled(move || status.get() == Status::Denied)
            .action(|| {
                if !open_settings(Permission::Notifications) {
                    day::task(async {
                        alert(res::str::permission_settings_unavailable()).await;
                    });
                }
            }),
    ))
    .spacing(Space::M)
}
```

<!-- technical-catalog: permissions -->
```ftl
permission_check = Enable reminders
permission_granted = Notification access is enabled.
permission_denied = Notification access was not granted.
permission_restricted = Notification access is restricted by device policy.
permission_unsupported = Notifications are unavailable on this target.
permission_pending = Check notification access to enable reminders.
permission_settings = Open notification settings
permission_settings_unavailable = Open your system settings to change notification access.
```

Await this from `day::task`; handle all `Status` variants, including a fallback for this
non-exhaustive enum. `Granted` means permission, not hardware availability or delivery.
Format `res::str::permission_denied()` on the UI thread when the result requires that message.
Dropping the future stops listening; it does not dismiss the OS prompt. `open_settings` returns
whether an attempt could be made, so retain an explanation if it returns false.

The remaining snippets are integration fragments, not complete app files. Platform mappings
below describe source behavior; OS consent UI was not exercised in the recipe compilation.

## 1. Declare it in Day.toml

Declare permissions before calling the protected API. Day uses these entries to generate
platform metadata, including user-facing reasons where the OS requires them:

```toml
[permissions]
camera = true
location-when-in-use = true
notifications        = true          # needs no reason on any platform
```

Put the reasons in every app locale's Fluent catalog (configuration fragment):

```ftl
permission_camera = Attach photos to your notes.
permission_location_when_in_use = Show stations near you.
```

The build reads these `permission_<id>` messages when generating platform usage descriptions.
Use their generated accessors for the same text in your UI. Do not put untranslated reason
sentences into application code. From these declarations and catalogs, `day build` writes the Android manifest permissions, the `NS…UsageDescription` keys in
`Info.plist`, and the HarmonyOS `module.json5` entries. The reason belongs to the app’s metadata, so `request()` does not take a reason string. For permissions outside the portable set, `[permissions.raw]` passes
platform-native names through. `day lint` flags a permission your code requests but `Day.toml`
doesn't declare, so the mismatch is caught in CI.

## 2. Check, request, react

The runtime half is `day_part_permissions`. `status(perm)` answers what the OS will do right now,
never blocking; `request(perm, on_done)` asks, showing the system prompt when one would appear.
For an authoritative result on async-only platforms, await `status_future` as in the module above.

The complete `notification_controls` module above keeps requesting access and opening Settings
as separate actions. `Granted` allows the operation; `Restricted` and `Unsupported` need an
explanation, not another prompt. `Denied` may permit another request on some targets, so check
`can_prompt` before retrying.

- The callback runs on an unspecified thread, possibly the UI thread, so it delivers into UI state
  through a `Setter`, not by touching a `Signal` directly. There is no blocking `request`, because
  the prompt is drawn by the very thread a blocking call would park. `request_future` and
  `status_future` exist for async code.
- `can_prompt` tells you whether to offer a request button. Apple never re-prompts after a denial and Android may stop, so
  once `can_prompt` is false a "grant access" button is a control that does nothing; offer
  `open_settings` instead. `should_show_rationale` is Android's "explain first" signal for drawing
  your own priming UI before the real prompt.
- `status` can answer `Unknown` on first call where the platform is async-only (the web, and Apple
  notifications). `status_async` and `status_future` wait for the platform’s answer instead of returning
  `Unknown`.

Concurrent requests for the same permission coalesce into one prompt, and
`request_many` batches several into one prompt sequence. The permission this example requests is
put to work in [Local notifications](/docs/how-to/guide-notifications).

## 3. Permissions a library crate uses

A library crate that uses a gated capability declares the machine-facing half in its own
manifest:

```toml
[package.metadata.day.permissions]
uses = ["camera"]
```

That names *which* permission, never the reason: the reason is app copy, shown to your user in
the OS prompt, and it belongs in the app's Fluent catalogs as `permission_<id>`. A
contribution without a required reason in the resolved app catalog is a build error on
Apple and HarmonyOS targets that consume that permission's reason. Keep the permission
declaration in `Day.toml` and the translated explanation in the catalog.

## 4. What each platform does with a request

The [reference](/docs/reference/permissions) carries the full matrix. Apple platforms go
through each framework's authorization API and ask the user once; after a denial, only
Settings can change the answer. Android shows its dialog via `requestPermissions`
and cannot tell "never asked" from "permanently denied" without app-side state, which Day
does not keep, so record it yourself in the `request` callback if you need the distinction.
HarmonyOS can check but not yet prompt from Day, so `can_prompt` is false there. The web
answers through `navigator.permissions` and the per-API request calls. Desktop Linux and
Windows resolve immediately as `Granted`.

## Pitfalls

- iOS and macOS terminate the process when it touches a gated API without the matching `Info.plist`
  key; there is no exception to catch. Android reports an undeclared permission as
  `Status::Restricted`: the request resolves denied in the same frame, with no dialog, and Settings
  offers nothing. HarmonyOS refuses the request outright. Step 1 is required, and `day lint` catches
  the mismatch.
- Where the capability doesn't exist, `gate()` answers `Absent` and `status()` answers
  `Unsupported`; a `request` resolves immediately with `Unsupported` and no prompt. The reverse case
  also exists: `Granted` on an ungated desktop is not a promise the hardware exists; ask the
  capability's own part about that.
- The current `macos-appkit` build/launch path uses an Xcode host app bundle. Verify its generated
  `Info.plist` and identity when diagnosing permission failures. A manually launched bare Rust
  binary is a different environment and cannot establish the bundle's permission behavior.
- Dropping a `StatusFuture` does not dismiss the prompt. No platform can take its own
  permission dialog off the screen. Dropping stops you listening; the user's answer is still
  recorded, and the next `status()` reflects it.

## Testing past the prompt

A dayscript drives Day's own views; the consent dialog is the system's window, out of its
reach. Avoid it rather than drive it: `--grant` marks the permission allowed on the device
between the install and the start, with the platform's own tool (`pm grant` on Android, `simctl
privacy grant` on the iOS Simulator), so the app's first `status` already reads `Granted`.

```sh
day launch -t android-mdc --grant camera --script dayscript/capture.yaml
```

Where no tool exists the flag is refused before the build: HarmonyOS, a physical iPhone,
macOS, the web, and the camera on the simulator, which has no camera at all. In the
[shared CI workflow](/docs/reference/cli#continuous-integration) the `grant-permissions` input names
grants per target, so the Android emulator's leg can take a real photo while the other legs
still assert the undecided state. [permissions](/docs/reference/permissions#testing-past-the-prompt)
lists what each target supports.

## Reference

[permissions](/docs/reference/permissions) — the portable-to-native mapping table,
manifest entries generated by Day, the `day lint` codes, and direct platform API access. The
metadata contribution mechanism is [extending](/contributing/extending), and
[Local notifications](/docs/how-to/guide-notifications) shows how to use notification permission in an app.
