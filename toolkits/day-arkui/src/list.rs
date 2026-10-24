// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The recycling list (docs/list.md): an `ARKUI_NODE_LIST` driven by a `NodeAdapter`
//! (`ohos_sys::arkui` `native_node`).
//!
//! A cell is a LIST_ITEM wrapping an inner Stack that Day mounts the row subtree into. Cells
//! scrolled out of view are pushed to a REUSE POOL rather than disposed, so the inner Stack
//! pointer stays stable and day-core's cell cache rebinds it. Recycling parks the subtree's
//! element ids until its next bind. Row counts, binds and the reorder/delete seams come from
//! `lib.rs`'s list callbacks.
//!
//! A sectioned list (docs/list.md "Sections") is a Stack holding the LIST and, above it, one
//! extra cell Day binds the current section's header into: the pinned header. ArkUI's own sticky
//! headers belong to `ListItemGroup`s, which would split the one adapter-fed row space into a
//! group (and an adapter) per section; the overlay keeps the single adapter and reads the
//! visible range the list already reports (`NODE_LIST_ON_SCROLL_INDEX`).

// Node handles are opaque runtime tokens (see node.rs), never dereferenced here.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
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
    ArkUI_IntOffset, ArkUI_ListItemSwipeActionOption, ArkUI_ScrollAlignment, ArkUI_TextAlignment,
    OH_ArkUI_ListItemSwipeActionItem_Create, OH_ArkUI_ListItemSwipeActionItem_SetContent,
    OH_ArkUI_ListItemSwipeActionItem_SetOnActionWithUserData,
    OH_ArkUI_ListItemSwipeActionOption_Create, OH_ArkUI_ListItemSwipeActionOption_Dispose,
    OH_ArkUI_ListItemSwipeActionOption_SetEnd,
};

use crate::node::{self, Handle};

struct List {
    adapter: ArkUI_NodeAdapterHandle,
    host_id: u64,
    /// Day's handle for the list: the LIST itself, or for a sectioned list the Stack around the
    /// LIST and its pinned header (`node` is then the LIST inside it).
    host: Handle,
    /// px; 0 = content-sized (`RowHeight::Automatic`): each cell then takes the height Day
    /// laid its row at, set after every bind and on `ListPatch::RowSizeInvalidated`.
    row_h: f32,
    /// A content-sized list's live heights, px, by LIST_ITEM: what the drop slot hit-tests
    /// against, since the rows have no common pitch.
    heights: HashMap<usize, f32>,
    /// Rows report taps as selection and paint the programmatic set (docs/list.md).
    selectable: bool,
    /// Bumped per reload so every row gets a FRESH adapter item id (see GET_NODE_ID): the
    /// adapter diffs by id, and ids that stay put keep their old bindings; a reload after a
    /// reparent/insert above the tail left every untouched cell showing the wrong row.
    generation: u32,
    pool: Vec<Handle>,
    /// Drag-to-reorder (docs/list.md): whether rows drag, the LIST node (for geometry and the
    /// scroll attributes), and each live cell's currently-bound row (cells recycle, so the row
    /// is re-recorded on every bind).
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
    /// A sectioned list's header cells (docs/list.md "Sections"): built inert (no tap, drag or
    /// swipe), sized to their header, and pooled apart from the item cells so a recycled
    /// header cell always rebinds a header.
    header_cells: HashSet<usize>,
    header_pool: Vec<Handle>,
    /// A sectioned list's pinned header; `None` on every other list.
    pin: Option<Pin>,
}

/// The pinned copy of the current section's header (docs/list.md "Sections"): one Stack laid
/// over the top of the list, bound through the row source like any cell, rebound only when the
/// section at the top changes, and pushed up as the next header arrives under it.
struct Pin {
    /// The overlay Stack, Day's cell for the header copy. Default hit testing: while shown it
    /// takes the taps over the rows it covers, and hidden (`VISIBILITY_NONE`) it takes none.
    cell: Handle,
    /// The header row bound into `cell`; `None` before the first bind and after a reload.
    bound: Option<i32>,
    /// `cell`'s height, vp: the header's laid height.
    height: f64,
    /// The visible rows `NODE_LIST_ON_SCROLL_INDEX` last reported.
    first: i32,
    last: i32,
    /// `(first, header)`: the header row of the section holding `first`, kept so a scroll
    /// within a section scans nothing and one across sections scans only the rows it passed.
    section: Option<(i32, i32)>,
    /// The first header below `first` inside the visible range, the one that pushes the pin.
    next: Option<i32>,
    shown: bool,
}

