---
title: Why Day
description: How Day compares with web-view shells, custom renderers, and per-platform native, including the cases where Day is the wrong choice.
order: 2
section: Start here
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day shares UI code across platforms while using their native controls. That choice affects
appearance, accessibility, portability, and the work required to add a feature. The comparisons
below explain the tradeoffs and where another approach may suit an app better.

## The options

Four established ways to ship one app on many platforms:

| Approach | Examples | Strength | Tradeoff |
|---|---|---|---|
| Web view shell | Electron, Tauri | Reuse web UI code and libraries | Platform features need integration with the host application |
| Custom renderer | Flutter, egui, Slint | Control the interface’s appearance across platforms | Native behavior and accessibility depend on the framework’s integration |
| Shared logic, separate UI | [Kotlin Multiplatform with native UI](https://kmp.jetbrains.com/templates/) | Share business logic while designing each platform’s interface separately | Maintain multiple UI implementations |
| Native widgets, shared UI code | **Day**, React Native* | Share UI declarations while using native controls | Appearance varies by platform; unsupported APIs need additional integration |

These are architectural approaches, not exclusive categories: a framework may support more
than one. For example, Kotlin Multiplatform can also share UI through
[Compose Multiplatform](https://kotlinlang.org/multiplatform/).

\* React Native uses native host components through its
[renderer](https://reactnative.dev/architecture/render-pipeline). Its language, update model,
and platform coverage differ from Day’s.

## What you get

### Native fidelity without per-platform UI code

Day uses native widgets for text rendering, input, selection, scrolling, and accessibility
integration. Their appearance and behavior follow the platform toolkit.

### Reactive updates to native controls

Day connects application state to native widget properties through reactive bindings. When a
value changes, dependent bindings update the affected properties—for example, a label’s text
([how this works](/docs/reactivity)).
The compiler monomorphizes your app against one toolkit [backend](/docs/glossary#backend) per binary, so a widget update
is a direct call. Binaries are ordinary Rust binaries that link the system's libraries.

### Write your UI and application logic in Rust

Write UI components, application state, and business logic in Rust, using the same type system
and ownership rules throughout. Platform integrations may also require native code.

### Localized, accessible, scriptable, extensible

These features work together. [Reactive](/docs/glossary#reactive) localized strings update when
the app’s [locale](/docs/glossary#locale) changes, and stable accessibility identifiers let
automation find controls. Use [dayscript](/docs/glossary#dayscript)
[walkthroughs](/docs/glossary#walkthrough) to [test workflows](/docs/dayscript),
[check accessibility](/docs/accessibility), and
[capture screenshots in different languages](/docs/localization).

1. **Localizable** — Mozilla [Fluent](/docs/glossary#fluent) throughout, with ICU-correct plurals, number and date
   formatting, and collation-aware sorting, with locale data thinned to the locales you ship.
   The current locale is a [signal](/docs/glossary#signal). ([guide](/docs/localization))
2. **Accessible** — native widgets give a native accessibility tree as the baseline; Day adds
   uniform annotations and stable identifiers, and CI can diff the native tree against your
   declarations. ([guide](/docs/accessibility))
3. **Scriptable** — use YAML scripts to interact with your running app and check its behavior
   across supported platforms. ([guide](/docs/dayscript))
4. **Extensible** — add widgets as Rust crates by composing existing pieces or implementing
   native code for each toolkit, without modifying Day itself. ([how](/docs/extending))

### Tools for development and automation

Use `day doctor` to check [target](/docs/glossary#target) toolchains and get setup guidance,
`day launch` to build and run apps, and `day pack` to [create distribution packages](/docs/packaging).
JSON output supports integration with automation tools.

## What you give up

### Hot reload

Day requires an incremental rebuild and relaunch to apply code changes. Dayscript can replay
interactions to return to the screen you’re developing, but it does not preserve running
application state as hot reload would. Consider this workflow when evaluating Day for frequent
UI experimentation.

### Pixel-level brand control

Day’s native controls differ in appearance across platforms. If your app requires identical
custom controls and animations everywhere, evaluate frameworks that render their own
interfaces, such as Flutter, Slint, or egui.
[Styling](/docs/styling) lists what you can restyle and what stays native. On macOS and iOS,
use [SwiftUI embedding](/docs/internal/swiftui) to add custom SwiftUI controls to your interface
as Day [pieces](/docs/glossary#piece).

### Ecosystem maturity

Day’s component library and ecosystem are still developing. Check that the controls,
integrations, and [platform support](/docs/platforms) your app needs are available before
committing to the framework.

Testing and maintenance vary by target. The [support tiers](/docs/platforms#support-tiers)
describe the coverage and expectations for each platform/toolkit combination.

### Rust, with a single-threaded UI

If your team doesn't know Rust, learning it is part of the
project. UI state is main-thread-only by construction (`Signal` isn't `Send`); background work
returns through explicit `Setter`/`on_main` calls. These constraints help prevent unsafe
cross-thread access to UI state and require explicit communication between background tasks
and the UI.

### Platform variance is still yours to test

Test each target platform for differences in focus order, dialogs, and text layout. Use dayscript
for automated checks, and manually test interactions it cannot control, including system
keyboards, input method editors (IMEs), and OS permission dialogs.

### Framework-mediated platform access

If Day does not expose a platform API your app needs, you may need to implement an integration.
Use the [parts](/docs/parts) pattern to organize platform-specific code behind a shared Rust API.
Account for the implementation and testing effort on each target.

## Choosing

Consider **Electron or Tauri** if you want to build your desktop interface with web technologies
and reuse your team’s web development experience. Consider **Flutter** if a consistent custom
interface across platforms and a hot-reload workflow are priorities for your team.
Consider **separate native implementations** if each platform needs a distinct interface or
extensive use of its native APIs. Consider **Day** if you want to share Rust UI code while using
native platform controls, and its current features and target support meet your app’s needs.

---

Follow the [getting-started guide](/docs/getting-started) to create and run your first Day app.
