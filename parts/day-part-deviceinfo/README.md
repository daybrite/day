# day-part-deviceinfo

A snapshot of device identity through native APIs: hardware model (`"MacBookPro18,3"`,
`"Pixel 7"`), OS name and version, and whether the code is running on a simulator or
emulator — one `get()` call, implemented per platform.

Parts are Day's headless extension unit — platform capability crates with no UI. This one
works in **any Rust program**; a Day app is not required.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
