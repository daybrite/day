<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-piece-combobox

An editable combo box for Day apps: type a value or pick one from the dropdown.

One Rust API, and a real native combo control on every platform that has one —
`NSComboBox` on macOS, `AutoCompleteTextView` on Android, `GtkComboBoxText` with an
entry on Linux, an editable `QComboBox` on Qt, an editable `ComboBox` on XAML. The
value is the text: a `Signal<String>` bound two-way, with a reactive list of
suggestions. iOS has no native combo-box control, so this piece carries no iOS
renderer; use `picker` or `text_field` there.

This crate also shows what a piece with its own native baggage looks like: it carries
its per-platform renderers, two C++ shims (Qt, XAML), and an Android Java factory —
all inside one crate, with no edits to Day itself.

Pieces are Day's reusable UI components, shipped as ordinary crates: one Rust API in
front, a real native control per platform behind it. Enable the backends you build for
with cargo features, and `day build` wires up the native side automatically.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's own widgets — AppKit, UIKit, Android's Material widgets, GTK 4, Qt 6,
XAML, and ArkUI — from one codebase. When you write `button("Save")`, macOS shows an
`NSButton` and Android shows a Material button. The framework also ships the tooling around
the app: the `day` CLI, a VS Code extension, GitHub CI workflows, localization,
accessibility, and dayscript automation.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
