---
title: "6. Finish and test the app"
description: Complete the session counter and verify counting, reset, navigation, and shared state with a dayscript.
order: 6
section: Tutorials
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Finish the app with buttons that navigate between its two sections, then run an automated
walkthrough. Start from [5. Navigate between pages](/docs/tutorials/navigation). Keep
`Cargo.toml`, `Day.toml`, `build.rs`, and `src/main.rs` from lesson 1. The complete UI and catalog
below replace their earlier versions.

## 1. Complete the catalog

Replace `resource/locales/en/app.ftl` with this catalog. The earlier alert exercise's unused
messages are no longer needed:

<!-- tutorial-file: resource/locales/en/app.ftl -->
```ftl
app_title = Session counter
counter_value = Count: { $count }
add_one = Add one
reset_count = Reset
session_hint = Add one after each session. Reset when you start a new day.
nav_counter = Counter
nav_summary = Summary
summary_value = Sessions completed: { $count }
summary_hint = This count lasts until you quit the app.
show_summary = Show summary
back_to_counter = Back to counter
```

## 2. Complete the UI

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
        nav_link(res::str::show_summary(), "summary").id(id!(summary_button)),
    ))
    .spacing(Space::M)
    .padding(Space::L)
    .grow()
}

fn summary_page(count: Signal<i64>) -> impl Piece {
    column((
        label(res::str::summary_value(count)).id(id!(summary_label)),
        label(res::str::summary_hint()),
        nav_link(res::str::back_to_counter(), "counter").id(id!(counter_button)),
    ))
    .spacing(Space::M)
    .padding(Space::L)
    .grow()
}
```

`nav_link` makes a button that navigates to a route key. These links and the sidebar update the
same navigation state. This app selects peer sections; it does not push a detail page onto a
back stack. Every visible message comes from the generated catalog accessors.

## 3. Run the finished app

```sh
day launch -t macos-appkit
```

Add two sessions and press **Show summary**. Expect **Sessions completed: 2**. Press **Back to
counter**, reset, and open the summary again; expect zero. Quit with **Command-Q**.

The app now combines event callbacks, reactive state, layout, and navigation. Press **Add one**
after completing a session. The count stays in memory and resets when you quit.

## 4. Automate the walkthrough

Create a script directory:

```sh
mkdir -p dayscript
```

Create `dayscript/session-counter.yaml`:

<!-- tutorial-file: dayscript/session-counter.yaml -->
```yaml
name: session-counter
on_failure: stop
flow:
  - wait_for: { id: increment_button }
  - assert_text: { id: counter_label, key: counter_value, args: { count: 0 } }
  - tap: { id: increment_button }
  - tap: { id: increment_button }
  - assert_text: { id: counter_label, key: counter_value, args: { count: 2 } }
  - tap: { id: summary_button }
  - assert_route: { route: summary }
  - assert_text: { id: summary_label, key: summary_value, args: { count: 2 } }
  - tap: { id: counter_button }
  - assert_route: { route: counter }
  - assert_text: { id: counter_label, key: counter_value, args: { count: 2 } }
  - tap: { id: reset_button }
  - assert_text: { id: counter_label, key: counter_value, args: { count: 0 } }
  - tap: { id: summary_button }
  - assert_text: { id: summary_label, key: summary_value, args: { count: 0 } }
```

A **dayscript** is a YAML walkthrough executed by Day's automation engine. IDs locate controls;
message keys and arguments express expected localized text. `assert_route` checks the selected
route. `on_failure: stop` stops this walkthrough at its first failure.

Run from the app directory:

```sh
day launch -t macos-appkit --locale en --script dayscript/session-counter.yaml
```

Expect all 15 steps to pass and the command to exit with status zero. Script assertion failures
exit with status 5. Scripted taps exercise Day's event dispatch inside the native app; also try
the controls by hand because this is not a physical mouse or keyboard test.

## Troubleshooting

| Symptom | Check and recovery |
| --- | --- |
| A script cannot find an ID | Match the YAML identifier to `id!(...)`, save both files, and rerun without `--skip-build`. |
| The first count is not zero | Start a fresh app run; the script assumes a new in-memory count. |
| Navigation does nothing | Match `nav_link` keys to the `.item` keys; a visible translated label is not a route key. |
| A text assertion fails | Compare the expected argument with the actual count and keep the final Fluent catalog and Rust file together. |

## Next steps

You have completed the macOS learning path. Try the
[other-platform branch](/docs/tutorials/other-platforms) if you need another target.
For a larger app, continue with [localization](/docs/how-to/localization),
[state](/docs/explanation/state), or [navigation](/docs/how-to/navigation).
