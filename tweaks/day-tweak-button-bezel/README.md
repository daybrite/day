<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-tweak-button-bezel

Choose an AppKit bezel style for a stock Day button: `.bezel(Bezel::Textured)` and
friends on any `button(...)`.

It is macOS-only and amounts to a single native call — which is the point. This crate
exists mainly as the smallest possible example of the tweak pattern, worth a read if you
plan to write one.

Tweaks are Day's smallest kind of extension: a little crate that adjusts the native widget
behind a built-in piece. On platforms a tweak doesn't cover, it quietly does nothing, so
your app code stays free of platform checks.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's own widgets — AppKit, UIKit, Android's Material widgets, GTK 4, Qt 6,
XAML, and ArkUI — from one codebase. When you write `button("Save")`, macOS shows an
`NSButton` and Android shows a Material button. The framework also ships the tooling around
the app: the `day` CLI, a VS Code extension, GitHub CI workflows, localization,
accessibility, and dayscript automation.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
