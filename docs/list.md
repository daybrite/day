---
navTitle: "Lists"
title: "Native list"
description: "The list piece over native table and collection views: virtualized rows, selection, swipe actions, reorder, and editing mode."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Lists

A list presents related items as rows that people can scan, select, or open. It suits collections
such as messages or search results, including those too long to fit onscreen at once.

[`list`](https://docs.rs/day-pieces/latest/day_pieces/fn.list.html) drives the platform's recycling list ([`NSTableView`](https://developer.apple.com/documentation/appkit/nstableview) / [`UITableView`](https://developer.apple.com/documentation/uikit/uitableview) /
[`RecyclerView`](https://developer.android.com/reference/androidx/recyclerview/widget/RecyclerView) / [`GtkListView`](https://docs.gtk.org/gtk4/class.ListView.html) / [`QListView`](https://doc.qt.io/qt-6/qlistview.html) / WinUI [`ItemsRepeater`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.itemsrepeater)), so large collections get native
virtualization, scroll physics, and platform behaviors. It is the one place Day's
build-once model meets cell reuse, and the resolution is the same model: a row subtree is
built once per physical cell and *rebound* (a single slot-write into its `ItemSlot`) every
time that cell is recycled for a new item.

Contrast with [`each`](../crates/day-pieces): [`each`](https://docs.rs/day-pieces/latest/day_pieces/fn.each.html) builds every row eagerly under one
anchor (fine for a dozen items; ten thousand would all be built up front). [`list`](https://docs.rs/day-pieces/latest/day_pieces/fn.list.html) builds only
the rows the native widget currently shows.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Add to your app

This control is included in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces)
and re-exported through `day::prelude::*`. No additional crate or control feature is required.

## Authoring

On **[windows-winui](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui")**, [`ItemsRepeater`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.itemsrepeater) and `StackLayout` own realization and scroll extents
inside a native [`ScrollViewer`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.scrollviewer). Its element factory reuses `ItemContainer` rows with Day
content; it does not allocate a cell per source index. Selection, keyboard navigation,
drag-and-drop, and content-sized rows follow the realized containers. A [`SwipeControl`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.swipecontrol)
offers declared leading/trailing actions and guarded deletion, with the same commands in
a native row context menu for mouse and keyboard use. Row content can still declare its
own context menu. Action callbacks validate the row token and cell generation before use.

`list-recycles-distant-rows` checks that distant jumps keep a bounded number of rows alive and rebind their actions; `list-releases-discarded-rows` churns a list by jumps, shrinks, regrowth and reorders and checks the same bound;
`list-shrink-and-regrow` checks emptying and repopulating a scrolled list. Run them with the
other list cases using `day test --project apps/conformance -t windows-winui 'list-*'`.

[`list(source, row)`](https://docs.rs/day-pieces/latest/day_pieces/fn.list.html) and [`each(source, row)`](https://docs.rs/day-pieces/latest/day_pieces/fn.each.html) take a **`RowSource`** (where the rows come from)
and one row builder. Two sources ship:

**Plain data**: an items closure and a key function, wrapped in [`items(…)`](https://docs.rs/day-pieces/latest/day_pieces/fn.items.html):

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
> **A recycled row needs a reactive ID.** Use [`.id_of`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id_of)`(move || id!(message_row, row`[`.key`](https://docs.rs/day-pieces/latest/day_pieces/struct.ItemSlot.html#method.key)`()))`
> so the ID follows the item occupying the cell. An eager [`.id`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id)`(id!(message_row, row`[`.key`](https://docs.rs/day-pieces/latest/day_pieces/struct.ItemSlot.html#method.key)`()))`
> would retain its first item's ID. Sender names and message previews are user content;
> app-owned actions and empty states belong in the [Fluent catalog](/docs/how-to/localization).

The row builder receives an `ItemSlot<T, K>` (Copy handle, tracked [`get()`](https://docs.rs/day-pieces/latest/day_pieces/struct.ItemSlot.html#method.get), memoised [`field()`](https://docs.rs/day-pieces/latest/day_pieces/struct.ItemSlot.html#method.field)
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

Builder options: [`.row_height(RowHeight)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.row_height), [`.on_select(Fn(Ref))`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.on_select) (the key for a plain source,
the row's `Elem` for a store source), [`.multi_select()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.multi_select), [`.on_selection(Fn(Vec<Ref>))`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.on_selection),
[`.selected_rows(Fn() -> Vec<usize>)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.selected_rows), [`.selection(binding)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.selection) (store sources, below), and
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

A two-axis [`.frame(width, height)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.frame) keeps rows stable while loading. It is a measurement
boundary, not a placement barrier: both image-source updates and conditional children inside
it are laid out when they change, including in detached native list and tree cells.
List cells parked by `recycle` retain their contents and pending layout dirt, but skip both
width-change layout and dirty-cell sweeps. `bind_row` reactivates and lays them out before
redisplay. Resize work therefore follows the active viewport rather than every physical
cell accumulated in the native reuse pool.

### Following the end

A chat timeline or a log keeps its newest row in view, until the reader scrolls up to read
something older.

```rust
list(items(move || messages.get(), |m| m.id), message_row)
    .follow_end(40.0)
```

[`.follow_end(threshold)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.follow_end) scrolls the list to its new end whenever the rows
change while the end is within `threshold` points of the bottom of the viewport. A reader who
has scrolled further up than that is left where they are. They rejoin by scrolling back down,
or with a button:

```rust
let latest = day_reactive::Trigger::new();
zstack((
    list(items(move || messages.get(), |m| m.id), message_row)
        .follow_end(40.0)
        .scroll_to_end(latest),
    button(res::str::jump_to_latest()).action(move || latest.notify()),
))
```

[`.scroll_to_end(Trigger)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.scroll_to_end) scrolls to the end each time the trigger fires,
wherever the list is. Building the list never scrolls, and an empty list ignores both.

New rows scroll into view with the platform's scroll animation. A row that only grows (a reply
streamed into the last row) is kept in view without one, so thirty updates a second do not
start thirty animations. While a followed scroll is still on its way, the positions the list
reports are not read as the reader leaving. The moment the reader takes the list themselves
(a drag, the scroll wheel or trackpad, the scroll bar, or a key that scrolls), following stops,
even mid-animation, and the next row leaves them where they are until they come back within
the threshold. The native list tells the reader's scrolls from the app's
(`Cap::ListReaderScroll`); where it cannot, following stops only once the list reports the
reader further from the end than the threshold.

| Toolkit | The reader's scrolls (`Cap::ListReaderScroll`, all `Native`) |
| --- | --- |
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | A trackpad or Magic Mouse gesture with its momentum, the scroller, the mouse wheel, and the arrow, Home and End keys in a focused list. The app's own scroll waits while a gesture or a scroller drag holds the list. |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | A finger drag (and an iPad's trackpad or mouse, through the same pan) and a tap on the status bar. Scrolling with a hardware keyboard looks like the app's own scroll and is not reported. The app's scroll waits while a finger is on the list. |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | `RecyclerView`'s drag, and the scroll a mouse wheel, a scrolling key or an accessibility scroll action starts, each as it first moves the list. A touch that only stops a followed scroll without moving the list is not reported. |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | A wheel or touchpad step that moves the list, the list moving while a finger holds the content or the pointer holds the scroll bar, and a scrolling key (arrows, Page Up/Down, Home/End) that moves it. The key also stops a followed scroll. |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | The scroll bar's own actions when they move the list (wheel, trackpad, thumb drag, a click in the trough) and a key whose cursor move scrolled it. Qt lists have no touch scrolling. |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | A wheel or scrolling key that moves the view, a DirectManipulation that starts under a touch or pen contact, and the scroll bar. A key stops the list's own animated scroll first. A precision touchpad that arrives without a pointer press is not reported. |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | A scroll under a finger, while the scroll bar is held, or right after a wheel or a scrolling key. Input that lands on the list's own smooth scroll stops it where it is. Middle-click autoscroll is not reported. |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | The scroll source ArkUI gives each scroll: a drag, other user input (wheel, keys) or the scroll bar, reported with the first viewport report it changes. A touch that only stops an animation is not reported. |

The distance to the end comes from the native list (`Cap::ListViewport`). On a toolkit that
does not report it, `follow_end` follows every change.

### Reaching the end

[`.on_end_reached(threshold, f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.on_end_reached) calls `f` when the reader comes within
`threshold` points of the end of the list, which is the moment to load the next page:

```rust
list(rows, row_view)
    .on_end_reached(400.0, move || feed.load_more())
    .footer(move || spinner().padding(Space::L))
```

It fires once on the way in. It fires again after the end has left that distance and come
back, or after the number of rows has changed, so a handler that appends rows is called again
when the reader reaches the new end. The check runs on the list's next viewport report, and a
toolkit reports one once the new rows are laid out: when a page leaves the end within
`threshold` (a first page shorter than the viewport), `f` runs again without a scroll, and
again after each page, until the end is farther than `threshold`. A call that adds no rows
re-arms nothing, so a feed that has run out is not called in a loop. An empty list does not
fire it. It needs `Cap::ListViewport`.

### Header, footer and empty state

```rust
list(rows, row_view)
    .header(move || search_summary(query))
    .footer(|| label(res::str::end_of_results()).secondary().padding(Space::L))
    .empty(|| label(res::str::no_results()).secondary())
    .content_padding(Insets { top: 0.0, leading: 0.0, bottom: 88.0, trailing: 0.0 })
```

- [`.header(build)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.header) and [`.footer(build)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.footer) put content above the first
  row and below the last. Both scroll with the rows.
- [`.empty(build)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.empty) shows content in place of the rows while the list has none,
  centered in the viewport (between the header and the footer when the list has them). It goes
  when the first row arrives and returns when the last one leaves.
- [`.content_padding(insets)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.content_padding) adds space around the content, inside the
  scrolling area. The top and bottom scroll with the rows, so the last row can clear a
  floating composer or tab bar. The sides inset every row.

None of these is a row. Nothing selects, activates, swipes or drags them, and every row index
(selection, swipe offers, `selected_rows`, `scroll_to_row`, `visible_rows`) counts rows only.

They work the same on every toolkit, because day-core adds them as rows of their own around
the items: the host binds them like any row. `ListSource::is_supplement` tells the host which
rows they are, so that it sizes them to their content in a `Uniform` list and draws no
separator, selection or swipe affordance on them.

### Sections

[`.sections(key, header)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.sections)
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
  swipe offers, `visible_rows` and dayscript's `select`, `swipe_row`, `reorder` and
  `delete_row`.
- A header sizes to its content, even in a [`Uniform`](https://docs.rs/day-spec/latest/day_spec/props/enum.RowHeight.html)
  list, and takes the platform's side insets but not its vertical insets or minimum row height.
- A row drags only within its own section, the rule
  [`UITableView`](https://developer.apple.com/documentation/uikit/uitableviewdelegate/tableview(_:targetindexpathformoveatindexpath:toproposedindexpath:))
  apps follow for grouped tables. A header never lifts, and a drop past either end of the row's
  section lands at that end, through the same retarget the
  [`reorderable`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.reorderable)
  guard uses. The guard sees row indices, and dayscript's `reorder` keeps to the same rule. A
  move between sections is a change to the row's data: update its section key, and the list
  regroups.
- A sectioned list offers no native swipe-to-delete, because deleting a section's last row
  takes its header with it. Swipe actions still work, and dayscript's `delete_row` still drives
  the app's handler.

`Cap::ListSections` says how a toolkit pins the header:

| Toolkit | Header rows | Pinned header |
|---|---|---|
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | [`NSTableView`](https://developer.apple.com/documentation/appkit/nstableview) group rows (`tableView(_:isGroupRow:)`), refused by `tableView(_:shouldSelectRow:)` | native: [`floatsGroupRows`](https://developer.apple.com/documentation/appkit/nstableview/floatsgrouprows) |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | one table section per header; the header is the section's [`UITableViewHeaderFooterView`](https://developer.apple.com/documentation/uikit/uitableviewheaderfooterview) | native: a plain-style table pins section headers |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | a `RecyclerView` view type of its own, with no click, ripple or focus | emulated: a copy in the list's overlay |
| [GTK 4](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | model items marked not selectable, activatable or focusable | emulated: a copy in a [`GtkOverlay`](https://docs.gtk.org/gtk4/class.Overlay.html) over the scrolled window |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | model rows without `ItemIsEnabled` or `ItemIsSelectable`, painted as a band by the delegate | emulated: a copy over the viewport |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | cells in a `Border` of their own, without the `ItemContainer` rows get, so they take no selection, hover or focus | emulated: a copy over the `ScrollContentPresenter`, under the scroll bars |
| [web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | cells with `role="heading"` that take no clicks | emulated: a copy in a `position: sticky` wrapper |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | `ListItem`s with no click, drag or swipe action | emulated: a copy stacked over the list |

Where the pin is emulated, the host binds the copy through `ListSource::bind_pinned`. The copy
carries no element ids, so a dayscript lookup of an id inside a header finds the header row in
the list, never its pinned copy.

Inside day-core the host sees display rows, headers included: `ListSource::is_header` says which,
and `len`, `token_at`, `bind_row`, the selection patch, the row scroll and the toolkit's
selection events all count headers. day-core translates between display rows and the app's rows, so a
toolkit needs no section logic beyond drawing headers and keeping them out of selection.

### Visible rows

[`.visible_rows(binding)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.visible_rows) follows the rows in view, as a half-open range of
row indices:

```rust
let visible = Signal::new(0..0);
list(rows, row_view).visible_rows(visible);
let heading = move || section_title(visible.get().start);
```

`start` is the first row with any part showing, overscan excluded, and `end` is one past the
last. The range is empty while no row is in view. Section headers and the list's own header
and footer are not rows: a header at the top of the viewport reports its section's first row.
The report is opt-in and event-driven: native scroll callbacks enqueue a deduplicated report,
and the event pump updates the binding when the tree is available. No timer is involved.
Clamp the range when the data shrinks. For section headers that float, use
[`.sections`](#sections) instead of an overlay driven by this range.

`visible_rows`, `follow_end` and `on_end_reached` all read one report from the host,
`ListSource::viewport`: the first and last visible rows and the distance from the bottom of
the viewport to the end of the content. A host calls it after a scroll, after a resize, and
after a reload or splice has been laid out; day-core translates its rows to items and drops
repeats. Where `Cap::ListViewport` is unsupported the range holds the first visible row only.

The report follows the layout, so it can arrive a frame after a programmatic scroll starts.
A dayscript step that reads the range after a scroll button waits for the scroll to settle
first: `wait_idle` and `tap` wait on
[`Toolkit::ui_idle`](https://docs.rs/day-spec/latest/day_spec/trait.Toolkit.html#method.ui_idle), and GTK reports busy while a
programmatic scroll is still gliding on a shown list, and while a shown list has row changes or
a move it has not reported yet. The web backend reports busy while a shown list's own smooth
scroll is still moving and while a shown list has not yet reported the offset, height or row
count it has now. WinUI reports busy while a list's row jump has not reached its row or its
animated scroll is still moving, and while a list still has a pass of row binds and its viewport
report posted. ArkUI reports busy while a list's programmatic scroll waits to be applied or,
animated, has not stopped, and while a list has a reload posted or a scroll or reload that
ArkUI has not laid out yet. Each wait is bounded, so a scroll that cannot finish never holds a
script for good. A list that has never reported counts as at its end for
`follow_end`, so a script that adds rows right after the list appears waits for that first
report too.

## Row lifetime

A physical cell keeps its row subtree while the toolkit reuses it for different items. Read
item values through the `ItemSlot` in reactive closures and action handlers; a value captured
when the row was first built may belong to a previous item. Store persistent per-item state
in the model, keyed by the item's identity.

Leaving the viewport does not necessarily dispose a row. Toolkits may keep spare cells for
reuse; a discarded cell or removed list disposes its row scope. Element IDs for recycled rows
must follow the slot's current identity. See [element IDs](/docs/reference/element-ids).

A plain items closure re-runs when its dependencies change, and the list applies the
difference as row insertions, removals and moves (see [Keyed updates](#keyed-updates)).
Sources that supply their own row deltas, such as persistence queries, apply those directly.

## Keyed updates

A list knows each row by the key its source gives ([`items(rows, |row| row.id)`](https://docs.rs/day-pieces/latest/day_pieces/fn.items.html)). When the rows
change, the list compares the old keys with the new ones and tells the native list exactly
what happened:

| change | what the native list gets |
| --- | --- |
| a row added, removed or moved | a splice: one insert, remove or move per row, animated where the toolkit animates them |
| the same rows, some values changed | nothing; the rows on screen are rebound in place |
| rows that share nothing with the old ones, or the list becoming empty or non-empty | a reload |
| a key that appears twice | a reload |
| more than 512 changed rows at once | a reload |

Appending one message to a 5,000-row timeline is therefore one inserted row, and a reply
streamed into a row thirty times a second touches that row and no other. The scroll position
and the native selection survive a splice; a reload keeps neither on some toolkits.

The moves are the fewest that produce the new order: the rows of the longest run that kept
its order stay where they are, and each other row moves once. Section headers and the list's
header, footer and empty state are part of the same comparison, so a sectioned list splices
too: a row that opens a new section brings its header in with it.

Keys must be unique within a list. A repeated key cannot say which of its rows moved, so the
list reloads; `each` treats the same mistake as a bug and panics in debug builds.

[`day_core::diff_rows`](https://docs.rs/day-core/latest/day_core/list/fn.diff_rows.html) is the
comparison, and [`day_core::list_sync`](https://docs.rs/day-core/latest/day_core/list/fn.list_sync.html)
applies it to a list node.

## Toolkit implementations

| Toolkit | List view | Behavior |
| --- | --- | --- |
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | [`NSTableView`](https://developer.apple.com/documentation/appkit/nstableview) | Native scrolling and reusable cells. |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | [`UITableView`](https://developer.apple.com/documentation/uikit/uitableview) | Native scrolling and reusable cells. |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | [`RecyclerView`](https://developer.android.com/reference/androidx/recyclerview/widget/RecyclerView) | Native scrolling and reusable cells. |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | [`GtkListView`](https://docs.gtk.org/gtk4/class.ListView.html) | Native scrolling; may retain spare cells beyond the visible rows. |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | [`QListView`](https://doc.qt.io/qt-6/qlistview.html) | Native list interaction with a pool of Day row views. |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | [`ItemsRepeater`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.itemsrepeater) | Virtualized rows within a native scroll viewer. |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | `List` with `NodeAdapter` | Native scrolling and reusable cells. |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | DOM scroller | Builds visible rows plus overscan; previously visited rows retain their elements. |

Toolkit-specific selection, swipe, and reorder behavior is described with each feature below.

## Row metrics

A row reads at the platform's size without the app padding it. day-core lays each row inside
the margins the toolkit's own list gives a row and keeps it at least that list's row height,
content centered ([`Toolkit::list_row_metrics`](https://docs.rs/day-spec/latest/day_spec/trait.Toolkit.html#method.list_row_metrics), asked of the list when a cell is first used), so
a row piece holds only its content: [`row((icon, label))`](https://docs.rs/day-pieces/latest/day_pieces/fn.row.html), not [`row`](https://docs.rs/day-pieces/latest/day_pieces/fn.row.html)`(..)`[`.padding`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding)`(..)`.

| Toolkit | Margins (top, sides) | Minimum height | Source |
|---|---|---|---|
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | a stock cell's top margin; the table's layout margins at the sides (16 or 20pt) | the height [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") fits a stock cell with one line of its default content configuration | measured from [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") at runtime |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | 8dp, 16dp | 56dp | Material one-line list item |
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | 4pt, 8pt | 28pt | [macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") table row |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | 8px, 12px | 44px | Adwaita rich-list row |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | 4px, 8px | 28px | item view row |
| [web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | 10px, 16px | 44px | a touch-sized web row |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | 8vp, 12vp | 48vp | [HarmonyOS](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") list item |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | 0, 12px | 40px | `ListViewItem` padding and `MinHeight` |

A `Uniform` row height still wins: the row is that tall and its content sits inside the margins,
or, where the row is too short for them, centered in the whole row.

## List items

[`list_item(title)`](https://docs.rs/day-pieces/latest/day_pieces/fn.list_item.html) is a row's standard content drawn as the platform draws its own list rows:
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
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | [`UIListContentView`](https://developer.apple.com/documentation/uikit/uilistcontentview) with the cell's default content configuration; the checkmark accessory's glyph and a secondary value beside it |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | a Material 3 list item (`DayListItem`): 24dp leading icon, body-large headline, body-medium supporting text, label-small trailing text, a primary-colored check; 56dp, 72dp with a supporting line |
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | an image-and-text table cell's parts: a 16pt template image, the system font, a smaller secondary subtitle, the accent checkmark; at least 28pt |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | a row box: a 16px icon, the title over a dim caption, a dim value, `object-select-symbolic` in the accent color; at least 44px |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt"), web, [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui"), [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | composed from [`vector`](https://docs.rs/day-pieces/latest/day_pieces/fn.vector.html), [`label`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html) and [`image`](https://docs.rs/day-pieces/latest/day_pieces/fn.image.html) with the same parts |

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
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | `tableView:heightOfRow:` answers the height recorded for the row's token, or the last measured height for a row not bound yet; a bound row whose drawn height differs gets an unanimated `noteHeightOfRowsWithIndexesChanged`. A `Uniform` list does not answer that selector, so it keeps [NSTableView](https://developer.apple.com/documentation/appkit/nstableview)'s fixed `rowHeight` |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | self-sizing cells: [`UITableViewAutomaticDimension`](https://developer.apple.com/documentation/uikit/uitableview/automaticdimension), and `DayListCell` answers `systemLayoutSizeFittingSize:…` with the row's height at its content view's width; a change runs an unanimated `beginUpdates`/`endUpdates` |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | the holder's layout height, set after each bind |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | `DayCell` measures from `cell_height`; a change queues a resize of the bound list item |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | the model answers each row's `SizeHintRole`: the height Day laid it at, or for a row not measured yet the running average of the measured ones; heights are remembered per row token across reloads, and the changes of one turn land as one relayout that keeps the top row where it was |
| [web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | rows stack by their measured heights; rows not built yet stand at the average |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | `NODE_HEIGHT` on the row's list item, rounded to whole device pixels: ArkUI snaps each row's edges to the pixel grid, so a row a fraction of a pixel tall would leave equal rows below it one pixel apart |

ArkUI's automatic row heights are covered by `list-rows-size-to-content`, verified on the
OpenHarmony 7.0 phone emulator.

### A text size change while the app runs

[`day_core::note_text_size_changed()`](https://docs.rs/day-core/latest/day_core/fn.note_text_size_changed.html) forgets every measurement Day holds and re-lays every
bound row, so labels everywhere, and content-sized rows with them, take the new size. UIKit
calls it after `UIContentSizeCategoryDidChangeNotification`, and again when a page that was
off screen then (another tab) is shown with the new size in its traits. Android recreates the
activity on a `fontScale` change, as it does for a theme or language change, and the tree is
rebuilt at the new scale. The desktop and web toolkits do not report a live change yet; the
new size applies from the next launch.

## Selection

Rows report selection through two events: `Event::SelectionChanged(row)` (single) and, in
multi-select mode, `Event::SelectionSet(rows)`, the full set of selected indices on every
change. [`.on_selection(Fn(Vec<K>))`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.on_selection) receives the selected keys either way (a single-selection
report arrives as a one-element set), so an app tracking the whole selection works on every
toolkit. [`.selected_rows(Fn() -> Vec<usize>)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.selected_rows) reactively syncs app state back into the native
selection (`ListPatch::Selected`; empty clears) without a selection-event echo; drive it from
the same signal [`on_selection`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.on_selection) writes to get a two-way binding and a "clear selection" action.
The binding also applies its initial value after mounting, deferred until row-source updates
settle. AppKit preserves the selected tokens across content reloads and reorders; deleting a
selected token clears it instead of selecting whichever item inherits its index.

A list of [day-model](/docs/explanation/model) store rows can bind single selection by key instead:
[`.selection(binding)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.selection) takes any `Binding<Option<u64>>` holding the selected row's key. A row the
user picks writes its key (clearing writes `None`) without an echo back to the native list; a
key the app writes selects that row through the same `ListPatch::Selected` sync and scrolls it
into view (`list_scroll_to_row`) once the row is in the snapshot, so an app that inserts a row
and selects it in one turn sees it revealed after the reload that brings it. The selection
follows its row through inserts, moves and reloads, and the initial key selects without
scrolling. The `day new` starter binds its list this way ([`.selection(scene.selected)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.selection)),
with the same signal driving [`ContentList::detail_for`](https://docs.rs/day-pieces/latest/day_pieces/struct.ContentList.html#method.detail_for) ([navigation](navigation.md)).

Support matrix: **[AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit")** (native [`NSTableView`](https://developer.apple.com/documentation/appkit/nstableview) multi-selection), **[Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt")** and
**[Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom")** (the emulated lists: a per-cell press hook, a highlight treatment on the cell's
background, ctrl/cmd toggles, shift extends) honor [`multi_select`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.multi_select) and `ListPatch::Selected`.
**[Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material")** and **[ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui")** report single selection (a tap replaces, the touch idiom) but do
honor `ListPatch::Selected`: the sync paints the visible cells (the theme accent at 20%
alpha as the cell background) and newly bound cells inherit their row's state, which is what
lets the composed tree's selection follow the canvas ([docs/tree](tree.md)). **[GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk")** uses native [`GtkSingleSelection`](https://docs.gtk.org/gtk4/class.SingleSelection.html) for programmatic sync,
reapplying the requested row after model reloads without an event echo. Empty sync clears
selection, and out-of-range rows remain unselected. The sync does not take keyboard focus, and
it does not scroll: a selected row that a keyed insert or removal moves stays where the reader
left it, and a row comes into view only through a scroll the app asks for.
The remaining toolkits report single selection (`SelectionChanged`) and ignore the multi flag and the
programmatic sync; the one-element [`on_selection`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.on_selection) contract still holds there.

AppKit separates selection from keyboard focus. [`.selected_rows`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.selected_rows) uses native selection but
never steals focus from search or other panes. Bind [`.focused(signal)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.focused) and set the signal to
`true` for explicit reading commands that should make the table the first responder (and
therefore give its selected row the active blue highlight). The native table reports focus
loss and gain back to the binding; the surrounding scroll view is not the responder.

### Keyboard

A selectable list is a tab stop, and the arrow keys walk it: ↑/↓ move one row, Home and End jump
to the first and last, and the row that lands is scrolled into view by the edge it left, so a
held arrow scrolls a line at a time rather than recentering on every step. On a [`multi_select`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.multi_select)
list, shift moves the far end of a range while the near end stays put, so `shift+↓ ↓ ↑` grows
twice and shrinks once instead of restarting from wherever the last press ended. A press reports
exactly what a click on the same row reports, so [`on_select`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.on_select)/[`on_selection`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.on_selection) need no keyboard case.

On the desktops this is the native widget's own behavior: nothing in Day sits ahead of the
responder chain, so a focused table, outline or [`QListView`](https://doc.qt.io/qt-6/qlistview.html) gets its arrow keys the way it
always would ([docs/menus](menus.md)). On **[Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom")** there is no native list to inherit it from, so the
toolkit builds it: the host carries `role="listbox"` and the tab stop, cells carry `role="option"`
and `aria-selected`, and the shim routes the keys into `day_dom_list_key`. Focus sits on the host
rather than a row, because rows are recycled as the list scrolls and focus parked on one would
evaporate under it.

UIKit table lists recycle hidden cells in `didEndDisplaying` and rebind them in `willDisplay`,
so removed rows stop answering element-id lookups and redisplayed rows regain their ids.
Reloads are deferred outside the tree borrow: UIKit may retire cells synchronously inside
`reloadData`, and recycling must be able to access Day's tree at that point.

## Programmatic scrolling

[`.scroll_to_end(Trigger)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.scroll_to_end) scrolls to the end of the list (above).
[`.scroll_to_row(binding)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.scroll_to_row) scrolls to any row:

```rust
let jump = Signal::new(None);
list(rows, row_view).scroll_to_row(jump);

button(res::str::first_unread()).action(move || jump.set(Some(unread_index())));
```

Each `Some` written to the binding scrolls there, realizing the row if it was virtualized
away, and the binding then resets to `None`, so the same row can be sent twice in a row. This
is the same contract as [`scroll`](https://docs.rs/day-pieces/latest/day_pieces/fn.scroll.html)`(...)`[`.scroll_target`](https://docs.rs/day-pieces/latest/day_pieces/trait.ScrollBuilder.html#tymethod.scroll_target)`(...)`:
a write is a command, and the list consumes it. The row is clamped to the list.

A binding of plain indices reveals the row with the least movement, animated. A
[`RowTarget`](https://docs.rs/day-pieces/latest/day_pieces/struct.RowTarget.html) says more:

```rust
let jump: Signal<Option<RowTarget>> = Signal::new(None);
jump.set(Some(RowTarget::new(250).align(RowAlign::Center)));
jump.set(Some(RowTarget::new(0).align(RowAlign::Start).instant()));
```

| `RowAlign` | where the row lands |
| --- | --- |
| `Nearest` (default) | the least movement that shows the whole row; a row already in view stays put |
| `Start` | its top edge at the top of the viewport |
| `Center` | centered in the viewport |
| `End` | its bottom edge at the bottom of the viewport |

A scroll is animated with the platform's own scroll animation unless the target says
[`.instant()`](https://docs.rs/day-pieces/latest/day_pieces/struct.RowTarget.html#method.instant), and never while the system's reduce-motion setting is on. Alignment and the
choice of animation need `Cap::ListScrollAlign`; where it is unsupported the row is revealed
where the platform's own "scroll to row" puts it. GTK and the web aim at a row from the heights
of the rows they have laid out and estimate the rest, so a jump to a far row with an automatic
height lands by estimate and settles on the row once it is laid out (see the table below).

Inside day-core a row scroll is one duty,
[`Toolkit::list_scroll(handle, ListScroll { row, align, animated })`](https://docs.rs/day-spec/latest/day_spec/trait.Toolkit.html#method.list_scroll),
with `row: None` for the end.

The dayscript `scroll_to` step goes through the same duty. `edge: top` scrolls a list to its
first row (a header above the items included) and `edge: bottom` to the end of its content;
a list has no `leading`, `trailing` or offset target. `scroll_to` with the id of an element
inside a row reveals that row with the least movement. A row must be realized to have an id, so
reach a row far from the viewport with an edge first, or with [`.scroll_to_row`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.scroll_to_row) from the app.

A `Splice` (see [Keyed updates](#keyed-updates)) animates its inserts, removals and moves as
native row changes on AppKit, UIKit and Android; the other toolkits apply it without
animation. A toolkit that applies a splice as row changes first checks it against the rows the
native list holds, and reloads, keeping the scroll position, when the two disagree. The source
already answers in the final state while the deltas land one at a time, so whatever the native
list asks about a row part way through (its height, whether it is a header, its content) is
answered for the row that index holds at that moment: AppKit maps each in-between row to the
source row it ends at, and GTK binds the rows it shows again once a splice of several runs has
landed. Otherwise a sectioned `Uniform` list sizes an inserted row as the header that ends up
at its index. A Reload whose rows are the same set in a new
order animates as native row moves on AppKit (`moveRowAtIndex` batch, the same animation a
drag commit gets).

### Toolkit support

| Toolkit | Viewport report (`Cap::ListViewport`) | Aligned and instant row scrolls (`Cap::ListScrollAlign`) | Supplementary rows | Animated splice |
| --- | --- | --- | --- | --- |
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | `Native`: the clip view's bounds and frame changes, and each realize pass | `Native`: the row's `rectOfRow:` aligned in the clip view, animated through its animator | their own reuse identifier, sized by `heightOfRow:`, never selected or dragged, no separator | yes, up to 64 deltas; a larger splice applies without animation. A change that arrives while the table is updating its visible rows is held and applied on the next main-loop turn |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | `Native`: the table's layout pass and `scrollViewDidScroll:` | `Native`: `scrollToRowAtIndexPath:atScrollPosition:animated:` | their own reuse identifier, self-sizing, never highlighted or selected, no separator | yes, as one batch of up to 64 deltas on the next main-queue turn, while the table shows each row's item where its row will land; a sectioned list or a larger splice reloads, keeping the offset |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | `Native`: the scroll listener and each layout pass, one report per main-loop turn | `Native`: `scrollToPositionWithOffset`, or `smoothScrollBy` over the aligned distance | a view type of their own sized to the content, with no ripple, click, selection or drag | yes, through `RecyclerView`'s item animator |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | `Native`, after each frame from the rows GTK laid out, so one frame behind the scroll | `Native`: a 250 ms glide on the frame clock. A row with an automatic height that is not laid out yet lands by estimate and settles a frame or two later; an instant scroll asks GTK 4.12 and later to lay the row out first | marked like section headers: no hover, press, selection or separator | no: model changes on the next main-loop turn |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | `Native`: the scroll bar's value and range and the rows at the viewport's edges | `Native`: aimed per pixel on the scroll bar, a 250 ms glide when animated; a wheel turn stops the glide | painted empty by the delegate, out of clicks, keys and drags | no: model row changes, which `QListView` draws without animation |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | `Native`: the `ItemsRepeater`'s realized rows and the scroll viewer's extent | `Native`: brought into view with an alignment ratio and the platform's animation | cells without row chrome, never selected or activated | no: a splice reloads from the source |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | `Native`: the list's scroll-index event | `Native`: `NODE_LIST_SCROLL_TO_INDEX`. A jump farther than three screens is not animated, because ArkUI builds every row it scrolls through | a cell pool of their own, inert | no: a splice reloads the adapter |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | `Native`: the list's own row table, after every scroll, resize and data change | `Native`: the browser's smooth scrolling | sized like section headers, with no role, separator or hover | no |

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
configured. [`reorderable`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.reorderable) turns on the platform's drag mechanism; probe `Cap::ListReorder` for
support. [`Reorder::new(moved)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Reorder.html#method.new) is the commit: row `from` landed at row `to`; apply the identical
rotation to the backing data (`let it = v.remove(from); v.insert(to, it);`) and persist it if
the order should survive a relaunch. It runs at the next event drain, never inside the native
drop callback.

[`Reorder::guard`](https://docs.rs/day-pieces/latest/day_pieces/struct.Reorder.html#method.guard) vets every proposed drop **synchronously, while the drag is live**: the native
affordance (the macOS gap, the no-drop cursor) reflects the answer before the user releases.
`ReorderVerdict::Deny` refuses the drop (the row springs back); `ReorderVerdict::Retarget(i)`
accepts it at a different index, the "pinned rows" pattern (the Showcase pins its first row
this way). Keep the guard pure: it runs inside the platform's drag callback, so read state and
return without mutating UI. [`Reorder::enabled`](https://docs.rs/day-pieces/latest/day_pieces/struct.Reorder.html#method.enabled) takes a constant or a reactive `bool` and is read
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
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | [`NSTableView`](https://developer.apple.com/documentation/appkit/nstableview) drag pipeline (pasteboard row, `validateDrop`/`acceptDrop`) | the `.gap` placeholder opens where the drop would land | live (validate retargets/denies) |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | drag delegate + `moveRow`/`targetIndexPathForMove` | long-press lift + gap, no editing mode | live (target-for-move) |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | `ItemTouchHelper` on the RecyclerView | long-press lift, elevation, incremental swaps | live per swap; `Retarget` reads as deny (the helper can't relocate the gap) |
| [GTK 4](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | `DragSource`/`DropTarget` (native DnD framework) | row snapshot as drag icon; forbidden cursor on deny (no insertion line yet) | live (motion) |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | [`QDrag`](https://doc.qt.io/qt-6/qdrag.html) over the [`QListView`](https://doc.qt.io/qt-6/qlistview.html) | grabbed-cell pixmap, 2px insertion line, no-drop cursor | live (drag-move) |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | WinRT drag events on recycled `ItemContainer` rows | system drag visuals + live no-drop cursor | live (DragOver); target is the actual row, including variable heights |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | `SetNodeDraggable` + `NODE_ON_DROP` | system drag preview; denied drops spring back | at drop (`SetDragResult`) |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | pointer-tracked (emulated — no native list reorder in the browser) | lifted cell + animated CSS gap, long-press on touch | live (every hovered slot) |
| mock | [`MockProbe::list_can_move`](https://docs.rs/day-mock/latest/day_mock/struct.MockProbe.html#method.list_can_move) / `list_move` | op log | live |

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
label, an optional per-row guard and an optional [`.enabled(..)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Reorder.html#method.enabled) availability. [`deletable`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.deletable) turns
on the platform's delete gesture; probe `Cap::ListDelete` for support. [`Delete::new(deleted)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Delete.html#method.new) is
the commit: row `index` is gone; apply the identical removal to the backing data
(`v.remove(index)`) and persist it if the change should survive a relaunch. It runs at the next
event drain, never inside the native swipe callback.

[`Delete::guard`](https://docs.rs/day-pieces/latest/day_pieces/struct.Delete.html#method.guard) is consulted **before the affordance is offered**, not after the gesture: a row
that answers `false` shows no delete action at all, rather than one that fails on use. Keep it
pure; it runs inside the platform's swipe callback.

[`Delete::label`](https://docs.rs/day-pieces/latest/day_pieces/struct.Delete.html#method.label) carries the app's own word for the action; it takes the same text sources as a
label (`res::str::delete()` directly, no [`.format()`](https://docs.rs/day-localization-ui/latest/day_localization_ui/struct.LocalizedText.html#method.format)), resolved in the locale current when the
list builds. A toolkit has no access to the app's Fluent catalog and so cannot translate
anything itself; left unset, each toolkit falls back to its platform's wordless idiom (a trash
glyph), which needs no translation.

The delete half of `ListSource` (`ListSource::delete`, present only when the list carries a `Delete`) has
two calls: `can_delete(index) -> bool` for the offer, and `delete_row(index)` for the commit,
which drops the row from Day's snapshot **before returning**, so `len`/`token_at`/`bind_row`
answer for the shorter list while the native removal animates, and defers the app's
`Delete` handler through the event queue; the reorder half follows the same rule.

**[WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") supports deletion** through [`SwipeControl`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.swipecontrol) and the row context menu. Other desktop
toolkits answer `Unsupported` for the delete affordance: GTK and Qt have
no swipe idiom at all, and macOS's row actions (the swipe-actions section below rides them) do
not carry the delete affordance yet. A list that must be editable everywhere pairs
[`.deletable(..)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.deletable) with an explicit control (a menu item, a per-row button) and lets the mobile
toolkits add the gesture on top.

The dayscript step `delete_row: { id, row }` drives the same guard → commit path without a
native gesture (a guard refusal fails the step, non-retryably), which is how CI asserts deletion
on every target, including the desktops, where there is no gesture to simulate.

| Toolkit | Mechanism | Affordance |
|---|---|---|
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | `trailingSwipeActionsConfigurationForRowAtIndexPath` | row tracks the finger, destructive action reveals behind it, full swipe commits |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | `ItemTouchHelper` swipe (`START`, so it is the trailing edge in RTL too) | row slides to reveal a red field carrying the label; `notifyItemRemoved` animates the close-up |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | [`SwipeControl`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.swipecontrol) trailing [`SwipeItems`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.swipeitems) and [`MenuFlyout`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.menuflyout) | reveal then press Delete; the guard is checked both when offering and invoking it |
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") · [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") · [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") · web | — | `Unsupported`: the delete affordance does not ride [macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit")'s row actions yet, and the rest have no swipe idiom; use a menu item or a row button |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | `NODE_LIST_ITEM_SWIPE_ACTION`: a trailing swipe-action item on each deletable row | the row slides to reveal the labeled action; pressing it runs the guard and commits |

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

[`symbol`](https://docs.rs/day-pieces/latest/day_pieces/struct.SwipeAction.html#method.symbol) puts a glyph on the button where the platform draws one: above the label on macOS
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

[`editing`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.editing) puts the list in the platform's own editing mode, the one an Edit button enters on
iOS: each row the `Delete` offers shows the red delete control at its leading edge, each row
the `Reorder` moves shows a grip at its trailing edge, and a tap still activates the row, so a
list of things to configure can open each one from the same screen. The value is reactive; a
change animates the controls in or out. The delete control commits through the same
[`Delete::guard`](https://docs.rs/day-pieces/latest/day_pieces/struct.Delete.html#method.guard) → handler path as the swipe, and the grip through the same [`Reorder::guard`](https://docs.rs/day-pieces/latest/day_pieces/struct.Reorder.html#method.guard) →
handler path as a long-press drag.

Probe `Cap::ListEditing`. Only UIKit has a separate editing mode: on the other toolkits a
list's rows already drag, swipe, and open their menus as they are, so [`editing`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.editing) is ignored
there and the list renders as it would without it. A screen that manages a list everywhere
uses [`.editing(true)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.editing) for iOS and keeps its explicit controls for the toolkits that answer
`Unsupported` for `ListDelete`.

| Toolkit | Mechanism |
|---|---|
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | [`UITableView.setEditing(_:animated:)`](https://developer.apple.com/documentation/uikit/uitableview), `editingStyleForRowAtIndexPath`, `commitEditingStyle`, `allowsSelectionDuringEditing` |
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") · [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") · [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") · web · [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") · [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | — (`Unsupported`; ignored) |

While editing, UIKit narrows each cell's content view for the two controls. Rows are
`DayListCell`s that report the narrower width through `ListSource::layout_cell`, so Day lays the
row out to fit between them rather than under them.

## Appearance

[`.inset_grouped()`](https://docs.rs/day-pieces/latest/day_pieces/struct.List.html#method.inset_grouped)
draws the list as the platform's inset grouped list: each section a rounded group of rows set in
from the edges on the grouped background, as iOS Settings and Safari's Bookmarks and History draw
them. It suits a list of places or settings more than a feed.

| Toolkit | Inset grouped |
|---|---|
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | `UITableViewStyleInsetGrouped`; section headers sit above their group and scroll with it rather than pinning |
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | `NSTableViewStyleInset`: rows set in from the edges with a rounded selection |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material"), [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk"), [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt"), [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui"), web, [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | the plain list |

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
[`.separators()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.separators) forces them on, [`.no_separators()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.no_separators) off (an iOS list whose rows draw their own
separation turns the native line off rather than showing both).

| Toolkit | Mechanism |
|---|---|
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | `gridStyleMask = solidHorizontalGridLineMask`, `separatorColor` |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | the table's own `separatorStyle` (default on; `false` sets `.none`) |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | a `border-bottom` on the ListView's `row` CSS nodes |
| [web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | a `border-bottom` inside each cell frame (border-box) |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | the delegate paints a hairline along each row's bottom edge (the palette's `Mid`, translucent) |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") · [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | *not lowered yet* — a force is ignored; rows separate by their pitch |

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
selection and programmatic selection do not activate rows. [`on_activate`](https://docs.rs/day-pieces/latest/day_pieces/trait.ListBuilder.html#tymethod.on_activate) receives the row's
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
