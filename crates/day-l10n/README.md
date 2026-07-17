# day-l10n

The localization engine underneath Day's `tr()` and its generated string functions.

It loads Fluent catalogs, tracks the current locale as a reactive value, resolves messages
with per-locale fallback, and carries the small built-in catalog Day itself needs (dialog
buttons, menu labels). It sits low in Day's crate stack on purpose, so the framework's own
strings localize exactly the way an app's strings do.

Apps don't reach for this crate directly — they use
[`day-fluent`](https://crates.io/crates/day-fluent) or the typed constants from
[`day-build`](https://crates.io/crates/day-build), and everything arrives through
[`day`](https://crates.io/crates/day).

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
