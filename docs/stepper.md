---
navTitle: "Steppers"
title: "Stepper field"
description: "A numeric field with increment/decrement arrows in two idioms (the platform's widget, and one Day composes), bound two-way to any Binding<f64>."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Stepper field

Let people adjust a number in steps, such as a quantity, zoom level, or dimension.
`day-piece-stepper` binds the value to app state and uses a native numeric control where
available, with a composed control on other targets.

## Add to your app

Defined in [`day-piece-stepper`](https://github.com/daybrite/day/tree/main/pieces/day-piece-stepper). Add this to your app’s `Cargo.toml`
`[dependencies]` table, using the same Day Git source as the rest of your app:

```toml
[dependencies]
day-piece-stepper = { git = "https://github.com/daybrite/day.git" }
```

`day build -t <target>` enables this direct dependency’s matching toolkit feature
automatically. No dependency entry is needed in `Day.toml`.

| Platforms / toolkits | Availability |
| --- | --- |
| AppKit, GTK, Qt | Native numeric stepper. |
| UIKit, Android, WinUI, ArkUI, Web | Composed control. Forcing [`.native()`](https://daybrite.dev/api/day_piece_stepper/struct.Stepper.html#method.native) on these targets shows a placeholder. |

[`stepper(value)`](https://daybrite.dev/api/day_piece_stepper/fn.stepper.html) is a numeric **stepper field**: a text field with increment/decrement arrows,
bound two-way to any `Binding<f64>` (a `Signal<f64>`, a [day-model](/docs/explanation/model) `Field`, or an app's own
binding over a selection). Each arrow click, and each settled edit (Return, focus loss), lands
through `write_commit`, so under an undo stack every click is one undoable step.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](/docs/how-to/localization).

## Authoring

```rust
use day_piece_stepper::stepper;

let width = Signal::new(1.0f64);

labeled(
    res::str::stroke_width(),
    stepper(width)
        .range(0.0..=64.0) // typed and stepped values both clamp (default 0..=100)
        .step(1.0) // one arrow click (default 1)
        .decimals(0) // fraction digits shown (default 0)
        .id(id!(insp_stroke_w)), // the field's dayscript id, both idioms
)
```

- **[`.native()`](https://daybrite.dev/api/day_piece_stepper/struct.Stepper.html#method.native) / [`.composed()`](https://daybrite.dev/api/day_piece_stepper/struct.Stepper.html#method.composed) / [`.idiom(…)`](https://daybrite.dev/api/day_piece_stepper/struct.Stepper.html#method.idiom)** pin one idiom; the default is `Automatic`:
  native where an arm exists ([`support()`](https://daybrite.dev/api/day_piece_stepper/fn.support.html) answers `Native` on appkit, gtk, qt), composed
  everywhere else. `Native` is literal: pinned on a toolkit with no renderer it draws Day's
  visible placeholder.
- **[`.id(…)`](https://daybrite.dev/api/day_piece_stepper/struct.Stepper.html#method.id)** names the interactive field for dayscript on both idioms, wherever it sits in the
  chain.

## Driving and asserting from dayscript

The native leaf accepts `input:` (typed text, parsed and clamped), `set_value:`
(`ValueChanged` previews, `ValueCommitted` commits), and mirrors its state into the probe
through day-core's `set_probe_value` hook, so `assert_text` sees the display form
([`fmt_value`](https://daybrite.dev/api/day_piece_stepper/fn.fmt_value.html)'s fixed-decimal text) and `assert_value` the number, exactly like a built-in
control. The composed idiom is a real text field and needs nothing special. One script drives
both idioms by the same key.

One scripted input commits once. The native event handler accepts `stepper:value` (and the
legacy bridge's empty tag), but ignores unrelated custom events, including the
`day:synthesized-text` paint notification that precedes `TextChanged`. Treating both as
edits would create duplicate undo entries. `set_value:` still delivers one preview followed
by one commit, so a binding can retain the value from before the gesture for undo.

> [!NOTE]
> **Windows and Web / DOM: composed control.**
> These targets use the composed stepper rather than a native numeric input.
> It is built from ordinary Day pieces.

## Platform guidance

The AppKit renderer combines [`NSTextField`](https://developer.apple.com/documentation/appkit/nstextfield) and [`NSStepper`](https://developer.apple.com/documentation/appkit/nsstepper); GTK uses [`GtkSpinButton`](https://docs.gtk.org/gtk4/class.SpinButton.html), and
Qt uses [`QDoubleSpinBox`](https://doc.qt.io/qt-6/qdoublespinbox.html). Other targets compose the control from ordinary Day pieces.

- [Apple: steppers](https://developer.apple.com/design/human-interface-guidelines/steppers)
- [Windows: number boxes](https://learn.microsoft.com/en-us/windows/apps/design/controls/number-box)
