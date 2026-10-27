---
navTitle: "Lifecycle"
title: "App lifecycle"
description: "Foreground, background, and termination events as day delivers them, uniformly across platforms."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# App lifecycle

A Day app can hook the moments its process moves through: launching, gaining and losing focus,
going to and from the background, running low on memory, and terminating. Each phase maps to the
host OS's native app/activity delegate, when a backend receives the corresponding native callback. OS termination without a callback
cannot deliver a handler; persist important state before termination.

```rust
use day::prelude::*;

fn main() {
    // Register before launch so `WillLaunch` / `DidLaunch` are captured.
    day::on_lifecycle(Lifecycle::DidLaunch, || println!("up and running"));
    day::on_lifecycle(Lifecycle::WillTerminate, || save_state()); // ⌘Q / Ctrl+Q / OS quit

    day::launch(app::res::window_options(), app::root);
}
```

Handlers run in registration order, inside a reactive batch, so a lifecycle handler that writes a
`Signal` updates the UI just like a button callback. Register as many as you like per phase.
Before `WillTerminate` handlers run, Day cancels pending main-loop tasks while reactive state
is still alive. Handlers must finish shutdown synchronously; newly submitted tasks are not
polled. A returning loop does the same at `DidExit` if necessary. See [async shutdown](async.md).

## The phases

| Phase | When |
|---|---|
| `WillLaunch` | Before the window/UI exists; the first thing to run. Set up global state. |
| `DidLaunch` | The UI is mounted and laid out; the app is about to run. |
| `DidBecomeActive` | Came to the foreground and is the focused, interactive app. |
| `WillResignActive` | About to stop being active (an interruption, app switch, losing focus). |
| `WillEnterForeground` | *(mobile)* About to return to the foreground. |
| `DidEnterBackground` | *(mobile)* Left the foreground and is no longer visible. Persist state. |
| `DidReceiveMemoryWarning` | *(mobile)* The system is low on memory. Drop caches. |
| `WillTerminate` | About to terminate; the last chance to save. |
| `DidExit` | The run loop is over; nothing of the app runs after this. Day's exit line follows the handlers. |

`WillLaunch` and `DidLaunch` are emitted uniformly by day-core (reliable everywhere); the rest come
from each backend's native app/activity delegate.

## The exit line

Once the `DidExit` handlers have run, Day logs one last line at `info`:

```text
Clean exit for Day Rise v0.4.5 (debug) after 00:03:12. Memory usage: 48.21 MB
```

The name is the app's ([`WindowOptions::app_name`](https://docs.rs/day-spec/latest/day_spec/struct.WindowOptions.html#method.app_name), else its title), the version is
[`WindowOptions::version`](https://docs.rs/day-spec/latest/day_spec/struct.WindowOptions.html#method.version) (the scaffold's `window()` sets it to the crate's `CARGO_PKG_VERSION`, and
`day::day_start!` fills it in when it is unset), falling back to the `DAY_APP_VERSION` the `day` CLI
sets on a launch; the profile is the build's; the time is since [`day::launch`](https://docs.rs/day-app/latest/day/fn.launch.html); the memory is the
process's resident set as the platform counts it (the physical footprint on Apple platforms,
`/proc/self/statm` on Linux and Android, the working set on Windows, the linear memory on the
web). A handler registered for `DidExit` runs before the line, so it is the app's last word:

```rust
day::on_lifecycle(Lifecycle::DidExit, || log::info!("bye"));
```

The line marks completion of Day's exit handlers: the quit command, an orderly OS shutdown,
or a page-exit callback when the browser delivers one. It precedes final native and thread-local destruction, so a later
destructor failure can still crash the process after this line. A kill or earlier crash may
prevent the line entirely. The memory figure covers this process, not separate WebKit helpers.

### When to register

