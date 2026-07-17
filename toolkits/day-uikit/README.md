# day-uikit

Day's iOS backend: your interface, built from real UIKit views.

With this backend, pieces become `UILabel`, `UIButton`, `UISwitch`, `UIScrollView`, and
friends, created and driven directly from Rust through `objc2`. Dynamic Type and the safe
area behave the way iOS users expect because iOS itself is handling them, and the app
starts through a regular `UIApplicationMain` delegate. This is the backend behind the
`ios-uikit` target.

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
