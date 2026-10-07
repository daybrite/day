// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The recycling list (docs/list.md): an `ARKUI_NODE_LIST` driven by a `NodeAdapter`
//! (`ohos_sys::arkui` `native_node`).
//!
//! A cell is a LIST_ITEM wrapping an inner Stack that Day mounts the row subtree into. Cells
//! scrolled out of view are pushed to a REUSE POOL rather than disposed, so the inner Stack
//! pointer stays stable and day-core's cell cache rebinds it (Day's `recycle` is a no-op; cells
//! "stay cached"). Row counts, binds and the reorder/delete seams come from `lib.rs`'s list
//! callbacks.

// Node handles are opaque runtime tokens (see node.rs), never dereferenced here.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;

use ohos_sys::arkui::drag_and_drop::{
    ArkUI_DragResult, OH_ArkUI_AllowNodeAllDropDataTypes,
    OH_ArkUI_DragEvent_GetTouchPointYToWindow, OH_ArkUI_DragEvent_SetDragResult,
    OH_ArkUI_NodeEvent_GetDragEvent, OH_ArkUI_SetNodeDraggable,
};
use ohos_sys::arkui::native_node::{
    ArkUI_NodeAdapterEvent, ArkUI_NodeAdapterEventType as AdapterEv, ArkUI_NodeAdapterHandle,
    ArkUI_NodeAttributeType as Attr, ArkUI_NodeEvent, ArkUI_NodeEventType as Ev,
    OH_ArkUI_NodeAdapter_Create, OH_ArkUI_NodeAdapter_Dispose,
    OH_ArkUI_NodeAdapter_GetTotalNodeCount, OH_ArkUI_NodeAdapter_RegisterEventReceiver,
    OH_ArkUI_NodeAdapter_ReloadAllItems, OH_ArkUI_NodeAdapter_SetTotalNodeCount,
    OH_ArkUI_NodeAdapter_UnregisterEventReceiver, OH_ArkUI_NodeAdapterEvent_GetItemIndex,
    OH_ArkUI_NodeAdapterEvent_GetRemovedNode, OH_ArkUI_NodeAdapterEvent_GetType,
    OH_ArkUI_NodeAdapterEvent_GetUserData, OH_ArkUI_NodeAdapterEvent_SetItem,
    OH_ArkUI_NodeAdapterEvent_SetNodeId, OH_ArkUI_NodeEvent_GetNodeHandle,
    OH_ArkUI_NodeUtils_GetLayoutPositionInWindow,
};
use ohos_sys::arkui::native_type::{
    ArkUI_IntOffset, ArkUI_ListItemSwipeActionOption, ArkUI_TextAlignment,
    OH_ArkUI_ListItemSwipeActionItem_Create, OH_ArkUI_ListItemSwipeActionItem_SetContent,
    OH_ArkUI_ListItemSwipeActionItem_SetOnActionWithUserData,
    OH_ArkUI_ListItemSwipeActionOption_Create, OH_ArkUI_ListItemSwipeActionOption_Dispose,
    OH_ArkUI_ListItemSwipeActionOption_SetEnd,
};

use crate::node::{self, Handle};

struct List {
    adapter: ArkUI_NodeAdapterHandle,
    host_id: u64,
    /// px; 0 = content-sized
    row_h: f32,
    /// Rows report taps as selection and paint the programmatic set (docs/list.md).
    selectable: bool,
    /// Bumped per reload so every row gets a FRESH adapter item id (see GET_NODE_ID): the
    /// adapter diffs by id, and ids that stay put keep their old bindings; a reload after a
    /// reparent/insert above the tail left every untouched cell showing the wrong row.
    generation: u32,
    pool: Vec<Handle>,
    /// Drag-to-reorder (docs/list.md): whether rows drag, the list node (for geometry), and
    /// each live cell's currently-bound row (cells recycle, so the row is re-recorded on every
    /// bind).
    reorderable: bool,
    node: Handle,
    rows: HashMap<usize, i32>,
    /// Swipe-to-delete (docs/list.md): whether rows swipe, and the app's already-localized word
    /// for the action (empty ⇒ no text, matching the other backends' wordless fallback).
    deletable: bool,
    delete_label: String,
    /// Per-cell swipe bookkeeping, so the action callback can find which ROW the cell is bound
    /// to at the moment it fires: cells recycle, so the row is not fixed at creation.
    swipe_opts: HashMap<usize, *mut ArkUI_ListItemSwipeActionOption>,
    /// The per-cell swipe user data records, owned by the list and freed with it.
    swipe_cells: Vec<*mut SwipeCell>,
    /// A row jump asked for before the adapter had rows (a list the page scrolls as it builds):
    /// the next reload applies it.
    pending_scroll: Option<u32>,
}

