<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-fluent

Localized strings for Day apps, built on Mozilla Fluent.

You keep one plain-text `.ftl` file per language, and the typed `res::str::*` functions
[`day-build`](https://crates.io/crates/day-build) generates from them return the right string
for the current locale: `res::str::greeting(name)`. A missing key or a wrong argument is a
compile error. The result is reactive: switch languages while the app runs and every visible
string updates in place. Fluent puts the grammar problems of translation (plural forms,
gender, selection) in the translation file, where translators can actually solve them.

Day's standard strings (dialog buttons, menu commands, the Settings and About labels) come
with the same functions, as `res::str::day_cancel()` and the like. Underneath sits
[`day-l10n`](https://crates.io/crates/day-l10n), the engine; all of it arrives with
[`day`](https://crates.io/crates/day).

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's own widgets — AppKit, UIKit, Android's Material widgets, GTK 4, Qt 6,
WinUI, and ArkUI — from one codebase. When you write `button("Save")`, macOS shows an
`NSButton` and Android shows a Material button. The framework also ships the tooling around
the app: the `day` CLI, a VS Code extension, GitHub CI workflows, localization,
accessibility, and dayscript automation.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
