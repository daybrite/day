# day-arkui

Day's HarmonyOS / OpenHarmony backend, built on ArkUI's native C API.

Pieces become real ArkUI nodes — text, buttons, sliders, swipers — created from native
code and mounted into a slot provided by a small ArkTS host, which owns the window while
Rust owns the widget tree. The shape mirrors Day's Android backend. This is the backend
behind the `ohos-arkui` target; `day ohos` helps with emulators, and the details live in
Day's HarmonyOS guide.

You don't add this crate to a project yourself. Backends are chosen by a cargo feature on
[`day`](https://crates.io/crates/day) — each app binary contains exactly one — and the
`day` CLI selects the right one for the target you're building.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
