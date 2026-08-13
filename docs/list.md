<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Native `list` (§10)

`list` drives the platform's recycling list (`NSTableView` / `UITableView` /
`RecyclerView` / `GtkListView` / `QListView`), so large collections get native
virtualization, scroll physics, and platform behaviors. It is the one place Day's
"build once, bind forever" model meets cell reuse, and the resolution is the same model:
a row subtree is built once per physical cell and *rebound* (a single slot-write into its
`ItemSlot`) every time that cell is recycled for a new item.

Contrast with [`each`](../crates/day-pieces): `each` builds every row eagerly under one
anchor (great for a dozen items, hopeless for ten thousand). `list` builds only the rows the
native widget currently shows.

## API: shared `ItemSlot` with `each` (DP-16)

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
slot rather than move the item in, so a surviving cell can be fed a new `&T` with one write.

Builder options: `.row_height(RowHeight)`, `.on_select(Fn(K))`, `.multi_select(bool)`,
`.on_selection(Fn(Vec<K>))`, `.selected_rows(Fn() -> Vec<usize>)`, and (reserved) `.row_kind(Fn(&T) -> RowKind)`
mapping to native reuse pools.

### Imperative scroll-to-end (chat timelines)

A chat timeline wants to stick to the newest message. Two additive builder options drive the
native list's own scroller (not a Day-side scroll view):

```rust
let follow = day_reactive::Trigger::new();
list(move || messages.get(), |m| m.id, row_builder)
    .scroll_to_end(follow)   // each `follow.notify()` scrolls so the last row is fully visible
    .stick_to_bottom(true)   // convenience: auto-scroll to end after every data reload
// … after appending a message:
follow.notify();
```

- `.scroll_to_end(Trigger)`: a `watch` on the trigger applies a new `ListPatch::ScrollToEnd`, which
  each backend maps to its native "make the last row visible" call
  (`NSTableView::scrollRowToVisible` · `UITableView::scrollToRowAtIndexPath(.bottom)` ·
  `GtkScrolledWindow` vadjustment→max · `QScrollArea` scrollbar→max ·
  `ListView::smoothScrollToPosition` · XAML `ScrollViewer::ChangeView`). day-core guards the
  empty-list case (no patch is sent), and building the list never auto-scrolls.
- `.stick_to_bottom(bool)`: best-effort convenience that scrolls to the end after each data reload.
  It does not check whether the user is already near the bottom (no cross-backend scroll-position
  read exists yet); for that finer behavior drive `scroll_to_end` from your own logic instead.

## The seam: `ListSource` (native → Day, synchronous)

Recycling lists *pull*: the native data-source asks, synchronously, "how many rows?" and "fill
this cell for row N". Day's normal native→Day path is enqueue-only (`EventSink`), so `list` adds
a second, synchronous seam, injected into the backend the same way the event sink is:

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
and layout before returning, because the host measures the cell synchronously right after.

## The driver (day-core)

Per `LIST` node the tree holds:

- a **row factory** supplied by the `list()` piece (type-erased over `T`): given a row index and a
  cell-anchor `RNode`, it builds the row subtree and returns its `Scope` + root + slot-writer;
- a **snapshot** of the current items + their tokens, refreshed by an effect on the items closure;
- a **cell map**: `RawHandle → BoundRow { anchor, scope, root, slot_writer, token }`.

`list_bind_row(host, index, cell)`:
1. adopt `cell` into a cell-anchor `RNode` (a boundary node whose handle *is* the native cell,
   the same trick the window root uses);
2. if the cell is new, run the row factory (build once); otherwise rebind: one slot-write of
   `items[index]` into the existing row's signal, and update its token;
3. `flush_now` the row scope + lay the row out within the cell bounds, synchronously.

