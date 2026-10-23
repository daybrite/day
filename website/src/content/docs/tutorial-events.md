---
title: "2. Handle a button click"
description: Connect a native button to a callback and show a localized response.
order: 2
section: Tutorials
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Add a button that opens a confirmation alert. Start with the working macOS app from
[1. Open your first window](/docs/tutorials/first-app), in the same terminal and app directory.
Keep `Cargo.toml`, `Day.toml`, `build.rs`, and `src/main.rs` from that lesson.

## 1. Add the messages

Append these keys to `resource/locales/en/app.ftl`. Keep the existing keys:

<!-- tutorial-append: resource/locales/en/app.ftl -->
```ftl
try_button = Try the button
click_received = Your button click reached the app.
```

## 2. Connect the button

Replace all of `src/lib.rs` with this complete file:

<!-- tutorial-file: src/lib.rs -->
```rust
use day::prelude::*;

day::resources!();
day::day_start!(options: res::window_options(), root);

pub fn root() -> impl Piece {
    column((
        label(res::str::welcome()),
        button(res::str::try_button())
            .action(|| {
                day::task(async {
                    alert(res::str::click_received()).await;
                });
            })
            .id(id!(try_button)),
    ))
    .spacing(Space::M)
    .padding(Space::L)
}
```

[`column`](https://daybrite.dev/api/day_pieces/fn.column.html) arranges the label and button vertically. [`button`](https://daybrite.dev/api/day_pieces/fn.button.html) creates a button Piece; [`.action`](https://daybrite.dev/api/day_pieces/trait.ButtonBuilder.html#tymethod.action)
registers the closure Day calls when it receives a click. The `||` closure takes no arguments.
`id!(try_button)` gives the control a stable identity for automation, independent of its translated
label. IDs do not replace accessible text.

An alert waits for a response. [`day::task`](https://daybrite.dev/api/day/fn.task.html) runs this asynchronous UI operation without blocking
the native event loop; `await` waits for dismissal. This task stays on the UI thread. It is not
a place for blocking network or file work. The message is constructed inside the UI task.

## 3. Try the event

```sh
day launch -t macos-appkit
```

Press **Try the button**. An alert displays **Your button click reached the app.** Dismiss it,
then press the button again to confirm the callback runs on each click. Quit with **Command-Q**.

## Troubleshooting

If the alert does not appear, check that [`.action(...)`](https://daybrite.dev/api/day_pieces/trait.ButtonBuilder.html#tymethod.action) is attached to [`button`](https://daybrite.dev/api/day_pieces/fn.button.html), save the file,
quit the previous app, and relaunch. If `try_button` or `click_received` is missing at compile
time, check the Fluent keys and rebuild.

## Next step

Continue to [3. Update state](/docs/tutorials/state), where a click changes a value on screen.
