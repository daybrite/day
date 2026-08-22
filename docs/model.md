---
title: "Observable model"
description: "day-model's per-property observable store: Store, Keyed, Elem, Field, the Observable derive, and the change log."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# The observable model (`day-model`) — normative

`day-model` is a store whose writes wake only the readers of the field that changed. With one
`Signal<Vec<Item>>`, every observer re-runs when any field of any element changes; `bind`'s
equality gate keeps the native side precise, so what is wasted is compute — every row's closures
re-run and re-clone on every keystroke, and the waste grows with the list. Here each
(element, field) is its own dependency node: editing one item's name re-runs one closure, not a
hundred.

Enable it with the `day` facade's `model` feature:

```toml
day = { version = "0.2", features = ["model"] }
```

`use day::prelude::*` then brings `Store`, `Keyed`, `Elem`, `Field`, `Source`, the
`#[derive(Observable)]` macro, and the `day_model` crate name the derive's generated code
resolves against. The full API is `day::model::*`.

## Declaring a model

```rust
#[derive(Observable, Clone, PartialEq)]
pub struct Item {
    #[obs(key)]
    pub id: u32,
    pub name: String,
    pub done: bool,
    pub address: Address,        // any Observable struct nests
    #[obs(skip)]
    pub cache: Option<Vec<u8>>,  // no accessor, no path, no trigger
}
```

The derive generates an `ItemFields` trait with one typed accessor per field, implemented for
**every** `Source<Item>` — so `store.name()`, `store.elem(id).name()` and
`item.address().city()` all work — plus `Identified` from the `#[obs(key)]` field and an
`Item::OBSERVED_FIELDS` list for tests. Import the trait (`use crate::model::ItemFields;`)
where the accessors are called.

Two rules are deliberate. The key is **always explicit**: a struct that happens to carry an `id`
that is not its key would make inference a trap, and a keyed collection over a struct with no
`#[obs(key)]` is a compile error naming `Identified`. And field ids come from field **names**,
so no hand-assigned index can ever collide.

## The store

```rust
thread_local! {
    static ITEMS: Store<Keyed<Item>> = Store::new(Keyed::default());
}
fn items() -> Store<Keyed<Item>> { ITEMS.with(|s| *s) }
```

A `Store<T>` handle is `Copy` and process-lifetime, like `Signal::global`: created inside
whatever scope first touches it, it does not die with that scope. `Store<Keyed<T>>` is the
collection case:

- `elem(key)` — one element, O(1) by key; a handle whose fields bind controls.
- `keys()` — a **tracked read of the collection's shape** (which keys, in what order). A field
  write does not wake it; an insert, removal or reorder does.
- `restructure(label, op, key, f)` — the structural write. The `Op` (`Insert`/`Delete`/`Move`)
  and the key ride the change log so a persistence layer can tell an insert from a delete; the
  UI, which only re-reads `keys()`, does not care.
- `update(label, f)` — mutate the whole value; every reader wakes. The coarse hammer, right for
  wholesale loads.

## Fields are bindings

`it.name()` is a `Field`: `Copy`, itself a `Source` (so fields nest to any depth, precisely —
a write to `address.city` does not wake a reader of `address.postcode`), and a `Binding`, so
every two-way control takes it directly:

```rust
let it = items().elem(id);
text_field(it.name())                      // read AND write, no draft signal, no watch()
toggle(it.done())
day_piece_datetime::date_picker(it.date().map(date_of, iso_of))   // converted, still two-way
```

`.map(to, from)` converts on both sides with plain `fn`s (an ISO string as a date, `#RRGGBB` as
a `Color`) and the result is still `Copy` and still a binding. Reads track the **most specific
path touched**: `field.with(f)` wakes only for that field, `source.with(f)` is the coarse
subscription that wakes for anything under it. Precision is something a reader opts into by what
it reads — writes never need to know.

**Observation belongs to computations.** A tracked read inside a binding, memo or watch claims
its path for exactly that computation's current run, released when it re-tracks or dies — the
same per-run bookkeeping day-reactive keeps for its own sources. Outside any computation — a
build seeding an initial value, an event handler — a tracked read subscribes nothing and
therefore claims nothing: no trigger is created at all, because nothing could ever wake through
it.

## Driving a list

A store IS a row source ([docs/list.md](list.md)): `list(store, row)` shows the collection in its own
order, and `list(store.rows(projection), row)` orders it through a **key projection** — a
tracked read of key ids that reads only the fields the ORDER depends on. The row builder
receives a `ModelSlot`, itself a `Source`, so the derive's accessors hang off it and follow the
row across cell recycling:

