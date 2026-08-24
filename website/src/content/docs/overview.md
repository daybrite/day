---
title: Overview
description: What Day is, the bet it makes, the platforms it targets, and how the documentation is organized.
order: 1
section: Start here
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

**Day** is a Rust framework for building applications that look and behave like native
applications on every platform, because they are native applications.

You write your UI once, in Rust, as a declarative tree of **Pieces** (what SwiftUI calls a View
and Flutter calls a Widget). Each Piece is realized by a real platform widget (an
`NSTextField`, a `UILabel`, a Material button, a `GtkEntry`, a `QSlider`, a XAML `TextBox`)
through a per-platform **toolkit backend**. Day owns layout, reactivity, localization,
accessibility policy, and scripting; the platform owns pixels, text input, scrolling physics,
and assistive technology.

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
Windows, and OpenHarmony. There is no web view or bundled renderer, and no per-platform fork.

## The bet

Every cross-platform approach picks something to sacrifice — [Why Day](/docs/benefits)
walks the whole trade table. Day's bet is that the platform's own widgets already do most
things better than any framework can imitate, so it keeps them, and spends its effort only on
the parts native toolkits are bad at sharing:

- a layout engine that works identically everywhere while deferring to native measurement
  ([Layout](/docs/layout));
- fine-grained reactivity that builds the widget tree once and binds state directly to
  native attributes, with no virtual tree and no diffing ([Reactivity](/docs/reactivity));
- localization (Fluent), accessibility, and scripting designed into the core from
  the start ([how they compose](/docs/benefits#localized-accessible-scriptable-extensible));
- a CLI that builds, runs, tests, and [packages](/docs/packaging) for every target from one
  machine.

That bet has a price. Because the widgets are the platform's own, your app looks like a Mac app
on a Mac and a Material app on Android *whether you want that or not*, and heavy visual branding
is the wrong fit.
[Why Day (and why not)](/docs/benefits) covers the tradeoffs and when to pick something else.

## The targets

A *target* is an `(OS, toolkit)` pair. One binary is compiled per target, containing only that
toolkit's backend. The AppKit build has no GTK code in it, and there's no runtime abstraction
layer to pay for.

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

## What it's like day to day

Everything is one Cargo project plus a small `Day.toml` manifest. `day launch -p <target>`
builds and runs; several `-p` flags launch targets in parallel. Tests run against a headless
mock toolkit in ordinary `cargo test`, and [dayscript](/docs/dayscript) drives the real app.
The same YAML script taps buttons and asserts labels on every platform, which is also how the
[gallery](/gallery) screenshots on this site are captured in CI.

Rust compiles ahead of time, so there is no hot reload. The inner loop is an incremental
compile and relaunch, usually seconds on desktop, with script replay to put you back on the
screen you were working on. If sub-second hot reload is central to how you work, that is a
reason to look elsewhere: better to know now than in week two.

## What Day is not

- **Not a renderer:** Day never rasterizes text or widgets itself. Even the `canvas` Piece
  records drawing commands and replays them through the platform's native 2D API.
- **Not pixel-identical across platforms:** the goal is consistent behavior and information
  architecture with native look and feel, not one skin everywhere.
- **Not a lowest common denominator:** where platforms diverge, the API exposes the divergence
  (per-platform styling, capability flags) instead of hiding it; where a platform lacks a
  control, the backend composes one from primitives. And where you need a platform's own UI
  framework, you can drop into it: on macOS and iOS,
  [`day-piece-swiftui`](/docs/internal/swiftui) hosts your own SwiftUI views inside the Day tree,
  with typed Rust constructors generated from your Swift package.
- **Not finished:** Day is young. The core model is stable and exercised by a real
  Matrix chat client ([Day-Matrix](https://github.com/daybrite/Day-Matrix), a standalone Day
  app) running on five targets, but APIs still move and some designed features aren't built yet. The docs mark those
  explicitly rather than describing the roadmap as the present.

## Finding your way around

The documentation is sequenced so each section assumes only the ones before it:

1. **Start here** — this page, the [tradeoffs](/docs/benefits), and
   [getting started](/docs/getting-started).
2. **Coming from** — translation guides for [Flutter](/docs/day-for-flutter),
   [React Native](/docs/day-for-react-native), [SwiftUI](/docs/day-for-swiftui),
   [Compose](/docs/day-for-compose), [Electron](/docs/day-for-electron), and
   [other Rust frameworks](/docs/day-for-rust-frameworks) — start with yours, then come back.
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
   [how rendering works](/docs/rendering), for when you want to see the machinery.
8. **Reference** — [per-widget and per-subsystem reference pages](/docs/reference), and a
   [condensed page for AI coding agents](/docs/for-agents).
