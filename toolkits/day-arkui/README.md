# day-arkui

Day's HarmonyOS / OpenHarmony backend, over the ArkUI **Native NodeAPI**.

Every piece becomes a real `ArkUI_NodeHandle` (Text, Button, Slider, Swiper, …) built from
native code and mounted into an ArkTS `NodeContent` slot — the same architecture as Day's
Android backend, with ArkTS hosting the window and Rust owning the tree. This is the
backend behind the `ohos-arkui` target.

Backends are picked by a cargo feature on [`day`](https://crates.io/crates/day) — one per
binary; apps never depend on this crate directly.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
