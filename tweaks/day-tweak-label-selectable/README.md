# day-tweak-label-selectable

Let users select and copy a label's text: `.selectable()` on any `label(...)`.

Implemented on macOS, GTK, and Android — three platforms, three different native calls,
one modifier. Elsewhere it does nothing, and says so in its documentation.

Tweaks are Day's smallest kind of extension: a little crate that adjusts the native widget
behind a built-in piece. On platforms a tweak doesn't cover, it quietly does nothing, so
your app code stays free of platform checks.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
