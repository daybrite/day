//! Recycling-list driver (docs/list.md, §10). The native list host owns scrolling + cell reuse;
//! Day owns row *content*. day-core injects a [`day_spec::ListSource`] into the backend; when the
//! native data-source pulls a cell, `bind_row` builds it once (per physical cell) and thereafter
//! *rebinds* it — one slot-write — as the cell recycles.
//!
//! Re-entrancy (the crux): building a row uses `BuildCx`, and reactive bindings patch native
//! widgets — both acquire `with_tree` per operation. So `bind_row` phases the tree borrow:
//! `with_tree` (adopt cell) → build/rebind + `flush_sync` **outside** any borrow → `with_tree`
//! (lay the row out in its cell). Holding the borrow across the build would deadlock the RefCell.

use crate::tree::{RNode, try_with_tree, with_tree};
use day_reactive::Scope;
use day_spec::{ListSource, props::RowHeight};
use std::collections::HashMap;
use std::rc::Rc;

/// Supplied by the `list()` piece, type-erased over the item type. day-core invokes these to
/// answer the native data-source and to build/rebind rows.
pub struct ListDriver {
    pub row_height: RowHeight,
    /// Current row count (reads the piece's snapshot; no tree access).
    pub len: Box<dyn Fn() -> usize>,
    /// Stable identity token for row `index` (for native diffing).
    pub token_at: Box<dyn Fn(usize) -> u64>,
    /// Build row `index` into `anchor`. Uses `BuildCx` internally, so it MUST be called with no
    /// `with_tree` borrow held. Returns the row's scope + a rebind writer.
    pub build: Box<dyn Fn(usize, RNode) -> BuiltRow>,
    /// Drag-to-reorder half, present when the piece is `.reorderable()` (docs/list.md).
    pub reorder: Option<ListReorderDriver>,
}

/// The reorder closures the `list()` piece supplies (docs/list.md). Exposed to the backend as
/// [`day_spec::ListReorder`] by [`make_source`]; also driven directly by
/// [`list_try_reorder`] (the dayscript `reorder` step and the mock probe).
pub struct ListReorderDriver {
    /// Consult the app's guard: returns the accepted target index, or -1 to deny.
    pub can_move: Box<dyn Fn(usize, usize) -> i64>,
    /// Commit an accepted move: rotates the piece's snapshot + tokens synchronously and defers
    /// the app's `on_reorder` callback to the next event drain.
    pub moved: Box<dyn Fn(usize, usize)>,
}

/// A freshly built row: its `Scope` (owns the row's reactive graph) and a rebind writer that
/// slot-writes item `index` into the row's `ItemSlot` when the cell is recycled.
pub struct BuiltRow {
    pub scope: Scope,
    pub rebind: Rc<dyn Fn(usize)>,
}

pub(crate) struct BoundCell {
    pub anchor: RNode,
    /// The row subtree's reactive scope — disposed by `remove_subtree` when the LIST node
    /// goes away (the cells live in the list machinery, not the node tree, so nothing else
    /// would dispose their bindings).
    pub scope: Scope,
    pub rebind: Rc<dyn Fn(usize)>,
}

pub(crate) struct ListState {
    pub driver: Rc<ListDriver>,
    /// Physical cell (native handle as usize) → its built row.
    pub cells: HashMap<usize, BoundCell>,
}

/// Whether a `bind_row` must build a new row (fresh anchor) or rebind a recycled cell.
pub enum CellStep {
    Build {
        anchor: RNode,
    },
    Rebind {
        rebind: Rc<dyn Fn(usize)>,
        anchor: RNode,
    },
}

/// Register a list's driver and wire its native host's data-source. Call after the LIST node and
/// its native handle exist (from within the piece build; `with_tree` is acquired per op).
pub fn install_list(node: RNode, driver: ListDriver) {
    with_tree(|t| t.install_list(node, driver));
}

/// Tell the native list its data changed (re-query the source). Call with no borrow held.
pub fn list_reload(node: RNode) {
    with_tree(|t| t.list_reload(node));
}

