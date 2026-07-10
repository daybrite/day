# day-android

Day's Android backend: real Android views over JNI plus a small Java bridge.

Pieces realize as Material 3 widgets (`TextView`, `MaterialButton`, `SwitchMaterial`,
`SeekBar`, `RecyclerView`, …) created and mutated through `DayBridge`, the Java analogue of
Day's C++ shims. Day owns layout; Android owns rendering, IME, and accessibility. This is
the backend behind the `android-widget` target.

Backends are picked by a cargo feature on [`day`](https://crates.io/crates/day) — one per
binary; apps never depend on this crate directly.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
