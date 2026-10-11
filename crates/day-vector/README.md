<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-vector

Day's vector-graphics engine: parse an SVG, rasterize it to PNG, and write it out in the
icon formats each platform wants.

The parser and rasterizer are [resvg](https://crates.io/crates/resvg) with text shaping
turned off, so sources must carry outlined text. Everything around it is written here
because the formats are small: `.ico` and `.icns` are PNG containers, an Android
VectorDrawable is a constrained XML form of the SVG tree, a XAML `Geometry` is the same
path data in another spelling, and an SF Symbol template is cut apart and reassembled as
text. The crate also classifies a source as plain SVG, an SF Symbol template, or a
`.symbolset` bundle, and extracts one weight-and-scale variant from a template as a
standalone glyph.

The `day` CLI uses it at build time: `day icon build` renders an app-icon master into
every platform's icon set, and resource staging converts `resource/vectors/` into the form
each toolkit loads natively. Nothing in the crate touches the filesystem or a toolkit, so
the same byte-in, byte-out functions compile on wasm and can rasterize a downloaded SVG
inside an app.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's own widgets — AppKit, UIKit, Android's Material widgets, GTK 4, Qt 6,
XAML, and ArkUI — from one codebase. When you write `button("Save")`, macOS shows an
`NSButton` and Android shows a Material button. The framework also ships the tooling around
the app: the `day` CLI, a VS Code extension, GitHub CI workflows, localization,
accessibility, and dayscript automation.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
