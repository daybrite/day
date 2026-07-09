---
title: Overview
description: What Day is, the bet it makes, the platforms it targets, and how the documentation is organized.
order: 1
section: Start here
---

**Day** is a Rust framework for building applications that look and behave like native
applications on every platform — because they are native applications.

You write your UI once, in Rust, as a declarative tree of **Pieces** (what SwiftUI calls a View
and Flutter calls a Widget). Each Piece is realized by a real platform widget — an
`NSTextField`, a `UILabel`, a Material button, a `GtkEntry`, a `QSlider`, a WinUI `TextBox` —
through a per-platform **toolkit backend**. Day owns layout, reactivity, localization,
accessibility policy, and scripting; the platform owns pixels, text input, scrolling physics,
and assistive technology.

```rust
use day::prelude::*;

fn counter() -> AnyPiece {
    let count = Signal::new(0i64);
    column((
        label(move || format!("{} clicks", count.get())),
        button("Tap me").action(move || count.update(|c| *c += 1)),
    ))
    .spacing(12.0)
    .padding(16.0)
    .any()
}
```

That function produces a native label above a native button on macOS, iOS, Android, Linux,
Windows, and OpenHarmony. There is no web view, no bundled renderer, and no per-platform fork.

## The bet

Every cross-platform approach picks something to sacrifice. Web-view shells sacrifice native
behavior and memory; custom renderers sacrifice native look-and-feel and inherit the burden of
reimplementing text, scrolling, and accessibility; per-platform native sacrifices the single
codebase. Day's bet is that the platform's own widgets already do most things better than any
framework can imitate — so it keeps them, and spends its effort only on the parts native
toolkits are genuinely bad at sharing:

- a **layout engine** that works identically everywhere while deferring to native measurement
  ([Layout](/docs/layout));
- **fine-grained reactivity** that builds the widget tree once and binds state directly to
  native attributes, with no virtual tree and no diffing ([Reactivity](/docs/reactivity));
- **localization** (Fluent), **accessibility**, and **scripting** designed in from the start
  rather than bolted on ([the four pillars](/docs/benefits#the-four-pillars));
- a **CLI** that builds, runs, tests, and [packages](/docs/packaging) for every target from one
  machine.

The cost side of the bet is real too — your app looks like a Mac app on a Mac and a Material app
on Android *whether you want that or not*, and heavy visual branding is the wrong fit.
[Why Day (and why not)](/docs/benefits) treats the tradeoffs seriously.

## The targets

A *target* is an `(OS, toolkit)` pair. One binary is compiled per target, containing only that
toolkit's backend — the AppKit build has no GTK code in it, and there's no runtime abstraction
layer to pay for.

| Target | OS | Toolkit |
|---|---|---|
| `macos-appkit` | macOS | AppKit |
| `ios-uikit` | iOS | UIKit |
| `android-widget` | Android | Material Components / android.view |
| `linux-gtk` | Linux | GTK 4 · libadwaita |
| `linux-qt` | Linux | Qt 6 Widgets |
| `windows-winui` | Windows | WinUI (XAML Islands) |
| `ohos-arkui` | OpenHarmony / HarmonyOS | ArkUI |
| `macos-gtk`, `macos-qt` | macOS | GTK 4, Qt 6 |
| `windows-gtk`, `windows-qt` | Windows | GTK 4, Qt 6 |

The last two rows exist because GTK and Qt are themselves portable — useful for development (all
five desktop toolkits run side by side on one Mac) and for teams that prefer one toolkit across
Linux and Windows. Maturity varies by target; [Platform support](/docs/platforms) says exactly
where each one stands rather than implying they're all equal.

## What it's like day to day

Everything is one Cargo project plus a small `day.yaml` manifest. `day launch -p <target>`
builds and runs; several `-p` flags launch targets in parallel. Tests run against a headless
mock toolkit in ordinary `cargo test`, and [dayscript](/docs/dayscript) drives the real app —
the same YAML script taps buttons and asserts labels on every platform, which is also how the
[gallery](/gallery) screenshots on this site are captured in CI.

Rust compiles ahead of time, so there is no hot reload — the inner loop is an incremental
compile and relaunch, usually seconds on desktop, with script replay to put you back on the
screen you were working on. If sub-second hot reload is central to how you work, that's a
genuine reason to look elsewhere, and we'd rather say so here than have you discover it in week
two.

## What Day is not

- **Not a renderer.** Day never rasterizes text or widgets itself. Even the `canvas` Piece
  records drawing commands and replays them through the platform's native 2D API.
- **Not pixel-identical across platforms.** The goal is consistent behavior and information
  architecture with native look and feel — not one skin everywhere.
- **Not a lowest common denominator.** Where platforms diverge, the API exposes the divergence
  (per-platform styling, capability flags) instead of hiding it; where a platform lacks a
  control, the backend composes one from primitives.
- **Not finished.** Day is young. The core model is stable and exercised by a real
  [Matrix chat client](https://github.com/daybrite/day/tree/main/apps/matrix) running on five
  targets, but APIs still move and some designed features aren't built yet. The docs mark those
  explicitly rather than describing the roadmap as the present.

## Finding your way around

The documentation is sequenced so each section assumes only the ones before it:

1. **Start here** — this page, the [tradeoffs](/docs/benefits), and
   [getting started](/docs/getting-started).
2. **Concepts** — [Pieces](/docs/pieces), [Reactivity](/docs/reactivity),
   [Layout](/docs/layout), [Styling](/docs/styling): the model, once, properly.
3. **Guides** — task-oriented pages on [navigation](/docs/navigation),
   [localization](/docs/localization), [accessibility](/docs/accessibility),
   [testing with dayscript](/docs/dayscript), [resources](/docs/resources), and
   [device capabilities](/docs/parts).
4. **Build & ship** — the [CLI](/docs/cli), [project anatomy](/docs/project-structure),
   [packaging & signing](/docs/packaging), and [platform status](/docs/platforms).
5. **Extend** — [how the extension model works](/docs/extending) and three worked tutorials.
6. **Under the hood** — [architecture](/docs/architecture) and
   [how rendering actually works](/docs/rendering), for when you want to see the machinery.
7. **Reference** — [per-widget and per-subsystem reference pages](/docs/reference), and a
   [condensed page for AI coding agents](/docs/for-agents).
