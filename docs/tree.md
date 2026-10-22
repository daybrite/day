---
title: "Tree"
description: "Display hierarchical data with stable row IDs, expansion, and selection."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Tree

The tree piece displays hierarchical data with expandable rows. Use it for document outlines,
folders, and grouped navigation.

## Signature

API excerpt; not a complete app. `NodeSource` and its slot/key types are defined in
[structure.rs](../crates/day-pieces/src/structure.rs).

```rust
pub fn tree<S, P>(source: S, build_row: impl Fn(S::Slot) -> P + 'static) -> TreePiece<S>
where
    S: NodeSource + 'static,
    P: Piece;
```

Use `branches(items_fn, key_of, parent_of)` for plain data, or a model store's tree adapter with
the `model` feature. A source key must be stable; slots are recycled values, not permanent row
identity. `branches` preserves sibling input order and treats an absent parent key as a root.
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
| `row_height` | `RowHeight::Automatic`; `Uniform(h)` supplies fixed row sizing |
| `indent` | Unset: toolkit default indentation |
| `section_headers()` | Off; when enabled, root branches are nonselectable headings and children must be leaves |
| `expanded(binding)` | Optional app-owned `HashSet<Key>`; user disclosure writes it; app writes update the tree |
| `on_expansion(f)` | Reports user/script changes, not programmatic expansion writes |
| `expandable(f)` | Defaults to having children now; override for an empty branch that can accept children |
| `selected(f)` / `on_selection(f)` | Optional selection source and callback; connect both for two-way selection |
| `row_id(f)` | Optional stable element ID derived from the current key; reevaluated during recycling |
| `row_context_menu(f)` | Optional menu built when summoned for the current key |

Multi-selection and movement are off by default. The
[TreePiece API](/api/day_pieces/struct.TreePiece.html) lists move guards,
reveal, and type-ahead options.
`tree` does not return a construction `Result`; malformed source data is not thereby validated.

## Platform rendering

| Toolkit | Rendering |
| --- | --- |
| AppKit | Native [`NSOutlineView`](https://developer.apple.com/documentation/appkit/nsoutlineview) |
| UIKit | Native collection-view list |
| GTK | Native tree-list model and expander rows |
| Qt | Native [`QTreeWidget`](https://doc.qt.io/qt-6/qtreewidget.html) |
| WinUI / XAML | Native [`TreeView`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.treeview) hierarchy |
| Android | Composed list for arbitrary depth; native `ExpandableListView` for single-selection sections |
| ArkUI / DOM | Composed tree built from list rows |

The piece uses a native tree when `Cap::Tree` is `Native`. Otherwise it flattens expanded
branches into list rows while retaining the same expansion and selection API. A two-level
`section_headers()` tree can use `Cap::TreeSections`; multi-selection uses the full-tree path.

Use [lists](list.md) for flat rows and [navigation](navigation.md) for destinations.

## Platform guidance

- [Windows: tree views](https://learn.microsoft.com/en-us/windows/apps/design/controls/tree-view)
