# day-piece-combobox

An editable combo box for Day apps — one Rust API, a real native control per toolkit.

`NSComboBox` on macOS, a Material exposed-dropdown on Android, `GtkComboBoxText`,
`QComboBox`, and so on. This crate is also the reference implementation of a "tier 1"
external Day Piece: per-toolkit renderers registered link-time into the backend's registry.

Pieces are Day's extension unit: a crate with one Rust API and per-toolkit native
renderers, enabled per backend by cargo features (`day build` wires them automatically).

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