When the items signal changes, the effect refreshes the snapshot and applies a `ListPatch::Reload`
so the native widget re-queries the source. (Fine-grained insert/remove/move batching over the
keyed diff, like `each`'s, is a reserved refinement; `Reload` is the v1 behavior.)

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

1. **spec**: `kinds::LIST`, `ListProps { row_height, selectable, multi_select }`, `RowHeight`, `ListPatch`,
   `ListSource`, `Toolkit::attach_list`. *(additive; no backend breaks)*
2. **pieces**: `list()` + builder, reusing `ItemSlot`; produces the type-erased row factory.
3. **core**: the driver + cell-anchor adoption + `list_bind_row`/`list_len`/reload.
4. **mock**: a simulated viewport + `MockProbe` hooks; e2e tests: only-visible-rows built,
   recycle = slot-write (no rebuild), data change → reload rebinds, `on_select`.
5. **backends**: AppKit first (reference), then UIKit/Android/GTK/Qt; showcase `list` playground +
   walkthrough leg on all five.

## Selection

Rows report selection through two events: `Event::SelectionChanged(row)` (single) and, in
multi-select mode, `Event::SelectionSet(rows)`, the FULL set of selected indices on every
change. `.on_selection(Fn(Vec<K>))` receives the selected keys either way (a single-selection
report arrives as a one-element set), so an app tracking the whole selection works on every
toolkit. `.selected_rows(Fn() -> Vec<usize>)` reactively syncs app state back into the native
selection (`ListPatch::Selected`; empty clears) without a selection-event echo; drive it from
the same signal `on_selection` writes to get a two-way binding and a "clear selection" action.

Support matrix: **AppKit** (native `NSTableView` multi-selection), **Qt** and **XAML** (the
emulated lists: a per-cell press hook, a highlight treatment on the cell's background, ctrl/cmd
toggles, shift extends) honor `multi_select` and `ListPatch::Selected`. The remaining toolkits
report single selection (`SelectionChanged`) and ignore the multi flag and the programmatic
sync; the one-element `on_selection` contract still holds there.

## Programmatic scrolling

`.scroll_to_end(Trigger)` follows the last row (above); `.scroll_to_row(Signal<Option<usize>>)`
jumps to any row: set the signal to `Some(row)` and the native list scrolls it into view,
realizing it if it was virtualized away (`ListPatch::ScrollToRow`, clamped to the count). The
row rail's counterpart to `scroll(...).scroll_target(...)`. Backends without a native
scroll-to-index (GTK ≤ 4.10, Qt, XAML, web) position by uniform row pitch; prefer
`RowHeight::Uniform` when jumping programmatically there.

A Reload whose rows are the SAME set in a new order (a shuffle, a programmatic sort) animates
as native row moves on AppKit (`moveRowAtIndex` batch, the same animation a drag commit gets);
other backends apply it instantly. Inserts, removals, and content changes always reload flat.

## Drag-to-reorder

```rust
list(items, key, row)
    .reorderable(true)
    .on_reorder(|from, to| { /* rotate the backing Vec + persist */ })
    .reorder_guard(|from, to| Reorder::Allow)   // optional: Deny / Retarget(i)
```

`reorderable` turns on the platform's own drag mechanism; probe `Cap::ListReorder` for support.
`on_reorder` is the commit: row `from` landed at row `to`; apply the identical rotation to the
backing data (`let it = v.remove(from); v.insert(to, it);`) and persist it if the order should
survive a relaunch. It runs at the next event drain, never inside the native drop callback.

`reorder_guard` vets every proposed drop **synchronously, while the drag is live**: the native
affordance (the macOS gap, the no-drop cursor) reflects the answer before the user releases.
`Deny` refuses the drop (the row springs back); `Retarget(i)` accepts it at a different index,
the "pinned rows" pattern (the Showcase pins its first row this way). Keep the guard pure: it
runs inside the platform's drag callback, so read state and return: no UI mutation.

The seam is the reorder half of `ListSource` (`ListSource::reorder`, present only when
`.reorderable()`): `can_move(from, proposed) -> accepted-index-or--1` for the live verdict, and
`move_row(from, to)` for the commit, which rotates Day's row snapshot **before returning**, so
`len`/`token_at`/`bind_row` answer in the new order while the native move animates, and defers
the app's `on_reorder` through the event queue. When the app's own data change echoes back with
exactly the committed token order, the piece skips the redundant `Reload` (no post-drop flicker).

The dayscript step `reorder: { id, from, to }` drives the same guard → commit path without a
native gesture (a guard denial fails the step, non-retryably); that is how CI asserts reordering
on every target.

Per-backend affordances:

| Backend | Mechanism | Affordance | Guard |
|---|---|---|---|
| AppKit | `NSTableView` drag pipeline (pasteboard row, `validateDrop`/`acceptDrop`) | the `.gap` placeholder opens where the drop would land | live (validate retargets/denies) |
| UIKit | drag delegate + `moveRow`/`targetIndexPathForMove` | long-press lift + gap, no editing mode | live (target-for-move) |
| Android | `ItemTouchHelper` on the RecyclerView | long-press lift, elevation, incremental swaps | live per swap; `Retarget` reads as deny (the helper can't relocate the gap) |
| GTK 4 | `DragSource`/`DropTarget` (native DnD framework) | row snapshot as drag icon; forbidden cursor on deny (no insertion line yet) | live (motion) |
| Qt | `QDrag` over the emulated list | grabbed-cell pixmap, 2px insertion line, no-drop cursor | live (drag-move) |
| XAML | WinRT `CanDrag`/`DragOver`/`Drop` over the emulated list | system drag visuals + live no-drop cursor | live (DragOver) |
| ArkUI | `SetNodeDraggable` + `NODE_ON_DROP` | system drag preview; denied drops spring back | at drop (`SetDragResult`) |
| web-dom | pointer-tracked (emulated — no native list reorder in the browser) | lifted cell + animated CSS gap, long-press on touch | live (every hovered slot) |
| mock | `MockProbe::list_can_move` / `list_move` | op log | live |

## Swipe-to-delete

```rust
list(items, key, row)
    .deletable(true)
    .delete_label(res::str::delete().format())   // optional: the word on the affordance
    .on_delete(|index| { /* remove from the backing Vec + persist */ })
    .delete_guard(|index| index != 0)            // optional: protect individual rows
```

`deletable` turns on the platform's own delete gesture; probe `Cap::ListDelete` for support.
`on_delete` is the commit: row `index` is gone; apply the identical removal to the backing data
(`v.remove(index)`) and persist it if the change should survive a relaunch. It runs at the next
event drain, never inside the native swipe callback.

`delete_guard` is consulted **before the affordance is offered**, not after the gesture: a row
that answers `false` shows no delete action at all, rather than one that fails on use. Keep it
pure — it runs inside the platform's swipe callback.

`delete_label` carries the app's own word for the action, already localized. A toolkit has no
access to the app's Fluent catalog and so cannot translate anything itself; left unset, each
backend falls back to its platform's wordless idiom (a trash glyph), which is honest in every
language rather than shipping one language's word everywhere.

The seam is the delete half of `ListSource` (`ListSource::delete`, present only when
`.deletable()`): `can_delete(index) -> bool` for the offer, and `delete_row(index)` for the
commit, which drops the row from Day's snapshot **before returning**, so `len`/`token_at`/
`bind_row` answer for the shorter list while the native removal animates, and defers the app's
`on_delete` through the event queue — the same discipline the reorder half follows.

**The desktop toolkits answer `Unsupported`**: none of them has a swipe idiom, and inventing one
would be a Day invention rather than a platform's. A list that must be editable everywhere pairs
`.deletable()` with an explicit control — a menu item, a per-row button — and lets the mobile
toolkits add the gesture on top.

The dayscript step `delete_row: { id, row }` drives the same guard → commit path without a
native gesture (a guard refusal fails the step, non-retryably), which is how CI asserts deletion
on every target — including the desktops, where there is no gesture to simulate.

| Backend | Mechanism | Affordance |
|---|---|---|
| UIKit | `trailingSwipeActionsConfigurationForRowAtIndexPath` | row tracks the finger, destructive action reveals behind it, full swipe commits |
| Android | `ItemTouchHelper` swipe (`START`, so it is the trailing edge in RTL too) | row slides to reveal a red field carrying the label; `notifyItemRemoved` animates the close-up |
| AppKit · GTK · Qt · XAML · web | — | `Unsupported`: no swipe idiom; use a menu item or a row button |
| ArkUI | `NODE_LIST_ITEM_SWIPE_ACTION` | *not yet implemented* — the NDK exposes it; the shim does not build it yet |


Rows drag within their own list only; nothing is draggable out of the app. `RowHeight::Automatic`
lists compute the drop slot from a uniform-pitch approximation on GTK/Qt/XAML/ArkUI; prefer
`Uniform` heights for reorderable lists there.
