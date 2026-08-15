---
title: "Notifications (proposed)"
description: "The proposed notification story: local scheduling and push, as two parts and a sending tool."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Notifications (proposed: two parts + a sending tool)

> [!NOTE]
> **Status: `day-part-local-notify` phases 1 and 2a shipped on Apple and Android; the rest is
> proposed.** What exists: the crate, its API (`Channel`, `Notification`, `Trigger`,
> `capabilities`, `cancel`), the **iOS + macOS** arm over `UNUserNotificationCenter`, and the
> **Android** arm over `NotificationManager` + `AlarmManager` with its own Java shim, alarm
> receiver, and boot receiver — no Play services, no Firebase.
>
> **Phase 2a (alarm-grade scheduling) is shipped.** `Trigger::At(SystemTime)` schedules an
> absolute instant (a past instant fires immediately). The crate now declares
> `SCHEDULE_EXACT_ALARM`, so exact alarms actually engage on Android 12+ (before, the undeclared
> permission made `canScheduleExactAlarms()` answer false everywhere and every schedule silently
> degraded); an alarm-clock app adds `USE_EXACT_ALARM` in its own metadata for the install-time
> grant — Play restricts that one to clock/calendar apps, so the crate does not impose it. A
> schedule whose channel importance is `Urgent` goes through `setAlarmClock` (Doze-exempt,
> status-bar alarm icon, the most OEM-survivable path); other exact schedules stay on
> `setExactAndAllowWhileIdle`, and the boot receiver re-arms each with the exactness it was
> scheduled with. One deliberate divergence from the design tables below: Apple realizes `At` as
> a `UNTimeIntervalNotificationTrigger` delta rather than the designed `UNCalendarNotificationTrigger`
> — `At` takes an absolute instant, so a calendar trigger adds nothing for a one-shot; calendar
> semantics come with `Every`. Still not shipped from the scheduling design: `Trigger::Every`,
> notification actions (snooze from the shade), custom/looping sounds, and full-screen intents.
>
> Verified end to end, not inferred: on the iOS Simulator the showcase reports `Posted (#1)`; on an
> Android emulator `dumpsys notification` shows the created channel (`mImportance=3`, sound set) and
> the live record `android.title=(Hello from Day)`, and the notification renders in the shade with
> the crate's monochrome icon. Both receivers are present in the merged APK manifest.
>
> Not yet implemented: the **Linux** and **web-dom** arms (they fall through to the `Unsupported`
> stub), actions and inline reply, and both `day-part-push-notify` and `day-notify`. Those sections
> below remain a design.
>
> **Three things a first user hit, all now fixed — worth knowing because they fail SILENTLY.**
> (1) Nothing requested consent, so Apple accepted every post and dropped it. Consent belongs to
> day-part-permissions (`Permission::Notifications`), and an app must actually call it.
> (2) iOS suppresses a notification posted while the app is FOREGROUND unless a
> `UNUserNotificationCenterDelegate` returns presentation options from `willPresent` — the delegate
> now does, and its `didReceive` also delivers taps, so `Cap::tap_route` is true on Apple.
> (3) Android's `IMPORTANCE_DEFAULT` files a notification into the shade with no heads-up banner,
> which reads as "the button did nothing"; `Importance::High` or above is what shows a banner.
>
> Two framework gaps this part needed are DONE and proven by it: `manifest-components` in
> `[package.metadata.day.android]` ([docs/extending.md](extending.md)), which is how the two receivers reach the
> APK, and `day_core::request_route`.
>
> macOS caveat: `day launch` runs an unbundled binary, where `UNUserNotificationCenter` does not
> exist. Use `day pack` and run the `.app`; the page reports the difference rather than failing
> silently.

Notifications are the most-requested capability missing from Day: twelve Modern Apps declare
`POST_NOTIFICATIONS`, and Clock, Email, and Messages are Blocked on some platform for want of
them. Day already asks for the `Notifications` permission ([docs/permissions.md](permissions.md)) and cannot post
one. This closes that gap.

## Two capabilities, so two parts

"Notifications" names two capabilities with different costs, and they belong in different crates.

- **Local** — the app posts or schedules a notification itself. No server, no push service, no
  network. A timer that fires, a download that finished, an alarm the OS holds while the app is
  dead. Near-universal, and what most apps need.
- **Push** — a server reaches a device whose app is not running. Needs a transport, and the
  transport differs on every platform and is absent on the desktop.

Splitting them, rather than the single `day-part-notify` an earlier draft proposed, matches Day's
existing part granularity (battery, network, clipboard are each their own crate) and pays off
concretely:

- **An app that only wants a local "download done" toast** — Files, Photos, a game — depends on
  `day-part-local-notify` alone and compiles none of the push transport machinery: no APNs
  registration, no UnifiedPush receiver, no service worker, no VAPID. Its build-time footprint is
  a runtime permission, nothing more.
- **The build-time declarations divide cleanly.** Local needs `POST_NOTIFICATIONS` and maybe an
  exact-alarm permission. Push needs entitlements, background modes, manifest receivers, and a
  VAPID key. `day build` folds in only what the dependency graph pulls, so the manifest stays
  minimal for the common case.
- **The layering is real, not cosmetic.** `day-part-push-notify` **depends on**
  `day-part-local-notify` and calls into it to display. This mirrors the platforms: a data-only
  UnifiedPush or FCM message on Android hands you a payload and you call `NotificationManager`
  yourself — the local API. An Apple background push wakes the app, which may then post a local
  notification. Push is a transport plus the local display surface.

