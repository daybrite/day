// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-qt — the Qt 6 Widgets backend (linux-qt / macos-qt / windows-qt; DESIGN.md §9), over
//! the day-qt-sys C++ shim. `Handle = QtHandle(*mut QWidget)`; absolute geometry; toggle is a
//! QCheckBox (Qt Widgets has no native switch — an explicitly documented divergence).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::rc::Rc;

use day_qt_sys as ffi;
use linkme::distributed_slice;

use day_spec::props::*;
use day_spec::{
    A11yProps, AnimSpec, Builtin, Cap, Curve, DrawOp, Event, EventSink, Font, NodeId, PieceKind,
    Platform, Proposal, Rect, Registry, Renderer, Size, Support, Toolkit, Transform, WindowOptions,
    ffi_guard, kinds, props_of, sidetable::SideTable,
};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct QtHandle(pub *mut c_void);

/// Lower an `AnimSpec` to `(duration_ms, curve_code)` for the Qt shim (§8.4). `None` ⇒ `(0, 0)`
/// (instant). Curve codes match the shim's `day_qt_easing`: 0 linear, 1 easeIn, 2 easeOut,
/// 3 easeInOut, 4 spring (OutBack overshoot).
fn qt_anim_args(anim: Option<&AnimSpec>) -> (c_int, c_int) {
    match anim {
        None => (0, 0),
        Some(a) => (
            a.duration_ms as c_int,
            match a.curve {
                Curve::Linear => 0,
                Curve::EaseIn => 1,
                Curve::EaseOut => 2,
                Curve::EaseInOut => 3,
                Curve::Spring { .. } => 4,
            },
        ),
    }
}

// Built-in leaf pieces split into modules (moved in from their satellite crates 2026-07).
mod picker;
mod textarea;
mod toolbar;

pub type Handle = QtHandle;

pub mod ext;
pub use ext::*;

/// The day-core event sink (node-id keyed).
type Sink = Rc<dyn Fn(NodeId, Event)>;

day_core::tls_group! {
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
    /// Slider f64 range, keyed by node id (event callbacks) AND widget ptr (patch application).
    /// SideTables: `release`'s sweep retires the widget-ptr entry with the widget, so a later
    /// widget recycling the address can't read the dead slider's range. (The node-id side is
    /// generation-keyed by day-core and never reused, so it cannot mis-hit.)
    static RANGES: SideTable<(f64, f64)> = SideTable::new();
    static RANGES_BY_PTR: SideTable<(f64, f64)> = SideTable::new();
    /// (widget_ptr, kind) pairs already wired, so enable_gesture is idempotent.
    static GESTURES: RefCell<std::collections::HashSet<(usize, day_spec::GestureKind)>> =
        RefCell::new(std::collections::HashSet::new());

    /// Summon-time context-menu providers by node (docs/menus.md "Dynamic context menus").
    static CTX_MENU_FNS: RefCell<HashMap<u64, day_spec::ContextMenuFn>> =
        RefCell::new(HashMap::new());

    static LIST_STATE: RefCell<HashMap<usize, ListEntry>> = RefCell::new(HashMap::new());
    /// List NODE id → host key, so the cell-click callback (which carries the node) can
    /// find its entry.
    static LIST_BY_NODE: RefCell<HashMap<u64, usize>> = RefCell::new(HashMap::new());

    static NAV_STATE: RefCell<HashMap<usize, NavState>> = RefCell::new(HashMap::new());
    static NAV_PAGE_IDS: RefCell<HashMap<usize, NodeId>> = RefCell::new(HashMap::new());
    /// Each nav page's pane, recorded at realize because `insert` sees only handles
    /// (docs/size-classes.md).
    static PAGE_PANE: RefCell<HashMap<usize, day_spec::props::Pane>> = RefCell::new(HashMap::new());

    static INSPECTOR_STATE: RefCell<HashMap<usize, InspectorState>> = RefCell::new(HashMap::new());
    /// INSPECTOR_PANE widget → `(its NodeId, is-panel)`, recorded at realize because `insert`
    /// sees only handles — the PAGE_PANE pattern.
    static INSPECTOR_PANE_IDS: RefCell<HashMap<usize, (NodeId, bool)>> =
        RefCell::new(HashMap::new());

    /// Each label's own resolved point size, so a RELATIVE-size run can be written as the
    /// absolute `pt` Qt's rich-text CSS understands (a percentage is silently ignored there).
    ///
    /// A side table because the patch that carries runs does not carry the font: `LabelPatch`
    /// splits them, and by the time `Runs` arrives the `Font` it should scale against is
    /// whatever the last `Font` patch set. Swept with the widget in `release`.
    static LABEL_PT: SideTable<f64> = SideTable::new();

    static NAV_SUITES: RefCell<HashMap<usize, NavSuite>> = RefCell::new(HashMap::new());
    /// Suite page widgets, so a released one drops out of its suite. Unlike the `tabs()` piece's
    /// pages these ARE framed by Day: NavLayout computes exactly the content rect the tab widget
    /// would give them, and letting Qt's stacked layout do it instead left them at zero size.
    static NAV_SUITE_PAGES: RefCell<std::collections::HashSet<usize>> =
        RefCell::new(std::collections::HashSet::new());
    /// A realized NAV_MENU's rows by widget: `(node, titles, icon names)`, kept until the menu is
    /// inserted and a suite above it can be found and handed them.
    static NAV_SUITE_ROWS: RefCell<HashMap<usize, NavRow>> = RefCell::new(HashMap::new());

    /// The last reported window content size (seeds cover frames at Present).
    static LAST_WINDOW_SIZE: std::cell::Cell<Size> =
        const { std::cell::Cell::new(Size::new(0.0, 0.0)) };
    /// Presented emulated covers: (widget, NodeId).
    static COVERS: RefCell<Vec<(*mut c_void, NodeId)>> = const { RefCell::new(Vec::new()) };
    /// Cover widget → NodeId (set at realize).
    static COVER_IDS: RefCell<HashMap<usize, NodeId>> = RefCell::new(HashMap::new());

    /// NAV_MENU widget → row count (for measure).
    static NAV_MENU_ROWS: RefCell<HashMap<usize, usize>> = RefCell::new(HashMap::new());

}

pub fn emit(id: NodeId, ev: Event) {
    let sink = SINK.with(|s| s.borrow().clone());
    if let Some(sink) = sink {
        sink(id, ev);
    }
}

/// Deliver an event to day-core on the next event-loop turn — a genuine "safe point" (§8.3) —
/// instead of inline. A native list selection rebuilds the sidebar detail *synchronously*
/// (dispose old widgets, create new ones); doing that inside the QListWidget's own key/click
/// dispatch reparents widgets mid-event and reads freed memory (`QWidget::setParent` SIGSEGV).
/// Only `Send` data (id + event) is captured; the thread-local sink runs on the main thread.
fn emit_deferred(id: NodeId, ev: Event) {
    let boxed: Box<dyn FnOnce() + Send> = Box::new(move || emit(id, ev));
    let data = Box::into_raw(Box::new(boxed)) as *mut c_void;
    unsafe { ffi::day_qt_post(run_posted, data) };
}

/// Pack a color as `0xAARRGGBB`, the form the shim reads.
fn argb(c: day_spec::Color) -> u32 {
    let f = |v: f64| (v.clamp(0.0, 1.0) * 255.0) as u32;
    (f(c.a) << 24) | (f(c.r) << 16) | (f(c.g) << 8) | f(c.b)
}

/// Hand a [`day_spec::props::ButtonStyleSpec`] to the shim, which styles the QPushButton in
/// place. The widget stays a QPushButton, so focus, Space/Enter activation and the a11y role
/// are unaffected whatever the style is.
fn apply_button_style(w: *mut c_void, style: day_spec::props::ButtonStyleSpec) {
    use day_spec::props::ButtonStyleSpec as S;
    let (kind, fill) = match style {
        S::Automatic => (0, day_spec::Color::CLEAR),
        S::Bordered => (1, day_spec::Color::CLEAR),
        S::Prominent => (2, day_spec::Color::CLEAR),
        S::Tinted(c) => (3, c),
    };
    // SAFETY: `w` is a live QPushButton created by `day_qt_button_new`; the shim only reads the
    // packed colors.
    unsafe { ffi::day_qt_button_set_style(w, kind, argb(fill), argb(S::on_tint(fill))) };
}

pub(crate) fn cstr(s: &str) -> CString {
    // An interior NUL must not blank the whole string — a label, a menu item, a window title
    // would silently vanish. Strip the NULs and keep the visible text.
    CString::new(s).unwrap_or_else(|_| CString::new(s.replace('\0', "")).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Recycling list (docs/list.md, §10). Qt's item views are paint-based, so a widget list can't
// recycle natively — Day emulates it (DP-19): a QScrollArea whose content holds one absolutely
// positioned cell widget per row, each filled through the same `bind_row` seam. Cells are reused
// across reloads (append-only), so day-core's cell map never dangles.
// ---------------------------------------------------------------------------

/// Rows built beyond each edge of the viewport, so a flick has something to show before the
/// scroll callback lands. Cheap: a row costs one populate pass, and this is a handful of them.
const LIST_OVERSCAN: usize = 8;

struct ListEntry {
    host: *mut c_void,
    row_height: f64,
    source: Rc<RefCell<Option<day_spec::ListSource>>>,
    /// One slot per row, NULL until that row has been shown: cell index == row index, so a
    /// slot's identity never changes and a realized cell stays that row's for good.
    cells: Vec<*mut c_void>,
    /// Which rows' content is currently built and current. Cleared wholesale when the source
    /// changes under the cells; individual rows go true as they are bound.
    bound: Vec<bool>,
    /// The height day's layout last framed the host at, so the first fill — which runs before
    /// Qt has laid the scroll area out and its viewport still reports nothing — has a window.
    frame_height: c_int,
    /// A scroll-driven fill is already posted: one flick emits a stream of valueChanged, and
    /// they would each post a pass that does the same work.
    fill_pending: bool,
    /// Last host width a populate ran at — so `set_frame` only repopulates on a real width change
    /// (a populate's own child `set_frame`s must not schedule another, or it loops forever).
    last_width: c_int,
    /// Selection (docs/list.md): whether rows select at all, whether several may be selected,
    /// the currently selected rows, and the last plainly-clicked row (the shift-range anchor).
    node: u64,
    selectable: bool,
    multi: bool,
    reorderable: bool,
    selected: std::collections::BTreeSet<usize>,
    anchor: Option<usize>,
}

/// Repaint every cell's selected treatment from the entry's selection set.
fn list_paint_selection(entry: &ListEntry) {
    for (i, &cell) in entry.cells.iter().enumerate() {
        // Unrealized rows have no cell to paint; they pick the treatment up when they are built
        // (this runs at the end of every fill, so a row scrolled into a selection lands painted).
        if cell.is_null() {
            continue;
        }
        unsafe { ffi::day_qt_cell_set_selected(cell, entry.selected.contains(&i) as c_int) };
    }
}

/// A press on an emulated list cell (docs/list.md). Owns the selection semantics: plain click
/// replaces the selection, ctrl/cmd toggles the row, shift extends from the anchor (multi-select
/// only) — then repaints and reports (`SelectionSet` in multi mode, `SelectionChanged` single).
extern "C" fn on_list_row_click(node: u64, row: c_int, mods: c_int) {
    // Every extern "C" trampoline in this backend runs its body through `ffi_guard::contain`:
    // a panic unwinding into the C++ shim frame is undefined behavior (day-spec's ffi_guard).
    ffi_guard::contain((), || {
        let row = row.max(0) as usize;
        let Some(host_key) = LIST_BY_NODE.with(|m| m.borrow().get(&node).copied()) else {
            return;
        };
        let emit_ev = LIST_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let st = m.get_mut(&host_key)?;
            if !st.selectable {
                return None;
            }
            let (ctrl, shift) = (mods & 1 != 0, mods & 2 != 0);
            if st.multi && ctrl {
                if !st.selected.remove(&row) {
                    st.selected.insert(row);
                }
                st.anchor = Some(row);
            } else if st.multi && shift {
                let a = st.anchor.unwrap_or(row);
                st.selected = (a.min(row)..=a.max(row)).collect();
            } else {
                st.selected = std::iter::once(row).collect();
                st.anchor = Some(row);
            }
            list_paint_selection(st);
            Some(if st.multi {
                Event::SelectionSet(st.selected.iter().map(|r| *r as i64).collect())
            } else {
                Event::SelectionChanged(row as i64)
            })
        });
        if let Some(ev) = emit_ev {
            emit(NodeId(node), ev);
        }
    });
}

/// The reorder guard's verdict for a hovered drop (docs/list.md), called synchronously from the
/// shim's drag-move filter: the accepted target index, or -1. The reorder Rc is cloned out of
/// `LIST_STATE` before the app's guard runs, so no borrow is held.
extern "C" fn on_list_can_move(node: u64, from: c_int, to: c_int) -> c_int {
    // Runs the app's own guard closure — contained, with "reject the drop" as the default.
    ffi_guard::contain(-1, || {
        let Some(host_key) = LIST_BY_NODE.with(|m| m.borrow().get(&node).copied()) else {
            return -1;
        };
        let r = LIST_STATE.with(|m| {
            m.borrow().get(&host_key).and_then(|st| {
                let s = st.source.borrow().clone()?;
                Some(((s.len)(), s.reorder))
            })
        });
        let Some((len, Some(r))) = r else { return -1 };
        let (from, to) = (from.max(0) as usize, to.max(0) as usize);
        if from >= len {
            return -1;
        }
        let to = to.min(len - 1);
        ((r.can_move)(from, to) as c_int).min(len as c_int - 1)
    })
}

/// Commit a drop the shim's filter accepted: rotate Day's snapshot through the sync seam
/// (deferring the app callback) and re-bind the cells in the new order.
extern "C" fn on_list_move(node: u64, from: c_int, to: c_int) {
    ffi_guard::contain((), || {
        let Some(host_key) = LIST_BY_NODE.with(|m| m.borrow().get(&node).copied()) else {
            return;
        };
        let r = LIST_STATE.with(|m| {
            m.borrow()
                .get(&host_key)
                .and_then(|st| st.source.borrow().clone()?.reorder)
        });
        let Some(r) = r else { return };
        let (from, to) = (from.max(0) as usize, to.max(0) as usize);
        if from != to {
            (r.move_row)(from, to);
            schedule_list_populate(host_key);
        }
    });
}

/// Populate/refresh a list's cells on the next event-loop turn — NOT inline: a reload runs inside
/// a `with_tree` borrow, and `bind_row` re-enters `with_tree`, which would panic.
fn schedule_list_populate(host_key: usize) {
    let boxed: Box<dyn FnOnce() + Send> = Box::new(move || list_populate(host_key));
    let data = Box::into_raw(Box::new(boxed)) as *mut c_void;
    unsafe { ffi::day_qt_post(run_posted, data) };
}

