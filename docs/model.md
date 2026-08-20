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
consumer asks, nothing when none does). `observed_paths()` and `interned_nodes()` expose the
cost of observation itself, so a test can assert that triggers and interner slots are reclaimed
when the scopes observing them die.

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
persistence story.
