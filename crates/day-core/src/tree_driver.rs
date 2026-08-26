// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Recycling-tree driver (docs/tree.md). [`crate::list`]'s shape addressed by TOKEN instead of
//! row index: the native tree host owns scrolling, disclosure and cell reuse; Day owns row
//! *content*. day-core injects a [`day_spec::TreeSource`] into the backend; when the native
//! data-source pulls a cell, `bind_row` builds it once (per physical cell) and thereafter
//! *rebinds* it — one slot-write — as the cell recycles.
//!
//! Re-entrancy follows `list.rs` exactly: `bind_row` phases the tree borrow —
//! `with_tree` (adopt cell) → build/rebind + `flush_sync` **outside** any borrow → `with_tree`
//! (lay the row out in its cell). Holding the borrow across the build would deadlock the RefCell.

use crate::tree::{RNode, try_with_tree, with_tree};
use day_reactive::Scope;
use day_spec::{MoveVerdict, TreeSource, props::RowHeight};
use std::collections::HashMap;
use std::rc::Rc;

/// Supplied by the `tree()` piece, type-erased over the item type. day-core invokes these to
/// answer the native data-source and to build/rebind rows. All hierarchy queries are
/// token-addressed (`None` = the root level); see docs/tree.md for why indices cannot
/// address tree rows.
pub struct TreeDriver {
    pub row_height: RowHeight,
    /// How many children `parent` has (reads the piece's snapshot; no tree access).
    pub children_len: Box<dyn Fn(Option<u64>) -> usize>,
    /// The i-th child of `parent` — the stable token every backend keys its rows by.
    pub child_token: Box<dyn Fn(Option<u64>, usize) -> u64>,
    /// Whether this token can hold children at all (draws or omits the disclosure).
    pub expandable: Box<dyn Fn(u64) -> bool>,
    /// The piece's CURRENT desired expansion state for this token (untracked read of the
    /// app's expansion signal) — what the flattener descends into.
    pub expanded: Box<dyn Fn(u64) -> bool>,
    /// Build the row for `token` into `anchor`. Uses `BuildCx` internally, so it MUST be
    /// called with no `with_tree` borrow held. Returns the row's scope + a rebind writer.
    pub build: Box<dyn Fn(u64, RNode) -> TreeBuiltRow>,
    /// The row's type-ahead string (docs/tree.md) — native type-select answers from it.
    pub type_select_text: Box<dyn Fn(u64) -> String>,
    /// Resolve a dayscript row id (the piece's `.row_id` string) to its token — how the
    /// `expand:`/`tree_move:` steps and the mock address rows without native gestures.
    pub resolve_row: RowResolver,
    /// Drag-to-move half, present when the piece is `.movable(true)` (docs/tree.md).
    pub moves: Option<TreeMovesDriver>,
}

/// A `.row_id` string → token resolver (aliased for the field above; clippy's
/// type-complexity bound).
pub type RowResolver = Box<dyn Fn(&str) -> Option<u64>>;

/// The move closures the `tree()` piece supplies (docs/tree.md). Exposed to the backend as
/// [`day_spec::TreeMoves`] by [`make_tree_source`]; also driven directly by [`tree_try_move`]
/// (the dayscript `tree_move:` step and the mock probe).
pub struct TreeMovesDriver {
    /// Consult the guards (structural + the app's): may `token` land under `parent` at
    /// `index`? `index: None` = dropped ONTO the parent (append).
    #[allow(clippy::type_complexity)]
    pub can_move: Box<dyn Fn(u64, Option<u64>, Option<usize>) -> MoveVerdict>,
    /// Commit an accepted move: defers the app's `on_move` through the event queue
    /// ([`day_spec::Event::TreeMove`]); the app's own data write drives the reload.
    #[allow(clippy::type_complexity)]
    pub moved: Box<dyn Fn(u64, Option<u64>, Option<usize>)>,
}

/// A freshly built tree row: its `Scope` (owns the row's reactive graph) and a rebind writer
/// that slot-writes `token`'s row into the cell's slot when the cell is recycled.
pub struct TreeBuiltRow {
    pub scope: Scope,
    pub rebind: Rc<dyn Fn(u64)>,
}

