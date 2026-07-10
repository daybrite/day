# day-l10n

Day's core localization engine: Fluent bundles, locale state, and formatting.

It sits low in the crate graph on purpose, so even Day's own internals (dialog buttons,
menu-role labels) can localize their strings the same way apps do. Holds the current-locale
signal, per-locale bundle registry, and the built-in core catalog.

Most code uses [`day-fluent`](https://crates.io/crates/day-fluent)'s `tr(...)` on top of
this; both arrive with [`day`](https://crates.io/crates/day).

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
