# day-build

Build-script codegen for Day apps: every bundled resource becomes a typed Rust constant.

An app's `build.rs` calls `day_build::generate_resources()`, which scans the project's
`resource/` folder and generates a `res` module: a constant per image
(`res::images::logo`), per data file, and per bundled font, plus one function per
localized string — `res::str::greeting(name)` instead of a bare string key. Rename a file
or forget a translation argument and the app stops compiling, which is exactly when you
want to hear about it.

The crate also owns the naming rules the `day` CLI follows when it stages resources into
each platform's native format, so the constant in your code and the file on the device
can never disagree.

It is a normal `[build-dependencies]` entry, generated for you by `day new`.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
