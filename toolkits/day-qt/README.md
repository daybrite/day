# day-qt

Day's Qt 6 Widgets backend, over the [`day-qt-sys`](https://crates.io/crates/day-qt-sys)
C++ shim.

Pieces realize as real `QWidget`s (`QLabel`, `QPushButton`, `QSlider`, `QScrollArea`, …);
Day owns absolute geometry and feeds events back through the shim's callbacks. This is the
backend behind the `linux-qt`, `macos-qt`, and `windows-qt` targets.

Backends are picked by a cargo feature on [`day`](https://crates.io/crates/day) — one per
binary; apps never depend on this crate directly.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