/// The list scrolled: build whatever rows just came into view. Deferred and coalesced — the
/// signal arrives from inside Qt's own scroll handling, where `bind_row`'s re-entry into
/// `with_tree` cannot run.
extern "C" fn on_list_scrolled(node: u64) {
    ffi_guard::contain((), || {
        if let Some(host_key) = LIST_BY_NODE.with(|m| m.borrow().get(&node).copied()) {
            schedule_list_fill(host_key);
        }
    });
}

/// Fill the window on the next loop turn, at most once per turn however many times it is asked.
fn schedule_list_fill(host_key: usize) {
    let post = LIST_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let Some(st) = m.get_mut(&host_key) else {
            return false;
        };
        let first = !st.fill_pending;
        st.fill_pending = true;
        first
    });
    if !post {
        return;
    }
    let boxed: Box<dyn FnOnce() + Send> = Box::new(move || {
        LIST_STATE.with(|m| {
            if let Some(st) = m.borrow_mut().get_mut(&host_key) {
                st.fill_pending = false;
            }
        });
        list_fill_window(host_key);
    });
    let data = Box::into_raw(Box::new(boxed)) as *mut c_void;
    unsafe { ffi::day_qt_post(run_posted, data) };
}

/// Scroll the (emulated) list to its bottom on the next event-loop turn — deferred so any pending
/// `list_populate` has sized the content first (posted callbacks run FIFO), matching Qt's
/// scrollToBottom semantics.
fn schedule_list_scroll_end(host_key: usize) {
    let boxed: Box<dyn FnOnce() + Send> = Box::new(move || {
        if let Some(host) = LIST_STATE.with(|m| m.borrow().get(&host_key).map(|st| st.host)) {
            unsafe { ffi::day_qt_scroll_to_bottom(host) };
        }
    });
    let data = Box::into_raw(Box::new(boxed)) as *mut c_void;
    unsafe { ffi::day_qt_post(run_posted, data) };
}

/// Scroll the emulated list so `row` is at the top of the viewport (docs/list.md), on the next
/// event-loop turn — the same deferral as `schedule_list_scroll_end`.
fn schedule_list_scroll_row(host_key: usize, row: usize) {
    let boxed: Box<dyn FnOnce() + Send> = Box::new(move || {
        let target = LIST_STATE.with(|m| {
            m.borrow()
                .get(&host_key)
                .map(|st| (st.host, (row as f64 * st.row_height.max(1.0)) as c_int))
        });
        if let Some((host, y)) = target {
            unsafe { ffi::day_qt_scroll_to_y(host, y) };
        }
    });
    let data = Box::into_raw(Box::new(boxed)) as *mut c_void;
    unsafe { ffi::day_qt_post(run_posted, data) };
}

/// A source change under the cells: everything realized is now showing the wrong row's data, so
/// mark it all dirty and refill the window. Reload, reorder, and a width that re-lays every row
/// all come through here — the callers that used to rebind all n rows.
fn list_populate(host_key: usize) {
    let stale = LIST_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let st = m.get_mut(&host_key)?;
        st.bound.iter_mut().for_each(|b| *b = false);
        // A source that SHRANK leaves realized cells past its end. They stay in the pool (index
        // == row, so a source that grows back reuses each for the row it always held) and are
        // simply hidden — the same append-only pool, minus the eager building.
        let n = st.source.borrow().as_ref().map_or(0, |src| (src.len)());
        let recycle = st.source.borrow().as_ref().map(|src| src.recycle.clone())?;
        let stale: Vec<*mut c_void> = st
            .cells
            .iter()
            .skip(n)
            .copied()
            .filter(|c| !c.is_null())
            .collect();
        Some((recycle, stale))
    });
    if let Some((recycle, stale)) = stale {
        for cell in stale {
            unsafe { ffi::day_qt_set_visible(cell, 0) };
            // A hidden row past the shrunk source keeps its pooled cell but must stop
            // answering dayscript lookups — clear its element ids (day-core's recycle).
            recycle(cell);
        }
    }
    list_fill_window(host_key);
}

/// Build the rows the viewport shows and that are not built already. Idempotent and cheap when
/// nothing moved, which is what lets every scroll notification call it.
fn list_fill_window(host_key: usize) {
    // Phase 1 — under the LIST_STATE borrow: realize the window's cells + snapshot what we need.
    let Some((host, rowh, source, work, n, width)) = LIST_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let st = m.get_mut(&host_key)?;
        let source = st.source.borrow().clone()?;
        let content = unsafe { ffi::day_qt_scroll_content(st.host) };
        if content.is_null() {
            return None;
        }
        let (mut w, mut h) = (0.0_f64, 0.0_f64);
        unsafe { ffi::day_qt_widget_size(st.host, &mut w, &mut h) };
        let width = w.max(1.0) as c_int;
        let n = (source.len)();
        let rowh = st.row_height.max(1.0);
        // The rows on screen, plus the overscan. The viewport reports nothing until Qt has laid
        // the scroll area out — the framed height stands in until then.
        let (mut offset, mut vh) = (0.0_f64, 0.0_f64);
        unsafe { ffi::day_qt_list_viewport(st.host, &mut offset, &mut vh) };
        if vh <= 0.0 {
            vh = if st.frame_height > 0 {
                st.frame_height as f64
            } else {
                // Never framed and never laid out: build a screen's worth so the first paint is
                // not blank, and let the frame that follows widen the window.
                600.0
            };
        }
        let first = ((offset / rowh).floor() as usize).saturating_sub(LIST_OVERSCAN);
        let last = (((offset + vh) / rowh).ceil() as usize + LIST_OVERSCAN).min(n);
        // Slots exist for every row (a Vec of nulls, not of widgets): the cell for row i lives
        // at i for good, which is what keeps the press filter's baked row honest.
        if st.cells.len() < n {
            st.cells.resize(n, std::ptr::null_mut());
            st.bound.resize(n, false);
        }
        let mut work: Vec<(usize, *mut c_void)> = Vec::new();
        for i in first..last {
            if st.cells[i].is_null() {
                let cell = unsafe { ffi::day_qt_container_new() };
                unsafe { ffi::day_qt_add_child(content, cell) };
                // Cell index == row for the cell's whole life (docs/list.md): the press filter's
                // row is fixed at creation.
                if st.selectable {
                    unsafe {
                        ffi::day_qt_list_cell_click(cell, st.node, i as c_int, on_list_row_click)
                    };
                }
                if st.reorderable {
                    unsafe { ffi::day_qt_cell_drag(cell, st.node, i as c_int) };
                }
                st.cells[i] = cell;
            }
            if !st.bound[i] {
                st.bound[i] = true;
                work.push((i, st.cells[i]));
            }
        }
        st.last_width = width;
        Some((st.host, rowh, source, work, n, width))
    }) else {
        return;
    };
    // Phase 2 — no borrow held (bind_row re-enters with_tree, which may lay out + set_frame the
    // list host, taking LIST_STATE again).
    for (i, cell) in work {
        unsafe {
            ffi::day_qt_set_geometry(cell, 0, (i as f64 * rowh) as c_int, width, rowh as c_int);
            ffi::day_qt_set_visible(cell, 1);
        }
        (source.bind_row)(i, cell);
    }
    // The extent is the WHOLE source, built or not: the scrollbar is how the user reaches rows
    // that do not exist yet, so it cannot be sized to what happens to be realized.
    unsafe { ffi::day_qt_scroll_set_content_size(host, width, (n as f64 * rowh) as c_int) };
    // Rows realized just now start unpainted, and a reload can move which rows are selected
    // under a selection that never changed — so repaint from the entry's set on every fill.
    LIST_STATE.with(|m| {
        if let Some(st) = m.borrow().get(&host_key) {
            list_paint_selection(st);
        }
    });
}

/// A right-click landed on a widget with a summon-time menu: ask the provider and hand Qt a
/// fresh QMenu (null when the provider answers empty). Runs synchronously inside Qt's
/// customContextMenuRequested — the provider must not re-enter the event loop.
extern "C" fn on_context_menu_summon(
    node: u64,
    x: std::os::raw::c_double,
    y: std::os::raw::c_double,
) -> *mut c_void {
    ffi_guard::contain(std::ptr::null_mut(), || {
        let Some(f) = CTX_MENU_FNS.with(|m| m.borrow().get(&node).cloned()) else {
            return std::ptr::null_mut();
        };
        let items = f(day_spec::Point::new(x, y));
        if items.is_empty() {
            return std::ptr::null_mut();
        }
        let menu = unsafe { ffi::day_qt_menu_new() };
        build_qt_menu(menu, &items);
        menu
    })
}

extern "C" fn on_press(id: u64) {
    ffi_guard::contain((), || emit(NodeId(id), Event::Pressed));
}
extern "C" fn on_toggle(id: u64, on: c_int) {
    ffi_guard::contain((), || emit(NodeId(id), Event::ToggleChanged(on != 0)));
}
extern "C" fn on_text(id: u64, s: *const c_char) {
    ffi_guard::contain((), || {
        let text = unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned();
        emit(NodeId(id), Event::TextChanged(text));
    });
}
/// A styled run's link was clicked (docs/text-runs.md); the app's `.on_link()` decides what the
/// target does — Qt's own external opening is left off in the shim.
extern "C" fn on_link(id: u64, s: *const c_char) {
    ffi_guard::contain((), || {
        let url = unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned();
        emit(NodeId(id), Event::LinkActivated(url));
    });
}
extern "C" fn on_slider(id: u64, v: c_int, committed: c_int) {
    ffi_guard::contain((), || {
        let (min, max) = RANGES.with(|r| r.get(id as usize)).unwrap_or((0.0, 1.0));
        let value = min + (v as f64 / 1000.0) * (max - min);
        // The live value always; the settled one additionally, so a drag records once (the shim
        // decides which is which — see `day_qt_slider_new`).
        emit(NodeId(id), Event::ValueChanged(value));
        if committed != 0 {
            emit(NodeId(id), Event::ValueCommitted(value));
        }
    });
}
/// Focus callback from the C++ event filter (docs/focus.md).
/// kind: 0 = lost, 1 = gained, 2 = submitted (line-edit return key).
/// Arrow-key callback from the C++ key filter (docs/menus.md). Answers whether the app claimed
/// it: a canvas nobody hung `.on_key` on keeps none of them, so an enclosing scroll area still
/// scrolls with the keyboard.
extern "C" fn on_key(id: u64, code: c_int, modifiers: c_int) -> c_int {
    ffi_guard::contain(0, || {
        let key = match code {
            0 => "ArrowLeft",
            1 => "ArrowRight",
            2 => "ArrowUp",
            _ => "ArrowDown",
        };
        let node = NodeId(id);
        if !day_spec::keys::handled(node) {
            return 0;
        }
        emit(
            node,
            Event::Key(day_spec::KeyEvent {
                key: key.to_string(),
                modifiers: modifiers as u8,
            }),
        );
        1
    })
}

extern "C" fn on_focus(id: u64, kind: c_int) {
    ffi_guard::contain((), || {
        let ev = match kind {
            2 => Event::Submitted,
            k => Event::FocusChanged(k != 0),
        };
        emit(NodeId(id), ev);
    });
}

/// Gesture callback from the C++ event filter. phase: 0 tap; 1..=3 drag began/changed/ended;
/// 4..=6 pinch began/changed/ended (tx = cumulative scale); 7..=9 pan (tx/ty = delta).
extern "C" fn on_gesture(id: u64, phase: c_int, x: f64, y: f64, tx: f64, ty: f64) {
    use day_spec::{DragPhase, Point};
    ffi_guard::contain((), || {
        let at = Point::new(x, y);
        let seq_phase = |p: c_int| match p {
            0 => DragPhase::Began,
            2 => DragPhase::Ended,
            _ => DragPhase::Changed,
        };
        let ev = match phase {
            0 => Event::Tap(at),
            1 => Event::Drag {
                phase: DragPhase::Began,
                location: at,
                translation: Point::ZERO,
            },
            3 => Event::Drag {
                phase: DragPhase::Ended,
                location: at,
                translation: Point::new(tx, ty),
            },
            4..=6 => Event::Pinch {
                phase: seq_phase(phase - 4),
                scale: tx,
                location: at,
            },
            7..=9 => Event::Pan {
                phase: seq_phase(phase - 7),
                delta: Point::new(tx, ty),
                location: at,
            },
            _ => Event::Drag {
                phase: DragPhase::Changed,
                location: at,
                translation: Point::new(tx, ty),
            },
        };
        emit(NodeId(id), ev);
    });
}

fn slider_ticks(value: f64, min: f64, max: f64) -> c_int {
    if max <= min {
        return 0;
    }
    (((value - min) / (max - min)) * 1000.0).round() as c_int
}

/// A `0.0..=1.0` fraction as QProgressBar ticks (0..1000), clamped.
fn progress_ticks(fraction: f64) -> c_int {
    (fraction.clamp(0.0, 1.0) * 1000.0).round() as c_int
}

/// Renderers registered by external Day Piece crates (§8.2).
#[distributed_slice]
pub static RENDERERS: [fn() -> Renderer<Qt>];

/// A live secondary window (docs/windows.md): the day root node id, the DayWindow, and
/// its inner content widget (the adopted handle).
struct QtWin {
    win: *mut c_void,
    content: *mut c_void,
}

pub struct Qt {
    registry: Registry<Qt>,
    window: *mut c_void,
    secondary: Vec<QtWin>,
}

impl Qt {
    pub fn new() -> Self {
        register_resources();
        let mut registry = Registry::default();
        for f in RENDERERS {
            registry.register(f());
        }
        Qt {
            registry,
            window: std::ptr::null_mut(),
            secondary: Vec::new(),
        }
    }

    /// The dialog parent (docs/windows.md): the ACTIVE Day window at present time,
    /// primary as the fallback.
    fn present_parent(&self) -> *mut c_void {
        self.secondary
            .iter()
            .find(|w| unsafe { ffi::day_qt_window_is_active(w.win) } != 0)
            .map(|w| w.win)
            .unwrap_or(self.window)
    }
}

/// Register the app's native Qt resource blob (§18.3) if `day build` produced one. Once registered,
/// data reads go through `QResource::data` (zero-copy from the mmapped blob) via [`open_resource`],
/// and images load with `QPixmap(":/day/images/<name>")`.
fn register_resources() {
    let Ok(path) = std::env::var("DAY_QRESOURCE") else {
        return;
    };
    unsafe { ffi::day_qt_register_resource(cstr(&path).as_ptr()) };
    day_spec::resource::set_resource_opener(open_resource);
}

