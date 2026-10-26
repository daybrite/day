---
title: "3. Update state"
description: Store a count in a signal and bind a localized label to it.
order: 3
section: Tutorials
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Turn the button into a counter. Complete [2. Handle a button click](/docs/tutorials/events)
first. Work in the same app directory and keep the project files and catalog from earlier lessons.

## 1. Add the counter messages

Append to `resource/locales/en/app.ftl`:

<!-- tutorial-step: state/messages -->

`$count` is a Fluent argument. The generated `counter_value` accessor accepts that argument
and checks its name at compile time.

## 2. Store and display the count

Replace all of `src/lib.rs`:

<!-- tutorial-step: state/ui -->

A **signal** is a reactive value: reading it through a binding connects the UI to future changes.
[`Signal::new(0i64)`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.new) starts the count at zero in the root's current scope. Pass `count` to the
localized accessor so the label follows changes. Passing [`count.get()`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.get) would take a snapshot
instead.

The `move` closure captures the signal handle. Handles are copyable; both the label and button
refer to the same value. [`update`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.update) changes it and notifies its dependents. `saturating_add`
keeps this counter from overflowing at the integer limit. The callback runs on the UI
thread; do not send a `Signal` to a worker thread.

## 3. Observe the state

<!-- tutorial-step: state/launch -->

<!-- tutorial-step: state/quit -->

The label starts at **Count: 0**. Press **Add one** twice; it should read **Count: 2**.
Quit and relaunch: the count returns to zero. This app stores the count in memory only.

## Troubleshooting

If the count never changes, check that the label receives `count`, not [`count.get()`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.get), and that
the callback calls [`count.update(...)`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.update). If the generated function has the wrong arguments,
check that `counter_value` includes `{ $count }` in the catalog, then rebuild.

## Next step

Continue to [4. Arrange the controls](/docs/tutorials/layout).
