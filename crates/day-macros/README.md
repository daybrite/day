<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-macros

The procedural macros behind Day, re-exported by the `day` crate. You depend on `day`, not
on this crate.

- `build_path!("M 12,21 C …")` turns SVG path data into a `PathBuilder` method chain at
  compile time. The whole SVG 1.1 path grammar is accepted, arcs become cubics once rather
  than on every draw, and malformed data is a compile error naming the offending character.
- `#[derive(Observable)]` gives a struct typed per-field accessors on every store that
  holds it, so a view can observe `store.name()` instead of the whole record.
- `#[derive(Model)]` adds the persistence half: table name, columns, and the row mappers
  that `day-persistence` needs, with `#[model(…)]` attributes for keys, indexes, and
  codecs.
- `#[day::test]` marks a function that builds a conformance `Case` as a Day test, so
  `day test` and the mock harness find it.

The crate has no dependencies. Each macro reads a literal or a struct definition and emits
Rust as text, so there is no parser framework to compile.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's own widgets — AppKit, UIKit, Android's Material widgets, GTK 4, Qt 6,
XAML, and ArkUI — from one codebase. When you write `button("Save")`, macOS shows an
`NSButton` and Android shows a Material button. The framework also ships the tooling around
the app: the `day` CLI, a VS Code extension, GitHub CI workflows, localization,
accessibility, and dayscript automation.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
