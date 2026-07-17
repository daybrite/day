# day-arkui-sys

The C++ side of Day's HarmonyOS backend.

This crate holds the ArkUI/NAPI shim and the raw `extern "C"` declarations for it. The
build script compiles the shim against the OpenHarmony NDK, and ArkUI nodes cross into
Rust as opaque pointers. There is deliberately no safe API here: the safe layer is
[`day-arkui`](https://crates.io/crates/day-arkui), and apps depend on neither crate
directly — the backend arrives through a cargo feature on
[`day`](https://crates.io/crates/day).

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
