# day-pieces

Day's built-in piece library: the functions Day apps are written in.

`label`, `button`, `text_field`, `toggle`, `slider`, `image`, `column`, `row`, `stack`,
`scroll`, `canvas`, navigation and tabs, dialogs, menus — every constructor is a plain
function returning a value, configured by builder methods, made dynamic by accepting
signals and closures where it makes sense. Each piece realizes as a genuinely native
control on every backend.

Depend on [`day`](https://crates.io/crates/day) rather than on this crate directly — the
umbrella re-exports everything here through its prelude.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
