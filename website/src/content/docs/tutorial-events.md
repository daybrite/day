---
title: "2. Handle a button click"
description: Register a button callback and display a localized alert.
order: 2
section: Tutorials
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Add a button that opens an alert. Start from [lesson 1](/docs/tutorials/first-app) in the same
app directory. Keep `Cargo.toml`, `Day.toml`, `build.rs`, and `src/main.rs`.

## 1. Add the messages

Append to `resource/locales/en/app.ftl`; retain existing keys:

<!-- tutorial-step: events/messages -->

## 2. Connect the button

Replace `src/lib.rs`:

<!-- tutorial-step: events/ui -->

[`column`](https://docs.rs/day-pieces/latest/day_pieces/fn.column.html) arranges controls vertically.
[`button`](https://docs.rs/day-pieces/latest/day_pieces/fn.button.html) creates the button;
[`.action`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.action)
registers its zero-argument click callback. `id!(try_button)` identifies it for automation;
IDs do not replace accessible labels.

[`day::task`](https://docs.rs/day-app/latest/day/fn.task.html) runs the async UI operation on the UI
thread; `await` waits for alert dismissal without blocking the event loop. Construct localized
messages inside this task. Run blocking network or file work on a worker thread.

## 3. Try the event

<!-- tutorial-step: events/launch -->

Press **Try the button**. Expect **Your button click reached the app.** Dismiss the alert and
repeat to verify the callback runs on each click.

<!-- tutorial-step: events/quit -->

## Troubleshooting

If the alert is absent, check the button's
[`.action`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.action)
callback, save, quit, and relaunch. Missing `try_button` or `click_received` accessors indicate
missing Fluent keys; add them and rebuild.

## Next step

Continue to [3. Update state](/docs/tutorials/state).
