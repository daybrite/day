# day-tweak-slider-tickmarks

Native tick marks (and snap-to-tick where the platform has it) for Day's `slider(…)`,
configured with `.tickmarks(Tickmarks { count, snap, position })`.

The full-range packaged-tweak example: implementations across six toolkits exercise every
tweak access tier, from typed objc2/gtk4-rs calls through JNI to bring-your-own C++ against
raw Qt and WinUI handles — and it says plainly that UIKit has no native tick API.

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