/// `resource("name")` → the `:/day/assets/<name>` Qt resource, borrowed zero-copy from the
/// registered blob (valid for the app lifetime, so an empty guard suffices).
fn open_resource(name: &str) -> Option<day_spec::resource::Resource> {
    let respath = format!(":/day/assets/{name}");
    let mut len: usize = 0;
    let ptr = unsafe { ffi::day_qt_resource_data(cstr(&respath).as_ptr(), &mut len) };
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { day_spec::resource::Resource::from_raw(ptr as *const u8, len, Box::new(())) })
}

impl Default for Qt {
    fn default() -> Self {
        Self::new()
    }
}

fn content_of(parent: &QtHandle) -> *mut c_void {
    // Scroll areas expose an inner content widget; the shim returns null for non-scrolls.
    let inner = unsafe { ffi::day_qt_scroll_content(parent.0) };
    if inner.is_null() { parent.0 } else { inner }
}

/// Point size + the style's inherent weight for a logical [`Font`] (Qt has no semantic text styles;
/// we approximate the platform typographic scale, matching Apple's text-style sizes for consistency).
/// Public for standalone pieces (docs/extending.md), which have to resolve the same scale.
pub fn qt_style(f: Font) -> (f64, day_spec::FontWeight) {
    use day_spec::FontWeight::*;
    match f {
        Font::LargeTitle => (26.0, Regular),
        Font::Title => (22.0, Regular),
        Font::Title2 => (17.0, Regular),
        Font::Title3 => (15.0, Regular),
        Font::Headline => (13.0, Semibold),
        Font::Subheadline => (11.0, Regular),
        Font::Body => (13.0, Regular),
        Font::Callout => (12.0, Regular),
        Font::Footnote => (10.0, Regular),
        Font::Caption => (10.0, Regular),
        Font::Caption2 => (10.0, Regular),
        Font::System(pt) => (pt, Regular),
        Font::Custom(_, pt) => (pt, Regular),
    }
}

/// Render a label's runs as Qt rich text (docs/text-runs.md).
///
/// Qt's rich text is an HTML subset, so this goes through the SHARED serializer — the escaping
/// is the risk, and it is the same risk GTK has, so it is written and tested once in day-spec.
fn set_label_runs(w: *mut c_void, text: &str, runs: &[day_spec::TextRun]) {
    if runs.is_empty() {
        // The plain setter also puts the format back to PlainText, so a label that loses its
        // runs stops parsing markup.
        unsafe { ffi::day_qt_label_set_text(w, cstr(text).as_ptr()) };
        return;
    }
    let base_pt = LABEL_PT
        .with(|t| t.get(w as usize))
        .unwrap_or_else(|| qt_style(Font::Body).0);
    let html = day_spec::runs_to_markup(text, runs, day_spec::MarkupDialect::QtHtml, base_pt);
    unsafe { ffi::day_qt_label_set_rich_text(w, cstr(&html).as_ptr()) };
}

/// Remember a label's resolved point size for [`set_label_runs`].
fn remember_label_pt(w: *mut c_void, f: day_spec::FontSpec) {
    LABEL_PT.with(|t| t.insert(w as usize, f.resolved_points(qt_style(f.style).0)));
}

/// Apply a `Font::Custom` family on top of `day_qt_label_set_font` (which set size/weight/italic).
/// The family was registered with the QFontDatabase in `run()`; an unknown name falls back to the
/// default family inside Qt.
unsafe fn apply_custom_family(w: *mut c_void, spec: day_spec::FontSpec) {
    if let Font::Custom(family, _) = spec.style {
        unsafe { ffi::day_qt_label_set_font_family(w, cstr(family).as_ptr()) };
    }
}

/// Day weight → QFont::Weight numeric value (Thin=100 … Black=900).
fn qt_weight(w: day_spec::FontWeight) -> c_int {
    use day_spec::FontWeight as W;
    match w {
        W::Thin => 100,
        W::UltraLight => 200,
        W::Light => 300,
        W::Regular => 400,
        W::Medium => 500,
        W::Semibold => 600,
        W::Bold => 700,
        W::Heavy => 800,
        W::Black => 900,
    }
}

/// (point size, QFont weight, italic flag) for the C++ shim.
fn font_params(spec: day_spec::FontSpec) -> (f64, c_int, c_int, c_int) {
    let (pt, inherent) = qt_style(spec.style);
    let weight = qt_weight(spec.weight.unwrap_or(inherent));
    (pt, weight, spec.italic as c_int, spec.tabular as c_int)
}

// ---------------------------------------------------------------------------
// Navigation (docs/navigation.md): QSplitter host, sidebar + detail panes. Day sizes
// page content from the pane sizes reported here via FrameChanged.
// ---------------------------------------------------------------------------

struct NavState {
    sidebar_pane: *mut std::os::raw::c_void,
    detail_pane: *mut std::os::raw::c_void,
    /// (page, node id) in insertion order. The sidebar page — when the host has one — is
    /// always first; what changes with the presentation is its PARENT, not its position.
    pages: Vec<(QtHandle, NodeId)>,
    /// The host's sidebar page, once it has one (docs/size-classes.md).
    sidebar_page: Option<QtHandle>,
    /// Sidebar+detail split (selector Sidebar) vs. a pure push/pop stack (`stack`).
    split: bool,
    /// Whether the sidebar pane is showing. Tracked rather than read back because the shim
    /// exposes `day_qt_set_visible` and no getter — what a `SidebarToggle` item flips.
    sidebar_shown: bool,
    /// Stack presentation: title per level (index 0 = root) for the back header — desktop has
    /// no system back affordance, so the header gives a pushed page its way out.
    titles: Vec<String>,
}

/// Show/hide the sidebar of this process's `selector(Sidebar)` host — what a
/// [`day_spec::ToolbarItemKind::SidebarToggle`] item drives (docs/toolbars.md). `false` when
/// there is no split host, which is how the item knows to render disabled. Same
/// single-window limit as the GTK twin: `NAV_STATE` is not keyed by window.
pub(crate) fn toggle_sidebar() -> bool {
    NAV_STATE.with(|m| {
        for st in m.borrow_mut().values_mut() {
            if st.split {
                st.sidebar_shown = !st.sidebar_shown;
                unsafe { ffi::day_qt_set_visible(st.sidebar_pane, i32::from(st.sidebar_shown)) };
                return true;
            }
        }
        false
    })
}

/// The stack-nav header's back button: a day-initiated pop — the host's handler writes it
/// into the path signal, which reconciles the pop.
extern "C" fn nav_back_clicked(id: u64) {
    ffi_guard::contain((), || {
        emit(
            NodeId(id),
            Event::NavBack {
                already_popped: false,
            },
        )
    });
}

/// Sync the back header to the stack depth (visible while a pushed page shows), then
/// re-report page sizes — the pages host shrinks/grows by the header height.
fn nav_sync_header(host: *mut std::os::raw::c_void) {
    let (visible, title) = NAV_STATE.with(|m| {
        let m = m.borrow();
        let Some(state) = m.get(&(host as usize)) else {
            return (false, String::new());
        };
        (
            !state.split && state.pages.len() >= 2,
            state.titles.last().cloned().unwrap_or_default(),
        )
    });
    unsafe {
        ffi::day_qt_nav_header_update(host, visible as c_int, cstr(&title).as_ptr());
    }
    nav_sync_panes(host);
}

/// Is this page the host's sidebar pane?
fn is_sidebar_page(page: *mut std::os::raw::c_void) -> bool {
    PAGE_PANE
        .with(|m| m.borrow().get(&(page as usize)).copied() == Some(day_spec::props::Pane::Sidebar))
}

/// Re-present a live nav host (docs/size-classes.md): the window crossed a breakpoint, so the
/// chrome changes but the pages do not.
///
/// Both presentations are the same `QSplitter` with the same back header already installed, so
/// the morph is: show or hide the sidebar pane, re-parent the sidebar PAGE between the two panes,
/// and let the header follow the new depth. `day_qt_add_child` re-parents a widget rather than
/// rebuilding it, so each page keeps its children and its scroll state.
fn nav_present(host: *mut std::os::raw::c_void, split: bool) {
    let Some((sidebar_page, sidebar_pane, detail_pane, was_split)) = NAV_STATE.with(|m| {
        let st = m.borrow();
        let s = st.get(&(host as usize))?;
        Some((s.sidebar_page, s.sidebar_pane, s.detail_pane, s.split))
    }) else {
        return;
    };
    if was_split == split {
        return;
    }
    unsafe {
        ffi::day_qt_set_visible(sidebar_pane, i32::from(split));
        if let Some(page) = sidebar_page {
            // Its own pane when split, the stack's root page when not.
            ffi::day_qt_add_child(if split { sidebar_pane } else { detail_pane }, page.0);
            ffi::day_qt_set_visible(page.0, 1);
        }
    }
    NAV_STATE.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(s) = m.get_mut(&(host as usize)) {
            s.split = split;
            s.sidebar_shown = split;
            // Only the top detail page shows; the sidebar page, when split, sits outside that.
            let detail: Vec<QtHandle> = s
                .pages
                .iter()
                .filter(|(p, _)| !(split && is_sidebar_page(p.0)))
                .map(|(p, _)| *p)
                .collect();
            let last = detail.len().saturating_sub(1);
            for (i, page) in detail.iter().enumerate() {
                unsafe { ffi::day_qt_set_visible(page.0, i32::from(i == last)) };
            }
        }
    });
    // Deferred one turn, like the pop path: hiding a QSplitter pane does not resize its sibling
    // until Qt runs its own layout pass, so reading the pane sizes now would report the OLD
    // detail width and leave the page laid out for a window that still had a sidebar.
    let host_addr = host as usize;
    <Qt as Platform>::post(Box::new(move || {
        nav_sync_header(host_addr as *mut std::os::raw::c_void)
    }));
}

/// Report both pane sizes so NavLayout re-lays page content (enqueue-only, §8.3).
fn nav_sync_panes(host: *mut std::os::raw::c_void) {
    let reports: Vec<(NodeId, Size)> = NAV_STATE.with(|m| {
        let m = m.borrow();
        let Some(state) = m.get(&(host as usize)) else {
            return Vec::new();
        };
        let (mut sw, mut sh, mut dw, mut dh) = (0.0, 0.0, 0.0, 0.0);
        unsafe {
            ffi::day_qt_widget_size(state.sidebar_pane, &mut sw, &mut sh);
            ffi::day_qt_widget_size(state.detail_pane, &mut dw, &mut dh);
        }
        if sh <= 0.0 && dh <= 0.0 {
            return Vec::new();
        }
        state
            .pages
            .iter()
            .map(|(page, id)| {
                // A page in the sidebar pane is sized by it; everything else fills the detail.
                let size = if state.split && is_sidebar_page(page.0) {
                    Size::new(sw, sh)
                } else {
                    Size::new(dw, dh)
                };
                (*id, size)
            })
            .collect()
    });
    for (id, size) in reports {
        emit(id, Event::FrameChanged(size));
    }
}

extern "C" fn nav_splitter_moved(host: *mut std::os::raw::c_void) {
    ffi_guard::contain((), || nav_sync_panes(host));
}

// ---------------------------------------------------------------------------
// Inspector (docs/inspector.md): the nav QSplitter mirrored — content pane leading with the
// stretch, a show/hidable panel pane trailing at its preferred width, divider draggable.
// ---------------------------------------------------------------------------

struct InspectorState {
    content_pane: *mut std::os::raw::c_void,
    panel_pane: *mut std::os::raw::c_void,
    /// `(pane NodeId, is-panel)` in attach order, for frame reports.
    panes: Vec<(NodeId, bool)>,
    width: f64,
    shown: bool,
}

/// Report both pane sizes so `InspectorLayout` re-lays content (`nav_sync_panes`'s
/// counterpart). The panel reports its preferred width while hidden, so a reveal re-lays
/// nothing.
fn inspector_sync_panes(host: *mut std::os::raw::c_void) {
    let reports: Vec<(NodeId, Size)> = INSPECTOR_STATE.with(|m| {
        let m = m.borrow();
        let Some(state) = m.get(&(host as usize)) else {
            return Vec::new();
        };
        let (mut cw, mut ch, mut pw, mut ph) = (0.0, 0.0, 0.0, 0.0);
        unsafe {
            ffi::day_qt_widget_size(state.content_pane, &mut cw, &mut ch);
            ffi::day_qt_widget_size(state.panel_pane, &mut pw, &mut ph);
        }
        let _ = ph;
        if ch <= 0.0 {
            return Vec::new();
        }
        state
            .panes
            .iter()
            .map(|(id, panel)| {
                let size = if *panel {
                    let w = if state.shown && pw > 0.0 {
                        pw
                    } else {
                        state.width
                    };
                    Size::new(w, ch)
                } else {
                    Size::new(cw, ch)
                };
                (*id, size)
            })
            .collect()
    });
    for (id, size) in reports {
        emit(id, Event::FrameChanged(size));
    }
}

extern "C" fn inspector_splitter_moved(host: *mut std::os::raw::c_void) {
    ffi_guard::contain((), || inspector_sync_panes(host));
}

// ---------------------------------------------------------------------------
// The navigation suite (docs/navigation.md): a QTabWidget host that owns its page widgets.
// ---------------------------------------------------------------------------

/// The navigation suite (`NavPresentation::Tabs`): a QTabWidget whose bar IS the host's row list.
struct NavSuite {
    /// The host's own node — what the QTabWidget's signal carries, so a click can find its suite
    /// even when a window holds more than one.
    host: NodeId,
    /// The nav menu's node. A tab click reports against it, exactly as a sidebar row click does,
    /// so the two are one event to everything above this backend. Zero until the menu arrives.
    menu_node: NodeId,
    /// Destination pages in bar order — index i IS the `Select(i)` index. The sidebar page is
    /// not in here; it is a hidden tab, because its rows became the bar.
    pages: Vec<(QtHandle, NodeId)>,
    /// The row labels and glyph names, once the menu has arrived.
    titles: Vec<String>,
    icons: Vec<Option<String>>,
}

/// Put the rows on the bar, for as many tabs as currently exist.
///
/// Called from BOTH sides, because either can arrive first: the pages are inserted as the tree is
/// built, and the rows come with the nav menu nested somewhere inside the sidebar page. Whichever
/// lands last completes the bar.
fn suite_apply_rows(host: *mut std::os::raw::c_void) {
    NAV_SUITES.with(|m| {
        let m = m.borrow();
        let Some(suite) = m.get(&(host as usize)) else {
            return;
        };
        for (i, title) in suite.titles.iter().enumerate() {
            // Tab 0 is the hidden sidebar page, so row i is tab i + 1.
            if i >= suite.pages.len() {
                break;
            }
            let at = (i + 1) as c_int;
            unsafe { ffi::day_qt_tabs_set_title(host, at, cstr(title).as_ptr()) };
            let icon = suite
                .icons
                .get(i)
                .and_then(|o| o.as_deref())
                .filter(|n| !n.is_empty())
                .map(icon_file_path)
                .unwrap_or_default();
            if !icon.is_empty() {
                unsafe { ffi::day_qt_tabs_set_icon(host, at, cstr(&icon).as_ptr()) };
            }
        }
    });
}

