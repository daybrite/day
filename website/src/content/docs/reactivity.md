---
title: Reactivity
description: "Keep controls up to date with signals, derived values, and reactive closures."
order: 12
section: Explanation
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

A Piece builds a subtree; a reactive binding updates values inside that subtree. A signal read
only subscribes its caller when a tracked computation is active. Reading a signal in an ordinary
builder or event handler does not arrange for that function to run again.

When a button action changes a signal, Day reruns the bindings that read it. A counter label
can receive the new value while the button and surrounding layout keep their existing native
controls. The row builder runs again only when a structural operation creates a new row.

```text
Button action → Signal write → subscribed binding → native control update
                    ↓
                derived Memo → bindings that use its changed value
```

A signal stores state; a memo calculates a value from that state; a binding applies a value to
the interface. Put computations inside a tracked closure when they need to follow later changes.

## Follow values, not builder calls

API fragment; not a complete app. `counter_value` is defined in the
[example catalog](/docs/reference/examples/catalog).

```rust
use day::prelude::*;
let count = Signal::new(0i64);
label(res::str::counter_value(count.get()));
label(res::str::counter_value(count));
```

The first label receives the initial count. The second keeps the signal as a reactive message
argument, so it follows count and locale changes. Both use generated localized accessors.
Use [the state lesson](/docs/tutorials/state) for a complete app.

## Signal and memo contracts

| Surface | Contract |
| --- | --- |
| `Signal<T: 'static>` | Copyable handle to a scope-owned cell; copying does not clone `T`; the handle is not `Send` |
| `Signal::new(value)` | Allocates in the current scope; use `new_in(scope, value)` for an explicit owner |
| `get()` | Tracked read requiring `T: Clone` |
| `with(f)` | Tracked borrowed read; avoids cloning the stored value |
| `get_untracked()` | Clone without subscribing |
| `set(value)` / `update(f)` | Notify observers even if the resulting value compares equal |
| `set_if_changed(value)` | Uses `PartialEq` to avoid a write notification when unchanged |
| `Memo::new(f)` | Cached derived value with equality gating; dependents need not run when its output is unchanged |

A memo does not turn arbitrary mutation into reactive state. Its inputs must be tracked reads.
Keep derived calculations in memos or bindings; an effect that writes one of its own dependencies
can create a cycle. The runtime has a per-drain rerun cap, with debug panic and release warning/
deferral behavior. That guard is a diagnostic, not an application control-flow mechanism.

## Effects, bindings, and watches

`Effect::new(f)` runs immediately and tracks reads. `bind(compute, apply)` tracks `compute`,
then applies the initial value and unequal later values outside tracking. `bind_always` has no
equality gate. `watch(source, callback)` records the initial source without calling the callback;
later reruns pass references to the new and previous values. It has a `Clone` bound, not a
`PartialEq` bound, so it is not an equality-filtered version of `bind`.

Day's control builders establish bindings when built. A changed signal can update a native
attribute without rebuilding the root. Structural pieces such as `when`, `each`, and navigation
hosts can still build and dispose subtrees. A source change is not guaranteed to produce exactly
one native call: fan-out, structure, layout, and backend implementations determine the work.

## Ownership and disposal

Create long-lived application or window state outside replaceable page closures. A signal made
inside a page is owned by that page's scope. Re-entering the page creates a new signal unless
its state was retained above the page. [State ownership](/docs/explanation/state) explains scene
and environment patterns; [navigation](/docs/how-to/navigation) describes page lifetime.

Disposed signal reads panic in all builds; use `try_with` when a callback may outlive its owner.
Writes to a disposed signal are ignored (debug builds can warn). A `Setter` delivery after
disposal silently does nothing. These rules do not make a stale read safe.

A window's tree and its reactive scope have related but distinct ownership. Scope disposal
removes its reactive nodes and cleanups; the renderer handles native subtree release. Do not
hold native handles past their owning tree's lifetime.

## Threading and async work

Signals, memos, and UI trees belong to the UI thread. `Signal::setter()` requires `T: Send` and
returns a copyable, write-only `Setter<T>` for data delivery from workers. The backend's main
poster must already be installed. `on_main(f)` also requires `FnOnce() + Send + 'static`; it
cannot capture a `Signal` merely because the closure will eventually run on the UI thread.
Calling it before backend initialization panics.

Use `day::task` for a future that reads or writes signals after an await. It polls first before
returning and resumes on the UI loop; it does not move blocking work to a worker. Use
`day::spawn_blocking` for blocking native work. On web, blocking jobs run inline when polled.
Send result data back and format localized messages on the UI thread.

Tasks are not scope-owned. `TaskHandle::abort` drops a pending future; dropping the handle alone
does not cancel it. `day::reactive::Resource` supplies scope-owned, latest-result-wins loading.
See [async ownership and shutdown](/docs/explanation/async) and the
[HTTP recipe](/docs/how-to/guide-http).

## Batches and limits

Event dispatch batches signal writes, then drains reactions. `batch(f)` groups explicit writes.
The scheduler orders structural reactions before attribute reactions and uses scope depth to
avoid running disposed children. Layout and native event delivery add their own scheduling;
this is not a frame-rate or timing guarantee.


## Handle an event and update state

A native button action handles pointer, keyboard, and accessibility activation. Use `.action`
for buttons rather than installing a separate `.on_tap` gesture. Keep the count in the caller's
scope when it must survive page replacement. The saturating increment avoids integer overflow.

Complete module for `src/events.rs` in the
[technical recipe harness](/docs/reference/examples/catalog#technical-recipes).
Add the matching Fluent messages below. This module is compiled as part of the harness;
it is not a standalone app.

<!-- technical-example: events -->
```rust
use crate::res;
use day::prelude::*;

pub fn counter(count: Signal<i64>) -> impl Piece {
    column((
        label(res::str::event_count(count)).id(id!(event_count)),
        button(res::str::event_increment())
            .id(id!(event_increment))
            .action(move || count.update(|value| *value = value.saturating_add(1))),
        button(res::str::event_reset())
            .id(id!(event_reset))
            .action(move || count.set(0)),
    ))
    .spacing(Space::M)
}
```

<!-- technical-catalog: events -->
```ftl
event_count = Count: { $count }
event_increment = Add one
event_reset = Reset
```

These action closures return `()`. When starting asynchronous work, use a block with a
semicolon after `day::task(...)` so its `TaskHandle` is not the callback's return value.
The task can outlive this page; abort it on cleanup when it owns page-specific work.
