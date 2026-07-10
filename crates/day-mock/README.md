# day-mock

Day's headless toolkit: every native call recorded into an op log instead of a widget.

The mock backend performs deterministic text measurement (fixed per-character metrics),
supports the full toolkit contract, and exposes a probe for tests to inspect widgets,
frames, and the exact sequence of mutations — which is how Day asserts its
"one state change, one native mutation" invariant as a test, not a slogan. It also powers
golden tests for external pieces without any platform SDK installed.

Useful when testing code built on [`day`](https://crates.io/crates/day); apps enable it
with the `mock` backend feature.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
