# day-gtk

Day's GTK 4 backend, written in pure Rust over gtk4-rs.

Pieces become real GTK widgets with libadwaita styling; Day places them, measures text
through Pango, and turns GTK signals back into app events. It is the backend behind the
`linux-gtk` target — and behind `macos-gtk`, because GTK itself is portable, which makes
this a comfortable way to develop for Linux from a Mac.

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
