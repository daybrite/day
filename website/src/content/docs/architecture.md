---
title: Architecture
description: "The crate graph, one-binary-per-target compilation, and how the CLI and platform build systems cooperate."
order: 50
section: Contributing
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day separates the shared UI model from platform-specific widget implementations. This page
explains the crate boundaries and build process. [Rendering](/contributing/rendering) follows the
corresponding runtime path, from creating a widget to handling its events.

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
      │              │  + Decorate API   │ (one per binary)  │  day-gtk / day-qt / day-mdc /
      └──────┬───────┘                   └────────┬─────────┘  day-xaml / day-arkui / day-dom /
             │                                    │            day-mock
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
   day-build (each app's build.rs: typed resource constants, SwiftUI bindings)
```

Support crates omitted from the diagram: `day-fonts` and `day-vector` (shared resource rules the
CLI and runtime agree on), `day-toolchain` (SDK discovery). Crash reporting lives in the external
[`day-piece-break`](https://github.com/daybrite/day-piece-break) repository.
JS/TS miniapps run on [`day-lite`](https://github.com/daybrite/day-lite), which lives in its own
repository.

`day-spec` is the boundary between portable and platform code: it defines the `Toolkit` trait and
the descriptor types (`LabelProps`, `ButtonPatch`, events, …) that flow across it. `day-core` uses the toolkit contract to manage trees and delegate widget operations. Public
portable APIs can still have unsupported target operations; a dependency boundary alone does
not establish complete runtime support.

Around the core sit the extension surfaces: [`pieces/day-piece-*`](https://github.com/daybrite/day/tree/main/pieces) crates add widgets
([extension model](/contributing/extending)), [`parts/day-part-*`](https://github.com/daybrite/day/tree/main/parts) add headless capabilities
([parts](/docs/reference/parts)), and `day-mock` is a full Toolkit implementation with no display, used by
tests.

## One binary per target

A Day binary contains exactly one backend, selected by a Cargo feature at compile time. The
backend is chosen when the binary is built, so the AppKit build contains only AppKit code, and
a call like "set this label's text" compiles down to the backend's concrete function.

The cost is that n [targets](/docs/reference/glossary#target) mean n compilations (CI budgets around it; your laptop builds one at
a time), and a Linux app ships as a GTK build or a Qt build rather than one binary that picks at
runtime. The benefit is that a widget update compiles to a direct call into the one linked
backend, and dead-code elimination works on whole [toolkits](/docs/reference/glossary#toolkit).

The same idea extends to [piece](/docs/reference/glossary#piece) renderers: backends expose a link-time registry (a `linkme`
distributed slice), and each piece crate's renderer registers into it during linking. Startup
iterates the slice once to build the [kind](/docs/reference/glossary#kind) → renderer table. A missing or discarded registration can instead produce a runtime placeholder. Link success
alone does not prove that every renderer was retained; exercise the extension on the target. The one exception is `day-dom`: `linkme` has no wasm32
implementation, so the web backend keeps a runtime registry that pieces register into from
their constructors.

## How a build works

`day build -t <target>` orchestrates; platform tools do the platform work. Most desktop targets
are plain cargo builds (each target gets its own `CARGO_TARGET_DIR`, so parallel target builds
never contend); the exception is `macos-appkit`, which builds through its `platform/macos/`
Xcode host project like a mobile target — including any macOS Swift a dependency contributes,
the [SwiftUI embedding](/docs/reference/swiftui) path. Mobile targets invert control with the **callback pattern**, borrowed
from Flutter: the checked-in platform project drives, and calls back into `day` for the Rust
part, so building from Xcode/Android Studio and building from the CLI produce identical results
and neither goes stale.

```text
 day build -t ios-uikit                    day build -t android-mdc
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
`libentry.so`), and `macos-appkit` generates its own `DayPieces` SwiftPM package (at
`build/day/macos/DayPieces`), referenced by the `platform/macos/`
Xcode host project. Metadata flows one way: [`Day.toml`](/docs/reference/glossary#day-toml) (identity) and the Cargo
`version` are conveyed into generated, gitignored files that the checked-in projects read; the
generated projects are derived under `build/`, while explicitly materialized native projects
are app-owned customization overrides. `day project add-target` can create host sources.
Inspect the selected host mode before editing a generated file. [Project structure](/docs/reference/project-structure) documents every directory;
[Packaging](/docs/how-to/packaging) covers the signed-artifact pipeline built on top.

## How each backend reaches its toolkit

Each backend crosses into its toolkit using the narrowest viable mechanism:

| Backend | Mechanism |
|---|---|
| AppKit / UIKit | `objc2` bindings: Rust calls the Objective-C runtime directly, no shim |
| GTK | `gtk4-rs` (gobject bindings) |
| Qt | a small hand-written C++ shim (`day-qt-sys`) compiled by `cc` at build time; Rust calls its C API |
| XAML | same pattern with C++/WinRT (`day-xaml-sys`) |
| Android | JNI plus a small Java bridge class shipped with the framework; Rust holds `GlobalRef`s to widgets |
| ArkUI | the ArkUI NDK C API through openharmony-rs's `ohos-sys` bindings; the ArkTS-facing module through ohos-rs's `napi-ohos` (no C++) |
| DOM | a wasm32 `extern "C"` boundary implemented by a small JS shim the CLI embeds in the page |

Each shim creates widgets, sets properties, and forwards events. Layout policy and reactivity have shared Rust implementations. A backend must still implement
widget semantics, native measurement, callbacks, lifetime, resources, and platform integration;
code size and effort are not established by this boundary.

## Where the CLI fits

`day-cli` is a separate binary with no dependency on the UI crates. Its jobs: scaffolding
(`day new`), orchestration (`build`/`launch` across targets in parallel, streaming prefixed
logs), diagnosis (`doctor`, with per-toolkit probes and fix-it text), validation (`lint`),
distribution (`pack`, `sign`), and the plumbing subcommands the platform builds call back into.
It's built in the flutter_tools mold: services behind injectable traits for testability, a
JSON event stream (`--format json`) whose current schema must be checked against the CLI version, and documented exit codes, so scripts and CI
consume the same interface people do.

## Source and lifecycle contracts

The [facade manifest](https://github.com/daybrite/day/blob/main/crates/day/Cargo.toml) selects
backend features; `winui` includes `xaml` because they share one implementation.
The [core launch path](https://github.com/daybrite/day/blob/main/crates/day-core/src/lib.rs)
installs the toolkit, scheduler, and executor hooks before building the app tree.
`Platform::RUN_ENDS_APP` distinguishes an owned loop from a host-owned loop; a return from a
mobile/web host setup is not necessarily app exit. [Lifecycle](/docs/explanation/lifecycle)
and [async shutdown](/docs/explanation/async) document cleanup and the limits of OS callbacks.

Use [backend contracts](/contributing/backend-contracts) for implementation obligations.
The [extension model](/contributing/extending)
covers optional native contributions. See the
[platform reference](/docs/reference/platforms) for target requirements and limitations.
