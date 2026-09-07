<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-appkit

Day's macOS backend: your interface, built from real AppKit views.

With this backend, `label(...)` is an `NSTextField`, `button(...)` is an `NSButton`, and
scrolling is an `NSScrollView` — created and driven directly from Rust through `objc2`,
with no C or Objective-C glue layer in between. Day decides where everything goes; AppKit
does the drawing, the text editing, the focus rings, and the accessibility. This is the
backend behind the `macos-appkit` target.

You don't add this crate to a project yourself. Backends are chosen by a cargo feature on
[`day`](https://crates.io/crates/day) — each app binary contains exactly one — and the
`day` CLI selects the right one for the target you're building.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's own widgets — AppKit, UIKit, Android's Material widgets, GTK 4, Qt 6,
XAML, and ArkUI — from one codebase. When you write `button("Save")`, macOS shows an
`NSButton` and Android shows a Material button. The framework also ships the tooling around
the app: the `day` CLI, a VS Code extension, GitHub CI workflows, localization,
accessibility, and dayscript automation.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
