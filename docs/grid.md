---
title: "Grid"
description: "The grid piece's design and implementation: fixed and adaptive tracks, spans, and native measurement."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Grid

Arrange content in rows with shared column widths, without assigning a fixed width to every
cell. Use `grid` and `grid_row` for tables of values, forecasts, or aligned controls; cells can
span columns and choose their alignment.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](/docs/how-to/localization).

## 1. Goal

A grid replaces the hand-tuned-width idiom. Before grids, a table-shaped layout was a `column` of `row`s with
`.width(104.0)`-style constants keeping columns aligned across rows, and `spacer().width(40.0)`
placeholders holding empty cells open. A grid derives the columns from the cells:

```rust
grid((
    grid_row((
        label(res::str::mon()),
        icon,
        label(res::str::sample_probability()),
        label(res::str::sample_low()),
        bar.grow_w(),
        label(res::str::sample_high()),
    )),
    grid_row((
        label(res::str::tue()),
        icon,
        spacer(),
        label(res::str::sample_low_tuesday()),
        bar.grow_w(),
        label(res::str::sample_high_tuesday()),
    )),
))
.column_spacing(8.0)
```

Each column is as wide as its widest cell; the `grow_w` bar column takes whatever is left; the
`spacer()` is an inert empty cell that still occupies its column. The rows need no width
constants or placeholders.

## 2. API

```rust
pub fn grid<C: PieceSeq>(children: C) -> Grid<C>;
impl Grid {
    pub fn spacing(self, s: f64) -> Self;          // both gutters
    pub fn row_spacing(self, s: f64) -> Self;
    pub fn column_spacing(self, s: f64) -> Self;
    pub fn align(self, a: Alignment) -> Self;      // default cell alignment (9-way)
}

pub fn grid_row<C: PieceSeq>(children: C) -> GridRow<C>;
impl GridRow {
    pub fn align(self, a: VAlign) -> Self;         // per-row vertical override
}

// Decorate (any piece; inert outside a grid):
fn grid_span(self, n: usize) -> Decorated<Self>;     // span n ≥ 1 columns
fn grid_align(self, a: Alignment) -> Decorated<Self>;// per-cell alignment override
```

SwiftUI mapping: `grid` ↔ `Grid(alignment:horizontalSpacing:verticalSpacing:)`, `grid_row` ↔
`GridRow(alignment:)`, `.grid_span` ↔ `gridCellColumns(_:)`, `.grid_align` ↔ a 9-way
`gridCellAnchor(_:)`. Alignment precedence per axis: cell `.grid_align` > row `.align`
(vertical only) > the grid's `.align`.

Semantics:

- **Columns infer from rows.** A row's cells occupy columns left to right (spans advance the
  cursor); the grid's column count is the max over rows.
- **Column width = the max ideal width of its span-1 cells.** A `grow_w` cell makes its column
  **flexible**: rigid columns keep their ideals and the flexible ones split the leftover width
  evenly (the `StackLayout` share rule). Unconstrained (e.g. inside `scroll(..).horizontal()`),
  a flexible column falls back to its own ideal.
- **A spanning cell** (`.grid_span(n)`) is measured once; if it needs more than its spanned
  columns provide, the deficit distributes in one shot: onto the spanned flexible columns if
  any, else evenly across the spanned columns.
- **A bare (non-row) child is a full-width cell** spanning every column, SwiftUI's rule.
  Dividers and section-spanning cards need no modifier.
- **`spacer()` in a row is an inert empty cell**: it occupies its column, contributes no width,
  and is never placed. This diverges from stacks (where a spacer pushes content apart)
  because a grid has explicit gutters, so push-apart spacers have no meaning here.
- **`when`/`each` groups expand inline** at both levels: at grid level they produce rows, inside
  a row they produce cells (the `StackLayout::flatten` recursion), so reactive row sets reflow
  and renegotiate columns for free.
- **Row heights** are the max cell height *at the final column widths* (text height-for-width is
  correct); a `grow_h` cell makes its row stretch under a height proposal. Nested grids are
  ordinary cells.
- **RTL mirrors columns** transparently: geometry is computed LTR and every placement mirrors
  around the grid's width in the engine's `place_child`, like every other layout.

## 6. Restrictions

- Don't decorate a `grid_row`: rows are transparent carriers with no frame; `.background` etc.
  on a row is unsupported (decorate the cells or the grid). Consequently dayscript `node_frame`
  on a row id returns nothing. (A row-band placement variant was considered and rejected for the
  hidden geometry coupling it adds; revisit if row striping becomes a real need.)
- Grid modifiers must be outermost; see the [facts channel](backend-contracts/grid.md#3-the-facts-channel).
- Rigid content sharing a flexible column can clip when the leftover is small, the same property
  stacks have; a per-column minimum-width floor is a possible v2.
- Cell gestures/ids work as usual (cells are ordinary pieces); rows have no identity.


## Related contracts

See the [layout contract](backend-contracts/grid.md) for measurement and placement.
