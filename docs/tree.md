---
navTitle: "Trees"
title: "Tree"
description: "Display hierarchical data with stable row IDs, expansion, and selection."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Tree

A tree presents items in a hierarchy, with branches that expand to reveal their children. It helps
people explore nested content such as folders or a document outline without showing every level at
once.

## Add to your app

This control is included in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces)
and re-exported through `day::prelude::*`. No additional crate or control feature is required.

## Signature

API excerpt; not a complete app. `NodeSource` and its slot/key types are defined in
[structure.rs](../crates/day-pieces/src/structure.rs).

```rust
pub fn tree<S, P>(source: S, build_row: impl Fn(S::Slot) -> P + 'static) -> TreePiece<S>
where
    S: NodeSource + 'static,
    P: Piece;
```

Use [`branches(items_fn, key_of, parent_of)`](https://docs.rs/day-pieces/latest/day_pieces/fn.branches.html) for plain data, or a model store's tree adapter with
the `model` feature. A source key must be stable; slots are recycled values, not permanent row
identity. [`branches`](https://docs.rs/day-pieces/latest/day_pieces/fn.branches.html) preserves sibling input order and treats an absent parent key as a root.
The row builder runs on the UI thread and returns a Piece; render changing slot values through
tracked reads. Use generated accessors for app-owned row labels and keep user data separate.

## Show hierarchical data

This module accepts document titles supplied by the user. They are data, while the tree's
accessible label comes from the app's Fluent catalog. The recipe harness compiles the module;
call `outline` with a signal of your entries.

<!-- technical-example: tree_recipe -->
```rust
use crate::res;
use day::prelude::*;

#[derive(Clone)]
pub struct Entry {
    pub key: u64,
    pub parent: Option<u64>,
    pub title: String,
}

pub fn outline(entries: Signal<Vec<Entry>>) -> impl Piece {
    tree(
        branches(
            move || entries.get(),
            |entry| entry.key,
            |entry| entry.parent,
        ),
        |row: ItemSlot<Entry, u64>| label(move || row.get().title),
    )
    .row_id(|key: &u64| id!(project_node, *key))
    .id(id!(project_tree))
    .a11y(|a| a.label(res::str::project_tree_label()))
}
```

<!-- technical-catalog: tree_recipe -->
```ftl
project_tree_label = Project outline
```

A `None` parent creates a root row. A child names its parent's stable key. Keep keys independent
of the row's title so renaming or translating a title does not change its test identity.

## Defaults and state

| Builder | Default and behavior |
| --- | --- |
| [`row_height`](https://docs.rs/day-pieces/latest/day_pieces/trait.TreeBuilder.html#tymethod.row_height) | `RowHeight::Automatic`; `Uniform(h)` supplies fixed row sizing |
| [`indent`](https://docs.rs/day-pieces/latest/day_pieces/trait.TreeBuilder.html#tymethod.indent) | Unset: toolkit default indentation |
| [`section_headers()`](https://docs.rs/day-pieces/latest/day_pieces/trait.TreeBuilder.html#tymethod.section_headers) | Off; when enabled, root branches are nonselectable headings and children must be leaves |
| [`expanded(binding)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TreeBuilder.html#tymethod.expanded) | Optional app-owned `HashSet<Key>`; user disclosure writes it; app writes update the tree |
| [`on_expansion(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TreeBuilder.html#tymethod.on_expansion) | Reports user/script changes, not programmatic expansion writes |
| [`expandable(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TreeBuilder.html#tymethod.expandable) | Defaults to having children now; override for an empty branch that can accept children |
| [`selected(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TreeBuilder.html#tymethod.selected) / [`on_selection(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TreeBuilder.html#tymethod.on_selection) | Optional selection source and callback; connect both for two-way selection |
| [`row_id(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TreeBuilder.html#tymethod.row_id) | Optional stable element ID derived from the current key; reevaluated during recycling |
| [`row_context_menu(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TreeBuilder.html#tymethod.row_context_menu) | Optional menu built when summoned for the current key |

Multi-selection and movement are off by default. The
[TreePiece API](https://docs.rs/day-pieces/latest/day_pieces/struct.TreePiece.html) lists move guards,
reveal, and type-ahead options.
[`tree`](https://docs.rs/day-pieces/latest/day_pieces/fn.tree.html) does not return a construction `Result`; malformed source data is not thereby validated.

## Toolkit implementations

| Toolkit | Rendering |
| --- | --- |
| AppKit | Native [`NSOutlineView`](https://developer.apple.com/documentation/appkit/nsoutlineview) |
| UIKit | Native collection-view list |
| GTK | Native tree-list model and expander rows |
| Qt | Native [`QTreeWidget`](https://doc.qt.io/qt-6/qtreewidget.html) |
| WinUI | Native [`TreeView`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.treeview) hierarchy |
| Android | Composed list for arbitrary depth; native `ExpandableListView` for single-selection sections |
| ArkUI / DOM | Composed tree built from list rows |

The piece uses a native tree when `Cap::Tree` is `Native`. Otherwise it flattens expanded
branches into list rows while retaining the same expansion and selection API. A two-level
[`section_headers()`](https://docs.rs/day-pieces/latest/day_pieces/trait.TreeBuilder.html#tymethod.section_headers) tree can use `Cap::TreeSections`; multi-selection uses the full-tree path.

Use [lists](list.md) for flat rows and [navigation](navigation.md) for destinations.

## Platform guidance

- [Windows: tree views](https://learn.microsoft.com/en-us/windows/apps/design/controls/tree-view)
