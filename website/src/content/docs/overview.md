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
Windows, and OpenHarmony. Each binary links only its own platform's toolkit, and every platform
shares the same UI code.

## What Day does itself

Day keeps the platform's widgets and concentrates its own code on the parts native toolkits
don't share:

- a layout engine that works identically everywhere while deferring to native measurement
  ([Layout](/docs/layout));
- fine-grained reactivity that builds the widget tree once and binds state directly to
  native attributes ([Reactivity](/docs/reactivity));
- localization (Fluent), accessibility, and scripting designed into the core from
  the start ([how they compose](/docs/benefits#localized-accessible-scriptable-extensible));
- a CLI that builds, runs, tests, and [packages](/docs/packaging) for every target from one
  machine.

The cost is that your app looks like a Mac app on a Mac and a Material app on Android, so heavy
visual branding is a poor fit. [Why Day](/docs/benefits) covers the tradeoffs and when to pick
something else.

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

## What it's like day to day

Everything is one Cargo project plus a small `Day.toml` manifest. `day launch -p <target>`
builds and runs; several `-p` flags launch targets in parallel. Tests run against a headless
mock toolkit in ordinary `cargo test`, and [dayscript](/docs/dayscript) drives the real app.
The same YAML script taps buttons and asserts labels on every platform, which is also how the
[gallery](/gallery) screenshots on this site are captured in CI.

Rust compiles ahead of time, so there is no hot reload. The inner loop is an incremental
compile and relaunch, usually seconds on desktop, with script replay to put you back on the
screen you were working on. If sub-second hot reload is central to how you work, another
framework will suit you better.

## What to expect

- **The platform draws everything.** Text and widgets are drawn by the platform, never by Day.
  Even the `canvas` Piece records drawing commands and replays them through the platform's
  native 2D API.
- **Native on each platform rather than identical across them.** The goal is consistent
  behavior and information architecture with each platform's own look and feel.
- **Platform differences stay visible.** Where platforms diverge, the API shows the divergence
  (per-platform styling, capability flags); where a platform lacks a control, the backend
  composes one from primitives. Where you need a platform's own UI framework, you can use it:
  on macOS and iOS, [`day-piece-swiftui`](/docs/internal/swiftui) hosts your own SwiftUI views
  inside the Day tree, with typed Rust constructors generated from your Swift package.
- **Day is young.** The core model is stable and runs a Matrix chat client
  ([Day-Matrix](https://github.com/daybrite/Day-Matrix), a standalone Day app) on five targets,
  but APIs still move and some designed features aren't built yet. The docs mark those.

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
