# day-arkui-sys

Raw `extern "C"` declarations plus the ArkUI/NAPI C++ shim, built against the OpenHarmony NDK that
[`day-arkui`](https://crates.io/crates/day-arkui) drives.

The shim is compiled by this crate's build script against the platform SDK; handles cross
the FFI boundary as opaque pointers. There is no safe API here on purpose — the safe layer
is `day-arkui`, and apps depend on neither directly (the backend arrives via a cargo
feature on [`day`](https://crates.io/crates/day)).

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
