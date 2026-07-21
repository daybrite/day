# Grid: design & implementation

> **Status: implemented.** `grid()`/`grid_row()` (`day_pieces`), the `.grid_span`/`.grid_align`
> cell modifiers, and `GridLayout` (`day_core`) ship with zero backend work — a grid is the same
> dumb native panel as `column`/`row`, laid out entirely by shared Rust (§7 of DESIGN.md).
> Demonstrated by the showcase "Grid" section (basics → sizing → spanning → composite → stress)
> and consumed by Day Skies' 10-day forecast and detail cards. This is the SwiftUI `Grid`/
> `GridRow` analogue for Day.

## 1. Goal

Kill the hand-tuned-width idiom. Before grids, a table-shaped layout was a `column` of `row`s with
`.width(104.0)`-style constants keeping columns aligned across rows — and `spacer().width(40.0)`
placeholders holding empty cells open. A grid derives the columns from the cells:

```rust
grid((
    grid_row((label("Mon"), icon, label("40%"), label("9°"),  bar.grow_w(), label("15°"))),
    grid_row((label("Tue"), icon, spacer(),     label("12°"), bar.grow_w(), label("22°"))),
))
.column_spacing(8.0)
```

Each column is as wide as its widest cell; the `grow_w` bar column takes whatever is left; the
`spacer()` is an inert empty cell that still occupies its column. No constants, no placeholders.

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
fn grid_span(self, n: usize) -> AnyPiece;          // span n ≥ 1 columns
fn grid_align(self, a: Alignment) -> AnyPiece;     // per-cell alignment override
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
  columns provide, the deficit distributes in one shot — onto the spanned flexible columns if
  any, else evenly across the spanned columns.
- **A bare (non-row) child is a full-width cell** spanning every column — SwiftUI's exact rule.
  Dividers and section-spanning cards need no modifier.
- **`spacer()` in a row is an inert empty cell**: it occupies its column, contributes no width,
  and is never placed. This diverges from stacks (where a spacer greedily pushes content apart)
  on purpose — a grid has explicit gutters, so push-apart spacers have no meaning here.
- **`when`/`each` groups expand inline** at both levels: at grid level they produce rows, inside
  a row they produce cells (the `StackLayout::flatten` recursion), so reactive row sets reflow
  and renegotiate columns for free.
- **Row heights** are the max cell height *at the final column widths* (text height-for-width is
  correct); a `grow_h` cell makes its row stretch under a height proposal. Nested grids are
  ordinary cells.
- **RTL mirrors columns** transparently — geometry is computed LTR and every placement mirrors
  around the grid's width in the engine's `place_child`, like every other layout.

## 3. The facts channel

Cell/row metadata rides `Flex.grid: GridFacts { is_row, row_valign, col_span, align }`
(day-core `tree.rs`) — the shipped form of §7.2's ChildRef facts surface, read via
`LayoutOps::flex_of` and inert to every other layout. `grid_row` builds a **layout-only node**
with `is_row` set, carrying a horizontal `StackLayout` that only runs when the row is *not*
inside a grid — a stray `grid_row` degrades to a plain `row`. `.grid_span`/`.grid_align` build
their piece and merge facts onto its root node (`TreeOps::set_grid_facts`) — no wrapper node.

**Ordering rule (the `.grow` rule, same class):** grid modifiers go LAST in the chain. They mark
the node the grid will see; a later wrapper (`.padding`, `.frame`) would hide the facts:
`label("x").padding(4.0).grid_span(2)` works, `label("x").grid_span(2).padding(4.0)` does not.
A `day lint` rule (wrapper node carrying non-default `GridFacts` under a grid) is a candidate
follow-up.

## 4. Layout algorithm (normative)

One `geometry()` function serves both `measure` and `place`:

1. **Collect**: flatten group anchors; classify children into rows (`is_row`) and full-width
   cells; assign column indices from spans.
2. **Pass A — ideals**: measure every span-1 cell **unconstrained**; rigid cells set their
   column's ideal (max), `grow_w` cells flag their column flexible (their ideal is recorded only
   when the grid itself is width-unconstrained). Spanning rigid cells then distribute any deficit
   (once, closed-form). Full-width rigid cells record the grid-wide ideal; a full-width `grow_w`
   cell makes the grid width-flexible.
3. **Resolve columns**: with a width proposal and flexible columns, leftover = proposal − rigid
   ideals − gutters, split evenly among the flexible columns; otherwise every column takes its
   ideal. Grid width = the proposal when anything is flexible, else max(columns+gutters,
   full-width ideal).
4. **Pass B — heights**: measure every cell once at its final span width (`Proposal::new(Some(w),
   None)`); row height = max cell height; `grow_h` rows stretch additively under a height
   proposal (stretching never re-measures).
5. **Place**: re-run `geometry` at `Proposal::exact(bounds)` — every per-cell proposal is
   bit-identical to the measure pass (only `p.width` affects cell proposals), so placement
   measures purely from the engine's cache and positions each cell by the resolved alignment.
   Row nodes are transparent carriers and are never placed.

## 5. The performance contract

**Exactly two measure proposals per cell per layout — unconstrained and at-final-width — zero
measures during place, and no iterative negotiation.** Flexible widths and span deficits are
single closed-form divisions, so there is no candidate-width probing and no quadratic
re-measurement. Incremental updates re-enter at the boundary; clean cells answer both proposals
from the engine's per-node cache, so a one-cell change costs ~2 real measures. Everything else is
O(rows × columns) arithmetic per pass.

This is pinned as a golden test (`day-pieces/tests/grid.rs::grid_measure_calls_bounded`, per
§7.4's "measure-call counts are part of the golden tests"): a 600-cell boot stays ≤ 2·cells +
slack mock measures, and a single-cell update stays ≤ 6 with exactly one native mutation op.

The grid is **eager**: every cell realizes a node. The showcase stress page runs 800+ cells
comfortably inside a `scroll`; thousands-of-rows datasets belong in `list` (recycling) or a
future lazy grid (§7).

## 6. Restrictions

- Don't decorate a `grid_row` — rows are transparent carriers with no frame; `.background` etc.
  on a row is unsupported (decorate the cells or the grid). Consequently dayscript `node_frame`
  on a row id returns nothing. (A row-band placement variant was considered and rejected for the
  hidden geometry coupling it adds; revisit if row striping becomes a real need.)
- Grid modifiers must be outermost (§3).
- Rigid content sharing a flexible column can clip when the leftover is small — the same property
  stacks have; a per-column minimum-width floor is a possible v2.
- Cell gestures/ids work as usual (cells are ordinary pieces); rows have no identity.

## 7. Deferred (deliberately)

- **Per-column alignment via any cell** (SwiftUI `gridColumnAlignment`): position-dependent
  spooky action; per-cell `.grid_align` written in the row covers real layouts.
- **`gridCellUnsizedAxes`**: a third sizing mode whose interaction with flexible columns is
  subtle; `grow_w`/`grow_h` + spans cover the motivating cases.
- **`UnitPoint` cell anchors, numeric layout priority / weighted distribution, explicit column
  templates, baseline alignment** (Day has no baseline concept anywhere yet).
- **Lazy/adaptive grid** — SwiftUI itself splits eager `Grid` from `LazyVGrid`. A future
  `vgrid(grid_items, …)` needs viewport-driven realization (the `list` machinery), a different
  data flow entirely; nothing in this design blocks it.