/// User data for a cell's swipe action. The row is not captured here: cells recycle, so the
/// bound row is read from the list's rows when the action actually fires.
struct SwipeCell {
    list: *mut List,
    cell: Handle,
}

day_reactive::tls_slots! {
    list;
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
    // A header never shows selection (docs/list.md "Sections"); its ground is the header band.
    if !dl.selectable || dl.header_cells.contains(&(cell as usize)) {
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
                    let header = dl.pin.is_some() && crate::list_is_header(dl.host_id, idx);
                    let cell = if header {
                        match dl.header_pool.pop() {
                            Some(cell) => cell,
                            None => new_header_cell(dl),
                        }
                    } else {
                        match dl.pool.pop() {
                            Some(cell) => cell,
                            None => new_cell(dl),
                        }
                    };
                    // Unconditional: the swipe action resolves its row through this map too,
                    // and a guarded row is filtered at delete time by the seam's own
                    // `can_delete`.
                    dl.rows.insert(cell as usize, idx as i32);
                    let inner = node::child_at(cell, 0);
                    // Build (fresh) or rebind (recycled).
                    crate::list_bind(dl.host_id, idx, inner);
                    if dl.row_h <= 0.0 || header {
                        // Content-sized: the height Day just laid the row at. Left unset, the
                        // item only wraps its Stack, whose children are placed, not measured.
                        // A header sizes to its content even in a uniform list.
                        let height = crate::list_cell_height(dl.host_id, inner);
                        size_row(dl, cell, inner, height);
                    }
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
                        if dl.header_cells.contains(&(removed as usize)) {
                            dl.header_pool.push(removed);
                        } else {
                            dl.pool.push(removed);
                        }
                    }
                }
                _ => {}
            }
        }
    });
}

/// A fresh item cell: a LIST_ITEM around the Stack Day mounts the row in.
fn new_cell(dl: &mut List) -> Handle {
    let cell = node::create(node::LIST_ITEM);
    let inner = node::create(node::STACK);
    // Day's adopted anchor owns its contents, not its native frame. Fill the list's width
    // explicitly so the selection/touch target covers the row instead of hugging its absolutely
    // placed label.
    node::set_f32(cell, Attr::NODE_WIDTH_PERCENT, 1.0);
    node::set_f32(inner, Attr::NODE_WIDTH_PERCENT, 1.0);
    if dl.row_h > 0.0 {
        let h = dl.row_h / node::density() as f32;
        node::set_f32(cell, Attr::NODE_HEIGHT, h);
        node::set_f32(inner, Attr::NODE_HEIGHT, h);
    }
    node::add_child(cell, inner);
    if dl.reorderable {
        // Long-press lifts the row with ArkUI's own drag preview; the drop lands on the LIST
        // node's NODE_ON_DROP (registered in `init`).
        // SAFETY: a live node.
        unsafe { OH_ArkUI_SetNodeDraggable(cell, true) };
        node::register_event(cell, Ev::NODE_ON_DRAG_START, 0);
    }
    if dl.selectable {
        // A tap selects the bound row (docs/list.md); the shared receiver resolves the cell
        // through `cell_click`.
        node::register_event(cell, Ev::NODE_ON_CLICK, 0);
    }
    attach_swipe(dl, cell);
    cell
}

/// A fresh header cell (docs/list.md "Sections"): the item cell's shape with no height of its
/// own (the header's laid height is set after each bind), the header band's opaque ground, and
/// nothing to act on: no tap, no drag, no swipe, and no focus stop for keyboard navigation.
fn new_header_cell(dl: &mut List) -> Handle {
    let cell = node::create(node::LIST_ITEM);
    let inner = node::create(node::STACK);
    node::set_f32(cell, Attr::NODE_WIDTH_PERCENT, 1.0);
    node::set_f32(inner, Attr::NODE_WIDTH_PERCENT, 1.0);
    node::add_child(cell, inner);
    node::set_i32(cell, Attr::NODE_FOCUSABLE, 0);
    crate::list_header_ground(dl.host, cell);
    dl.header_cells.insert(cell as usize);
    cell
}

