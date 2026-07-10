# day-uikit

Day's iOS backend: real UIKit, driven from pure Rust via `objc2` — no shim layer.

Pieces realize as `UILabel`, `UIButton`, `UISwitch`, `UIScrollView`, and friends; Dynamic
Type and the safe area are respected natively, and the app boots through a regular
`UIApplicationMain` delegate. This is the backend behind the `ios-uikit` target.

Backends are picked by a cargo feature on [`day`](https://crates.io/crates/day) — one per
binary; apps never depend on this crate directly.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
