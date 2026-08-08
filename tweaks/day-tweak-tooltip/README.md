<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-tweak-tooltip

Give any Day piece a native help tooltip: `.tooltip("Save your changes (⌘S)")` on any
`button(...)`, `label(...)`, or other piece. It shows on hover on macOS and GTK, and on
long-press on Android.

The interesting part is the three access tiers behind one modifier: objc2 (`NSView.setToolTip:`),
gtk4-rs (`GtkWidget.set_tooltip_text`), and JNI (`View.setTooltipText`). On every other toolkit
`.tooltip(...)` quietly does nothing, so your app code stays free of platform checks.

Tweaks are Day's smallest kind of extension: a little crate that adjusts the native widget
behind a built-in piece. This one is a good mid-size example — one modifier, three native APIs,
no companion C++ shim.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, XAML, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
