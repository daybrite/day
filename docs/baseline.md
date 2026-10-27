---
navTitle: "Baseline"
title: "Baseline alignment"
description: "How text in a row sits on a shared baseline across toolkits, and the layout rules that keep mixed fonts aligned."
---

<!-- Copyright © The Daybrite Project
     SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Baseline alignment

Text in a row sits on its **baseline** (the line the letters stand on) rather than in the
middle of whatever box happens to contain it.

Baseline alignment keeps labels and controls aligned when their font sizes or internal
padding differ. Use it for a value beside a unit, or a label beside an input.

```rust
// Baseline-aligned by default: the label meets the field's inset text.
labeled(res::str::quantity(), text_field(qty))

// The explicit opt-in for an ordinary row.
row((label(res::str::total()), label(res::str::sample_total()).font(Font::Title2), label(res::str::usd()).font(Font::Caption)))
    .align(VAlign::FirstBaseline)
```

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Alignment behavior

Day uses each child's first text baseline. A child without a text baseline, such as an image
or slider, is centered vertically instead. The measurement accounts for the child's allocated
height, including controls that center their text within a larger frame.

## Toolkit implementations

| toolkit | source | `Cap::BaselineAlignment` |
|---|---|---|
| AppKit | the control's font, centered in its `alignmentRectInsets` box  | Native |
| GTK | `gtk_widget_measure`'s natural-baseline out-param | Native |
| Android | `View.getBaseline()` | Native |
| UIKit | derived: the view's font ascent, centered in its height | Emulated |
| Qt | derived: `QFontMetricsF::ascent()`, centered in its height | Emulated |
| Web | derived: canvas `TextMetrics.fontBoundingBoxAscent`, offset by the element's border and padding | Emulated |
| WinUI | `TextBlock.BaselineOffset`; font-derived for templated controls | Emulated |
| ArkUI | derived from `NODE_FONT_SIZE` | Emulated |

`Native` and `Emulated` describe how the toolkit obtains the measurement. Derived baselines
can be less accurate for custom controls with unusual internal text placement. Unsupported
controls fall back to vertical centering.

## Containers and decorators

A container answers on behalf of its content, through [`Layout::baseline`](https://docs.rs/day-core/latest/day_core/trait.Layout.html#method.baseline):

- a **row** reports the line its children were aligned to;
- a **column** reports its first baseline-bearing child's, offset by where that child sits;
- a **[`labeled`](https://docs.rs/day-pieces/latest/day_pieces/fn.labeled.html) row** reports the line its label and control now share, so a form row nested in
  another baseline-aligned row joins the same line;
- every **single-child wrapper** ([`.width()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.width), [`.frame()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.frame), [`.padding()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding), [`.grow()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.grow),
  [`.max_width()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.max_width), [`.min_width()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.min_width), [`.background()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.background)) forwards its child's, offset by where it places it.

Grid cells are **not** baseline-aligned; [`grid_row`](https://docs.rs/day-pieces/latest/day_pieces/fn.grid_row.html)`(..)`[`.align`](https://docs.rs/day-pieces/latest/day_pieces/trait.GridRowBuilder.html#tymethod.align)`(VAlign::FirstBaseline)` reads as
`Center`. See [docs/grid](grid.md).

## Row height

A baseline-aligned row is as tall as the deepest baseline plus the deepest descent below one,
which can exceed the height of its tallest child. Shifting a child down to meet the line can push
it past where the tallest child ends, and a row measured at the tallest child's height would
clip it.
