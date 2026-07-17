# day-part-battery

Ask the battery how it's doing, on any platform, with one call.

`status()` returns the charge level, whether the device is charging, and whether low-power
mode is on. Behind that one call sit the native APIs: IOKit on macOS, `UIDevice` on iOS,
`BatteryManager` on Android, sysfs on Linux, and `GetSystemPowerStatus` on Windows.

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