```
day-part-push-notify   (transport: APNs / UnifiedPush / Web Push / optional FCM)
        │  on a data push, to show something, calls ──┐
        ▼                                             ▼
day-part-local-notify  (Channel, Notification, Trigger, Action, display, tap→route)
```

The rest of this document details `day-part-local-notify`, then sketches the push part and the
sending tool, which are only worth building once local exists.

---

# `day-part-local-notify`

A headless part, selected by `cfg(target_os)` like every other, callback-plus-future per the
async policy. No UI piece. It works in a plain Rust program the way the other parts do.

## The API

```rust
use day_part_local_notify::{Notification, Channel, Importance, Trigger};

// Channels are declared once. Android requires them; other platforms map them onto
// their own model (see per-platform below). Importance is fixed at registration.
Channel::new("timers", Importance::High).sound(true).register();
Channel::new("alarms", Importance::Urgent)
    .sound(true)
    .action("snooze", tr("snooze"))
    .action("stop", tr("stop"))
    .register();

// Post now. Returns a stable NotifId so a later post with the same id updates in place.
let id = Notification::new(tr("timer-done"))
    .body(tr("timer-done-body"))
    .channel("timers")
    .route("clock/timer")             // tapping navigates here (docs/navigation.md)
    .post();

// Schedule. The OS holds it where it can, so it fires even if the app has exited.
Notification::new(tr("wake-up"))
    .channel("alarms")
    .trigger(Trigger::At(seven_am))   // ::In(Duration) | ::At(SystemTime) | ::Every(Duration)
    .post();

cancel(id);                           // remove a pending or scheduled notification
cancel_all();
let pending: Vec<NotifId> = pending(); // what is still scheduled
```

Types:

- **`Channel`** carries an `Importance` (`Min`, `Low`, `Default`, `High`, `Urgent`), an optional
  sound, a group key, and the action buttons every notification on that channel offers.
  `register()` is idempotent and must be called before posting to the channel.
- **`Notification`** is a builder: title, optional body and subtitle, the channel id, an optional
  badge count, a per-notification sound override, a `.route()` for tap, and an `.id()` for
  update/cancel (generated if omitted).
- **`Trigger`** is `Now` (the default), `In(Duration)`, `At(SystemTime)`, or `Every(Duration)`.

Delivery comes back through the standard event sink, so no new channel is invented:

- **A tap** emits `Event::RouteRequested` with the notification's route; the app navigates through
  the same rail deep links and dayscript already use.
- **An action** emits `Event::Custom { tag: "notify:action", text: <action-id> }`. For an inline
  reply action (`Channel::action_reply`), the typed text rides alongside.

Capabilities are a struct an app queries rather than branching on target name:

```rust
let c = capabilities();
c.schedule_while_dead   // OS holds a scheduled notification (Apple, Android, Windows, Harmony)
c.channels              // native channel model (Android)
c.actions               // action buttons
c.inline_reply          // typed reply from the notification
c.badge                 // app-icon badge count
```

## Per-platform realization

### Apple (iOS, macOS) — `UNUserNotificationCenter`

One framework covers both OSes and both immediate and scheduled. Immediate: build a
`UNMutableNotificationContent` (title, subtitle, body, sound, badge, `userInfo` carrying the Day
route, `categoryIdentifier` = the channel), wrap it in a `UNNotificationRequest` with a `nil`
trigger, and `add` it. Scheduled: a `UNTimeIntervalNotificationTrigger` (`In`/`Every`) or
`UNCalendarNotificationTrigger` (`At`), which the OS holds and fires while the app is dead — this
is what lets a Clock alarm ring without a background process. Channels map to interruption level
(`Importance::Urgent` → `.timeSensitive`, needing the time-sensitive entitlement) and
`threadIdentifier` for grouping. Actions are a `UNNotificationCategory` of `UNNotificationAction`
/ `UNTextInputNotificationAction`, registered with `setNotificationCategories` at launch. Taps and
actions arrive at the `UNUserNotificationCenterDelegate` (`didReceive response`); foreground
presentation is decided in `willPresent`, which the part defaults to showing. Native half: raw
`objc2` + `objc2-user-notifications`, the CoreLocation budget from `day-part-location`. **Local
notifications need no entitlement** — only push needs `aps-environment` — so the local part is
lighter than the push part on Apple.

### Android — `NotificationManager` + `AlarmManager`, no Google

Immediate: a Java shim builds the `NotificationChannel` once (API 26+) and calls
`NotificationManagerCompat.notify(id, notification)`. No Play Services, so it runs on any AOSP
build, GrapheneOS, or a Kindle. Scheduling is not OS-held the way Apple's is — Android has no
notification scheduler, so the part schedules an **`AlarmManager` alarm** that wakes a manifest
`<receiver>`, which posts the notification from data persisted at schedule time. `setAlarmClock`
for user alarms (Doze-exempt, shows the status-bar alarm icon) and `setExactAndAllowWhileIdle` for
other exact triggers; inexact `set` for the rest. Actions are `Notification.Action` with a
`PendingIntent` (`FLAG_IMMUTABLE`); inline reply is `RemoteInput` on the action, delivered to a
receiver that emits the `notify:action` event with the reply text. The receiver runs in a fresh
process with no Day tree alive, so it posts directly through the shim — the notification content
must be fully materialized at schedule time (see pitfalls). The shim, its receiver, and the
boot receiver are declared through `[package.metadata.day.android]`.

### Linux (GTK, Qt on Linux) — `org.freedesktop.Notifications`