/// Imperatively scroll the native list so its last row is fully visible (chat "stick to bottom").
/// A no-op while the list is empty. Call with no borrow held.
pub fn list_scroll_to_end(node: RNode) {
    with_tree(|t| t.list_scroll_to_end(node));
}

/// Programmatically sync the list's selected rows (empty = clear selection). Applied by the
/// toolkit without re-emitting a selection event. Call with no borrow held.
pub fn list_set_selected(node: RNode, rows: Vec<usize>) {
    with_tree(|t| t.list_set_selected(node, rows));
}

/// Programmatically reorder a list row through the same guard → commit path a native drag takes
/// (docs/list.md): consult the piece's guard (which may retarget), rotate the snapshot, defer the
/// app's `on_reorder`, and tell the native host to re-query. This is how the dayscript `reorder`
/// step and the mock probe drive reordering without a native gesture. Returns the index the row
/// actually landed at. Call with no borrow held.
pub fn list_try_reorder(node: RNode, from: usize, to: usize) -> Result<usize, &'static str> {
    let driver = with_tree(|t| t.list_driver(node)).ok_or("no list at this node")?;
    let Some(re) = driver.reorder.as_ref() else {
        return Err("list is not reorderable");
    };
    let len = (driver.len)();
    if from >= len || to >= len {
        return Err("row index out of bounds");
    }
    let accepted = (re.can_move)(from, to);
    if accepted < 0 {
        return Err("reorder denied by the guard");
    }
    let accepted = (accepted as usize).min(len - 1);
    if accepted != from {
        (re.moved)(from, accepted);
    }
    // No native animation on this path — a reload re-binds the visible rows in the new order.
    list_reload(node);
    Ok(accepted)
}

/// Build the `ListSource` the backend calls from its data-source. `len`/`token_at` read the driver
/// directly (no tree). `bind_row` phases the tree borrow around the build + flush (see module doc).
pub(crate) fn make_source(node: RNode, driver: Rc<ListDriver>) -> ListSource {
    let (d_len, d_tok, d_reorder, d_bind) =
        (driver.clone(), driver.clone(), driver.clone(), driver);
    ListSource {
        len: Rc::new(move || (d_len.len)()),
        token_at: Rc::new(move |i| (d_tok.token_at)(i)),
        bind_row: Rc::new(move |index, cell| {
            let key = cell as usize;
            // A backend snapshot draws the window while holding the tree borrow; if that draw
            // re-enters here (a lazy list realizing a row mid-`cacheDisplayInRect`), skip rather
            // than double-borrow — the row binds on the next real layout pass (tree.rs::try_with_tree).
            let Some(step) = try_with_tree(|t| t.list_prepare_cell(node, key, cell)) else {
                return;
            };
            match step {
                CellStep::Build { anchor } => {
                    // Build outside the borrow — BuildCx re-acquires with_tree per op.
                    let built = (d_bind.build)(index, anchor);
                    with_tree(|t| t.list_store_cell(node, key, anchor, built));
                }
                CellStep::Rebind { rebind, .. } => rebind(index),
            }
            // Apply the slot-write (or first bindings); reactive effects patch natives via their
            // own with_tree — so this too runs with no borrow held. Then lay the row out.
            day_reactive::flush_sync();
            with_tree(|t| t.list_layout_cell(node, key));
        }),
        recycle: Rc::new(|_cell| { /* v1: cells stay cached in the reuse pool */ }),
        reorder: d_reorder.reorder.as_ref().map(|_| day_spec::ListReorder {
            can_move: {
                let d = d_reorder.clone();
                Rc::new(move |from, to| {
                    (d.reorder.as_ref().expect("mapped from Some").can_move)(from, to)
                })
            },
            move_row: {
                let d = d_reorder.clone();
                Rc::new(move |from, to| {
                    (d.reorder.as_ref().expect("mapped from Some").moved)(from, to)
                })
            },
        }),
    }
}