/// Report each destination's content size so NavLayout re-lays it (enqueue-only, §8.3) — the
/// suite's counterpart of the list's own sizing, and for the same reason: QTabWidget owns page
/// geometry.
fn nav_suite_sync(host: *mut std::os::raw::c_void) {
    let reports: Vec<(NodeId, Size)> = NAV_SUITES.with(|m| {
        let m = m.borrow();
        let Some(suite) = m.get(&(host as usize)) else {
            return Vec::new();
        };
        let (mut w, mut h) = (0.0, 0.0);
        unsafe { ffi::day_qt_tabs_content_size(host, &mut w, &mut h) };
        if w <= 0.0 || h <= 0.0 {
            return Vec::new();
        }
        suite
            .pages
            .iter()
            .map(|(_, id)| (*id, Size::new(w, h)))
            .collect()
    });
    for (id, size) in reports {
        emit(id, Event::FrameChanged(size));
    }
}

/// A realized nav menu's rows: `(node, titles, icon names)`.
type NavRow = (NodeId, Vec<String>, Vec<Option<String>>);

/// A tab click. Reported against the MENU, not the host, so one handler serves every presentation.
extern "C" fn nav_suite_changed(id: u64, index: c_int) {
    ffi_guard::contain((), || {
        let node = NAV_SUITES.with(|m| {
            m.borrow()
                .values()
                .find(|s| s.host.0 == id && s.menu_node.0 != 0)
                .map(|s| s.menu_node)
        });
        emit(
            node.unwrap_or(NodeId(id)),
            Event::SelectionChanged(index as i64),
        );
    });
}

/// Report each tab page's content size so NavLayout re-lays it (enqueue-only, §8.3).
extern "C" fn present_cb(req: u64, tag: c_int, index: i64, text: *const c_char) {
    ffi_guard::contain((), || {
        let text = if text.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(text) }
                .to_string_lossy()
                .into_owned()
        };
        let result = day_spec::present::PresentResult::decode(tag, index, text);
        emit_window(Event::PresentResult { req, result });
    });
}

fn emit_window(ev: Event) {
    let sink = SINK.with(|s| s.borrow().clone());
    if let Some(sink) = sink {
        sink(day_spec::WINDOW_NODE, ev);
    }
}

// Secondary-window event trampolines (docs/windows.md): the shim keys them by day node id.
extern "C" fn win_resized(node: u64, w: c_int, h: c_int) {
    ffi_guard::contain((), || {
        emit(
            day_spec::NodeId(node),
            Event::WindowResized(Size::new(w as f64, h as f64)),
        )
    });
}
extern "C" fn win_closed(node: u64) {
    ffi_guard::contain((), || emit(day_spec::NodeId(node), Event::WindowClosed));
}
extern "C" fn win_focused(node: u64, active: c_int) {
    ffi_guard::contain((), || {
        emit(day_spec::NodeId(node), Event::WindowFocused(active != 0))
    });
}

extern "C" fn window_resized(w: c_int, h: c_int) {
    ffi_guard::contain((), || {
        let size = Size::new(w as f64, h as f64);
        LAST_WINDOW_SIZE.with(|c| c.set(size));
        emit(day_spec::WINDOW_NODE, Event::WindowResized(size));
        // Presented emulated covers track the content area (docs/cover.md: the frame is
        // native-owned while presented).
        COVERS.with(|c| {
            for (cover, node) in c.borrow().iter() {
                unsafe {
                    ffi::day_qt_set_geometry(
                        *cover,
                        0,
                        0,
                        size.width as c_int,
                        size.height as c_int,
                    )
                };
                emit(*node, Event::FrameChanged(size));
            }
        });
    });
}

extern "C" fn nav_menu_changed(id: u64, row: std::os::raw::c_int) {
    // -1 = cleared (programmatic unselect fires nothing thanks to blockSignals; a clear
    // reaching here means the widget emptied — ignore). Deferred: selecting a sidebar item
    // rebuilds the detail, which must not run inside the list's own event dispatch.
    ffi_guard::contain((), || {
        if row >= 0 {
            emit_deferred(NodeId(id), Event::SelectionChanged(row as i64));
        }
    });
}

extern "C" fn on_menu_action(id: u64) {
    ffi_guard::contain((), || emit_window(Event::MenuAction(id)));
}

/// Resolve a bundled image NAME to a loadable file path (or "" if it doesn't resolve).
/// Used for per-row nav-menu icons, which the shim loads + tints to the palette text color.
fn icon_file_path(name: &str) -> String {
    // The staged glyph SVG first (docs/vectors.md): Qt's SVG icon engine renders it at the icon
    // size asked for, which keeps a nav row or tab glyph sharp at any scale. The raster cache is
    // the fallback for names that have no vector — ordinary bundled images.
    day_spec::resource::resolve_vector_svg(name)
        .or_else(|| day_spec::resource::resolve_image_file(name))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Build the nav list's per-row context menus (docs/menus.md) and hand them to the shim:
/// one QMenu per row with entries (null where a row has none), the same
/// [`build_qt_menu`] walk the piece decorator uses.
fn navlist_apply_row_menus(w: *mut c_void, menus: &[Vec<day_spec::MenuItem>]) {
    let ptrs: Vec<*mut c_void> = menus
        .iter()
        .map(|items| {
            if items.is_empty() {
                std::ptr::null_mut()
            } else {
                let m = unsafe { ffi::day_qt_menu_new() };
                build_qt_menu(m, items);
                m
            }
        })
        .collect();
    unsafe { ffi::day_qt_navlist_set_row_menus(w, ptrs.as_ptr(), ptrs.len() as i32) };
}

/// Per-row nav icon tints as a U+001F-joined list of "#rrggbb" entries (empty entry = no
/// row tint, the shim falls back to the palette text color), parallel to the icon list
/// (docs/vectors.md).
/// A color as the `"#rrggbb"` the shim's `QColor` parses.
fn hex_rgb(c: day_spec::Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        (c.r * 255.0) as u8,
        (c.g * 255.0) as u8,
        (c.b * 255.0) as u8
    )
}

fn nav_tints_joined(tints: &[Option<day_spec::Color>]) -> String {
    tints
        .iter()
        .map(|t| match t {
            Some(c) => format!(
                "#{:02x}{:02x}{:02x}",
                (c.r * 255.0) as u8,
                (c.g * 255.0) as u8,
                (c.b * 255.0) as u8
            ),
            None => String::new(),
        })
        .collect::<Vec<_>>()
        .join("\u{1f}")
}

/// Which lifecycle phases this desktop backend delivers (docs/lifecycle.md): the universal set.
/// `const` so `day::require_lifecycle!` can reject unsupported phases at compile time.
pub const fn lifecycle_supported(phase: day_spec::Lifecycle) -> bool {
    phase.is_universal()
}

/// Phase codes (from the Qt shim's QApplication signals) → day lifecycle events.
extern "C" fn on_lifecycle(code: c_int) {
    use day_spec::Lifecycle::*;
    ffi_guard::contain((), || {
        let phase = match code {
            2 => DidBecomeActive,
            3 => WillResignActive,
            7 => WillTerminate,
            _ => return,
        };
        emit_window(Event::Lifecycle(phase));
    });
}

/// A QKeySequence-parseable string for a shortcut. Qt maps `Ctrl` to ⌘ on macOS, so `primary`
/// renders as the platform's command modifier everywhere.
fn qt_shortcut(sc: &day_spec::Shortcut) -> String {
    let mut parts: Vec<String> = Vec::new();
    if sc.primary {
        parts.push("Ctrl".into());
    }
    if sc.shift {
        parts.push("Shift".into());
    }
    if sc.alt {
        parts.push("Alt".into());
    }
    if sc.control {
        // The *physical* Control key: Qt::MetaModifier is Control on macOS, Control elsewhere.
        #[cfg(target_os = "macos")]
        parts.push("Meta".into());
        #[cfg(not(target_os = "macos"))]
        parts.push("Ctrl".into());
    }
    // Single letters are case-insensitive to Qt; named keys ("Delete", "Return", "F11") pass through.
    let key = if sc.key.chars().count() == 1 {
        sc.key.to_uppercase()
    } else {
        sc.key.clone()
    };
    parts.push(key);
    // Delete and Backspace end up bound together — the shim pairs them once the sequence is
    // parsed, where the key CODE is visible and no separator has to survive a shortcut string
    // that may itself contain a comma or semicolon (⌘, is Preferences).
    parts.join("+")
}

/// Default label for a standard role when the app left the entry's label empty.
fn qt_role_label(role: day_spec::MenuRole) -> &'static str {
    use day_spec::MenuRole::*;
    match role {
        Cut => "Cut",
        Copy => "Copy",
        Paste => "Paste",
        SelectAll => "Select All",
        Undo => "Undo",
        Redo => "Redo",
        Delete => "Delete",
        About => "About",
        Quit => "Quit",
        Preferences => "Preferences…",
        Minimize => "Minimize",
        CloseWindow => "Close",
        Fullscreen => "Enter Full Screen",
        NewWindow => "New Window",
    }
}

/// Default portable shortcut for a role (Qt maps `Ctrl` → ⌘ on macOS).
fn qt_role_shortcut(role: day_spec::MenuRole) -> Option<&'static str> {
    use day_spec::MenuRole::*;
    Some(match role {
        Cut => "Ctrl+X",
        Copy => "Ctrl+C",
        Paste => "Ctrl+V",
        SelectAll => "Ctrl+A",
        Undo => "Ctrl+Z",
        Redo => "Ctrl+Shift+Z",
        Quit => "Ctrl+Q",
        CloseWindow => "Ctrl+W",
        Minimize => "Ctrl+M",
        Preferences => "Ctrl+,",
        NewWindow => "Ctrl+N",
        Delete | About | Fullscreen => return None,
    })
}

/// Walk the day-neutral menu tree, issuing flat builder calls against a QMenu pointer.
pub(crate) fn build_qt_menu(menu: *mut c_void, items: &[day_spec::MenuItem]) {
    for item in items {
        match item {
            day_spec::MenuItem::Separator => unsafe { ffi::day_qt_menu_add_separator(menu) },
            day_spec::MenuItem::Submenu { label, items, .. } => {
                let sub = unsafe { ffi::day_qt_menu_add_submenu(menu, cstr(label).as_ptr()) };
                build_qt_menu(sub, items);
            }
            day_spec::MenuItem::Action {
                id,
                label,
                shortcut,
                enabled,
                role,
                icon,
            } => {
                // A nonzero id ALWAYS wins (the appkit precedence, docs/menus.md): the item
                // dispatches the day action, with the role only supplying label/shortcut
                // defaults — the auto Preferences item and `MenuRole::NewWindow` arrive
                // this way. Routing them through the role-only path dropped the dispatch
                // id, leaving visible-but-dead items (the macos-qt bug).
                if *id != 0 {
                    let text = match role {
                        Some(r) if label.is_empty() => qt_role_label(*r).to_string(),
                        _ => label.clone(),
                    };
                    let sc = shortcut
                        .as_ref()
                        .map(qt_shortcut)
                        .or_else(|| role.and_then(|r| qt_role_shortcut(r).map(str::to_string)))
                        .unwrap_or_default();
                    let (ic, ic_fallback) = crate::toolbar::icon_args(icon.as_ref());
                    unsafe {
                        ffi::day_qt_menu_add_action(
                            menu,
                            cstr(&text).as_ptr(),
                            *id,
                            cstr(&sc).as_ptr(),
                            *enabled as c_int,
                            cstr(&ic).as_ptr(),
                            ic_fallback,
                        )
                    };
                } else if let Some(role) = role {
                    let text = if label.is_empty() {
                        qt_role_label(*role).to_string()
                    } else {
                        label.clone()
                    };
                    let sc = shortcut
                        .as_ref()
                        .map(qt_shortcut)
                        .or_else(|| qt_role_shortcut(*role).map(str::to_string))
                        .unwrap_or_default();
                    unsafe {
                        ffi::day_qt_menu_add_role(
                            menu,
                            cstr(&text).as_ptr(),
                            *role as c_int,
                            cstr(&sc).as_ptr(),
                        )
                    };
                }
            }
        }
    }
}

/// Warn ONCE per kind that this backend has no registered renderer for `kind`, before falling back to
/// a visible placeholder. A missing renderer usually means the piece's `qt` feature wasn't enabled
/// (Tier A.2 derives it automatically under `day build`). Deduped per kind so a placeholder rendered
/// every frame doesn't spam the log.
fn warn_missing_renderer(kind: PieceKind) {
    day_spec::placeholder::report(kind, "qt");
}

/// The visible placeholder a realize arm degrades to when its props payload has the wrong type
/// (`props_of` has already reported the mismatch) — the same label the missing-renderer arm shows.
pub(crate) fn placeholder_handle(kind: PieceKind) -> QtHandle {
    QtHandle(unsafe { ffi::day_qt_label_new(cstr(&format!("⟨{kind}⟩")).as_ptr()) })
}

impl Toolkit for Qt {
    type Handle = QtHandle;

    fn capability(&self, cap: Cap) -> Support {
        match cap {
            // QPlainTextEdit honors editable + selectable; Qt ships no built-in spell-check, so
            // Cap::TextSpellCheck stays Unsupported (the default arm).
            Cap::TextRuns
            | Cap::TextLinks
            | Cap::Snapshot
            | Cap::NavSplit
            // One QSplitter carries both presentations, with the back header installed either
            // way — so re-presenting is a pane visibility flip plus one re-parent of the
            // sidebar page (docs/size-classes.md).
            | Cap::NavRepresent
            | Cap::Dialogs
            | Cap::FileDialogs
            | Cap::TextEditable
            | Cap::TextSelectable
            // Qt's own QDrag pipeline: grabbed-cell pixmap, insertion line, no-drop cursor.
            | Cap::ListReorder
            // Real DayWindows on the shared QApplication (docs/windows.md).
            | Cap::MultiWindow
            // A QTabWidget, which is Qt's own one-of-N container and already the shape Day
            // wants: it owns its pages and shows one at a time (docs/navigation.md).
            //
            // `Cap::NavTabsAdaptive` is deliberately not here: a Qt app may PIN a tab bar, but a
            // narrowing window hides its sidebar and pushes rather than growing one.
            | Cap::NavTabs
            // A real QToolBar under the menu bar (docs/toolbars.md).
            | Cap::AppMenu
            | Cap::Toolbar
            // The nav QSplitter mirrored: panel pane trailing, divider draggable
            // (docs/inspector.md — not a QDockWidget; DayWindow is no QMainWindow).
            | Cap::Inspector => Support::Native,
            // A topmost child of the window content — not a system modal (docs/cover.md).
            Cap::Cover => Support::Emulated,
            // The COMPOSED tree (docs/tree.md M2): the piece flattens onto this backend's
            // emulated list; disclosure, indentation and row menus are day pieces. No native
            // drag, so `Cap::TreeMove` stays Unsupported.
            Cap::Tree => Support::Emulated,
            // Derived from QFontMetrics — Qt publishes no baseline of its own
            // (docs/baseline.md).
            Cap::BaselineAlignment => Support::Emulated,
            _ => Support::Unsupported,
        }
    }

