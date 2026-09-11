---
title: "Wake lock"
description: "Keep the screen on while something is showing, via day-part-wakelock."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Wake lock

`day-part-wakelock` keeps the screen on while something is showing, whatever the device's
auto-lock setting: a timer counting down, a recipe beside the stove, a game round played with the
phone held up. It is a headless part in `parts/`.

## Authoring

```rust
let lock = day_part_wakelock::keep_screen_on();
// The screen stays on until the lock is dropped.
drop(lock);
```

| Function | What it does |
|---|---|
| `keep_screen_on()` | keep the screen on until the returned `ScreenLock` is dropped |
| `is_held()` | whether any lock is alive in the app |
| `is_supported()` | whether this platform can keep the screen on |

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

| Platform | Mechanism |
|---|---|
| iOS | `UIApplication.isIdleTimerDisabled` |
| macOS | an IOKit power assertion, `PreventUserIdleDisplaySleep`; `pmset -g assertions` lists it |
| Windows | `SetThreadExecutionState(ES_CONTINUOUS \| ES_DISPLAY_REQUIRED)` |
| Android | `FLAG_KEEP_SCREEN_ON` on the activity's window, set on the UI thread |
| HarmonyOS | `Window.setWindowKeepScreenOn` on the app's window |
| web | the Screen Wake Lock API, requested again each time the page becomes visible |
| Linux (GTK, Qt) | none yet: `is_supported()` is false |

None of these needs a permission. The web's wake lock needs a secure context (HTTPS or localhost),
which `day launch` and a hosted build both provide.

Android and HarmonyOS reach their window through the daybridge arms in `src/lib.rs`
([bridge.md](bridge.md)), and the web through its JavaScript arm; Apple and Windows call the OS
from Rust. On macOS the part links `IOKit` through `[package.metadata.day.macos]`
([extending.md](extending.md)).

Linux has two routes, neither built yet: GTK's `gtk_application_inhibit`, which needs the toolkit's
application object, and the desktop's `org.freedesktop.ScreenSaver.Inhibit` over D-Bus, which needs
a D-Bus client the tree does not have.
