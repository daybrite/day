//! Recycling-list driver (docs/list.md, §10). The native list host owns scrolling + cell reuse;
//! day owns row *content*. day-core injects a [`day_spec::ListSource`] into the backend; when the
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
}

/// A freshly built row: its `Scope` (owns the row's reactive graph) and a rebind writer that
/// slot-writes item `index` into the row's `ItemSlot` when the cell is recycled.
pub struct BuiltRow {
    pub scope: Scope,
    pub rebind: Rc<dyn Fn(usize)>,
}

pub(crate) struct BoundCell {
    pub anchor: RNode,
    pub _scope: Scope,
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

/// Build the `ListSource` the backend calls from its data-source. `len`/`token_at` read the driver
/// directly (no tree). `bind_row` phases the tree borrow around the build + flush (see module doc).
pub(crate) fn make_source(node: RNode, driver: Rc<ListDriver>) -> ListSource {
    let (d_len, d_tok, d_bind) = (driver.clone(), driver.clone(), driver);
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
    }
}
