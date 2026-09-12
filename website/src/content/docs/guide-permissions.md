---
title: Permissions
description: "Build-time permission declarations and runtime access requests."
order: 31
section: Guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Access to the camera, location, and other protected features may require the user’s permission.
Day handles this in two stages: a declaration in `Day.toml` supplies the platform metadata,
and `day-part-permissions` checks or requests access while the app is running.

```rust
use day_part_permissions::{Permission, Status, request, status};

if status(Permission::Camera) != Status::Granted {
    request(Permission::Camera, |s| println!("camera: {s}"));
}
```

**Works on:** every target. iOS, macOS, Android, HarmonyOS, and the web keep consent records and
can prompt; desktop Linux and Windows have no consent database, so there `gate()` answers
`Ungated` and `status()` answers `Granted`: proceed, and let a real failure surface at the device.
The full per-platform matrix is in [the permissions reference](/docs/internal/permissions).

## 1. Declare it in Day.toml

The declaration half is required: every mobile OS demands a manifest entry in addition to the
runtime request, and skipping it crashes or silently denies (see pitfalls). Declare what you use,
with the reason the OS shows the user in its own prompt:

```toml
[permissions]
camera               = "Attach photos to your notes."
location-when-in-use = "Show stations near you."
notifications        = true          # needs no reason on any platform
```

From this, `day build` writes the Android manifest permissions, the `NS…UsageDescription` keys in
`Info.plist`, and the HarmonyOS `module.json5` entries. Every OS reads the reason from its
manifest and none accepts it at runtime, which is why it lives here and why `request()` takes
none. For permissions outside the portable set, `[permissions.raw]` passes
platform-native names through. `day lint` flags a permission your code requests but `Day.toml`
doesn't declare, so the mismatch is caught in CI.

## 2. Check, request, react

The runtime half is `day_part_permissions`. `status(perm)` answers what the OS will do right now,
never blocking; `request(perm, on_done)` asks, showing the system prompt when one would appear. This
notifications flow feeds the answer into UI through a signal:

```rust
use day_part_permissions::{Permission, Status, can_prompt, open_settings, request, status};

let granted = Signal::new(status(Permission::Notifications) == Status::Granted);

button("Enable reminders").action(move || {
    if can_prompt(Permission::Notifications) {
        let set = granted.setter();
        request(Permission::Notifications, move |s| {
            set.set(s == Status::Granted);
        });
    } else {
        // The answer is final for this launch; the OS settings page is the remedy.
        open_settings(Permission::Notifications);
    }
});
```

- The callback runs on an unspecified thread, possibly the UI thread, so it delivers into UI state
  through a `Setter`, not by touching a `Signal` directly. There is no blocking `request`, because
  the prompt is drawn by the very thread a blocking call would park. `request_future` and
  `status_future` exist for async code.
- `can_prompt` picks the affordance. Apple never re-prompts after a denial and Android may stop, so
  once `can_prompt` is false a "grant access" button is a control that does nothing; offer
  `open_settings` instead. `should_show_rationale` is Android's "explain first" signal for drawing
  your own priming UI before the real prompt.
- `status` can answer `Unknown` on first call where the platform is async-only (the web, and Apple
  notifications). `status_async`/`status_future` wait for the platform's answer and never return
  it.

Concurrent requests for the same permission coalesce into one prompt, and
`request_many` batches several into one prompt sequence. The permission this example requests is
put to work in [Local notifications](/docs/guide-notifications).

## 3. Permissions a library crate uses

A library crate that uses a gated capability declares the machine-facing half in its own
manifest:

```toml
[package.metadata.day.permissions]
uses = ["camera"]
```

That names *which* permission, never the reason: the reason is app copy, shown to your user in
the OS prompt, and it belongs in the app's `Day.toml` where you write and localize it. A
contribution whose reason is missing from the app's `Day.toml` is a hard build error on the
platforms that show one (iOS and HarmonyOS), naming the crate and the lines to paste.

## 4. What each platform does with a request

The [reference](/docs/internal/permissions) carries the full matrix. Apple platforms go
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
- macOS dev builds can't be granted anything. `day launch -p macos-appkit` runs a bare
  binary, and TCC reads usage descriptions from a bundle's `Info.plist`, so an unbundled process
  is denied or killed regardless of what `Day.toml` says. `day pack -p macos-appkit` produces the
  bundle that can hold a grant.
- Dropping a `StatusFuture` does not dismiss the prompt. No platform can take its own
  permission dialog off the screen. Dropping stops you listening; the user's answer is still
  recorded, and the next `status()` reflects it.

## Reference

[permissions](/docs/internal/permissions) — the portable-to-native mapping table,
manifest entries generated by Day, the `day lint` codes, and direct platform API access. The
metadata contribution mechanism is [extending](/docs/extending), and
[Local notifications](/docs/guide-notifications) walks the permission this guide's example
requests.
