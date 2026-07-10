# day-fluent

The piece-facing localization layer for Day apps, built on Mozilla Fluent.

`tr("key")` returns a reactive string that re-resolves when the app locale changes;
`tr_with` interpolates arguments with Fluent's plural and formatting rules. Catalogs are
plain `.ftl` files per locale, and piece packages can ship their own without colliding
with the app's.

Pairs with [`day-l10n`](https://crates.io/crates/day-l10n) (the engine) underneath, and is
re-exported to apps through [`day`](https://crates.io/crates/day).

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
