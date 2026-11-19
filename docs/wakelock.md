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

A wake lock asks the system to keep the display active during an ongoing task. It is useful when
people need to keep viewing content without touching the device, such as following a recipe or
watching a timer.

[`day-part-wakelock`](https://docs.rs/day-part-wakelock/latest/day_part_wakelock/index.html)
provides this capability independently of the app’s UI. Availability and system policy vary by
platform.

## Add to your app

Add [`day-part-wakelock`](https://github.com/daybrite/day/tree/main/crates/day-part-wakelock) to your app:

```sh
cargo add 'day-part-wakelock@0.*'
```

The implementation is selected by the compilation target; no toolkit feature flags
or dependency entry in `Day.toml` are needed.

| Platforms / toolkits | Availability |
| --- | --- |
| [macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit"), [iOS](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit"), [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material"), [Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui"), [HarmonyOS](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | Keeps the display awake while the lock is held. |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | Requires the browser Screen Wake Lock API; the page must be visible. |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") / [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") ([Linux](https://daybrite.dev/docs/reference/platforms "toolkit:linux-gtk,linux-qt")) | Not implemented; [`is_supported()`](https://docs.rs/day-part-wakelock/latest/day_part_wakelock/fn.is_supported.html) returns false. |

## Authoring

```rust
let lock = day_part_wakelock::keep_screen_on();
// The screen stays on until the lock is dropped.
drop(lock);
```

| Function | What it does |
|---|---|
| [`keep_screen_on()`](https://docs.rs/day-part-wakelock/latest/day_part_wakelock/fn.keep_screen_on.html) | keep the screen on until the returned [`ScreenLock`](https://docs.rs/day-part-wakelock/latest/day_part_wakelock/struct.ScreenLock.html) is dropped |
| [`is_held()`](https://docs.rs/day-part-wakelock/latest/day_part_wakelock/fn.is_held.html) | whether any lock is alive in the app |
| [`is_supported()`](https://docs.rs/day-part-wakelock/latest/day_part_wakelock/fn.is_supported.html) | whether this platform can keep the screen on |

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

## Platform implementations

Platform guidance: [Android: keeping the screen on](https://developer.android.com/develop/background-work/background-tasks/awake/screen-on).

| Platform | Mechanism |
|---|---|
| [iOS](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | [`UIApplication.isIdleTimerDisabled`](https://developer.apple.com/documentation/uikit/uiapplication) |
| [macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | an IOKit power assertion, [`PreventUserIdleDisplaySleep`](https://developer.apple.com/documentation/iokit/kiopmassertiontypepreventuseridledisplaysleep); `pmset -g assertions` lists it |
| [Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | [`SetThreadExecutionState(ES_CONTINUOUS \| ES_DISPLAY_REQUIRED)`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-setthreadexecutionstate) |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | [`FLAG_KEEP_SCREEN_ON`](https://developer.android.com/reference/android/view/WindowManager.LayoutParams#FLAG_KEEP_SCREEN_ON) on the activity's window, set on the UI thread |
| [HarmonyOS](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | [`Window.setWindowKeepScreenOn`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/arkts-apis-window-Window.md) on the app's window |
| [web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | the Screen Wake Lock API, requested again each time the page becomes visible |
| [Linux](https://daybrite.dev/docs/reference/platforms "toolkit:linux-gtk,linux-qt") ([GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk"), [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt")) | none yet: [`is_supported()`](https://docs.rs/day-part-wakelock/latest/day_part_wakelock/fn.is_supported.html) is false |

None of these needs a permission. The web's wake lock needs a secure context (HTTPS or localhost),
which `day launch` and a hosted build both provide.

[Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") and [HarmonyOS](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") reach their window through the daybridge arms in `src/lib.rs`
([bridge](bridge.md)), and the web through its JavaScript arm; Apple and [Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") call the OS
from Rust. On [macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") the part links `IOKit` through `[package.metadata.day.macos]`
([extending](extending.md)).

[Linux](https://daybrite.dev/docs/reference/platforms "toolkit:linux-gtk,linux-qt") has two routes, neither built yet: [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk")'s [`gtk_application_inhibit`](https://docs.gtk.org/gtk4/method.Application.inhibit.html), which needs the toolkit's
application object, and the desktop's [`org.freedesktop.ScreenSaver.Inhibit`](https://specifications.freedesktop.org/idle-inhibit-spec/latest/) over D-Bus, which needs
a D-Bus client the tree does not have.