/// User data for a cell's swipe action. The row is not captured here: cells recycle, so the
/// bound row is read from the list's rows when the action actually fires.
struct SwipeCell {
    list: *mut List,
    cell: Handle,
}

thread_local! {
    /// List node → its adapter binding. Boxed, so the adapter's user-data pointer is stable.
    static LISTS: RefCell<HashMap<usize, Box<List>>> = RefCell::new(HashMap::new());
    /// The in-flight reorder drag (one at a time; Day lists accept only their OWN rows).
    static DRAG: RefCell<Option<(*mut List, i32)>> = const { RefCell::new(None) };
}

/// A list's binding as a raw pointer, for the callbacks that hold no map borrow.
fn list_ptr(n: Handle) -> *mut List {
    LISTS.with(|l| {
        l.borrow_mut()
            .get_mut(&(n as usize))
            .map_or(ptr::null_mut(), |b| ptr::from_mut(&mut **b))
    })
}

/// One cell's selection fill (docs/list.md `ListPatch::Selected`): the HarmonyOS accent at
/// 20% alpha as the LIST_ITEM's background; Day's row content paints above it.
fn paint_cell(dl: &List, cell: Handle, row: i32) {
    if !dl.selectable {
        return;
    }
    let sel = crate::list_is_selected(dl.host_id, row as u32);
    node::set_bg_color(cell, if sel { 0x330A_59F7 } else { 0x0000_0000 });
}

/// The user completed the swipe's delete action on this cell.
unsafe extern "C" fn swipe_action(user_data: *mut c_void) {
    day_spec::ffi_guard::contain((), || {
        // SAFETY: a SwipeCell the list owns until it is disposed.
        let sc = unsafe { &*user_data.cast::<SwipeCell>() };
        if sc.list.is_null() {
            return;
        }
        // SAFETY: the list this record belongs to, alive while its cells are.
        let dl = unsafe { &mut *sc.list };
        let Some(row) = dl.rows.get(&(sc.cell as usize)).copied() else {
            return;
        };
        if crate::list_delete(dl.host_id, row as u32) {
            // The seam already shortened Day's snapshot; re-query so the adapter drops the row.
            let count = crate::list_count(dl.host_id);
            // SAFETY: a live adapter.
            unsafe { OH_ArkUI_NodeAdapter_SetTotalNodeCount(dl.adapter, count) };
        }
    });
}

