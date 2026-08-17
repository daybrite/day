---
title: "Baseline alignment"
description: "How text in a row sits on a shared baseline across toolkits, and the layout rules that keep mixed fonts aligned."
---

<!-- Copyright © The Daybrite Project
     SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Baseline alignment

Text in a row sits on its **baseline** — the line the letters stand on — rather than in the
middle of whatever box happens to contain it.

The two are the same thing only when every child puts its text at the same height inside its own
box. Real controls do not: a bordered text field insets its text by the border, a date picker
with a stepper is taller than the text it shows, and a Title-size number has a much taller ascent
than the Caption unit beside it. Center those boxes and the text lands on three different
invisible lines, a few points apart — close enough to look accidental rather than designed.

```rust
// Baseline-aligned by default: the label meets the field's inset text.
labeled("Quantity", text_field(qty))

// The explicit opt-in for an ordinary row.
row((label("Total"), label("$1,240.00").font(Font::Title2), label("USD").font(Font::Caption)))
    .align(VAlign::FirstBaseline)
```

## What day asks the toolkit

One duty, on `Toolkit`:

```rust
fn first_baseline(&mut self, h: &Self::Handle, kind: PieceKind, size: Size) -> Option<f64>;
```

The distance from the top of the widget's frame to its first text baseline, in points, when the
widget is `size` tall. `None` means the widget has no text baseline — an image, a slider, a bare
container — and a baseline-aligned row falls back to centering that child.

It is a **measurement, not a layout mode**. Day computes every frame itself and never hands a row
to a native baseline-aligning container, so what it needs from the toolkit is where the text sits
inside the box, not the ability to align. That distinction is what makes the support broad: a
toolkit with no baseline-aligning container at all can still report a font ascent.

The size matters. A control that centers its text vertically moves its baseline with its height,
so the answer is only meaningful at the height the row settled on — which is why the size is a
parameter rather than something the backend reads off the current frame.

## Per-toolkit

| toolkit | source | `Cap::BaselineAlignment` |
|---|---|---|
| AppKit | the control's font, centered in its `alignmentRectInsets` box (see below) | Native |
| GTK | `gtk_widget_measure`'s natural-baseline out-param | Native |
| Android | `View.getBaseline()` | Native |
| UIKit | derived: the view's font ascent, centered in its height | Emulated |
| Qt | derived: `QFontMetricsF::ascent()`, centered in its height | Emulated |
| web-dom | derived: canvas `TextMetrics.fontBoundingBoxAscent`, offset by the element's border and padding | Emulated |
| XAML | `TextBlock.BaselineOffset`; font-derived for templated controls | Emulated |
| ArkUI | derived from `NODE_FONT_SIZE` | Emulated |

`Native` means the platform reports the baseline itself; `Emulated` means day derives it from the
widget's font. Both align correctly — the distinction is whether a control with unusual internal
text placement can be wrong. `Unsupported` would mean rows fall back to centering, which is what
every row did before this existed; nothing breaks, it just does not align.

**Why AppKit derives rather than asks.** `firstBaselineOffsetFromTop` looks like the exact
answer, and it is nearly right, but it has two problems. It is measured from the top of the view's
*alignment rect* rather than its frame — a bezelled `NSDatePicker` insets its alignment rect 4pt —
and it is **rounded to whole points**. A picker in a 26pt frame answers 15 (19 in frame terms)
while it paints at 19.9, so a label beside it sits a point high: exactly the drift baseline
alignment exists to remove.

So the AppKit backend derives the baseline from the metrics AppKit rounded — one line of the
control's own font, centered in its alignment rect, baseline an ascender below the line's top. That
reproduces AppKit's own numbers where they are right (a plain label: 12.91 against its 13) and
keeps the fractional part where they are not. A `TEXT_AREA` keeps the reported value, since its
first line sits at the top of its text container rather than centered in it, and a control with no
font of its own falls back to the report.

## Containers and decorators

A container answers on behalf of its content, through `Layout::baseline`:

- a **row** reports the line its children were aligned to;
- a **column** reports its first baseline-bearing child's, offset by where that child sits;
- a **`labeled` row** reports the line its label and control now share, so a form row nested in
  another baseline-aligned row joins the same line;
- every **single-child wrapper** — `.width()`, `.frame()`, `.padding()`, `.grow()`,
  `.max_width()`, `.background()` — forwards its child's, offset by where it places it.

That last one matters more than it looks. A decorator is invisible at the call site:
`label("Qty").width(90.0)` still reads as "a label". If wrappers reported no baseline, a row
would silently center the very children the author asked to align, and the feature would look
like it simply did not work. The `decorated_children_keep_their_baseline` test in
`day-pieces/tests/mock_e2e.rs` pins it.

Grid cells are **not** baseline-aligned; `grid_row(..).align(VAlign::FirstBaseline)` reads as
`Center`. See [docs/grid.md](grid.md).

## Row height

A baseline-aligned row is as tall as the deepest baseline plus the deepest descent below one —
not simply as tall as its tallest child. Shifting a child down to meet the line can push it past
where the tallest child ends, and a row measured at the tallest child's height would clip it.

## Cost

`first_baseline` is asked once per child per measure generation, cached beside the measure cache
on the node and invalidated with it. A row that is not baseline-aligned never asks at all.

## Trying it

Day-Showcase's **Text** page ends with a *Baseline alignment* section: three rows mixing type
sizes and control kinds, and a toggle that turns the alignment off. On AppKit the aligned rows
measure a 0px spread between the label, the field's text and the trailing unit; centered, they
drift 3–4px apart.
