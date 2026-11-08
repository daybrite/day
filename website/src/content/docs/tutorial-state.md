---
title: "3. Update state"
description: Bind a localized label to a reactive counter.
order: 3
section: Tutorials
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Add a counter to the app from [lesson 2](/docs/tutorials/events). Keep the project files and catalog.

## 1. Add the counter messages

Append to `resource/locales/en/app.ftl`:

<!-- tutorial-step: state/messages -->

`$count` defines the argument accepted by the generated `counter_value` accessor.

## 2. Store and display the count

Replace `src/lib.rs`:

<!-- tutorial-step: state/ui -->

[`Signal::new(0i64)`](https://docs.rs/day-reactive/latest/day_reactive/struct.Signal.html#method.new)
creates reactive state in the root's scope. Pass `count` to the localized accessor to track changes;
[`count.get()`](https://docs.rs/day-reactive/latest/day_reactive/struct.Signal.html#method.get)
would pass a snapshot.

The `move` closure copies the signal handle. The label and button share its value.
[`update`](https://docs.rs/day-reactive/latest/day_reactive/struct.Signal.html#method.update)
changes the value and notifies dependents; `saturating_add` prevents integer overflow.
The callback runs on the UI thread. Do not send a Signal to a worker thread.

## 3. Observe the state

<!-- tutorial-step: state/launch -->

<!-- tutorial-step: state/quit -->

Expect **Count: 0** initially and **Count: 2** after pressing **Add one** twice.
Relaunching resets the count to zero; state is stored only in memory.

## Troubleshooting

If the count does not change, pass `count` to the label and call
[`count.update(...)`](https://docs.rs/day-reactive/latest/day_reactive/struct.Signal.html#method.update)
in the callback. If the accessor signature is incorrect, check for `{ $count }` in
`counter_value` and rebuild.

## Next step

Continue to [4. Arrange the controls](/docs/tutorials/layout).