/// Build the trailing swipe action for a cell: a text button carrying the app's own word (or a
/// bare destructive area when it supplied none), revealed as the row slides.
fn attach_swipe(dl: &mut List, cell: Handle) {
    if !dl.deletable || dl.swipe_opts.contains_key(&(cell as usize)) {
        return;
    }
    let btn = node::create(node::TEXT);
    if !dl.delete_label.is_empty() {
        node::set_text(btn, &dl.delete_label);
    }
    node::set_f32(btn, Attr::NODE_WIDTH, 96.0);
    if dl.row_h > 0.0 {
        node::set_f32(btn, Attr::NODE_HEIGHT, dl.row_h / node::density() as f32);
    }
    // The M3/HarmonyOS destructive red, matching the other backends' delete field.
    node::set_bg_color(btn, 0xFFB3_261E);
    node::set_font_color(btn, 0xFFFF_FFFF);
    node::set_i32(
        btn,
        Attr::NODE_TEXT_ALIGN,
        ArkUI_TextAlignment::ARKUI_TEXT_ALIGNMENT_CENTER.0 as i32,
    );
    let sc = Box::into_raw(Box::new(SwipeCell {
        list: ptr::from_mut(dl),
        cell,
    }));
    dl.swipe_cells.push(sc);
    // SAFETY: option and item objects the list owns until it is disposed.
    unsafe {
        let item = OH_ArkUI_ListItemSwipeActionItem_Create();
        OH_ArkUI_ListItemSwipeActionItem_SetContent(item, btn);
        OH_ArkUI_ListItemSwipeActionItem_SetOnActionWithUserData(
            item,
            sc.cast(),
            Some(swipe_action),
        );
        let opt = OH_ArkUI_ListItemSwipeActionOption_Create();
        // END, not START: the trailing edge, which is what UIKit and ItemTouchHelper both use
        // and what RTL mirrors for free.
        OH_ArkUI_ListItemSwipeActionOption_SetEnd(opt, item);
        node::set_object(cell, Attr::NODE_LIST_ITEM_SWIPE_ACTION, opt.cast());
        dl.swipe_opts.insert(cell as usize, opt);
    }
}

unsafe extern "C" fn adapter_receiver(ev: *mut ArkUI_NodeAdapterEvent) {
    day_spec::ffi_guard::contain((), || {
        // SAFETY: a live adapter event whose user data is the boxed List.
        unsafe {
            let dl = OH_ArkUI_NodeAdapterEvent_GetUserData(ev).cast::<List>();
            if dl.is_null() {
                return;
            }
            let dl = &mut *dl;
            match OH_ArkUI_NodeAdapterEvent_GetType(ev) {
                AdapterEv::NODE_ADAPTER_EVENT_ON_GET_NODE_ID => {
                    // Generation-salted (kept positive in the int32 id): a reload renames every
                    // row, which is what makes ReloadAllItems REMOVE + re-ADD them all, the
                    // full-rebind reload the other emulated lists do, instead of keeping stale
                    // binds.
                    let id = ((dl.generation % 21474) * 100_000)
                        + OH_ArkUI_NodeAdapterEvent_GetItemIndex(ev);
                    OH_ArkUI_NodeAdapterEvent_SetNodeId(ev, id as i32);
                }
                AdapterEv::NODE_ADAPTER_EVENT_ON_ADD_NODE_TO_ADAPTER => {
                    let idx = OH_ArkUI_NodeAdapterEvent_GetItemIndex(ev);
                    let cell = match dl.pool.pop() {
                        Some(cell) => cell,
                        None => {
                            let cell = node::create(node::LIST_ITEM);
                            let inner = node::create(node::STACK);
                            if dl.row_h > 0.0 {
                                let h = dl.row_h / node::density() as f32;
                                node::set_f32(cell, Attr::NODE_HEIGHT, h);
                                node::set_f32(inner, Attr::NODE_HEIGHT, h);
                            }
                            node::add_child(cell, inner);
                            if dl.reorderable {
                                // Long-press lifts the row with ArkUI's own drag preview; the
                                // drop lands on the LIST node's NODE_ON_DROP (registered in
                                // `init`).
                                OH_ArkUI_SetNodeDraggable(cell, true);
                                node::register_event(cell, Ev::NODE_ON_DRAG_START, 0);
                            }
                            if dl.selectable {
                                // A tap selects the bound row (docs/list.md); the shared
                                // receiver resolves the cell through `cell_click`.
                                node::register_event(cell, Ev::NODE_ON_CLICK, 0);
                            }
                            attach_swipe(dl, cell);
                            cell
                        }
                    };
                    // Unconditional: the swipe action resolves its row through this map too,
                    // and a guarded row is filtered at delete time by the seam's own
                    // `can_delete`.
                    dl.rows.insert(cell as usize, idx as i32);
                    let inner = node::child_at(cell, 0);
                    // Build (fresh) or rebind (recycled).
                    crate::list_bind(dl.host_id, idx, inner);
                    // A rebound cell inherits its row's programmatic selection state.
                    paint_cell(dl, cell, idx as i32);
                    OH_ArkUI_NodeAdapterEvent_SetItem(ev, cell);
                }
                AdapterEv::NODE_ADAPTER_EVENT_ON_REMOVE_NODE_FROM_ADAPTER => {
                    // Return the cell to the pool for reuse; keep the inner Stack + Day's cache
                    // intact, but clear the row subtree's dayscript ids (day-core's recycle
                    // rule): a pooled cell must stop answering lookups until its next bind.
                    let removed = OH_ArkUI_NodeAdapterEvent_GetRemovedNode(ev);
                    if !removed.is_null() {
                        let inner = node::child_at(removed, 0);
                        if !inner.is_null() {
                            crate::list_recycle(dl.host_id, inner);
                        }
                        dl.pool.push(removed);
                    }
                }
                _ => {}
            }
        }
    });
}

