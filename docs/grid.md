---
navTitle: "Grids"
title: "Grid"
description: "Arrange cells in shared columns, control spacing and alignment, and span content across columns."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Grid

Arrange content in rows with shared column widths. Use a grid for aligned controls, summaries,
and small tables of values. Columns size to their contents, so labels can change without
maintaining matching width constants in every row.

## Add to your app

This layout is included in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces)
and re-exported through `day::prelude::*`. No additional crate or layout feature is required.

A grid builds every cell. Use a [list](/docs/reference/list) for large collections that need
row recycling, and [rows or columns](/docs/explanation/layout) when items do not need to align
across several rows.

## Authoring

This module accepts a name and location supplied by the caller. The surrounding labels come
from the app's Fluent catalog. Add the messages below to an app with generated
[resources](/docs/reference/resources).

<!-- technical-example: grid_recipe -->
```rust
use crate::res;
use day::prelude::*;

pub fn details(name: Signal<String>, location: Signal<String>) -> impl Piece {
    grid((
        label(res::str::grid_details()),
        grid_row((label(res::str::grid_name()), label(name))),
        grid_row((label(res::str::grid_location()), label(location).grow_w())),
        grid_row((label(res::str::grid_note()).grid_span(2).padding(4.0),)),
    ))
    .column_spacing(16.0)
    .row_spacing(8.0)
    .align(Alignment::TopLeading)
}
```

<!-- technical-catalog: grid_recipe -->
```ftl
grid_details = Details
grid_name = Name
grid_location = Location
grid_note = Details for the selected item.
```

Each row contributes two columns. The location value uses
[`.grow_w()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.grow_w), making the
second column share the available width. The first label is a direct child of the grid, so
it occupies a full-width row. The note spans both columns within a row.

## Spacing and alignment

| Builder | Default and behavior |
| --- | --- |
| [`spacing(value)`](https://docs.rs/day-pieces/latest/day_pieces/trait.GridBuilder.html#tymethod.spacing) | Sets both row and column spacing; both default to zero. Accepts points or a `Space` value. |
| [`row_spacing(value)`](https://docs.rs/day-pieces/latest/day_pieces/trait.GridBuilder.html#tymethod.row_spacing) | Sets the gap between rows. |
| [`column_spacing(value)`](https://docs.rs/day-pieces/latest/day_pieces/trait.GridBuilder.html#tymethod.column_spacing) | Sets the gap between columns. |
| [`Grid::align(value)`](https://docs.rs/day-pieces/latest/day_pieces/trait.GridBuilder.html#tymethod.align) | Aligns cells within their allocated space; defaults to `Alignment::Center`. |
| [`GridRow::align(value)`](https://docs.rs/day-pieces/latest/day_pieces/trait.GridRowBuilder.html#tymethod.align) | Overrides vertical alignment for that row; otherwise inherits the grid's alignment. |
| [`.grid_align(value)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Decorated.html#method.grid_align) | Overrides both alignment axes for one cell. |

A cell's alignment takes precedence over its row's vertical alignment and the grid's default.
Leading and trailing placement follows the app's layout direction; columns mirror in
right-to-left layouts.

## Column widths and row heights

Cells occupy columns in declaration order. The row with the most occupied columns determines
the grid's column count. Without growth modifiers, each column takes the largest ideal width
of its single-column cells; spanning cells can require additional width.

A cell with [`.grow_w()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.grow_w)
makes its column flexible. Under a bounded width, rigid columns retain their ideal widths and
flexible columns split the remaining space after column gaps. When width is unconstrained,
such as inside a horizontal [scroll view](/docs/reference/scroll), flexible columns use their
ideal widths instead.

Row heights account for the cells' heights at their final column widths, including wrapped
text. A [`.grow_h()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.grow_h) cell
lets its row share extra height when the grid receives a bounded height.

Rigid content in a flexible column can clip when little width remains. Allow long text to
wrap or scroll the grid when its contents cannot fit. Test narrow windows and longer translations.

## Spanning and empty cells

[`.grid_span(n)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Decorated.html#method.grid_span)
makes a cell occupy consecutive columns and advances the next cell accordingly. The default
span is one; zero is treated as one. Row spanning is not supported. A direct child that is
not a row occupies a full-width row without needing a span modifier.

A [`spacer()`](https://docs.rs/day-pieces/latest/day_pieces/fn.spacer.html) inside a grid row reserves one
empty column position. It contributes no width and does not push adjacent cells apart.
Use column spacing for gaps between cells.

Grid span and alignment apply to the cell's outermost layout node, including padding or frame
wrappers. Within a modifier chain, they do not have to be the last modifiers. For example,
the note above still spans two columns even though padding follows its span modifier.

## Reactive content and lifetime

Signal-backed labels and controls update normally. [`each()`](https://docs.rs/day-pieces/latest/day_pieces/fn.each.html)
and [`when()`](https://docs.rs/day-pieces/latest/day_pieces/fn.when.html) expand into rows at grid level or
into cells within a row. Changes to content or the row set can resize columns across the grid.
Nested grids behave as ordinary cells.

The grid is eager: placing it inside a scroll view does not defer creating off-screen cells.
For large datasets, use the collection and lifetime guidance in [Lists](/docs/reference/list).

## Toolkit implementations

Day computes the shared column widths and cell positions, while each cell uses its selected
toolkit's control. There is no separate platform table control or automatic native row
selection. Font metrics and control sizes can produce different measurements across toolkits.

Grid rows are layout-only carriers, not rendered controls. Apply backgrounds, padding,
gestures, accessibility labels, and element IDs to the cells or the grid, rather than to a
`grid_row`. The row's vertical alignment builder is supported.

Use [accessibility](/docs/reference/accessibility) to describe the controls and values, and
[element IDs](/docs/reference/element-ids) to address individual cells in walkthroughs.
