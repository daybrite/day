# day-part-clipboard

Cross-platform plain-text clipboard access: `set_text`, `get_text`, `has_text`, through
`NSPasteboard`, `UIPasteboard`, the GTK/Qt clipboards, Android's `ClipboardManager`, or the
Win32 clipboard — whichever is native to the running platform.

Parts are Day's headless extension unit — platform capability crates with no UI. This one
works in **any Rust program**; a Day app is not required.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
