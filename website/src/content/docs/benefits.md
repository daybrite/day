---
title: Why Day
description: The benefits — native fidelity, one codebase, fine-grained reactivity, and four pillars built in.
order: 2
---

Day exists to remove a false choice: *native fidelity* **or** *a single codebase*. You get both.

## Genuinely native, not native-ish

Your Pieces become real platform widgets, so you inherit — for free, and forever — everything the
platform does better than any cross-platform renderer can imitate:

- system text rendering, input methods, and spellcheck;
- native scrolling physics, selection, and drag;
- the real accessibility tree (VoiceOver, TalkBack, Narrator, Orca) — not an approximation;
- platform theming, dark mode, dynamic type, and right-to-left;
- OS updates that improve your app without you shipping anything.

A Day app on macOS uses AppKit; on Android it uses `android.widget`; on Linux it adopts
libadwaita. It looks like it belongs, because it does.

## One codebase, one language

UI, state, layout, localization, and tests are all Rust. No FFI seams between your view layer and
your logic, no template DSL to context-switch into, no per-platform UI forks to keep in sync. The
[showcase app](../gallery) is a single Rust program that runs on all ten targets.

## Fine-grained reactivity with a native runtime profile

Because Day **builds once and binds forever**, a state change doesn't re-run your view functions or
diff a virtual tree — it updates precisely the native attributes that depend on the changed
`Signal`. You write declaratively; it runs like hand-tuned native code.

```rust
let volume = Signal::new(40.0);
row((
    slider(volume).range(0.0..=100.0),
    // Only this label re-reads `volume`; nothing else in the tree is touched.
    label(move || format!("{:.0}", volume.get())),
))
```

## Four pillars, built in

Day treats these as first-class framework concerns, not add-ons:

- **Fluent localization.** Text is localized through Mozilla's Fluent (`tr("key")`), with
  arguments and per-locale plurals. Switching locale re-binds affected labels live.
- **Accessibility.** Every Piece carries an accessibility role/label surfaced to the real platform
  AT. Validation compares native trees against a reference so regressions are caught in CI.
- **dayscript.** A YAML automation/testing language drives and asserts a running app over a socket
  — tap a button by id, assert a route, capture a screenshot — the same script on every platform.
- **dayffi.** Day Pieces can be authored and shipped as packages, including across a small, stable
  C ABI, so a native component (a combo box, a chart, a web view) plugs in like a built-in.

## Small, direct, and toolable

- **One backend per binary** — no runtime toolkit indirection; the compiler monomorphizes to the
  chosen toolkit.
- **A `flutter_tools`-style CLI** — `day new / build / launch / pack / lint`, designed from Day
  one for humans, CI, IDEs, and AI agents.
- **Screenshot-validated CI** — every target builds the showcase and captures screenshots on each
  push; the [gallery](../gallery) on this site is assembled from exactly those artifacts.

Ready to see the API? Continue to the [API tour](./api-tour).
