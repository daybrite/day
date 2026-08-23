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
> **Status: designed, not built.** This document is the plan: the per-toolkit assessment that
> shaped it, the seam it adds, and the order to build it in. Nothing here ships yet, and the
> API below is the proposal to argue with before code exists.

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
| **Qt** | `QTreeView` | no — rows are painted by delegates; arbitrary widgets need `setIndexWidget` per row, which defeats virtualization | **emulated**, for the same reason Day's Qt list already is |
| **Android** | none | — | **emulated**; Material has no tree, and the platform idiom IS a flat `RecyclerView` with indentation and a chevron |
| **ArkUI** | none — the C node API's list vocabulary stops at `ARKUI_NODE_LIST` and its items | — | **emulated**; the ArkTS advanced component set has a tree, the node API Day speaks does not |
| **web-dom** | none | — | **emulated**; the browser has no tree element, only `role="tree"` and the ARIA pattern |
| **mock** | simulated | yes | drives the tests, as it does for `list` |

Four toolkits carry a real tree that will host Day's rows. Four have none that fits. That
split decides the architecture. The seam is hierarchical, so the native trees drive it
directly, and ONE flattener in `day-core` turns the same seam into indented rows for everyone
else — not four hand-rolled imitations.

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

## One flattener for the four without a tree

`day-core` turns an `OutlineSource` plus the expansion set into the flat row sequence the
emulated backends render: a walk from the root that descends only into open rows, producing
`(token, depth)` pairs. The emulated backends then reuse the **list** cell machinery unchanged
and wrap each row in an indent plus a disclosure button built from ordinary Day pieces.

That is one implementation of expansion, indentation, keyboard handling and drop targeting,
shared by Qt, Android, ArkUI and web-dom, exercised by the mock backend's tests. A bug fixed
in the flattener is fixed on four platforms.

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
- Rows carry ids, so `tap`, `assert_text` and `assert_missing` already work on them.

## Order of work

Mock-first, exactly as `list` was built, so the driver is proven before any backend:

1. **Spec + pieces + core + mock.** `OutlineSource`, the flattener, the expansion and selection
   plumbing, `MockProbe` hooks, e2e tests: only visible rows build, collapse hides descendants,
   a move rewrites the snapshot before the native animation, guards deny.
2. **AppKit.** `NSOutlineView` over the existing view-based row path — the reference native
   implementation, and the one whose drop vocabulary the seam is shaped after.
3. **The emulation, on web-dom and Qt.** Both already emulate `list`, so they exercise the
   shared flattener path first and cheapest.
4. **GTK and UIKit.** `GtkTreeListModel` + `GtkTreeExpander`; `UICollectionView` sidebar lists
   with outline disclosure accessories.
5. **XAML, Android, ArkUI.** WinUI `TreeView` native; the other two on the shared emulation.
6. **Day Sketch.** The layer panel, plus walkthrough legs on every target.

## What Day Sketch needs beyond the piece

The panel goes on the **leading** edge, and Day has no piece for that. [`inspector`](inspector.md)
is the trailing utility pane; `selector(Sidebar)` is a navigation split, not a place to put a
panel. The smallest honest fix is an edge on the pane Day already has —
`inspector(...).edge(Edge::Leading)` — resolving to `NSSplitViewItem.sidebar` rather than
`.inspector`, `AdwOverlaySplitView`'s start side, the first pane of a `QSplitter`, WinUI's
`SplitView` pane, and on phones the same sheet the inspector already uses.

## Risks worth deciding early

- **Token stability becomes a requirement.** `list` tolerates index churn; a tree does not.
  Sources whose keys move break expansion and selection, so the docs must say it and the mock
  tests must prove it.
- **Reload granularity.** v1 reloads the whole tree on a data change, as `list` does. Large
  outlines under frequent edits will want the keyed diff (`OutlinePatch::Splice`) sooner than
  lists did, because a reload also disturbs expansion.
- **Qt's real tree stays on the table.** `QTreeView` is genuinely good; it is Day's
  widgets-in-cells model that makes it a poor fit today. If Day ever grows delegate-drawn rows,
  Qt should move from emulated to native, and the seam above is already the shape its model
  wants.
