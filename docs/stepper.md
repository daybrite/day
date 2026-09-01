---
title: "Stepper field"
description: "A numeric field with increment/decrement arrows in two idioms (the platform's own widget, and one Day composes), bound two-way to any Binding<f64>."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Stepper field (external piece)

> **Status: implemented** as `day-piece-stepper`. Three toolkits get a native renderer
> registered link-time into their slice: an `NSTextField` + `NSStepper` composite on AppKit
> (macOS has no combined control; the pair is the platform idiom), `GtkSpinButton`, and a
> `QDoubleSpinBox` shim. Every other target gets the same composed row, built from
> ordinary Day pieces. Verified on macos-appkit, macos-gtk, macos-qt, ios-uikit, android-mdc
> and web-dom through Day-Sketch's inspector walkthrough.

`stepper(value)` is a numeric **stepper field**: a text field with increment/decrement arrows,
bound two-way to any `Binding<f64>` (a `Signal<f64>`, a day-model `Field`, or an app's own
binding over a selection). Each arrow click, and each settled edit (Return, focus loss), lands
through `write_commit`, so under an undo stack every click is one undoable step.

## Authoring

```rust
use day_piece_stepper::stepper;

let width = Signal::new(1.0f64);

labeled(
    tr("stroke_width"),
    stepper(width)
        .range(0.0..=64.0)     // typed and stepped values both clamp (default 0..=100)
        .step(1.0)             // one arrow click (default 1)
        .decimals(0)           // fraction digits shown (default 0)
        .key("insp-stroke-w"), // the field's dayscript id, both idioms
)
```

- **`.native()` / `.composed()` / `.idiom(…)`** pin one idiom; the default is `Automatic`:
  native where an arm exists (`support()` answers `Native` on appkit, gtk, qt), composed
  everywhere else. `Native` is literal: pinned on a toolkit with no renderer it draws Day's
  visible placeholder.
- **`.key(…)`** names the field for dayscript on both idioms (the composed row's wrapper is a
  layout node no toolkit realizes, so `Decorate::id` on the piece would tag nothing).

## Driving and asserting from dayscript

The native leaf accepts `input:` (typed text, parsed and clamped), `set_value:`
(`ValueChanged` previews, `ValueCommitted` commits), and mirrors its state into the probe
through day-core's `set_probe_value` hook, so `assert_text` sees the display form
(`fmt_value`'s fixed-decimal text) and `assert_value` the number, exactly like a built-in
control. The composed idiom is a real text field and needs nothing special. One script drives
both idioms by the same key.

## Verification status

The AppKit, GTK and Qt arms plus the composed row are exercised by Day-Sketch's walkthrough
(the stroke-width row of its inspector). There is no XAML or web arm yet. Both platforms have
native spin controls (a WinUI `NumberBox`, `<input type="number">`), and arms for them are
welcome, but the composed row already behaves correctly there, so they are an upgrade rather
than a gap.
