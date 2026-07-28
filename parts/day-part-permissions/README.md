# day-part-permissions

Ask the OS for the camera, the microphone, location, notifications, photos, or motion — the
way each OS wants to be asked.

Six portable permissions cover what most apps need, and a per-platform escape hatch reaches
anything else. Before you ask, you can find out what asking would even do: whether the
platform gates this capability at all, whether a prompt would appear, and whether the answer
is already final so you should send the user to Settings instead.

Parts are Day's small capability crates: no UI, just a plain Rust API over something the
platform already provides. This one works in any Rust program — you don't need a Day app
around it.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
