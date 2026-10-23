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

<!-- tutorial-append: resource/locales/en/app.ftl -->
```ftl
reset_count = Reset
session_hint = Add one after each session. Reset when you start a new day.
```

## 2. Compose the layout

Replace all of `src/lib.rs`:

<!-- tutorial-file: src/lib.rs -->
```rust
use day::prelude::*;

day::resources!();
day::day_start!(options: res::window_options(), root);

pub fn root() -> impl Piece {
    let count = Signal::new(0i64);

    column((
        label(res::str::welcome()),
        label(res::str::counter_value(count)).id(id!(counter_label)),
        row((
            button(res::str::add_one())
                .action(move || count.update(|n| *n = n.saturating_add(1)))
                .id(id!(increment_button)),
            button(res::str::reset_count())
                .action(move || count.set(0))
                .id(id!(reset_button)),
        ))
        .spacing(Space::M),
        label(res::str::session_hint()),
    ))
    .spacing(Space::M)
    .padding(Space::L)
    .grow()
}
```

[`row`](https://daybrite.dev/api/day_pieces/fn.row.html) arranges its children horizontally; [`column`](https://daybrite.dev/api/day_pieces/fn.column.html) arranges them vertically. Both take a tuple
of Pieces. [`.spacing`](https://daybrite.dev/api/day_pieces/trait.ColumnBuilder.html#tymethod.spacing) separates siblings; [`.padding`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.padding) leaves room around the container's content.
[`.grow()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.grow) asks the outer container to take available space. Labels wrap by default, so the hint
can use multiple lines when space is narrow. `Space::M` and `Space::L` are spacing tokens, not pixel literals.

The reset callback uses [`set(0)`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.set) to replace the value. Both actions share the signal you created
above the layout.

## 3. Resize and reset

```sh
day launch -t macos-appkit
```

Press **Add one** twice and then **Reset**. The label returns to **Count: 0**. Resize the window
toward its minimum width: the buttons stay in a row and the hint can wrap beneath them. Test the
window's minimum size as well as its initial size. Quit before the next edit.

## Troubleshooting

If the buttons stack vertically, check that they are inside [`row((...))`](https://daybrite.dev/api/day_pieces/fn.row.html). If the hint is clipped,
check the available space and the manifest's minimum dimensions. Check longer translations at the minimum window size, too.

## Next step

Continue to [5. Navigate between pages](/docs/tutorials/navigation).
