---
title: "Local notifications"
description: "Post and schedule local notifications, inspect platform capabilities, and handle permission failures."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Local notifications

The `day-part-local-notify` crate provides local notifications. Its current API is separate
from the proposed push service and sending tool in the [notification design](design/notify.md).

## API

The [crate](../parts/day-part-local-notify/src/lib.rs) provides `Channel`,
`Importance`, `Notification`, `NotifId`, `Capabilities`, and `NotifyError`.
`Notification::post(self)` returns `Result<NotifId, NotifyError>`.
`Trigger` has exactly three variants: `Now`, `In(Duration)`, and `At(SystemTime)`.
Use `cancel(NotifId)` or `cancel_all()` to cancel notifications.

Call `capabilities()` to inspect the running platform's behavior, including `post`,
`schedule_while_dead`, `schedule_exact`, and `tap_route`. Request notification permission
through [permissions](permissions.md); localize notification text through generated accessors
before passing it to the API. See the [task guide](https://daybrite.dev/docs/how-to/guide-notifications).

## Limits and proposals

The source exposes `Unsupported`, `PermissionDenied`, and `Failed(String)` errors.
The [design record](design/notify.md) preserves the earlier platform findings and implementation
plan, including proposed recurring triggers, actions, push notifications, and a sending tool.
Do not use that record's proposed signatures as reference for the installed crate.

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

These values follow each backend's `capabilities()` implementation. Query it at runtime:
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
Android falls back to an inexact alarm. See the [scheduling guide](https://daybrite.dev/docs/how-to/guide-notifications#4-schedule-and-cancel).

## Delivery pitfalls

- iOS retains at most 64 pending local notifications. Keep a rolling schedule instead of
  submitting an unbounded set of reminders.
- Android small icons must be monochrome silhouettes. A full-color launcher image can become
  a white blob in the status bar; the generated default supplies a notification silhouette.
- A bare macOS executable has no usable notification center. Launch the generated `.app`
  bundle with `day launch` instead of running the Rust binary directly.
- Permission, Focus/Do Not Disturb settings, channel settings, and launcher behavior affect
  what the user sees. A successful `post` returns an assigned ID, not a delivery receipt.

The [notification recipe](https://daybrite.dev/docs/how-to/guide-notifications) supplies imports
and catalog entries. Test delayed delivery, app exit, and tap routing on each target you ship.
