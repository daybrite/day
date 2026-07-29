# day-part-location

Where the device is — once, or as a live stream — through the platform's own location service.

Ask for a single fix or subscribe to updates and stop them by dropping the handle. Every field
the platform declined to measure comes back as `None` rather than a plausible-looking zero, so
"we don't know the altitude" never reads as "sea level".

Parts are Day's small capability crates: no UI, just a plain Rust API over something the
platform already provides. This one works in any Rust program — you don't need a Day app
around it.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, XAML, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
