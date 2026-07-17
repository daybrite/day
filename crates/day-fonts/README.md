# day-fonts

A small library for reading a font file's names — the family name and PostScript name —
straight from `.ttf` or `.otf` bytes.

Day uses it to make bundled fonts work by family name on every platform: the build tool
reads each font's real family name and stores the file under a predictable identifier, and
the runtime derives the same identifier back. This crate is that shared vocabulary, in one
place, used by both sides.

It has no dependencies and no text-shaping machinery, so it's also handy in any program
that just needs to know what a font file calls itself.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
