---
title: "Deferred grid design"
description: "Deferred grid design: retained contributor discussion and its implementation boundaries."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Deferred grid design

> [!NOTE]
> **Status: deferred design.** This contributor record preserves earlier design and implementation
> discussion. Examples here are not stable API guidance. See the [app documentation](../grid.md)
> for the current entry point. Platform execution claims are historical and were not rerun in Phase 2.

## 7. Deferred

- **Per-column alignment via any cell** (SwiftUI `gridColumnAlignment`): one cell would change
  the alignment of every cell in its column, an effect at a distance; per-cell `.grid_align`
  written in the row covers real layouts.
- **`gridCellUnsizedAxes`**: a third sizing mode whose interaction with flexible columns is
  subtle; `grow_w`/`grow_h` + spans cover the motivating cases.
- **`UnitPoint` cell anchors, numeric layout priority / weighted distribution, explicit column
  templates**.
- **Baseline alignment between cells.** Day does have a baseline concept now
  ([docs/baseline.md](../baseline.md)) and rows use it, but a grid would have to reconcile one baseline per row
  across cells that span rows, so `grid_row(..).align(VAlign::FirstBaseline)` reads as
  `Center` for the moment.
- **Lazy/adaptive grid.** SwiftUI itself splits eager `Grid` from `LazyVGrid`. A future
  `vgrid(grid_items, …)` needs viewport-driven realization (the `list` machinery), a different
  data flow entirely; nothing in this design blocks it.
