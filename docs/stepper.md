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

A stepper adjusts a value in increments so people can repeat small changes. It suits quantities
such as an item count or a dimension when each increase or decrease has a predictable size.

`day-pieces` binds the value to app state and uses a native numeric control where available, with a
composed control on other targets.

## Add to your app

This control is included in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces)
and re-exported through `day::prelude::*`. No additional crate or control feature is required.
The selected toolkit supplies its native implementation; [`day::stepper::support()`](https://docs.rs/day-pieces/latest/day_pieces/stepper/fn.support.html) reports
its availability after the Day runtime starts.

| Platforms / toolkits | Availability |
| --- | --- |
| AppKit, GTK, Qt | Native numeric stepper. |
| UIKit, Android, WinUI, ArkUI, Web | Composed control. Forcing [`.native()`](https://docs.rs/day-pieces/latest/day_pieces/stepper/struct.Stepper.html#method.native) on these targets shows a placeholder. |

[`stepper(value)`](https://docs.rs/day-pieces/latest/day_pieces/stepper/fn.stepper.html) is a numeric **stepper field**: a text field with increment/decrement arrows,
bound two-way to any `Binding<f64>` (a `Signal<f64>`, a [day-model](/docs/explanation/model) `Field`, or an app's own
binding over a selection). Each arrow click, and each settled edit (Return, focus loss), lands
through `write_commit`, so under an undo stack every click is one undoable step.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Authoring

```rust
use day::stepper::stepper;

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

- **[`.native()`](https://docs.rs/day-pieces/latest/day_pieces/stepper/struct.Stepper.html#method.native) / [`.composed()`](https://docs.rs/day-pieces/latest/day_pieces/stepper/struct.Stepper.html#method.composed) / [`.idiom(…)`](https://docs.rs/day-pieces/latest/day_pieces/stepper/struct.Stepper.html#method.idiom)** pin one idiom; the default is `Automatic`:
  native where an arm exists ([`support()`](https://docs.rs/day-pieces/latest/day_pieces/stepper/fn.support.html) answers `Native` on appkit, gtk, qt), composed
  everywhere else. `Native` is literal: pinned on a toolkit with no renderer it draws Day's
  visible placeholder.
- **[`.id(…)`](https://docs.rs/day-pieces/latest/day_pieces/stepper/struct.Stepper.html#method.id)** names the interactive field for dayscript on both idioms, wherever it sits in the
  chain.

## Platform guidance

The AppKit renderer combines [`NSTextField`](https://developer.apple.com/documentation/appkit/nstextfield) and [`NSStepper`](https://developer.apple.com/documentation/appkit/nsstepper); GTK uses [`GtkSpinButton`](https://docs.gtk.org/gtk4/class.SpinButton.html), and
Qt uses [`QDoubleSpinBox`](https://doc.qt.io/qt-6/qdoublespinbox.html). Other targets compose the control from ordinary Day pieces.

- [Apple: steppers](https://developer.apple.com/design/human-interface-guidelines/steppers)
- [Windows: number boxes](https://learn.microsoft.com/en-us/windows/apps/design/controls/number-box)
