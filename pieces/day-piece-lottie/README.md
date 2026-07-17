# day-piece-lottie

Play Lottie animations in Day apps on iOS and Android.

The piece wraps each platform's standard player — lottie-ios and lottie-android — and
nothing is vendored: the crate declares those dependencies in its metadata, and
`day build` fetches them through SwiftPM and Gradle as part of the normal build.

It also serves as the reference example of a piece that pulls in third-party native
libraries, if you're planning one of your own.

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
