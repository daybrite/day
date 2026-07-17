# day-mock

A pretend backend for testing Day code: instead of creating widgets, it writes down every
call the engine makes.

Tests run a real Day app against it — no window, no platform SDK, the same engine — and
then inspect the recording: which widgets exist, what their frames are, and exactly which
native mutations a state change produced. That last part is how Day's central promise
("one state change, one native update") is checked in CI rather than just claimed. Text
measurement uses fixed per-character metrics, so layout comes out identical on every
machine.

If you're writing a piece of your own or testing app logic, enable the `mock` backend
feature on [`day`](https://crates.io/crates/day) and assert against the log.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
