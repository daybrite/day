---
title: "4. Arrange the controls"
description: Arrange controls with rows, columns, spacing, and padding.
order: 4
section: Tutorials
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Add a reset button and a horizontal button row to the app from [lesson 3](/docs/tutorials/state).

## 1. Add the layout messages

Append to `resource/locales/en/app.ftl`:

<!-- tutorial-step: layout/messages -->

## 2. Compose the layout

Replace `src/lib.rs`:

<!-- tutorial-step: layout/ui -->

[`row`](https://docs.rs/day-pieces/latest/day_pieces/fn.row.html) arranges children horizontally;
[`column`](https://docs.rs/day-pieces/latest/day_pieces/fn.column.html) arranges them vertically.
Both accept tuples of Pieces. [`.spacing`](https://docs.rs/day-pieces/latest/day_pieces/trait.ColumnBuilder.html#tymethod.spacing)
separates siblings; [`.padding`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding)
adds space around content. [`.grow(Dimension::Both)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.grow)
uses available width and height. Labels wrap by default. `Space::M` and `Space::L` are spacing tokens.

[`set(0)`](https://docs.rs/day-reactive/latest/day_reactive/struct.Signal.html#method.set) resets
the shared count.

## 3. Resize and reset

<!-- tutorial-step: layout/launch -->

<!-- tutorial-step: layout/quit -->

Press **Add one** twice, then **Reset**; expect **Count: 0**. Check initial and minimum window
sizes: buttons remain in one row, and the hint wraps as needed. Quit before editing.

## Troubleshooting

If buttons stack, place them in [`row((...))`](https://docs.rs/day-pieces/latest/day_pieces/fn.row.html).
If the hint is clipped, check available space and manifest minimum dimensions. Test longer
translations at minimum size.

## Next step

Continue to [5. Navigate between pages](/docs/tutorials/navigation).