The desktop notification spec, over the session D-Bus: `Notify(app_name, replaces_id, icon,
summary, body, actions, hints, timeout)`, with `ActionInvoked` and `NotificationClosed` signals
for tap and action routing. Two implementation tiers, matching how `day-part-clipboard` handles a
missing native API: first cut shells out to `notify-send` (from libnotify-bin, common but not
guaranteed), which shows the notification but gives no action callbacks or stable id; the full
path speaks the D-Bus wire protocol directly over `$DBUS_SESSION_BUS_ADDRESS` (the EXTERNAL auth
handshake plus message marshalling, std only) to get actions, tap, and replace-by-id. The portable
path is deliberately **not** `Gio.Notification`/`GApplication.send_notification`: that needs
`GApplication` and an installed `.desktop` file and does not work in a `day-qt` binary, the same
toolkit-independence reason clipboard avoids GDK. There is **no OS-held scheduler** — a running
Day process schedules in-process and calls `Notify` at fire time; if the process exits, a
scheduled notification is lost.

### Windows (XAML) — `ToastNotification`

`Windows.UI.Notifications` through the XAML shim. Immediate toasts are toast-XML built and shown
via `ToastNotificationManager.CreateToastNotifier(aumid)`; `ScheduledToastNotification` gives
OS-held scheduling that fires while the app is closed, like Apple. Actions and inputs are
`<action>`/`<input>` in the toast XML. The pitfall is unpackaged-app registration (below): a Win32
XAML-islands host must register a Start Menu shortcut carrying an AppUserModelID and a COM
activator to receive taps and actions, which the shim does at first run.

### HarmonyOS (ArkUI) — Notification Kit

`OH_NotificationManager` through the C node API for immediate notifications; notification slots
map to channels. Scheduled notifications use the reminder agent, which the C API exposes partially,
so scheduling may ship as an ArkTS half (`[package.metadata.day.ohos]`) like the webview piece.
Local first; anything the C API cannot reach is deferred, not faked.

### Web (web-dom) — the Notification API, foreground only

`new Notification(title, options)` while the page is open, or `ServiceWorkerRegistration
.showNotification()` through the day-dom shim; taps arrive at the service worker's
`notificationclick`. There is **no scheduling from a closed tab** — the Notification Triggers
(`showTrigger`) proposal is Chromium-only and effectively abandoned, so the part does not rely on
it — and no delivery at all while the tab is closed without the push part. A running page can post
immediately; that is the honest extent of local notifications on the web.

### mock

Records `notify`/`schedule`/`cancel` ops and answers a deterministic `capabilities()`, so the
whole flow is unit-testable on `day-mock` without a display, matching every other part.

## Capability matrix (local)

`N` native, `E` emulated, `–` unsupported.

| capability | macos | ios | android | linux | windows | harmony | web |
|---|---|---|---|---|---|---|---|
| post now | N | N | N | N | N | N | N |
| schedule while app is dead | N | N | N¹ | –² | N | N | –³ |
| channels | E | E | N | E | E | N | E |
| action buttons | N | N | N | N | N | N | N |
| inline reply | N | N | N | – | N | – | – |
| badge count | N | N | N⁴ | – | N | – | – |
| tap → Day route | N | N | N | N⁵ | N | N | N |

1. `AlarmManager`, not an OS-held queue — and increasingly restricted (see pitfalls).
2. A running process can schedule in-process; a dead one cannot.
3. No closed-tab scheduling; a running page can post now.
4. Android badge support is launcher-dependent (`setNumber` + a badge-capable launcher).
5. Requires a `.desktop` file for some daemons to route actions back.

## Build-time declarations (local is light)

Through the existing machinery ([docs/permissions.md](permissions.md), §15.2), and only what the app uses:

- **Always**: `POST_NOTIFICATIONS` (Android 13+) and the notification permission prompt, via
  `day-part-permissions` (`Permission::Notifications`) — the parts compose, they do not duplicate
  consent.
- **Only if the app schedules**: Android `SCHEDULE_EXACT_ALARM`/`USE_EXACT_ALARM`,
  `RECEIVE_BOOT_COMPLETED`, and the `<receiver>`s; add `USE_FULL_SCREEN_INTENT` only for
  alarm-style full-screen notifications.
- **Apple**: nothing for local, except the time-sensitive entitlement if a channel uses
  `Importance::Urgent`.
- **Windows**: the Start Menu shortcut + AUMID + COM activator, written at first run by the shim.

The app declares intent in `Day.toml`, and `day lint` flags a `post()`/schedule call whose
manifest bits were not declared, catching the mismatch in CI rather than on a device:

```toml
[notify]
local = true
schedule = true          # pulls in the exact-alarm + boot-receiver bits on Android
full-screen = false      # alarm-style takeover; restricted on Android 14+
```

## Pitfalls

The reason a cross-platform notification part is more than a thin wrapper. These are the traps,
each with how the part handles it.

- **Scheduled content is eager, not reactive.** Everywhere the OS holds a scheduled notification
  (Apple, Android via the receiver, Windows), the Day tree may not exist when it fires, so the
  content is snapshotted at schedule time — a scheduled `Notification` cannot bind a signal or a
  closure the way live UI does. The part enforces this in the type: `Trigger::At`/`Every` take
  resolved strings, and this is documented as the one place Day's reactivity does not reach.
- **Localized text: schedule-time vs fire-time locale.** A notification scheduled in English and
  fired after the user switched to French shows English on Android (the string is baked into the
  alarm data) but could show French on Apple (which can resolve a localized key at fire time). The
  part picks one rule — **bake at schedule time** — so behavior is identical everywhere, and
  documents that a locale change between schedule and fire keeps the scheduled locale. Predictable
  beats clever.
