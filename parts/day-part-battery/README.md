# day-part-battery

Cross-platform battery status through each platform's native API — level, charging state,
and low-power mode, from one `status()` call.

macOS IOKit, iOS `UIDevice`, Android `BatteryManager`, Linux sysfs, Windows
`GetSystemPowerStatus`.

Parts are Day's headless extension unit — platform capability crates with no UI. This one
works in **any Rust program**; a Day app is not required.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
