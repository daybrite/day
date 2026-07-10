# day-script

The embedded dayscript engine: drive a running Day app from a YAML script.

Bind-only-when-invited: the TCP server starts only when `DAYSCRIPT_PORT` and
`DAYSCRIPT_TOKEN` are present in the environment — i.e. when `day launch --script` invited
it. Steps tap, type, toggle, navigate, assert text and visibility, and capture screenshots;
Day's own showcase runs a 200+-step walkthrough on all eleven targets in CI through exactly
this engine. The same mechanism doubles as a rescue line for iterating without hot reload:
a script can click its way back to the screen you're working on after every relaunch.

Embedded automatically by [`day`](https://crates.io/crates/day); driven by
[`day-cli`](https://crates.io/crates/day-cli).

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
