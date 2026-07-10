# day-part-sensors

Motion sensors through native APIs — accelerometer/gyro sampling where the hardware and
platform provide them, with capability reporting where they don't.

Parts are Day's headless extension unit — platform capability crates with no UI. This one
works in **any Rust program**; a Day app is not required.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
