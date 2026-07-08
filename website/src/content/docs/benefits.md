---
title: Why Day
description: "The benefits: native fidelity, one codebase, fine-grained reactivity, and four pillars built in."
order: 2
---

Cross-platform tools usually make you choose between native fidelity and a single codebase. Day
gives you both.

## Native widgets

Your Pieces become platform widgets, so you inherit everything the platform does better than a
cross-platform renderer can imitate:

- system text rendering, input methods, and spellcheck;
- native scrolling physics, selection, and drag;
- the platform's own accessibility tree (VoiceOver, TalkBack, Narrator, Orca);
- platform theming, dark mode, dynamic type, and right-to-left;
- OS updates that improve your app without you shipping anything.

A Day app on macOS uses AppKit; on Android it uses `android.widget`; on Linux it adopts
libadwaita.

## One codebase, one language

UI, state, layout, localization, and tests are all Rust. You don't maintain an FFI seam between
your view layer and your logic, a separate template DSL, or per-platform UI forks. The
[showcase app](/gallery) is a single Rust program that runs on all ten targets.

## Fine-grained reactivity with a native runtime profile

Day builds the widget tree once and binds signals to it. A state change doesn't re-run your view
functions or diff a virtual tree; it updates the native attributes that depend on the changed
`Signal`, and nothing else. You write declaratively; it runs like hand-written native code.

```rust
let volume = Signal::new(40.0);
row((
    slider(volume).range(0.0..=100.0),
    // Only this label re-reads `volume`; nothing else in the tree is touched.
    label(move || format!("{:.0}", volume.get())),
))
```

## Four pillars, built in

Four things Day builds into the framework itself:

- **Fluent localization.** Text is localized through Mozilla's Fluent (`tr("key")`), with
  arguments and per-locale plurals. Switching locale re-binds affected labels live.
- **Accessibility.** Every Piece carries an accessibility role/label surfaced to the platform's
  assistive technology. Validation compares native trees against a reference so regressions are
  caught in CI.
- **dayscript.** A YAML automation/testing language drives and asserts a running app over a
  socket: tap a button by id, assert a route, capture a screenshot. The same script runs on every
  platform.
- **dayffi.** Day Pieces can be authored and shipped as packages, including across a small, stable
  C ABI, so a native component (a combo box, a chart, a web view) plugs in like a built-in.

## Small and toolable

- **One backend per binary.** The compiler monomorphizes to the chosen toolkit, so there's no
  runtime toolkit indirection.
- **A `flutter_tools`-style CLI.** `day new / build / launch / pack / lint`, built for humans,
  CI, IDEs, and AI agents.
- **Screenshot-validated CI.** Every target builds the showcase and captures screenshots on each
  push. The [gallery](/gallery) on this site is assembled from those artifacts.

Next up: the [API tour](/docs/api-tour).
