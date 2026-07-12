---
title: Architecture
description: "The crate graph, one-binary-per-target compilation, and how the CLI and platform build systems cooperate."
order: 50
section: Under the hood
---

This page is the map: which crates exist, why the boundaries sit where they do, and how a build
actually happens on each platform. The companion page, [How rendering works](/docs/rendering),
follows a widget through the running system.

## The crate graph

```text
                       ┌────────────────┐
   your app ─────────► │      day       │  facade: re-exports + one launch() per backend feature
                       └───┬───────┬────┘
                           │       │ (exactly one, by Cargo feature)
              ┌────────────┘       └──────────────┐
              ▼                                   ▼
      ┌──────────────┐                   ┌──────────────────┐
      │  day-pieces  │  built-in pieces  │ toolkit backend   │  day-appkit / day-uikit /
      │              │  + Decorate API   │ (one per binary)  │  day-gtk / day-qt / day-android /
      └──────┬───────┘                   └────────┬─────────┘  day-winui / day-arkui / day-mock
             ▼                                    │ implements
      ┌──────────────┐   realized tree,           ▼
      │   day-core   │   layout engine,   ┌──────────────┐
      │              │   mounting, events │   day-spec   │  the Toolkit trait, props/patches,
      └──────┬───────┘ ◄──────────────────│              │  events, resources, window options
             ▼                            └──────────────┘
      ┌──────────────┐    ┌──────────────┐    ┌──────────────┐
      │ day-reactive │    │ day-geometry │    │  day-l10n /  │
      │ signals etc. │    │ Point/Size/… │    │  day-fluent  │
      └──────────────┘    └──────────────┘    └──────────────┘

   day-cli (the `day` binary)   day-script (dayscript engine, compiled into apps)
```

The load-bearing boundary is **`day-spec`**: it defines the `Toolkit` trait and the descriptor
types (`LabelProps`, `ButtonPatch`, events, …) that flow across it. `day-core` is written against
that trait and monomorphized over the concrete backend, so core code calls native operations
directly — no `dyn` dispatch, no message bus. Everything above `day-spec` is portable; everything
below it is one platform's business.

Around the core sit the extension surfaces: [`pieces/day-piece-*`](https://github.com/daybrite/day/tree/main/pieces) crates add widgets
([extension model](/docs/extending)), [`parts/day-part-*`](https://github.com/daybrite/day/tree/main/parts) add headless capabilities
([parts](/docs/parts)), and `day-mock` is a full Toolkit implementation with no display, used by
tests.

## One binary per target

A Day binary contains exactly one backend, selected by a Cargo feature at compile time. There is
no runtime toolkit registry, no abstraction layer choosing a backend at startup — the AppKit
build literally does not contain GTK code, and a call like "set this label's text" compiles down
to the backend's concrete function.

The costs of this choice are the ones you'd guess: n targets mean n compilations (CI budgets
around it; your laptop builds one at a time), and there's no single "universal Linux binary" that
picks GTK or Qt at runtime. The benefit is that the per-widget overhead of the framework is
approximately a function call, and dead-code elimination works on whole toolkits.

The same idea extends to piece renderers: backends expose a link-time registry (a `linkme`
distributed slice), and each piece crate's renderer registers into it during linking. Startup
iterates the slice once to build the kind → renderer table. Registration failures are link
errors, not runtime surprises.

## How a build works

`day build -p <target>` orchestrates; platform tools do the platform work. Desktop targets are
plain cargo builds (each target gets its own `CARGO_TARGET_DIR`, so parallel target builds never
contend). Mobile targets invert control with the **callback pattern**, borrowed deliberately
from Flutter: the checked-in platform project drives, and calls back into `day` for the Rust
part, so building from Xcode/Android Studio and building from the CLI produce identical results
and neither goes stale.

```text
 day build -p ios-uikit                    day build -p android-widget
──────────────────────────                ─────────────────────────────
 day CLI                                   day CLI
   │  generates DayPieces SwiftPM pkg        │  cargo-ndk → libapp.so per ABI
   │  (piece Swift shims + deps)             │  writes day-pieces.json (piece java/
   ▼                                         │  gradle deps/permissions) + app props
 xcodebuild ──► "Build Rust (day)" phase     ▼
   │            calls `day xcode-backend    gradle ──► reads the generated files,
   │            build` → cargo staticlib     │         stages jniLibs, merges manifests
   ▼            for the iOS triple           ▼
 Runner.app  ◄── links libapp.a            app-debug.apk
```

The same shape covers OpenHarmony (hvigor builds the ArkTS host around a cross-compiled
`libentry.so`). Metadata flows one way: `Day.toml` (identity, version) is conveyed into
generated, gitignored files that the checked-in projects read — the scaffolds themselves are
never edited by tooling. [Project structure](/docs/project-structure) documents every directory;
[Packaging](/docs/packaging) covers the signed-artifact pipeline built on top.

## The native seams

Each backend crosses into its toolkit using the narrowest viable mechanism:

| Backend | Seam |
|---|---|
| AppKit / UIKit | `objc2` bindings — Rust calls Objective-C runtime directly, no shim |
| GTK | `gtk4-rs` (gobject bindings) |
| Qt | a small hand-written C++ shim (`day-qt-sys`) compiled by `cc` at build time; Rust calls its C API |
| WinUI | same pattern with C++/WinRT (`day-winui-sys`) |
| Android | JNI plus a small Java bridge class shipped with the framework; Rust holds `GlobalRef`s to widgets |
| ArkUI | the ArkUI NDK C API (`day-arkui-sys`) |

The shims are deliberately boring: create widget, set property, forward event. All policy —
layout, reactivity, when to update what — lives on the Rust side of the seam, which keeps each
new backend's surface area small and auditable. This "one thin shim per toolkit" pattern is
inherited from the systems Day descends from, where it carried six backends in production.

## Where the CLI fits

`day-cli` is a separate binary with no dependency on the UI crates. Its jobs: scaffolding
(`day new`), orchestration (`build`/`launch` across targets in parallel, streaming prefixed
logs), diagnosis (`doctor`, with per-toolkit probes and fix-it text), validation (`lint`),
distribution (`pack`, `sign`), and the plumbing subcommands the platform builds call back into.
It's built in the flutter_tools mold — services behind injectable traits for testability, a
stable JSON event stream for machines (`--format json`), documented exit codes — because CI
systems and AI agents are first-class users of the tool, not afterthoughts.
