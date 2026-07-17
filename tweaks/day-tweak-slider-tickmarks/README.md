# day-tweak-slider-tickmarks

Native tick marks for Day's slider, with snap-to-tick where the platform supports it:
`.tickmarks(Tickmarks { count, snap, position })`.

Six toolkits are covered, and the implementations span every way a tweak can reach native
code — typed Rust bindings on AppKit and GTK, JNI on Android, and bring-your-own C++
against raw Qt and WinUI handles — which makes this crate the full reference example of the
pattern. UIKit has no native tick API, and this crate says so rather than imitating one.

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
