# day-android

Day's Android backend: real Android views over JNI, with a small Java bridge.

Pieces become Material 3 widgets — text views, buttons, switches, sliders, recycling lists
— created and updated through `DayBridge`, a compact Java class that plays the role Day's
C++ shims play on other platforms. Day owns layout; Android owns rendering, the keyboard,
and accessibility. This is the backend behind the `android-mdc` target.

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