/// Give a content-sized list's cell (the LIST_ITEM and the Stack the row mounts in) the row's
/// height in vp. A zero height (a bind that skipped under a day-core borrow) leaves it as is.
fn size_row(dl: &mut List, cell: Handle, inner: Handle, height_vp: f64) {
    if height_vp <= 0.0 || cell.is_null() || inner.is_null() {
        return;
    }
    node::set_f32(cell, Attr::NODE_HEIGHT, height_vp as f32);
    node::set_f32(inner, Attr::NODE_HEIGHT, height_vp as f32);
    dl.heights
        .insert(cell as usize, (height_vp * node::density()) as f32);
}

/// `ListPatch::RowSizeInvalidated`: the row mounted in `inner` (a cell's Stack) was re-laid at
/// `height_vp` after it was first sized. Called posted, out of the day-core borrow; a uniform
/// list's rows keep their declared height.
pub fn resize_row(n: Handle, inner: Handle, height_vp: f64) {
    let dl = list_ptr(n);
    if dl.is_null() || inner.is_null() {
        return;
    }
    // SAFETY: a live list; no map borrow is held.
    let dl = unsafe { &mut *dl };
    if let Some(pin) = dl.pin.as_mut()
        && pin.cell == inner
    {
        if height_vp > 0.0 {
            pin.height = height_vp;
            node::set_f32(pin.cell, Attr::NODE_HEIGHT, height_vp as f32);
            place_pin(dl);
        }
        return;
    }
    let cell = node::parent(inner);
    // A uniform list's item rows keep their declared height; its headers size to content.
    if dl.row_h > 0.0 && !dl.header_cells.contains(&(cell as usize)) {
        return;
    }
    // Only a cell this list still binds: a pooled one gets its height on its next bind.
    if dl.rows.contains_key(&(cell as usize))
        && !dl.pool.contains(&cell)
        && !dl.header_pool.contains(&cell)
    {
        size_row(dl, cell, inner, height_vp);
    }
}

