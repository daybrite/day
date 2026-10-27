---
title: "4. Arrange the controls"
description: Combine rows and columns, add spacing, and reset the session counter.
order: 4
section: Tutorials
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Put two actions in a row beneath the count and make the content use the available space.
Start from [3. Update state](/docs/tutorials/state), in the same app directory.

## 1. Add the layout messages

Append to `resource/locales/en/app.ftl`:

<!-- tutorial-step: layout/messages -->

## 2. Compose the layout

Replace all of `src/lib.rs`:

<!-- tutorial-step: layout/ui -->

[`row`](https://docs.rs/day-pieces/latest/day_pieces/fn.row.html) arranges its children horizontally; [`column`](https://docs.rs/day-pieces/latest/day_pieces/fn.column.html) arranges them vertically. Both take a tuple
of Pieces. [`.spacing`](https://docs.rs/day-pieces/latest/day_pieces/trait.ColumnBuilder.html#tymethod.spacing) separates siblings; [`.padding`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding) leaves room around the container's content.
[`.grow()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.grow) asks the outer container to take available space. Labels wrap by default, so the hint
can use multiple lines when space is narrow. `Space::M` and `Space::L` are spacing tokens, not pixel literals.

The reset callback uses [`set(0)`](https://docs.rs/day-reactive/latest/day_reactive/struct.Signal.html#method.set) to replace the value. Both actions share the signal you created
above the layout.

## 3. Resize and reset

<!-- tutorial-step: layout/launch -->

<!-- tutorial-step: layout/quit -->

Press **Add one** twice and then **Reset**. The label returns to **Count: 0**. Resize the window
toward its minimum width: the buttons stay in a row and the hint can wrap beneath them. Test the
window's minimum size as well as its initial size. Quit before the next edit.

## Troubleshooting

If the buttons stack vertically, check that they are inside [`row((...))`](https://docs.rs/day-pieces/latest/day_pieces/fn.row.html). If the hint is clipped,
check the available space and the manifest's minimum dimensions. Check longer translations at the minimum window size, too.

## Next step

Continue to [5. Navigate between pages](/docs/tutorials/navigation).
