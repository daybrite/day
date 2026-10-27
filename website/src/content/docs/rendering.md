---
navTitle: "Rendering"
title: How rendering works
description: "Following a widget through the system: mounting, the realized tree, patches, events, the turn, and the mock toolkit."
order: 51
section: Contributing
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day’s renderer connects the piece tree to native widgets. This page follows widget creation,
event handling, and signal updates through that process. It complements the
[architecture guide](/contributing/architecture) and is intended for developers working on the framework.

## The realized tree

`day-core` keeps one arena-allocated tree per window, the **realized tree**. Each node records:

```text
NodeData
├─ kind            "day.label" | "day.button" | "day.column" | …
├─ handle          Option<toolkit handle>      ← None for layout-only nodes
├─ parent/children tree links
├─ layout          the node's Layout impl + flex facts (grow, spacer, group)
├─ scope           reactive Scope owning this node's bindings & handlers
├─ id, a11y        stable identifier + accessibility props
└─ measure cache   proposal → size, plus a needs_measure flag
```

The `handle` is whatever the [backend](/docs/reference/glossary#backend) wants it to be: a retained `NSView` pointer on AppKit, a
JNI `GlobalRef` on Android, an opaque C++ pointer on Qt. `day-core` never looks inside it; it
only hands it back to the [toolkit](/docs/reference/glossary#toolkit).

The tree has no shadow copy for diffing; it is the only representation of the UI that Day keeps.

## Mounting

When a Piece's [`build`](https://docs.rs/day-core/latest/day_core/trait.Piece.html#tymethod.build) runs, the node is created, and (for native kinds) the backend is asked
to realize it:

```text
button(res::str::save()) → Piece::build (conceptual flow)
  │
  ├─ insert node into the arena, under the current parent
  ├─ toolkit.realize("day.button", props, node_id)  → native NSButton, returns Handle
  ├─ compute native insertion index                  (skipping layout-only ancestors:
  │                                                   padding wrappers have no native counterpart;
  │                                                   row/column create native containers;
  │                                                   flatten only layout-only ancestors)
  ├─ toolkit.insert(parent_handle, handle, index)
  └─ mark ancestors needs_measure
```

The index computation is subtle because Day's tree has structure (columns, padding wrappers)
that the native view hierarchy doesn't, so native children of a container are the *flattened*
in-order native descendants. Keeping those indices right during [`each`](https://docs.rs/day-pieces/latest/day_pieces/fn.each.html) reorders is one of the
jobs the mock-toolkit golden tests pin down.

## Updates are patches

Bound attributes normally update an existing widget. Structural changes may dispose and rebuild
subtrees. Attribute changes arrive as **patches**, small enums per kind
(`LabelPatch::Text(String)`, `SliderPatch::Value(f64)`), produced by the bindings that
[reactivity](/docs/explanation/reactivity) re-runs:

```text
signal write ─► binding re-runs ─► eq-gate ─► tree.patch(node, patch, affects_size)
                                                 │
                                                 ├─ toolkit.update(handle, patch)   ← one native setter
                                                 └─ if affects_size:
                                                       mark needs_measure, bubble to boundary
```

The `affects_size` flag is decided by the [piece](/docs/reference/glossary#piece) author: a text change might, a color change
doesn't. Size-affecting patches queue incremental relayout
([how that works](/docs/explanation/layout#incremental-relayout)); everything else is done after one native
call.

## Event delivery

Backends register native callbacks once per widget and translate them to a uniform
`(NodeId, Event)` stream. On AppKit, for instance, a Rust-defined Objective-C class holds the
node id and is set as the widget's target; its action method classifies the sender (switch →
`ToggleChanged`, slider → `ValueChanged`, button → `Pressed`) and emits. Core queues events for batched delivery. Backend callbacks and custom extensions must still
respect native re-entrancy rules; queued dispatch is not a guarantee that arbitrary native
callbacks cannot re-enter toolkit code.

```text
user clicks NSButton
  └─ [DayTarget action:] ─► emit(node 17, Pressed) ─► event queue
                                             (native callback returns)
 next: day-core drains the queue as a fresh batch
  └─ handler registered via cx.on(node 17, …) runs your closure
      └─ count.update(|c| *c += 1)      … and we're in the reactivity story
```

Two-way controls (text fields especially) take extra care: while the field is focused, the native
widget holds the authoritative value, writes are origin-tagged so a signal update echoing back
doesn't overwrite what the user is typing, and programmatic writes during IME composition are
deferred until composition ends. The framework's controlled-input path handles all of this, so
your code never sees it.

## The turn

Everything above is sequenced by the **turn**, Day's unit of "handle things, then settle":

```text
native event(s)
  1. handlers run, signal writes batch
  2. reactive drain to fixpoint          (bindings re-run, patches applied)
  3. one posted main-loop callback:
       incremental layout for dirty boundaries
       set_frame for frames that actually changed (½-pixel epsilon)
       release queue drains               (widgets disposed this turn are freed)
```

Each turn runs at most one layout pass and groups native mutations where the toolkit wants them.
Bindings do not require a continuous rebuild loop. Timers, animations, background services,
and toolkit work can still run while the app has no user input.

## Drawing: canvas as a display list

[`canvas(|d, size| …)`](https://docs.rs/day-pieces/latest/day_pieces/fn.canvas.html) doesn't hand you a native graphics context; the closure records into a
`Vec<DrawOp>` (fill/stroke shape, text run, transform, clip…), and the backend replays the ops
through the platform's 2D API: Core Graphics, `android.graphics.Canvas`, cairo, `QPainter`.
The closure is itself a binding, so a signal it reads re-records and replays just that node; the
op list's `PartialEq` is the equality gate, so an identical recording skips the replay entirely.
One FFI hop carries the whole buffer, which matters on JNI. And because text ops go through the
toolkit's text engine, canvas text gets native fonts, shaping, and bidi; Day still isn't
rasterizing anything itself.

## The mock toolkit

`day-mock` implements the full `Toolkit` trait with no display: handles are plain structs,
measurement is deterministic (fixed metrics per kind), and every call appends to an op log.
Tests mount real Pieces with this toolkit and inspect mutations, frames, and event behavior.
Use the actual [mock integration tests](https://github.com/daybrite/day/blob/main/crates/day-pieces/tests/mock_e2e.rs)
for the harness API, rather than assuming probe helpers or timing assertions from pseudocode.
Run `cargo test --locked -p day-pieces --test mock_e2e` from the repository root. Mock success
establishes shared behavior with deterministic measurement, not native rendering or accessibility.

## Teardown

When structure changes ([`when`](https://docs.rs/day-pieces/latest/day_pieces/fn.when.html) flips, an [`each`](https://docs.rs/day-pieces/latest/day_pieces/fn.each.html) row leaves), the subtree's scope is disposed
(bindings and handlers die with it), and the nodes go onto a release queue drained at the turn
boundary, where the backend frees the native widgets (with toolkit-appropriate deferral, like Qt's
`deleteLater`). Writes to a disposed signal are ignored; reads can panic. A task that reads after scope
disposal therefore still needs ownership or cancellation handling. The scope owns the
[reactive](/docs/reference/glossary#reactive) machinery, the tree owns the handles, and both are torn down
together at the turn boundary.

---

The [reference section](/docs/reference) documents each subsystem in more depth.

## Evidence and limits

Source review: [tree](https://github.com/daybrite/day/blob/main/crates/day-core/src/tree.rs),
[build context](https://github.com/daybrite/day/blob/main/crates/day-core/src/build.rs), and
[container realization](https://github.com/daybrite/day/blob/main/crates/day-pieces/src/containers.rs).
Diagrams are conceptual flows, not literal call signatures or measured operation counts.
Per-backend controlled input, native measurement, and release ordering require their target
regression tests; this page does not certify them on all platforms.