/// Create the list's adapter, binding it to `host_id` so the row callbacks can find the
/// source. `row_h_vp` is the uniform row height in vp (0 = content-sized). `n` is Day's handle:
/// a LIST, or for a sectioned list (`p.sections`) a Stack this fills with the LIST and the
/// pinned header (docs/list.md "Sections").
pub fn init(n: Handle, host_id: u64, row_h_vp: f64, p: &day_spec::props::ListProps) {
    if n.is_null() || node::api().is_none() {
        return;
    }
    let (list, pin) = if p.sections {
        let list = node::create(node::LIST);
        node::set_f32(list, Attr::NODE_WIDTH_PERCENT, 1.0);
        node::set_f32(list, Attr::NODE_HEIGHT_PERCENT, 1.0);
        node::add_child(n, list);
        // Added after the LIST, so it draws (and hit-tests) above the rows; placed at the top
        // edge by position, hidden until a header has scrolled off.
        let cell = node::create(node::STACK);
        node::set_f32(cell, Attr::NODE_WIDTH_PERCENT, 1.0);
        node::set_values(
            cell,
            Attr::NODE_POSITION,
            &[node::f32v(0.0), node::f32v(0.0)],
        );
        node::set_visibility(cell, false);
        crate::list_header_ground(n, cell);
        node::add_child(n, cell);
        // The pin slides up past the list's top edge as the next header arrives.
        node::set_i32(n, Attr::NODE_CLIP, 1);
        let pin = Pin {
            cell,
            bound: None,
            height: 0.0,
            first: 0,
            last: 0,
            section: None,
            next: None,
            shown: false,
        };
        (list, Some(pin))
    } else {
        (n, None)
    };
    // SAFETY: a fresh adapter, owned by the list until `forget`.
    let adapter = unsafe { OH_ArkUI_NodeAdapter_Create() };
    let sectioned = pin.is_some();
    let mut dl = Box::new(List {
        adapter,
        host_id,
        host: n,
        row_h: (row_h_vp * node::density()) as f32,
        heights: HashMap::new(),
        selectable: p.selectable,
        generation: 0,
        pending_scroll: None,
        pool: Vec::new(),
        reorderable: p.reorderable,
        node: list,
        rows: HashMap::new(),
        deletable: p.deletable,
        delete_label: p.delete_label.clone(),
        swipe_opts: HashMap::new(),
        swipe_cells: Vec::new(),
        header_cells: HashSet::new(),
        header_pool: Vec::new(),
        pin,
    });
    let user = ptr::from_mut(&mut *dl).cast::<c_void>();
    // SAFETY: the receiver's user data is the boxed list, stable until disposal.
    unsafe {
        OH_ArkUI_NodeAdapter_RegisterEventReceiver(adapter, user, Some(adapter_receiver));
    }
    LISTS.with(|l| l.borrow_mut().insert(n as usize, dl));
    node::set_object(list, Attr::NODE_LIST_NODE_ADAPTER, adapter.cast());
    node::register_event(list, Ev::NODE_LIST_ON_SCROLL_INDEX, host_id);
    if sectioned {
        // Every scrolled frame moves the pin (the index event fires only as rows enter/leave).
        node::register_event(list, Ev::NODE_LIST_ON_DID_SCROLL, host_id);
    }
    if p.reorderable {
        // Accept Day-row drops anywhere over the list; the verdict comes from the app's guard
        // at drop time (`list_can_move`).
        // SAFETY: a live node.
        unsafe { OH_ArkUI_AllowNodeAllDropDataTypes(list) };
        node::register_event(list, Ev::NODE_ON_DROP, 0);
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
    // A sectioned list's Stack holds this module's LIST and pin (the pin's header content is
    // Day's, handed back through `ListSource::release` in lib.rs's `release`).
    if dl.node != n {
        if let Some(pin) = &dl.pin {
            node::remove_child(n, pin.cell);
            node::dispose_raw(pin.cell);
        }
        node::remove_child(n, dl.node);
        node::dispose_raw(dl.node);
    }
}

/// A sectioned list's pinned-header cell, for `ListSource::release` when the list goes.
pub fn pin_cell(n: Handle) -> Option<Handle> {
    let dl = list_ptr(n);
    if dl.is_null() {
        return None;
    }
    // SAFETY: a live list; no map borrow is held.
    unsafe { (*dl).pin.as_ref().map(|pin| pin.cell) }
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
            let header = dl.header_cells.contains(&(n as usize));
            dl.rows
                .get(&(n as usize))
                .map(|row| (dl.host_id, *row, header))
        })
    });
    let Some((host_id, row, header)) = hit else {
        return false;
    };
    // Header cells register no tap; should one arrive anyway it is inert (docs/list.md
    // "Sections"), and still consumed so it is not reported as a press.
    if header {
        return true;
    }
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

