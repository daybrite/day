# day-geometry

Plain `Copy` geometry types shared across Day: `Point`, `Size`, `Rect`, `Insets`, and friends.

Everything is measured in points (density-independent pixels); each native backend converts
to device pixels at its own boundary. This crate sits at the very bottom of Day's crate
graph and has no dependencies of its own.

You will rarely add this crate yourself — it is re-exported wherever it matters, most
visibly through [`day`](https://crates.io/crates/day).

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
