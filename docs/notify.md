---
title: "Local notifications"
description: "Post and schedule local notifications, inspect platform capabilities, and handle permission failures."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Local notifications

Show reminders and updates through the operating system’s notification UI.
`day-part-local-notify` posts notifications immediately or schedules them for later;
permission and scheduling support depend on the platform.

## Add to your app

Defined in [`day-part-local-notify`](https://github.com/daybrite/day/tree/main/parts/day-part-local-notify). Add this to your app’s `Cargo.toml`
`[dependencies]` table, using the same Day Git source as the rest of your app:

```toml
[dependencies]
day-part-local-notify = { git = "https://github.com/daybrite/day.git" }
```

The implementation is selected by the compilation target; no toolkit feature flags
or dependency entry in `Day.toml` are needed.

| Platforms / toolkits | Availability |
| --- | --- |
| macOS, iOS, Android | Posting and scheduled delivery; Android exact scheduling requires an additional grant. |
| HarmonyOS | Immediate delivery only. |
| Windows, Linux (GTK / Qt), Web | Posting is unsupported. |

## API

The [crate](../parts/day-part-local-notify/src/lib.rs) provides `Channel`,
`Importance`, `Notification`, `NotifId`, `Capabilities`, and `NotifyError`.
`Notification::post(self)` returns `Result<NotifId, NotifyError>`.
`Trigger` has exactly three variants: `Now`, `In(Duration)`, and `At(SystemTime)`.
Use `cancel(NotifId)` or `cancel_all()` to cancel notifications.

Call `capabilities()` to inspect the running platform's behavior, including `post`,
`schedule_while_dead`, `schedule_exact`, and `tap_route`. Request notification permission
through [permissions](permissions.md); localize notification text through generated accessors
before passing it to the API. See the [task guide](/docs/how-to/guide-notifications).

## Limits

The source exposes `Unsupported`, `PermissionDenied`, and `Failed(String)` errors.
Recurring triggers, actions, and remote push notifications are not part of this API.

## Parameters, defaults, and failure behavior

| Surface | Contract |
| --- | --- |
| `Channel::new(id, importance)` | Starts with `id` as its display name; call `.name(res::str::…().format())` before registering |
| Channel sound | Initially true for High/Urgent importance, false below High; `.sound(bool)` overrides it |
| `Channel::register(self)` | Returns `()`; no portable registration error result |
| `Notification::new(title)` | Empty body/subtitle/route/icon, no explicit ID or badge, channel `"default"`, `Trigger::Now` |
| `.id(NotifId)` | Requests replacement under a stable ID |
| `.trigger(Trigger)` | Now, relative duration, or absolute system time; no recurring variant |
| `.post(self)` | Returns an assigned ID or `Unsupported`, `PermissionDenied`, or `Failed(String)` |
| `cancel(id)` / `cancel_all()` | Return `()`; do not report whether the OS previously displayed the notification |

Format channel names and notification text on the UI thread. Notification strings are snapshots,
not reactive bindings; posting after a locale change requires a newly formatted notification.
On Android the user owns channel settings after creation, so re-registering a channel does not
promise to replace their chosen importance. Apple acceptance is not permission or delivery proof.

## Platform capabilities

Platform guidance: [Apple: notifications](https://developer.apple.com/design/human-interface-guidelines/notifications); [Android: notifications](https://developer.android.com/develop/ui/views/notifications).

These values follow each toolkit's `capabilities()` implementation. Query it at runtime:
Android and HarmonyOS depend on their native bridge being available, and macOS requires an app bundle.

| Platform | Post | Schedule after exit | Exact scheduling | User channel settings | Badge | Custom icon | Tap route |
| --- | --- | --- | --- | --- | --- | --- | --- |
| macOS / iOS | Yes | Yes | Yes | No | Yes | No | Yes |
| Android | Yes | Yes | With exact-alarm grant | Yes | Launcher-dependent | Yes | Yes |
| HarmonyOS | Yes | No | No | No | Yes | No | Yes |
| Linux / Windows / browser | No | No | No | No | No | No | No |

HarmonyOS uses Notification Kit. Importance maps to system notification slots rather than
app-defined channel settings. A delayed notification uses an in-process timer: exiting loses
it, and background suspension can delay it. Request notification permission before posting.

Apple schedules through the system notification center. Android persists schedules and uses
an `AlarmManager` receiver, including re-arming after reboot. Without the exact-alarm grant,
Android falls back to an inexact alarm. See the [scheduling guide](/docs/how-to/guide-notifications#4-schedule-and-cancel).

## Delivery pitfalls

- iOS retains at most 64 pending local notifications. Keep a rolling schedule instead of
  submitting an unbounded set of reminders.
- Android small icons must be monochrome silhouettes. A full-color launcher image can become
  a white blob in the status bar; the generated default supplies a notification silhouette.
- A bare macOS executable has no usable notification center. Launch the generated `.app`
  bundle with `day launch` instead of running the Rust binary directly.
- Permission, Focus/Do Not Disturb settings, channel settings, and launcher behavior affect
  what the user sees. A successful `post` returns an assigned ID, not a delivery receipt.

The [notification recipe](/docs/how-to/guide-notifications) supplies imports
and catalog entries. Test delayed delivery, app exit, and tap routing on each target you ship.