pub(crate) struct TreeBoundCell {
    pub anchor: RNode,
    /// The row subtree's reactive scope — disposed by `remove_subtree` when the TREE node
    /// goes away (cells live in the tree machinery, not the node tree).
    pub scope: Scope,
    pub rebind: Rc<dyn Fn(u64)>,
}

pub(crate) struct TreeState {
    pub driver: Rc<TreeDriver>,
    /// Physical cell (native handle as usize) → its built row.
    pub cells: HashMap<usize, TreeBoundCell>,
}

/// Whether a `bind_row` must build a new row (fresh anchor) or rebind a recycled cell.
pub enum TreeCellStep {
    Build {
        anchor: RNode,
    },
    Rebind {
        rebind: Rc<dyn Fn(u64)>,
        anchor: RNode,
    },
}

/// Register a tree's driver and wire its native host's data-source. Call after the TREE node
/// and its native handle exist (from within the piece build; `with_tree` is acquired per op).
pub fn install_tree(node: RNode, driver: TreeDriver) {
    with_tree(|t| t.install_tree(node, driver));
}

/// Tell the native tree its data changed (re-query the source). Expansion and selection
/// survive by token (docs/tree.md). Call with no borrow held.
pub fn tree_reload(node: RNode) {
    with_tree(|t| t.tree_reload(node));
}

/// Programmatically disclose or collapse one row. Applied by the toolkit without re-emitting
/// `Event::TreeExpanded`; redundant applications are no-ops. Call with no borrow held.
pub fn tree_set_expanded(node: RNode, token: u64, expanded: bool) {
    with_tree(|t| t.tree_set_expanded(node, token, expanded));
}

/// Programmatically sync the tree's selected rows (empty = clear). Applied by the toolkit
/// without re-emitting a selection event. Call with no borrow held.
pub fn tree_set_selected(node: RNode, tokens: Vec<u64>) {
    with_tree(|t| t.tree_set_selected(node, tokens));
}

/// Scroll this row into view, realizing it if needed. The piece expands the row's ancestors
/// (through the app's expansion signal) BEFORE issuing this. Call with no borrow held.
pub fn tree_reveal(node: RNode, token: u64) {
    with_tree(|t| t.tree_reveal(node, token));
}

/// The tree's driver, for the guard → commit paths run outside the borrow (`None` when
/// `node` hosts no tree).
pub fn tree_driver(node: RNode) -> Option<Rc<TreeDriver>> {
    with_tree(|t| t.tree_driver(node))
}

/// Programmatically move a tree row through the same guard → commit path a native drag takes
/// (docs/tree.md): consult the piece's guards, defer the app's `on_move`, and tell the native
/// host to re-query. This is how the dayscript `tree_move:` step and the mock probe drive
/// moves without a native gesture. Call with no borrow held.
pub fn tree_try_move(
    node: RNode,
    token: u64,
    parent: Option<u64>,
    index: Option<usize>,
) -> Result<(), &'static str> {
    let driver = tree_driver(node).ok_or("no tree at this node")?;
    let Some(mv) = driver.moves.as_ref() else {
        return Err("tree is not movable");
    };
    match (mv.can_move)(token, parent, index) {
        MoveVerdict::Deny => Err("move denied by the guard"),
        MoveVerdict::Allow => {
            (mv.moved)(token, parent, index);
            Ok(())
        }
    }
}

/// The flattened VISIBLE rows — `(token, depth)` in display order, descending only into
/// expanded rows (docs/tree.md). The shared substrate the emulated backends, the keyboard
/// handler and the mock probe all read; one implementation, kind-agnostic over any token tree.
pub fn tree_visible_rows(driver: &TreeDriver) -> Vec<(u64, u16)> {
    let mut out = Vec::new();
    // Manual stack, children pushed in reverse so pop order is display order.
    let mut stack: Vec<(Option<u64>, u16)> = vec![(None, 0)];
    while let Some((parent, depth)) = stack.pop() {
        if let Some(tok) = parent {
            out.push((tok, depth - 1));
            if !(driver.expandable)(tok) || !(driver.expanded)(tok) {
                continue;
            }
        }
        let n = (driver.children_len)(parent);
        for i in (0..n).rev() {
            stack.push((Some((driver.child_token)(parent, i)), depth + 1));
        }
    }
    out
}