    fn realize(&mut self, kind: PieceKind, props: &dyn std::any::Any, id: NodeId) -> QtHandle {
        unsafe {
            match Builtin::from_key(kind) {
                Some(Builtin::Inspector) => {
                    let (visible, width, leading) = props
                        .downcast_ref::<InspectorProps>()
                        .map(|p| {
                            (
                                p.visible,
                                p.width,
                                p.edge == day_spec::props::PaneEdge::Leading,
                            )
                        })
                        .unwrap_or((false, 280.0, false));
                    let host = ffi::day_qt_inspector_new(width, c_int::from(leading));
                    // Positional panes, role-mapped: the LEADING arrangement holds the panel
                    // first (docs/inspector.md `.edge`).
                    let (content_ix, panel_ix) = if leading { (1, 0) } else { (0, 1) };
                    let content_pane = ffi::day_qt_splitter_pane(host, content_ix);
                    let panel_pane = ffi::day_qt_splitter_pane(host, panel_ix);
                    ffi::day_qt_splitter_on_moved(host, inspector_splitter_moved);
                    // …and on every layout pass, so the panes stop reporting the placeholder
                    // geometry they have at insert time (before the window lays the splitter
                    // out at all).
                    ffi::day_qt_splitter_on_resized(host, inspector_splitter_moved);
                    ffi::day_qt_set_visible(panel_pane, c_int::from(visible));
                    INSPECTOR_STATE.with(|m| {
                        m.borrow_mut().insert(
                            host as usize,
                            InspectorState {
                                content_pane,
                                panel_pane,
                                panes: Vec::new(),
                                width,
                                shown: visible,
                            },
                        )
                    });
                    QtHandle(host)
                }
                Some(Builtin::InspectorPane) => {
                    let w = ffi::day_qt_container_new();
                    let panel = props
                        .downcast_ref::<InspectorPaneProps>()
                        .map(|p| p.panel)
                        .unwrap_or(false);
                    INSPECTOR_PANE_IDS.with(|m| m.borrow_mut().insert(w as usize, (id, panel)));
                    QtHandle(w)
                }
                Some(Builtin::Container) => {
                    let w = ffi::day_qt_container_new();
                    if let Some(p) = props.downcast_ref::<ContainerProps>()
                        && p.role == Some(day_spec::SurfaceRole::SectionCard)
                    {
                        // A translucent neutral over the window color: subtle in any palette,
                        // and it follows the platform / forced (DAY_THEME) color scheme.
                        ffi::day_qt_widget_set_section_card(w, p.corner_radius);
                    } else if let Some(p) = props.downcast_ref::<ContainerProps>()
                        && (p.background.is_some() || p.corner_radius > 0.0 || p.clips)
                    {
                        let bg = p.background.unwrap_or(day_spec::Color::CLEAR);
                        ffi::day_qt_widget_set_surface(
                            w,
                            bg.r,
                            bg.g,
                            bg.b,
                            bg.a,
                            p.corner_radius,
                            p.clips as c_int,
                        );
                    }
                    QtHandle(w)
                }
                Some(Builtin::Nav) => {
                    let nav_props = props.downcast_ref::<NavProps>();
                    // Where the rows are the CHROME, the host is a QTabWidget — Qt's own
                    // one-of-N container, and already the shape Day wants here: it owns its page
                    // widgets and shows one at a time, so the pages stay resident and
                    // `NavPatch::Select` drives the bar instead of push/pop.
                    //
                    // A desktop only reaches this by PINNING `SelectorStyle::Tabs`;
                    // `Cap::NavTabsAdaptive` is off here, so a narrowing window hides the sidebar
                    // and pushes rather than growing a bar.
                    if nav_props.is_some_and(|p| p.presentation.rows_are_chrome()) {
                        let w = ffi::day_qt_tabs_new(id.0, nav_suite_changed);
                        NAV_SUITES.with(|m| {
                            m.borrow_mut().insert(
                                w as usize,
                                NavSuite {
                                    host: id,
                                    menu_node: NodeId(0),
                                    pages: Vec::new(),
                                    titles: Vec::new(),
                                    icons: Vec::new(),
                                },
                            )
                        });
                        return QtHandle(w);
                    }
                    let is_split = nav_props.map(|p| p.presentation.is_split()).unwrap_or(true);
                    let host = ffi::day_qt_splitter_new();
                    let sidebar_pane = ffi::day_qt_splitter_pane(host, 0);
                    let mut detail_pane = ffi::day_qt_splitter_pane(host, 1);
                    ffi::day_qt_splitter_on_moved(host, nav_splitter_moved);
                    ffi::day_qt_splitter_on_resized(host, nav_splitter_moved);
                    // The back header goes in for BOTH presentations, hidden until a stack
                    // actually pushes. Installing it lazily would mean restructuring the detail
                    // pane's layout under live, absolutely-positioned pages the first time a
                    // window narrowed (docs/size-classes.md) — this way the widget tree is the
                    // same shape either way and a re-present only flips visibility.
                    let pages = ffi::day_qt_nav_header_install(host, id.0, nav_back_clicked);
                    if !pages.is_null() {
                        detail_pane = pages;
                    }
                    // Depth titles for the back header. Recorded in BOTH presentations, so a
                    // window that narrows into a stack already knows what to put in the header
                    // (docs/size-classes.md); the header simply stays hidden while split.
                    let titles = vec![nav_props.map(|p| p.title.clone()).unwrap_or_default()];
                    if !is_split {
                        // A stack has no sidebar: hide the empty pane so the detail is full-width.
                        ffi::day_qt_set_visible(sidebar_pane, 0);
                    }
                    NAV_STATE.with(|m| {
                        m.borrow_mut().insert(
                            host as usize,
                            NavState {
                                sidebar_pane,
                                detail_pane,
                                pages: Vec::new(),
                                sidebar_page: None,
                                split: is_split,
                                sidebar_shown: is_split,
                                titles,
                            },
                        )
                    });
                    QtHandle(host)
                }
                Some(Builtin::NavPage) => {
                    let page = QtHandle(ffi::day_qt_container_new());
                    NAV_PAGE_IDS.with(|m| m.borrow_mut().insert(page.0 as usize, id));
                    if let Some(p) = props.downcast_ref::<day_spec::props::NavPageProps>() {
                        PAGE_PANE.with(|m| m.borrow_mut().insert(page.0 as usize, p.pane));
                    }
                    page
                }
                // Emulated fullscreen cover (docs/cover.md): parked hidden; Present re-homes
                // it onto the window content, topmost, at the content size.
                Some(Builtin::Cover) => {
                    let cover = QtHandle(ffi::day_qt_container_new());
                    ffi::day_qt_set_visible(cover.0, 0);
                    COVER_IDS.with(|m| m.borrow_mut().insert(cover.0 as usize, id));
                    cover
                }
                Some(Builtin::NavMenu) => {
                    let Some(p) = props_of::<NavMenuProps>(kind, "qt", props) else {
                        return placeholder_handle(kind);
                    };
                    let w = ffi::day_qt_navlist_new(id.0, nav_menu_changed);
                    // Keep the rows for `insert`, where a navigation suite above this menu can
                    // finally be found and handed them.
                    NAV_SUITE_ROWS.with(|m| {
                        m.borrow_mut()
                            .insert(w as usize, (id, p.items.clone(), p.icons.clone()))
                    });
                    let joined = p.items.join("\u{1f}");
                    // Parallel per-row icon file paths (empty entry = no icon). Resolve the
                    // bundled image NAME to a loadable file path on the Rust side; the shim
                    // loads a QPixmap, tints it to the palette text color, and setIcon()s it.
                    let icons_joined = p
                        .icons
                        .iter()
                        .map(|ic| ic.as_deref().map(icon_file_path).unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join("\u{1f}");
                    let tints_joined = nav_tints_joined(&p.tints);
                    // The trailing status glyph rides the same resolve-then-tint path as the
                    // leading icon; the shim's delegate paints it at the row's far edge.
                    let badge_icons_joined = p
                        .badge_icons
                        .iter()
                        .map(|ic| ic.as_deref().map(icon_file_path).unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join("\u{1f}");
                    let badge_tints_joined = nav_tints_joined(&p.badge_tints);
                    ffi::day_qt_navlist_set_items(
                        w,
                        cstr(&joined).as_ptr(),
                        cstr(&icons_joined).as_ptr(),
                        cstr(&tints_joined).as_ptr(),
                        cstr(&badge_icons_joined).as_ptr(),
                        cstr(&badge_tints_joined).as_ptr(),
                    );
                    navlist_apply_row_menus(w, &p.menus);
                    ffi::day_qt_navlist_set_selected(
                        w,
                        p.selected.map(|i| i as c_int).unwrap_or(-1),
                    );
                    NAV_MENU_ROWS.with(|m| m.borrow_mut().insert(w as usize, p.items.len()));
                    QtHandle(w)
                }
                Some(Builtin::Scroll) => {
                    let horizontal = props
                        .downcast_ref::<day_spec::props::ScrollProps>()
                        .map(|p| p.horizontal)
                        .unwrap_or(false);
                    QtHandle(ffi::day_qt_scroll_new(horizontal as c_int))
                }
                Some(Builtin::Label) => {
                    let Some(p) = props_of::<LabelProps>(kind, "qt", props) else {
                        return placeholder_handle(kind);
                    };
                    let w = ffi::day_qt_label_new(cstr(&p.text).as_ptr());
                    ffi::day_qt_label_set_align(
                        w,
                        match p.align {
                            day_spec::props::TextAlign::Center => 1,
                            day_spec::props::TextAlign::Trailing => 2,
                            day_spec::props::TextAlign::Leading => 0,
                        },
                    );
                    let (pt, weight, italic, tabular) = font_params(p.font);
                    ffi::day_qt_label_set_font(w, pt, weight, italic, tabular);
                    apply_custom_family(w, p.font);
                    if p.font.monospace {
                        ffi::day_qt_label_set_monospace(w);
                    }
                    if let Some(c) = p.color {
                        ffi::day_qt_label_set_color(w, c.r, c.g, c.b, c.a, 1);
                    }
                    remember_label_pt(w, p.font);
                    set_label_runs(w, &p.text, &p.runs);
                    // Wired unconditionally: runs can arrive later by patch, and connecting a
                    // signal on a label that never gets a link costs nothing.
                    ffi::day_qt_label_on_link(w, id.0, on_link);
                    QtHandle(w)
                }
                Some(Builtin::Button) => {
                    let Some(p) = props_of::<ButtonProps>(kind, "qt", props) else {
                        return placeholder_handle(kind);
                    };
                    let w = ffi::day_qt_button_new(cstr(&p.title).as_ptr(), id.0, on_press);
                    apply_button_style(w, p.style);
                    ffi::day_qt_enable_focus(w, id.0, on_focus);
                    QtHandle(w)
                }
                Some(Builtin::Toggle) => {
                    let Some(p) = props_of::<ToggleProps>(kind, "qt", props) else {
                        return placeholder_handle(kind);
                    };
                    let w = ffi::day_qt_checkbox_new(p.on as c_int, id.0, on_toggle);
                    ffi::day_qt_set_enabled(w, p.enabled as c_int);
                    ffi::day_qt_enable_focus(w, id.0, on_focus);
                    QtHandle(w)
                }
                Some(Builtin::Slider) => {
                    let Some(p) = props_of::<SliderProps>(kind, "qt", props) else {
                        return placeholder_handle(kind);
                    };
                    RANGES.with(|r| r.insert(id.0 as usize, (p.min, p.max)));
                    let w = ffi::day_qt_slider_new(
                        slider_ticks(p.value, p.min, p.max),
                        id.0,
                        on_slider,
                    );
                    RANGES_BY_PTR.with(|r| r.insert(w as usize, (p.min, p.max)));
                    ffi::day_qt_enable_focus(w, id.0, on_focus);
                    QtHandle(w)
                }
                Some(Builtin::Picker) => picker::realize_any(self, props, id),
                Some(Builtin::TextArea) => textarea::realize_any(self, props, id),
                Some(Builtin::TextField) => {
                    let Some(p) = props_of::<TextFieldProps>(kind, "qt", props) else {
                        return placeholder_handle(kind);
                    };
                    let w = ffi::day_qt_lineedit_new(
                        cstr(&p.text).as_ptr(),
                        cstr(&p.placeholder).as_ptr(),
                        id.0,
                        on_text,
                    );
                    ffi::day_qt_enable_focus(w, id.0, on_focus);
                    QtHandle(w)
                }
                Some(Builtin::Divider) => QtHandle(ffi::day_qt_separator_new()),
                Some(Builtin::Progress) => {
                    let Some(p) = props_of::<ProgressProps>(kind, "qt", props) else {
                        return placeholder_handle(kind);
                    };
                    match p.value {
                        Some(v) => QtHandle(ffi::day_qt_progress_new(1, progress_ticks(v))),
                        None => QtHandle(ffi::day_qt_progress_new(0, 0)),
                    }
                }
                Some(Builtin::Canvas) => {
                    let w = ffi::day_qt_canvas_new();
                    // Focus, and with it the keyboard (docs/menus.md, docs/focus.md): a canvas
                    // is the one built-in piece with no native control under it, so nothing
                    // else would ever make it the focus widget.
                    ffi::day_qt_enable_keys(w, id.0, on_key);
                    ffi::day_qt_enable_focus(w, id.0, on_focus);
                    QtHandle(w)
                }
                Some(Builtin::Image) => {
                    let Some(p) = props_of::<ImageProps>(kind, "qt", props) else {
                        return placeholder_handle(kind);
                    };
                    // Prefer the native Qt resource `:/day/images/<name>` (§18.3); else a loose file.
                    let res_path = format!(":/day/images/{}", p.source);
                    let path = if ffi::day_qt_resource_exists(cstr(&res_path).as_ptr()) != 0 {
                        res_path
                    } else {
                        // The staged glyph SVG first (docs/vectors.md), the raster cache after —
                        // `icon_file_path`'s rule, so the piece and the icon channels agree.
                        icon_file_path(&p.source)
                    };
                    // Scaling: 0=fit, 1=fill (crop), 2=stretch.
                    let mode = match p.content_mode {
                        ContentMode::Fit => 0,
                        ContentMode::Fill => 1,
                        ContentMode::Stretch => 2,
                    };
                    // Vector-glyph tint (docs/vectors.md): the same SourceIn recolor the nav
                    // rows use, over a glyph the SVG engine renders at size.
                    let tint = p.tint.map(hex_rgb).unwrap_or_default();
                    QtHandle(ffi::day_qt_image_new(
                        cstr(&path).as_ptr(),
                        mode,
                        cstr(&tint).as_ptr(),
                    ))
                }
                Some(Builtin::List) => {
                    let Some(p) = props_of::<ListProps>(kind, "qt", props) else {
                        return placeholder_handle(kind);
                    };
                    let host = ffi::day_qt_scroll_new(0);
                    let row_height = match p.row_height {
                        RowHeight::Uniform(h) => h,
                        RowHeight::Automatic => 44.0,
                    };
                    if p.reorderable {
                        // Native QDrag reorder (docs/list.md): the content widget accepts
                        // day-row drops; verdict + commit call back into the seam above.
                        let content = ffi::day_qt_scroll_content(host);
                        if !content.is_null() {
                            ffi::day_qt_list_enable_reorder(
                                content,
                                id.0,
                                row_height as c_int,
                                on_list_can_move,
                                on_list_move,
                            );
                        }
                    }
                    LIST_STATE.with(|m| {
                        m.borrow_mut().insert(
                            host as usize,
                            ListEntry {
                                host,
                                row_height,
                                source: Rc::new(RefCell::new(None)),
                                cells: Vec::new(),
                                bound: Vec::new(),
                                frame_height: -1,
                                fill_pending: false,
                                last_width: -1,
                                node: id.0,
                                selectable: p.selectable,
                                multi: p.multi_select,
                                reorderable: p.reorderable,
                                selected: Default::default(),
                                anchor: None,
                            },
                        )
                    });
                    LIST_BY_NODE.with(|m| m.borrow_mut().insert(id.0, host as usize));
                    // Rows are built as they scroll in, so the list has to hear about scrolling.
                    ffi::day_qt_list_on_scroll(host, id.0, on_list_scrolled);
                    QtHandle(host)
                }
                // A recycled list cell is ADOPTED from the native list, never realized
                // through this path; anything else is an extension piece.
                Some(Builtin::ListCell) | Some(Builtin::Tree) | None => {
                    if let Some(make) = self.registry.get(kind).map(|r| r.make) {
                        return make(self, props, id);
                    }
                    warn_missing_renderer(kind);
                    placeholder_handle(kind)
                }
            }
        }
    }

    fn update(
        &mut self,
        h: &QtHandle,
        kind: PieceKind,
        patch: &dyn std::any::Any,
        _anim: Option<&AnimSpec>,
    ) {
        unsafe {
            match kind {
                kinds::IMAGE => {
                    if let Some(day_spec::props::ImagePatch::Tint(c)) =
                        patch.downcast_ref::<day_spec::props::ImagePatch>()
                    {
                        let tint = c.map(hex_rgb).unwrap_or_default();
                        ffi::day_qt_image_set_tint(h.0, cstr(&tint).as_ptr());
                    }
                }
                kinds::CONTAINER => {
                    if let Some(ContainerPatch::Background(c)) =
                        patch.downcast_ref::<ContainerPatch>()
                    {
                        let bg = c.unwrap_or(day_spec::Color::CLEAR);
                        // Preserve the corner radius set at realize — a reactive `.background`
                        // must not square off a rounded surface.
                        ffi::day_qt_widget_set_bg(h.0, bg.r, bg.g, bg.b, bg.a);
                    }
                }
                kinds::NAV_MENU => {
                    if let Some(NavMenuPatch::Items {
                        items,
                        icons,
                        tints,
                        menus,
                        badge_icons,
                        badge_tints,
                        selected,
                        ..
                    }) = patch.downcast_ref::<NavMenuPatch>()
                    {
                        // Data-driven rows: rebuild the QListWidget from the new labels/icons
                        // (the same shim call realize uses), then apply the selection.
                        let joined = items.join("\u{1f}");
                        let icons_joined = icons
                            .iter()
                            .map(|ic| ic.as_deref().map(icon_file_path).unwrap_or_default())
                            .collect::<Vec<_>>()
                            .join("\u{1f}");
                        let tints_joined = nav_tints_joined(tints);
                        let badge_icons_joined = badge_icons
                            .iter()
                            .map(|ic| ic.as_deref().map(icon_file_path).unwrap_or_default())
                            .collect::<Vec<_>>()
                            .join("\u{1f}");
                        let badge_tints_joined = nav_tints_joined(badge_tints);
                        ffi::day_qt_navlist_set_items(
                            h.0,
                            cstr(&joined).as_ptr(),
                            cstr(&icons_joined).as_ptr(),
                            cstr(&tints_joined).as_ptr(),
                            cstr(&badge_icons_joined).as_ptr(),
                            cstr(&badge_tints_joined).as_ptr(),
                        );
                        navlist_apply_row_menus(h.0, menus);
                        ffi::day_qt_navlist_set_selected(
                            h.0,
                            selected.map(|i| i as c_int).unwrap_or(-1),
                        );
                        NAV_MENU_ROWS.with(|m| m.borrow_mut().insert(h.0 as usize, items.len()));
                    } else if let Some(NavMenuPatch::Selected(sel)) =
                        patch.downcast_ref::<NavMenuPatch>()
                    {
                        ffi::day_qt_navlist_set_selected(
                            h.0,
                            sel.map(|i| i as c_int).unwrap_or(-1),
                        );
                    }
                }
                // Emulated cover (docs/cover.md): present = re-home + raise + opaque default
                // surface (palette Window color); dismiss = hide + report `CoverHidden` at
                // once. No interactive dismissal exists on this backend.
                kinds::COVER => {
                    if let Some(p) = patch.downcast_ref::<CoverPatch>() {
                        let node = COVER_IDS
                            .with(|m| m.borrow().get(&(h.0 as usize)).copied())
                            .unwrap_or(day_spec::WINDOW_NODE);
                        match p {
                            CoverPatch::Present { background, .. } => {
                                if let Some(bg) = background {
                                    ffi::day_qt_widget_set_bg(h.0, bg.r, bg.g, bg.b, bg.a);
                                }
                                ffi::day_qt_add_child(self.window, h.0);
                                ffi::day_qt_cover_top(h.0);
                                let size = LAST_WINDOW_SIZE.with(|c| c.get());
                                ffi::day_qt_set_geometry(
                                    h.0,
                                    0,
                                    0,
                                    size.width as c_int,
                                    size.height as c_int,
                                );
                                ffi::day_qt_set_visible(h.0, 1);
                                COVERS.with(|c| c.borrow_mut().push((h.0, node)));
                                emit(node, Event::FrameChanged(size));
                            }
                            CoverPatch::DismissDisabled(_) => {}
                            CoverPatch::Dismiss => {
                                ffi::day_qt_set_visible(h.0, 0);
                                COVERS.with(|c| c.borrow_mut().retain(|(w, _)| *w != h.0));
                                emit(node, Event::CoverHidden);
                            }
                        }
                    }
                }
                kinds::INSPECTOR => {
                    if let Some(InspectorPatch::Visible(v)) = patch.downcast_ref::<InspectorPatch>()
                    {
                        INSPECTOR_STATE.with(|m| {
                            if let Some(state) = m.borrow_mut().get_mut(&(h.0 as usize)) {
                                state.shown = *v;
                                ffi::day_qt_set_visible(state.panel_pane, c_int::from(*v));
                            }
                        });
                        // Deferred one turn, like the nav pop path: hiding a QSplitter pane
                        // does not resize its sibling until Qt runs its own layout pass.
                        let host_addr = h.0 as usize;
                        <Qt as Platform>::post(Box::new(move || {
                            inspector_sync_panes(host_addr as *mut std::os::raw::c_void)
                        }));
                    }
                }
                kinds::NAV => {
                    if let Some(NavPatch::Presentation(next)) = patch.downcast_ref::<NavPatch>() {
                        // A rail lands on the sidebar this backend does have: nothing in Qt is a
                        // vertical strip of icon-only destinations, and `NavPresentation::Rail` is
                        // ROUNDABLE for exactly that case (docs/size-classes.md) — the same call
                        // AppKit makes. What it shares with a split is the shape that matters
                        // here: rows beside the content.
                        let beside =
                            next.is_split() || *next == day_spec::props::NavPresentation::Rail;
                        nav_present(h.0, beside);
                        return;
                    }
                    if let Some(p) = patch.downcast_ref::<NavPatch>() {
                        NAV_STATE.with(|m| {
                            let mut m = m.borrow_mut();
                            let Some(state) = m.get_mut(&(h.0 as usize)) else {
                                return;
                            };
                            // Split: the sidebar's page is not part of the detail stack.
                            // Stack: every page participates, its root included.
                            let split_now = state.split;
                            let detail: Vec<(QtHandle, NodeId)> = state
                                .pages
                                .iter()
                                .filter(|(p, _)| !(split_now && is_sidebar_page(p.0)))
                                .copied()
                                .collect();
                            let detail = &detail[..];
                            match p {
                                NavPatch::Pushed { title, .. } => {
                                    let last = detail.len().saturating_sub(1);
                                    for (i, (page, _)) in detail.iter().enumerate() {
                                        ffi::day_qt_set_visible(page.0, (i == last) as _);
                                    }
                                    state.titles.push(title.clone());
                                }
                                NavPatch::Popped => {
                                    let n = detail.len();
                                    if let Some((top, _)) = detail.last() {
                                        ffi::day_qt_set_visible(top.0, 0);
                                    }
                                    if n >= 2 {
                                        ffi::day_qt_set_visible(detail[n - 2].0.0, 1);
                                    }
                                    if state.titles.len() > 1 {
                                        state.titles.pop();
                                    }
                                }
                                NavPatch::Title(t) => {
                                    if let Some(last) = state.titles.last_mut() {
                                        *last = t.clone();
                                    }
                                }
                                // Custom back header → always NavBack{already_popped:false};
                                // no native auto-pop to suppress (docs/navigation.md).
                                NavPatch::GuardTop(_) => {}
                                // Handled before this borrow (it re-parents widgets).
                                NavPatch::Presentation(_) => {}
                                // The resident-page switch (docs/navigation.md): the app moved the
                                // selection, so the suite shows that destination. Tab 0 is the
                                // hidden sidebar page, so destination i is tab i + 1.
                                NavPatch::Select(i) => {
                                    // The resident-page switch (docs/navigation.md). A suite
                                    // drives its tab widget; a split — which is also what a
                                    // ROUNDED `Rail` lands on here, Qt having no rail widget —
                                    // shows destination `i` and hides its siblings. Same
                                    // contract, different chrome.
                                    if NAV_SUITES.with(|m| m.borrow().contains_key(&(h.0 as usize)))
                                    {
                                        // Tab 0 is the hidden sidebar page.
                                        ffi::day_qt_tabs_set_current(h.0, (*i + 1) as c_int);
                                    } else {
                                        for (n, (page, _)) in detail.iter().enumerate() {
                                            ffi::day_qt_set_visible(page.0, c_int::from(n == *i));
                                        }
                                    }
                                }
                            }
                        });
                        // Header visibility follows the depth AFTER the pop completes (the
                        // popped page leaves `pages` via remove()); defer one turn so the
                        // header + page sizes settle against the final stack. The raw QWidget
                        // pointer crosses the (main-thread-only) post as usize.
                        let host = h.0 as usize;
                        <Qt as Platform>::post(Box::new(move || {
                            nav_sync_header(host as *mut std::os::raw::c_void)
                        }));
                    }
                }
                kinds::LABEL => {
                    if let Some(p) = patch.downcast_ref::<LabelPatch>() {
                        match p {
                            LabelPatch::Text(t) => {
                                ffi::day_qt_label_set_text(h.0, cstr(t).as_ptr())
                            }
                            LabelPatch::Font(f) => {
                                let (pt, weight, italic, tabular) = font_params(*f);
                                ffi::day_qt_label_set_font(h.0, pt, weight, italic, tabular);
                                apply_custom_family(h.0, *f);
                                remember_label_pt(h.0, *f);
                            }
                            LabelPatch::Runs(text, runs) => set_label_runs(h.0, text, runs),
                            LabelPatch::Color(c) => match c {
                                Some(c) => ffi::day_qt_label_set_color(h.0, c.r, c.g, c.b, c.a, 1),
                                None => ffi::day_qt_label_set_color(h.0, 0.0, 0.0, 0.0, 0.0, 0),
                            },
                        }
                    }
                }
                kinds::BUTTON => {
                    if let Some(p) = patch.downcast_ref::<ButtonPatch>() {
                        match p {
                            ButtonPatch::Title(t) => {
                                ffi::day_qt_button_set_title(h.0, cstr(t).as_ptr())
                            }
                            ButtonPatch::Enabled(e) => ffi::day_qt_set_enabled(h.0, *e as c_int),
                            ButtonPatch::Style(s) => apply_button_style(h.0, *s),
                        }
                    }
                }
                kinds::TOGGLE => {
                    if let Some(p) = patch.downcast_ref::<TogglePatch>() {
                        match p {
                            TogglePatch::On(on) => ffi::day_qt_checkbox_set(h.0, *on as c_int),
                            TogglePatch::Enabled(e) => ffi::day_qt_set_enabled(h.0, *e as c_int),
                        }
                    }
                }
                kinds::SLIDER => {
                    if let Some(p) = patch.downcast_ref::<SliderPatch>() {
                        match p {
                            SliderPatch::Value(v) => {
                                let (min, max) = RANGES_BY_PTR
                                    .with(|r| r.get(h.0 as usize))
                                    .unwrap_or((0.0, 1.0));
                                ffi::day_qt_slider_set(h.0, slider_ticks(*v, min, max));
                            }
                            SliderPatch::Enabled(e) => ffi::day_qt_set_enabled(h.0, *e as c_int),
                        }
                    }
                }
                kinds::PROGRESS => {
                    if let Some(ProgressPatch::Value(Some(v))) =
                        patch.downcast_ref::<ProgressPatch>()
                    {
                        ffi::day_qt_progress_set(h.0, progress_ticks(*v));
                    }
                }
                kinds::PICKER => picker::update_any(self, h, patch),
                kinds::TEXT_AREA => textarea::update_any(self, h, patch),
                kinds::TEXT_FIELD => {
                    if let Some(p) = patch.downcast_ref::<TextFieldPatch>() {
                        match p {
                            TextFieldPatch::Text { text, from_native } => {
                                if !*from_native {
                                    ffi::day_qt_lineedit_set_text(h.0, cstr(text).as_ptr());
                                }
                            }
                            TextFieldPatch::Placeholder(t) => {
                                ffi::day_qt_lineedit_set_placeholder(h.0, cstr(t).as_ptr())
                            }
                            TextFieldPatch::Enabled(e) => ffi::day_qt_set_enabled(h.0, *e as c_int),
                        }
                    }
                }
                kinds::LIST => match patch.downcast_ref::<ListPatch>() {
                    Some(ListPatch::Reload) | Some(ListPatch::Splice(_)) => {
                        schedule_list_populate(h.0 as usize)
                    }
                    Some(ListPatch::ScrollToEnd) => schedule_list_scroll_end(h.0 as usize),
                    Some(ListPatch::ScrollToRow(row)) => {
                        schedule_list_scroll_row(h.0 as usize, *row)
                    }
                    Some(ListPatch::Selected(rows)) => {
                        // Programmatic selection sync (empty = clear): repaint, no re-emit.
                        LIST_STATE.with(|m| {
                            if let Some(st) = m.borrow_mut().get_mut(&(h.0 as usize)) {
                                st.selected = rows.iter().copied().collect();
                                st.anchor = rows.last().copied();
                                list_paint_selection(st);
                            }
                        });
                    }
                    // Not implemented: RowSizeInvalidated — the emulated list re-lays out every
                    // row on the next Reload.
                    Some(ListPatch::RowSizeInvalidated(_)) | None => {}
                },
                _ => {
                    if let Some(update) = self.registry.get(kind).map(|r| r.update) {
                        update(self, h, patch);
                    }
                }
            }
        }
    }

    /// Offer a satellite piece its teardown hook before `release` frees the handle (§15.2).
    fn release_piece(&mut self, kind: day_spec::PieceKind, h: &Self::Handle) {
        // Copy the fn pointer out first: the registry lookup borrows `self` immutably and
        // the hook needs it mutably.
        let f = self.registry.get(kind).and_then(|r| r.release);
        if let Some(f) = f {
            f(self, h);
        }
    }
    fn release(&mut self, h: QtHandle) {
        // A released window content = that window is gone (docs/windows.md teardown):
        // NOW destroy the whole DayWindow (deleteLater — after this event dispatch), never
        // before, so day's child-widget releases can't touch freed memory.
        self.secondary.retain(|w| {
            if w.content == h.0 {
                unsafe { ffi::day_qt_window_destroy(w.win) };
                false
            } else {
                true
            }
        });
        let key = h.0 as usize;
        if let Some(entry) = LIST_STATE.with(|m| m.borrow_mut().remove(&key)) {
            LIST_BY_NODE.with(|m| {
                m.borrow_mut().remove(&entry.node);
            });
        }
        // A disposed nav host / page MUST drop its NAV_STATE / NAV_PAGE_IDS entry — otherwise a
        // later widget that reuses the freed address is mistaken for a nav host in `set_frame`,
        // and `nav_sync_panes` reads its freed panes (a use-after-free SIGSEGV).
        NAV_STATE.with(|m| {
            m.borrow_mut().remove(&key);
        });
        NAV_PAGE_IDS.with(|m| {
            m.borrow_mut().remove(&key);
        });
        NAV_MENU_ROWS.with(|m| {
            m.borrow_mut().remove(&key);
        });
        // The same use-after-free rule for a disposed inspector split and its panes.
        INSPECTOR_STATE.with(|m| {
            m.borrow_mut().remove(&key);
        });
        INSPECTOR_PANE_IDS.with(|m| {
            m.borrow_mut().remove(&key);
        });
        // Same rule for the navigation suite's three tables: a freed QTabWidget whose address is
        // reused would otherwise inherit the old suite's destination pages, and report sizes to
        // node ids that no longer exist.
        NAV_SUITES.with(|m| {
            m.borrow_mut().remove(&key);
        });
        NAV_SUITE_ROWS.with(|m| {
            m.borrow_mut().remove(&key);
        });
        NAV_SUITE_PAGES.with(|m| {
            m.borrow_mut().remove(&key);
        });
        GESTURES.with(|g| {
            g.borrow_mut().retain(|(ptr, _)| *ptr != key);
        });
        // A cover torn down while presented (no Dismiss patch first) must leave the
        // presented set — `window_resized` writes through every entry's raw pointer, and a
        // stale one is a use-after-free once `day_qt_delete` runs below.
        COVERS.with(|c| c.borrow_mut().retain(|(w, _)| *w != h.0));
        COVER_IDS.with(|m| {
            m.borrow_mut().remove(&key);
        });
        // ONE sweep drops this handle from EVERY SideTable registered on this thread — the
        // slider ranges, the tab icons, the textarea line bands, and any table added later —
        // so a widget recycling the freed address can't inherit the dead widget's entries.
        // (The explicit purges above stay: they key by node id / value pairs, not this address.)
        day_spec::sidetable::sweep(key);
        unsafe {
            ffi::day_qt_remove_child(h.0);
            ffi::day_qt_delete(h.0);
        }
    }

    fn insert(&mut self, parent: &QtHandle, child: &QtHandle, index: usize) {
        // Navigation suite: every destination becomes a tab. The page at index 0 is the SIDEBAR
        // page, whose rows became the bar — it stays a tab so its nav menu keeps a parent chain
        // up to the suite, but a hidden one, because drawing the rows again as a list would be
        // the same navigation twice.
        let suite_handled = NAV_SUITES.with(|m| {
            let mut m = m.borrow_mut();
            let Some(suite) = m.get_mut(&(parent.0 as usize)) else {
                return false;
            };
            unsafe {
                ffi::day_qt_tabs_add_page(parent.0, child.0, cstr("").as_ptr(), index as c_int);
                if index == 0 {
                    ffi::day_qt_tabs_set_page_visible(parent.0, child.0, 0);
                } else {
                    let id = NAV_PAGE_IDS
                        .with(|ids| ids.borrow().get(&(child.0 as usize)).copied())
                        .unwrap_or(NodeId(0));
                    suite.pages.push((*child, id));
                    // Hiding tab 0 does not move off it — QTabWidget keeps showing whatever the
                    // current index points at, so the FIRST destination has to claim it or the
                    // content area stays on the sidebar page nobody is meant to see.
                    if suite.pages.len() == 1 {
                        ffi::day_qt_tabs_set_current(parent.0, 1);
                    }
                }
                NAV_SUITE_PAGES.with(|m| m.borrow_mut().insert(child.0 as usize));
            }
            true
        });
        if suite_handled {
            suite_apply_rows(parent.0);
            nav_suite_sync(parent.0);
            return;
        }
        // A nav menu that has just gained a parent chain: if a suite is above it, its rows are
        // that suite's bar. Labels and glyphs both, in row order.
        if let Some((node, titles, icons)) =
            NAV_SUITE_ROWS.with(|m| m.borrow().get(&(child.0 as usize)).cloned())
        {
            let suite = unsafe { ffi::day_qt_enclosing_tabs(parent.0) };
            if !suite.is_null() {
                NAV_SUITES.with(|m| {
                    if let Some(s) = m.borrow_mut().get_mut(&(suite as usize)) {
                        s.menu_node = node;
                        s.titles = titles;
                        s.icons = icons;
                    }
                });
                suite_apply_rows(suite);
            }
        }
        // Nav host: pages land by their PANE, not their position (docs/size-classes.md).
        let sidebar = is_sidebar_page(child.0);
        let handled = NAV_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let Some(state) = m.get_mut(&(parent.0 as usize)) else {
                return false;
            };
            let id = NAV_PAGE_IDS
                .with(|ids| ids.borrow().get(&(child.0 as usize)).copied())
                .unwrap_or(NodeId(0));
            if sidebar {
                state.sidebar_page = Some(*child);
            }
            let pane = if state.split && sidebar {
                state.sidebar_pane
            } else {
                state.detail_pane
            };
            unsafe { ffi::day_qt_add_child(pane, child.0) };
            state.pages.push((*child, id));
            true
        });
        if handled {
            nav_sync_panes(parent.0);
            return;
        }
        // Inspector host: the pane's own record says which side it is (docs/inspector.md).
        let inspected = INSPECTOR_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let Some(state) = m.get_mut(&(parent.0 as usize)) else {
                return false;
            };
            let (pane_id, panel) = INSPECTOR_PANE_IDS
                .with(|ids| ids.borrow().get(&(child.0 as usize)).copied())
                .unwrap_or((NodeId(0), index == 1));
            let pane = if panel {
                state.panel_pane
            } else {
                state.content_pane
            };
            unsafe { ffi::day_qt_add_child(pane, child.0) };
            state.panes.push((pane_id, panel));
            true
        });
        if inspected {
            inspector_sync_panes(parent.0);
        } else {
            unsafe { ffi::day_qt_add_child(content_of(parent), child.0) };
        }
    }

    fn remove(&mut self, parent: &QtHandle, child: &QtHandle) {
        PAGE_PANE.with(|m| m.borrow_mut().remove(&(child.0 as usize)));
        NAV_STATE.with(|m| {
            if let Some(state) = m.borrow_mut().get_mut(&(parent.0 as usize)) {
                state.pages.retain(|(p, _)| p.0 != child.0);
                if state.sidebar_page.map(|p| p.0) == Some(child.0) {
                    state.sidebar_page = None;
                }
            }
        });
        // A suite destination leaving: drop its tab too (removeTab keeps the widget, which
        // Day owns and disposes).
        let in_suite = NAV_SUITES.with(|m| {
            if let Some(suite) = m.borrow_mut().get_mut(&(parent.0 as usize)) {
                suite.pages.retain(|(p, _)| p.0 != child.0);
                true
            } else {
                false
            }
        });
        if in_suite {
            unsafe { ffi::day_qt_tabs_remove_page(parent.0, child.0) };
        }
        unsafe { ffi::day_qt_remove_child(child.0) };
    }

    fn move_child(&mut self, _parent: &QtHandle, _child: &QtHandle, _to: usize) {}

    fn measure(&mut self, h: &QtHandle, kind: PieceKind, p: Proposal) -> Size {
        let mut w = 0.0;
        let mut hh = 0.0;
        unsafe { ffi::day_qt_size_hint(h.0, &mut w, &mut hh) };
        match kind {
            kinds::NAV_MENU => {
                let rows =
                    NAV_MENU_ROWS.with(|m| m.borrow().get(&(h.0 as usize)).copied().unwrap_or(0));
                Size::new(
                    p.width.unwrap_or(220.0),
                    p.height.unwrap_or(rows as f64 * 34.0 + 8.0),
                )
            }
            kinds::LABEL => {
                // Natural width from font metrics — QLabel::sizeHint() suggests a narrow
                // "readable column" for word-wrapped labels, which is not day's contract
                // (natural = unwrapped). Height always via heightForWidth at the width day
                // actually grants; sizeHint's heuristic height is never mixed in.
                let nat_w = unsafe { ffi::day_qt_label_natural_width(h.0) } as f64;
                let width = match p.width {
                    Some(pw) => nat_w.min(pw),
                    None => nat_w,
                };
                let hfw = unsafe {
                    ffi::day_qt_label_height_for_width(h.0, width.round().max(1.0) as c_int)
                };
                if hfw > 0 {
                    Size::new(width.ceil(), hfw as f64)
                } else {
                    Size::new(width.ceil(), hh.ceil())
                }
            }
            // Buttons hug their text (sizeHint = content + chrome) like every other
            // toolkit: the generic arm would swallow a COLUMN's cross-axis width proposal
            // and stretch the button across the full content span.
            kinds::BUTTON => Size::new(w.ceil(), hh.ceil()),
            kinds::SLIDER => Size::new(p.width.unwrap_or(180.0), hh.max(20.0)),
            kinds::PICKER => picker::measure_any(self, h, p),
            kinds::TEXT_AREA => textarea::measure_any(self, h, p),
            kinds::TEXT_FIELD => Size::new(p.width.unwrap_or(180.0), hh.max(24.0)),
            kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 2.0),
            kinds::PROGRESS => Size::new(p.width.unwrap_or(180.0), hh.max(16.0)),
            kinds::LIST => Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0)),
            _ => {
                if let Some(measure) = self.registry.get(kind).and_then(|r| r.measure) {
                    measure(self, h, p)
                } else {
                    Size::new(p.width.unwrap_or(w), p.height.unwrap_or(hh))
                }
            }
        }
    }

    fn set_opacity(&mut self, h: &QtHandle, opacity: f64, anim: Option<&AnimSpec>) {
        // Opacity + transform share one custom QGraphicsEffect per widget (see the shim).
        let (dur, curve) = qt_anim_args(anim);
        unsafe { ffi::day_qt_set_opacity(h.0, opacity, dur, curve) };
    }

    fn set_transform(&mut self, h: &QtHandle, t: Transform, _size: Size, anim: Option<&AnimSpec>) {
        // The effect re-draws the widget through a painter transformed about its center, spilling
        // beyond the widget's own frame (§8.4) so a moved/scaled/rotated box isn't clipped.
        let (dur, curve) = qt_anim_args(anim);
        unsafe { ffi::day_qt_set_transform(h.0, t.tx, t.ty, t.sx, t.sy, t.rotate_deg, dur, curve) };
    }

    fn set_selectable(&mut self, h: &QtHandle, selectable: bool) -> Option<QtHandle> {
        // The shim qobject_casts to QLabel, so a non-label handle is a safe no-op (docs/text.md).
        unsafe { ffi::day_qt_label_set_selectable(h.0, selectable as c_int) };
        None
    }

    /// Derived from the widget's own font metrics in the shim (docs/baseline.md): Qt has no
    /// baseline protocol to ask — `QFormLayout` aligns boxes, not text — so `Cap` reports this
    /// as `Emulated`.
    fn first_baseline(&mut self, h: &QtHandle, kind: PieceKind, size: Size) -> Option<f64> {
        if !day_spec::kind_has_baseline(kind) {
            return None;
        }
        let b = unsafe { ffi::day_qt_baseline(h.0, size.height) };
        (b >= 0.0).then_some(b)
    }

    fn set_frame(&mut self, h: &QtHandle, frame: Rect, _anim: Option<&AnimSpec>) {
        unsafe {
            ffi::day_qt_set_geometry(
                h.0,
                frame.origin.x.round() as c_int,
                frame.origin.y.round() as c_int,
                frame.size.width.round() as c_int,
                frame.size.height.round() as c_int,
            )
        };
        // Nav host resized (window resize): re-report page sizes for relayout.
        if NAV_STATE.with(|m| m.borrow().contains_key(&(h.0 as usize))) {
            nav_sync_panes(h.0);
        }
        if NAV_SUITES.with(|m| m.borrow().contains_key(&(h.0 as usize))) {
            nav_suite_sync(h.0);
        }
        // Same for an inspector split: its pane frames are native-owned too.
        if INSPECTOR_STATE.with(|m| m.borrow().contains_key(&(h.0 as usize))) {
            inspector_sync_panes(h.0);
        }
        // List host framed: (re)fill its cells — but ONLY when the width actually changed, so the
        // set_frame calls a populate itself makes (on row content) don't schedule another forever.
        let width_changed = LIST_STATE.with(|m| {
            m.borrow()
                .get(&(h.0 as usize))
                .map(|st| st.last_width != frame.size.width.round() as c_int)
                .unwrap_or(false)
        });
        // A taller host shows MORE rows, and the ones it grew into are not built yet — so a
        // height change refills the window even though every built row is still valid. Width is
        // the one that invalidates content: each row is laid out to it.
        let framed_h = frame.size.height.round() as c_int;
        let height_changed = LIST_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let Some(st) = m.get_mut(&(h.0 as usize)) else {
                return false;
            };
            let changed = st.frame_height != framed_h;
            st.frame_height = framed_h;
            changed
        });
        if width_changed {
            schedule_list_populate(h.0 as usize);
        } else if height_changed {
            schedule_list_fill(h.0 as usize);
        }
    }

    fn set_scroll_content(&mut self, h: &QtHandle, content: Size) {
        unsafe {
            ffi::day_qt_scroll_set_content_size(
                h.0,
                content.width.round() as c_int,
                content.height.round() as c_int,
            )
        };
    }

    fn scroll_to(&mut self, h: &QtHandle, target: Rect, _animated: bool) {
        // Qt scroll bars have no built-in animation; jumps are immediate (dayscript-friendly).
        unsafe {
            ffi::day_qt_scroll_to_rect(
                h.0,
                target.origin.x.round() as c_int,
                target.origin.y.round() as c_int,
                target.size.width.round() as c_int,
                target.size.height.round() as c_int,
            )
        };
    }

    fn present(&mut self, req: u64, spec: &day_spec::present::PresentSpec) {
        use day_spec::present::PresentSpec;
        let parent = self.present_parent();
        match spec {
            PresentSpec::Dialog { .. } => unsafe {
                ffi::day_qt_present_dialog(
                    req,
                    cstr(spec.title()).as_ptr(),
                    cstr(spec.message().unwrap_or("")).as_ptr(),
                    cstr(&spec.buttons_joined()).as_ptr(),
                    cstr(&spec.roles_joined()).as_ptr(),
                    parent,
                )
            },
            PresentSpec::Prompt {
                placeholder,
                initial,
                ok,
                cancel,
                ..
            } => unsafe {
                ffi::day_qt_present_prompt(
                    req,
                    cstr(spec.title()).as_ptr(),
                    cstr(spec.message().unwrap_or("")).as_ptr(),
                    cstr(placeholder).as_ptr(),
                    cstr(initial).as_ptr(),
                    cstr(ok).as_ptr(),
                    cstr(cancel).as_ptr(),
                    parent,
                )
            },
            PresentSpec::OpenFile { .. } => unsafe {
                ffi::day_qt_present_file_open(
                    req,
                    cstr(spec.title()).as_ptr(),
                    cstr(&spec.filters_joined()).as_ptr(),
                    parent,
                )
            },
            PresentSpec::SaveFile { suggested_name, .. } => unsafe {
                // The pieces layer copies the staged bytes to the chosen path.
                ffi::day_qt_present_file_save(
                    req,
                    cstr(spec.title()).as_ptr(),
                    cstr(suggested_name).as_ptr(),
                    cstr(&spec.filters_joined()).as_ptr(),
                    parent,
                )
            },
        }
    }

    fn dismiss(&mut self, req: u64) {
        unsafe { ffi::day_qt_dismiss_present(req) };
    }

    fn open_url(&mut self, url: &str) {
        if let Ok(c) = std::ffi::CString::new(url) {
            unsafe { ffi::day_qt_open_url(c.as_ptr()) };
        }
    }

    fn focus(&mut self, h: &QtHandle, _node: NodeId, focused: bool) {
        // The shim clears only while this widget still owns focus, so a stale release
        // can't blur a sibling.
        unsafe { ffi::day_qt_widget_focus(h.0, focused as c_int) };
    }

    fn set_event_sink(&mut self, sink: EventSink) {
        SINK.with(|s| *s.borrow_mut() = Some(Rc::from(sink)));
    }

    fn enable_gesture(&mut self, h: &QtHandle, node: NodeId, kind: day_spec::GestureKind) {
        let key = (h.0 as usize, kind);
        if !GESTURES.with(|g| g.borrow_mut().insert(key)) {
            return; // already wired
        }
        use day_spec::GestureKind as K;
        let code: c_int = match kind {
            K::Tap | K::LongPress => 0,
            K::Drag => 1,
            K::Pinch => 2,
            K::Pan => 3,
        };
        unsafe { ffi::day_qt_enable_gesture(h.0, node.0, code, on_gesture) };
    }

    fn set_toolbar(&mut self, h: &QtHandle, items: &[day_spec::ToolbarItem]) {
        self.install_toolbar(h, items);
    }

    fn update_toolbar(&mut self, h: &QtHandle, patch: &day_spec::ToolbarPatch) {
        self.patch_toolbar(h, patch);
    }

    fn set_app_menu(&mut self, items: &[day_spec::MenuItem]) {
        if self.window.is_null() {
            return;
        }
        // KDE's bar: File, Edit, View, the app's own menus, then Settings and Help.
        let items = day_core::menu::standard_menu_bar_for(
            day_core::menu::MenuBarStyle::Kde,
            items.to_vec(),
        );
        let items = &items[..];
        let bar = unsafe { ffi::day_qt_window_menubar(self.window) };
        // Top level entries are the menu-bar menus; a bare action becomes a single-item menu.
        for item in items {
            match item {
                day_spec::MenuItem::Submenu { label, items, .. } => {
                    let menu = unsafe { ffi::day_qt_menubar_add_menu(bar, cstr(label).as_ptr()) };
                    build_qt_menu(menu, items);
                }
                other => {
                    let menu = unsafe { ffi::day_qt_menubar_add_menu(bar, cstr("").as_ptr()) };
                    build_qt_menu(menu, std::slice::from_ref(other));
                }
            }
        }
        // An in-window bar (Linux/Windows) now has its real height: reserve its strip so the
        // content area — and day's layout — sit below it instead of underneath it.
        unsafe { ffi::day_qt_window_menubar_done(self.window) };
    }

    fn modifiers(&mut self) -> day_spec::Modifiers {
        let m = unsafe { ffi::day_qt_modifiers() };
        day_spec::Modifiers {
            shift: m & i32::from(day_spec::KeyEvent::SHIFT) != 0,
            primary: m & i32::from(day_spec::KeyEvent::PRIMARY) != 0,
            alt: m & i32::from(day_spec::KeyEvent::ALT) != 0,
        }
    }

    fn set_context_menu_fn(&mut self, h: &QtHandle, node: NodeId, f: day_spec::ContextMenuFn) {
        CTX_MENU_FNS.with(|m| m.borrow_mut().insert(node.0, f));
        unsafe { ffi::day_qt_context_menu_fn(h.0, node.0, on_context_menu_summon) };
    }

    fn set_context_menu(&mut self, h: &QtHandle, _node: NodeId, items: &[day_spec::MenuItem]) {
        if items.is_empty() {
            unsafe { ffi::day_qt_set_context_menu(h.0, std::ptr::null_mut()) };
            return;
        }
        let menu = unsafe { ffi::day_qt_menu_new() };
        build_qt_menu(menu, items);
        unsafe { ffi::day_qt_set_context_menu(h.0, menu) };
    }

    fn attach_list(&mut self, host: &QtHandle, source: day_spec::ListSource) {
        LIST_STATE.with(|m| {
            if let Some(st) = m.borrow().get(&(host.0 as usize)) {
                *st.source.borrow_mut() = Some(source);
            }
        });
        // Deferred (see schedule_list_populate): populating re-enters with_tree via bind_row.
        schedule_list_populate(host.0 as usize);
    }

    fn adopt(&mut self, raw: day_spec::RawHandle) -> QtHandle {
        // A recycling list cell (a plain QWidget) — Day fills/rebinds its row content in place.
        QtHandle(raw)
    }

    fn set_a11y(&mut self, h: &QtHandle, a11y: &A11yProps) {
        unsafe {
            if let Some(id) = &a11y.identifier {
                ffi::day_qt_set_object_name(h.0, cstr(id).as_ptr());
            }
            if let Some(label) = &a11y.label {
                // Real QAccessible name (screen readers) + a visible tooltip.
                ffi::day_qt_set_accessible_name(h.0, cstr(label).as_ptr());
                ffi::day_qt_set_tooltip(h.0, cstr(label).as_ptr());
            }
            if let Some(hint) = &a11y.hint {
                ffi::day_qt_set_accessible_description(h.0, cstr(hint).as_ptr());
            }
            // Qt derives role/value from the widget type (QAccessibleInterface); Day sets the
            // text fields it can. `hidden`/canvas roles need a QAccessible subclass (follow-up).
        }
    }

    fn replay(&mut self, h: &QtHandle, ops: &[DrawOp], _size: Size) {
        let (nums, texts) = day_spec::encode_ops(ops);
        let joined = cstr(&texts.join("\u{1f}"));
        unsafe {
            ffi::day_qt_canvas_set_ops(h.0, nums.as_ptr(), nums.len() as c_int, joined.as_ptr())
        };
    }

    fn dark_mode(&mut self) -> bool {
        // Ask Qt, not the `DAY_THEME` default: Qt Widgets follow the system scheme on their
        // own, so the default's "light unless DAY_THEME says otherwise" left app-painted
        // surfaces light while every native control around them rendered dark.
        // SAFETY: a plain palette read; no arguments and no pointers cross the boundary.
        unsafe { ffi::day_qt_dark_mode() != 0 }
    }

    fn toggle_sidebar(&mut self) -> bool {
        crate::toggle_sidebar()
    }
    fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
        if self.window.is_null() {
            return Err("no window".into());
        }
        snapshot_qt_widget(self.window)
    }

    fn snapshot_window_of(&mut self, host: &QtHandle) -> Result<Vec<u8>, String> {
        snapshot_qt_widget(host.0)
    }

    fn open_window(
        &mut self,
        id: NodeId,
        options: &WindowOptions,
        kind: day_spec::WindowKind,
    ) -> day_spec::WindowOpenReply<QtHandle> {
        let fixed = (kind == day_spec::WindowKind::Preferences) as c_int;
        let win = unsafe {
            ffi::day_qt_window_new2(
                cstr(&options.title).as_ptr(),
                options.size.width as c_int,
                options.size.height as c_int,
                id.0,
                fixed,
            )
        };
        unsafe { ffi::day_qt_window_show(win) };
        let content = unsafe { ffi::day_qt_window_content(win) };
        self.secondary.push(QtWin { win, content });
        day_spec::WindowOpenReply::Open(QtHandle(content))
    }

    fn close_window(&mut self, host: &QtHandle) {
        if let Some(w) = self.secondary.iter().find(|w| w.content == host.0) {
            // closeEvent confirms with WindowClosed; the window HIDES — destruction is
            // anchored to day-core releasing the content handle (`release`, below).
            unsafe { ffi::day_qt_window_close(w.win) };
        }
    }

    fn focus_window(&mut self, host: &QtHandle) {
        if let Some(w) = self.secondary.iter().find(|w| w.content == host.0) {
            unsafe { ffi::day_qt_window_raise(w.win) };
        }
    }

    fn set_window_title(&mut self, host: &QtHandle, title: &str) {
        if let Some(w) = self.secondary.iter().find(|w| w.content == host.0) {
            unsafe { ffi::day_qt_window_set_title(w.win, cstr(title).as_ptr()) };
        }
    }
}

