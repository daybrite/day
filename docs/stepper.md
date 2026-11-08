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
  visible placeholder. A custom [`.format()`](https://docs.rs/day-pieces/latest/day_pieces/stepper/struct.Stepper.html#method.format) uses the composed control regardless of this setting.
- **[`.id(…)`](https://docs.rs/day-pieces/latest/day_pieces/stepper/struct.Stepper.html#method.id)** names the interactive field for dayscript on both idioms, wherever it sits in the
  chain.

[`StepperBuilder`](https://docs.rs/day-pieces/latest/day_pieces/stepper/trait.StepperBuilder.html) keeps
[`.range(...)`](https://docs.rs/day-pieces/latest/day_pieces/stepper/trait.StepperBuilder.html#tymethod.range),
[`.step(...)`](https://docs.rs/day-pieces/latest/day_pieces/stepper/trait.StepperBuilder.html#tymethod.step), formatting, and idiom selection available after layout modifiers
such as [`.padding(...)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding). Use [`number_field`](textfield.md#numeric-input) for the same numeric
editing behavior without step buttons.

## Formatted values

Use [`.format(NumberFormat::percent())`](https://docs.rs/day-pieces/latest/day_pieces/stepper/struct.Stepper.html#method.format)
to display and edit percentages while storing a fraction:

```rust
let value = Signal::new(0.4);
stepper(value)
    .range(0.0..=1.0)
    .step(0.01)
    .format(NumberFormat::percent())
```

[`NumberFormat::decimal()`](https://docs.rs/day-localization/latest/day_localization/struct.NumberFormat.html#method.decimal) defaults to two fraction digits; [`NumberFormat::percent()`](https://docs.rs/day-localization/latest/day_localization/struct.NumberFormat.html#method.percent) defaults
to none. Set a different precision with [`.fraction_digits(1)`](https://docs.rs/day-localization/latest/day_localization/struct.NumberFormat.html#method.fraction_digits). Formatting and parsing use the
current locale's separators and digits. ASCII digits are accepted too. For percentages, `40`
and `40%` both represent `0.4`; in German, `40,5 %` represents `0.405`.

Return or focus loss commits the edit and restores formatted text. While typing, unfinished
input leaves the number unchanged. Invalid input, non-finite numbers, and malformed grouping
are rejected; valid numbers clamp to the configured range. A locale or bound-value change
refreshes the displayed text, including during editing. Arrow clicks and DayScript `set_value`
use the stored units (`0.01` is one percentage point here).

Formatted steppers use Day's composed −/field/+ control on every toolkit, overriding
[`.native()`](https://docs.rs/day-pieces/latest/day_pieces/stepper/struct.Stepper.html#method.native), [`.idiom()`](https://docs.rs/day-pieces/latest/day_pieces/stepper/struct.Stepper.html#method.idiom), and [`.decimals()`](https://docs.rs/day-pieces/latest/day_pieces/stepper/struct.Stepper.html#method.decimals). The field and buttons remain native controls.

For application-specific notation, [`.format_with(display, parse)`](https://docs.rs/day-pieces/latest/day_pieces/stepper/struct.Stepper.html#method.format_with)
accepts a display function returning a string or generated Fluent message and a parser returning
`Option<f64>`. Earlier code passed these callbacks to the format method; rename those calls to format_with.
Keep units and other application-owned text in the Fluent catalog.

Use [`number_field(value)`](/docs/reference/textfield#numeric-input) for the same locale-aware
editing without step buttons. See [numeric formats](/docs/reference/localization#editable-numbers)
for the accepted notation and explicit-locale methods.

## Platform guidance

The AppKit renderer combines [`NSTextField`](https://developer.apple.com/documentation/appkit/nstextfield) and [`NSStepper`](https://developer.apple.com/documentation/appkit/nsstepper); GTK uses [`GtkSpinButton`](https://docs.gtk.org/gtk4/class.SpinButton.html), and
Qt uses [`QDoubleSpinBox`](https://doc.qt.io/qt-6/qdoublespinbox.html). Other targets compose the control from ordinary Day pieces.

- [Apple: steppers](https://developer.apple.com/design/human-interface-guidelines/steppers)
- [Windows: number boxes](https://learn.microsoft.com/en-us/windows/apps/design/controls/number-box)
