# day-part-deviceinfo

What is this code running on?

One `get()` call returns the hardware model (`"MacBookPro18,3"`, `"Pixel 7"`), the OS name
and version, and whether you're on a simulator or emulator — each read from the native API
for the platform at hand. Useful for diagnostics screens, support emails, and the
occasional device-specific workaround.

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