```rust
list(items.rows(model::ordered_keys), |slot: ModelSlot<Item>| {
    row((
        label(move || slot.name().read()),   // wakes only for THIS row's name
        toggle(slot.done()),                 // two-way; follows the recycle
    ))
})
.on_select(|it: Elem<Item>| … )
```

The costs land where they should: a field edit patches the one control showing it (no reload,
no rebind, nothing cloned); a change the projection reads re-runs only the projection and
reloads natively; and a cell scrolled across the whole collection leaves no claims behind —
`day-pieces/tests/model_rows.rs` measures all three.

## Deleted rows

Reading a field of a deleted row returns the field's `Default` — never a panic — and
`elem.exists()` is the **tracked** guard: it re-runs its reader when the row is deleted or comes
back. A write to a gone row is a silent no-op and announces nothing.

```rust
when(move || it.exists(), page(it), gone_notice())
```

## Threads

`Store` is `Send + Sync` (when `T` is). A worker edits through a transaction; the write lock is
the atomicity — a reader never sees half of one:

```rust
let mut tx = store.transact();
tx.data().get_mut(2).unwrap().name = "from a worker".into();
store.elem(2).name().touch(tx.paths());   // name what changed, as portable components
drop(tx);                                  // the drop commits and queues the announcements
```

Announcing is the main thread's job: `store.pump()` wakes exactly the paths the worker named.
The trigger tables are thread-local to the main thread; what crosses the boundary is plain path
components, re-established on arrival.

## The change log

Every write announces `(path components, field label, operation)` — observable headlessly, with
no UI at all:

```rust
let (_, log) = day_model::record(|| {
    store.elem(1).name().write("x".into());
});
assert_eq!(log, vec!["name"]);
```

`record_changes` yields the full `Change` records; `record_values` additionally captures each
write's **prior and new value** (the form an undo unit needs — one clone per write while a
consumer asks, nothing when none does). Where those are scoped test seams,
`install_change_sink(f)` registers a STANDING consumer of every announced change until
`remove_change_sink` — how [day-persistence](persistence.md)'s container watches the stores it
loaded; `store.store_id()` names a store the way a change's first path component does.
`observed_paths()` and `interned_nodes()` expose the cost of observation itself, so a test can
assert that triggers and interner slots are reclaimed when the scopes observing them die.

## Sessions and undo

Two consumers of the change log live here rather than in persistence, because neither needs a
database. **Sessions** are the write-side of `ValueChanged`/`ValueCommitted`:
`field.write_preview(v)` updates the value and wakes this field's readers but records nothing;
`field.write_commit(v)` seals the gesture as one record whose prior is the pre-session value;
`field.session()` adds `cancel()` (Escape restores, zero records). Bound controls drive the
pair automatically. **`UndoStack::new(levels)`** + `stack.watch(store)` turns the same log into
history: units are turns, inversion comes from the captured prior values (a `Delete` carries
its row), replay is tagged `author: "undo"` so consumers can tell it from the user, and
`can_undo`/`undo_label` are signals. `#[derive(Observable)]` emits the `ApplyField` impl replay
writes back through. `day::install_undo(&stack)` fronts the stack natively where the platform
has an undo system ([docs/persistence.md](persistence.md) has the platform table).

### Transient UI state

`stack.set_transient_context(capture, restore)` rides UI state that is not model data — a
selection, a scroll position — along the history. `capture` runs as each unit seals and its
snapshot belongs to that point of history; undo restores the snapshot of the unit history
lands ON (the previous unit's, or the base snapshot taken at install once the stack empties),
and redo restores the redone unit's own. Changing the UI state *between* units records
nothing and restores nowhere: it is not history. That asymmetry is the point — select shape
A, move it, select shape B, move it, undo, and the selection lands on A, the state as it
stood when A's move sealed, not on the transient switch to B. Snapshots live only in the
stack's memory, so nothing persists; and because `capture` runs at seal, a selection write
made in the same turn as its operation (place-then-select) is part of that unit's snapshot.
One rule for `restore`: write plain signals, never a watched store — a store write inside a
restore would fork history from inside a replay.

## Costs, and where they go

- A trigger exists only where something looked; unobserved paths cost nothing to write.
- Triggers are refcounted by observing scope and reclaimed when the last one dies; interner
  slots are refcounted by their triggers and children and reclaimed the same way. A `Copy`
  handle held across a reclamation revalidates and re-interns on its next use.
- Element lookup is O(1): `Keyed` maintains its own key→index map.
- Building a field handle costs no interner lookup; interning is paid once per element handle
  and once per nested struct, not per read.

The scaffold's own editor (`day new`, `src/pages/detail.rs`) is the worked example: each form
control binds a field accessor directly, and the model file's one coarse `watch` is the whole
persistence story. When a coarse watch stops being enough, [docs/persistence.md](persistence.md)
is the next step: the same store, loaded from and autosaved to SQLite.
