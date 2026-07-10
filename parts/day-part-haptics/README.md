# day-part-haptics

Haptic feedback through native APIs: impact and notification styles on iOS, `Vibrator`
effects on Android, and trackpad haptics on macOS.

`is_supported()` tells the truth per platform, and playing a haptic where there is no
hardware is an honest no-op — never an error path in your app code.

Parts are Day's headless extension unit — platform capability crates with no UI. This one
works in **any Rust program**; a Day app is not required.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
