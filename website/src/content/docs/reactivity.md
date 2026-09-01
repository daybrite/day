---
title: Reactivity
description: "Signals, memos, effects, and scopes: how a built-once tree stays live without diffing."
order: 12
section: Concepts
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day's reactive system is the reason a [Piece tree built once](/docs/pieces) can keep moving. It's
a fine-grained signal graph in the SolidJS and floem tradition: state lives in **signals**,
derived values in **memos**, and side effects (including every native-widget update) in
**effects** that re-run when something they read changes.

If you've used SwiftUI, React, or Flutter, the difference to learn is how a state change
propagates. A signal write re-runs exactly the closures that read that signal, and each of those
typically ends in one native setter call. Your view function does not run again, and there is no
tree to compare.

## Signals

A `Signal<T>` is a reactive cell. The handle is `Copy`; you move it into as many closures as you
like without cloning it first:

```rust
let count = Signal::new(0i64);

count.get();                       // read (tracked — see below)
count.set(5);                      // write
count.update(|c| *c += 1);         // read-modify-write
count.with(|c| c.to_string());     // read by reference, no clone
count.get_untracked();             // read without subscribing
```

A read is *tracked* when it happens inside a reactive context: a memo, an effect, or one of the
reactive closures you hand to Pieces. Tracking is how the graph learns its edges: while your
closure runs, every `.get()` registers the enclosing computation as an observer of that signal.

```rust
let name = Signal::new(String::from("Ada"));

// The closure is a reactive context. It reads `name`, so it re-runs — and
// updates this one native label — whenever `name` changes.
label(move || format!("Hello, {}", name.get()))
```

Untracked reads are the escape hatch for "I want the current value but no subscription" (common
in event handlers, which are not reactive contexts anyway, and in effects that would otherwise
over-subscribe).

## Memos

A `Memo<T>` is a derived value: computed from signals, cached, and only recomputed when a source
actually changed. Observers of a memo re-run only when the memo's *output* changes (`PartialEq`
decides), which stops irrelevant updates from propagating:

```rust
let items = Signal::new(Vec::<Item>::new());
let total = Memo::new(move || items.with(|v| v.iter().map(|i| i.price).sum::<f64>()));

// Re-runs when the total changes — not on every items edit that leaves it equal.
label(move || format!("{:.2} €", total.get()))
```

Memos are pull-based and glitch-free: reading one mid-update gives you a value consistent with
all its sources. You don't need them for cheap derivations (a closure reading two signals is
fine), but they save work when the computation is expensive or when many observers hang off
one derived value.

## Effects and bindings

`Effect::new(f)` runs `f` now and re-runs it when any tracked read changes. Day itself uses the
specialized forms below to wire widgets, and they're available to you:

```rust
// compute (tracked) → apply (untracked), gated by PartialEq on the computed value.
bind(move || count.get() * 2, |doubled| println!("{doubled}"));

// watch: like bind, but you also get the previous value (an Option) and
// nothing runs at setup.
watch(move || route.get(), |new, old| log::info!("{old:?} → {new}"));
```

Every dynamic attribute in Day is one of these underneath. When you write
`label(move || …)`, the build step creates a binding whose apply-side patches one native
widget:

```text
count.set(3)
  │ marks observers dirty, queues their reactions
  ▼
binding for the label re-runs its compute closure   ← the only code that re-runs
  │ new string ≠ old string (PartialEq gate)
  ▼
apply: tree.patch(node, Text("3 clicks"))
  │
  ▼
toolkit.update(handle, patch)   →   NSTextField.stringValue = "3 clicks"
```

Nothing above the label in the tree is visited, and there is no render pass to schedule. The
cost of a state change is proportional to the number of things that observe it.

## Batching and the turn

Writes inside an event handler are batched: the handler runs to completion, then the reactive
graph drains to a fixpoint, then (once per turn) layout runs for whatever became dirty and
native frames are updated. You can batch explicitly too:

```rust
batch(|| {
    first.set("Ada");
    last.set("Lovelace");
});
// Bindings that read both ran once, not twice.
```

The drain is synchronous and ordered (structural changes before attribute updates, outer scopes
before inner). A cycle (an effect that keeps re-dirtying itself) trips a re-run cap rather than
hanging: debug builds panic with the creation site of the offending effect; release builds warn
and defer.

## Scopes: ownership and cleanup

Every signal, memo, effect, and event handler is owned by the `Scope` that was current when it
was created. Day's structural Pieces manage scopes for you: each `when` arm and each `each`/`list`
row gets a child scope, and when that arm or row goes away, disposing the scope tears down
everything it owns: bindings stop firing, handlers are dropped, and the native widgets are
released. There's no unsubscribe bookkeeping to forget.

```text
root scope
 ├─ page scope ("settings")
 │   ├─ binding: title label
 │   └─ when(logged_in) ── arm scope   ← disposed when the condition flips
 │                          ├─ binding: avatar image
 │                          └─ handler: logout tap
 └─ each(todos) row scopes, one per key ← disposed when the row's key disappears
```

Scopes also carry **context**: `with_environment(value, || …)` provides a value that
`environment::<T>()` reads back anywhere below, which is how ambient configuration like theming
works, and how app state is structured. A window's content builds in a scope of its own, so a
`Copy` struct of signal handles provided there is *that window's* state: `Scene::scoped(|s| …)`
to provide it, `Scene::ambient()` to read it back in any piece below, `Scene::focused()` for an
app-wide menu bar whose items belong to no window. [App state](/docs/internal/state) covers the
full model, including why a `thread_local!` is the wrong default even when it looks equivalent.

Two rules follow from scope ownership.

- **A read with no observer never re-runs.** Reading a signal in a plain function body computes
  the value once and forgets it. If you meant "keep this up to date", the read has to be inside a
  binding, memo, or reactive closure.
- **Disposed handles:** writing to a signal whose scope is gone is a defined no-op, warned once
  per call site (normal in async races, where a background task completes after the page
  closed). Reading one panics in every build and names the signal's creation site.

## Threads

The UI, the reactive graph, and the realized tree are single-threaded on the platform's main
thread. `Signal` is `!Send`, so the compiler stops you from moving one into a worker thread.
Two paths lead back to the main thread:

```rust
let progress = Signal::new(0.0);
let set_progress = progress.setter();   // Setter<f64>: Send + Copy, write-only

std::thread::spawn(move || {
    for step in 0..100 {
        // heavy work…
        set_progress.set(step as f64 / 100.0);  // marshals to the main thread
    }
});

// Or run an arbitrary closure on the main thread:
on_main(move || { /* touch signals freely here */ });
```

A `Setter` checks liveness on arrival: if the target scope was disposed while the worker ran,
the write drops silently. That's the behavior you want when a download finishes after its page
was dismissed.

This model makes single-threaded UI code simple and makes the
compiler enforce the threading rule, but there's no shared-state shortcut. Anything computed off
the main thread comes back through a `Setter` or `on_main`, the same way it would with
`DispatchQueue.main.async` or a `Handler`; Day expresses that rule as a type.

## What this model asks of you

The cost of this build-once model is that *you* mark what's dynamic. A closure makes text
live; a bare value doesn't. Structure changes only through `when`, `each`, and `list`. Deriving
structure from a signal in plain Rust freezes it at build time. In diffing frameworks these
distinctions don't exist because everything re-runs; here they're the price of nothing
re-running. The rules are few, the runtime diagnostics catch the common misses, and the payoff
is a UI whose update cost you can reason about line by line.

---

Next: [Layout](/docs/layout), how measured, native-sized widgets end up in the right place.
