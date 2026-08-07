---
title: Send local notifications
description: "Post a notification now or schedule one the OS fires later, route the tap back into your app, and get the consent step right so it actually appears."
order: 28
section: Guides
---

A timer that finished, a download that completed, a reminder that should fire after the app has
exited — `day-part-local-notify` posts and schedules the platform's own notifications from Rust.
It needs no server and no push transport; the call site is:

```rust
Notification::new("Timer done")
    .body("Your 5 minute timer finished.")
    .channel("timers")
    .route("clock/timer")
    .post()?;
```

**Works on:** iOS, macOS, and Android. The arm is picked by `target_os`, so every backend on those
OSes gets it. Linux, Windows, HarmonyOS, and the web compile the same code and report
`NotifyError::Unsupported` — gate the UI with `capabilities()` (step 6). The full design and the
per-platform details are in [the notify reference](/docs/internal/notify).

## 1. Declare the permission

Consent belongs to `day-part-permissions`, not to this crate — the two compose rather than
duplicate the prompt. The build-time half is one line in your `Day.toml`:

```toml
[permissions]
notifications = true      # needs no reason string on any platform
```

Most permissions take a user-facing reason as their value — the sentence the OS shows in its own
prompt. Notifications are the one portable permission that needs none, so `true` is enough.
`day build` generates the platform entries from this (`POST_NOTIFICATIONS` on Android 13+; Apple
needs no `Info.plist` key for local notifications).

## 2. Request consent at runtime

Declaring is not asking. An app must actually request `Permission::Notifications`, and on Apple
the failure mode for skipping this is silent: the system accepts every post and drops it with no
error. The Apple arm cannot even warn you — its settings accessor is async-only, so `post()` never
returns `PermissionDenied` there.

```rust
use day_part_permissions::{Permission, Status, request, status};

if status(Permission::Notifications) != Status::Granted {
    let set = granted.setter();       // Signal<bool> in your UI state
    request(Permission::Notifications, move |s| {
        set.set(s == Status::Granted);
    });
}
```

The callback may run on another thread, so deliver into UI state through a `Setter`. The full
flow — priming UI, `can_prompt`, the switch to Open Settings after a final denial — is
[Ask for permissions](/docs/guide-permissions).

## 3. Post one now

Every notification posts on a channel. Channels exist because Android has a real per-channel
settings model the user owns; on Apple a channel still groups notifications and carries their
importance and sound. Register the channel once, before posting to it — registration is
idempotent:

```rust
use day_part_local_notify::{Channel, Importance, Notification};

Channel::new("timers", Importance::High).sound(true).register();

let id = Notification::new("Timer done")
    .body("Your 5 minute timer finished.")
    .channel("timers")
    .post()?;
```

Pick the importance deliberately. `Importance::High` or above is what shows a heads-up banner on
Android; `Default` files the notification into the shade with no banner, which reads as "the
button did nothing". And on Android the importance is immutable after first registration — the
user owns the channel from then on — which is why it's a required argument at `Channel::new`
rather than a setter.

`post()` returns a `NotifId`. Posting again with the same id (via `.id(id)`) updates the existing
notification in place instead of stacking a second one.

## 4. Schedule and cancel

`Trigger` has two variants — `Now` (the default) and `In(Duration)`:

```rust
use day_part_local_notify::{Trigger, cancel};
use std::time::Duration;

let id = Notification::new("Stand up")
    .channel("timers")
    .trigger(Trigger::In(Duration::from_secs(20 * 60)))
    .post()?;

cancel(id);                           // or cancel_all()
```

Where `capabilities().schedule_while_dead` is true, the OS holds the trigger and fires it even if
the app has exited. On Apple the system owns the whole thing. Android has no notification
scheduler, so the part sets an `AlarmManager` alarm that wakes a receiver; a reboot clears every
alarm, and the part's boot receiver re-arms schedules from persisted data. When Android withholds
the exact-alarm grant, `capabilities().schedule_exact` is false and the notification still
arrives, possibly late in Doze.

A scheduled notification may fire in a process with no Day tree alive, so its content is
snapshotted at post time as plain strings — a `Notification` cannot bind a signal or a closure.
This is the one place Day's reactivity deliberately does not reach.

## 5. Route the tap

`.route("clock/timer")` names the Day route a tap navigates to, the same route strings the
navigation rail and deep links use. The tap is delivered through `day_core::request_route`, which
handles both cases: a warm tap navigates the running app, and a tap that cold-starts the process
is buffered and replayed once routing is live — so the app opens on the target screen, not its
default one. `capabilities().tap_route` says whether the running platform delivers taps.

## 6. Gate the UI

Query what the platform can do instead of branching on target names:

```rust
let caps = day_part_local_notify::capabilities();
caps.post                 // notifications work at all (shorthand: is_supported())
caps.schedule_while_dead  // the OS holds a schedule across app exit
caps.channels             // a user-facing channel model (Android)
caps.badge                // app-icon badge counts
caps.icon                 // custom small icons (Android)
caps.tap_route            // taps route into the app
caps.schedule_exact       // scheduled fires are on time
```

On the unwired targets every field is false and `post()` returns `NotifyError::Unsupported`, so a
notifications section can hide itself cleanly.

## Pitfalls

Three of these fail silently, which is why they lead the [reference's](/docs/internal/notify)
status notes.

- **Nothing appears, no error (Apple).** Without granted consent, the system accepts the post and
  drops it. Request `Permission::Notifications` first (step 2) — `post()` returning `Ok` is not
  proof of delivery.
- **No banner on Android.** `Importance::Default` puts the notification in the shade only. Use
  `Importance::High` or `Urgent` for a heads-up banner.
- **Foreground posts on iOS — handled for you.** iOS suppresses a notification posted while the
  app is frontmost unless a delegate opts in from `willPresent`. The crate ships that delegate; it
  shows the banner, the list entry, and the channel's sound, and its `didReceive` delivers taps.
  You don't write any of it — but if you install your own `UNUserNotificationCenterDelegate`, you
  take that job over.
- **Nothing appears from `day launch` on macOS.** `day launch` runs an unbundled binary, and
  `UNUserNotificationCenter` does not exist without a bundle identifier — the crate reports
  `Unsupported` rather than crashing. Run `day pack -p macos-appkit` and launch the `.app` to see
  real notifications in the dev loop.

## Reference

[notify](/docs/internal/notify) — the full API, the per-platform capability matrix, scheduling
internals, and the design for the platforms not yet wired.
[permissions](/docs/internal/permissions) covers the consent half, and
[Ask for permissions](/docs/guide-permissions) is the task-shaped version of it.
