# day-part-prefs

A small persistent settings store: strings that survive relaunches.

`set`, `get`, `remove`, and `contains`, kept in each platform's native preferences home —
`NSUserDefaults` on macOS and iOS, `SharedPreferences` on Android, and a plain,
documented file under the user's config directory on Linux, Windows, and OpenHarmony.
Right-sized for window positions, chosen units, and the other small things apps remember.

Parts are Day's small capability crates: no UI, just a plain Rust API over something the
platform already provides. This one works in any Rust program — you don't need a Day app
around it.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
