# day-core

Day's engine room: the piece model, the realized native tree, layout, and event routing.

`day-core` turns a tree of piece values into real native widgets exactly once, then keeps
the two in sync — reactive updates arrive as targeted patches to individual nodes, never as
a rebuild-and-diff pass. It owns measurement and absolute layout (each backend reports
natural sizes; day-core decides frames), routes native events back to app closures, and is
generic over the `Toolkit` trait so one backend is monomorphized into each binary.

Apps don't depend on this crate directly — it arrives through the
[`day`](https://crates.io/crates/day) umbrella crate.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
