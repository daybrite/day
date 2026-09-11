---
title: Overview
description: What Day is, what it does itself, the platforms it targets, and how the documentation is organized.
order: 1
section: Start here
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

**Day** is a Rust framework for building applications out of each platform's own native widgets.

You describe your UI in Rust as a tree of **Pieces**, similar to Views in SwiftUI or Widgets in
Flutter. Pieces can represent native controls, layouts, or compositions of other pieces. A
**toolkit [backend](/docs/glossary#backend)** creates the native controls for the selected platform. Day handles layout, reactive updates, localization,
accessibility configuration, and scripting. Native toolkits provide widget rendering, text
input, scrolling behavior, and integration with assistive technology.

```rust
use day::prelude::*;

fn counter() -> impl Piece {
    let count = Signal::new(0i64);
    column((
        label(move || format!("{} clicks", count.get())),
        button("Tap me").action(move || count.update(|c| *c += 1)),
    ))
    .spacing(12.0)
    .padding(16.0)
}
```

That function produces a native label above a native button on macOS, iOS, Android, Linux,
Windows, and OpenHarmony. Each target build includes its selected toolkit backend and shares the example’s Rust UI code.

Day also includes development tools:

- The [`day` CLI](/docs/cli) for creating, checking, building, running, and packaging projects.
- A [VS Code extension](https://marketplace.visualstudio.com/items?itemName=daybrite.day-vscode)
  for working with Day apps in the editor.
- A reusable [GitHub Actions workflow](/docs/cli#continuous-integration) for automated builds
  and tests.
- [Dayscript](/docs/dayscript) for app automation and testing.

See the [localization](/docs/localization) and [accessibility](/docs/accessibility) guides for
those features, or continue below for the UI model.

## What Day does itself

Day keeps the platform's widgets and concentrates its own code on the parts native toolkits
don't share:

- a shared layout engine that uses native widget measurements
  ([Layout](/docs/layout));
- fine-grained reactivity that builds the widget tree once and binds state directly to
  native attributes ([Reactivity](/docs/reactivity));
- localization ([Fluent](/docs/glossary#fluent)), accessibility, and scripting in the core
  ([how they compose](/docs/benefits#localized-accessible-scriptable-extensible));
- a CLI for building, running, testing, and [packaging apps](/docs/packaging), using the host
  tools and SDKs required by each target.

Native controls give your app a different appearance on each platform. If your design requires
a highly customized interface that looks identical across platforms, review the tradeoffs in
[Why Day](/docs/benefits).

## The targets

A *target* is an `(OS, toolkit)` pair. One binary is compiled per target, containing only that
toolkit's backend. The AppKit build contains only AppKit code, and each widget call compiles to
a direct call into that toolkit.

| Target | OS | Toolkit | Tier |
|---|---|---|---|
| `macos-appkit` | macOS | AppKit | [Tier 1](/docs/platforms#support-tiers) |
| `ios-uikit` | iOS | UIKit | [Tier 1](/docs/platforms#support-tiers) |
| `android-mdc` | Android | Material Components / android.view | [Tier 1](/docs/platforms#support-tiers) |
| `linux-gtk` | Linux | GTK 4 · libadwaita | [Tier 2](/docs/platforms#support-tiers) |
| `linux-qt` | Linux | Qt 6 Widgets | [Tier 2](/docs/platforms#support-tiers) |
| `windows-xaml` | Windows | XAML (XAML Islands) | [Tier 2](/docs/platforms#support-tiers) |
| `harmony-arkui` | OpenHarmony / HarmonyOS | ArkUI | [Tier 3](/docs/platforms#support-tiers) |
| `web-dom` | Web (any modern browser) | DOM — wasm32 + semantic HTML | [Tier 3](/docs/platforms#support-tiers) |
| `macos-gtk`, `macos-qt` | macOS | GTK 4, Qt 6 | [Tier 4](/docs/platforms#support-tiers) |
| `windows-gtk`, `windows-qt` | Windows | GTK 4, Qt 6 | [Tier 4](/docs/platforms#support-tiers) |

The last two rows exist because GTK and Qt are themselves portable, useful for development
(`macos-appkit`, `macos-gtk`, and `macos-qt` run side by side on one Mac) and for teams that
prefer one toolkit across Linux and Windows. Maturity varies by target, and the tier in the last
column says how much testing and maintenance each one gets: Tier 1 is fully supported and
thoroughly tested, Tier 4 exists for compatibility testing. [Support tiers](/docs/platforms#support-tiers)
defines all four, and [Platform support](/docs/platforms) has the per-target detail.

## Development workflow

A Day app uses a Cargo project for its Rust code and a [`Day.toml`](/docs/glossary#day-toml)
manifest for app and platform configuration. `day launch -p <target>`
builds and runs; several `-p` flags launch targets in parallel. Tests run against a headless
mock toolkit in ordinary `cargo test`, and [dayscript](/docs/dayscript) drives the real app.
Walkthroughs can tap buttons, check labels, and capture screenshots across your app’s targets.
The [gallery](/gallery) shows screenshots captured by app walkthroughs in CI.

Day uses incremental compilation and relaunching rather than hot reload. Replay a dayscript
walkthrough to return to the screen you’re developing. Rebuild times depend on your app,
target, and development machine.

## What to expect

- **The platform draws everything.** Text and widgets are drawn by the platform, never by Day.
  Even the `canvas` Piece records drawing commands and replays them through the platform's
  native 2D API.
- **Native on each platform rather than identical across them.** The goal is consistent
  behavior and information architecture with each platform's own look and feel.
- **Handle platform differences explicitly.** Use platform-specific styling and
  [capability flags](/docs/glossary#capability) to adapt your interface. Backends can compose
  missing controls from simpler components. On macOS and iOS,
  [`day-piece-swiftui`](/docs/internal/swiftui) lets you embed SwiftUI views using typed Rust
  constructors generated from your Swift package.
- **Check feature and target maturity.** Apps such as
  [Day-Matrix](https://github.com/daybrite/Day-Matrix) use Day, but APIs and platform coverage
  are still developing. Review [platform support](/docs/platforms) and the relevant API
  documentation for your app’s requirements.

## Finding your way around

Use the sections below to find introductory material, framework concepts, task guides, and
deployment instructions:

1. **Start here** — this page, the [tradeoffs](/docs/benefits), and
   [getting started](/docs/getting-started).
2. **Coming from** — guides that relate familiar concepts from [Flutter](/docs/day-for-flutter),
   [React Native](/docs/day-for-react-native), [SwiftUI](/docs/day-for-swiftui),
   [Compose](/docs/day-for-compose), [Electron](/docs/day-for-electron), and
   [other Rust frameworks](/docs/day-for-rust-frameworks) to Day.
3. **Concepts** — [Pieces](/docs/pieces), [Reactivity](/docs/reactivity),
   [Layout](/docs/layout), [Styling](/docs/styling): the model in full.
4. **Guides** — task-oriented pages on [navigation](/docs/navigation),
   [localization](/docs/localization), [accessibility](/docs/accessibility),
   [testing with dayscript](/docs/dayscript), [resources](/docs/resources), and
   [device capabilities](/docs/parts).
5. **Build & ship** — the [CLI](/docs/cli), [project anatomy](/docs/project-structure),
   [packaging & signing](/docs/packaging), [platform status](/docs/platforms), and a
   full page per target (e.g. [macOS](/docs/platforms/macos-appkit),
   [Android](/docs/platforms/android-mdc)).
6. **Extend** — [how the extension model works](/docs/extending) and three worked tutorials.
7. **Under the hood** — [architecture](/docs/architecture) and
   [how rendering works](/docs/rendering).
8. **Reference** — [per-widget and per-subsystem reference pages](/docs/reference), a
   [condensed page for AI coding agents](/docs/for-agents), and a [glossary](/docs/glossary) of
   the words these pages use.
