# day-toolchain

One place that knows where host toolchains and SDKs live — shared by the
[`day` CLI](https://crates.io/crates/day-cli) and by Day crates' build scripts.

Android SDK/NDK and JDK, Windows kits and C++/WinRT, the OpenHarmony NDK, rustup homes,
NSIS — every lookup follows the conventional environment variables first
(`ANDROID_HOME`, `JAVA_HOME`, `OHOS_NDK_HOME`, `DAY_WINDOWS_KITS_ROOT`, …) and only then
probes the usual install locations. No literal `C:\Program Files` paths buried in build
scripts, and one crate to fix when a vendor moves things.

Generic enough to read, small enough to audit — but shaped by what Day's build pipeline
actually needs.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
