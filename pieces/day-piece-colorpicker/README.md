<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-piece-colorpicker

A color well — a swatch showing the current color that opens a chooser when pressed —
bound two-way to a `Signal<Color>`, in two idioms.

`color_picker(tint)` opens the platform's own chooser where the toolkit has one: the
shared `NSColorPanel` on macOS, `UIColorPickerViewController` on iOS, `GtkColorDialog`,
`QColorDialog`, the XAML `ColorPicker` in a flyout, and `<input type="color">` on the web.
Each brings its own tabs, palettes and eyedropper, because each IS the system chooser.

`color_picker(tint).composed()` opens a panel Day draws itself, out of ordinary pieces and
a canvas: a saturation/brightness field, a hue strip, an opacity strip and a preset
palette. It carries no native code and no per-backend renderer, so it is the same picker
on all nine targets — which is what makes it the answer on the two that ship no color
chooser at any layer (Android and HarmonyOS), and an option anywhere an app would rather
have one identical color experience than platform chrome.

The default picks between them: the platform's where there is one, Day's where there is
not. The bound value is Day's ordinary `Color`, so the same signal drives a glyph tint, a
surface background, a canvas fill or a gradient stop with no conversion. See
`docs/colorpicker.md` in the Day repository for the per-platform table and the honest
non-promises, and `docs/color.md` for what a native pick can carry that `Color` cannot.

Pieces are Day's reusable UI components, shipped as ordinary crates: one Rust API in
front, a real native control per platform behind it. Enable the backends you build for
with cargo features, and `day build` wires up the native side automatically.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, XAML, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
