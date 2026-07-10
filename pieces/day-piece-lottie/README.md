# day-piece-lottie

A Lottie animation piece for Day apps, iOS and Android only. It is the reference for a
piece that pulls an *external native package*: lottie-ios arrives via SwiftPM and
lottie-android via Gradle, both declared in the crate's metadata and wired by `day build`
— nothing vendored.

Pieces are Day's extension unit: a crate with one Rust API and per-toolkit native
renderers, enabled per backend by cargo features (`day build` wires them automatically).

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
