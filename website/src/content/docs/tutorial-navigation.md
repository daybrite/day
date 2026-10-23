---
title: "5. Navigate between pages"
description: Add counter and summary sections while keeping shared state above both pages.
order: 5
section: Tutorials
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Add a summary page that displays the same count. Complete
[4. Arrange the controls](/docs/tutorials/layout) first; keep the existing project files and catalog.

## 1. Name the sections

Append to `resource/locales/en/app.ftl`:

<!-- tutorial-append: resource/locales/en/app.ftl -->
```ftl
nav_counter = Counter
nav_summary = Summary
summary_value = Sessions completed: { $count }
summary_hint = This count lasts until you quit the app.
```

## 2. Share state across pages

Replace all of `src/lib.rs`:

<!-- tutorial-file: src/lib.rs -->
```rust
use day::prelude::*;

day::resources!();
day::day_start!(options: res::window_options(), root);

pub fn root() -> impl Piece {
    let count = Signal::new(0i64);
    let section = Signal::new("counter".to_string());

    nav(section)
        .style(NavStyle::Sidebar)
        .title(res::str::app_title())
        .item("counter", res::str::nav_counter(), move || {
            counter_page(count)
        })
        .item("summary", res::str::nav_summary(), move || {
            summary_page(count)
        })
        .grow()
}

fn counter_page(count: Signal<i64>) -> impl Piece {
    column((
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

fn summary_page(count: Signal<i64>) -> impl Piece {
    column((
        label(res::str::summary_value(count)).id(id!(summary_label)),
        label(res::str::summary_hint()),
    ))
    .spacing(Space::M)
    .padding(Space::L)
    .grow()
}
```

[`nav(section)`](https://daybrite.dev/api/day_pieces/fn.nav.html) binds navigation to the selected route key. The strings `counter` and `summary`
are protocol identifiers, not visible labels; the Fluent accessors supply the labels.
[`.item`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.item) associates each key with a page-building closure. `NavStyle::Sidebar` selects a sidebar
presentation on this desktop target.

Create `count` in `root`, above the page builders. Sidebar pages can be disposed when you switch
sections; state created inside a page would belong to that page's lifetime. Passing the root's
signal to both pages keeps the count alive across switches. This does not save it across app launches.

## 3. Follow the count

```sh
day launch -t macos-appkit
```

In **Counter**, press **Add one** three times. Select **Summary** and expect **Sessions completed: 3**.
Return to **Counter** and expect **Count: 3**. Press **Reset**, then revisit **Summary**; it
should show zero. Quit the app.

## Troubleshooting

If switching pages resets the count, ensure [`Signal::new(0i64)`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.new) appears only in `root`, not in a
page function. If a section is blank, compare the initial selection with the [`.item`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.item) keys.
Both keys and the selection must use the same spelling.

## Next step

Continue to [6. Finish and test the app](/docs/tutorials/complete-app).
