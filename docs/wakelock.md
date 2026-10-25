---
navTitle: "Wake locks"
title: "Wake lock"
description: "Keep the screen on while something is showing, via day-part-wakelock."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Wake lock

[`day-part-wakelock`](https://daybrite.dev/api/day_part_wakelock/index.html) keeps the screen on while something is showing, whatever the device's
auto-lock setting: a timer counting down, a recipe beside the stove, a game round played with the
phone held up. It is a headless part in `parts/`.

## Add to your app

Defined in [`day-part-wakelock`](https://github.com/daybrite/day/tree/main/crates/day-part-wakelock). Add this to your app’s `Cargo.toml`
`[dependencies]` table, using the same Day Git source as the rest of your app:

```toml
[dependencies]
day-part-wakelock = { git = "https://github.com/daybrite/day.git" }
```

The implementation is selected by the compilation target; no toolkit feature flags
or dependency entry in `Day.toml` are needed.

| Platforms / toolkits | Availability |
| --- | --- |
| macOS, iOS, Android, Windows, HarmonyOS | Keeps the display awake while the lock is held. |
| Web | Requires the browser Screen Wake Lock API; the page must be visible. |
| GTK / Qt (Linux) | Not implemented; [`is_supported()`](https://daybrite.dev/api/day_part_wakelock/fn.is_supported.html) returns false. |

## Authoring

```rust
let lock = day_part_wakelock::keep_screen_on();
// The screen stays on until the lock is dropped.
drop(lock);
```

| Function | What it does |
|---|---|
| [`keep_screen_on()`](https://daybrite.dev/api/day_part_wakelock/fn.keep_screen_on.html) | keep the screen on until the returned [`ScreenLock`](https://daybrite.dev/api/day_part_wakelock/struct.ScreenLock.html) is dropped |
| [`is_held()`](https://daybrite.dev/api/day_part_wakelock/fn.is_held.html) | whether any lock is alive in the app |
| [`is_supported()`](https://daybrite.dev/api/day_part_wakelock/fn.is_supported.html) | whether this platform can keep the screen on |

Locks nest: the screen may sleep again once the last one is dropped, so two parts of an app can
each hold one without coordinating. Tie a lock to the page that needs it, so closing the page
releases it:

```rust
let lock = day_part_wakelock::keep_screen_on();
day_reactive::Scope::current().on_cleanup(move || drop(lock));
```

Take and drop locks on the UI thread, where a Day app's code runs. UIKit and Windows tie the setting
to that thread, so `ScreenLock` is not `Send`. None of the calls returns an error or panics; a
platform that cannot keep the screen on lets it sleep as usual.

A lock keeps the screen on only while the app is in front. Every platform lets the screen sleep
once the user switches away, and the part takes the setting back when the app returns.

## Per-platform realization

Platform guidance: [Android: keeping the screen on](https://developer.android.com/develop/background-work/background-tasks/awake/screen-on).

| Platform | Mechanism |
|---|---|
| iOS | [`UIApplication.isIdleTimerDisabled`](https://developer.apple.com/documentation/uikit/uiapplication) |
| macOS | an IOKit power assertion, [`PreventUserIdleDisplaySleep`](https://developer.apple.com/documentation/iokit/kiopmassertiontypepreventuseridledisplaysleep); `pmset -g assertions` lists it |
| Windows | [`SetThreadExecutionState(ES_CONTINUOUS \| ES_DISPLAY_REQUIRED)`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-setthreadexecutionstate) |
| Android | [`FLAG_KEEP_SCREEN_ON`](https://developer.android.com/reference/android/view/WindowManager.LayoutParams#FLAG_KEEP_SCREEN_ON) on the activity's window, set on the UI thread |
| HarmonyOS | [`Window.setWindowKeepScreenOn`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/arkts-apis-window-Window.md) on the app's window |
| web | the Screen Wake Lock API, requested again each time the page becomes visible |
| Linux (GTK, Qt) | none yet: [`is_supported()`](https://daybrite.dev/api/day_part_wakelock/fn.is_supported.html) is false |

None of these needs a permission. The web's wake lock needs a secure context (HTTPS or localhost),
which `day launch` and a hosted build both provide.

Android and HarmonyOS reach their window through the daybridge arms in `src/lib.rs`
([bridge](bridge.md)), and the web through its JavaScript arm; Apple and Windows call the OS
from Rust. On macOS the part links `IOKit` through `[package.metadata.day.macos]`
([extending](extending.md)).

Linux has two routes, neither built yet: GTK's [`gtk_application_inhibit`](https://docs.gtk.org/gtk4/method.Application.inhibit.html), which needs the toolkit's
application object, and the desktop's [`org.freedesktop.ScreenSaver.Inhibit`](https://specifications.freedesktop.org/idle-inhibit-spec/latest/) over D-Bus, which needs
a D-Bus client the tree does not have.