- **iOS silently keeps only 64 pending notifications.** Schedule a 65th and the OS drops the
  furthest-out one with no error. A calendar or alarm app that wants more must maintain a rolling
  window and re-arm as notifications fire. The part exposes `pending()` and documents the limit;
  it does not paper over it.
- **Android channel importance is immutable after first registration.** You cannot raise or lower
  a channel's importance in code once it exists — the user owns it thereafter. Picking it wrong
  means either living with it or creating a new channel id (orphaning the old one's user
  settings). The API makes importance a required argument at `Channel::new` to force the decision
  up front, and the docs say plainly that changing it later is not possible.
- **Android exact alarms are restricted and getting more so.** `SCHEDULE_EXACT_ALARM` is
  auto-granted but revocable on Android 12–13, and on Android 14 it is not granted to a general
  app at all unless it declares itself a clock/calendar via `USE_EXACT_ALARM`. The part checks
  `canScheduleExactAlarms()`, and on denial either falls back to an inexact alarm (documented as
  "may fire late in Doze") or surfaces the settings deep link for the app to prompt — it never
  silently drops the alarm.
- **A reboot wipes every Android alarm.** The OS clears all pending alarms on restart. Scheduled
  notifications survive only if the part persists them and a `BOOT_COMPLETED` receiver re-arms
  them. The part owns this: schedule data is written to app-local storage and re-registered on
  boot, so `schedule = true` pulls in the boot receiver automatically.
- **OEM battery killers.** Xiaomi, Huawei, Samsung, and others aggressively kill background work,
  and no code fixes it. The part documents it and points at `setAlarmClock` (the most survivable
  path) rather than pretending an exact alarm is a hard guarantee.
- **Android's small icon must be a monochrome silhouette.** A full-color icon renders as a white
  square. The part requires a declared notification icon resource (`res::images::notify`, §18) and
  defaults to a silhouette of the app icon, so the white-blob failure cannot happen by omission.
- **macOS drops local notifications from an unsigned or unbundled binary, silently.** They need a
  signed `.app` with a bundle identifier. `day pack` produces one, but `day launch` in the dev
  loop may run a bare binary that shows nothing — a confusing "works after pack, not in dev"
  failure. `day doctor` should report it, and the docs call it out.
- **Windows unpackaged apps need a Start Menu shortcut and a COM activator.** Without an
  AppUserModelID-carrying shortcut and a registered COM callback, toasts either do not appear or
  do not route taps back. The shim registers both at first run; the pitfall is that this touches
  the user's Start Menu, which is documented.
- **Linux has no scheduler and an inconsistent daemon.** `notify-send` may be absent, gives no
  callbacks, and different daemons handle actions and `.desktop` matching differently. Scheduling
  only works while the process runs. The part states this rather than implying parity with Apple.
- **The cold-launch tap must be buffered.** Tapping a notification that launches a dead app
  produces the tap before the Day root exists. The part reads the launch payload (Apple's delegate
  / `didFinishLaunching`, Android's `Intent` extras in `DayActivity`, Windows COM activation args),
  buffers it, and replays it as `Event::RouteRequested` once routing is ready. Without this, a tap
  opens the app to its default screen instead of the target — the single most common notification
  bug, designed out.
- **Do not auto-prompt for permission at launch.** iOS guidance and user trust both argue for
  asking at a natural moment. The part offers `request()` but never prompts on its own, leaving the
  timing to the app.

## Phasing (local)

1. **Post now, plus channels, actions, and tap-to-route**, on Apple, Android, Linux, Windows, and
   web-when-open. This alone serves every app whose notifications are self-posted — Files, Photos,
   a completion alert.
2. **Scheduling** on Apple, Android (with the alarm/boot/exact machinery), and Windows. This is
   what Clock needs, with the iOS caveat that a scheduled notification shows but cannot loop audio.
3. **Inline reply and badges** where the platform supports them; HarmonyOS scheduling.

---

# `day-part-push-notify` (layered on local)

Push is opt-in, depends on `day-part-local-notify` for display, and returns a **routing token**
the app ships to its own backend. Day provides the client half and the sending tool; it never runs
the server.

```rust
use day_part_push_notify as push;

let token = push::register();          // Signal<Option<PushToken>>, may rotate
day::watch(move || if let Some(t) = token.get() {
    upload_token(t.as_str());          // to YOUR server, over day-part-http
});
push::on_message(|msg| store.merge(msg.data));   // data pushes, when the app is alive
```

A push either **shows a notification** — drawn by the OS, or by the local part from the payload on
a data push, no app code — or **wakes the app** into `on_message`. The token is opaque and
scheme-tagged so the sender routes it without the app knowing the transport: `apns:`,
`unifiedpush:`, `webpush:`, `fcm:`.

Transports, by platform, none requiring Google as a hard dependency:

- **Apple**: APNs. Register with `registerForRemoteNotifications`; the server sends over APNs
  HTTP/2 with token auth (a `.p8` JWT). Needs the `aps-environment` entitlement — the reason push
  is a heavier part than local.
- **Android**: **UnifiedPush first** (a user's distributor app — ntfy, NextPush — holds one
  connection; Day registers over broadcast intents and gets an HTTP endpoint as its token; the
  server pushes by POSTing to it — the GrapheneOS-friendly path), a **self-hosted foreground
  connection** as fallback when no distributor is installed (the Modern Apps `messages` model,
  quarantined like `matrix-core`), and **FCM as an optional, feature-gated** transport for
  stock-Android battery life — never in the default build, never a `google-services.json` unless
  the app opts in. The part probes and picks the best available, so one APK degrades FCM →
  UnifiedPush → self-hosted without a rebuild.
- **Web**: the Web Push API + VAPID (RFC 8030 + 8292) via a service worker; the `PushSubscription`
  is the token. Vendor-neutral — Safari, Firefox, and Chromium speak the same protocol.
- **Desktop**: generally none (Windows `WNS` needs a Store identity, macOS push needs a
  provisioning profile). Local notifications cover the desktop.

# `day-notify` (the sending tool)

The logic lives in a `day-notify` library crate, exposed as a `day notify` porcelain subcommand
for developers and as a standalone thin binary (`cargo install day-notify`, no toolchain) for
servers. It abstracts APNs HTTP/2 (JWT from a `.p8`), Web Push (RFC 8291 encryption + VAPID),
UnifiedPush (a plain POST), and optional FCM (HTTP v1 + service account) behind one command,
choosing the transport from the token's scheme. Configuration (`day-notify.toml`) holds the
per-transport credentials, which live on the server and never in the app. The payload type is the
same one `day-part-local-notify` renders, so what the tool sends and what the app shows are one
contract, and a batch send returns per-token results (a 410 means "drop this stale token") so a
server can prune its table.

```sh
day notify send --to "apns:9c8b…" --title "Build done" --body "macos-appkit passed" \
    --channel builds --route "ci/run/42" --data run=42
```

---

# Implementation plan — `day-part-local-notify`

## Crate configuration

Modelled on `day-part-location`'s manifest (the closest existing part: per-OS halves, a Java
shim, an Apple framework). Platform selection is `cfg(target_os)` with no backend features,
because a notification is an OS concern, not a widget-toolkit one.

