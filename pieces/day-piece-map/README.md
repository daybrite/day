# day-piece-map

A native map view for Day apps on Apple platforms, backed by MapKit's `MKMapView`.

It is deliberately Apple-only: on every other platform the piece renders Day's standard
placeholder, and this README is the capability statement. It exists partly as the
reference example of a piece that supports some platforms and says so, rather than
pretending to support them all.

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