/// Create the list's adapter, binding it to `host_id` so the row callbacks can find the
/// source. `row_h_vp` is the uniform row height in vp (0 = content-sized).
pub fn init(
    n: Handle,
    host_id: u64,
    row_h_vp: f64,
    selectable: bool,
    reorderable: bool,
    deletable: bool,
    delete_label: &str,
) {
    if n.is_null() || node::api().is_none() {
        return;
    }
    // SAFETY: a fresh adapter, owned by the list until `forget`.
    let adapter = unsafe { OH_ArkUI_NodeAdapter_Create() };
    let mut dl = Box::new(List {
        adapter,
        host_id,
        row_h: (row_h_vp * node::density()) as f32,
        selectable,
        generation: 0,
        pending_scroll: None,
        pool: Vec::new(),
        reorderable,
        node: n,
        rows: HashMap::new(),
        deletable,
        delete_label: delete_label.to_owned(),
        swipe_opts: HashMap::new(),
        swipe_cells: Vec::new(),
    });
    let user = ptr::from_mut(&mut *dl).cast::<c_void>();
    // SAFETY: the receiver's user data is the boxed list, stable until disposal.
    unsafe {
        OH_ArkUI_NodeAdapter_RegisterEventReceiver(adapter, user, Some(adapter_receiver));
    }
    LISTS.with(|l| l.borrow_mut().insert(n as usize, dl));
    node::set_object(n, Attr::NODE_LIST_NODE_ADAPTER, adapter.cast());
    node::register_event(n, Ev::NODE_LIST_ON_SCROLL_INDEX, host_id);
    if reorderable {
        // Accept Day-row drops anywhere over the list; the verdict comes from the app's guard
        // at drop time (`list_can_move`).
        // SAFETY: a live node.
        unsafe { OH_ArkUI_AllowNodeAllDropDataTypes(n) };
        node::register_event(n, Ev::NODE_ON_DROP, 0);
    }
}

/// A LIST node is being disposed: tear down its adapter binding. The adapter handle, the cells
/// (created in the adapter's add callback), and the swipe-action bookkeeping are all this
/// module's; day-core never owns them (a cell's inner Stack is only BORROWED through `adopt`),
/// so this is the one place they can be freed.
pub fn forget(n: Handle) {
    let Some(dl) = LISTS.with(|l| l.borrow_mut().remove(&(n as usize))) else {
        return;
    };
    DRAG.with(|d| {
        let mut d = d.borrow_mut();
        if d.is_some_and(|(list, _)| ptr::eq(list, &*dl)) {
            *d = None; // an in-flight drag cannot outlive its list
        }
    });
    // No adapter callbacks may fire into the half-dead binding while it is dismantled.
    // SAFETY: the adapter this list created.
    unsafe { OH_ArkUI_NodeAdapter_UnregisterEventReceiver(dl.adapter) };
    // `rows` holds every cell ever created (pooled and live): dispose each cell's inner Stack
    // and the cell itself. Day's own row-content nodes are disposed individually by day-core's
    // release queue; ArkUI's disposeNode is per-node, so the orders compose.
    for &cell in dl.rows.keys() {
        let cell = cell as Handle;
        let inner = node::child_at(cell, 0);
        if !inner.is_null() {
            node::dispose_raw(inner);
        }
        node::dispose_raw(cell);
    }
    // SAFETY: the swipe objects and adapter this list created.
    unsafe {
        for opt in dl.swipe_opts.values() {
            OH_ArkUI_ListItemSwipeActionOption_Dispose(*opt);
        }
        OH_ArkUI_NodeAdapter_Dispose(dl.adapter);
        for sc in &dl.swipe_cells {
            drop(Box::from_raw(*sc));
        }
    }
}

