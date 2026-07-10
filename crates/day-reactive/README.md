# day-reactive

Day's reactive core: signals, memos, effects, and bindings over a thread-local graph.

Build once, bind forever: UI code runs a single time, and everything dynamic is expressed
as a `Signal`, a `Memo`, or a closure over them. Dependency tracking is automatic, updates
are batched to turn boundaries, and disposal follows ownership scopes. The design targets
UI work on a single main thread — no locks, no `Send` bounds, no async runtime.

Usable on its own, but written for [`day`](https://crates.io/crates/day), which pairs it
with native widget trees.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
