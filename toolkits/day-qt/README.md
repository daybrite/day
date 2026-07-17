# day-qt

Day's Qt 6 Widgets backend.

Pieces become real `QWidget`s — `QLabel`, `QPushButton`, `QSlider`, `QScrollArea`, and so
on — created through [`day-qt-sys`](https://crates.io/crates/day-qt-sys), a small C++ shim
this crate drives. Day owns the geometry; Qt draws the widgets and feeds events back
through the shim's callbacks. It is the backend behind the `linux-qt`, `macos-qt`, and
`windows-qt` targets.

You don't add this crate to a project yourself. Backends are chosen by a cargo feature on
[`day`](https://crates.io/crates/day) — each app binary contains exactly one — and the
`day` CLI selects the right one for the target you're building.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