/// Repaint the live cells from Day's selection record; newly bound cells take theirs in the
/// adapter's add path.
pub fn paint_selection(n: Handle) {
    let dl = list_ptr(n);
    if dl.is_null() {
        return;
    }
    // SAFETY: a live list; no map borrow is held.
    let dl = unsafe { &*dl };
    for (&cell, &row) in &dl.rows {
        paint_cell(dl, cell as Handle, row);
    }
}

/// A tap landed on a selectable list cell: report the BOUND row as a selection. Cells carry no
/// Day node id, so the shared click receiver resolves them through the row maps here.
pub fn cell_click(n: Handle) -> bool {
    let hit = LISTS.with(|l| {
        l.borrow().values().find_map(|dl| {
            if !dl.selectable {
                return None;
            }
            dl.rows.get(&(n as usize)).map(|row| (dl.host_id, *row))
        })
    });
    let Some((host_id, row)) = hit else {
        return false;
    };
    use day_spec::bridge::BridgeKind as K;
    crate::on_event(host_id, K::SelectionChanged as i32, f64::from(row), "");
    crate::on_event(host_id, K::ListActivated as i32, f64::from(row), "");
    true
}

/// A row lifted (docs/list.md): the node is a pooled LIST_ITEM; find its list and
/// currently-bound row (the maps are small: one entry per live cell).
pub fn on_drag_start(ev: *mut ArkUI_NodeEvent) {
    // SAFETY: a live event.
    let n = unsafe { OH_ArkUI_NodeEvent_GetNodeHandle(ev) };
    let hit = LISTS.with(|l| {
        l.borrow_mut().values_mut().find_map(|dl| {
            let row = *dl.rows.get(&(n as usize))?;
            Some((ptr::from_mut(&mut **dl), row))
        })
    });
    if let Some(hit) = hit {
        DRAG.with(|d| *d.borrow_mut() = Some(hit));
    }
}

/// The slot under a drag event's touch point, in the list's row grid.
fn drag_slot(dl: &List, de: *mut ohos_sys::arkui::drag_and_drop::ArkUI_DragEvent) -> i32 {
    if dl.row_h <= 0.0 {
        return -1;
    }
    let mut pos = ArkUI_IntOffset { x: 0, y: 0 };
    // SAFETY: a live list node and drag event.
    let (y, n) = unsafe {
        OH_ArkUI_NodeUtils_GetLayoutPositionInWindow(dl.node, &mut pos);
        let mut y = OH_ArkUI_DragEvent_GetTouchPointYToWindow(de) - pos.y as f32;
        // Add the list's scroll offset (vp → px) so the slot is content-absolute.
        if let Some(oy) = node::get_f32(dl.node, Attr::NODE_SCROLL_OFFSET, 1) {
            y += oy * node::density() as f32;
        }
        (y, OH_ArkUI_NodeAdapter_GetTotalNodeCount(dl.adapter) as i32)
    };
    if n <= 0 {
        return -1;
    }
    ((y / dl.row_h) as i32).clamp(0, n - 1)
}

