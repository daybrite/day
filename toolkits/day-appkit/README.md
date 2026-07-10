# day-appkit

Day's macOS backend: real AppKit, driven from pure Rust via `objc2` — no shim layer.

Every Day piece realizes as a genuine Cocoa view (`NSTextField` labels, `NSButton`,
`NSSlider`, `NSScrollView`, …); Day owns absolute layout over flipped `NSView` containers,
and native events flow back through targets and delegates. This is the backend behind the
`macos-appkit` target.

Backends are picked by a cargo feature on [`day`](https://crates.io/crates/day) — one per
binary; apps never depend on this crate directly.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
