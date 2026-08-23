---
title: "Outline"
description: "A hierarchical outline piece over each platform's native tree view, with live selection, expansion, and drag-to-reparent."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Outline (design)

> [!NOTE]
> **Status: planned, not built.** This document carries the per-toolkit assessment, the API and
> seam that assessment shaped, and the milestones to build them in ([Implementation
> plan](#implementation-plan)). Nothing here ships yet, so the API is still a proposal to argue
> with rather than a contract to code against.

An outline is a list that nests: rows at several depths, disclosure controls that open and
close them, selection shared with whatever else shows the same data, and a drag that moves a
row *into* another one rather than only above or below it. A drawing app's layer panel, a
file browser's source list and an outliner's document are all the same control.

Day has [`list`](list.md) for flat rows and [`nav_menu`](navigation.md) for a fixed sidebar of
destinations. Neither nests, and neither can express "drag this node into that group", so an
outline needs its own piece.

## The driving case: Day Sketch's layer panel

Day Sketch keeps its scene in one table of nodes, each with a `parent` and a fractional `z`
([docs/model.md](model.md)). A group is a node whose children point at it. That is already a
tree, and the app wants it on the leading edge of the window:

- every node listed, groups nesting their members, in the canvas's own back-to-front order;
- selection synchronized both ways, because the canvas and the outline read one selection
  signal — click a row, the shape's handles appear; shift-click two shapes, both rows
  highlight;
- drag a row onto a group to reparent it, or between two rows to restack it, which is the
  same `parent` + `z` write the Arrange menu already makes, and one undo unit.

Nothing about that is specific to Day Sketch. It is what every outline does.

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
tree in it — which is what made this look like a gap. `TreeView` lives one layer up, in the
ArkTS advanced component set, and Day already crosses that layer twice: the app's `@Entry`
page mounts Day's native tree through a `NodeContent`/`ContentSlot`, gives every pushed
navigation page its own per-page `NodeContent`, and answers up-calls from Rust for things the
C API cannot do at all (the file picker runs on the ArkTS side and hands bytes back).

An outline uses the same two mechanisms:

- **Structure** comes from `TreeController` — `addNode(NodeParam { parentNodeId, currentNodeId,
  isFolder, … })` per node, then `buildDone()` — driven from Rust over the existing bridge.
- **Row content** comes from `NodeParam.container`, a builder slot ("set subcomponent binded on
  tree item"). It holds a `ContentSlot` bound to a per-node `NodeContent`, keyed by node id
  exactly as `navContents` keys pages today, and Day mounts the row's C-API subtree into it.
- **Events** arrive through `TreeListener`: `NODE_CLICK` for selection and `NODE_MOVE` with
  `CallbackParam { currentNodeId, parentNodeId, childIndex }` — which is precisely the
  `(node, parent, index)` commit this design's seam is shaped around.

Two honest costs. `TreeController` builds nodes imperatively with no cell reuse, so ArkUI's
outline does not recycle and `Cap::ListRecycling` should say so; a layer panel is fine, a
hundred-thousand-row outline is not. And the listener fires *after* a move, so `move_guard`
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
the emulated path is written off as Qt's permanent answer; the seam below does not change
either way.

## Core built-in, not a satellite piece

Day has two places a piece can live: `crates/day-pieces` with a `kinds::…` of its own, or a
satellite crate that registers per-backend renderers through `renderer!`
([docs/extending.md](extending.md)). Satellites are the default answer — the stepper, the color
picker and the web view are all satellites — so the burden is on the outline to justify core.

It clears it on four counts, three of which a satellite cannot reach at all:

| Needs | Reachable from a satellite? |
|---|---|
| A pull seam the backend calls synchronously (`children_len`, `child_token`, `bind_row`) | **yes, awkwardly** — the props struct can carry `Rc<dyn Fn…>` closures, so a satellite could ship its own seam without a `Toolkit` duty |
| Binding a Day row into a native cell (cell-anchor adoption, `BuiltRow`, scope ownership) | **no** — `Tree::install_list` and the cell machinery are day-core's; `install_outline` has to sit beside it |
| `Cap::Outline` / `Cap::OutlineMove`, `Role::Outline` / `Role::OutlineItem` | **no** — both enums live in day-spec, and a satellite cannot add variants |
| dayscript `expand:` / `outline_move:` steps | **no** — `Step` lives in day-script, and the walkthrough has to drive expansion and moves on every target |

The seam alone would not settle it. The cell machinery, the capability and a11y vocabulary, and
the test steps do: three of the four are spec-and-core edits whatever crate the piece nominally
lives in, and a satellite that needs three core edits to work is a core piece wearing a costume.

`kinds::OUTLINE` therefore joins `builtin_kinds!` beside `kinds::LIST`, and the piece ships in
`day-pieces` — with the same consequence every new builtin kind has: the backends whose realize
matches are exhaustive stop compiling until each names the kind, which is the checklist, not a
surprise.

## The piece

```rust
outline(source, row)
    .expanded(open)                       // Signal<HashSet<K>>, two-way
    .on_selection(move |keys| select(keys))
    .selected(move || selection.get())    // app state → native selection, echo-free
    .multi_select(true)
    .movable(true)
    .on_move(|node, parent, index| reparent(node, parent, index))
    .move_guard(|node, parent, index| Move::Allow)
    .id("layers")
```

The row builder is `list`'s: an `ItemSlot`/`ModelSlot` bound once per physical cell and
rebound as cells recycle, so a ten-thousand-node outline builds only what it shows. What
changes is the source, which is hierarchical, and the identity, which is a **token**, not a
row index. A tree cannot key rows by position: expanding one row renumbers everything below
it. Every native API here agrees — `NSOutlineView` keys by item pointer, diffable snapshots by
item identifier, `GtkTreeListRow` by item, `TreeViewNode` by content.

## The seam: `OutlineSource`

Recycling trees pull, exactly as recycling lists do, so the outline extends the same
synchronous seam `list` added ([docs/list.md](list.md)) rather than inventing a second one:

```rust
pub struct OutlineSource {
    /// How many children `parent` has (`None` = the root).
    pub children_len: Rc<dyn Fn(Option<u64>) -> usize>,
    /// The i-th child of `parent` — the stable token every backend keys its row by.
    pub child_token: Rc<dyn Fn(Option<u64>, usize) -> u64>,
    /// Whether this token can hold children at all — what draws (or omits) the disclosure.
    pub expandable: Rc<dyn Fn(u64) -> bool>,
    /// Build-or-rebind the row for `token` into this native cell.
    pub bind_row: Rc<dyn Fn(u64, RawHandle)>,
    pub recycle: Rc<dyn Fn(RawHandle)>,
    /// The drag half, present only under `.movable(true)`.
    pub moves: Option<OutlineMoves>,
}

pub struct OutlineMoves {
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
diffable snapshot's `append(to:)`, to `GtkTreeListRow`'s child model, and to a `TreeViewNode`'s
`Children.Insert`.

New spec surface, all additive: `kinds::OUTLINE`, `OutlineProps { multi_select, indent, … }`,
`OutlinePatch::{Reload, Expand(token, bool), Selected(Vec<u64>), ScrollToToken(u64)}`,
`Event::OutlineExpanded { token, expanded }`, `Event::OutlineMove { node, parent, index }`,
`Toolkit::attach_outline`, and `Cap::Outline` / `Cap::OutlineMove`.

## One flattener for the rest

`day-core` turns an `OutlineSource` plus the expansion set into the flat row sequence the
emulated backends render: a walk from the root that descends only into open rows, producing
`(token, depth)` pairs. The emulated backends then reuse the **list** cell machinery unchanged
and wrap each row in an indent plus a disclosure button built from ordinary Day pieces.

That is one implementation of expansion, indentation, keyboard handling and drop targeting,
shared by Qt, Android and web-dom, exercised by the mock backend's tests. A bug fixed in the
flattener is fixed on three platforms.

## Selection

`.on_selection(Fn(Vec<K>))` reports the full selected set on every change; `.selected(Fn() ->
Vec<K>)` writes app state back into the native selection without an echo. Point both at one
signal and selection is two-way — which is how Day Sketch's canvas and outline stay in step
without either knowing about the other.

## Expansion

The app owns it: `.expanded(Signal<HashSet<K>>)`. The user's disclosure clicks arrive as
`Event::OutlineExpanded` and update the signal; the app writing the signal expands or collapses
the native rows through `OutlinePatch::Expand`. An app that wants expansion to survive a
relaunch persists that set and nothing else changes.

A reload must not silently collapse the tree. Where the backend keys expansion by a stable
item (AppKit, diffable snapshots) it survives on its own; elsewhere Day re-applies the set
after the reload.

## Moving nodes

`.movable(true)` turns on the platform's drag; `move_guard` answers **while the drag is live**,
so the affordance reflects the verdict before the user lets go — the same contract as
`list`'s `reorder_guard`, and the reason a guard must stay pure. The obvious refusals an
outline needs: a node cannot move into itself, into its own descendant, or into a leaf.

`on_move` is the commit. In Day Sketch it writes `parent` and a fractional `z` between the new
neighbors, in one undo unit, which is the same write the Arrange menu makes.

## Keyboard and type-ahead

An outline is a keyboard control before it is a mouse one, and the platforms disagree about how
much of that they hand over.

| Behavior | AppKit | UIKit | GTK | XAML | ArkUI | emulated (Qt · Android · web) |
|---|---|---|---|---|---|---|
| Arrow up/down through visible rows | native | native (hardware kbd) | native | native | native | Day |
| Left/right to collapse and open | native | native | native | native | native | Day |
| Home/End, page up/down | native | native | native | native | native | Day |
| Type-ahead to a row | native, via `typeSelectStringForTableColumn:row:` | — | — | native | — | Day |

So Day writes the keyboard once, in `day-core`, and each backend opts out of the parts its
widget already does. The emulated handler is a focus-scoped key reader on the outline node:

- **Up/Down** move the cursor within the flattened visible rows; **Shift** extends the selection.
- **Left** collapses an open row, or moves to the parent when the row is already closed —
  the behavior every tree has, and the one people reach for to climb out of a group.
- **Right** opens a closed row, or moves to its first child.
- **Home/End** jump to the first and last visible row; **Page** keys move by the viewport.
- **Enter** activates (`on_activate`), **Space** toggles selection under `multi_select`.

**Type-ahead** needs a string per row, which only the app knows, so the source carries one:

```rust
pub type_select_text: Rc<dyn Fn(u64) -> String>,
```

The piece fills it from `.type_ahead(|slot| slot.name())`, defaulting to the row's first label
when the app says nothing. AppKit and XAML get it natively by answering their own type-select
callbacks from that closure; everywhere else `day-core` keeps a small buffer that appends
printable keys, resets after ~800 ms of silence, and selects the first visible row whose text
starts with it, wrapping from the cursor. One implementation, one behavior, and the two natives
that do it themselves still read their strings from the same place.

## Accessibility

The emulated path has to say what the native trees say for themselves, so this ships with the
piece rather than after it: `Role::Outline` and `Role::OutlineItem` join the a11y vocabulary,
carrying level, position in set, set size, and expanded state — `aria-level`/`aria-expanded` on
web, `AccessibilityNodeInfo` collection-item info plus expand/collapse actions on Android,
`ARKUI_ACCESSIBILITY` attributes on ArkUI, `QAccessible::Tree` on Qt. `a11y_audit` then holds
every backend to the same expectation.

## Driving it from dayscript

- `expand: { id, key, expanded }` — open or close a row.
- `outline_move: { id, key, parent, index }` — guard, then commit, with no native gesture;
  a denied move fails the step, the way `reorder:` does.
- `type_ahead: { id, text }` — feed the buffer and assert where the cursor lands, since the
  native type-select callbacks are not reachable from a synthetic key event on every backend.
- Rows carry ids, so `tap`, `assert_text` and `assert_missing` already work on them, and
  `key: { key: ArrowDown }` drives the keyboard on the emulated path.

## Implementation plan

Mock-first, as `list` was: the driver is proven headlessly before a single native tree exists,
so every backend after the first is a rendering problem rather than a semantics problem.

### M0 — spec, piece, driver, mock

*day-spec*: `kinds::OUTLINE` in `builtin_kinds!`; `OutlineProps { multi_select, indent,
row_height }`; `OutlinePatch::{Reload, Expand(u64, bool), Selected(Vec<u64>), ScrollTo(u64)}`;
`OutlineSource` + `OutlineMoves` + `MoveVerdict`; `Event::{OutlineExpanded, OutlineMove}`;
`Cap::{Outline, OutlineMove}`; `Role::{Outline, OutlineItem}` and their `Role::for_kind` arm;
`Toolkit::attach_outline` defaulting to a no-op. The `Builtin::ALL` length test moves by one,
and the four backends with exhaustive realize matches gain a fallthrough arm in the same commit
so the workspace keeps compiling.

*day-core* (`src/outline.rs`, beside `list.rs`): `OutlineDriver` — `children_len`,
`child_token`, `expandable`, `build(token, RNode) -> BuiltRow`, `type_select_text`, optional
`moves` — plus `install_outline`, `outline_reload`, `outline_set_expanded`,
`outline_set_selected`, `outline_scroll_to`, and `outline_try_move` for dayscript and the mock.
The **flattener** lives here: a walk from the root that descends only into open rows, memoised
per (reload, expansion) generation, yielding `(token, depth)`. The cell-anchor half of
`list.rs` — `BoundCell`, `CellStep`, scope ownership — is lifted into a shared module both
drivers use rather than copied.

*day-pieces* (`src/outline.rs`): `outline(source, row)`, the `TreeSource` trait with two
implementations (a closure tree, and a day-model adapter that reads `parent`/order columns), and
the builder: `.expanded`, `.on_selection`, `.selected`, `.multi_select`, `.movable`, `.on_move`,
`.move_guard`, `.on_activate`, `.type_ahead`, `.indent`.

*day-mock*: a simulated viewport plus `MockProbe::{outline_rows, outline_expand,
outline_can_move, outline_move, outline_type_ahead}`.

**Done when** these pass headlessly: only visible rows build; collapsing a row disposes its
descendants' scopes; recycling rebinds by slot-write rather than rebuilding; a move rewrites the
snapshot before returning and defers `on_move` to the next drain; a guard denial leaves the tree
untouched; expansion survives a reload; type-ahead selects the right row and resets on timeout.

### M1 — AppKit, the reference native

`NSOutlineView` over the existing view-based row path, so `makeView`/`viewFor` reach `bind_row`
unchanged. `outlineView(_:child:ofItem:)` / `isItemExpandable` / `numberOfChildrenOfItem` map
one-to-one onto the seam; `expandItem`/`collapseItem` apply `OutlinePatch::Expand`, and the
`ItemDidExpand`/`ItemDidCollapse` notifications emit `Event::OutlineExpanded`. Drag reuses the
table's pasteboard pipeline, with `validateDrop(proposedItem:proposedChildIndex:)` answering
from `can_move` — including `NSOutlineViewDropOnItemIndex` for a drop *onto* a row, which is
what `index: None` means. Type-select answers from `type_select_text`.

**Done when** the Showcase page (M5) drives expansion, multi-select, keyboard and a drag-reparent
on macOS, and `a11y_audit` reports outline rows with disclosure levels.

### M2 — the shared emulation, on web-dom and Qt

Both already emulate `list`, so they are the cheapest proof that one flattener serves three
backends. Rows come from the flattener; each row's Day subtree gets an indent and a disclosure
control built from ordinary pieces; the keyboard handler and type-ahead come from `day-core`.
web-dom carries the ARIA tree pattern (`role="tree"`/`treeitem`, `aria-level`, `aria-expanded`,
`aria-setsize`, `aria-posinset`, roving tabindex). Drop targeting is a hit test inside the row:
the top and bottom quarters mean *between*, the middle means *into*.

**Done when** the same walkthrough steps that passed on AppKit pass on both, and the Qt spike
from [the note above](#the-note-on-qt) has a verdict recorded here.

### M3 — GTK and UIKit

GTK: `GtkListView` over a `GtkTreeListModel` whose `create_model_func` pulls children lazily
from the seam, each row wrapped in a `GtkTreeExpander`; selection through
`GtkSingleSelection`/`GtkMultiSelection`; drag on the existing `GtkDragSource`/`GtkDropTarget`
rows, with Day computing into-versus-between from the pointer.

UIKit: `UICollectionView` with `UICollectionLayoutListConfiguration(appearance: .sidebar)`, an
`NSDiffableDataSourceSectionSnapshot` built from the seam, `.outlineDisclosure` accessories, and
`sectionSnapshotHandlers.willExpandItem`/`willCollapseItem` for the expansion events. Reorder
rides the collection's drag and drop delegates.

### M4 — XAML and ArkUI

XAML: WinUI `TreeView` with `TreeViewNode`s mirrored from the seam, `CanReorderItems` for the
drag, native type-ahead.

ArkUI: the ArkTS `TreeView` driven through the bridge described
[above](#arkui-reaching-an-arkts-component-from-the-c-node-api) — `TreeController.addNode` per
node with a per-node `NodeContent` in `NodeParam.container`, `TreeListener` for `NODE_CLICK` and
`NODE_MOVE`. This one lands last of the natives because it is the most unusual and wants the
seam settled.

### M5 — Android, and the Showcase page

Android joins the emulation (a `RecyclerView` of flattened rows, `ItemTouchHelper` for the drag,
`AccessibilityNodeInfo` collection-item info plus expand/collapse actions). The Showcase gains
its page, which is also where the walkthrough legs live — see below.

### M6 — Day Sketch

The leading pane and the layer panel, covered below.

## The Showcase page

A new `Section::Outline` beside `Section::List`: a `routes!` variant, a `source_file()` arm, a
`Dest { … page: outline_page }`, `src/pages/outline.rs`, a nav vector icon, and strings in all
four locales the Showcase ships (`en`, `fr`, `ar`, `zh-CN` — no raw literals).

The page shows one tree deep enough to be interesting and small enough to read: a mock project
with folders, files and a nested folder, each row a disclosure plus an icon plus a name, and
below it a live readout of the current selection, expansion set and last move. Controls beside
it exercise the API rather than decorating it: a multi-select toggle, an "expand all" and
"collapse all" pair driving the expansion signal, a `move_guard` switch that refuses drops into
one particular folder (so the denied affordance is visible on every platform), and a
scroll-to-row field.

The walkthrough leg — `dayscript/outline.yaml`, joining the per-target list in the Showcase's
CI — asserts what no screenshot can: expanding a folder reveals exactly its children, collapsing
hides their ids, `outline_move` reparents and the readout agrees, a guarded move fails the step,
type-ahead lands on the expected row, and `assert_no_placeholders` holds on every target.

## Day Sketch: the layer panel

**The pane.** Day has no leading utility pane: `inspector` is the trailing one and
`selector(Sidebar)` is a navigation split. `InspectorProps` grows an `edge: Edge` (defaulting to
`Trailing`, so no existing app moves), and the four backends that realize the inspector map it:
`NSSplitViewItem.sidebar` rather than `.inspector`, `AdwOverlaySplitView`'s start side, the
first pane of the `QSplitter`, WinUI's `SplitView` pane. On phones the leading pane presents as
the same sheet the inspector already uses. Day Sketch then reads: outline on the leading edge,
canvas in the middle, inspector on the trailing edge, each independently collapsible.

**The source.** Day Sketch's scene is already a tree: `children_of(parent)` ordered by `z` is
`children_len` and `child_token`, `expandable` is `kind == NodeKind::Group`, and the row is a
label of the node's kind and id with the shape's fill as a swatch. Nothing new is stored.

**Selection.** The outline binds `.selected(|| model::selection().get())` and
`.on_selection(|keys| model::selection().set(keys))` — the same signal the canvas reads, so the
two stay in step without either knowing the other exists, and undo's transient selection
restoration already flows to both.

**Moving.** `on_move(node, parent, index)` writes `parent` and a fractional `z` between the new
neighbors in one undo unit labeled `move`, which is the write the Arrange menu already makes.
`move_guard` refuses a node into itself or a descendant, and refuses a drop into a leaf.

**Keyboard and type-ahead** come with the piece: arrows walk the tree, left and right climb and
open, type-ahead jumps by node name.

**Walkthrough.** The demo script gains a section that expands the group made earlier in the run,
asserts its two children appear, selects a row and asserts the canvas frame readout changes,
drags a shape into the group with `outline_move` and asserts the count and the group's bounds,
then undoes it — proving canvas and outline share one model rather than two.

## Risks worth deciding early

- **Token stability becomes a requirement.** `list` tolerates index churn; a tree does not.
  Sources whose keys move break expansion and selection, so the docs must say it and the mock
  tests must prove it.
- **Reload granularity.** v1 reloads the whole tree on a data change, as `list` does. Large
  outlines under frequent edits will want the keyed diff (`OutlinePatch::Splice`) sooner than
  lists did, because a reload also disturbs expansion.
- **Recycling is not universal.** AppKit, UIKit, GTK and XAML reuse cells; ArkUI's
  `TreeController` does not, and Qt's index-widget path would not either. `Cap::ListRecycling`
  has to answer honestly per backend, and the docs have to say which outlines stay small.
- **Two natives cannot guard a drag live.** ArkUI reports a move after the fact; the emulated
  path can be as live as Day makes it. `move_guard`'s contract must therefore be "consulted as
  early as the platform allows", with the per-backend table saying where that is — the same
  shape `list`'s reorder guard already documents.
- **What v1 leaves out, on purpose.** Dragging a multi-row selection as one unit;
  spring-loading a collapsed row under a hovering drag (native on AppKit, a timer elsewhere);
  columns beside the disclosure, which is where an outline becomes a tree *table* and wants
  `NSOutlineView`'s and `GtkColumnView`'s column machinery; and lazy children that arrive
  asynchronously, which the seam's synchronous `children_len` cannot express and which a file
  browser eventually needs.
