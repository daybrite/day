# App lifecycle (§ lifecycle)

A Day app can hook the moments its process moves through: launching, gaining and losing focus,
going to and from the background, running low on memory, and terminating. Each phase maps to the
host OS's native app/activity delegate, so a handler runs at the right native moment on
every platform.

```rust
use day::prelude::*;

fn main() {
    // Register before launch so `WillLaunch` / `DidLaunch` are captured.
    day::on_lifecycle(Lifecycle::DidLaunch,    || println!("up and running"));
    day::on_lifecycle(Lifecycle::WillTerminate, || save_state());   // ⌘Q / Ctrl+Q / OS quit

    day::launch(WindowOptions::default(), app::root);
}
```

Handlers run in registration order, inside a reactive batch, so a lifecycle handler that writes a
`Signal` updates the UI just like a button callback. Register as many as you like per phase.

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

`WillLaunch` and `DidLaunch` are emitted uniformly by day-core (reliable everywhere); the rest come
from each backend's native app/activity delegate.

### When to register

Register before `day::launch` (in `main`) to catch `WillLaunch`/`DidLaunch`. Registering later
(inside your root builder, say) still catches everything from `DidLaunch` onward. On the mobile
shells, the root builder is the natural registration point (there is no `main` you own); those
handlers see `DidLaunch` and after.

## Platform availability

Not every phase exists on every platform: a desktop app doesn't enter the background or run out
of memory the way a phone does. The **universal** phases (`WillLaunch`, `DidLaunch`,
`DidBecomeActive`, `WillResignActive`, `WillTerminate`) are delivered by every backend. The
background/foreground/memory phases are delivered only by the mobile backends (UIKit, Android).

| Phase | AppKit | GTK | Qt | UIKit | Android | WinUI |
|---|:-:|:-:|:-:|:-:|:-:|:-:|
| WillLaunch / DidLaunch | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| DidBecomeActive / WillResignActive | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| WillEnterForeground / DidEnterBackground | — | — | — | ✓ | ✓ | — |
| DidReceiveMemoryWarning | — | — | — | ✓ | ✓ | — |
| WillTerminate | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |

Native mapping: AppKit `NSApplication` notifications; UIKit `UIApplicationDelegate`; GTK window
`is-active` + GApplication `shutdown`; Qt `applicationStateChanged` + `aboutToQuit`; Android Activity
lifecycle (`onResume`/`onPause`/`onStart`/`onStop`/`onTrimMemory`/`onDestroy`); WinUI window
`WM_ACTIVATE`/`WM_CLOSE`.

### Guarding platform-specific phases

Registering a handler for a phase the running backend doesn't deliver is not an error: the handler is
never called, and Day logs a one-time warning at launch:

```
day: an `on_lifecycle(DidEnterBackground)` handler was registered, but this backend never delivers
that phase, so it will not run. Guard it with `day::lifecycle_supported(..)` or a
`day::require_lifecycle!(..)` compile-time check.
```

There are two ways to guard, depending on how strict you want to be.

**Soft: register only where supported.** `day::lifecycle::supported` is a `const fn` that knows the
backend compiled into this binary, so it's `false` on desktop and `true` on mobile for the mobile-only
phases:

```rust
if day::lifecycle::supported(Lifecycle::DidEnterBackground) {
    day::on_lifecycle(Lifecycle::DidEnterBackground, || flush_to_disk());
}
```

(There is also a runtime `day::lifecycle_supported(phase)` for checks after the app is up.)

**Hard: require it at compile time.** If your app depends on a phase, assert it and get a build
error on a backend that can't deliver it:

```rust
day::require_lifecycle!(Lifecycle::DidEnterBackground);  // compile error on a desktop backend
```

## Quit

`WillTerminate` fires on every quit path: the `Quit` menu command (`menu_role(MenuRole::Quit)`), the
platform quit shortcut (⌘Q on macOS, Ctrl+Q elsewhere), and the OS reclaiming the app. The `Quit`
command exits the app. On GTK a standard `app.quit` action is registered so ⌘Q / Ctrl+Q and the menu
item both work; on macOS the App-menu Quit is standard; Qt/WinUI quit their event loops. Save work in a
`WillTerminate` handler.

Mobile note: iOS/Android apps don't have a user-facing "quit". There, `WillTerminate` means the OS is
tearing the app down (Android `onDestroy` while finishing; iOS `applicationWillTerminate:`). Prefer
`DidEnterBackground` for "the user left" on mobile.

## How it works

`on_lifecycle` records the closure in a day-core registry keyed by phase. Backends emit
`Event::Lifecycle(phase)` from their native delegate (the same event rail as menu actions); the event
pump routes it to `dispatch_lifecycle`, which runs the phase's handlers in a reactive batch. Adding
lifecycle support to a new backend is two things: implement `Toolkit::supports_lifecycle` (and the
matching `const fn lifecycle_supported`), and emit `Event::Lifecycle(..)` at the right native moments.