/// The slot under a drag event's touch point: in the list's row grid, or for a content-sized
/// list the live row under it.
fn drag_slot(dl: &List, de: *mut ohos_sys::arkui::drag_and_drop::ArkUI_DragEvent) -> i32 {
    if dl.row_h <= 0.0 {
        return measured_slot(dl, de);
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

/// A content-sized list's drop slot: the bound row whose cell spans the touch point, else the
/// nearest one above or below it (rows have no common pitch to divide by).
fn measured_slot(dl: &List, de: *mut ohos_sys::arkui::drag_and_drop::ArkUI_DragEvent) -> i32 {
    // SAFETY: a live drag event and live adapter.
    let (touch, n) = unsafe {
        (
            OH_ArkUI_DragEvent_GetTouchPointYToWindow(de),
            OH_ArkUI_NodeAdapter_GetTotalNodeCount(dl.adapter) as i32,
        )
    };
    if n <= 0 {
        return -1;
    }
    // (row, top, bottom) of every bound cell, in window px.
    let spans: Vec<(i32, f32, f32)> = dl
        .rows
        .iter()
        .filter(|(cell, _)| !dl.pool.contains(&(**cell as Handle)))
        .filter_map(|(cell, row)| {
            let height = *dl.heights.get(cell)?;
            let mut pos = ArkUI_IntOffset { x: 0, y: 0 };
            // SAFETY: a live cell node.
            unsafe { OH_ArkUI_NodeUtils_GetLayoutPositionInWindow(*cell as Handle, &mut pos) };
            Some((*row, pos.y as f32, pos.y as f32 + height))
        })
        .collect();
    let hit = spans
        .iter()
        .find(|(_, top, bottom)| touch >= *top && touch < *bottom)
        .or_else(|| {
            spans
                .iter()
                .filter(|(_, top, _)| touch < *top)
                .min_by(|a, b| a.1.total_cmp(&b.1))
        })
        .or_else(|| spans.iter().max_by(|a, b| a.2.total_cmp(&b.2)));
    hit.map_or(-1, |(row, ..)| (*row).clamp(0, n - 1))
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
        reveal_row(dl.node, (index as i32).min(count as i32 - 1));
    }
    // The rows changed under the pin: its section, and the header bound in it, are re-read.
    // A shown pin rebinds now; the cells just re-added are not laid out yet, so a hidden one
    // waits for the list's next index or scroll event to judge the geometry.
    if let Some(pin) = dl.pin.as_mut() {
        pin.bound = None;
        pin.section = None;
        let shown = pin.shown;
        locate_pin(dl);
        if shown {
            place_pin(dl);
        }
    }
}

/// Scroll the list so its last row is fully visible (docs/list.md, chat "stick to bottom").
pub fn scroll_to_end(n: Handle) {
    let dl = list_ptr(n);
    if dl.is_null() {
        return;
    }
    // SAFETY: a live list and its adapter.
    let (list, last) = unsafe {
        (
            (*dl).node,
            OH_ArkUI_NodeAdapter_GetTotalNodeCount((*dl).adapter) as i32 - 1,
        )
    };
    if last >= 0 {
        node::set_values(
            list,
            Attr::NODE_LIST_SCROLL_TO_INDEX,
            &[
                node::i32v(last),
                node::i32v(0),
                node::i32v(ArkUI_ScrollAlignment::ARKUI_SCROLL_ALIGNMENT_END.0 as i32),
            ],
        );
    }
}

/// Scroll the list so row `index` is visible, realizing it if needed.
pub fn scroll_to_row(n: Handle, index: u32) {
    let dl = list_ptr(n);
    if dl.is_null() {
        return;
    }
    // SAFETY: a live list and its adapter.
    let (list, count) = unsafe {
        (
            (*dl).node,
            OH_ArkUI_NodeAdapter_GetTotalNodeCount((*dl).adapter) as i32,
        )
    };
    if count > 0 {
        reveal_row(list, (index as i32).min(count - 1));
    } else {
        // No rows yet: the jump waits for the reload that brings them (`reload`).
        // SAFETY: a live list (checked non-null above).
        unsafe { (*dl).pending_scroll = Some(index) };
    }
}

/// Reveal with the least movement, leaving an already-visible row in place. ArkUI's
/// default START alignment moves even visible rows to the top of the viewport. `list` is the
/// LIST node itself.
fn reveal_row(list: Handle, index: i32) {
    node::set_values(
        list,
        Attr::NODE_LIST_SCROLL_TO_INDEX,
        &[
            node::i32v(index),
            node::i32v(0),
            node::i32v(ArkUI_ScrollAlignment::ARKUI_SCROLL_ALIGNMENT_AUTO.0 as i32),
        ],
    );
}

/// The sectioned list whose LIST node is `list` (an event's node), as a raw pointer.
fn sectioned_by_node(list: Handle) -> *mut List {
    LISTS.with(|l| {
        l.borrow_mut()
            .values_mut()
            .find(|dl| dl.node == list && dl.pin.is_some())
            .map_or(ptr::null_mut(), |b| ptr::from_mut(&mut **b))
    })
}

/// `NODE_LIST_ON_SCROLL_INDEX` on a LIST node: a sectioned list re-reads which section is at
/// its top and which header follows it, then moves its pin (docs/list.md "Sections").
pub fn on_scroll_index(list: Handle, first: i32, last: i32) {
    let dl = sectioned_by_node(list);
    if dl.is_null() {
        return;
    }
    // SAFETY: a live list; no map borrow is held.
    let dl = unsafe { &mut *dl };
    if let Some(pin) = dl.pin.as_mut() {
        pin.first = first.max(0);
        pin.last = last.max(pin.first);
    }
    locate_pin(dl);
    place_pin(dl);
}

/// `NODE_LIST_ON_DID_SCROLL`: every scrolled frame of a sectioned list moves its pin.
pub fn on_did_scroll(list: Handle) {
    let dl = sectioned_by_node(list);
    if dl.is_null() {
        return;
    }
    // SAFETY: a live list; no map borrow is held.
    place_pin(unsafe { &mut *dl });
}

/// Find the header of the section holding the first visible row, and the next header inside
/// the visible range. Row 0 is always a header; a scroll within the section scans nothing.
fn locate_pin(dl: &mut List) {
    let host_id = dl.host_id;
    let Some(pin) = dl.pin.as_mut() else {
        return;
    };
    let count = crate::list_count(host_id) as i32;
    if count <= 0 {
        pin.section = None;
        pin.next = None;
        return;
    }
    let first = pin.first.min(count - 1);
    let last = pin.last.clamp(first, count - 1);
    let is_header = |row: &i32| crate::list_is_header(host_id, *row as u32);
    // `(from, h)`: no header lies in h+1..=from, so any first in h..=from is still h's.
    let header = match pin.section {
        Some((from, h)) if (h..=from).contains(&first) => h,
        Some((from, h)) if first > from => ((from + 1)..=first).rev().find(is_header).unwrap_or(h),
        _ => (0..=first).rev().find(is_header).unwrap_or(0),
    };
    pin.section = Some((first, header));
    pin.next = ((first + 1)..=last).find(is_header);
}

/// The window y (px) of the live cell bound to `row`, if one is on the list now.
fn live_top(dl: &List, row: i32) -> Option<f32> {
    let (&cell, _) = dl.rows.iter().find(|&(&cell, &r)| {
        r == row
            && !dl.pool.contains(&(cell as Handle))
            && !dl.header_pool.contains(&(cell as Handle))
    })?;
    Some(window_y(cell as Handle))
}

fn window_y(n: Handle) -> f32 {
    let mut pos = ArkUI_IntOffset { x: 0, y: 0 };
    // SAFETY: a live node.
    unsafe { OH_ArkUI_NodeUtils_GetLayoutPositionInWindow(n, &mut pos) };
    pos.y as f32
}

/// Bind, place or hide the pin from the section `locate_pin` found: hidden while that
/// section's own header row shows in full at the top, otherwise a copy of it at the top edge,
/// slid up so its bottom never overlaps the next header.
fn place_pin(dl: &mut List) {
    let host_id = dl.host_id;
    let Some(pin) = dl.pin.as_ref() else {
        return;
    };
    let (cell, shown, section, next, bound) =
        (pin.cell, pin.shown, pin.section, pin.next, pin.bound);
    let hide = |dl: &mut List| {
        if shown {
            node::set_visibility(cell, false);
        }
        if let Some(pin) = dl.pin.as_mut() {
            pin.shown = false;
        }
    };
    let Some((_, header)) = section else {
        return hide(dl);
    };
    if bound != Some(header) {
        // A rebind only when the section at the top changes. A bind skipped inside a day-core
        // borrow lays nothing (height 0): stay hidden and retry on the next event.
        crate::list_bind_pinned(host_id, header as u32, cell);
        let height = crate::list_cell_height(host_id, cell);
        if height <= 0.0 {
            return hide(dl);
        }
        node::set_f32(cell, Attr::NODE_HEIGHT, height as f32);
        if let Some(pin) = dl.pin.as_mut() {
            pin.bound = Some(header);
            pin.height = height;
        }
    }
    let list_top = window_y(dl.node);
    // The real header row fully on screen at the top (or pulled below it in an overscroll).
    if live_top(dl, header).is_some_and(|top| top >= list_top) {
        return hide(dl);
    }
    let density = node::density() as f32;
    let height = dl.pin.as_ref().map_or(0.0, |pin| pin.height as f32) * density;
    let y = next
        .and_then(|row| live_top(dl, row))
        .map_or(0.0, |top| (top - list_top - height).min(0.0));
    if y <= -height {
        return hide(dl);
    }
    node::set_values(
        cell,
        Attr::NODE_POSITION,
        &[node::f32v(0.0), node::f32v(y / density)],
    );
    if !shown {
        node::set_visibility(cell, true);
    }
    if let Some(pin) = dl.pin.as_mut() {
        pin.shown = true;
    }
}
