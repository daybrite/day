# day-piece-combobox

An editable combo box for Day apps: type a value or pick one from the list.

One Rust API, and a real native control on every platform — `NSComboBox` on macOS, a
Material dropdown on Android, `GtkComboBoxText` on Linux, `QComboBox` on Qt, and so on.

This crate doubles as the reference example of a piece with per-toolkit Rust renderers:
if you want to build a native component of your own, its source is the place to start.

Pieces are Day's reusable UI components, shipped as ordinary crates: one Rust API in
front, a real native control per platform behind it. Enable the backends you build for
with cargo features, and `day build` wires up the native side automatically.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
