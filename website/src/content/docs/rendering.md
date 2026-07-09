---
title: How rendering works
description: "Following a widget through the system: mounting, the realized tree, patches, events, the turn, and the mock toolkit."
order: 51
section: Under the hood
---

[Architecture](/docs/architecture) drew the map; this page follows the traffic. It traces a
widget from `build` to pixels, a click from the native event to your closure, and a signal write
back out to the screen. None of this is required reading to *use* Day, but it makes the
framework predictable in the way only a mechanism you can picture is.

## The realized tree

`day-core` keeps one arena-allocated tree per window — the **realized tree**. Each node records:

```text
NodeData
├─ kind            "label" | "button" | "column" | …
├─ handle          Option<toolkit handle>      ← None for layout-only nodes
├─ parent/children tree links
├─ layout          the node's Layout impl + flex facts (grow, spacer, group)
├─ scope           reactive Scope owning this node's bindings & handlers
├─ id, a11y        stable identifier + accessibility props
└─ measure cache   proposal → size, plus a needs_measure flag
```

The `handle` is whatever the backend wants it to be — a retained `NSView` pointer on AppKit, a
JNI `GlobalRef` on Android, an opaque C++ pointer on Qt. `day-core` never looks inside it; it
only hands it back to the toolkit.

Note what the tree *doesn't* have: a shadow copy for diffing. This tree is the only tree.

## Mounting

When a Piece's `build` runs, the node is created, and — for native kinds — the backend is asked
to realize it:

```text
cx.leaf("button", &ButtonProps { title: "Save" })
  │
  ├─ insert node into the arena, under the current parent
  ├─ toolkit.realize("button", props, node_id)      → native NSButton, returns Handle
  ├─ compute native insertion index                  (skipping layout-only ancestors:
  │                                                   a column has no native counterpart,
  │                                                   so its children flatten into the
  │                                                   nearest real native container)
  ├─ toolkit.insert(parent_handle, handle, index)
  └─ mark ancestors needs_measure
```

That index computation is the subtle part: Day's tree has structure (columns, padding wrappers)
that the native view hierarchy doesn't, so native children of a container are the *flattened*
in-order native descendants. Keeping those indices right during `each` reorders is one of the
jobs the mock-toolkit golden tests pin down.

## Updates are patches

Nothing ever re-builds a mounted widget. Changes arrive as **patches** — small enums per kind
(`LabelPatch::Text(String)`, `SliderPatch::Value(f64)`) — produced by the bindings that
[reactivity](/docs/reactivity) re-runs:

```text
signal write ─► binding re-runs ─► eq-gate ─► tree.patch(node, patch, affects_size)
                                                 │
                                                 ├─ toolkit.update(handle, patch)   ← one native setter
                                                 └─ if affects_size:
                                                       mark needs_measure, bubble to boundary
```

The `affects_size` flag is decided by the piece author: a text change might, a color change
doesn't. Size-affecting patches queue incremental relayout
([how that works](/docs/layout#incremental-relayout)); everything else is done after one native
call.

## Events flow in through a trampoline

Backends register native callbacks once per widget and translate them to a uniform
`(NodeId, Event)` stream. On AppKit, for instance, a Rust-defined Objective-C class holds the
node id and is set as the widget's target; its action method classifies the sender (switch →
`ToggleChanged`, slider → `ValueChanged`, button → `Pressed`) and emits. The contract for every
backend's sink is *enqueue only*: no user code runs inside the native callback, which keeps
re-entrancy problems (a handler mutating the tree mid-native-dispatch) structurally impossible.

```text
user clicks NSButton
  └─ [DayTarget action:] ─► emit(node 17, Pressed) ─► event queue
                                             (native callback returns)
 next: day-core drains the queue as a fresh batch
  └─ handler registered via cx.on(node 17, …) runs your closure
      └─ count.update(|c| *c += 1)      … and we're in the reactivity story
```

Two-way controls (text fields especially) are more careful than they look: the native widget is
the source of truth while focused, writes are origin-tagged so a signal update echoing back
doesn't clobber what the user is typing, and programmatic writes during IME composition are
deferred until composition ends. This is the kind of edge you get to *not* think about because
the framework's controlled-input path owns it.

## The turn

Everything above is sequenced by the **turn** — Day's unit of "handle things, then settle":

```text
native event(s)
  1. handlers run, signal writes batch
  2. reactive drain to fixpoint          (bindings re-run, patches applied)
  3. one posted main-loop callback:
       incremental layout for dirty boundaries
       set_frame for frames that actually changed (½-pixel epsilon)
       release queue drains               (widgets disposed this turn are freed)
```

One turn, at most one layout pass, and native mutations grouped where the toolkit wants them.
There is no per-frame tick: an idle Day app runs no code, which is the runtime-profile claim in
concrete form.

## Drawing: canvas as a display list

`canvas(|d, size| …)` doesn't hand you a native graphics context — the closure records into a
`Vec<DrawOp>` (fill/stroke shape, text run, transform, clip…), and the backend replays the ops
through the platform's 2D API: Core Graphics, `android.graphics.Canvas`, cairo, `QPainter`.
The closure is itself a binding, so a signal it reads re-records and replays just that node; the
op list's `PartialEq` is the equality gate, so an identical recording skips the replay entirely.
One FFI hop carries the whole buffer, which matters on JNI. And because text ops go through the
toolkit's text engine, canvas text gets native fonts, shaping, and bidi — Day still isn't
rasterizing anything itself.

## The mock toolkit

`day-mock` implements the full `Toolkit` trait with no display: handles are plain structs,
measurement is deterministic (fixed metrics per kind), and every call appends to an op log.
Tests mount real Pieces against it inside ordinary `cargo test` — this is condensed from Day's
own test suite:

```rust
let (mock, probe) = MockToolkit::new();
day_core::launch_with(mock, options, || counter_ui());

probe.clear_log();
probe.emit(button_node, Event::Pressed);            // synthesize the click

let muts = probe.mutations();
assert_eq!(muts.len(), 1);                          // ONE native mutation for the click
assert!(muts[0].contains("update day.label"));
assert!(probe.measure_calls() <= 6);                // relayout stayed on the label's path
```

Those assertions are the interesting part: the framework's core promises — one click, one native
mutation; bounded measure calls per layout pass — are written down as golden tests over the op
log, so "fine-grained" is enforced by CI rather than by folklore. Your own component tests get
the same leverage: no simulator, no display server, milliseconds per test.

## Teardown

When structure changes (`when` flips, an `each` row leaves), the subtree's scope is disposed —
bindings and handlers die with it — and the nodes go onto a release queue drained at the turn
boundary, where the backend frees the native widgets (with toolkit-appropriate deferral, like
Qt's `deleteLater`). A signal write racing a disposed binding is a checked no-op. The ownership
story is short enough to state completely: the scope owns the reactive machinery, the tree owns
the handles, and both are torn down together, once, at a safe point.

---

That's the whole loop: build once, patch forever, one turn at a time. The
[reference section](/docs/reference) documents each subsystem in more depth.