Register before [`day::launch`](https://docs.rs/day-app/latest/day/fn.launch.html) (in `main`) to catch `WillLaunch`/`DidLaunch`. Registering later
(inside your root builder, say) still catches everything from `DidLaunch` onward. On the mobile
shells, the root builder is the natural registration point (there is no `main` you own); those
handlers see `DidLaunch` and after.

## Platform availability

Not every phase exists on every platform: a desktop app doesn't enter the background or run out
of memory the way a phone does. The **universal** phases (`WillLaunch`, `DidLaunch`,
`DidBecomeActive`, `WillResignActive`, `WillTerminate`) are delivered by every backend. The
background/foreground/memory phases are delivered only by the mobile backends (UIKit, Android,
ArkUI).

| Phase | AppKit | GTK | Qt | UIKit | Android | ArkUI | WinUI |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| WillLaunch / DidLaunch | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| DidBecomeActive / WillResignActive | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| WillEnterForeground / DidEnterBackground | — | — | — | ✓ | ✓ | ✓ | — |
| DidReceiveMemoryWarning | — | — | — | ✓ | ✓ | ✓ | — |
| WillTerminate | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| DidExit | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |

Native mapping: AppKit `NSApplication` notifications; UIKit `UIApplicationDelegate`; GTK window
`is-active` + GApplication `shutdown`; Qt `applicationStateChanged` + `aboutToQuit`; Android Activity
lifecycle (`onResume`/`onPause`/`onStart`/`onStop`/`onTrimMemory`/`onDestroy`); ArkUI's entry
`UIAbility` (`onForeground`/`onBackground`/`onMemoryLevel`/`onDestroy`, with the window stage's
`ACTIVE`/`INACTIVE` events for the active pair); WinUI window `WM_ACTIVATE`/`WM_CLOSE`.

The web backend delivers `WillLaunch`, `DidLaunch`, the active pair, and `DidExit` (on `pagehide`,
the page's last moment).

`DidExit` is the last moment each backend has: where the native loop returns to [`day::launch`](https://docs.rs/day-app/latest/day/fn.launch.html)
(GTK, Qt, WinUI) it is emitted there; where the process ends inside the loop it follows
`WillTerminate` at the same native moment (AppKit's and UIKit's `applicationWillTerminate`,
Android's `onDestroy`, ArkUI's `onDestroy`); on the web it is `pagehide`. A backend whose host
owns the loop, so that [`Platform::run`](https://docs.rs/day-spec/latest/day_spec/trait.Platform.html#tymethod.run) hands back at once with the app still to come (ArkUI,
Android, the web), says so with `Platform::RUN_ENDS_APP = false`: day-core then leaves the
phase to the host's callback instead of treating the return as the exit.

### Guarding platform-specific phases

Registering a handler for a phase the running backend doesn't deliver is not an error: the handler is
never called, and Day logs a one-time warning at launch:

```text
day: an `on_lifecycle(DidEnterBackground)` handler was registered, but this backend never delivers
that phase, so it will not run. Guard it with `day::lifecycle_supported(..)` or a
`day::require_lifecycle!(..)` compile-time check.
```

There are two ways to guard, depending on how strict you want to be.

**Soft: register only where supported.** [`day::lifecycle::supported`](https://docs.rs/day-app/latest/day/lifecycle/fn.supported.html) is a `const fn` that knows the
backend compiled into this binary, so it's `false` on desktop and `true` on mobile for the mobile-only
phases:

```rust
if day::lifecycle::supported(Lifecycle::DidEnterBackground) {
    day::on_lifecycle(Lifecycle::DidEnterBackground, || flush_to_disk());
}
```

(There is also a runtime [`day::lifecycle_supported(phase)`](https://docs.rs/day-app/latest/day/fn.lifecycle_supported.html) for checks after the app is up.)

**Hard: require it at compile time.** If your app depends on a phase, assert it and get a build
error on a backend that can't deliver it:

```rust
day::require_lifecycle!(Lifecycle::DidEnterBackground);  // compile error on a desktop backend
```

## Quit

An orderly quit can deliver `WillTerminate`: the `Quit` menu command ([`menu_role(MenuRole::Quit)`](https://docs.rs/day-pieces/latest/day_pieces/fn.menu_role.html)), the
platform quit shortcut (⌘Q on macOS, Ctrl+Q elsewhere), and native shutdown callbacks. A forced kill or mobile process reclamation may deliver none. The `Quit`
command exits the app. On GTK a standard `app.quit` action is registered so ⌘Q / Ctrl+Q and the menu
item both work; on macOS the App-menu Quit is standard; Qt/WinUI quit their event loops.
Use `WillTerminate` for synchronous cleanup, not as the sole durability point for user data.

Mobile note: iOS/Android apps don't have a user-facing "quit". There, `WillTerminate` means the OS is
tearing the app down (Android `onDestroy` while finishing, for the activity Day is mounted in: one a newer instance
replaced, as a launch that clears the task does, finishes without ending the app; iOS `applicationWillTerminate:`). Prefer
`DidEnterBackground` for "the user left" on mobile.

## Launch memory profiling

`day launch` enables `DAY_MEMORY_PROFILE=1` for native apps by default. A process-global
sampler reads current-process memory every second, keeps constant-space min/max/sum/count,
and appends min/max/average/end MB to the normal exit line. `--memory-profile=false`
disables it. The initial and final samples are included; the average is an arithmetic sample
mean and the maximum is a sampled maximum. The sampler uses no UI state or TLS.
Abrupt process termination cannot emit a lifecycle exit line. Scripted launches additionally
checkpoint host reports and summarize the last observations when the connection is lost;
see the DayScript contract in [DESIGN.md](../DESIGN.md#appendix-c--dayscript-reference-v1).

## Saving state

An OS can kill the process without delivering a final lifecycle callback. Save important
state as it changes rather than waiting for shutdown.
