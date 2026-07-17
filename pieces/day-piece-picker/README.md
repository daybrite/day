# day-piece-picker

Pick one option from a short list, in whichever style fits: a menu, a segmented control,
or an inline list.

Each style is a real native control per platform — `NSPopUpButton` and
`NSSegmentedControl` on macOS, `UISegmentedControl` on iOS, Material controls on Android,
and so on. You state the options and the style; the platform decides what it looks like.

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
