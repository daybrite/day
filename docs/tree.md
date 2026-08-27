---
title: "Tree"
description: "A hierarchical tree piece over each platform's native tree view, with live selection, expansion, drag-to-reparent, keyboard navigation and type-ahead."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Tree (plan)

> [!IMPORTANT]
> **Status: M0 + M1 + M2 + M3 shipped (2026-08), M5's Android half shipped (2026-08), M6
> partially.** The seam, driver, flattener, `tree()` piece, mock probes, THREE native
> backends — AppKit `NSOutlineView`, GTK `GtkListView`+`GtkTreeListModel`+`GtkTreeExpander`,
> and UIKit's list-layout `UICollectionView` over one diffable SECTION snapshot — and the
> COMPOSED tree (M2, on web-dom, the qt toolkit and Android) are implemented and proven by
> the mock e2e suite and Day Sketch's layer panel: ONE `dayscript/tree.yaml` passes verbatim
> on macos-appkit, macos-gtk, ios-uikit, web-dom, macos-qt and android-mdc (89/89), and the
> leading pane rides the `.edge(PaneEdge::Leading)` inspector on all six. The remaining
> milestones — XAML/ArkUI (M4), the Showcase page + `day-tweak-tree-style` (M5's other
> half) — are still the build order. M3's as-built notes:
>
> - **Native drag-to-move is still AppKit-only**: GTK and UIKit answer `Cap::Tree` Native
>   but not yet `Cap::TreeMove` — the dayscript `tree_move:` step drives the seam on every
>   target regardless, which is what the walkthrough parity proves. Their native drag halves
>   are the next tree work.
> - GTK rebuilds its `TreeListModel` per Reload (deferred to an idle — GTK binds
>   synchronously, the `schedule_list_resize` rule) and restores disclosure + selection
>   top-down from token records; rows report disclosure through `TreeListRow`'s `expanded`
>   notify, and the factory's unbind hook is where `recycle` clears a hidden row's ids.
> - UIKit owns disclosure END TO END: the outline-disclosure accessory carries a custom
>   action handler that only EMITS `TreeExpanded`, and the patch re-applies the section
>   snapshot — a native tap and the `expand:` step share one path by construction. Cells are
>   `DayTreeListCell`s that pin their self-sizing height to the uniform row height and
>   re-lay day content at the content view's own width per layout pass.
> - The iOS walkthrough wraps its `insp-tab` asserts in `only_on: [uikit]` inspector-sheet
>   toggles: a FULLSCREEN modal removes the presenting view from the window, and a detached
>   `UICollectionView` never creates cells — so the tree sections run with the sheet closed.
>   Visibility toggles only; the undo history stays the same depth on every target.
> - The tree grew **`.row_context_menu(|key| …)`** (2026-08): per-row summon-time context
>   menus, served natively on all three backends — see [docs/menus.md](menus.md) "Dynamic
>   context menus". Day Sketch's rows and its canvas share one selection menu through it.
> - A composed LEADING inspector pane stays a side pane at EVERY width (no compact-sheet
>   re-homing): a layer panel beside a narrow canvas beats a modal that detaches the window
>   — see [docs/inspector.md](inspector.md). As-built deltas
> from the text below, applied where implementation decided differently:
>
> - The pane-edge enum shipped as **`PaneEdge`** (`Trailing`/`Leading`) on `InspectorProps` —
>   day's `Edges` was already the gesture-defer bitset.
> - Selection reports through a dedicated **`Event::TreeSelection(Vec<u64>)`** (full token
>   set), not the list's index-addressed `SelectionSet`.
> - `TreeSource` grew **`layout_cell(cell, width)`**: indentation makes every tree cell's
>   width per-row, so the cell's own native layout pass re-lays the Day row — a seam the
>   list never needed.
> - The piece grew **`.expandable(|key| …)`** (the branch/leaf rule; default "has children
>   right now") and **`.row_id(|key| …)`** (per-row dayscript ids, re-applied on recycle —
>   what `expand:`/`tree_move:` resolve rows by). `.on_activate` and the four delegate hooks
>   (`row_height_for`/`can_expand`/`can_select`/`is_group_row`) are NOT yet built — they land
>   with the backends that consume them.
> - The dayscript `type_ahead:` step is not yet built (AppKit's native type-select answers
>   from `.type_ahead` regardless); `expand:` addresses rows by `.row_id` string, not token.
> - The expansion echo rule sharpened: with an app-owned `.expanded` signal, the piece's
>   native-state record moves only when a PATCH applies — a native disclosure and the
>   dayscript `expand:` event then share one path, and redundant patches no-op natively.
> - `Subcontrol` + the accessors shipped for **AppKit only** so far (`with_native_subcontrol`,
>   `.appkit_subcontrol`); `.row_tweak` and the other backends' accessors land with M2+.
> - AppKit rows draw the modern ROUNDED selection through their own `NSTableRowView`
>   subclass rather than `NSTableViewStyle::Inset` — the style pads by making the table
>   wider than its clip, which fights day's fixed-frame layout. The outline itself is a
>   `DayOutlineView` that pins its width to the clip on EVERY layout pass (a table sizes
>   itself from its columns and autoresizing only tracks deltas, so occasional deferred
>   syncs left windows where a stale width drew a clipped pill). And tree cell anchors use
>   a CENTERING layout (`CellCenter`), not the list's `PassThrough`: row content hugs its
>   own height, and pinning a 16pt row to the top of a 28pt cell read as misalignment.

A tree is a list that nests: rows at several depths, disclosure controls that open and close
them, selection shared with whatever else shows the same data, and a drag that moves a row
*into* another one rather than only above or below it. A drawing app's layer panel, a file
browser's source list and an outliner's document are all the same control.

Day has [`list`](list.md) for flat rows and [`nav_menu`](navigation.md) for a fixed sidebar of
destinations. Neither nests, and neither can express "drag this node into that group", so a
tree needs its own piece: `tree(source, row)`, `kinds::TREE`.

## The driving case: Day Sketch's layer panel

Day Sketch keeps its scene in one table of nodes, each with a `parent` and a fractional `z`
([docs/model.md](model.md)). A group is a node whose children point at it. That is already a
tree, and the app wants it on the leading edge of the window:

- every node listed, groups nesting their members, in the canvas's own back-to-front order;
- selection synchronized both ways, because the canvas and the tree read one selection signal —
  click a row, the shape's handles appear; shift-click two shapes, both rows highlight;
- drag a row onto a group to reparent it, or between two rows to restack it, which is the same
  `parent` + `z` write the Arrange menu already makes, and one undo unit.

Nothing about that is specific to Day Sketch. It is what every tree does.

## What each toolkit brings

The question that decides the design is not "does this platform have a tree widget" but "can
its tree widget host a row that Day built". Day rows are real native subtrees bound into
recycled cells ([docs/list.md](list.md)), so a tree that paints its rows through a delegate is
a worse fit than a flat list that hosts child views.

