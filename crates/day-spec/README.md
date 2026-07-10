# day-spec

The contract between Day's engine and its native backends.

`day-spec` defines the `Toolkit` trait, the piece-kind vocabulary (`day.label`,
`day.button`, …), typed props and patch enums, the event model, and the piece-registration
seam that lets external crates ship native renderers. Backends depend on this crate and
nothing else of Day's core, which is what keeps one-backend-per-binary linking honest.

This is internal plumbing for [`day`](https://crates.io/crates/day); app code sees its
types re-exported from there.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