/// A drop over the list node: vet through the app's guard and commit through the sync seam; a
/// denied drop reports FAILED so ArkUI's native spring-back carries the affordance.
pub fn on_drop(ev: *mut ArkUI_NodeEvent) {
    // SAFETY: a live event.
    let (n, de) = unsafe {
        (
            OH_ArkUI_NodeEvent_GetNodeHandle(ev),
            OH_ArkUI_NodeEvent_GetDragEvent(ev.cast()),
        )
    };
    let dl = list_ptr(n);
    let drag = DRAG.with(|d| d.borrow_mut().take());
    let Some((from_list, from)) = drag else {
        return;
    };
    if de.is_null() || dl.is_null() || !ptr::eq(from_list, dl) || from < 0 {
        return;
    }
    // SAFETY: a live list; no map borrow is held.
    let dl = unsafe { &*dl };
    let slot = drag_slot(dl, de);
    let accepted = if slot < 0 {
        -1
    } else {
        crate::list_can_move(dl.host_id, from as u32, slot as u32)
    };
    // SAFETY: a live drag event.
    unsafe {
        if accepted >= 0 {
            if accepted != from {
                crate::list_move(dl.host_id, from as u32, accepted as u32);
                OH_ArkUI_NodeAdapter_ReloadAllItems(dl.adapter);
            }
            OH_ArkUI_DragEvent_SetDragResult(de, ArkUI_DragResult::SUCCESSFUL);
        } else {
            OH_ArkUI_DragEvent_SetDragResult(de, ArkUI_DragResult::FAILED);
        }
    }
}

/// Re-query the row count and re-bind the visible cells. ReloadAllItems, not just the count:
/// SetTotalNodeCount alone diffs by item id (= the index), so a list that shrank and grew back
/// keeps stale bindings, and after shrinking to EMPTY the adapter stopped firing ADD for the
/// regrown rows at all. The pool + day-core's cell cache make the re-adds cheap rebinds.
pub fn reload(n: Handle) {
    let dl = list_ptr(n);
    if dl.is_null() {
        return;
    }
    // SAFETY: a live list; the count callback may realize rows but holds no map borrow.
    let dl = unsafe { &mut *dl };
    dl.generation += 1;
    let count = crate::list_count(dl.host_id);
    // SAFETY: a live adapter.
    unsafe {
        OH_ArkUI_NodeAdapter_SetTotalNodeCount(dl.adapter, count);
        OH_ArkUI_NodeAdapter_ReloadAllItems(dl.adapter);
    }
    if count > 0
        && let Some(index) = dl.pending_scroll.take()
    {
        node::set_i32(
            n,
            Attr::NODE_LIST_SCROLL_TO_INDEX,
            (index as i32).min(count as i32 - 1),
        );
    }
}

/// Scroll the list so its last row is fully visible (docs/list.md, chat "stick to bottom").
pub fn scroll_to_end(n: Handle) {
    let dl = list_ptr(n);
    if dl.is_null() {
        return;
    }
    // SAFETY: a live adapter.
    let last = unsafe { OH_ArkUI_NodeAdapter_GetTotalNodeCount((*dl).adapter) } as i32 - 1;
    if last >= 0 {
        node::set_i32(n, Attr::NODE_LIST_SCROLL_TO_INDEX, last);
    }
}

/// Scroll the list so row `index` is visible, realizing it if needed.
pub fn scroll_to_row(n: Handle, index: u32) {
    let dl = list_ptr(n);
    if dl.is_null() {
        return;
    }
    // SAFETY: a live adapter.
    let count = unsafe { OH_ArkUI_NodeAdapter_GetTotalNodeCount((*dl).adapter) } as i32;
    if count > 0 {
        node::set_i32(
            n,
            Attr::NODE_LIST_SCROLL_TO_INDEX,
            (index as i32).min(count - 1),
        );
    } else {
        // No rows yet: the jump waits for the reload that brings them (`reload`).
        // SAFETY: a live list (checked non-null above).
        unsafe { (*dl).pending_scroll = Some(index) };
    }
}