```toml
[package]
name = "day-part-local-notify"
publish = false
version.workspace = true
edition.workspace = true
rust-version.workspace = true
description.workspace = true
repository.workspace = true
license.workspace = true

# A HEADLESS day-ecosystem crate (no UI Piece): post and schedule the platform's own
# notifications. Any Rust code can depend on it and call `day_part_local_notify::post(...)`.
# Platform selection is by `#[cfg(target_os)]` — there are no backend features:
#   iOS/macOS — UNUserNotificationCenter (immediate + OS-held scheduling), delegate for taps
#   Android   — NotificationManager via a Java shim; scheduling is AlarmManager + a receiver,
#               because Android has NO notification scheduler. Re-armed on BOOT_COMPLETED.
#   Windows   — ToastNotification / ScheduledToastNotification through the day-xaml-sys shim
#   Linux     — org.freedesktop.Notifications over the session D-Bus (std-only, no dbus crate);
#               no OS-held scheduler, so scheduling only works while the process runs
#   HarmonyOS — Notification Kit; scheduling may need an ArkTS half (see below)
#   Web       — the Notification API through the day-dom shim; foreground only, no scheduling
# See docs/notify.md + docs/extending.md.
#
# PERMISSIONS. This crate does not ask for any: a denial surfaces as
# `NotifyError::PermissionDenied` and the app requests access through day-part-permissions
# (`Permission::Notifications`). Keeping them separate means neither crate depends on the other —
# the same split day-part-location keeps.

[dependencies]
day-reactive = { workspace = true }   # Signal for `pending()`/`last_action`, on_main marshalling

[target.'cfg(any(target_os = "macos", target_os = "ios"))'.dependencies]
objc2 = "0.6"
objc2-foundation = { version = "0.3", features = ["NSObject", "NSDictionary", "NSDate"] }
# The one framework wrapper: the delegate protocol and the content/trigger/request classes are
# too broad to hand-roll with `msg_send!` the way day-part-battery does for UIDevice.
objc2-user-notifications = "0.3"

[target.'cfg(target_os = "android")'.dependencies]
day-android = { workspace = true }

# --- Android backend contribution (docs/extending.md) ---
[package.metadata.day.android]
java = ["android/java"]
res = ["android/res"]            # the monochrome default small icon (see pitfalls)
# POST_NOTIFICATIONS is the runtime permission the APP declares via day-part-permissions; the
# scheduling permissions are structural (no user-facing prompt, no reason string), so the crate
# contributes them and `day build` merges them into the overlay.
permissions = ["android.permission.RECEIVE_BOOT_COMPLETED"]
proguard = ["android/proguard-rules.pro"]   # receivers are resolved by name from the manifest

# --- PROPOSED, see "Two framework gaps" below: manifest components a part must contribute ---
# manifest-components = ["android/components.xml"]

[package.metadata.day.ios]
frameworks = ["UserNotifications"]

[package.metadata.day.macos]
# Consumed since 2026-08 by the macOS Swift leg (docs/swiftui.md): `-framework` link args when a
# build has Swift contributions. This part links UserNotifications through objc2 either way, so
# the declaration is documentation-plus-belt here rather than the working mechanism.
frameworks = ["UserNotifications"]