/// Build the `TreeSource` the backend calls from its data-source. The hierarchy queries read
/// the driver directly (no tree); `bind_row` phases the tree borrow around the build + flush
/// exactly as the list's does (see module doc).
pub(crate) fn make_tree_source(node: RNode, driver: Rc<TreeDriver>) -> TreeSource {
    let (d_len, d_tok, d_exp, d_type, d_moves, d_bind) = (
        driver.clone(),
        driver.clone(),
        driver.clone(),
        driver.clone(),
        driver.clone(),
        driver,
    );
    TreeSource {
        children_len: Rc::new(move |p| (d_len.children_len)(p)),
        child_token: Rc::new(move |p, i| (d_tok.child_token)(p, i)),
        expandable: Rc::new(move |t| (d_exp.expandable)(t)),
        type_select_text: Rc::new(move |t| (d_type.type_select_text)(t)),
        bind_row: Rc::new(move |token, cell| {
            let key = cell as usize;
            // Same skip rule as the list: a backend snapshot drawing inside a with_tree
            // borrow may re-enter here — bind on the next real layout pass instead.
            let Some(step) = try_with_tree(|t| t.tree_prepare_cell(node, key, cell)) else {
                return;
            };
            match step {
                TreeCellStep::Build { anchor } => {
                    // Build outside the borrow — BuildCx re-acquires with_tree per op.
                    let built = (d_bind.build)(token, anchor);
                    with_tree(|t| t.tree_store_cell(node, key, anchor, built));
                }
                TreeCellStep::Rebind { rebind, .. } => rebind(token),
            }
            day_reactive::flush_sync();
            with_tree(|t| t.tree_layout_cell(node, key));
        }),
        recycle: Rc::new(move |cell| {
            // The pooled cell keeps its built row (rebind is the fast path), but its element
            // ids go now — see `TreeOps::tree_recycle_cell`. Skip quietly inside a borrow;
            // the backend defers this call to a safe turn.
            let _ = try_with_tree(|t| t.tree_recycle_cell(node, cell as usize));
        }),
        layout_cell: Rc::new(move |cell, width| {
            // From the native cell's own layout pass — skip inside a snapshot borrow, the
            // next real pass corrects it (same rule as bind_row).
            let _ = try_with_tree(|t| t.tree_layout_cell_width(node, cell as usize, width));
        }),
        moves: d_moves.moves.as_ref().map(|_| day_spec::TreeMoves {
            can_move: {
                let d = d_moves.clone();
                Rc::new(move |tok, parent, idx| {
                    (d.moves.as_ref().expect("mapped from Some").can_move)(tok, parent, idx)
                })
            },
            move_node: {
                let d = d_moves.clone();
                Rc::new(move |tok, parent, idx| {
                    (d.moves.as_ref().expect("mapped from Some").moved)(tok, parent, idx)
                })
            },
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed tree: 1{ 2, 3{ 4 } }, 5 — driver closures over static tables.
    fn fixture(open: &'static [u64]) -> TreeDriver {
        fn kids(parent: Option<u64>) -> &'static [u64] {
            match parent {
                None => &[1, 5],
                Some(1) => &[2, 3],
                Some(3) => &[4],
                _ => &[],
            }
        }
        TreeDriver {
            row_height: RowHeight::Automatic,
            children_len: Box::new(|p| kids(p).len()),
            child_token: Box::new(|p, i| kids(p)[i]),
            expandable: Box::new(|t| !kids(Some(t)).is_empty()),
            expanded: Box::new(move |t| open.contains(&t)),
            build: Box::new(|_, _| unreachable!("flattener never builds")),
            type_select_text: Box::new(|_| String::new()),
            resolve_row: Box::new(|_| None),
            moves: None,
        }
    }

    #[test]
    fn flattener_descends_only_into_expanded_rows() {
        assert_eq!(tree_visible_rows(&fixture(&[])), [(1, 0), (5, 0)]);
        assert_eq!(
            tree_visible_rows(&fixture(&[1])),
            [(1, 0), (2, 1), (3, 1), (5, 0)]
        );
        assert_eq!(
            tree_visible_rows(&fixture(&[1, 3])),
            [(1, 0), (2, 1), (3, 1), (4, 2), (5, 0)]
        );
        // An expanded token that is not itself visible (3 without 1) changes nothing.
        assert_eq!(tree_visible_rows(&fixture(&[3])), [(1, 0), (5, 0)]);
    }
}