| Toolkit | Native tree | Hosts Day-built rows | Verdict |
|---|---|---|---|
| **AppKit** | `NSOutlineView` | yes — view-based rows, same `makeView`/`viewFor` path as `NSTableView` | **native**, and close to a drop-in over Day's existing table code |
| **UIKit** | `UICollectionView` list, `.sidebar` appearance | yes — `UICollectionViewListCell` hosts a content view | **native**, but a different widget from Day's `UITableView` list: new realize, new data source |
| **GTK 4** | `GtkListView` + `GtkTreeListModel` + `GtkTreeExpander` | yes — the list-item factory binds arbitrary widgets | **native**; `GtkTreeView` is deprecated at the 4.10 API level Day targets, so the model-based path is also the current one |
| **XAML (WinUI)** | `TreeView` / `TreeViewNode` | yes — items are content controls | **native**; a better fit than `ListView` was, since content hosting is the thing WinUI's tree does well |
| **Qt** | `QTreeView` | awkwardly — rows are painted by delegates; arbitrary widgets need `setIndexWidget` per row, which defeats virtualization | **emulated** to start, but see the [note on Qt](#the-note-on-qt) — Day's Qt list already declines to virtualize |
| **Android** | none | — | **emulated**; Material has no tree, and the platform idiom IS a flat `RecyclerView` with indentation and a chevron |
| **ArkUI** | `TreeView` + `TreeController` (`@ohos.arkui.advanced`, API 10+) | yes — `NodeParam.container` is a builder slot, which can hold a `ContentSlot` Day mounts a row into | **native**, through Day's existing ArkTS bridge; see [below](#arkui-reaching-an-arkts-component-from-the-c-node-api) |
| **web-dom** | none | — | **emulated**; the browser has no tree element, only `role="tree"` and the ARIA pattern |
| **mock** | simulated | yes | drives the tests, as it does for `list` |

Five toolkits carry a real tree that will host Day's rows. Two have no tree at all, and one
has a tree its cell model fights. That split decides the architecture. The seam is
hierarchical, so the native trees drive it directly, and ONE flattener in `day-core` turns the
same seam into indented rows for the rest — not three hand-rolled imitations.

An "emulated" verdict here means the tree *semantics* are Day's; it does not mean no native
widget. Qt's emulated tree still scrolls a real Qt container of real Qt row widgets, Android's
rides the same native `RecyclerView` the `list` piece uses, and web-dom's rows are real DOM.
That matters for [customization](#deep-customization-per-toolkit): there is always something
real to tweak.

### What the native trees give away for free

Worth naming, because the emulation has to earn each one back:

- **Expand and collapse**, animated, with the platform's own disclosure glyph and indent step.
- **Keyboard**: left/right to close and open a row, arrows through the visible rows, type-select.
- **Accessibility**: `NSOutlineView` reports rows with a disclosure level to VoiceOver; WinUI's
  `TreeView` reports expand state to Narrator. A flat list of indented rows announces as a flat
  list of rows unless Day says otherwise — see [Accessibility](#accessibility).
- **Drop targeting**: `NSOutlineView` hands the app `(parent item, child index)` and a sentinel
  for "onto this row", which is exactly the vocabulary a reparent needs. GTK and the emulation
  compute that themselves from the pointer's position in the row.
- **Spring-loading**: hovering a collapsed group during a drag opens it. Native on AppKit;
  a timer everywhere else, and a v2 item.

### ArkUI: reaching an ArkTS component from the C node API

Day's ArkUI backend speaks the **C node API** (`ARKUI_NODE_*`), whose list vocabulary has no
tree in it — which is what made this look like a gap at first. `TreeView` lives one layer up,
in the ArkTS advanced component set, and Day already crosses that layer twice: the app's
`@Entry` page mounts Day's native tree through a `NodeContent`/`ContentSlot`, gives every
pushed navigation page its own per-page `NodeContent`, and answers up-calls from Rust for
things the C API cannot do at all (the file picker runs on the ArkTS side and hands bytes
back).

A tree uses the same two mechanisms:

- **Structure** comes from `TreeController` — `addNode(NodeParam { parentNodeId, currentNodeId,
  isFolder, … })` per node, then `buildDone()` — driven from Rust over the existing bridge.
- **Row content** comes from `NodeParam.container`, a builder slot ("set subcomponent binded on
  tree item"). It holds a `ContentSlot` bound to a per-node `NodeContent`, keyed by node id
  exactly as `navContents` keys pages today, and Day mounts the row's C-API subtree into it.
- **Events** arrive through `TreeListener`: `NODE_CLICK` for selection and `NODE_MOVE` with
  `CallbackParam { currentNodeId, parentNodeId, childIndex }` — which is precisely the
  `(node, parent, index)` commit this design's seam is shaped around.

Two honest costs. `TreeController` builds nodes imperatively with no cell reuse, so ArkUI's
tree does not recycle and `Cap::ListRecycling` should say so; a layer panel is fine, a
hundred-thousand-row tree is not. And the listener fires *after* a move, so `move_guard`
cannot run live there — the same drop-time verdict Day's ArkUI list reorder already documents.
The component also ships its own add/delete/rename affordances (`NODE_ADD`, `NODE_DELETE`,
`NODE_MODIFY`, `editIcon`), which Day either suppresses or maps deliberately.

### The note on Qt

`QTreeView` renders through delegates, so hosting a Day-built row means `setIndexWidget` per
row, which Qt documents as inappropriate for large models because it defeats virtualization.
That reads like a disqualification until you notice that **Day's Qt list already declines to
virtualize** — it builds a real widget per row into an emulated scroller. On that basis a
`QTreeView` with per-row index widgets costs what Qt's list costs today and buys native
expansion, indentation, keyboard handling and `QAccessible::Tree`. It is worth a spike before
the emulated path is written off as Qt's permanent answer; the seam does not change either way.

## Core built-in, not a satellite piece

Day has two places a piece can live: `crates/day-pieces` with a `kinds::…` of its own, or a
satellite crate that registers per-backend renderers through `renderer!`
([docs/extending.md](extending.md)). Satellites are the default answer — the stepper, the color
picker and the web view are all satellites — so the burden is on the tree to justify core.

It clears it on four counts, three of which a satellite cannot reach at all:

| Needs | Reachable from a satellite? |
|---|---|
| A pull seam the backend calls synchronously (`children_len`, `child_token`, `bind_row`) | **yes, awkwardly** — the props struct can carry `Rc<dyn Fn…>` closures, so a satellite could ship its own seam without a `Toolkit` duty |
| Binding a Day row into a native cell (cell-anchor adoption, `BuiltRow`, scope ownership) | **no** — `Tree::install_list` and the cell machinery are day-core's; `install_tree` has to sit beside it |
| `Cap::Tree` / `Cap::TreeMove`, `Role::Tree` / `Role::TreeItem` | **no** — both enums live in day-spec, and a satellite cannot add variants |
| dayscript `expand:` / `tree_move:` steps | **no** — `Step` lives in day-script, and the walkthrough has to drive expansion and moves on every target |

The seam alone would not settle it. The cell machinery, the capability and a11y vocabulary, and
the test steps do: three of the four are spec-and-core edits whatever crate the piece nominally
lives in, and a satellite that needs three core edits to work is a core piece wearing a costume.

`kinds::TREE` therefore joins `builtin_kinds!` beside `kinds::LIST`, and the piece ships in
`day-pieces` — with the same consequence every new builtin kind has: the backends whose realize
matches are exhaustive stop compiling until each names the kind, which is the checklist, not a
surprise.

## Authoring

The row builder is `list`'s: an `ItemSlot`/`ModelSlot` bound once per physical cell and rebound
as cells recycle, so a ten-thousand-node tree builds only what it shows. What changes is the
source, which is hierarchical, and the identity, which is a **token**, not a row index. A tree
cannot key rows by position — expanding one row renumbers everything below it — and every
native API here agrees: `NSOutlineView` keys by item, diffable snapshots by identifier,
`GtkTreeListRow` by item, `TreeViewNode` by content, `TreeController` by node id.

### A closure-backed tree

`branches(items, key, parent)` is the tree counterpart of `items(…)`: a tracked flat
collection, a token per item, and a *parent token* per item (`None` = root). The flattener
derives children by grouping, in the items' own order.

```rust
#[derive(Clone, PartialEq)]
struct Entry { id: u64, parent: Option<u64>, name: String, folder: bool }

let entries = Signal::new(seed_entries());
let open    = Signal::new(HashSet::from([DOCS, DOCS_GUIDES]));   // expansion, app-owned
let picked  = Signal::new(Vec::<u64>::new());                    // selection, app-owned

tree(
    branches(move || entries.get(), |e| e.id, |e| e.parent),
    |row: ItemSlot<Entry, u64>| {
        row((
            vector(move || if row.field(|e| e.folder) { gv::folder } else { gv::doc })
                .frame(16.0, 16.0),
            label(move || row.field(|e| e.name.clone())),
        ))
        .spacing(6.0)
    },
)
.expanded(open)
.selected(move || picked.get())               // app state → native selection, echo-free
.on_selection(move |keys| picked.set(keys))   // native selection → app state
.on_activate(move |key| open_entry(key))      // double-click / Enter
.type_ahead(|slot| slot.field(|e| e.name.clone()))
.id("library")
```

Both directions of selection point at one signal, so anything else reading `picked` stays in
step with the tree for free. `expanded` works the same way: the user's disclosure clicks update
`open`, and the app writing `open` drives the native rows.

### A store-backed tree

A day-model store passes through a tracked *children projection* — the tree counterpart of
`store.rows(projection)` — mapping a parent key (`None` = root) to its ordered child keys:

```rust
// Day Sketch: the scene store IS the tree. children_of already exists for the canvas.
tree(model::nodes().tree(model::children_of), |slot: ModelSlot<Node>| {
    row((
        kind_glyph(slot),                                  // rect / oval / line / group
        label(move || slot.name().read()),                 // wakes only for THIS row
        spacer(),
        swatch(move || slot.fill().read()),                // the shape's fill, live
    ))
    .spacing(6.0)
})
.expanded(model::open_groups())
.selected(move || model::selection().get())
.on_selection(move |keys| model::selection().set(keys))
.movable(true)
.move_guard(|node, parent, _| {
    // Not into itself, not into a descendant, not into a leaf.
    Move::deny_if(parent.is_some_and(|p| p == node || model::is_descendant(p, node)))
})
.on_move(|node, parent, index| model::reparent(node, parent, index))
.id("layers")
```

`ModelSlot`'s costs land exactly as they do under `list`: a field edit patches the one control
showing it, a change the projection reads re-runs only the projection, and a recycled cell
leaves no observation claims behind.

### Maintenance: the flows an app actually runs

**Edits reload; expansion and selection survive.** A store write (or a change the `branches`
items closure reads) refreshes the snapshot and applies `TreePatch::Reload`; the expansion set
and the selection are re-applied by token, so nothing collapses and nothing deselects unless
its token is gone. v1 reload is whole-tree, as `list`'s is; the keyed splice is a named
refinement (see [Risks](#risks-worth-deciding-early)).

```rust
// Adding a node under a group: one store write. The tree reloads itself.
let id = model::place_shape_in(group, NodeKind::Rect, 40.0, 40.0);
model::selection().set(vec![id]);      // …and the new row is selected via the same signal
reveal.set(Some(id));                  // …and scrolled into view, ancestors expanded:
```

**Reveal.** `.reveal(Signal<Option<K>>)` is `list`'s `scroll_to_row` grown up for a tree:
setting it expands every ancestor of the token (through the same expansion signal, so the app
sees the change), then scrolls the row into view. "Find in canvas → show in layers" is one
signal write.

**Programmatic expansion.** The expansion signal is plain state, so bulk operations are plain
code, and persistence is the app's choice of where to put the set:

```rust
expand_all.action(move || open.set(model::all_group_keys()));
collapse_all.action(move || open.set(HashSet::new()));
// survive relaunch: persist it like any other pref
watch(move || open.get(), |set| day::prefs::set("layers.open", &encode(set)));
```

**Programmatic moves** go through the same commit the drag uses, so dayscript, an app's own
"Move to group" menu item, and the drop gesture are one code path:

```rust
menu_item(tr("move_to_group")).action(move || {
    for id in model::selection().get_untracked() {
        model::reparent(id, Some(group), None);          // None = append
    }
});
```

## The seam: `TreeSource`

Recycling trees pull, exactly as recycling lists do, so the tree extends the same synchronous
seam `list` added ([docs/list.md](list.md)) rather than inventing a second one:

```rust
pub struct TreeSource {
    /// How many children `parent` has (`None` = the root).
    pub children_len: Rc<dyn Fn(Option<u64>) -> usize>,
    /// The i-th child of `parent` — the stable token every backend keys its row by.
    pub child_token: Rc<dyn Fn(Option<u64>, usize) -> u64>,
    /// Whether this token can hold children at all — what draws (or omits) the disclosure.
    pub expandable: Rc<dyn Fn(u64) -> bool>,
    /// Build-or-rebind the row for `token` into this native cell.
    pub bind_row: Rc<dyn Fn(u64, RawHandle)>,
    pub recycle: Rc<dyn Fn(RawHandle)>,
    /// The row's type-ahead string (see Keyboard below).
    pub type_select_text: Rc<dyn Fn(u64) -> String>,
    /// The drag half, present only under `.movable(true)`.
    pub moves: Option<TreeMoves>,
}

pub struct TreeMoves {
    /// The live verdict, called inside the platform's drag callback: may this node land
    /// under `parent` at `index`? `index: None` means "onto the parent" (append).
    pub can_move: Rc<dyn Fn(u64, Option<u64>, Option<usize>) -> MoveVerdict>,
    /// The commit, after the drop is accepted. Rewrites Day's snapshot before returning and
    /// defers the app's `on_move` through the event queue, as `list`'s reorder does.
    pub move_node: Rc<dyn Fn(u64, Option<u64>, Option<usize>)>,
}
```

`index: Option<usize>` is the whole drop vocabulary: `Some(i)` drops between rows, `None`
drops onto the parent. It maps to `NSOutlineViewDropOnItemIndex` without translation, to a
diffable snapshot's `append(to:)`, to `GtkTreeListRow`'s child model, to a `TreeViewNode`'s
`Children.Insert`, and to `TreeController`'s move callback.

New spec surface, all additive: `kinds::TREE`, `TreeProps { multi_select, indent, row_height }`,
`TreePatch::{Reload, Expand(u64, bool), Selected(Vec<u64>), Reveal(u64)}`,
`Event::TreeExpanded { token, expanded }`, `Event::TreeMove { node, parent, index }`,
`Toolkit::attach_tree`, `Cap::Tree` / `Cap::TreeMove`, `Role::Tree` / `Role::TreeItem`, and
`Subcontrol` (below).

## One flattener for the rest

`day-core` turns a `TreeSource` plus the expansion set into the flat row sequence the emulated
backends render: a walk from the root that descends only into open rows, producing
`(token, depth)` pairs. The emulated backends then reuse the **list** cell machinery unchanged
and wrap each row in an indent plus a disclosure control built from ordinary Day pieces.

That is one implementation of expansion, indentation, keyboard handling and drop targeting,
shared by Qt, Android and web-dom, exercised by the mock backend's tests. A bug fixed in the
flattener is fixed on three platforms — and the flattener is written kind-agnostic
(rows-with-depth over any token tree), because it is the seed of the shared composed tier
described in [Stepping back](#stepping-back-what-the-tree-stresses-in-days-architecture).

## Keyboard and type-ahead

A tree is a keyboard control before it is a mouse one, and the platforms disagree about how
much of that they hand over.

| Behavior | AppKit | UIKit | GTK | XAML | ArkUI | emulated (Qt · Android · web) |
|---|---|---|---|---|---|---|
| Arrow up/down through visible rows | native | native (hardware kbd) | native | native | native | Day |
| Left/right to collapse and open | native | native | native | native | native | Day |
| Home/End, page up/down | native | native | native | native | native | Day |
| Type-ahead to a row | native, via `typeSelectStringForTableColumn:row:` | — | — | native | — | Day |

So Day writes the keyboard once, in `day-core`, and each backend opts out of the parts its
widget already does. The emulated handler is a focus-scoped key reader on the tree node:

- **Up/Down** move the cursor within the flattened visible rows; **Shift** extends the selection.
- **Left** collapses an open row, or moves to the parent when the row is already closed —
  the behavior every tree has, and the one people reach for to climb out of a group.
- **Right** opens a closed row, or moves to its first child.
- **Home/End** jump to the first and last visible row; **Page** keys move by the viewport.
- **Enter** activates (`on_activate`), **Space** toggles selection under `multi_select`.

**Type-ahead** needs a string per row, which only the app knows, so the source carries
`type_select_text` and the piece fills it from `.type_ahead(|slot| …)`, defaulting to the row's
first label. AppKit and XAML answer their own type-select callbacks from that closure;
everywhere else `day-core` keeps a small buffer that appends printable keys, resets after
~800 ms of silence, and selects the first visible row whose text starts with it, wrapping from
the cursor. One source of truth, two implementations of the mechanics.

## Customization: three layers, and what each one owns

Tree views are the most configurable control on every desktop toolkit, and no portable API is
going to span `NSOutlineView`'s style, row-size, group-row, autosave and disclosure options
*plus* GTK's factories *plus* WinUI's node templates. Trying produces the worst of both: an API
too wide to implement everywhere and still too narrow for anyone who cares. So the tree splits
its surface deliberately.

| Layer | What belongs there | Reaches |
|---|---|---|
| **Portable API** | what every tree has and an app would otherwise fake: expansion, selection, moves, indent, row height, keyboard, type-ahead | all nine targets |
| **Hooks** | the per-row *decisions* native trees express as delegate callbacks: may this row expand, may it be selected, how tall is it, is it a group row | all nine, mapped to each toolkit's callback or run by the emulation |
| **Tweaks** | everything else — the platform's own vocabulary, on the real widget | one toolkit at a time, no-op elsewhere |

The dividing rule: **if a knob changes what the tree MEANS, it is portable; if it changes how
one platform DRAWS it, it is a tweak.** Row height changes meaning (rows overlap or clip if a
backend ignores it), so it is portable. `NSTableViewStyle::SourceList` changes appearance, so
it is a tweak, and an app that wants it on Windows asks WinUI for its own equivalent rather
than Day inventing a lowest common denominator of both.

### Why hooks exist as their own layer

A tweak reaches the widget. It cannot reach the widget's *delegate*, because Day owns that
object — day-appkit's sidebar already installs a `DayNavMenuData` as both
`NSOutlineViewDataSource` and `NSOutlineViewDelegate`, and an app that assigned its own would
tear the tree's data out from under Day. Yet the delegate is exactly where AppKit puts
`outlineView(_:heightOfRowByItem:)`, `shouldExpandItem:`, `shouldSelectItem:` and
`isGroupItem:`.

So the tree names that surface itself, and each backend routes it to its own mechanism:

```rust
tree(source, row)
    .row_height_for(|n: RowInfo| if n.expandable { 28.0 } else { 22.0 })
    .can_expand(|n| n.token != LOCKED_FOLDER)
    .can_select(|n| !n.expandable)          // folders are containers, not selections
    .is_group_row(|n| n.depth == 0)
```

AppKit answers its delegate methods from these; GTK sets the row widget's height request and
`sensitive`; UIKit's list configuration reads them per item; WinUI applies them to the node's
container; and the emulation consults them in the flattener. One vocabulary, five native
implementations, one fallback — and an app that sets none of them gets a plain tree.

### What the tweak system reaches today, and what it does not

[Tweaks](tweaks.md) hand a closure the node's native handle plus its concrete class, per
toolkit, typed on AppKit/UIKit/GTK/Android and raw on Qt/XAML/ArkUI. For a tree that covers
most of the interesting surface immediately, because most of `NSOutlineView`'s fiddliness is
*properties*. Four things it does not cover, all of which the tree makes acute:

1. **A composite backing exposes only its outer handle.** Day's sidebar realizes an
   `NSOutlineView` inside an `NSScrollView` and returns the scroll view as the node's handle,
   so `with_native` hands a tweak the scroller, and reaching the tree means guessing
   `documentView()`. The tweaks doc's own rule — match the class, do not assume it — cannot be
   followed when the class you want is not the one you are given. This is not new to the tree:
   `list` and `text_area` are composite on the same backends today.
2. **No handle for a row.** Rows are native cells the backend creates (`NSTableRowView`,
   `UICollectionViewListCell`, `GtkListItem`), and Day builds the row's *content* inside them.
   A tweak inside the row builder reaches the content widgets; nothing reaches the cell.
3. **No participation in delegate decisions.** Covered by hooks above, and stated as a rule in
   [docs/tweaks.md](tweaks.md): a tweak must never install its own data source or delegate on a
   widget Day drives.
4. **`.tweak` runs once at mount.** Correct for a widget that lives as long as the node, wrong
   for anything that must be re-applied per row bind.

### The three additions this plan makes

**Native subcontrols** (`day-spec`, each backend's `ext`). A *subcontrol* is one addressable
widget within a composite backing — Qt's own name for exactly this concept
(`QStyle::SubControl`), and chosen here because the natural word, "part", already means a
headless platform-service package in Day (`parts/day-part-*`, DESIGN.md §15) and must not be
overloaded. A kind whose backing is composite reports its subcontrols, and the accessors take
one:

```rust
day_appkit::with_native_subcontrol(node, Subcontrol::Content, |view, class, mtm| …)
button("x").appkit_subcontrol(Subcontrol::Content, |…| …)    // the Decorate form
```

`Subcontrol::Host` is today's behavior and stays the default; `Subcontrol::Content` is the
widget inside the scroller; `Subcontrol::Header` is the header view where one exists. Each kind
documents its subcontrols per toolkit in the same table that documents its native class, so the
mapping is *reported* rather than guessed — the property that makes Day's tweaks stronger than
introspection libraries in the first place. This lands with the tree and retrofits `list` and
`text_area` in the same change.

**Row tweaks** (`day-pieces`, `day-core`). The tree and the list gain
`.row_tweak(|native_row, class, RowInfo|)`, invoked when a cell is bound — after the row's
content exists, with the cell's own handle and the row's token, depth, expansion and selection
state. It runs on every bind, which is what makes it correct for recycled cells.

**web-dom joins the tweak system.** The tweaks table stops at seven toolkits; the browser is
missing. The emulated tree makes that a real gap, and CSS is the web's native customization
language, so day-dom gains a minimal accessor and two helpers over new shim calls:

```rust
day_dom::with_element(node, Subcontrol::Host, |el| {
    el.add_class("layers-panel");
    el.set_style("scrollbar-width", "thin");
});
```

### Deep customization per toolkit

What follows is the litmus test for the whole design: for each toolkit, the fiddly
platform-specific configuration its tree users actually reach for, written against this plan's
API. Every snippet compiles only under its own backend's feature and is a silent no-op
everywhere else; delete all of them and the tree still works on all nine targets, just plainer.

#### AppKit — `NSOutlineView` in an `NSScrollView`

Subcontrols: `Host` = `NSScrollView`, `Content` = `NSOutlineView`, `Header` = the
`NSTableHeaderView` (absent by default; the tree ships headerless). Rows are `NSTableRowView`.

```rust
use day_appkit::{AppKitExt, Subcontrol};
use objc2_app_kit::{NSOutlineView, NSScrollView, NSTableRowView, NSTableViewRowSizeStyle,
                    NSTableViewSelectionHighlightStyle, NSTableViewStyle};
use objc2_foundation::{NSSize, ns_string};

tree(layers, row)
    .appkit_subcontrol(Subcontrol::Content, |view, class, _mtm| {
        // `class` is "NSOutlineView"; the Host subcontrol would have handed us NSScrollView.
        let Some(ov) = view.downcast_ref::<NSOutlineView>() else { return };
        unsafe {
            // The Finder-sidebar treatment, which is most of what people want from a
            // macOS tree and none of which belongs in a portable API:
            ov.setStyle(NSTableViewStyle::SourceList);
            ov.setSelectionHighlightStyle(NSTableViewSelectionHighlightStyle::SourceList);
            ov.setRowSizeStyle(NSTableViewRowSizeStyle::Medium);
            ov.setIndentationPerLevel(13.0);
            ov.setIndentationMarkerFollowsCell(true);
            ov.setFloatsGroupRows(true);
            ov.setIntercellSpacing(NSSize { width: 0.0, height: 2.0 });
            // AppKit persists expansion itself, keyed per window — an app can lean on this
            // INSTEAD of persisting Day's expansion signal, but not both:
            ov.setAutosaveName(Some(ns_string!("layers")));
            ov.setAutosaveExpandedItems(true);
        }
    })
    .appkit(|view, _class, _mtm| {                      // Subcontrol::Host — the scroller
        if let Some(sv) = view.downcast_ref::<NSScrollView>() {
            unsafe {
                sv.setDrawsBackground(false);
                sv.setAutomaticallyAdjustsContentInsets(true);
            }
        }
    })
    .row_tweak(|row, class, info| {
        // The cell itself, on every bind — the thing plain tweaks could never reach.
        if class == "NSTableRowView" && info.is_group {
            if let Some(rv) = row.downcast_ref::<NSTableRowView>() {
                unsafe { rv.setEmphasized(false) };
            }
        }
    })
```

#### UIKit — `UICollectionView` sidebar list

Subcontrols: `Host` = the `UICollectionView` itself (it is its own scroller). Rows are
`UICollectionViewListCell`. One UIKit-specific rule: the *list configuration* (sidebar
appearance, separators, swipe providers) is consumed when the layout is created, so those
choices are build-time — they ride `TreeProps` hints and the packaged style tweak below, not a
post-mount poke. Post-mount tweaks get everything that is a live property:

```rust
use day_uikit::{Subcontrol, UikitExt};
use objc2_ui_kit::{UICollectionView, UICollectionViewListCell};

tree(src, row)
    .uikit(|view, _class, _mtm| {                       // Host — the collection view
        if let Some(cv) = view.downcast_ref::<UICollectionView>() {
            unsafe {
                cv.setDragInteractionEnabled(true);      // drags on iPhone, not just iPad
                cv.setKeyboardDismissMode(
                    objc2_ui_kit::UIScrollViewKeyboardDismissMode::OnDrag,
                );
            }
        }
    })
    .row_tweak(|cell, class, info| {
        // `class` is "UICollectionViewListCell"; runs on every bind and rebind.
        let Some(c) = cell.downcast_ref::<UICollectionViewListCell>() else { return };
        unsafe {
            c.setIndentationWidth(18.0);
            c.setIndentsAccessories(true);
            // Group rows read as headers: clear background, no reorder accessory.
            if info.is_group {
                let mut bg = c.defaultBackgroundConfiguration();
                bg.setBackgroundColor(None);
                c.setBackgroundConfiguration(&bg);
            }
        }
    })
```

#### GTK — `GtkListView` + `GtkTreeListModel` in a `GtkScrolledWindow`

Subcontrols: `Host` = `GtkScrolledWindow`, `Content` = `GtkListView`. The row handle
`.row_tweak` receives is the `GtkTreeExpander` the factory wrapped the Day content in — which
is exactly the widget GTK's own tree knobs live on.

```rust
use day_gtk::{GtkExt, Subcontrol};

tree(src, row)
    .gtk_subcontrol(Subcontrol::Content, |w, class| {
        // `class` is "GtkListView".
        if let Some(lv) = w.downcast_ref::<gtk4::ListView>() {
            lv.add_css_class("navigation-sidebar");     // the GNOME sidebar treatment
            lv.set_show_separators(false);
            lv.set_single_click_activate(false);
            lv.set_enable_rubberband(true);             // marquee multi-select
        }
    })
    .gtk(|w, _class| {                                  // Host — the scrolled window
        if let Some(sw) = w.downcast_ref::<gtk4::ScrolledWindow>() {
            sw.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
            sw.set_overlay_scrolling(true);
        }
    })
    .row_tweak(|row, class, info| {
        // `class` is "GtkTreeExpander".
        if let Some(exp) = row.downcast_ref::<gtk4::TreeExpander>() {
            exp.set_indent_for_icon(true);
            exp.set_hide_expander(!info.expandable);    // leaves get no chevron gutter
        }
    })
```

#### XAML (WinUI) — `TreeView` / `TreeViewNode`

Raw tier, as all XAML tweaks are: the accessor hands the borrowed ABI pointer and the class,
and the app's own C++/WinRT does the work (compiled by the crate's `build.rs`, exactly as
`day-tweak-slider-tickmarks` does). Subcontrols: `Host` = the `TreeView`. Rows are
`TreeViewItem`s.

```rust
tree(src, row).xaml_raw(|abi, class| {
    let cls = std::ffi::CString::new(class).unwrap();
    unsafe { layers_tree_style(abi, cls.as_ptr()) };
})
```

```cpp
#include <cstring>
extern "C" void layers_tree_style(void* abi, const char* cls) {
    if (!cls || std::strcmp(cls, "TreeView") != 0) return;
    WUX::UIElement e{ nullptr };
    winrt::copy_from_abi(e, abi);                       // AddRef for this call's duration
    auto tv = e.try_as<WUXC::TreeView>();
    if (!tv) return;
    tv.SelectionMode(WUXC::TreeViewSelectionMode::Multiple);
    tv.CanDragItems(true);
    tv.CanReorderItems(true);
}
```

```cpp
// row_tweak's raw form receives the TreeViewItem — the glyph knobs live there:
extern "C" void layers_row_style(void* abi, const char* cls, int is_group) {
    if (!cls || std::strcmp(cls, "TreeViewItem") != 0) return;
    WUX::UIElement e{ nullptr };
    winrt::copy_from_abi(e, abi);
    auto item = e.try_as<WUXC::TreeViewItem>();
    if (!item) return;
    item.GlyphSize(10.0);
    item.CollapsedGlyph(L"");                     // Segoe Fluent chevrons
    item.ExpandedGlyph(L"");
    if (is_group) item.GlyphOpacity(0.0);
}
```

WinUI's deeper theming — `TreeViewItemBackgroundSelected` and the other lightweight-styling
resource keys — stays available through the app's own resource dictionary, outside Day
entirely, which is the correct place for it.

#### ArkUI — the ArkTS `TreeView`

The component's public surface is deliberately small — `TreeView { treeController }` plus
`NodeParam` per node — so "deep customization" on HarmonyOS is mostly *per-node*, and it
crosses the bridge rather than a widget pointer. The arkui extension exposes exactly that:

```rust
// The NodeParam fields Day does not consume itself, per token, re-sent on reload:
day_arkui::tree_ext::node_params(node, |token| NodeParamExtras {
    icon: Some("shape-rect.svg".into()),
    selected_icon: Some("shape-rect-filled.svg".into()),
    secondary_title: Some(count_label(token)),   // the little trailing count TreeView draws
    ..Default::default()
});
```

`.arkui_raw` still works, but it reaches the row's *mounted Day subtree* (the `ContentSlot`
host), not the ArkTS component — the component has no C-node handle to hand out. An app that
needs more than `NodeParam` offers replaces the ArkTS component wholesale through the piece
extension mechanism (`[package.metadata.day.ohos]`, [docs/extending.md](extending.md)), which
is the platform's own escape hatch, not a Day invention.

#### The emulated targets: Qt, Android, web-dom

The tree logic is Day's here, and that inverts the customization story in the app's favor: the
disclosure, the indent and the row chrome are ordinary Day pieces, so the *row builder and the
piece's own options* are the deep-customization surface — no FFI required for anything the
emulation draws. What remains native is the host, and the host is still tweakable:

**Qt.** Rows are real `QWidget`s Day built; the host is the emulated scroller. Host-level
polish goes through `qt_raw`; row-level polish is just Rust in the row builder, with
`row_tweak` available for the row *container* (per-depth stylesheets, for instance):

```rust
tree(src, row)
    .qt_raw(|w, class| {
        let cls = std::ffi::CString::new(class).unwrap();
        unsafe { kinetic_scroll(w, cls.as_ptr()) };      // QScroller::grabGesture, 4 lines of C++
    })
    .row_tweak(|row_w, _class, info| {
        // The row container QWidget: zebra-stripe by depth with a stylesheet.
        day_qt::ext::set_style_sheet(row_w, if info.depth % 2 == 0 {
            "background: palette(base)"
        } else {
            "background: palette(alternate-base)"
        });
    })
```

If the [QTreeView spike](#the-note-on-qt) lands, `Subcontrol::Content` starts answering with
the `QTreeView` and the native knobs (`setAnimated`, `setRootIsDecorated`,
`setUniformRowHeights`, `::branch` stylesheets) become reachable exactly as on AppKit —
which is the point of naming subcontrols rather than classes.

**Android.** The tree flattens onto the same native `RecyclerView` the `list` piece uses, so
host tweaks are ordinary JNI tweaks and rows are Day-built `ViewGroup`s:

```rust
tree(src, row)
    .android(|view, class, env| {
        // `class` is "androidx.recyclerview.widget.RecyclerView".
        let _ = env.call_method(view, "setOverScrollMode", "(I)V", &[2.into()]); // never
        let _ = env.call_method(view, "setVerticalScrollBarEnabled", "(Z)V", &[true.into()]);
    })
    .row_tweak(|row_view, _class, info| {
        day_android::ext::set_elevation(row_view, if info.is_group { 2.0 } else { 0.0 });
    })
```

**web-dom.** The emitted structure carries stable hooks — `day-tree` on the host,
`day-tree-row` with `data-depth` per row, and the full ARIA tree attributes — so a stylesheet
in `resource/assets` restyles everything CSS can reach, and the new `with_element` accessor
covers the dynamic remainder:

```rust
tree(src, row).dom(|el| {
    el.add_class("layers-panel");
})
```

```css
/* resource/assets/app.css — the web's native customization language */
.layers-panel { scrollbar-width: thin; }
.layers-panel .day-tree-row[data-depth="0"] { font-weight: 600; }
.layers-panel .day-tree-row[aria-expanded="true"] > .day-tree-disclosure { rotate: 90deg; }
.layers-panel .day-tree-row.day-drop-into { outline: 2px solid var(--accent); }
```

#### Packaged: `day-tweak-tree-style`

Anything reusable becomes a crate, as the tweaks doc prescribes. `.tree_style(SourceList |
Plain | Inset)` maps to `NSTableViewStyle` on AppKit, the sidebar list configuration on UIKit
(a build-time hint, per the note above), `.navigation-sidebar` on GTK, `TreeViewItem` template
resources on XAML, a stylesheet on Qt, a class on web-dom — and nothing at all on Android and
ArkUI, where "nothing at all" is a documented no-op, not a silent surprise.

## Selection

`.on_selection(Fn(Vec<K>))` reports the full selected set on every change; `.selected(Fn() ->
Vec<K>)` writes app state back into the native selection without an echo. Point both at one
signal and selection is two-way — which is how Day Sketch's canvas and layer panel stay in
step without either knowing about the other.

## Expansion

The app owns it: `.expanded(Signal<HashSet<K>>)`. The user's disclosure clicks arrive as
`Event::TreeExpanded` and update the signal; the app writing the signal expands or collapses
the native rows through `TreePatch::Expand`. An app that wants expansion to survive a relaunch
persists that set and nothing else changes.

A reload must not silently collapse the tree. Where the backend keys expansion by a stable
item (AppKit, diffable snapshots) it survives on its own; elsewhere Day re-applies the set
after the reload.

## Moving nodes

`.movable(true)` turns on the platform's drag; `move_guard` answers **while the drag is
live**, so the affordance reflects the verdict before the user lets go — the same contract as
`list`'s `reorder_guard`, and the reason a guard must stay pure. The refusals every tree
needs: a node cannot move into itself, into its own descendant, or into a leaf.

`on_move` is the commit. In Day Sketch it writes `parent` and a fractional `z` between the new
neighbors, in one undo unit, which is the same write the Arrange menu makes.

## Accessibility

The emulated path has to say what the native trees say for themselves, so this ships with the
piece rather than after it: `Role::Tree` and `Role::TreeItem` join the a11y vocabulary,
carrying level, position in set, set size, and expanded state — `aria-level`/`aria-expanded` on
web, `AccessibilityNodeInfo` collection-item info plus expand/collapse actions on Android,
`ARKUI_ACCESSIBILITY` attributes on ArkUI, `QAccessible::Tree` on Qt. `a11y_audit` then holds
every backend to the same expectation.

## Driving it from dayscript

- `expand: { id, key, expanded }` — open or close a row.
- `tree_move: { id, key, parent, index }` — guard, then commit, with no native gesture; a
  denied move fails the step, the way `reorder:` does.
- `type_ahead: { id, text }` — feed the buffer and assert where the cursor lands, since the
  native type-select callbacks are not reachable from a synthetic key event on every backend.
- Rows carry ids, so `tap`, `assert_text` and `assert_missing` already work on them, and
  `key: { key: ArrowDown }` drives the keyboard on the emulated path.

## Implementation plan

Mock-first, as `list` was: the driver is proven headlessly before a single native tree exists,
so every backend after the first is a rendering problem rather than a semantics problem.

### M0 — spec, piece, driver, mock (SHIPPED 2026-08)

*day-spec*: `kinds::TREE` in `builtin_kinds!`; `TreeProps`; `TreePatch`; `TreeSource` +
`TreeMoves` + `MoveVerdict`; `Event::{TreeExpanded, TreeMove}`; `Cap::{Tree, TreeMove}`;
`Role::{Tree, TreeItem}` and their `Role::for_kind` arm; `Toolkit::attach_tree` defaulting to
a no-op; `Subcontrol`. The `Builtin::ALL` length test moves by one, and the backends with
exhaustive realize matches gain a fallthrough arm in the same commit so the workspace keeps
compiling.

*day-core* (`src/tree_driver.rs`, beside `list.rs`): `TreeDriver` — `children_len`,
`child_token`, `expandable`, `build(token, RNode) -> BuiltRow`, `type_select_text`, optional
`moves` — plus `install_tree`, `tree_reload`, `tree_set_expanded`, `tree_set_selected`,
`tree_reveal`, and `tree_try_move` for dayscript and the mock. The **flattener** lives here,
kind-agnostic and memoised per (reload, expansion) generation. The cell-anchor half of
`list.rs` — `BoundCell`, `CellStep`, scope ownership — is lifted into a shared module both
drivers use rather than copied, and gains the row-bind hook `.row_tweak` rides.

*Tweak surface* (`day-spec` + every backend's `ext`): `Subcontrol`, `with_native_subcontrol`,
and the `…_subcontrol` decorator on each toolkit's extension trait, with `Subcontrol::Host`
preserving today's behavior. `list` and `text_area` declare their subcontrols in the same
change, since they have been composite all along.

*day-pieces* (`src/tree.rs`): `tree(source, row)`, the `NodeSource` trait with two
implementations (`branches(items, key, parent)` and the store adapter
`Store::tree(children_projection)`), and the builder: `.expanded`, `.selected`,
`.on_selection`, `.multi_select`, `.movable`, `.on_move`, `.move_guard`, `.on_activate`,
`.type_ahead`, `.reveal`, `.indent`, plus the hooks — `.row_height_for`, `.can_expand`,
`.can_select`, `.is_group_row` — and `.row_tweak`.

*day-mock*: a simulated viewport plus `MockProbe::{tree_rows, tree_expand, tree_can_move,
tree_move, tree_type_ahead}`.

**Done when** these pass headlessly: only visible rows build; collapsing a row disposes its
descendants' scopes; recycling rebinds by slot-write rather than rebuilding; a move rewrites
the snapshot before returning and defers `on_move` to the next drain; a guard denial leaves
the tree untouched; expansion and selection survive a reload by token; reveal expands
ancestors before scrolling; type-ahead selects the right row and resets on timeout; every hook
is consulted by the flattener; `.row_tweak` fires on each bind and rebind with the right
`RowInfo`.

### M1 — AppKit, the reference native (SHIPPED 2026-08)

`NSOutlineView` over the existing view-based row path, so `makeView`/`viewFor` reach
`bind_row` unchanged. `outlineView(_:child:ofItem:)` / `isItemExpandable` /
`numberOfChildrenOfItem` map one-to-one onto the seam; `expandItem`/`collapseItem` apply
`TreePatch::Expand`, and the `ItemDidExpand`/`ItemDidCollapse` notifications emit
`Event::TreeExpanded`. Drag reuses the table's pasteboard pipeline, with
`validateDrop(proposedItem:proposedChildIndex:)` answering from `can_move` — including
`NSOutlineViewDropOnItemIndex` for a drop *onto* a row, which is what `index: None` means.
Type-select answers from `type_select_text`.

This is also where the tweak additions earn their keep: the node's handle is the
`NSScrollView`, so `Subcontrol::Content` is what hands a tweak the `NSOutlineView`, and
`.row_tweak` receives the `NSTableRowView`. The four delegate hooks land here first, since
AppKit is the backend with the richest delegate to route them to.

**Done when** the Showcase page (M5) drives expansion, multi-select, keyboard and a
drag-reparent on macOS; `a11y_audit` reports tree rows with disclosure levels; and the
source-list tweak from [the AppKit example](#appkit--nsoutlineview-in-an-nsscrollview)
compiles and visibly changes the rendering.

### M2 — the shared emulation, on web-dom and Qt (SHIPPED 2026-08 — see the as-built notes)

Both already emulate `list`, so they are the cheapest proof that one flattener serves three
backends. Rows come from the flattener; each row's Day subtree gets an indent and a disclosure
control built from ordinary pieces.

**As built (2026-08).** The emulation landed one level higher than planned: in the PIECE, not
in each backend. `TreePiece::build` branches on `capability(Cap::Tree)` — `Native` takes the
attach path above, anything else takes `build_composed`, which flattens the connection's
visible rows (a DFS descending only into expanded rows, tracked against the shape read and
the expansion signal) onto the existing [`list`] piece. So the composed tree costs a backend
NOTHING: web-dom and the qt toolkit answer `Cap::Tree` `Emulated`, and the same code would
render on any backend with the list machinery. Per row:

- **Indent** is a layout (`TreeIndent`) reading the row's depth from a `Cell` the rebind
  watch writes before the cell's relayout — a recycled cell re-indents without rebuilding.
- **Disclosure** is a chevron label (`▸`/`▾`, tracked against the expansion state) with an
  `on_tap` that flips the app's `.expanded` signal (or the piece's internal set). The
  dayscript `expand:` step emits the same `Event::TreeExpanded` the native backends do, and
  the handler routes it into the same signal — one echo-free path for both.
- **Selection rides the list's own machinery** — `.selected_rows` / `.on_selection` with
  tokens translated to keys — so multi-select, shift/ctrl clicks and the painted highlight
  are the list backend's, not re-implemented.
- **The row shell is a NATIVE transparent container**, not a layout-only node: the row's
  dayscript id lands as a real a11y identifier, and `.row_context_menu` (lowered onto the
  ordinary `.context_menu_fn` decorator, key read AT SUMMON) needs an element to arm its
  listener on ([docs/menus.md](menus.md) "Dynamic context menus").
- **The driver still installs** on the returned node — `expand:`/`tree_move:` resolve rows
  and route moves through the same guard → commit seam, which is what lets ONE
  `tree.yaml` pass verbatim on native and composed backends alike.

Pool honesty: an emulated list that SHRANK hides pooled cells in place, so
`ListSource::recycle` now clears a hidden cell's element ids (`list_recycle_cell` — the
list twin of the tree's recycle rule); web-dom and qt call it as they hide. Qt also honors
`.edge(PaneEdge::Leading)` now (the splitter's panel pane goes FIRST) — the layers pane was
the first leading inspector a qt target ever showed.

Deliberate deltas, still open: a disclosure click also selects its row (the tap reaches the
cell-click machinery too); keyboard navigation and type-ahead do not exist on composed trees;
the ARIA tree pattern (`role="tree"`, `aria-level`, …) and the `day-tree`/`data-depth` class
hooks are NOT emitted yet; **"day-dom joins the tweak system"** (`with_element`, `add_class`,
`set_style`) was not taken in this pass; and there is no drag-to-move — `Cap::TreeMove` stays
`Unsupported`, with `tree_move:` driving the seam synthetically. Verified by tree.yaml 87/87
on web-dom and macos-qt plus a real-pointer browser suite (chevron clicks, row and canvas
right-clicks through the composed menu, drag-move) against the live server.

### M3 — GTK and UIKit (SHIPPED 2026-08 — native drag pending on both; see the status alert)

GTK: `GtkListView` over a `GtkTreeListModel` whose `create_model_func` pulls children lazily
from the seam, each row wrapped in a `GtkTreeExpander`; selection through
`GtkMultiSelection`; drag on the existing `GtkDragSource`/`GtkDropTarget` rows, with Day
computing into-versus-between from the pointer.

UIKit: `UICollectionView` with `UICollectionLayoutListConfiguration(appearance: .sidebar)`, an
`NSDiffableDataSourceSectionSnapshot` built from the seam, `.outlineDisclosure` accessories,
and `sectionSnapshotHandlers.willExpandItem`/`willCollapseItem` for the expansion events.
Reorder rides the collection's drag and drop delegates. The list-configuration knobs are
build-time on this backend — `TreeProps` hints, per the customization note.

### M4 — XAML and ArkUI

XAML: WinUI `TreeView` with `TreeViewNode`s mirrored from the seam, `CanReorderItems` for the
drag, native type-ahead, the raw subcontrol/row tweak channel from the examples above.

ArkUI: the ArkTS `TreeView` driven through the bridge described
[above](#arkui-reaching-an-arkts-component-from-the-c-node-api) — `TreeController.addNode` per
node with a per-node `NodeContent` in `NodeParam.container`, `TreeListener` for `NODE_CLICK`
and `NODE_MOVE`, and `tree_ext::node_params` for the per-node extras. This lands last of the
natives because it is the most unusual and wants the seam settled.

### M5 — Android, the Showcase page, and the style crate (Android SHIPPED 2026-08)

Android joins the emulation (the flattener over the existing `RecyclerView` list machinery,
`ItemTouchHelper` for the drag, `AccessibilityNodeInfo` collection-item info plus
expand/collapse actions). The Showcase gains its page — see below — and
`day-tweak-tree-style` ships with whatever coverage the backends built so far support.

**As built (2026-08, the Android half).** Because M2 landed the emulation in the PIECE,
Android's cost was the list gaps, not a tree: `Cap::Tree` answers `Emulated` and the
composed build runs unchanged. What the RecyclerView machinery gained with it:

- **`ListPatch::Selected`** — recorded per list, painted onto the visible holders (the theme
  accent at 20% alpha as the cell BACKGROUND, under the ripple foreground) and inherited by
  newly bound holders, with no selection-event echo. Taps still report single selection —
  the touch idiom — and the round trip through the app's signal highlights the row.
- **`onViewRecycled` → `ListSource::recycle`** — a pooled holder's day content keeps its
  views but sheds its dayscript ids (the same rule every other backend follows), keyed by
  the per-cell GlobalRef `nativeListBind` binds with.

Day Sketch grew the phone ergonomics in the same change: a COMPACT window starts with the
layers pane closed (the canvas needs the room), the tool row carries a Layers toggle, and
`tree.yaml` opens the pane up front on the phone targets (89 steps; the two open-toggle
steps are `only_on: [uikit, mdc]`). Verified on the emulator: tree.yaml 89/89 and demo.yaml
321/321, a cold-start check of the compact default, and REAL `adb input` taps — a row tap
moving the selection tree→canvas, a chevron tap collapsing and re-expanding (dayscript
injects events, so only real taps prove the recognizers; the inner chevron listener wins
over the cell's click, so a disclosure tap does NOT re-target the selection — Android and
iOS get this right where the web's bubbling cannot). Still open on Android: the
`ItemTouchHelper` drag half (`Cap::TreeMove` stays `Unsupported`), the
`AccessibilityNodeInfo` expand/collapse actions, and row context menus (no
`set_context_menu_fn` wiring — a long-press summon would need the composed presenter's
`Event::ContextMenu`).

### M6 — Day Sketch (leading pane, layer panel and walkthrough SHIPPED 2026-08; the Showcase page waits for M5)

The leading pane and the layer panel, covered below.

## The Showcase page

A new `Section::Tree` beside `Section::List`: a `routes!` variant, a `source_file()` arm, a
`Dest { … page: tree_page }`, `src/pages/tree.rs`, a nav vector icon, and strings in all four
locales the Showcase ships (`en`, `fr`, `ar`, `zh-CN` — no raw literals).

The page shows one tree deep enough to be interesting and small enough to read: a mock project
with folders, files and a nested folder, each row a disclosure plus an icon plus a name, and
below it a live readout of the current selection, expansion set and last move. Controls beside
it exercise the API rather than decorating it: a multi-select toggle, an "expand all" and
"collapse all" pair driving the expansion signal, a `move_guard` switch that refuses drops
into one particular folder (so the denied affordance is visible on every platform), and a
reveal field.

The page also demonstrates the three customization layers in one place, since that is the part
of this design an app author has to understand: the portable options drive the controls, one
hook (`can_select` on folders) is toggleable, and a `day-tweak-tree-style` line sits in the
source under a comment explaining that it changes the macOS and Windows rendering and no-ops
on the rest — the Showcase's usual job of being the worked example.

The walkthrough leg — `dayscript/tree.yaml`, joining the per-target list in the Showcase's
CI — asserts what no screenshot can: expanding a folder reveals exactly its children,
collapsing hides their ids, `tree_move` reparents and the readout agrees, a guarded move fails
the step, type-ahead lands on the expected row, and `assert_no_placeholders` holds on every
target.

## Day Sketch: the layer panel

**The pane.** Day has no leading utility pane: [`inspector`](inspector.md) is the trailing one
and `selector(Sidebar)` is a navigation split. `InspectorProps` grows an `edge: Edge`
(defaulting to `Trailing`, so no existing app moves), and the four backends that realize the
inspector map it: `NSSplitViewItem.sidebar` rather than `.inspector`, `AdwOverlaySplitView`'s
start side, the first pane of the `QSplitter`, WinUI's `SplitView` pane. On phones the leading
pane presents as the same sheet the inspector already uses. Day Sketch then reads: layers on
the leading edge, canvas in the middle, inspector on the trailing edge, each independently
collapsible.

**The source.** Day Sketch's scene is already a tree: `children_of(parent)` ordered by `z` is
the children projection, `expandable` is `kind == NodeKind::Group`, and the row is a kind
glyph, the node's name and a fill swatch — the [store-backed example](#a-store-backed-tree)
above, verbatim.

**Selection.** The tree binds `.selected(|| model::selection().get())` and
`.on_selection(|keys| model::selection().set(keys))` — the same signal the canvas reads, so
the two stay in step without either knowing the other exists, and undo's transient selection
restoration already flows to both.

**Moving.** `on_move(node, parent, index)` writes `parent` and a fractional `z` between the
new neighbors in one undo unit labeled `move`, which is the write the Arrange menu already
makes. `move_guard` refuses a node into itself or a descendant, and refuses a drop into a
leaf.

**Keyboard and type-ahead** come with the piece: arrows walk the tree, left and right climb
and open, type-ahead jumps by node name.

**Walkthrough.** The demo script gains a section that expands the group made earlier in the
run, asserts its two children appear, selects a row and asserts the canvas frame readout
changes, drags a shape into the group with `tree_move` and asserts the count and the group's
bounds, then undoes it — proving canvas and layer panel share one model rather than two.

## Stepping back: what the tree stresses in Day's architecture

Designing this piece is also a test of the framework, and it is worth recording what the test
found, because the conclusions outlive the piece.

**Tweaks are constitutionally shallow, and that is their virtue.** The tweak model assumes one
node ↔ one widget, behavior-in-properties, and mount-once lifetime. A tree breaks all three,
and each addition above patches exactly one: subcontrols address the composite backing, row
tweaks address cells Day does not own, hooks address behavior that lives in a delegate rather
than in properties. The first two are still tweak-shaped. The hooks are not tweaks at all —
and calling them "deep customization via tweaks" would misfile them. Depth comes from the seam
and the hooks; the hatch stays shallow so it stays safe.

**The policy rung already existed, unnamed.** `list` grew `reorder_guard` and `delete_guard`;
`nav` grew `on_back`; this piece needs `can_expand`, `can_select`, `row_height_for`,
`move_guard`. All the same shape: pure, synchronous, called inside a native callback, Day
answering the platform's question by asking the app. The extension ladder in
[docs/tweaks.md](tweaks.md) documents styling / tweaks / native pieces; the fourth rung —
policy — has been growing piecemeal since `list` shipped. When the tree lands, the ladder
should name it.

**Tokens are the corrected identity contract.** `ListSource` addresses rows by index
(`bind_row(usize, …)`, guards over `(from, to)`); the tree cannot, and every native tree API
agrees with the tree. That leaves two contracts in the codebase. This plan does not migrate
`list`, but the mismatch is a known debt, and new collection kinds should follow the token
seam.

**The emulations are converging on one composed tier.** web-dom and Qt each carry their own
emulated list; the flattener here is deliberately kind-agnostic so it becomes shared substrate
rather than a third copy. The composed colorpicker and stepper prove the wider pattern: for
complex kinds, one Rust reference implementation as the guaranteed floor, native upgrades
where a platform genuinely offers more. That inversion — a working composed fallback instead
of a `⟨kind⟩` placeholder — is right for containers and wrong for buttons, and the boundary
between those is a decision Day should make once, on purpose.

None of this reopens the macro-architecture. The retained tree, capability honesty, mock-first
drivers and the per-toolkit asset pipeline all held under this design's weight — and the
platforms' independent agreement on the `(parent, index)` drop vocabulary is direct evidence
the portable semantic core is real. The corrections are all one level down, in the collection
middle layer — subcontrols, hooks, row tweaks, the shared flattener — and this piece is the
deliberate first step through them rather than another accretion.

## Risks worth deciding early

- **Token stability becomes a requirement.** `list` tolerates index churn; a tree does not.
  Sources whose keys move break expansion and selection, so the docs must say it and the mock
  tests must prove it.
- **Reload granularity.** v1 reloads the whole tree on a data change, as `list` does. Large
  trees under frequent edits will want the keyed diff (`TreePatch::Splice`) sooner than lists
  did, because a reload also disturbs expansion.
- **Recycling is not universal.** AppKit, UIKit, GTK and XAML reuse cells; ArkUI's
  `TreeController` does not, and Qt's index-widget path would not either. `Cap::ListRecycling`
  has to answer honestly per backend, and the docs have to say which trees stay small.
- **Two natives cannot guard a drag live.** ArkUI reports a move after the fact; the emulated
  path can be as live as Day makes it. `move_guard`'s contract must therefore be "consulted as
  early as the platform allows", with the per-backend table saying where that is — the same
  shape `list`'s reorder guard already documents.
- **Two identity contracts until `list` migrates.** The tree is token-addressed; `list` is
  index-addressed. Both are correct alone, and the pair is a wart: shared machinery has to
  speak both, and apps that use both pieces learn two vocabularies. The migration is out of
  scope here and should not be forgotten.
- **Subcontrols widen the contract.** Naming `Subcontrol::Content` promises a kind keeps
  having a content widget. That is a weaker promise than a class name (the tweaks doc already
  refuses to freeze those), but it is still a promise, so subcontrols are declared per kind in
  the docs and an unknown subcontrol must resolve to `None` rather than to the host.
- **UIKit's build-time configuration.** The sidebar list's appearance knobs are consumed when
  the layout is created, so style choices there ride `TreeProps` hints rather than post-mount
  tweaks. The packaged style crate has to front both channels, and the docs have to say which
  knob rides which.
- **What v1 leaves out, on purpose.** Dragging a multi-row selection as one unit;
  spring-loading a collapsed row under a hovering drag (native on AppKit, a timer elsewhere);
  columns beside the disclosure, which is where a tree becomes a tree *table* and wants
  `NSOutlineView`'s and `GtkColumnView`'s column machinery; and lazy children that arrive
  asynchronously, which the seam's synchronous `children_len` cannot express and which a file
  browser eventually needs.
