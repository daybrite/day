# day-tweak-button-bezel

The smallest possible packaged Day tweak: symbolic constants for AppKit's `NSButton`
bezel styles, applied to a stock `button(…)` with `.bezel(Bezel::Textured)`.

AppKit only, and one native call — this crate exists mostly as the minimal example of the
tweak pattern.

Tweaks are Day's lightest extension tier: per-toolkit configuration applied to the native
widget behind a stock Day piece, packaged as a reusable crate. On toolkits a tweak doesn't
cover, it is a documented no-op — apps never need platform `#[cfg]`s.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
