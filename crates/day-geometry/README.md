# day-geometry

The small geometry types the rest of Day shares: `Point`, `Size`, `Rect`, `Insets`,
`Color`, and friends.

All of them are plain `Copy` values measured in points (density-independent pixels); each
native backend converts to device pixels at its own edge. The crate sits at the very
bottom of Day's stack and depends on nothing.

You'll rarely add it to a project yourself — it comes re-exported wherever you need it,
most visibly through [`day`](https://crates.io/crates/day).

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
