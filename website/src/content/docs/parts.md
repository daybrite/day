---
title: Device capabilities (parts)
description: "Headless platform capabilities (battery, clipboard, preferences, sensors, network, permissions, location) as ordinary crates with per-OS implementations."
order: 25
section: Reference
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

A **part** exposes a platform capability without a UI, such as battery status, storage, or
sensors. It provides a shared Rust API with an implementation selected at compile time for
the target operating system. Parts are Cargo dependencies and require no runtime plugin registry.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/examples/catalog) and [catalog setup](/docs/how-to/localization).

## The catalog

| Crate | What it does | Reference |
|---|---|---|
| [`day-part-battery`](/docs/reference/battery) | charge level and charging state | [battery](/docs/reference/battery) |
| [`day-part-clipboard`](/docs/reference/clipboard) | read/write the system clipboard (text) | [clipboard](/docs/reference/clipboard) |
| [`day-part-prefs`](/docs/reference/prefs) | small key-value preference storage in the platform's conventional location | [prefs](/docs/reference/prefs) |
| [`day-part-fs`](/docs/reference/fs) | app-local file storage: read, write, remove, list (sync and async) | [fs](/docs/reference/fs) |
| [`day-part-local-notify`](/docs/reference/notify) | local notifications: post now or schedule, channels, tap-to-route | [notify](/docs/reference/notify) |
| [`day-part-keychain`](/docs/reference/keychain) | small secrets in the platform's secure store, with a user-presence level | [keychain](/docs/reference/keychain) |
| [`day-part-biometrics`](/docs/reference/biometrics) | the platform's own authentication prompt: biometrics or the device credential | [biometrics](/docs/reference/biometrics) |
| [`day-part-network`](/docs/reference/network) | connectivity status | [network](/docs/reference/network) |
| [`day-part-deviceinfo`](/docs/reference/deviceinfo) | device model, OS version | [deviceinfo](/docs/reference/deviceinfo) |
| [`day-part-sensors`](/docs/reference/sensors) | accelerometer and friends, as a live stream | [sensors](/docs/reference/sensors) |
| [`day-part-http`](/docs/reference/http) | HTTP through each platform's networking stack | [http](/docs/reference/http) |
| [`day-part-permissions`](/docs/reference/permissions) | ask the OS for the camera, location, notifications … and declare them at build time | [permissions](/docs/reference/permissions) |
| [`day-part-location`](/docs/reference/location) | the device's position, once or as a live stream | [location](/docs/reference/location) |
| [`day-part-haptics`](/docs/reference/haptics) | haptic feedback | [haptics](/docs/reference/haptics) |
| [`day-part-sound`](/docs/reference/sound) | sound effects | [sound](/docs/reference/sound) |
| [`day-part-wakelock`](/docs/reference/wakelock) | keeping the screen on | [wakelock](/docs/reference/wakelock) |
| `day-part-speech` | text to speech through each platform's speech API; in a separate repository | [day-part-speech](https://github.com/daybrite/day-part-speech) |

## Select an API by its completion model

| Need | Entry point and result | Constraint |
| --- | --- | --- |
| Current battery reading | `day_part_battery::status() -> Option<BatteryStatus>` | Absence can mean no battery/API; inspect the returned fields |
| Small preference | `day::prefs::get(&str) -> Option<String>`; `set(&str, &str) -> bool` | Synchronous; the default facade feature supplies it; false/None can include failure |
| File data | `day_part_fs::read_future(&str)` | Await a result; native blocking forms must stay off the UI thread |
| HTTP response | `day_part_http::fetch_future(Request)` | Check transport errors and HTTP status separately |
| Permission | `day_part_permissions::status_future(Permission)` | Await authoritative status; a grant does not establish hardware availability |
| Notification | `Notification::post() -> Result<NotifId, NotifyError>` | Consent and channel setup are separate; success does not establish delivery |

A part is a crate category, not a shared `Part` trait. There is no universal error or support
contract: check each part's `capabilities`, `is_supported`, optional value, or error result.
OS selection often uses conditional compilation; some parts also depend on a live host bridge.
Enabling a UI toolkit feature does not prove that all parts are available.

For callbacks requiring `Send`, pass result data through a `Setter`. For futures awaited inside
`day::task`, update signals and format generated localized messages on the UI thread. Keep task
and scope lifetimes explicit. See [async ownership](/docs/explanation/async).

## Complete tasks

Use the compile-designated [HTTP](/docs/how-to/guide-http), [storage](/docs/how-to/guide-storage),
[permission](/docs/how-to/guide-permissions), and [notification](/docs/how-to/guide-notifications)
recipes. Clipboard's synchronous text helpers cannot express browser async permission behavior;
consult the [clipboard reference](/docs/reference/clipboard) for its asynchronous content API.

## Implement a part

A part can contribute native source, dependencies, and permissions through Cargo metadata.
Follow the [part tutorial](/contributing/tutorials/part),
[extension model](/contributing/extending), and [bridge contract](/contributing/backend-contracts/bridge)
for implementation details. Inline bridge language arms are contributor material, not an
additional app-facing Part interface.

Source review: [part crates](https://github.com/daybrite/day/tree/main/parts) and
[facade feature gates](https://github.com/daybrite/day/blob/main/crates/day/Cargo.toml), 2026-10-22.
The speech part is an external project; its implementation and current compatibility were not
verified in this repository-only review.
