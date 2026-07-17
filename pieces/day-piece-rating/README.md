# day-piece-rating

A tappable star-rating control for Day apps, with a card surface and a numbered badge as
companions.

There is no native code in this crate at all: everything is composed from Day's public
primitives — rows, canvas drawing, gestures, signals. That's the point it proves: when a
component can be built from core pieces, it runs on every backend for free, and this crate
is the worked example of doing exactly that.

Pieces are Day's reusable UI components, shipped as ordinary crates: one Rust API in
front, a real native control per platform behind it. Enable the backends you build for
with cargo features, and `day build` wires up the native side automatically.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