[package.metadata.day.permissions]
uses = ["notifications"]         # machine-facing: WHICH permission, never the user-facing reason
```

## Two framework gaps this part is the first to hit

Both are small, and both should land before the part does.

**1. A part cannot contribute manifest components.** `AndroidMeta` in
`crates/day-cli/src/pieces.rs` accepts `java`, `res`, `gradle-dependencies`,
`gradle-repositories`, `permissions`, and `proguard` — there is no way to declare a
`<receiver>`. Scheduled notifications need three (alarm, action, boot). Proposed: a
`manifest-components` key naming an XML fragment holding `<receiver>`/`<service>` elements, which
`day build` merges into the same generated overlay `permissions` already flows through. It is
additive, `deny_unknown_fields`-compatible as a new field, and other parts will want it (a future
push part needs a receiver; a background-work part would need a service).

**2. Nothing could request a route from off the UI thread, or before launch.** An earlier draft of
this document said day-core needed a buffered launch-route slot. Implementing it showed that was
wrong: `nav::set_launch_deeplink` already existed and `launch_with` already consumed it one turn
after the first mount (web-dom seeds it from the URL hash). Two things were genuinely missing, and
both bite exactly the notification case:

- **`set_launch_deeplink` writes a thread-local.** A notification tap arrives on a JNI thread on
  Android and a delegate callback on Apple, so calling it from there would set the slot on the
  wrong thread and the route would vanish.
- **`day_reactive::on_main` panics when no poster is installed**, which is precisely the state a
  tap that cold-starts the process arrives in — so glue could not simply post the navigation.

Shipped instead: **`day_core::request_route(route)`**, thread-safe and lifecycle-agnostic. It
writes a process-global buffer and, only if a backend has installed the poster
(`day_reactive::has_main_poster()`, also new), posts a drain onto the UI thread. Before launch the
buffer is picked up by `launch_deeplink()` — peeked, not taken, so `has_launch_deeplink()` keeps
answering true and a tap still beats restored navigation state — and `launch_with` consumes it
after the first mount. Cold start and warm tap are the same call from the caller's side.

What each backend still has to do is small: read its platform's launch payload (Apple's delegate
response, Android's `Intent` extras in `DayActivity.onCreate`/`onNewIntent`, Windows COM
activation args, the web's `notificationclick` postMessage) and call `request_route`.

## Shared crate layout

```
day-part-local-notify/
  Cargo.toml
  README.md
  src/
    lib.rs          # public API, NotifId, Channel, Notification, Trigger, capabilities(); dispatch
    types.rs        # Importance, Action, NotifyError, Payload (serializable, shared with schedule)
    store.rs        # persisted schedule records (day-part-fs), used by Android boot re-arm
    apple.rs        # ios + macos
    android.rs
    linux.rs
    windows.rs
    ohos.rs
    web.rs
    unsupported.rs  # catch-all: every call returns NotifyError::Unsupported
  android/
    java/dev/daybrite/day/notify/DayLocalNotify.java
    java/dev/daybrite/day/notify/DayNotifyAlarmReceiver.java
    java/dev/daybrite/day/notify/DayNotifyActionReceiver.java
    java/dev/daybrite/day/notify/DayNotifyBootReceiver.java
    res/drawable/ic_day_notify.xml      # monochrome silhouette default
    components.xml                      # the three <receiver> declarations
    proguard-rules.pro
  examples/notify.rs                    # plain `main`, no Day framework — the part convention
