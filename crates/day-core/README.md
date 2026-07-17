# day-core

The engine at the center of Day: it turns your description of an interface into real
native widgets, then keeps the two in step.

When a Day app starts, `day-core` walks the tree of pieces your code returned, asks the
active backend to create a native widget for each one, and connects your signals to those
widgets. That construction happens once. From then on, a state change flows directly to
the one widget it affects — the tree is never rebuilt and never compared against a copy.

The crate also owns layout and events. It measures native widgets (so text is exactly as
tall as the platform says it is), decides where everything goes, and routes native events
— a tap, a keystroke, a scroll — back to the Rust closures you wrote.

You won't depend on `day-core` directly: the [`day`](https://crates.io/crates/day)
umbrella crate brings it in and re-exports what apps need.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
