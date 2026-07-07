# Native `list` (§10)

`list` drives the platform's **recycling** list — `NSTableView` / `UITableView` /
`RecyclerView` / `GtkListView` / `QListView` — so large collections get native
virtualization, scroll physics, and platform behaviours. It is the one place Day's
"build once, bind forever" model meets cell reuse, and the resolution is exactly that model:
**a row subtree is built once per physical cell and *rebound* — a single slot-write into its
`ItemSlot` — every time that cell is recycled for a new item.**

Contrast with [`each`](../crates/day-pieces): `each` builds *every* row eagerly under one
anchor (great for a dozen items, hopeless for ten thousand). `list` builds only the rows the
native widget currently shows.

## API — shared `ItemSlot` with `each` (DP-16)

```rust
list(move || messages.get(), |m| m.id, move |row: ItemSlot<Message, u64>| {
    column((
        label(move || row.field(|m| m.sender.clone())),
        label(move || row.field(|m| m.preview.clone())),
    ))
})
.row_height(RowHeight::Uniform(56.0))   // or ::Automatic (self-sizing, slower)
.on_select(move |key| open(key))
.id("inbox")
```

The row builder receives the same `ItemSlot<T, K>` as `each` (Copy handle, tracked `get()`,
memoised `field()` projections). Because cells are recycled, the builder must read through the
slot — never move the item in — so a surviving cell can be fed a new `&T` with one write.

Builder options: `.row_height(RowHeight)`, `.on_select(Fn(K))`, and (reserved) `.row_kind(Fn(&T) -> RowKind)`
mapping to native reuse pools.

### Imperative scroll-to-end (chat timelines)

A chat timeline wants to *stick to the newest message*. Two additive builder options drive the
native list's own scroller (never a Day-side scroll view):

```rust
let follow = day_reactive::Trigger::new();
list(move || messages.get(), |m| m.id, row_builder)
    .scroll_to_end(follow)   // each `follow.notify()` scrolls so the LAST row is fully visible
    .stick_to_bottom(true)   // convenience: auto-scroll to end after every data reload
// … after appending a message:
follow.notify();
```

- `.scroll_to_end(Trigger)` — a `watch` on the trigger applies a new `ListPatch::ScrollToEnd`, which
  each backend maps to its native "make the last row visible" call
  (`NSTableView::scrollRowToVisible` · `UITableView::scrollToRowAtIndexPath(.bottom)` ·
  `GtkScrolledWindow` vadjustment→max · `QScrollArea` scrollbar→max ·
  `ListView::smoothScrollToPosition` · WinUI `ScrollViewer::ChangeView`). day-core guards the
  **empty-list** case (no patch is sent), and building the list never auto-scrolls.
- `.stick_to_bottom(bool)` — best-effort convenience that scrolls to the end after each data reload.
  It does *not* check whether the user is already near the bottom (no cross-backend scroll-position
  read exists yet); for that finer behaviour drive `scroll_to_end` from your own logic instead.

## The seam — `ListSource` (native → Day, synchronous)

Recycling lists *pull*: the native data-source asks, synchronously, "how many rows?" and "fill
this cell for row N". Day's normal native→Day path is enqueue-only (`EventSink`), so `list` adds
a second, synchronous seam — injected into the backend the same way the event sink is:

```rust
// day-spec
pub struct ListSource {
    pub len: Rc<dyn Fn() -> usize>,
    pub token_at: Rc<dyn Fn(usize) -> u64>,     // stable per-row identity for the native widget
    pub bind_row: Rc<dyn Fn(usize, RawHandle)>, // build-or-rebind row `i` into this native cell
    pub recycle: Rc<dyn Fn(RawHandle)>,         // cell leaving the viewport (optional bookkeeping)
}

trait Toolkit {
    // default no-op; a recycling backend stores the source and calls it from its data-source.
    fn attach_list(&mut self, _host: &Self::Handle, _source: ListSource) {}
}
```

day-core builds the `ListSource` when it realises a `LIST` node; each closure re-enters the tree
via `with_tree(...)`. The backend calls them on the UI thread from *outside* any `with_tree`
borrow (a fresh native scroll callback), so the re-entry is safe.

`bind_row` is the sanctioned exception to turn-batching (§3.3): it runs the row's reactive flush
and layout **before returning**, because the host measures the cell synchronously right after.

## The driver (day-core)

Per `LIST` node the tree holds:

- a **row factory** supplied by the `list()` piece (type-erased over `T`): given a row index and a
  *cell-anchor* `RNode`, it builds the row subtree and returns its `Scope` + root + slot-writer;
- a **snapshot** of the current items + their tokens, refreshed by an effect on the items closure;
- a **cell map**: `RawHandle → BoundRow { anchor, scope, root, slot_writer, token }`.

`list_bind_row(host, index, cell)`:
1. adopt `cell` into a cell-anchor `RNode` (a boundary node whose handle *is* the native cell —
   the same trick the window root uses);
2. if the cell is new, run the row factory (build once); otherwise **rebind** — one slot-write of
   `items[index]` into the existing row's signal, and update its token;
3. `flush_now` the row scope + lay the row out within the cell bounds, synchronously.

When the items signal changes, the effect refreshes the snapshot and applies a `ListPatch::Reload`
so the native widget re-queries the source. (Fine-grained insert/remove/move batching over the
keyed diff — like `each`'s — is a reserved refinement; `Reload` is the honest v1.)

## Per-backend mapping

| Backend | Widget | Recycling | Notes |
|---|---|---|---|
| mock    | simulated viewport | yes (test-driven) | `MockProbe::scroll_list(range)` drives binds; proves the driver |
| AppKit  | `NSTableView` (view-based) | native | `makeView`/`viewFor` → `bind_row`; `numberOfRows` → `len` |
| UIKit   | `UITableView` + reuse id | native | `cellForRowAt` → `bind_row` |
| Android | `RecyclerView` + `Adapter` | native | `onBindViewHolder` → `bind_row` |
| GTK 4   | `GtkListView` + `GtkListItemFactory` | native | factory `bind`/`unbind` → `bind_row`/`recycle` |
| Qt      | `QListView` + abstract model, or delegate | emulated (Cap reports `Emulated`, DP-19) | model `rowCount`/`data` |

## Building it (mock-first, like M0–M1)

1. **spec** — `kinds::LIST`, `ListProps { row_height, selectable }`, `RowHeight`, `ListPatch`,
   `ListSource`, `Toolkit::attach_list`. *(additive; no backend breaks)*
2. **pieces** — `list()` + builder, reusing `ItemSlot`; produces the type-erased row factory.
3. **core** — the driver + cell-anchor adoption + `list_bind_row`/`list_len`/reload.
4. **mock** — a simulated viewport + `MockProbe` hooks; **e2e tests**: only-visible-rows built,
   recycle = slot-write (no rebuild), data change → reload rebinds, `on_select`.
5. **backends** — AppKit first (reference), then UIKit/Android/GTK/Qt; showcase `list` playground +
   walkthrough leg on all five.
