# day-gtk

Day's GTK 4 backend, in pure Rust over gtk4-rs.

Pieces realize as real GTK widgets under libadwaita styling; Day owns absolute layout over
`GtkFixed` containers, measures text through Pango, and routes signals back as events. This
is the backend behind the `linux-gtk` and `macos-gtk` targets.

Backends are picked by a cargo feature on [`day`](https://crates.io/crates/day) — one per
binary; apps never depend on this crate directly.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
