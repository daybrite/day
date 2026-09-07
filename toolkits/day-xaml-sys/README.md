<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-xaml-sys

The C++/WinRT side of Day's Windows backend.

This crate holds the XAML Islands shim and the raw `extern "C"` declarations for it. The
build script compiles the shim against the Windows kits, and XAML objects cross into Rust
as opaque pointers. There is deliberately no safe API here: the safe layer is
[`day-xaml`](https://crates.io/crates/day-xaml), and apps depend on neither crate
directly — the backend arrives through a cargo feature on
[`day`](https://crates.io/crates/day).

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's own widgets — AppKit, UIKit, Android's Material widgets, GTK 4, Qt 6,
XAML, and ArkUI — from one codebase. When you write `button("Save")`, macOS shows an
`NSButton` and Android shows a Material button. The framework also ships the tooling around
the app: the `day` CLI, a VS Code extension, GitHub CI workflows, localization,
accessibility, and dayscript automation.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
