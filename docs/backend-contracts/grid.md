---
title: "Grid layout contract"
description: "Grid layout contract: retained contributor discussion and its implementation boundaries."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Grid layout contract

> [!NOTE]
> **Status: normative implementation contract.** This contributor record preserves earlier design and implementation
> discussion. Examples here are not stable API guidance. See the [app documentation](../grid.md)
> for the current entry point. Platform execution claims are historical and were not rerun in Phase 2.

## 3. The facts channel

Cell/row metadata rides `Flex.grid: GridFacts { is_row, row_valign, col_span, align }`
(day-core `tree.rs`), the shipped form of §7.2's ChildRef facts surface, read via
`LayoutOps::flex_of` and inert to every other layout. `grid_row` builds a **layout-only node**
with `is_row` set, carrying a horizontal `StackLayout` that only runs when the row is *not*
inside a grid; a stray `grid_row` degrades to a plain `row`. `.grid_span`/`.grid_align` build
their piece and merge facts onto its root node (`TreeOps::set_grid_facts`): no wrapper node.

**Position-free.** Grid facts are outer-layout facts ([docs/api-style.md](../api-style.md)
"Modifier classes"): they describe the piece to the grid it sits in, so they land on the
outermost node whatever wrappers follow them. `label(res::str::sample_label()).padding(4.0).grid_span(2)` and
`label(res::str::sample_label()).grid_span(2).padding(4.0)` are the same cell.

## 4. Layout algorithm (normative)

One `geometry()` function serves both `measure` and `place`:

1. **Collect**: flatten group anchors; classify children into rows (`is_row`) and full-width
   cells; assign column indices from spans.
2. **Pass A (ideals)**: measure every span-1 cell **unconstrained**; rigid cells set their
   column's ideal (max), `grow_w` cells flag their column flexible (their ideal is recorded only
   when the grid itself is width-unconstrained). Spanning rigid cells then distribute any deficit
   (once, closed-form). Full-width rigid cells record the grid-wide ideal; a full-width `grow_w`
   cell makes the grid width-flexible.
3. **Resolve columns**: with a width proposal and flexible columns, leftover = proposal − rigid
   ideals − gutters, split evenly among the flexible columns; otherwise every column takes its
   ideal. Grid width = the proposal when anything is flexible, else max(columns+gutters,
   full-width ideal).
4. **Pass B (heights)**: measure every cell once at its final span width (`Proposal::new(Some(w),
   None)`); row height = max cell height; `grow_h` rows stretch additively under a height
   proposal (stretching never re-measures).
5. **Place**: re-run `geometry` at `Proposal::exact(bounds)`; every per-cell proposal is
   bit-identical to the measure pass (only `p.width` affects cell proposals), so placement
   measures purely from the engine's cache and positions each cell by the resolved alignment.
   Row nodes are transparent carriers and are never placed.

## 5. The performance contract

**Exactly two measure proposals per cell per layout (unconstrained and at-final-width), zero
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

## Verification scope

This retained technical material has not received a complete signature, default, lifecycle,
and native-runtime review against the 2026-10-22 working tree. Treat those details and unmarked
code examples as unverified for this revision. The
[technical-review ledger](https://github.com/daybrite/day/blob/main/docs/history/documentation/coverage.csv)
records its status. Code blocks are API excerpts or integration fragments unless explicitly
designated in the [example catalog](https://daybrite.dev/docs/reference/examples/catalog#technical-recipes).
No current all-platform pass or screenshot attribution is established by this page.