```

`Payload` is the pivot type: a serializable snapshot of everything a notification needs to render
(title, body, channel, route, icon, badge, actions). It is what `post()` renders immediately and
what `schedule()` persists, which is what makes the "scheduled content is eager" rule a type-level
fact rather than a documentation note.

## Per-platform implementation

### Apple — `src/apple.rs`, shared by ios and macos

| Concern | Realization |
|---|---|
| Handle | `UNUserNotificationCenter::currentNotificationCenter()` |
| Channel | `UNNotificationCategory(identifier:actions:intentIdentifiers:options:)`, installed as a set via `setNotificationCategories:`. Importance maps to `interruptionLevel` (`Urgent` → `.timeSensitive`, which needs the time-sensitive entitlement) |
| Content | `UNMutableNotificationContent`: `title`, `subtitle`, `body`, `sound` (`UNNotificationSound.defaultSound`), `badge` (`NSNumber`), `categoryIdentifier`, `threadIdentifier` (grouping), `userInfo` carrying `day.route` |
| Trigger | `nil` for `Now`; `UNTimeIntervalNotificationTrigger` for `In`/`Every` (repeats ≥ 60 s, enforced by the OS); `UNCalendarNotificationTrigger` from `NSDateComponents` for `At` |
| Post | `UNNotificationRequest(identifier:content:trigger:)` → `addNotificationRequest:withCompletionHandler:` |
| Cancel | `removePendingNotificationRequestsWithIdentifiers:` + `removeDeliveredNotificationsWithIdentifiers:` |
| Pending | `getPendingNotificationRequestsWithCompletionHandler:` — **async**, so the API is `pending_async`/`pending_future`, with a `Signal<Vec<NotifId>>` mirror the UI can bind |
| Tap / action | A delegate defined with `objc2::define_class!` implementing `UNUserNotificationCenterDelegate`: `userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:` for taps and actions (`actionIdentifier`, plus `userText` on a `UNTextInputNotificationResponse`), and `willPresentNotification:` returning `.banner | .sound | .list` so a notification shows while the app is foreground |

The delegate must be installed **before** the app finishes launching, or the cold-launch response
is dropped. The part registers it from a `WillLaunch` lifecycle hook ([docs/lifecycle.md](lifecycle.md)) and
buffers the first response until routing is live. macOS additionally requires a signed, bundled
`.app` with a bundle identifier — `day pack` produces one, `day launch` may not, so `day doctor`
should report "local notifications need a signed bundle" on macOS rather than leaving a silent
no-show.

### Android — `src/android.rs` + the Java shim

Immediate posting is the easy half: `DayLocalNotify.createChannel(...)` once per channel
(`NotificationChannel`, API 26+, importance immutable after first registration), then
`NotificationManagerCompat.notify(id, builder.build())` with `setSmallIcon`, `setContentTitle`,
`setContentText`, `setAutoCancel(true)`, `setContentIntent(PendingIntent)`, and one
`Notification.Action` per channel action (`FLAG_IMMUTABLE`, plus `RemoteInput` on a reply action).
No Play Services anywhere, so it runs on AOSP, GrapheneOS, or a Kindle.

Scheduling is the hard half, because **Android has no notification scheduler**. The flow:

1. `schedule()` serializes the `Payload` and writes a record through `day-part-fs`
   (`notify/scheduled/<id>.json`), so it survives process death and reboot.
2. It sets an `AlarmManager` alarm whose `PendingIntent` targets `DayNotifyAlarmReceiver`, choosing
   the strongest API the app is entitled to: `setAlarmClock` for `Importance::Urgent` (Doze-exempt,
   shows the status-bar alarm icon, best OEM survival), `setExactAndAllowWhileIdle` for other exact
   triggers, `set`/`setInexactRepeating` otherwise.
3. `canScheduleExactAlarms()` is checked first. On denial the part either downgrades to inexact
   (returning `Scheduled::Inexact` so the app can tell the user "may fire late") or hands back the
   `ACTION_REQUEST_SCHEDULE_EXACT_ALARM` settings deep link. It never silently drops the alarm.
4. `DayNotifyAlarmReceiver` fires in a fresh process with **no Day tree alive**. It reads the
   record and posts through the same shim — which is precisely why the payload must be fully
   materialized at schedule time.
5. `DayNotifyBootReceiver` re-arms every persisted record on `BOOT_COMPLETED` and
   `MY_PACKAGE_REPLACED`, because a reboot clears all alarms.

Taps use a `PendingIntent` into `DayActivity` carrying a `day.route` extra; day-android reads it in
`onCreate`/`onNewIntent` and feeds the buffered launch-route slot. Actions go to
`DayNotifyActionReceiver`, which — when the process is alive — emits
`Event::Custom { tag: "notify:action", … }` through the JNI bridge, and when it is not, performs
the record's declared side effect and cancels the notification. `android/res` ships a monochrome
`ic_day_notify.xml` as the default small icon so the white-square failure cannot happen by
omission; an app overrides it with a `res::images` handle.

### Linux — `src/linux.rs`

Two tiers, mirroring how `day-part-clipboard` handles a missing toolkit-independent API:

- **Tier 1**: shell out to `notify-send`. Shows the notification, gives no action callbacks and no
  stable id. Used when the D-Bus socket is unavailable.
- **Tier 2 (target)**: speak `org.freedesktop.Notifications` directly over the session bus from
  `$DBUS_SESSION_BUS_ADDRESS` — EXTERNAL auth (`AUTH EXTERNAL <hex uid>`, `BEGIN`), then a method
  call to `Notify(app_name, replaces_id, app_icon, summary, body, actions, hints, expire_timeout)`,
  with a match rule and a listener for the `ActionInvoked` and `NotificationClosed` signals.
  `replaces_id` gives update-in-place; the `urgency` hint (0/1/2) carries importance. std only, no
  D-Bus crate — the tree does not have one and this does not justify adding one.

Deliberately **not** `Gio.Notification`/`GApplication.send_notification`: that needs a
`GApplication` and an installed `.desktop` file, and it would not work in a `day-qt` binary — the
same toolkit-independence reason clipboard avoids GDK. There is no OS-held scheduler, so `schedule`
arms a timer through `Platform::post_delayed` and is documented as lost if the process exits.

### Windows — `src/windows.rs` + the day-xaml-sys shim

`Windows.UI.Notifications` through the existing C++/WinRT shim. Toast content is `ToastGeneric`
XML built as a string (`<text>`, `<image>`, `<action>`, `<input>` for reply);
`ToastNotificationManager.CreateToastNotifier(aumid).Show(toast)` posts, and
`ScheduledToastNotification(xml, deliveryTime)` + `AddToSchedule` gives OS-held scheduling that
fires while the app is closed. `GetScheduledToastNotifications` backs `pending()`.

The pitfall is unpackaged activation: a Win32 XAML-islands host must have a Start Menu shortcut
carrying an `AppUserModelID` and a `ToastActivatorCLSID`, plus a registered COM class implementing
`INotificationActivationCallback`, or taps and actions never reach the app. The shim writes the
shortcut and registers the CLSID at first run; that it touches the user's Start Menu is documented
rather than hidden.

### HarmonyOS — `src/ohos.rs`

Immediate notifications through the Notification Kit C API (`OH_Notification_*`,
`Notification_NotificationRequest`), with notification slots mapping to channels. **Flagged as
uncertain**: the NDK C surface for notifications is narrower than the ArkTS one, and scheduled
notifications go through the reminder agent, which may be ArkTS-only. If it is, the scheduling half
ships as an ArkTS source dir through `[package.metadata.day.ohos].ets`, the same route
`day-piece-webview` takes for the `Web` component. Immediate first; whatever the C API cannot reach
is deferred and reported as `Unsupported`, not faked.

### Web — `src/web.rs` + the day-dom shim

`new Notification(title, options)` while the page is open, or
`ServiceWorkerRegistration.showNotification()` when actions are wanted (the plain constructor
supports none). New shim imports (`day_dom_notify_post`, `day_dom_notify_cancel`,
`day_dom_notify_permission`) follow the `day-part-prefs`/`day-part-location` web-arm pattern. Taps
arrive at the service worker's `notificationclick`, which `postMessage`s to the page, which calls
back into wasm. `capabilities()` reports no `schedule_while_dead`; a running page schedules through
the delayed poster. The service worker must be a served file, not a bundled asset, since
`resource()` returns `None` on web-dom ([docs/web.md](web.md)).

### mock

Records `notify`/`schedule`/`cancel` ops for assertion and answers a deterministic
`capabilities()`, so the whole flow is unit-testable on `day-mock` with no display — the same
contract every other part keeps.

---

# The showcase "Notifications" page

A new page under `Day-Showcase/src/pages/notify.rs`, registered in `destinations()` in
alphabetical position (between Menus and Preferences), with `Section::Notify => "notify"`. It
follows the `services.rs` shape: `page(title, id, caption, form((section, …)))`, every string a
`crate::res::str::*` key, every control an `.id()` so dayscript can drive it.

Its job is to make the *capability differences* visible, not to hide them — the showcase's
purpose. The first section is a live capability readout, and controls the backend cannot honor are
disabled rather than silently ignored.

### Sections and controls

**1. Capabilities** — a read-only grid bound to `capabilities()`: post now, schedule while dead,
channels, actions, inline reply, badge. Each row shows Native / Emulated / Unsupported for the
running backend. This is the page's honest header, and it is what a screenshot of this page on
eight backends is actually worth looking at.

**2. Compose** — the message itself:
- `text_field` title (`notify-title`), default from a res string so it localizes.
- `text_area` body (`notify-body`), 2–4 lines.
- `text_field` subtitle (`notify-subtitle`), disabled where the platform has no subtitle.

**3. Delivery** — when and how loudly:
- `picker` delay (`notify-delay`): Now / 5 s / 30 s / 1 minute / 1 hour. Values above "Now" are
  disabled when `capabilities().schedule_while_dead` is false **and** the platform cannot even
  schedule in-process, with a footnote naming why.
- `picker` importance (`notify-importance`): Min / Low / Default / High / Urgent, disabled on
  backends with no importance model, with a caption noting Android fixes importance at channel
  registration so the page registers five channels up front rather than mutating one.
- `toggle` sound (`notify-sound`).

**4. Presentation** — the metadata:
- `picker` icon (`notify-icon`) over three bundled monochrome glyphs plus "app default",
  demonstrating `res::images` and the Android silhouette rule.
- `slider` badge count 0–9 (`notify-badge`), disabled where unsupported.
- `text_field` group key (`notify-group`) — `threadIdentifier` on Apple, group on Android, `tag` on
  the web.
- `picker` route (`notify-route`) over a few real showcase routes, so tapping the notification
  navigates and the tap-routing rail is demonstrable rather than asserted.
- `toggle` actions (`notify-actions`) adding "Snooze" and a "Reply" inline-text action where
  supported.

**5. Post and manage**:
- `button` Post (`notify-post`), `button` Cancel all (`notify-cancel`).
- `label` status (`notify-status`) — posted, scheduled with the fire time, or the `NotifyError`.
- `label` pending count (`notify-pending`), bound to the `pending()` signal.
- `label` last action (`notify-last-action`), showing the id and any reply text that came back,
  which is how the event round-trip becomes visible in a screenshot.

### Metadata configuration

`Day-Showcase/Day.toml` already declares `notifications = true` under `[permissions]`, so consent
is covered. The page adds:

```toml
[notify]
local = true
schedule = true        # pulls in the Android exact-alarm + boot-receiver bits
full-screen = false    # the showcase does not demonstrate alarm-style takeover
```

Per platform, `day build` then folds in: `POST_NOTIFICATIONS` + `RECEIVE_BOOT_COMPLETED` + the
three `<receiver>`s + the proguard keeps on Android; the `UserNotifications` framework on
iOS/macOS (no entitlement, since no channel uses `Urgent`… and if the page's Urgent option is
exercised, the time-sensitive entitlement is required, which is itself worth demonstrating and
should be declared); the AUMID shortcut on Windows; the service worker on web-dom. Four new locale
blocks in `resource/locales/{en,fr,ar,zh-CN}/app.ftl` — the showcase requires all four, and
`day lint` enforces cross-locale coverage.

### Walkthrough, and what it can honestly assert

A `dayscript/notify-post.yaml` navigates to the page, fills the fields, posts, and asserts the
in-app status and pending count, then captures a screenshot. Added to `walkthrough.yaml` with
`skip_on: [web-dom]` for the scheduling steps.

**What it cannot assert is that the OS actually displayed anything.** A notification is drawn
outside the app's window, so `snapshot_window` will not contain it and a green walkthrough proves
only that the part returned success. Verifying real delivery means looking at a device — a
simulator notification banner, an emulator shade pull, a desktop toast. The page and its script
should say so, and the per-backend gallery caption should not imply otherwise. This is the same
trap as asserting native menus through a script that bypasses them: the assertions are worth
having, and they are not evidence of delivery.

## Phasing

1. **The two framework gaps** (`manifest-components`, buffered launch route), since the part
   cannot be correct on Android or route a tap anywhere without them.
2. **Post now** on Apple, Android, Linux, Windows, and web, plus channels, actions, and
   tap-to-route. The showcase page lands here, capability-gated, and is immediately meaningful.
3. **Scheduling** on Apple, Android (alarms, persistence, boot re-arm), and Windows. The page's
   delay picker becomes fully live; Clock becomes portable.
4. **Inline reply, badges, HarmonyOS**, and whatever the ArkTS half turns out to require.
