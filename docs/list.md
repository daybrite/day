---
navTitle: "Lists"
title: "Native list"
description: "The list piece over native table and collection views: virtualized rows, selection, swipe actions, reorder, and editing mode."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Native [`list`](https://daybrite.dev/api/day_pieces/fn.list.html)

Display large collections as scrollable rows without building every row at once. A [`list`](https://daybrite.dev/api/day_pieces/fn.list.html)
reuses native cells as they enter and leave the viewport.

[`list`](https://daybrite.dev/api/day_pieces/fn.list.html) drives the platform's recycling list ([`NSTableView`](https://developer.apple.com/documentation/appkit/nstableview) / [`UITableView`](https://developer.apple.com/documentation/uikit/uitableview) /
[`RecyclerView`](https://developer.android.com/reference/androidx/recyclerview/widget/RecyclerView) / [`GtkListView`](https://docs.gtk.org/gtk4/class.ListView.html) / [`QListView`](https://doc.qt.io/qt-6/qlistview.html) / WinUI [`ItemsRepeater`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.itemsrepeater)), so large collections get native
virtualization, scroll physics, and platform behaviors. It is the one place Day's
build-once model meets cell reuse, and the resolution is the same model: a row subtree is
built once per physical cell and *rebound* (a single slot-write into its `ItemSlot`) every
time that cell is recycled for a new item.

Contrast with [`each`](../crates/day-pieces): [`each`](https://daybrite.dev/api/day_pieces/fn.each.html) builds every row eagerly under one
anchor (fine for a dozen items; ten thousand would all be built up front). [`list`](https://daybrite.dev/api/day_pieces/fn.list.html) builds only
the rows the native widget currently shows.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](/docs/how-to/localization).

## API: row sources, shared with [`each`](https://daybrite.dev/api/day_pieces/fn.each.html)

On **windows-winui**, [`ItemsRepeater`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.itemsrepeater) and `StackLayout` own realization and scroll extents
inside a native [`ScrollViewer`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.scrollviewer). Its element factory reuses `ItemContainer` rows with Day
content; it does not allocate a cell per source index. Selection, keyboard navigation,
drag-and-drop, and content-sized rows follow the realized containers. A [`SwipeControl`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.swipecontrol)
offers declared leading/trailing actions and guarded deletion, with the same commands in
a native row context menu for mouse and keyboard use. Row content can still declare its
own context menu. Action callbacks validate the row token and cell generation before use.

`list-recycles-distant-rows` checks that distant jumps keep a bounded number of rows alive and rebind their actions; `list-releases-discarded-rows` churns a list by jumps, shrinks, regrowth and reorders and checks the same bound;
`list-shrink-and-regrow` checks emptying and repopulating a scrolled list. Run them with the
other list cases using `day test --project apps/conformance -t windows-winui 'list-*'`.

[`list(source, row)`](https://daybrite.dev/api/day_pieces/fn.list.html) and [`each(source, row)`](https://daybrite.dev/api/day_pieces/fn.each.html) take a **`RowSource`** (where the rows come from)
and one row builder. Two sources ship:

**Plain data**: an items closure and a key function, wrapped in [`items(…)`](https://daybrite.dev/api/day_pieces/fn.items.html):

```rust
list(
    items(move || messages.get(), |m| m.id),
    move |row: ItemSlot<Message, u64>| {
        column((
            label(move || row.field(|m| m.sender.clone())),
            label(move || row.field(|m| m.preview.clone())),
        ))
        .id_of(move || id!(message_row, row.key()))
    },
)
.row_height(RowHeight::Automatic) // the default: each row as tall as its content
.on_select(move |key| open(key))
.id(id!(inbox))
```

> [!TIP]
> **A recycled row needs a reactive ID.** Use [`.id_of`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id_of)`(move || id!(message_row, row`[`.key`](https://daybrite.dev/api/day_pieces/struct.ItemSlot.html#method.key)`()))`
> so the ID follows the item occupying the cell. An eager [`.id`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id)`(id!(message_row, row`[`.key`](https://daybrite.dev/api/day_pieces/struct.ItemSlot.html#method.key)`()))`
> would retain its first item's ID. Sender names and message previews are user content;
> app-owned actions and empty states belong in the [Fluent catalog](/docs/how-to/localization).

The row builder receives an `ItemSlot<T, K>` (Copy handle, tracked [`get()`](https://daybrite.dev/api/day_pieces/struct.ItemSlot.html#method.get), memoised [`field()`](https://daybrite.dev/api/day_pieces/struct.ItemSlot.html#method.field)
projections). Because cells are recycled, the builder must read through the slot rather than
move the item in, so a surviving cell can be fed a new `&T` with one write.

**A [day-model](/docs/explanation/model) store** (feature `model`; [docs/model](model.md)): passed directly for collection order,
or through `store.rows(projection)` for a display order/filter expressed as a tracked key
projection:

```rust
list(store.rows(model::ordered_keys), |slot: ModelSlot<Item>| {
    row((
        label(move || slot.name().read()),   // wakes only for THIS row's name
        toggle(slot.done()),                 // two-way; follows the cell across recycles
    ))
})
.on_select(|it: Elem<Item>| open(it.key()))
```

`ModelSlot` is itself a [day-model](/docs/explanation/model) `Source`, so the derive's field accessors hang off it and
resolve the current row on every operation; a control bound once at build keeps working as
the cell recycles. The costs stay local: a field edit patches the one control showing it (the
row set is unchanged, so the native reload is skipped and nothing is cloned); a change the
projection reads re-runs only the projection; and a cell scrolled
across the whole collection leaves no observation claims behind
(`day-pieces/tests/model_rows.rs` measures all three).

Builder options: [`.row_height(RowHeight)`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.row_height), [`.on_select(Fn(Ref))`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.on_select) (the key for a plain source,
the row's `Elem` for a store source), [`.multi_select()`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.multi_select), [`.on_selection(Fn(Vec<Ref>))`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.on_selection),
[`.selected_rows(Fn() -> Vec<usize>)`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.selected_rows), [`.selection(binding)`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.selection) (store sources, below), and
(reserved, unshipped) `.row_kind` mapping to native reuse pools.

### Asynchronous row images

Android row binding can reenter during a DayFixed measurement while the Day tree is busy.
The adapter retains the cell and posts a deferred bind with a row-token/lifetime check;
RecyclerView otherwise keeps the empty holder as already bound. This matters for lists
first opened in a cover, before any later search or reload has occurred.

Keep the image view mounted and bind its source to the current item's loaded image or a
bundled placeholder. When a cell is rebound, cancel its previous request and associate the
result with the requested URL or item key. Check that identity before displaying a result;
an older request must not populate a newer row. Clear or replace the previous pixels before
waiting for the network, and dispose the task with the row's scope.

A two-axis [`.frame(width, height)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame) keeps rows stable while loading. It is a measurement
boundary, not a placement barrier: both image-source updates and conditional children inside
it are laid out when they change, including in detached native list and tree cells.
List cells parked by `recycle` retain their contents and pending layout dirt, but skip both
width-change layout and dirty-cell sweeps. `bind_row` reactivates and lays them out before
redisplay. Resize work therefore follows the active viewport rather than every physical
cell accumulated in the native reuse pool.

### Imperative scroll-to-end (chat timelines)

A chat timeline wants to stick to the newest message. Two additive builder options drive the
native list's own scroller (not a Day-side scroll view):

```rust
let follow = day_reactive::Trigger::new();
list(items(move || messages.get(), |m| m.id), row_builder)
    .scroll_to_end(follow)   // each `follow.notify()` scrolls so the last row is fully visible
    .stick_to_bottom()       // convenience: auto-scroll to end after every data reload
// … after appending a message:
follow.notify();
```

- [`.scroll_to_end(Trigger)`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.scroll_to_end): a [`watch`](https://daybrite.dev/api/day_reactive/fn.watch.html) on the trigger applies a new `ListPatch::ScrollToEnd`, which
  each toolkit maps to its native "make the last row visible" call
  ([`NSTableView::scrollRowToVisible`](https://developer.apple.com/documentation/appkit/nstableview) · [`UITableView::scrollToRowAtIndexPath(.bottom)`](https://developer.apple.com/documentation/uikit/uitableview) ·
  [`GtkScrolledWindow`](https://docs.gtk.org/gtk4/class.ScrolledWindow.html) vadjustment→max · [`QScrollArea`](https://doc.qt.io/qt-6/qscrollarea.html) scrollbar→max ·
  `ListView::smoothScrollToPosition`). day-core guards the
  empty-list case (no patch is sent), and building the list never auto-scrolls.
- [`.stick_to_bottom()`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.stick_to_bottom): best-effort convenience that scrolls to the end after each data reload.
  It does not check whether the user is already near the bottom (no cross-toolkit scroll-position
  read exists yet); for that finer behavior drive [`scroll_to_end`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.scroll_to_end) from your own logic instead.

### Sections

[`.sections(key, header)`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.sections)
groups the rows under section headers. `key` runs with each row index when the rows change and
names that row's section. A header starts every run of rows with the same key, so sort the rows
by section first. `header` builds the header from the slot of the section's first row, the same
slot a row builder gets.

```rust
list(items(move || articles.get(), |a: &Article| a.id), article_row)
    .sections(
        move |i| articles.with(|a| a[i].feed_id),
        |slot| label(move || slot.field(|a| a.feed_title.clone())).font(Font::Footnote),
    )
```

Headers are list rows of their own, drawn in the platform's group-header style. The header of
the section under the top edge stays pinned there while its rows scroll beneath it, and the next
header pushes it up.

- A header is never selected, activated or swiped, and keyboard navigation steps over it.
- Row indices everywhere count rows only: selection reports, `selected_rows`, `scroll_to_row`,
  swipe offers, `first_visible_row` and dayscript's `select`, `swipe_row`, `reorder` and
  `delete_row`.
- A header sizes to its content, even in a [`Uniform`](https://daybrite.dev/api/day_spec/props/enum.RowHeight.html)
  list, and takes the platform's side insets but not its vertical insets or minimum row height.
- A row drags only within its own section, the rule
  [`UITableView`](https://developer.apple.com/documentation/uikit/uitableviewdelegate/tableview(_:targetindexpathformoveatindexpath:toproposedindexpath:))
  apps follow for grouped tables. A header never lifts, and a drop past either end of the row's
  section lands at that end, through the same retarget the
  [`reorderable`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.reorderable)
  guard uses. The guard sees row indices, and dayscript's `reorder` keeps to the same rule. A
  move between sections is a change to the row's data: update its section key, and the list
  regroups.
- A sectioned list offers no native swipe-to-delete, because deleting a section's last row
  takes its header with it. Swipe actions still work, and dayscript's `delete_row` still drives
  the app's handler.

`Cap::ListSections` says how a toolkit pins the header:

| Toolkit | Header rows | Pinned header |
|---|---|---|
| AppKit | [`NSTableView`](https://developer.apple.com/documentation/appkit/nstableview) group rows (`tableView(_:isGroupRow:)`), refused by `tableView(_:shouldSelectRow:)` | native: [`floatsGroupRows`](https://developer.apple.com/documentation/appkit/nstableview/floatsgrouprows) |
| UIKit | one table section per header; the header is the section's [`UITableViewHeaderFooterView`](https://developer.apple.com/documentation/uikit/uitableviewheaderfooterview) | native: a plain-style table pins section headers |
| Android | a `RecyclerView` view type of its own, with no click, ripple or focus | emulated: a copy in the list's overlay |
| GTK 4 | model items marked not selectable, activatable or focusable | emulated: a copy in a [`GtkOverlay`](https://docs.gtk.org/gtk4/class.Overlay.html) over the scrolled window |
| Qt | model rows without `ItemIsEnabled` or `ItemIsSelectable`, painted as a band by the delegate | emulated: a copy over the viewport |
| WinUI | cells in a `Border` of their own, without the `ItemContainer` rows get, so they take no selection, hover or focus | emulated: a copy over the `ScrollContentPresenter`, under the scroll bars |
| web | cells with `role="heading"` that take no clicks | emulated: a copy in a `position: sticky` wrapper |
| ArkUI | `ListItem`s with no click, drag or swipe action | emulated: a copy stacked over the list |

Where the pin is emulated, the host binds the copy through `ListSource::bind_pinned`. The copy
carries no element ids, so a dayscript lookup of an id inside a header finds the header row in
the list, never its pinned copy.

Inside day-core the host sees display rows, headers included: `ListSource::is_header` says which,
and `len`, `token_at`, `bind_row`, the selection and scroll patches and the toolkit's selection
events all count headers. day-core translates between display rows and the app's rows, so a
toolkit needs no section logic beyond drawing headers and keeping them out of selection.

### First visible row

[`.first_visible_row(Signal<usize>)`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.first_visible_row)
receives the first actually visible row, excluding overscan and counting rows only (a header
reports its section's first row). It is opt-in and event-driven: native callbacks enqueue a
deduplicated report, and the event pump updates the signal when the tree is available (a report
during layout waits for the current borrow to end). AppKit, UIKit, Android, GTK, Qt, WinUI, DOM
and ArkUI report scroll changes; mock exposes `list_first_visible` for tests. The initial index
is zero; clamp it when data shrinks. For section headers that float, use
[`.sections`](#sections) instead of an overlay driven by this index.

## `ListSource`: how the toolkit pulls rows

Recycling lists *pull*: the native data-source asks, synchronously, "how many rows?" and "fill
this cell for row N". Day's normal native→Day path is enqueue-only (`EventSink`), so [`list`](https://daybrite.dev/api/day_pieces/fn.list.html) adds
a second, synchronous channel, `ListSource`, injected into the toolkit the same way the event
sink is:

```rust
// day-spec
pub struct ListSource {
    pub first_visible: Option<Rc<dyn Fn(usize)>>, // opt-in enqueue-only viewport report
    pub len: Rc<dyn Fn() -> usize>,
    pub token_at: Rc<dyn Fn(usize) -> u64>,     // stable per-row identity for the native widget
    pub bind_row: Rc<dyn Fn(usize, RawHandle)>, // build-or-rebind row `i` into this native cell
    pub recycle: Rc<dyn Fn(RawHandle)>,         // cell leaving the viewport: its ids park until the next bind
    pub release: Rc<dyn Fn(RawHandle)>,         // host discarded the cell for good: its row is disposed
    pub layout_cell: Rc<dyn Fn(RawHandle, f64)>, // re-lay that cell's row at the width the platform gave it
    pub cell_height: Rc<dyn Fn(RawHandle) -> f64>, // the row's laid height, padding included (a self-measuring cell's height)
}

trait Toolkit {
    // default no-op; a recycling backend stores the source and calls it from its data-source.
    fn attach_list(&mut self, _host: &Self::Handle, _source: ListSource) {}
}
```

day-core builds the `ListSource` when it realizes a `LIST` node; each closure re-enters the tree
via [`with_tree(...)`](https://daybrite.dev/api/day_core/fn.with_tree.html). The toolkit calls them on the UI thread from *outside* any [`with_tree`](https://daybrite.dev/api/day_core/fn.with_tree.html)
borrow (a fresh native scroll callback), so the re-entry is safe.

`bind_row` is the sanctioned exception to turn-batching: it runs the row's reactive flush
and layout before returning, because the host measures the cell synchronously right after.

## The driver (day-core)

Per `LIST` node the tree holds:

- a **row factory** supplied by the [`list()`](https://daybrite.dev/api/day_pieces/fn.list.html) piece (type-erased over `T`): given a row index and a
  cell-anchor `RNode`, it builds the row subtree and returns a `BuiltRow`: the row's `Scope` and a
  rebind writer that slot-writes another index into the row's `ItemSlot`;
- a **snapshot** of the current items + their tokens, refreshed by an effect on the items closure;
- a **cell map**: native cell → `BoundCell { anchor, active, scope, rebind, native_width,
  laid_width, laid_height }`: the row each physical cell holds, whether it is on display, and
  the width and height it was last laid at.

`list_bind_row(host, index, cell)`:
1. adopt `cell` into a cell-anchor `RNode` (a boundary node whose handle *is* the native cell,
   the same trick the window root uses);
2. if the cell is new, run the row factory (build once); otherwise rebind: restore the row's
   parked element ids and slot-write `items[index]` into the existing row's signal;
3. flush the row scope and lay the row out within the cell bounds, synchronously; the host reads
   `cell_height` right after.

`recycle(cell)` parks the row's element ids and marks the cell inactive; its laid height is
forgotten, so the next row it shows is sized afresh rather than reported as a change. A later
`bind_row` of the same cell rebinds instead of building.

`release(cell)` is the host's goodbye: it discarded the cell for good (GTK's factory
`teardown`, a [`UITableViewCell`](https://developer.apple.com/documentation/uikit/uitableviewcell) deallocating), so Day disposes the row built there, scope,
nodes and ids, exactly as it does for every cell when the list itself goes. Pools discard
cells on a far jump ([UITableView](https://developer.apple.com/documentation/uikit/uitableview) handed back 465 fresh cells over one run of the recycling
case, [GtkListView](https://docs.gtk.org/gtk4/class.ListView.html) 244), and without this call each discarded cell left one dead row behind. The call is safe inside a day-core borrow (a host often tears cells down from inside
a reload or a removal Day started): the work is held for the borrow's end, never dropped. A
handle seen again after its release is a new cell and builds afresh.

When the items signal changes, the effect refreshes the snapshot and applies a `ListPatch::Reload`
so the native widget re-queries the source. A source connection that reports its own row events
(a [day-persistence](/docs/reference/persistence) query, for one) sends `ListPatch::Splice(Vec<RowDelta>)` instead: sequential
`Insert`, `Remove` and `Move` deltas the host applies as native row changes, with the selection
following its rows. A plain items closure has no such events and reloads.

## Per-toolkit mapping

Platform guidance: [Apple: lists and tables](https://developer.apple.com/design/human-interface-guidelines/lists-and-tables); [Android: RecyclerView](https://developer.android.com/develop/ui/views/layout/recyclerview).

| Toolkit | Widget | Recycling | Notes |
|---|---|---|---|
| mock    | simulated viewport | yes (test-driven) | [`MockProbe::list_bind`](https://daybrite.dev/api/day_mock/struct.MockProbe.html#method.list_bind)/`list_recycle`/`list_release` drive binds, pooling and discards; exercises the driver |
| AppKit  | [`NSTableView`](https://developer.apple.com/documentation/appkit/nstableview) (view-based) | native | `makeView`/`viewFor` → `bind_row`; `numberOfRows` → `len` |
| UIKit   | [`UITableView`](https://developer.apple.com/documentation/uikit/uitableview) + reuse id | native | `cellForRowAt` → `bind_row`; `didEndDisplaying` → `recycle`; the cell's `dealloc` → `release` (the table discards surplus cells as well as reusing them) |
| Android | [`RecyclerView`](https://developer.android.com/reference/androidx/recyclerview/widget/RecyclerView) + `Adapter` | native | `onBindViewHolder` → `bind_row` |
| GTK 4   | [`GtkListView`](https://docs.gtk.org/gtk4/class.ListView.html) + [`GtkListItemFactory`](https://docs.gtk.org/gtk4/class.ListItemFactory.html); each cell a `DayCell`, a custom container that asks for no width and lays the row out at the width the themed `row` node grants it | native | factory `bind`/`unbind` → `bind_row`/`recycle`, `teardown` → `release`; the cell's `size_allocate` → `layout_cell`. [GtkListView](https://docs.gtk.org/gtk4/class.ListView.html) keeps up to 200 spare items by design and rebinds them, so a five-row viewport settles at about 205 rows alive; items beyond that are torn down and released. A `Reload` splices only the row-count difference and re-binds the bound cells in place, top to bottom: replacing every model item would make [GtkListView](https://docs.gtk.org/gtk4/class.ListView.html) destroy each row widget and build a second set |
| Qt      | [`QListView`](https://doc.qt.io/qt-6/qlistview.html) over a count-only [`QAbstractListModel`](https://doc.qt.io/qt-6/qabstractlistmodel.html) and a delegate that paints only the item chrome; each cell a plain [`QWidget`](https://doc.qt.io/qt-6/qwidget.html) placed over its row as a viewport child | native — a bounded pool (the viewport plus overscan) the Rust side retargets on every window change: a row scrolling out parks its cell and `recycle`s it, a row scrolling in takes a spare and `bind_row` rebinds it | the view owns selection, keys, focus, drag; Day owns the rows. Cells are never index widgets: the view deletes those on removal. Every Qt callback into Day is posted |
| ArkUI   | `List` + `NodeAdapter`; each cell a `ListItem` holding the row | native | adapter attach/detach → `bind_row`/`recycle`; item ids carry a generation so a reload never rebinds a stale cell |
| WinUI   | [`ItemsRepeater`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.itemsrepeater) + `StackLayout`; each cell an `ItemContainer` holding a `Canvas` for the row | native | the repeater's realization callbacks post `bind_row`/`recycle` (calling the source inside WinUI layout would re-enter Day) |
| web     | a DOM scroller with a slot per row | emulated | rows in view plus an overscan are built; a row scrolled far away keeps its element |

## Row metrics

A row reads at the platform's size without the app padding it. day-core lays each row inside
the margins the toolkit's own list gives a row and keeps it at least that list's row height,
content centered ([`Toolkit::list_row_metrics`](https://daybrite.dev/api/day_spec/trait.Toolkit.html#method.list_row_metrics), asked of the list when a cell is first used), so
a row piece holds only its content: [`row((icon, label))`](https://daybrite.dev/api/day_pieces/fn.row.html), not [`row`](https://daybrite.dev/api/day_pieces/fn.row.html)`(..)`[`.padding`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.padding)`(..)`.

| Toolkit | Margins (top, sides) | Minimum height | Source |
|---|---|---|---|
| UIKit | a stock cell's top margin; the table's layout margins at the sides (16 or 20pt) | the height UIKit fits a stock cell with one line of its default content configuration | measured from UIKit at runtime |
| Android | 8dp, 16dp | 56dp | Material one-line list item |
| AppKit | 4pt, 8pt | 28pt | macOS table row |
| GTK | 8px, 12px | 44px | Adwaita rich-list row |
| Qt | 4px, 8px | 28px | item view row |
| web | 10px, 16px | 44px | a touch-sized web row |
| ArkUI | 8vp, 12vp | 48vp | HarmonyOS list item |
| WinUI | 0, 12px | 40px | `ListViewItem` padding and `MinHeight` |

A `Uniform` row height still wins: the row is that tall and its content sits inside the margins,
or, where the row is too short for them, centered in the whole row.

## List items

[`list_item(title)`](https://daybrite.dev/api/day_pieces/fn.list_item.html) is a row's standard content drawn as the platform draws its own list rows:
a leading icon, the title, an optional subtitle and trailing detail, and a check mark. Every part
takes a constant, a signal or a closure, and changes in place, so a recycled row follows its item.

```rust
list(scene.items.rows(keys), move |slot| {
    list_item(move || slot.name().read())
        .icon(move || slot.kind().read().icon())     // a resource/vectors glyph, as a template
        .icon_tint(move || slot.tint().read().color())
        .subtitle(move || slot.note().read())
        .detail(move || slot.count().read().to_string())
        .checked(move || slot.done().read())         // the platform's check mark, announced as selected
})
```

Where `Cap::ListItem` answers `Native` the toolkit realizes `kinds::LIST_ITEM` itself, and the
row takes the platform's fonts, icon size, spacing, secondary colors and check mark:

| Toolkit | Realized as |
|---|---|
| UIKit | [`UIListContentView`](https://developer.apple.com/documentation/uikit/uilistcontentview) with the cell's default content configuration; the checkmark accessory's glyph and a secondary value beside it |
| Android | a Material 3 list item (`DayListItem`): 24dp leading icon, body-large headline, body-medium supporting text, label-small trailing text, a primary-colored check; 56dp, 72dp with a supporting line |
| AppKit | an image-and-text table cell's parts: a 16pt template image, the system font, a smaller secondary subtitle, the accent checkmark; at least 28pt |
| GTK | a row box: a 16px icon, the title over a dim caption, a dim value, `object-select-symbolic` in the accent color; at least 44px |
| Qt, web, ArkUI, WinUI | composed from [`vector`](https://daybrite.dev/api/day_pieces/fn.vector.html), [`label`](https://daybrite.dev/api/day_pieces/fn.label.html) and [`image`](https://daybrite.dev/api/day_pieces/fn.image.html) with the same parts |

A native list item is the platform's whole row (`Flex::whole_row`): the list gives it the side
margins from the table above, which line up with the separators, and the item sizes its own
height. A composed item, like any custom row, takes the full row metrics.

## Row heights

`RowHeight::Uniform(h)` makes every row `h` points tall. Each platform has a fixed-pitch fast
path for that, and content taller than `h` is clipped.

`RowHeight::Automatic`, the default, makes each row as tall as its content at the row's width,
padding included. A row grows when the reader picks a larger text size (Dynamic Type on iOS,
font size on Android), when its text wraps onto another line, and when its content changes
after it was first shown. Use it for any row with text in it; keep `Uniform` for rows whose
height cannot change, such as a fixed-size thumbnail grid.

day-core measures a row when the toolkit binds it, and `ListSource::cell_height` answers that
height as soon as `bind_row` returns. When day-core re-lays a bound row at a different height
later, it sends `ListPatch::RowSizeInvalidated { cell, height }`. That patch arrives inside a
layout pass, so a toolkit applies it on its next main-loop turn and never asks the source
anything synchronously.

| Toolkit | How a row takes its height |
|---|---|
| AppKit | `tableView:heightOfRow:` answers the height recorded for the row's token, or the last measured height for a row not bound yet; a bound row whose drawn height differs gets an unanimated `noteHeightOfRowsWithIndexesChanged`. A `Uniform` list does not answer that selector, so it keeps [NSTableView](https://developer.apple.com/documentation/appkit/nstableview)'s fixed `rowHeight` |
| UIKit | self-sizing cells: [`UITableViewAutomaticDimension`](https://developer.apple.com/documentation/uikit/uitableview/automaticdimension), and `DayListCell` answers `systemLayoutSizeFittingSize:…` with the row's height at its content view's width; a change runs an unanimated `beginUpdates`/`endUpdates` |
| Android | the holder's layout height, set after each bind |
| GTK | `DayCell` measures from `cell_height`; a change queues a resize of the bound list item |
| Qt | the model answers each row's `SizeHintRole`: the height Day laid it at, or for a row not measured yet the running average of the measured ones; heights are remembered per row token across reloads, and the changes of one turn land as one relayout that keeps the top row where it was |
| web | rows stack by their measured heights; rows not built yet stand at the average |
| ArkUI | `NODE_HEIGHT` on the row's list item |

ArkUI's automatic row heights are covered by `list-rows-size-to-content`, verified on the
OpenHarmony 7.0 phone emulator.

### A text size change while the app runs

[`day_core::note_text_size_changed()`](https://daybrite.dev/api/day_core/fn.note_text_size_changed.html) forgets every measurement Day holds and re-lays every
bound row, so labels everywhere, and content-sized rows with them, take the new size. UIKit
calls it after `UIContentSizeCategoryDidChangeNotification`, and again when a page that was
off screen then (another tab) is shown with the new size in its traits. Android recreates the
activity on a `fontScale` change, as it does for a theme or language change, and the tree is
rebuilt at the new scale. The desktop and web toolkits do not report a live change yet; the
new size applies from the next launch.

The `list-rows-size-to-content` conformance case checks it on every toolkit: caption, body and
large-title rows and a wrapped footnote each take their content's height natively (every next
row starts one row height below, allowing only for the chrome the host adds to each row), the
large-title row is taller than the caption row, the wrapped row taller than the body row, and a
body row that grows is re-laid at its new height and shrinks back.

## Building it (mock-first, like M0–M1)

1. **spec**: `kinds::LIST`, `ListProps { row_height, selectable, multi_select }`, `RowHeight`, `ListPatch`,
   `ListSource`, [`Toolkit::attach_list`](https://daybrite.dev/api/day_spec/trait.Toolkit.html#method.attach_list). *(additive; no toolkit breaks)*
2. **pieces**: [`list()`](https://daybrite.dev/api/day_pieces/fn.list.html) + builder, reusing `ItemSlot`; produces the type-erased row factory.
3. **core**: the driver + cell-anchor adoption + `list_bind_row`/`list_len`/reload.
4. **mock**: a simulated viewport + `MockProbe` hooks; e2e tests: only-visible-rows built,
   recycle = slot-write (no rebuild), data change → reload rebinds, [`on_select`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.on_select).
5. **toolkits**: AppKit first (reference), then UIKit/Android/GTK/Qt; showcase [`list`](https://daybrite.dev/api/day_pieces/fn.list.html) playground +
   walkthrough leg on all five.

## Selection

Rows report selection through two events: `Event::SelectionChanged(row)` (single) and, in
multi-select mode, `Event::SelectionSet(rows)`, the full set of selected indices on every
change. [`.on_selection(Fn(Vec<K>))`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.on_selection) receives the selected keys either way (a single-selection
report arrives as a one-element set), so an app tracking the whole selection works on every
toolkit. [`.selected_rows(Fn() -> Vec<usize>)`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.selected_rows) reactively syncs app state back into the native
selection (`ListPatch::Selected`; empty clears) without a selection-event echo; drive it from
the same signal [`on_selection`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.on_selection) writes to get a two-way binding and a "clear selection" action.
The binding also applies its initial value after mounting, deferred until row-source updates
settle. AppKit preserves the selected tokens across content reloads and reorders; deleting a
selected token clears it instead of selecting whichever item inherits its index.

A list of [day-model](/docs/explanation/model) store rows can bind single selection by key instead:
[`.selection(binding)`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.selection) takes any `Binding<Option<u64>>` holding the selected row's key. A row the
user picks writes its key (clearing writes `None`) without an echo back to the native list; a
key the app writes selects that row through the same `ListPatch::Selected` sync and scrolls it
into view (`list_scroll_to_row`) once the row is in the snapshot, so an app that inserts a row
and selects it in one turn sees it revealed after the reload that brings it. The selection
follows its row through inserts, moves and reloads, and the initial key selects without
scrolling. The `day new` starter binds its list this way ([`.selection(scene.selected)`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.selection)),
with the same signal driving [`ContentList::detail_for`](https://daybrite.dev/api/day_pieces/struct.ContentList.html#method.detail_for) ([navigation](navigation.md)).

Support matrix: **AppKit** (native [`NSTableView`](https://developer.apple.com/documentation/appkit/nstableview) multi-selection), **Qt** and
**Web** (the emulated lists: a per-cell press hook, a highlight treatment on the cell's
background, ctrl/cmd toggles, shift extends) honor [`multi_select`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.multi_select) and `ListPatch::Selected`.
**Android** and **ArkUI** report single selection (a tap replaces, the touch idiom) but do
honor `ListPatch::Selected`: the sync paints the visible cells (the theme accent at 20%
alpha as the cell background) and newly bound cells inherit their row's state, which is what
lets the composed tree's selection follow the canvas ([docs/tree](tree.md)). **GTK** uses native [`GtkSingleSelection`](https://docs.gtk.org/gtk4/class.SingleSelection.html) for programmatic sync,
reapplying the requested row after model reloads without an event echo. Empty sync clears
selection, and out-of-range rows remain unselected. The sync does not take keyboard focus.
The remaining toolkits report single selection (`SelectionChanged`) and ignore the multi flag and the
programmatic sync; the one-element [`on_selection`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.on_selection) contract still holds there.

AppKit separates selection from keyboard focus. [`.selected_rows`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.selected_rows) uses native selection but
never steals focus from search or other panes. Bind [`.focused(signal)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.focused) and set the signal to
`true` for explicit reading commands that should make the table the first responder (and
therefore give its selected row the active blue highlight). The native table reports focus
loss and gain back to the binding; the surrounding scroll view is not the responder.

### Keyboard

A selectable list is a tab stop, and the arrow keys walk it: ↑/↓ move one row, Home and End jump
to the first and last, and the row that lands is scrolled into view by the edge it left, so a
held arrow scrolls a line at a time rather than recentering on every step. On a [`multi_select`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.multi_select)
list, shift moves the far end of a range while the near end stays put, so `shift+↓ ↓ ↑` grows
twice and shrinks once instead of restarting from wherever the last press ended. A press reports
exactly what a click on the same row reports, so [`on_select`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.on_select)/[`on_selection`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.on_selection) need no keyboard case.

On the desktops this is the native widget's own behavior: nothing in Day sits ahead of the
responder chain, so a focused table, outline or [`QListView`](https://doc.qt.io/qt-6/qlistview.html) gets its arrow keys the way it
always would ([docs/menus](menus.md)). Qt earns it since 2026-09, when its list became a real
Qt item view hosting Day's cells; before that it was a scroll area of Day's own, and an arrow
scrolled it. On **Web** there is no native list to inherit it from, so the
toolkit builds it: the host carries `role="listbox"` and the tab stop, cells carry `role="option"`
and `aria-selected`, and the shim routes the keys into `day_dom_list_key`. Focus sits on the host
rather than a row, because rows are recycled as the list scrolls and focus parked on one would
evaporate under it.

UIKit table lists recycle hidden cells in `didEndDisplaying` and rebind them in `willDisplay`,
so removed rows stop answering element-id lookups and redisplayed rows regain their ids.
Reloads are deferred outside the tree borrow: UIKit may retire cells synchronously inside
`reloadData`, and recycling must be able to access Day's tree at that point.

## Programmatic scrolling

[`.scroll_to_end(Trigger)`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.scroll_to_end) follows the last row (above); [`.scroll_to_row(Signal<Option<usize>>)`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.scroll_to_row)
jumps to any row: set the signal to `Some(row)` and the native list scrolls it into view,
realizing it if it was virtualized away (`ListPatch::ScrollToRow`, clamped to the count). The
signal is the row rail's counterpart to [`scroll`](https://daybrite.dev/api/day_pieces/fn.scroll.html)`(...)`[`.scroll_target`](https://daybrite.dev/api/day_pieces/trait.ScrollBuilder.html#tymethod.scroll_target)`(...)`. Toolkits without a native
scroll-to-index (GTK, web) position by uniform row pitch; prefer
`RowHeight::Uniform` when jumping programmatically there.

A Reload whose rows are the same set in a new order (a shuffle, a programmatic sort) animates
as native row moves on AppKit (`moveRowAtIndex` batch, the same animation a drag commit gets);
other toolkits apply it instantly. A `Splice` from a source that reports row events animates its
inserts and removals as native row changes on AppKit, UIKit (one delta at a time; a splice of
several reloads) and Qt; the other toolkits and every content change reload flat.

## Drag-to-reorder

```rust
list(source, row)
    .reorderable(
        Reorder::new(|from, to| { /* rotate the backing Vec + persist */ })
            .guard(|from, to| ReorderVerdict::Allow)   // optional: Deny / Retarget(i)
            .enabled(move || !busy.get()),             // optional: withhold the gesture live
    )
```

A list is reorderable exactly when it carries a `Reorder` value; the value holds the commit
handler, an optional guard and an optional availability, so the feature cannot be half
configured. [`reorderable`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.reorderable) turns on the platform's drag mechanism; probe `Cap::ListReorder` for
support. [`Reorder::new(moved)`](https://daybrite.dev/api/day_pieces/struct.Reorder.html#method.new) is the commit: row `from` landed at row `to`; apply the identical
rotation to the backing data (`let it = v.remove(from); v.insert(to, it);`) and persist it if
the order should survive a relaunch. It runs at the next event drain, never inside the native
drop callback.

[`Reorder::guard`](https://daybrite.dev/api/day_pieces/struct.Reorder.html#method.guard) vets every proposed drop **synchronously, while the drag is live**: the native
affordance (the macOS gap, the no-drop cursor) reflects the answer before the user releases.
`ReorderVerdict::Deny` refuses the drop (the row springs back); `ReorderVerdict::Retarget(i)`
accepts it at a different index, the "pinned rows" pattern (the Showcase pins its first row
this way). Keep the guard pure: it runs inside the platform's drag callback, so read state and
return without mutating UI. [`Reorder::enabled`](https://daybrite.dev/api/day_pieces/struct.Reorder.html#method.enabled) takes a constant or a reactive `bool` and is read
when the platform asks whether a drag may begin, so an edit mode or a loading state can withhold
the gesture without rebuilding the list.

The reorder half of `ListSource` (`ListSource::reorder`, present only when the list carries a `Reorder`)
has two calls: `can_move(from, proposed) -> accepted-index-or--1` for the live verdict, and
`move_row(from, to)` for the commit, which rotates Day's row snapshot **before returning**, so
`len`/`token_at`/`bind_row` answer in the new order while the native move animates, and defers
the app's `Reorder` handler through the event queue. When the app's own data change echoes back with
exactly the committed token order, the piece skips the redundant `Reload`. When it answers with
any other order, the piece reloads: a store's own row deltas describe the change from its order
before the gesture, not from the rows the gesture already moved, so they cannot be replayed. On AppKit, a deferred
native reload and retile reconciles the table after the drop callback, preserving its selection
and scroll position. This keeps temporary drag geometry from becoming the final document scroll
extent. The reconciliation drains the app callback first and safely skips a disposed list.

The dayscript step `reorder: { id, from, to }` drives the same guard → commit path without a
native gesture (a guard denial fails the step, non-retryably); that is how CI asserts reordering
on every target.

Per-toolkit affordances:

| Toolkit | Mechanism | Affordance | Guard |
|---|---|---|---|
| AppKit | [`NSTableView`](https://developer.apple.com/documentation/appkit/nstableview) drag pipeline (pasteboard row, `validateDrop`/`acceptDrop`) | the `.gap` placeholder opens where the drop would land | live (validate retargets/denies) |
| UIKit | drag delegate + `moveRow`/`targetIndexPathForMove` | long-press lift + gap, no editing mode | live (target-for-move) |
| Android | `ItemTouchHelper` on the RecyclerView | long-press lift, elevation, incremental swaps | live per swap; `Retarget` reads as deny (the helper can't relocate the gap) |
| GTK 4 | `DragSource`/`DropTarget` (native DnD framework) | row snapshot as drag icon; forbidden cursor on deny (no insertion line yet) | live (motion) |
| Qt | [`QDrag`](https://doc.qt.io/qt-6/qdrag.html) over the [`QListView`](https://doc.qt.io/qt-6/qlistview.html) | grabbed-cell pixmap, 2px insertion line, no-drop cursor | live (drag-move) |
| WinUI | WinRT drag events on recycled `ItemContainer` rows | system drag visuals + live no-drop cursor | live (DragOver); target is the actual row, including variable heights |
| ArkUI | `SetNodeDraggable` + `NODE_ON_DROP` | system drag preview; denied drops spring back | at drop (`SetDragResult`) |
| Web | pointer-tracked (emulated — no native list reorder in the browser) | lifted cell + animated CSS gap, long-press on touch | live (every hovered slot) |
| mock | [`MockProbe::list_can_move`](https://daybrite.dev/api/day_mock/struct.MockProbe.html#method.list_can_move) / `list_move` | op log | live |

## Swipe-to-delete

```rust
list(source, row)
    .deletable(
        Delete::new(|index| { /* remove from the backing Vec + persist */ })
            .label(res::str::delete())      // optional: the word on the affordance
            .guard(|index| index != 0),     // optional: protect individual rows
    )
```

A list is deletable exactly when it carries a `Delete` value: the commit handler, an optional
label, an optional per-row guard and an optional [`.enabled(..)`](https://daybrite.dev/api/day_pieces/struct.Reorder.html#method.enabled) availability. [`deletable`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.deletable) turns
on the platform's delete gesture; probe `Cap::ListDelete` for support. [`Delete::new(deleted)`](https://daybrite.dev/api/day_pieces/struct.Delete.html#method.new) is
the commit: row `index` is gone; apply the identical removal to the backing data
(`v.remove(index)`) and persist it if the change should survive a relaunch. It runs at the next
event drain, never inside the native swipe callback.

[`Delete::guard`](https://daybrite.dev/api/day_pieces/struct.Delete.html#method.guard) is consulted **before the affordance is offered**, not after the gesture: a row
that answers `false` shows no delete action at all, rather than one that fails on use. Keep it
pure; it runs inside the platform's swipe callback.

[`Delete::label`](https://daybrite.dev/api/day_pieces/struct.Delete.html#method.label) carries the app's own word for the action; it takes the same text sources as a
label (`res::str::delete()` directly, no [`.format()`](https://daybrite.dev/api/day_fluent/struct.LocalizedText.html#method.format)), resolved in the locale current when the
list builds. A toolkit has no access to the app's Fluent catalog and so cannot translate
anything itself; left unset, each toolkit falls back to its platform's wordless idiom (a trash
glyph), which needs no translation.

The delete half of `ListSource` (`ListSource::delete`, present only when the list carries a `Delete`) has
two calls: `can_delete(index) -> bool` for the offer, and `delete_row(index)` for the commit,
which drops the row from Day's snapshot **before returning**, so `len`/`token_at`/`bind_row`
answer for the shorter list while the native removal animates, and defers the app's
`Delete` handler through the event queue; the reorder half follows the same rule.

**WinUI supports deletion** through [`SwipeControl`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.swipecontrol) and the row context menu. Other desktop
toolkits answer `Unsupported` for the delete affordance: GTK and Qt have
no swipe idiom at all, and macOS's row actions (the swipe-actions section below rides them) do
not carry the delete affordance yet. A list that must be editable everywhere pairs
[`.deletable(..)`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.deletable) with an explicit control (a menu item, a per-row button) and lets the mobile
toolkits add the gesture on top.

The dayscript step `delete_row: { id, row }` drives the same guard → commit path without a
native gesture (a guard refusal fails the step, non-retryably), which is how CI asserts deletion
on every target, including the desktops, where there is no gesture to simulate.

| Toolkit | Mechanism | Affordance |
|---|---|---|
| UIKit | `trailingSwipeActionsConfigurationForRowAtIndexPath` | row tracks the finger, destructive action reveals behind it, full swipe commits |
| Android | `ItemTouchHelper` swipe (`START`, so it is the trailing edge in RTL too) | row slides to reveal a red field carrying the label; `notifyItemRemoved` animates the close-up |
| WinUI | [`SwipeControl`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.swipecontrol) trailing [`SwipeItems`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.swipeitems) and [`MenuFlyout`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.menuflyout) | reveal then press Delete; the guard is checked both when offering and invoking it |
| AppKit · GTK · Qt · web | — | `Unsupported`: the delete affordance does not ride macOS's row actions yet, and the rest have no swipe idiom; use a menu item or a row button |
| ArkUI | `NODE_LIST_ITEM_SWIPE_ACTION`: a trailing swipe-action item on each deletable row | the row slides to reveal the labeled action; pressing it runs the guard and commits |

## Swipe actions

Swipe actions generalize swipe-to-delete: app-declared buttons reveal behind a row as the
user drags it aside, on either edge, with the platform's full-swipe shortcut for the first
one (Mail's triage gestures).

```rust
list(source, row)
    .swipe_trailing(move |i| {
        let read = is_read(i);
        vec![
            swipe_action(if read { str::mark_unread() } else { str::mark_read() })
                .symbol(if read { Symbol::CircleFilled } else { Symbol::Circle })
                .tint(palette().accent)
                .action(move || set_read(i, !read)),
        ]
    })
    .swipe_leading(move |i| vec![
        swipe_action(str::delete_word())
            .destructive()
            .action(move || remove(i)),
    ])
```

The provider runs at **gesture time**, with the row index, as the row starts to slide, so the
offer reflects the row's current state (the "Mark as Read" / "Mark as Unread" flip above is
why it is a closure rather than a fixed list). Keep it pure and fast: it runs inside the
platform's swipe callback. The `action` handlers run later, at the event drain, never inside
the native gesture. When an activation drains, the provider is invoked again and the action
looked up by position, so the handler always closes over the row's current state.

[`symbol`](https://daybrite.dev/api/day_pieces/struct.SwipeAction.html#method.symbol) puts a glyph on the button where the platform draws one: above the label on macOS
(Mail's row-action look), in place of it on iOS (the label stays the accessibility name). The
platforms without a glyph slot show the label alone.

Edges are semantic, not geometric: `Leading` follows the reading direction (left in LTR, right
in RTL), exactly as every platform's swipe API already spells it. On Apple platforms a full
swipe activates the edge's first action; WinUI uses reveal-and-press buttons. `destructive` takes the platform's destructive styling
(red, on the Apple toolkits); `tint` colors the button where the platform honors one.

Probe `Cap::ListSwipeActions`: **Native** on macOS ([`NSTableView`](https://developer.apple.com/documentation/appkit/nstableview)'s
`tableView:rowActionsForRow:edge:`, the two-finger Mail swipe) and iOS
([`UISwipeActionsConfiguration`](https://developer.apple.com/documentation/uikit/uiswipeactionsconfiguration), sharing one pipeline with swipe-to-delete: on the trailing
edge a deletable list offers its delete action first, then the row's own trailing offer).
It is also **Native** on WinUI ([`SwipeControl`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.swipecontrol), with labels, symbols and tints), which adds a
mouse/keyboard [`MenuFlyout`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.menuflyout) for the same actions. Everywhere else the answer is `Unsupported`
and no affordance is drawn, so pair each action
with an explicit control (a menu item, a toolbar button) for the rest, as the delete section
advises.

`ListSource::swipe`, present only when an edge is declared, carries the swipe half:
`actions_at(index, edge) -> Vec<ListSwipeAction>` pulls the offer (label and styling; the
handlers stay in the pieces layer), and `perform(index, edge, action)` commits an
activation, deferring the app's handler through the event queue.

The dayscript step `swipe_row: { id, row, edge?, action?, label?|key? }` drives the same
offer → commit path without a native gesture (`edge` defaults to `trailing`, `action` to 0,
the full-swipe button). `label:` (literal) or `key:` (a Fluent key resolved in the run's
locale) pins which button the step may press; pinning matters because offers are
state-dependent. The pin is checked before the press: a mismatched offer refuses the
activation and fails the step, leaving the row's state untouched, so a stale pin (say, from
an aborted earlier run's leftover state) fails once instead of flipping state and poisoning
every later run. An empty offer or an out-of-range action also fails, non-retryably. This is
how a walkthrough exercises the wiring on every target, including the toolkits that show no
affordance.

## Editing mode

```rust
list(source, row)
    .reorderable(Reorder::new(|from, to| { /* … */ }))
    .deletable(Delete::new(|index| { /* … */ }))
    .on_activate(|row| edit(row))
    .editing(move || editing.get())   // or .editing(true) for a page that is always editing
```

[`editing`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.editing) puts the list in the platform's own editing mode, the one an Edit button enters on
iOS: each row the `Delete` offers shows the red delete control at its leading edge, each row
the `Reorder` moves shows a grip at its trailing edge, and a tap still activates the row, so a
list of things to configure can open each one from the same screen. The value is reactive; a
change animates the controls in or out. The delete control commits through the same
[`Delete::guard`](https://daybrite.dev/api/day_pieces/struct.Delete.html#method.guard) → handler path as the swipe, and the grip through the same [`Reorder::guard`](https://daybrite.dev/api/day_pieces/struct.Reorder.html#method.guard) →
handler path as a long-press drag.

Probe `Cap::ListEditing`. Only UIKit has a separate editing mode: on the other toolkits a
list's rows already drag, swipe, and open their menus as they are, so [`editing`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.editing) is ignored
there and the list renders as it would without it. A screen that manages a list everywhere
uses [`.editing(true)`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.editing) for iOS and keeps its explicit controls for the toolkits that answer
`Unsupported` for `ListDelete`.

| Toolkit | Mechanism |
|---|---|
| UIKit | [`UITableView.setEditing(_:animated:)`](https://developer.apple.com/documentation/uikit/uitableview), `editingStyleForRowAtIndexPath`, `commitEditingStyle`, `allowsSelectionDuringEditing` |
| AppKit · GTK · Qt · web · Android · ArkUI | — (`Unsupported`; ignored) |

While editing, UIKit narrows each cell's content view for the two controls. Rows are
`DayListCell`s that report the narrower width through `ListSource::layout_cell`, so Day lays the
row out to fit between them rather than under them.

## Separators

```rust
list(source, row).separators()      // or .no_separators()
```

The host draws row separators, at the row boundary; row content does not. A hand-drawn
hairline inside a row sits wherever the row's own layout puts it, which is not where the
native selection ends and not where the platform slides its rows: it misaligns with the
selection under a uniform pitch, doubles up with iOS's native line, and stays frozen while a
swipe-action reveal slides the row past it. The host's separator sits at the boundary, so
none of those apply.

Left unset, each platform keeps its own default: iOS draws separators, the desktops don't.
[`.separators()`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.separators) forces them on, [`.no_separators()`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.no_separators) off (an iOS list whose rows draw their own
separation turns the native line off rather than showing both).

| Toolkit | Mechanism |
|---|---|
| AppKit | `gridStyleMask = solidHorizontalGridLineMask`, `separatorColor` |
| UIKit | the table's own `separatorStyle` (default on; `false` sets `.none`) |
| GTK | a `border-bottom` on the ListView's `row` CSS nodes |
| web | a `border-bottom` inside each cell frame (border-box) |
| Qt | the delegate paints a hairline along each row's bottom edge (the palette's `Mid`, translucent) |
| Android · ArkUI | *not lowered yet* — a force is ignored; rows separate by their pitch |

## Reorder + row-height caveat

Rows drag within their own list only; nothing is draggable out of the app. The web, GTK, Qt
and ArkUI find the row under the pointer from the rows' own heights, and AppKit, UIKit and
Android use their platform's own drag.

## Selection and activation

Selection identifies the row the user is browsing. Activation performs its main action,
such as opening a document or playing a station. Use separate callbacks when selecting a
row should update an inspector without starting that action:

```rust,ignore
list(stations, station_row)
    .on_select(move |station| selected.set(Some(station)))
    .on_activate(move |station| player.play(station));
```

Desktop lists activate on double-click or Enter; Qt follows the platform's activation
preference. UIKit, Android, and ArkUI activate on a tap. The browser uses double-click or
Enter for a mouse and a single tap for touch. Arrow-key
selection and programmatic selection do not activate rows. [`on_activate`](https://daybrite.dev/api/day_pieces/trait.ListBuilder.html#tymethod.on_activate) receives the row's
key or model reference from the current list snapshot, so reordering does not leave it pointing
to an old index. An explicit button within a row can offer the same action separately.

At the toolkit boundary, `Event::ListActivated(index)` carries the current zero-based row
index. It is independent of `SelectionChanged` and `SelectionSet`; no selection change is
required when the user activates an already selected row.

Dayscript keeps the two operations separate:

```yaml
- select: { id: stations, index: 0 }
- activate: { id: stations, index: 0 }
```

`activate` invokes the row without changing selection. An out-of-range index is ignored.
Recordings preserve activation even when the row was already selected.

AppKit plain labels inside selected native rows use the system selected text color at paint
time, including inactive-window selection. Their normal colors are preserved for reuse and
deselection; explicitly attributed text retains its authored run styling.