/// Grab any widget to PNG through the shim (the dayscript screenshot seam).
fn snapshot_qt_widget(widget: *mut c_void) -> Result<Vec<u8>, String> {
    let path = std::env::temp_dir().join(format!("day-qt-snap-{}.png", std::process::id()));
    let cpath = cstr(path.to_str().unwrap_or("/tmp/day-qt-snap.png"));
    let rc = unsafe { ffi::day_qt_snapshot_png(widget, cpath.as_ptr()) };
    if rc != 0 {
        return Err("grab failed".into());
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&path);
    Ok(bytes)
}

extern "C" fn run_posted(data: *mut c_void) {
    // The posted-closure trampoline runs arbitrary Rust (deferred emits, list populates) —
    // contained like every other FFI entry (day-spec's ffi_guard).
    let f: Box<Box<dyn FnOnce() + Send>> = unsafe { Box::from_raw(data as *mut _) };
    ffi_guard::contain((), f);
}

impl Platform for Qt {
    const TARGET: &'static str = if cfg!(target_os = "macos") {
        "macos-qt"
    } else if cfg!(target_os = "windows") {
        "windows-qt"
    } else {
        "linux-qt"
    };
    const TOOLKIT: &'static str = "qt";

    fn run(mut self, options: WindowOptions, ready: Box<dyn FnOnce(Self, QtHandle, Size)>) {
        unsafe {
            // The macOS app menu (Quit/Hide) takes its name from argv[0] at QApplication
            // construction, so pass the app's display name ("Showcase", falling back to the window
            // title) into `day_qt_app_new` up front.
            let app_name = options.app_name.as_deref().unwrap_or(&options.title);
            let app = ffi::day_qt_app_new(cstr(app_name).as_ptr());
            // RTL locales (docs/localization): mirror native widget internals app-wide;
            // Day's own frames mirror in the layout engine.
            if day_core::layout_direction() == day_spec::LayoutDirection::Rtl {
                ffi::day_qt_app_set_rtl();
            }
            // Bundled custom fonts (§18.4): register with the QFontDatabase (needs the
            // QApplication above) before the first label realizes.
            for path in day_spec::fonts::bundled_fonts() {
                if ffi::day_qt_register_font(cstr(&path.to_string_lossy()).as_ptr()) < 0 {
                    log::warn!("could not register bundled font {}", path.display());
                }
            }
            // App icon (§18.2): Dock on macOS, taskbar on Linux/Windows (set by `day launch`).
            if let Ok(icon) = std::env::var("DAY_APP_ICON") {
                ffi::day_qt_set_app_icon(cstr(&icon).as_ptr());
            }
            let window = ffi::day_qt_window_new(
                cstr(&options.title).as_ptr(),
                options.size.width as c_int,
                options.size.height as c_int,
            );
            self.window = window;
            ffi::day_qt_set_present_cb(present_cb);
            ffi::day_qt_set_menu_cb(on_menu_action);
            ffi::day_qt_set_toolbar_cb(toolbar::on_toolbar_value);
            ffi::day_qt_set_lifecycle_cb(on_lifecycle);
            ready(self, QtHandle(window), options.size);
            ffi::day_qt_window_on_resize(window, window_resized);
            ffi::day_qt_set_window_events_cb(win_resized, win_closed, win_focused);
            ffi::day_qt_window_show(window);
            ffi::day_qt_app_run(app);
        }
    }

    fn locale_hints(&self) -> Vec<String> {
        // No desktop-wide API here that beats the environment every session sets
        // (§12.2, docs/localization.md).
        day_spec::posix_locale_hints()
    }

    fn post(f: Box<dyn FnOnce() + Send>) {
        let data = Box::into_raw(Box::new(f)) as *mut c_void;
        unsafe { ffi::day_qt_post(run_posted, data) };
    }
}
