// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-xaml — the Windows backend (target `windows-xaml`; DESIGN.md §1, §9), over the
//! day-xaml-sys C++/WinRT XAML-Islands shim. `Handle = WinHandle(*mut UIElement)`; every Day
//! node is a real `Windows.UI.Xaml` control (TextBlock, Button, ToggleSwitch, Slider, TextBox,
//! ComboBox) hosted inside a `DesktopWindowXamlSource`. Day owns layout — containers are XAML
//! `Canvas`es and children are placed by absolute frame — exactly like the GTK/AppKit/Qt
//! backends. Native events (Click/Toggled/ValueChanged/TextChanged) funnel through the shim's
//! id-keyed callbacks into Day's event sink.

#![cfg(windows)]

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_double, c_int, c_void};
use std::rc::Rc;

use day_xaml_sys as ffi;
use linkme::distributed_slice;

use day_spec::props::*;
use day_spec::{
    A11yProps, AnimSpec, Builtin, Cap, Curve, DrawOp, Event, EventSink, Font, NodeId, PieceKind,
    Platform, Point, Proposal, Rect, Registry, Renderer, Size, Support, Toolkit, Transform,
    WindowOptions, ffi_guard, kinds, props_of,
};

/// An `AnimSpec` as the shim's `(duration_ms, curve)` pair — `(0, 0)` meaning "no animation, set
/// it outright". The curve encoding is shared with day-qt's shim (DESIGN.md §8.4).
fn xaml_anim_args(anim: Option<&AnimSpec>) -> (c_int, c_int) {
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

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct WinHandle(pub *mut c_void);

// Built-in leaf pieces split into modules (moved in from their satellite crates 2026-07).
mod picker;
mod textarea;
mod toolbar;

pub type Handle = WinHandle;

pub mod ext;
pub use ext::*;

/// The day-core event sink (node-id keyed).
type Sink = Rc<dyn Fn(NodeId, Event)>;

thread_local! {
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
    /// Tabs host ptr → (Pivot ptr, pages, initial). Pages reuse day.container.
    /// Recycling-list host ptr → its ScrollViewer/content + cell pool (docs/list.md).
    static LIST_STATE: RefCell<HashMap<usize, ListEntry>> = RefCell::new(HashMap::new());
    /// Label ptr → node id, so a `LabelPatch::Runs` (which carries no id) can still tell a link
    /// run's Hyperlink which node to report against. Entries drop in `release`.
    static LABEL_NODE: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
    /// NAV_MENU widget ptr → row count (for measure).
    static NAV_MENU_ROWS: RefCell<HashMap<usize, usize>> = RefCell::new(HashMap::new());
    /// NAV host ptr → its native presentation (NavigationView split / two-pane, docs/navigation.md).
    static NAV_STATE: RefCell<HashMap<usize, NavState>> = RefCell::new(HashMap::new());
    /// NAV_PAGE handle ptr → its node id (so region-resize callbacks can emit FrameChanged).
    static NAV_PAGE_IDS: RefCell<HashMap<usize, NodeId>> = RefCell::new(HashMap::new());
    /// NAV host node id → host handle ptr (the shim's callbacks carry the host node id).
    static NAV_HOST_BY_ID: RefCell<HashMap<u64, *mut c_void>> = RefCell::new(HashMap::new());
    /// A split NavigationView whose NAV_MENU hasn't been created yet (nav is app-root-only, so at
    /// most one is pending). The next NAV_MENU feeds this NavigationView's MenuItems.
    static PENDING_SPLIT_NAV: Cell<*mut c_void> = const { Cell::new(std::ptr::null_mut()) };
    /// NAV_MENU placeholder ptr → its NavigationView (split navs drive MenuItems, not a ListView).
    static NAV_MENU_HOST: RefCell<HashMap<usize, *mut c_void>> = RefCell::new(HashMap::new());
    /// Split-nav sidebar-page ptrs: the NavigationView's PaneHeader, clipped to a fixed height.
    static SPLIT_SIDEBAR_PAGES: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
    /// SCROLL host ptr → its inner content Canvas ptr (children live in the content, docs §7.6).
    static SCROLL_STATE: RefCell<HashMap<usize, *mut c_void>> = RefCell::new(HashMap::new());
    /// Handles with a native gesture recognizer wired, keyed by (handle ptr, kind) — idempotent.
    static GESTURES: RefCell<HashSet<(usize, c_int)>> = RefCell::new(HashSet::new());
}

// Navigation host — always a native `NavigationView` (docs/navigation.md), in one of two modes
// chosen by NavProps.presentation:
//  • Split       → the idiomatic Windows sidebar+header selector (as in Settings): MenuItems are the
//    destinations, the Header names the current one, Content holds the detail page.
//  • Stack       → a push/pop stack: no menu, the back button appears once a page is pushed, and
//    pages stack in the content region.
// `menu_node` is the NAV_MENU node id whose SelectionChanged the pane synthesizes; `sidebar_page`
// (day's logo/title piece) is the PaneHeader; detail pages live in `content_host` (nv.Content) kept
// with their node ids so a region resize can report FrameChanged (mirrors TABS).
struct SplitNav {
    nav_view: *mut c_void,
    content_host: *mut c_void,
    menu_node: u64,
    sidebar_page: Option<(*mut c_void, NodeId)>,
    /// (page, node, title) — the title is attached by `NavPatch::Pushed`/`Title` so a pop can
    /// restore the PREVIOUS page's title into the NavigationView header (stack_sync).
    detail_pages: Vec<(*mut c_void, NodeId, String)>,
    /// A push/pop stack (NavProps.presentation == Stack): no menu/sidebar, every page stacks in the
    /// content region, and the NavigationView back button appears once a page is pushed.
    is_stack: bool,
}

enum NavState {
    Split(SplitNav),
}

/// Fixed height (pt) of the NavigationView PaneHeader that hosts day's sidebar header piece
/// (logo + app title) — a bare Canvas has no desired size, so the slot needs an explicit one.
const NAV_PANE_HEADER_H: c_int = 60;

extern "C" fn nav_menu_changed(id: u64, index: c_int) {
    // Every extern "C" trampoline in this backend runs its body through `ffi_guard::contain`:
    // a panic unwinding into the C++/WinRT shim frame is undefined behavior (day-spec's
    // ffi_guard).
    ffi_guard::contain((), || {
        emit(NodeId(id), Event::SelectionChanged(index as i64))
    });
}

/// A user pick in a NavigationView pane: the shim passes the HOST node id + item index; route it to
/// the host's NAV_MENU node (whose day handler maps the index back to a route).
extern "C" fn nav_selection(host_id: u64, index: c_int) {
    ffi_guard::contain((), || {
        let host = NAV_HOST_BY_ID.with(|m| m.borrow().get(&host_id).copied());
        let Some(host) = host else { return };
        let menu = NAV_STATE.with(|m| match m.borrow().get(&(host as usize)) {
            Some(NavState::Split(s)) if s.menu_node != 0 => Some(s.menu_node),
            _ => None,
        });
        if let Some(menu_node) = menu {
            emit(NodeId(menu_node), Event::SelectionChanged(index as i64));
        }
    });
}

/// A NavigationView region reflowed (window resize, pane open/close): report the true size so day
/// re-lays the affected page(s). region 0 = content (detail pages), 1 = pane header (sidebar page).
extern "C" fn nav_region_size(host_id: u64, region: c_int, w: c_int, h: c_int) {
    ffi_guard::contain((), || {
        if w <= 0 || h <= 0 {
            return;
        }
        let host = NAV_HOST_BY_ID.with(|m| m.borrow().get(&host_id).copied());
        let Some(host) = host else { return };
        let size = Size::new(w as f64, h as f64);
        let reports: Vec<NodeId> = NAV_STATE.with(|m| {
            let m = m.borrow();
            let Some(NavState::Split(s)) = m.get(&(host as usize)) else {
                return Vec::new();
            };
            if region == 1 {
                s.sidebar_page.map(|(_, id)| id).into_iter().collect()
            } else {
                s.detail_pages.iter().map(|(_, id, _)| *id).collect()
            }
        });
        for id in reports {
            emit(id, Event::FrameChanged(size));
        }
    });
}

/// The NavigationView back button: pop one level (the stack surface writes it back into its path).
extern "C" fn nav_back(host_id: u64) {
    ffi_guard::contain((), || {
        emit(
            NodeId(host_id),
            Event::NavBack {
                already_popped: false,
            },
        )
    });
}

/// A stack's pages overlap in the content region; show only the top one so a transparent page
/// can't reveal those beneath it, and refresh the back button (visible once a page is pushed).
fn stack_sync(host: *mut c_void) {
    NAV_STATE.with(|m| {
        if let Some(NavState::Split(s)) = m.borrow().get(&(host as usize)) {
            let last = s.detail_pages.len().saturating_sub(1);
            for (i, (page, _, _)) in s.detail_pages.iter().enumerate() {
                unsafe { ffi::day_xaml_set_visible(*page, (i == last) as c_int) };
            }
            unsafe {
                ffi::day_xaml_nav_set_back_visible(s.nav_view, (s.detail_pages.len() >= 2) as c_int)
            };
            // Restore the (new) top page's title — without this a pop left the POPPED page's
            // title in the NavigationView header until the next push.
            if let Some((_, _, title)) = s.detail_pages.last() {
                unsafe { ffi::day_xaml_nav_set_header(s.nav_view, cstr(title).as_ptr()) };
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Recycling list (docs/list.md, §10). XAML's ListView virtualizes with a data source, which
// doesn't fit Day's synchronous `bind_row` pull; instead — like the Qt backend (DP-19) — Day
// EMULATES recycling: a ScrollViewer whose content Canvas holds one absolutely-positioned cell
// per row, each filled through the same `bind_row` seam. Cells are pooled append-only.
// ---------------------------------------------------------------------------

struct ListEntry {
    host: *mut c_void,
    content: *mut c_void,
    row_height: f64,
    source: Rc<RefCell<Option<day_spec::ListSource>>>,
    cells: Vec<*mut c_void>,
    /// Last host width a populate ran at, so `set_frame` only repopulates on a real width change
    /// (a populate's own child `set_frame`s must not schedule another, or it loops forever).
    last_width: c_int,
    /// The width day's layout last framed the host at (-1 = never framed). This — not the host's
    /// `ActualWidth`, which lags a posted populate by a layout pass — is what cells are sized from.
    frame_width: c_int,
    /// Drag-to-reorder (docs/list.md): whether new cells get the WinRT drag armed.
    reorderable: bool,
    node: u64,
    /// Selection (docs/list.md): whether rows select at all, whether several may be selected at
    /// once, the currently selected rows, and the last plainly-clicked row (the shift anchor).
    selectable: bool,
    multi: bool,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
}

thread_local! {
    /// List NODE id → host key, so the reorder + row-click callbacks (which carry the node) find
    /// their entry.
    static LIST_BY_NODE: RefCell<HashMap<u64, usize>> = RefCell::new(HashMap::new());
}

/// Repaint every cell's selected treatment from the entry's selection set.
fn list_paint_selection(entry: &ListEntry) {
    for (i, &cell) in entry.cells.iter().enumerate() {
        unsafe { ffi::day_xaml_cell_set_selected(cell, entry.selected.contains(&i) as c_int) };
    }
}

/// A press on an emulated list cell (docs/list.md). Owns the selection semantics: a plain click
/// replaces the selection, ctrl toggles the row, shift extends from the anchor (multi-select
/// only) — then repaints and reports (`SelectionSet` in multi mode, `SelectionChanged` single).
extern "C" fn on_list_row_click(node: u64, row: c_int, mods: c_int) {
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
/// shim's DragOver handler: the accepted target index, or -1. The source is cloned out before
/// the app's guard runs — no thread-local borrow held.
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

/// Commit a drop the shim accepted: rotate day's snapshot through the sync seam (deferring the
/// app callback) and re-bind the cells in the new order.
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

/// Populate/refresh a list's cells on the next loop turn — NOT inline: a reload runs inside a
/// `with_tree` borrow, and `bind_row` re-enters `with_tree`, which would panic.
fn schedule_list_populate(host_key: usize) {
    let boxed: Box<dyn FnOnce() + Send> = Box::new(move || list_populate(host_key));
    let data = Box::into_raw(Box::new(boxed)) as *mut c_void;
    unsafe { ffi::day_xaml_post(run_posted, data) };
}

/// Scroll the (emulated) list host to its bottom on the next turn — deferred so any pending
/// `list_populate` has sized the content Canvas first (posts run FIFO). No-op when empty.
fn schedule_list_scroll_end(host_key: usize) {
    let boxed: Box<dyn FnOnce() + Send> = Box::new(move || list_scroll_end(host_key));
    let data = Box::into_raw(Box::new(boxed)) as *mut c_void;
    unsafe { ffi::day_xaml_post(run_posted, data) };
}

/// Scroll the emulated list so row `row` sits at the top of the viewport (docs/list.md), on the
/// next loop turn — the same deferral as `schedule_list_scroll_end`.
fn schedule_list_scroll_row(host_key: usize, row: usize) {
    let boxed: Box<dyn FnOnce() + Send> = Box::new(move || {
        let target = LIST_STATE.with(|m| {
            let m = m.borrow();
            let st = m.get(&host_key)?;
            let rowh = st.row_height.max(1.0);
            Some((
                st.host,
                (row as f64 * rowh).round() as c_int,
                rowh.round() as c_int,
            ))
        });
        if let Some((host, y, rowh)) = target {
            unsafe { ffi::day_xaml_scroll_to(host, y, rowh, 1) };
        }
    });
    let data = Box::into_raw(Box::new(boxed)) as *mut c_void;
    unsafe { ffi::day_xaml_post(run_posted, data) };
}

fn list_scroll_end(host_key: usize) {
    // The list host is a ScrollViewer; make the last row's band [y, y+rowh] visible (its content
    // Canvas is `rows * rowh` tall). Reuses the general scroll seam — no new XAML shim.
    let target = LIST_STATE.with(|m| {
        let m = m.borrow();
        let st = m.get(&host_key)?;
        let n = st.source.borrow().as_ref().map(|s| (s.len)()).unwrap_or(0);
        if n == 0 {
            return None;
        }
        let rowh = st.row_height.max(1.0);
        Some((st.host, (n - 1) as f64 * rowh, rowh))
    });
    if let Some((host, y, rowh)) = target {
        unsafe { ffi::day_xaml_scroll_to(host, y.round() as c_int, rowh.round() as c_int, 1) };
    }
}

fn list_populate(host_key: usize) {
    // Phase 1 — under the LIST_STATE borrow: grow the cell pool + snapshot what we need.
    let Some((content, rowh, source, cells, n, width)) = LIST_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let st = m.get_mut(&host_key)?;
        let source = st.source.borrow().clone()?;
        // Cells are sized from the width day's layout FRAMED the host at — NOT from the host's
        // `ActualWidth`. A populate is posted for the next loop turn, which routinely runs before
        // XAML's layout pass has published the size `set_frame` just assigned, so `ActualWidth`
        // reads a stale 0 and every cell gets built 1px wide.
        let width = if st.frame_width > 0 {
            st.frame_width
        } else {
            // `attach_list` populates before the host is ever framed: use whatever is realized.
            let (mut w, mut h) = (0.0_f64, 0.0_f64);
            unsafe { ffi::day_xaml_widget_size(st.host, &mut w, &mut h) };
            w.round() as c_int
        };
        if width <= 0 {
            // No usable width yet. Bail WITHOUT recording `last_width`, so the host's next
            // `set_frame` still reads as a width change and schedules the real populate.
            return None;
        }
        let n = (source.len)();
        while st.cells.len() < n {
            let cell = unsafe { ffi::day_xaml_container_new() };
            unsafe { ffi::day_xaml_add_child(st.content, cell) };
            // Cell index == row for the cell's whole life (docs/list.md), so both the press
            // handler's row and the drag's are fixed here, at creation.
            if st.selectable {
                unsafe {
                    ffi::day_xaml_list_cell_click(
                        cell,
                        st.node,
                        st.cells.len() as c_int,
                        on_list_row_click,
                    )
                };
            }
            if st.reorderable {
                unsafe { ffi::day_xaml_cell_drag(cell, st.node, st.cells.len() as c_int) };
            }
            st.cells.push(cell);
        }
        st.last_width = width;
        Some((
            st.content,
            st.row_height.max(1.0),
            source,
            st.cells.clone(),
            n,
            width,
        ))
    }) else {
        return;
    };
    // Phase 2 — no borrow held: bind_row re-enters with_tree (lays the row out, set_frames the
    // list host — taking LIST_STATE again).
    for (i, &cell) in cells.iter().enumerate().take(n) {
        unsafe {
            ffi::day_xaml_set_geometry(cell, 0, (i as f64 * rowh) as c_int, width, rowh as c_int);
            ffi::day_xaml_set_visible(cell, 1);
        }
        (source.bind_row)(i, cell);
    }
    for &cell in cells.iter().skip(n) {
        unsafe { ffi::day_xaml_set_visible(cell, 0) };
    }
    unsafe { ffi::day_xaml_list_set_content_size(content, width, (n as f64 * rowh) as c_int) };
    // Cells just added to the pool start unpainted, and a reload can move which rows are selected
    // under a selection that hasn't changed — so repaint from the entry's set on every populate.
    LIST_STATE.with(|m| {
        if let Some(st) = m.borrow().get(&host_key) {
            list_paint_selection(st);
        }
    });
}

/// Emit an event into day-core's queue (public for external Day Piece renderers).
pub fn emit(id: NodeId, ev: Event) {
    let sink = SINK.with(|s| s.borrow().clone());
    if let Some(sink) = sink {
        sink(id, ev);
    }
}

/// Hand a [`day_spec::props::ButtonStyleSpec`] to the shim, which styles the Button in place.
/// The element stays a `Button`, so focus, keyboard activation and its automation peer are
/// unaffected by the style.
fn apply_button_style(h: *mut c_void, style: day_spec::props::ButtonStyleSpec) {
    use day_spec::props::ButtonStyleSpec as S;
    let argb = |c: day_spec::Color| {
        let f = |v: f64| (v.clamp(0.0, 1.0) * 255.0) as u32;
        (f(c.a) << 24) | (f(c.r) << 16) | (f(c.g) << 8) | f(c.b)
    };
    let (kind, fill) = match style {
        S::Automatic => (0, day_spec::Color::CLEAR),
        S::Bordered => (1, day_spec::Color::CLEAR),
        S::Prominent => (2, day_spec::Color::CLEAR),
        S::Tinted(c) => (3, c),
    };
    // SAFETY: `h` is a live Button handle from `day_xaml_button_new`; the shim only reads the
    // packed colors.
    unsafe { ffi::day_xaml_button_set_style(h, kind, argb(fill), argb(S::on_tint(fill))) };
}

/// Send a label's runs across as a begin + one add per run (docs/text-runs.md).
///
/// Runs become `Inline`s in the one `TextBlock`, so the paragraph still wraps and selects as a
/// unit. Flags pack the styling so each run is a single call with no marshaling.
fn set_label_runs(h: *mut c_void, node: u64, text: &str, runs: &[day_spec::TextRun]) {
    if runs.is_empty() {
        // The plain setter also clears the Inlines, so a label losing its runs stops rendering
        // the styled version.
        unsafe { ffi::day_xaml_label_set_text(h, cstr(text).as_ptr()) };
        return;
    }
    unsafe { ffi::day_xaml_label_runs_begin(h, node) };
    let mut at = 0usize;
    let add = |slice: &str, run: Option<&day_spec::TextRun>| {
        let mut flags = 0i32;
        let mut argb = 0u32;
        let mut bg_argb = 0u32;
        let mut scale_permille = 1000i32;
        let mut link = String::new();
        if let Some(r) = run {
            scale_permille = (r.font.scale * 1000.0).round() as i32;
            if r.font
                .weight
                .is_some_and(|w| w >= day_spec::FontWeight::Semibold)
            {
                flags |= 1;
            }
            if r.font.italic {
                flags |= 2;
            }
            if r.font.monospace {
                flags |= 4;
            }
            if r.strikethrough {
                flags |= 8;
            }
            if let Some(c) = r.color {
                flags |= 16;
                let f = |v: f64| (v.clamp(0.0, 1.0) * 255.0) as u32;
                argb = (f(c.a) << 24) | (f(c.r) << 16) | (f(c.g) << 8) | f(c.b);
            }
            let pack = |c: day_spec::Color| {
                let f = |v: f64| (v.clamp(0.0, 1.0) * 255.0) as u32;
                (f(c.a) << 24) | (f(c.r) << 16) | (f(c.g) << 8) | f(c.b)
            };
            if let Some(c) = r.background {
                flags |= 32;
                bg_argb = pack(c);
            }
            if r.underline.is_on() {
                flags |= 64;
            }
            if let Some(u) = r.link.as_deref() {
                link = u.to_string();
            }
        }
        unsafe {
            ffi::day_xaml_label_runs_add(
                h,
                cstr(slice).as_ptr(),
                flags,
                argb,
                bg_argb,
                scale_permille,
                cstr(&link).as_ptr(),
            )
        };
    };
    for r in runs {
        let Some(styled) = text.get(r.range.clone()) else {
            continue;
        };
        if r.range.start > at
            && let Some(plain) = text.get(at..r.range.start)
        {
            add(plain, None);
        }
        add(styled, Some(r));
        at = r.range.end;
    }
    if let Some(tail) = text.get(at..) {
        add(tail, None);
    }
}

fn cstr(s: &str) -> CString {
    // An interior NUL must not blank the whole string — a label, a menu item, a window title
    // would silently vanish. Strip the NULs and keep the visible text.
    CString::new(s).unwrap_or_else(|_| CString::new(s.replace('\0', "")).unwrap_or_default())
}

/// A styled run's link was clicked (docs/text-runs.md). The Hyperlink carries no NavigateUri,
/// so nothing has opened anything yet: the label's `.on_link()` decides.
extern "C" fn on_link(id: u64, url: *const c_char) {
    ffi_guard::contain((), || {
        let url = unsafe { CStr::from_ptr(url) }
            .to_string_lossy()
            .into_owned();
        emit(NodeId(id), Event::LinkActivated(url));
    });
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
extern "C" fn on_slider(id: u64, v: f64, committed: c_int) {
    // XAML's Slider is driven in the app's real f64 units, so its Value is the event value as-is.
    // The live value always; the settled one additionally, so a drag records once (the shim
    // decides which is which — see `day_xaml_slider_new`).
    ffi_guard::contain((), || {
        emit(NodeId(id), Event::ValueChanged(v));
        if committed != 0 {
            emit(NodeId(id), Event::ValueCommitted(v));
        }
    });
}
/// Focus callback from the shim (docs/focus.md). kind: 0 = lost, 1 = gained, 2 = submitted.
extern "C" fn on_focus(id: u64, kind: c_int) {
    ffi_guard::contain((), || {
        let ev = match kind {
            2 => Event::Submitted,
            k => Event::FocusChanged(k != 0),
        };
        emit(NodeId(id), ev);
    });
}

/// A `0.0..=1.0` fraction as ProgressBar ticks (0..1000), clamped.
fn progress_ticks(fraction: f64) -> c_int {
    (fraction.clamp(0.0, 1.0) * 1000.0).round() as c_int
}

/// Renderers registered by external Day Piece crates (§8.2).
#[distributed_slice]
pub static RENDERERS: [fn() -> Renderer<Xaml>];

/// A live secondary window (docs/windows.md): the shim's SecWindow plus the day content
/// root the tree adopted.
struct XamlWin {
    win: *mut c_void,
    content: *mut c_void,
    /// A Preferences panel, which carries no menu bar (docs/windows.md). Recorded because the
    /// app menu is re-installed on a locale change, and that pass walks every open window — it
    /// would put a File/Edit/View bar back on the settings panel.
    menuless: bool,
}

pub struct Xaml {
    registry: Registry<Xaml>,
    window: *mut c_void,
    secondary: Vec<XamlWin>,
    /// The app menu's serialized spec, replayed into each window that opens after it was set
    /// (docs/menus.md): one menu for the app, but Windows draws it per window.
    menu_spec: String,
    /// The primary window's root container. Held so `release` can recognize it and destroy the
    /// host — the primary is an ordinary window now (docs/windows.md close policy), torn down
    /// on the same released-root signal as any other.
    primary_root: *mut c_void,
}

impl Xaml {
    /// The shim-side window token owning `h` — the primary window, or the secondary whose content
    /// canvas this is. `None` when the handle belongs to no window day opened, which is how a
    /// chrome call for an unknown host is dropped rather than landing on the wrong window.
    fn window_token(&self, h: &WinHandle) -> Option<*mut c_void> {
        if let Some(w) = self.secondary.iter().find(|w| w.content == h.0) {
            return Some(w.win);
        }
        (!self.window.is_null()).then_some(self.window)
    }
}

impl Xaml {
    pub fn new() -> Self {
        let mut registry = Registry::default();
        for f in RENDERERS {
            registry.register(f());
        }
        // Data resources (§18.3): no custom opener registered. Unlike Android/GTK/Qt/ArkUI — whose
        // bytes live inside a packaged store (AAssetManager/GResource/QResource/rawfile) and so need
        // a backend opener — an unpackaged Win32 app reaches loose files next to its exe directly.
        // `day build` stages data under `assets/` there, which is exactly what day-spec's default
        // opener mmaps (env `DAY_ASSET_ROOT`, then exe-relative `assets/`), so `resource("name")`
        // works with zero backend wiring. A custom opener would only earn its keep for embedded
        // RCDATA, which this backend does not use.
        Xaml {
            registry,
            window: std::ptr::null_mut(),
            secondary: Vec::new(),
            menu_spec: String::new(),
            primary_root: std::ptr::null_mut(),
        }
    }
}

impl Default for Xaml {
    fn default() -> Self {
        Self::new()
    }
}

/// Day font intents → (XAML FontSize in DIPs, bold).
/// Point size + the style's inherent weight for a logical [`Font`]. XAML's `TextBlock.FontSize`
/// auto-scales with the OS text-scale-factor (Settings ▸ Accessibility ▸ Text size), so these sizes
/// honor accessibility. Aligned with the desktop scale used by the GTK/Qt backends.
fn xaml_style(f: Font) -> (f64, day_spec::FontWeight) {
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

/// Apply a `Font::Custom` family on top of `day_xaml_label_set_font` (which set size/weight).
/// Unpackaged Win32 XAML can't load a font by file path — a raw path isn't a valid `Uri` and
/// `file://` is rejected (exactly like `BitmapImage`; see the image-loading path). The one font
/// location system XAML *does* resolve is `ms-appx:///`, which maps to the executable's directory
/// and its subtree, so `stage_bundled_fonts` copies each font under `<exe>/fonts/` and here we
/// reference it as `ms-appx:///fonts/<file>#<family>`. Resolution parses font name tables, so it is
/// cached per family; an unknown family logs once and leaves the system font in place.
fn apply_custom_family(h: *mut c_void, spec: day_spec::FontSpec) {
    let Font::Custom(family, _) = spec.style else {
        return;
    };
    thread_local! {
        static RESOLVED: RefCell<HashMap<&'static str, Option<CString>>> =
            RefCell::new(HashMap::new());
    }
    RESOLVED.with(|cache| {
        let mut cache = cache.borrow_mut();
        let entry =
            cache.entry(family).or_insert_with(|| {
                match day_spec::fonts::resolve_font_file(family)
                    .as_deref()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                {
                    Some(file) => Some(cstr(&format!("ms-appx:///fonts/{file}#{family}"))),
                    None => {
                        eprintln!(
                            "day: unknown font family {family:?} — falling back to the system font \
                         (is the file in the project's fonts/ directory?)"
                        );
                        None
                    }
                }
            });
        if let Some(s) = entry {
            unsafe { ffi::day_xaml_label_set_font_family(h, s.as_ptr()) };
        }
    });
}

/// Stage the bundled font files (§18.4) next to the executable so XAML can load them. Unpackaged
/// system XAML only resolves fonts under `ms-appx:///` (the exe directory and its subtree), so copy
/// every `DAY_FONT_ROOT` font into `<exe>/fonts/` — a no-op when packed, where `day pack` already
/// ships them there (`font_dir()` returns that same directory, so src == dst). `apply_custom_family`
/// then references each as `ms-appx:///fonts/<file>#<family>`.
fn stage_bundled_fonts() {
    let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("fonts")))
    else {
        return;
    };
    for src in day_spec::fonts::bundled_fonts() {
        let Some(dst) = src.file_name().map(|n| dir.join(n)) else {
            continue;
        };
        if src == dst {
            continue; // already staged next to the exe (packed apps)
        }
        if let Err(e) =
            std::fs::create_dir_all(&dir).and_then(|_| std::fs::copy(&src, &dst).map(|_| ()))
        {
            eprintln!(
                "day-xaml: could not stage bundled font {}: {e}",
                src.display()
            );
        }
    }
}

/// Stage bundled images (DAY_IMAGE_ROOT, plus the vector raster cache DAY_VECTOR_RASTER_ROOT —
/// docs/vectors.md) next to the exe under `images/` so a `BitmapIcon` can load them via
/// `ms-appx:///images/<file>` (same unpackaged-islands workaround as the fonts). Only nav
/// icons need this today (regular `image()` loads bytes via a stream); a no-op when packed
/// (`day pack` already merges both trees into the exe-relative `images/`) or when neither
/// root is set.
fn stage_bundled_images() {
    let Some(dst_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("images")))
    else {
        return;
    };
    let src_dirs: Vec<std::path::PathBuf> = ["DAY_IMAGE_ROOT", "DAY_VECTOR_RASTER_ROOT"]
        .iter()
        .filter_map(|var| std::env::var_os(var).map(std::path::PathBuf::from))
        .collect();
    for src_dir in src_dirs {
        let Ok(entries) = std::fs::read_dir(&src_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let src = entry.path();
            if !src.is_file() {
                continue;
            }
            let Some(dst) = src.file_name().map(|n| dst_dir.join(n)) else {
                continue;
            };
            if src == dst {
                continue;
            }
            let _ = std::fs::create_dir_all(&dst_dir)
                .and_then(|_| std::fs::copy(&src, &dst).map(|_| ()));
        }
    }
}

/// Day weight → Windows.UI.Text.FontWeight numeric value (Thin=100 … Black=900).
fn xaml_weight(w: day_spec::FontWeight) -> c_int {
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

/// (point size, FontWeight numeric, italic) for the C++/WinRT shim.
fn font_params(spec: day_spec::FontSpec) -> (f64, c_int, c_int, c_int) {
    let (pt, inherent) = xaml_style(spec.style);
    let weight = xaml_weight(spec.weight.unwrap_or(inherent));
    (pt, weight, spec.italic as c_int, spec.tabular as c_int)
}

/// Natural (unconstrained) desired size from the shim's XAML Measure.
fn natural(h: *mut c_void) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { ffi::day_xaml_measure(h, -1.0, -1.0, &mut w, &mut hh) };
    Size::new(w, hh)
}

// ---- menus (docs/menus.md) -------------------------------------------------

extern "C" fn on_menu_action(id: u64) {
    ffi_guard::contain((), || emit(day_spec::WINDOW_NODE, Event::MenuAction(id)));
}

/// Which lifecycle phases this desktop backend delivers (docs/lifecycle.md): the universal set.
/// `const` so `day::require_lifecycle!` can reject unsupported phases at compile time.
pub const fn lifecycle_supported(phase: day_spec::Lifecycle) -> bool {
    phase.is_universal()
}

/// Phase codes (from the shim's WndProc) → day lifecycle events.
extern "C" fn on_lifecycle(code: c_int) {
    use day_spec::Lifecycle::*;
    ffi_guard::contain((), || {
        let phase = match code {
            2 => DidBecomeActive,
            3 => WillResignActive,
            7 => WillTerminate,
            _ => return,
        };
        emit(day_spec::WINDOW_NODE, Event::Lifecycle(phase));
    });
}

fn win_role_label(role: day_spec::MenuRole) -> String {
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
        Quit => "Exit",
        Preferences => "Settings",
        Minimize => "Minimize",
        CloseWindow => "Close",
        Fullscreen => "Full Screen",
        NewWindow => "New Window",
    }
    .to_string()
}

/// Standard (keycode, modifier-bitmask) for a role: bit0 Control, bit1 Shift, bit2 Alt.
fn win_role_keymods(role: day_spec::MenuRole) -> (i32, i32) {
    use day_spec::MenuRole::*;
    match role {
        Cut => (b'X' as i32, 1),
        Copy => (b'C' as i32, 1),
        Paste => (b'V' as i32, 1),
        SelectAll => (b'A' as i32, 1),
        Undo => (b'Z' as i32, 1),
        Redo => (b'Y' as i32, 1),
        NewWindow => (b'N' as i32, 1),
        CloseWindow => (b'W' as i32, 1),
        _ => (0, 0),
    }
}

fn win_mods(sc: &day_spec::Shortcut) -> i32 {
    let mut m = 0;
    if sc.primary || sc.control {
        m |= 1; // Control is the primary modifier on Windows
    }
    if sc.shift {
        m |= 2;
    }
    if sc.alt {
        m |= 4;
    }
    m
}

/// Windows `VirtualKey` code for a shortcut key string (0 = none/unmapped).
fn win_keycode(key: &str) -> i32 {
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.clone().next()) {
        if c.is_ascii_alphabetic() {
            return c.to_ascii_uppercase() as i32;
        }
        if c.is_ascii_digit() {
            return c as i32;
        }
        return match c {
            ',' => 0xBC,
            '.' => 0xBE,
            '-' => 0xBD,
            '=' => 0xBB,
            '/' => 0xBF,
            _ => 0,
        };
    }
    match key {
        "Return" | "Enter" => 0x0D,
        "Delete" | "Del" => 0x2E,
        "Space" => 0x20,
        "Escape" | "Esc" => 0x1B,
        "Tab" => 0x09,
        "Backspace" | "Back" => 0x08,
        "Left" => 0x25,
        "Up" => 0x26,
        "Right" => 0x27,
        "Down" => 0x28,
        "Home" => 0x24,
        "End" => 0x23,
        _ => key
            .strip_prefix('F')
            .and_then(|n| n.parse::<i32>().ok())
            .filter(|n| (1..=12).contains(n))
            .map(|n| 0x70 + (n - 1))
            .unwrap_or(0),
    }
}

/// Serialize the day-neutral tree to the shim's line format:
/// `kind \t id \t role \t key \t mods \t enabled \t label` (kinds A/R/S/E/`-`).
fn serialize_menu_xaml(items: &[day_spec::MenuItem], out: &mut String) {
    fn clean(s: &str) -> String {
        s.replace(['\t', '\n'], " ")
    }
    for item in items {
        match item {
            day_spec::MenuItem::Separator => out.push_str("-\t0\t-1\t0\t0\t1\t\n"),
            day_spec::MenuItem::Submenu { label, items, .. } => {
                out.push_str(&format!("S\t0\t-1\t0\t0\t1\t{}\n", clean(label)));
                serialize_menu_xaml(items, out);
                out.push_str("E\t0\t-1\t0\t0\t1\t\n");
            }
            day_spec::MenuItem::Action {
                id,
                label,
                shortcut,
                enabled,
                role,
            } => {
                let en = *enabled as i32;
                // Label/shortcut fall back to the role's platform defaults; the DISPATCH
                // is chosen separately below.
                let text = match role {
                    Some(r) if label.is_empty() => win_role_label(*r),
                    _ => clean(label),
                };
                let (key, mods) = match (shortcut, role) {
                    (Some(sc), _) => (win_keycode(&sc.key), win_mods(sc)),
                    (None, Some(r)) => win_role_keymods(*r),
                    (None, None) => (0, 0),
                };
                // A nonzero id ALWAYS wins (the appkit precedence, docs/menus.md): the item
                // dispatches the day action and the role only decorates it. day-core's auto
                // Preferences item and `MenuRole::NewWindow` arrive exactly this way — routing
                // them through the role-only path dropped the id, leaving visible-but-DEAD
                // menu items (the same bug macos-qt had).
                match role {
                    Some(r) if *id == 0 => out.push_str(&format!(
                        "R\t0\t{}\t{}\t{}\t{}\t{}\n",
                        *r as i32,
                        key,
                        mods,
                        en,
                        clean(&text)
                    )),
                    _ => out.push_str(&format!(
                        "A\t{}\t-1\t{}\t{}\t{}\t{}\n",
                        id,
                        key,
                        mods,
                        en,
                        clean(&text)
                    )),
                }
            }
        }
    }
}

/// Warn ONCE per kind that this backend has no registered renderer for `kind`, before falling back to
/// a visible placeholder. A missing renderer usually means the piece's `xaml` feature wasn't enabled
/// (Tier A.2 derives it automatically under `day build`). Deduped per kind so a placeholder rendered
/// every frame doesn't spam the log.
fn warn_missing_renderer(kind: PieceKind) {
    day_spec::placeholder::report(kind, "xaml");
}

/// The visible placeholder a realize arm degrades to when its props payload has the wrong type
/// (`props_of` has already reported the mismatch) — the same label the missing-renderer arm shows.
pub(crate) fn placeholder_handle(kind: PieceKind) -> WinHandle {
    WinHandle(unsafe { ffi::day_xaml_label_new(cstr(&format!("⟨{kind}⟩")).as_ptr()) })
}

impl Toolkit for Xaml {
    type Handle = WinHandle;

    fn capability(&self, cap: Cap) -> Support {
        match cap {
            Cap::Snapshot => Support::Native,
            // TextBlock.BaselineOffset for text, font-derived for templated controls
            // (docs/baseline.md).
            Cap::BaselineAlignment => Support::Emulated,
            // Runs are TextBlock inlines; a link run is a Hyperlink whose Click reports back
            // (docs/text-runs.md).
            Cap::TextRuns | Cap::TextLinks => Support::Native,
            // text_area attributes (docs/textarea.md): editable and spell-check are plain TextBox
            // properties (IsReadOnly / IsSpellCheckEnabled).
            Cap::TextEditable | Cap::TextSpellCheck => Support::Native,
            // Selection is emulated: IsTextSelectionEnabled is TextBlock's, not TextBox's, so the
            // shim collapses selections as they form and suppresses the context menu instead.
            Cap::TextSelectable => Support::Emulated,
            Cap::ListRecycling => Support::Emulated,
            // The real WinRT drag pipeline (CanDrag/DragOver/Drop) over the emulated list —
            // system drag visuals + live no-drop cursor from the app's guard (docs/list.md).
            Cap::ListReorder => Support::Native,
            // A second Win32 host + its own XAML island per window (docs/windows.md).
            Cap::MultiWindow => Support::Native,
            // A Fluent CommandBar under the menu bar (docs/toolbars.md).
            Cap::AppMenu | Cap::Toolbar => Support::Native,
            // Present `nav()` as split panes: NAV/NAV_PAGE are plain Canvases and day-core's
            // NavLayout positions the sidebar + detail (no native split control needed).
            Cap::NavSplit => Support::Native,
            // The SAME NavigationView with a different pane: `Top` is WinUI's tab bar and
            // `LeftCompact` a real icon rail (docs/navigation.md).
            //
            // `Cap::NavTabsAdaptive` is deliberately not here: a Windows app may PIN a tab bar,
            // but a narrowing window collapses its pane rather than growing one — the same rule
            // every other desktop follows.
            Cap::NavTabs => Support::Native,
            // Native modals (ContentDialog) + WinRT file pickers (docs/dialogs.md, docs/files.md).
            Cap::Dialogs | Cap::FileDialogs => Support::Native,
            // The system light/dark setting, read live and re-reported when the user changes it
            // (docs/appearance.md). XAML's own controls follow theme resources by themselves; this
            // is what makes DAY's palette follow too.
            Cap::Appearance => Support::Native,
            // A topmost child of the content Canvas — not a system modal (docs/cover.md).
            Cap::Cover => Support::Emulated,
            // The NavigationView shows the current destination in its Header, so pages needn't
            // repeat their title in-content (docs/navigation.md).
            Cap::NavHeader => Support::Native,
            _ => Support::Unsupported,
        }
    }

    fn realize(&mut self, kind: PieceKind, props: &dyn std::any::Any, id: NodeId) -> WinHandle {
        unsafe {
            match Builtin::from_key(kind) {
                Some(Builtin::Container) => {
                    let h = ffi::day_xaml_container_new();
                    if let Some(p) = props.downcast_ref::<ContainerProps>() {
                        if p.role == Some(day_spec::SurfaceRole::SectionCard) {
                            // Theme-resource card brush — tracks light/dark automatically.
                            ffi::day_xaml_container_set_card(h, p.corner_radius);
                        }
                        if let Some(bg) = p.background {
                            ffi::day_xaml_container_set_bg(h, argb(bg));
                        }
                        if p.corner_radius > 0.0 && p.role.is_none() {
                            ffi::day_xaml_container_set_corner(h, p.corner_radius);
                        }
                    }
                    WinHandle(h)
                }
                Some(Builtin::Scroll) => {
                    let horizontal = props
                        .downcast_ref::<day_spec::props::ScrollProps>()
                        .map(|p| p.horizontal)
                        .unwrap_or(false);
                    let mut content: *mut c_void = std::ptr::null_mut();
                    let sv = ffi::day_xaml_scroll_new(&mut content, horizontal as c_int);
                    SCROLL_STATE.with(|m| m.borrow_mut().insert(sv as usize, content));
                    WinHandle(sv)
                }
                Some(Builtin::Canvas) => WinHandle(ffi::day_xaml_canvas_new()),
                Some(Builtin::Nav) => {
                    let Some(p) = props_of::<NavProps>(kind, "xaml", props) else {
                        return placeholder_handle(kind);
                    };
                    // Both presentations are a native NavigationView: a sidebar+header selector
                    // (split) or a push/pop stack with a back button (docs/navigation.md).
                    let is_stack = !p.presentation.is_split();
                    let mut content: *mut c_void = std::ptr::null_mut();
                    // Where the rows are the CHROME the SAME NavigationView wears a different
                    // pane: `Top` is WinUI's tab bar and `LeftCompact` a real icon rail, so a
                    // rail lands on a rail here rather than rounding to a sidebar the way it must
                    // on macOS (docs/navigation.md). Pages stay resident and `Select` switches
                    // them, exactly as the tab presentation does everywhere else.
                    let pane_mode = match p.presentation {
                        day_spec::props::NavPresentation::Tabs => 2,
                        day_spec::props::NavPresentation::Rail => 3,
                        // Split and Stack keep the NavigationView's own pane display mode, which
                        // is what `is_stack` above already configured.
                        _ => -1,
                    };
                    let nav = ffi::day_xaml_nav_new(
                        id.0,
                        nav_selection,
                        nav_region_size,
                        nav_back,
                        &mut content,
                        is_stack as c_int,
                    );
                    if pane_mode >= 0 {
                        ffi::day_xaml_nav_set_pane_mode(nav, pane_mode);
                    }
                    NAV_STATE.with(|m| {
                        m.borrow_mut().insert(
                            nav as usize,
                            NavState::Split(SplitNav {
                                nav_view: nav,
                                content_host: content,
                                menu_node: 0,
                                sidebar_page: None,
                                detail_pages: Vec::new(),
                                is_stack,
                            }),
                        )
                    });
                    NAV_HOST_BY_ID.with(|m| m.borrow_mut().insert(id.0, nav));
                    if !is_stack {
                        // The next NAV_MENU (built into the sidebar page) feeds this pane's items.
                        PENDING_SPLIT_NAV.with(|c| c.set(nav));
                    }
                    WinHandle(nav)
                }
                Some(Builtin::NavPage) => {
                    let page = ffi::day_xaml_container_new();
                    NAV_PAGE_IDS.with(|m| m.borrow_mut().insert(page as usize, id));
                    WinHandle(page)
                }
                // Emulated fullscreen cover (docs/cover.md): parked hidden; Present re-homes
                // it onto the window's content Canvas, appended last (= topmost), at the
                // content size.
                Some(Builtin::Cover) => {
                    let cover = ffi::day_xaml_container_new();
                    ffi::day_xaml_set_visible(cover, 0);
                    COVER_IDS.with(|m| m.borrow_mut().insert(cover as usize, id));
                    WinHandle(cover)
                }
                Some(Builtin::NavMenu) => {
                    let Some(p) = props_of::<NavMenuProps>(kind, "xaml", props) else {
                        return placeholder_handle(kind);
                    };
                    let pending = PENDING_SPLIT_NAV.with(|c| c.get());
                    if !pending.is_null() {
                        // Split nav: the destinations become the NavigationView's own MenuItems, so
                        // the menu node is just an invisible placeholder inside the sidebar page.
                        let icons_joined = p
                            .icons
                            .iter()
                            .map(|ic| ic.as_deref().map(icon_file_name).unwrap_or_default())
                            .collect::<Vec<_>>()
                            .join("\n");
                        // The trailing status glyph rides the same three channels the leading
                        // icon does: staged file name, vector geometry, tint.
                        let badge_icons_joined = p
                            .badge_icons
                            .iter()
                            .map(|ic| ic.as_deref().map(icon_file_name).unwrap_or_default())
                            .collect::<Vec<_>>()
                            .join("\n");
                        ffi::day_xaml_nav_set_items(
                            pending,
                            cstr(&p.items.join("\n")).as_ptr(),
                            cstr(&icons_joined).as_ptr(),
                            cstr(&join_geoms(&p.icons)).as_ptr(),
                            cstr(&join_tints(&p.tints)).as_ptr(),
                            cstr(&badge_icons_joined).as_ptr(),
                            cstr(&join_geoms(&p.badge_icons)).as_ptr(),
                            cstr(&join_tints(&p.badge_tints)).as_ptr(),
                        );
                        ffi::day_xaml_nav_set_selected(
                            pending,
                            p.selected.map(|i| i as c_int).unwrap_or(-1),
                        );
                        NAV_STATE.with(|m| {
                            if let Some(NavState::Split(s)) =
                                m.borrow_mut().get_mut(&(pending as usize))
                            {
                                s.menu_node = id.0;
                            }
                        });
                        PENDING_SPLIT_NAV.with(|c| c.set(std::ptr::null_mut()));
                        let placeholder = ffi::day_xaml_container_new();
                        NAV_MENU_HOST
                            .with(|m| m.borrow_mut().insert(placeholder as usize, pending));
                        WinHandle(placeholder)
                    } else {
                        // Standalone ListView (non-split fallback).
                        let w = ffi::day_xaml_navlist_new(id.0, nav_menu_changed);
                        ffi::day_xaml_navlist_set_items(w, cstr(&p.items.join("\n")).as_ptr());
                        ffi::day_xaml_navlist_set_selected(
                            w,
                            p.selected.map(|i| i as c_int).unwrap_or(-1),
                        );
                        NAV_MENU_ROWS.with(|m| m.borrow_mut().insert(w as usize, p.items.len()));
                        WinHandle(w)
                    }
                }
                Some(Builtin::Label) => {
                    let Some(p) = props_of::<LabelProps>(kind, "xaml", props) else {
                        return placeholder_handle(kind);
                    };
                    let h = ffi::day_xaml_label_new(cstr(&p.text).as_ptr());
                    let (pt, weight, italic, tabular) = font_params(p.font);
                    ffi::day_xaml_label_set_font(h, pt, weight, italic, tabular);
                    apply_custom_family(h, p.font);
                    if let Some(c) = p.color {
                        ffi::day_xaml_label_set_color(h, argb(c));
                    }
                    LABEL_NODE.with(|m| m.borrow_mut().insert(h as usize, id.0));
                    if !p.runs.is_empty() {
                        set_label_runs(h, id.0, &p.text, &p.runs);
                    }
                    WinHandle(h)
                }
                Some(Builtin::Button) => {
                    let Some(p) = props_of::<ButtonProps>(kind, "xaml", props) else {
                        return placeholder_handle(kind);
                    };
                    let h = ffi::day_xaml_button_new(cstr(&p.title).as_ptr(), id.0, on_press);
                    apply_button_style(h, p.style);
                    ffi::day_xaml_enable_focus(h, id.0, on_focus);
                    ffi::day_xaml_set_enabled(h, p.enabled as c_int);
                    WinHandle(h)
                }
                Some(Builtin::Toggle) => {
                    let Some(p) = props_of::<ToggleProps>(kind, "xaml", props) else {
                        return placeholder_handle(kind);
                    };
                    let h = ffi::day_xaml_toggle_new(p.on as c_int, id.0, on_toggle);
                    ffi::day_xaml_enable_focus(h, id.0, on_focus);
                    ffi::day_xaml_set_enabled(h, p.enabled as c_int);
                    WinHandle(h)
                }
                Some(Builtin::Slider) => {
                    let Some(p) = props_of::<SliderProps>(kind, "xaml", props) else {
                        return placeholder_handle(kind);
                    };
                    // Default to a fine 1/1000-of-range step (matching the GTK backend) when the app
                    // leaves it unset, so the slider stays effectively continuous.
                    let step = p.step.unwrap_or((p.max - p.min) / 1000.0).max(1e-9);
                    let h = ffi::day_xaml_slider_new(p.value, p.min, p.max, step, id.0, on_slider);
                    ffi::day_xaml_enable_focus(h, id.0, on_focus);
                    ffi::day_xaml_set_enabled(h, p.enabled as c_int);
                    WinHandle(h)
                }
                Some(Builtin::Picker) => picker::realize_any(self, props, id),
                Some(Builtin::TextArea) => textarea::realize_any(self, props, id),
                Some(Builtin::TextField) => {
                    let Some(p) = props_of::<TextFieldProps>(kind, "xaml", props) else {
                        return placeholder_handle(kind);
                    };
                    let h = ffi::day_xaml_textbox_new(
                        cstr(&p.text).as_ptr(),
                        cstr(&p.placeholder).as_ptr(),
                        id.0,
                        on_text,
                    );
                    ffi::day_xaml_enable_focus(h, id.0, on_focus);
                    ffi::day_xaml_set_enabled(h, p.enabled as c_int);
                    WinHandle(h)
                }
                Some(Builtin::Divider) => WinHandle(ffi::day_xaml_divider_new()),
                Some(Builtin::List) => {
                    let Some(p) = props_of::<ListProps>(kind, "xaml", props) else {
                        return placeholder_handle(kind);
                    };
                    let mut content: *mut c_void = std::ptr::null_mut();
                    let host = ffi::day_xaml_list_new(&mut content);
                    let row_height = match p.row_height {
                        RowHeight::Uniform(h) => h,
                        RowHeight::Automatic => 44.0,
                    };
                    if p.reorderable && !content.is_null() {
                        // WinRT drag reorder (docs/list.md): drops on the content Canvas route
                        // through the seam callbacks above.
                        ffi::day_xaml_list_enable_reorder(
                            content,
                            id.0,
                            row_height as c_int,
                            on_list_can_move,
                            on_list_move,
                        );
                    }
                    LIST_STATE.with(|m| {
                        m.borrow_mut().insert(
                            host as usize,
                            ListEntry {
                                host,
                                content,
                                row_height,
                                source: Rc::new(RefCell::new(None)),
                                cells: Vec::new(),
                                last_width: -1,
                                frame_width: -1,
                                reorderable: p.reorderable,
                                node: id.0,
                                selectable: p.selectable,
                                multi: p.multi_select,
                                selected: BTreeSet::new(),
                                anchor: None,
                            },
                        )
                    });
                    LIST_BY_NODE.with(|m| m.borrow_mut().insert(id.0, host as usize));
                    WinHandle(host)
                }
                Some(Builtin::Progress) => {
                    let Some(p) = props_of::<ProgressProps>(kind, "xaml", props) else {
                        return placeholder_handle(kind);
                    };
                    match p.value {
                        Some(v) => WinHandle(ffi::day_xaml_progress_new(1, progress_ticks(v))),
                        None => WinHandle(ffi::day_xaml_progress_new(0, 0)),
                    }
                }
                Some(Builtin::Image) => {
                    let Some(p) = props_of::<ImageProps>(kind, "xaml", props) else {
                        return placeholder_handle(kind);
                    };
                    // Scaling: 0=fit, 1=fill (crop), 2=stretch.
                    let mode = match p.content_mode {
                        ContentMode::Fit => 0,
                        ContentMode::Fill => 1,
                        ContentMode::Stretch => 2,
                    };
                    // A `vector(…)` glyph draws as real geometry (docs/vectors.md) — vector at
                    // any size, and its tint composed as a brush at realize time. Tried FIRST,
                    // tinted or not; `vector_geometry` is None for a raster `image(…)` name and
                    // for art the CLI could not convert, which is what falls through below.
                    let geometry = vector_geometry(&p.source).and_then(|spec| {
                        let h = ffi::day_xaml_vector_new(
                            cstr(&spec).as_ptr(),
                            mode,
                            p.tint.map(argb).unwrap_or(0),
                            c_int::from(p.tint.is_some()),
                        );
                        (!h.is_null()).then_some(h)
                    });
                    // Raster fallbacks: a monochrome BitmapIcon still honors a tint, and a
                    // plain Image carries the art as authored.
                    let tinted = || {
                        p.tint.and_then(|c| {
                            let file = icon_file_name(&p.source);
                            if file.is_empty() {
                                return None;
                            }
                            let h =
                                ffi::day_xaml_image_tinted_new(cstr(&file).as_ptr(), mode, argb(c));
                            (!h.is_null()).then_some(h)
                        })
                    };
                    WinHandle(geometry.or_else(tinted).unwrap_or_else(|| {
                        ffi::day_xaml_image_new(cstr(&image_uri(&p.source)).as_ptr(), mode)
                    }))
                }
                // A recycled list cell is ADOPTED from the native list, never realized
                // through this path; anything else is an extension piece.
                Some(Builtin::ListCell) | None => {
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
        h: &WinHandle,
        kind: PieceKind,
        patch: &dyn std::any::Any,
        anim: Option<&AnimSpec>,
    ) {
        unsafe {
            match kind {
                kinds::CONTAINER => {
                    if let Some(ContainerPatch::Background(c)) =
                        patch.downcast_ref::<ContainerPatch>()
                    {
                        // A cleared background maps to fully transparent (best-effort on XAML).
                        // Unlike the other desktop backends this one INTERPOLATES an animated fill
                        // (DESIGN.md §8.4): the fill is a SolidColorBrush we own, and XAML tweens
                        // brush color given EnableDependentAnimation.
                        let (dur, curve) = xaml_anim_args(anim);
                        ffi::day_xaml_container_animate_bg(
                            h.0,
                            argb(c.unwrap_or(day_spec::Color::CLEAR)),
                            dur,
                            curve,
                        );
                    }
                }
                kinds::LABEL => {
                    if let Some(p) = patch.downcast_ref::<LabelPatch>() {
                        match p {
                            LabelPatch::Text(t) => {
                                ffi::day_xaml_label_set_text(h.0, cstr(t).as_ptr())
                            }
                            LabelPatch::Font(f) => {
                                let (pt, weight, italic, tabular) = font_params(*f);
                                ffi::day_xaml_label_set_font(h.0, pt, weight, italic, tabular);
                                apply_custom_family(h.0, *f);
                            }
                            LabelPatch::Runs(text, runs) => {
                                let node = LABEL_NODE
                                    .with(|m| m.borrow().get(&(h.0 as usize)).copied())
                                    .unwrap_or(0);
                                set_label_runs(h.0, node, text, runs)
                            }
                            LabelPatch::Color(c) => ffi::day_xaml_label_set_color(
                                h.0,
                                argb(c.unwrap_or(day_spec::Color::CLEAR)),
                            ),
                        }
                    }
                }
                kinds::BUTTON => {
                    if let Some(p) = patch.downcast_ref::<ButtonPatch>() {
                        match p {
                            ButtonPatch::Title(t) => {
                                ffi::day_xaml_button_set_title(h.0, cstr(t).as_ptr())
                            }
                            ButtonPatch::Enabled(e) => ffi::day_xaml_set_enabled(h.0, *e as c_int),
                            ButtonPatch::Style(s) => apply_button_style(h.0, *s),
                        }
                    }
                }
                kinds::TOGGLE => {
                    if let Some(p) = patch.downcast_ref::<TogglePatch>() {
                        match p {
                            TogglePatch::On(on) => ffi::day_xaml_toggle_set(h.0, *on as c_int),
                            TogglePatch::Enabled(e) => ffi::day_xaml_set_enabled(h.0, *e as c_int),
                        }
                    }
                }
                kinds::SLIDER => {
                    if let Some(p) = patch.downcast_ref::<SliderPatch>() {
                        match p {
                            SliderPatch::Value(v) => ffi::day_xaml_slider_set(h.0, *v),
                            SliderPatch::Enabled(e) => ffi::day_xaml_set_enabled(h.0, *e as c_int),
                        }
                    }
                }
                kinds::PROGRESS => {
                    if let Some(ProgressPatch::Value(Some(v))) =
                        patch.downcast_ref::<ProgressPatch>()
                    {
                        ffi::day_xaml_progress_set(h.0, progress_ticks(*v));
                    }
                }
                kinds::LIST => match patch.downcast_ref::<ListPatch>() {
                    Some(ListPatch::Reload) => schedule_list_populate(h.0 as usize),
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
                    // Not implemented: RowSizeInvalidated — pooled cells re-measure on the next
                    // populate.
                    Some(ListPatch::RowSizeInvalidated(_)) | None => {}
                },
                kinds::NAV_MENU => {
                    // Split navs drive the NavigationView pane; a plain ListView otherwise.
                    let host = NAV_MENU_HOST.with(|m| m.borrow().get(&(h.0 as usize)).copied());
                    match patch.downcast_ref::<NavMenuPatch>() {
                        Some(NavMenuPatch::Selected(sel)) => {
                            let idx = sel.map(|i| i as c_int).unwrap_or(-1);
                            match host {
                                Some(nav) => ffi::day_xaml_nav_set_selected(nav, idx),
                                None => ffi::day_xaml_navlist_set_selected(h.0, idx),
                            }
                        }
                        // The row set changed (a filtered sidebar, a data-driven list). Without
                        // this the pane kept its original rows for the life of the window, and
                        // NAV_MENU_ROWS — which `measure` sizes the list from — went stale.
                        // Text badges and sections still have no NavigationView counterpart and
                        // are dropped here as at realize; the trailing status GLYPH does have one
                        // (it composes into the item's Content) and rides along below.
                        Some(NavMenuPatch::Items {
                            items,
                            icons,
                            tints,
                            badge_icons,
                            badge_tints,
                            selected,
                            ..
                        }) => {
                            let idx = selected.map(|i| i as c_int).unwrap_or(-1);
                            let joined = cstr(&items.join("\n"));
                            match host {
                                Some(nav) => {
                                    let icons_joined = icons
                                        .iter()
                                        .map(|ic| {
                                            ic.as_deref().map(icon_file_name).unwrap_or_default()
                                        })
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                    ffi::day_xaml_nav_set_items(
                                        nav,
                                        joined.as_ptr(),
                                        cstr(&icons_joined).as_ptr(),
                                        cstr(&join_geoms(icons)).as_ptr(),
                                        cstr(&join_tints(tints)).as_ptr(),
                                        cstr(
                                            &badge_icons
                                                .iter()
                                                .map(|ic| {
                                                    ic.as_deref()
                                                        .map(icon_file_name)
                                                        .unwrap_or_default()
                                                })
                                                .collect::<Vec<_>>()
                                                .join("\n"),
                                        )
                                        .as_ptr(),
                                        cstr(&join_geoms(badge_icons)).as_ptr(),
                                        cstr(&join_tints(badge_tints)).as_ptr(),
                                    );
                                    ffi::day_xaml_nav_set_selected(nav, idx);
                                }
                                None => {
                                    ffi::day_xaml_navlist_set_items(h.0, joined.as_ptr());
                                    ffi::day_xaml_navlist_set_selected(h.0, idx);
                                    NAV_MENU_ROWS
                                        .with(|m| m.borrow_mut().insert(h.0 as usize, items.len()));
                                }
                            }
                        }
                        None => {}
                    }
                }
                // Split navs show the current destination in the NavigationView Header (the whole
                // point of the Settings-like presentation); Pushed/Title carry that title. Two-pane
                // navs need no native work — NavLayout re-places the pages.
                // Emulated cover (docs/cover.md): present = re-home onto the content Canvas
                // (appended last = topmost) with an opaque theme-background surface; dismiss =
                // hide + report `CoverHidden` at once. No interactive dismissal here.
                kinds::COVER => {
                    if let Some(p) = patch.downcast_ref::<CoverPatch>() {
                        let node = COVER_IDS
                            .with(|m| m.borrow().get(&(h.0 as usize)).copied())
                            .unwrap_or(day_spec::WINDOW_NODE);
                        match p {
                            CoverPatch::Present { background, .. } => {
                                match background {
                                    Some(bg) => ffi::day_xaml_container_set_bg(h.0, argb(*bg)),
                                    None => ffi::day_xaml_cover_ground(h.0),
                                }
                                let root = ffi::day_xaml_window_root(self.window);
                                ffi::day_xaml_add_child(root, h.0);
                                let size = LAST_WINDOW_SIZE.with(|c| c.get());
                                ffi::day_xaml_set_geometry(
                                    h.0,
                                    0,
                                    0,
                                    size.width.round() as c_int,
                                    size.height.round() as c_int,
                                );
                                ffi::day_xaml_set_visible(h.0, 1);
                                COVERS.with(|c| c.borrow_mut().push((h.0, node)));
                                emit(node, Event::FrameChanged(size));
                            }
                            CoverPatch::DismissDisabled(_) => {}
                            CoverPatch::Dismiss => {
                                ffi::day_xaml_set_visible(h.0, 0);
                                COVERS.with(|c| c.borrow_mut().retain(|(w, _)| *w != h.0));
                                emit(node, Event::CoverHidden);
                            }
                        }
                    }
                }
                kinds::NAV => {
                    if let Some(np) = patch.downcast_ref::<NavPatch>() {
                        let title = match np {
                            NavPatch::Pushed { title, .. } => Some(title.as_str()),
                            NavPatch::Title(t) => Some(t.as_str()),
                            // The pop's header restore happens in stack_sync (after the page
                            // leaves detail_pages), where the new top's title is known.
                            NavPatch::Popped => None,
                            // NavigationView BackRequested already routes back through Day
                            // (never a native auto-pop), so nothing to suppress here.
                            NavPatch::GuardTop(_) => None,
                            // Unreachable: this backend answers `Cap::NavRepresent =
                            // Unsupported`, so the pieces layer never sends it. A NavigationView
                            // owns its own PaneDisplayMode, so re-presenting here means driving
                            // that rather than re-homing pages (docs/size-classes.md).
                            NavPatch::Presentation(_) => None,
                            // The resident-page switch (docs/navigation.md): show that
                            // destination and hide its siblings — the same visibility pass
                            // `stack_sync` makes, driven by the app's selection rather than depth.
                            // No header change: a tab bar names the destination itself.
                            NavPatch::Select(i) => {
                                let pages: Vec<*mut c_void> =
                                    NAV_STATE.with(|m| match m.borrow().get(&(h.0 as usize)) {
                                        Some(NavState::Split(st)) => {
                                            st.detail_pages.iter().map(|(p, _, _)| *p).collect()
                                        }
                                        _ => Vec::new(),
                                    });
                                for (n, page) in pages.iter().enumerate() {
                                    ffi::day_xaml_set_visible(*page, (n == *i) as c_int);
                                }
                                None
                            }
                        };
                        if let Some(title) = title {
                            let nav = NAV_STATE.with(|m| {
                                let mut m = m.borrow_mut();
                                match m.get_mut(&(h.0 as usize)) {
                                    Some(NavState::Split(s)) => {
                                        // Record on the top entry so a later pop can restore it.
                                        if let Some(top) = s.detail_pages.last_mut() {
                                            top.2 = title.to_string();
                                        }
                                        Some(s.nav_view)
                                    }
                                    _ => None,
                                }
                            });
                            if let Some(nav) = nav {
                                ffi::day_xaml_nav_set_header(nav, cstr(title).as_ptr());
                            }
                        }
                    }
                }
                kinds::PICKER => picker::update_any(self, h, patch),
                kinds::TEXT_AREA => textarea::update_any(self, h, patch),
                kinds::TEXT_FIELD => {
                    if let Some(p) = patch.downcast_ref::<TextFieldPatch>() {
                        match p {
                            TextFieldPatch::Text { text, from_native } => {
                                if !*from_native {
                                    ffi::day_xaml_textbox_set_text(h.0, cstr(text).as_ptr());
                                }
                            }
                            TextFieldPatch::Placeholder(t) => {
                                ffi::day_xaml_textbox_set_placeholder(h.0, cstr(t).as_ptr())
                            }
                            TextFieldPatch::Enabled(e) => {
                                ffi::day_xaml_set_enabled(h.0, *e as c_int)
                            }
                        }
                    }
                }
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
    fn release(&mut self, h: WinHandle) {
        // A released window content = that window is gone (docs/windows.md teardown): NOW
        // destroy the whole secondary window, never before (child releases come first).
        self.secondary.retain(|w| {
            if w.content == h.0 {
                unsafe { ffi::day_xaml_window_destroy2(w.win) };
                false
            } else {
                true
            }
        });
        // The PRIMARY window's root, released the same way now that it is an ordinary window
        // (docs/windows.md close policy). day-core has finished with its content, so the host
        // can go — and if that was the last primary, `quit_app` follows right behind.
        if !self.primary_root.is_null() && self.primary_root == h.0 {
            self.primary_root = std::ptr::null_mut();
            unsafe { ffi::day_xaml_destroy_primary() };
        }
        let key = h.0 as usize;
        LABEL_NODE.with(|m| m.borrow_mut().remove(&key));
        NAV_MENU_ROWS.with(|m| m.borrow_mut().remove(&key));
        NAV_PAGE_IDS.with(|m| m.borrow_mut().remove(&key));
        NAV_MENU_HOST.with(|m| m.borrow_mut().remove(&key));
        SPLIT_SIDEBAR_PAGES.with(|m| m.borrow_mut().remove(&key));
        if let Some(NavState::Split(s)) = NAV_STATE.with(|m| m.borrow_mut().remove(&key)) {
            NAV_HOST_BY_ID.with(|m| m.borrow_mut().retain(|_, v| *v != s.nav_view));
            unsafe { ffi::day_xaml_delete(s.content_host) };
        }
        // day-core never releases the adopted cell handles (docs/list.md: they are the host's own
        // cells, handed out through `adopt`), so the list host owns cell + content cleanup.
        if let Some(st) = LIST_STATE.with(|m| m.borrow_mut().remove(&key)) {
            // …and the node→host mapping the reorder callbacks resolve through, or a later list
            // that lands on this freed address answers drags aimed at the dead one.
            LIST_BY_NODE.with(|m| m.borrow_mut().remove(&st.node));
            for cell in st.cells {
                unsafe { ffi::day_xaml_delete(cell) };
            }
            unsafe { ffi::day_xaml_delete(st.content) };
        }
        // The scroll host's content Canvas is boxed separately from the ScrollViewer handle.
        if let Some(content) = SCROLL_STATE.with(|m| m.borrow_mut().remove(&key)) {
            unsafe { ffi::day_xaml_delete(content) };
        }
        GESTURES.with(|g| g.borrow_mut().retain(|(ptr, _)| *ptr != key));
        // A cover torn down while presented (no Dismiss patch first) must leave the
        // presented set — `window_resized` writes through every entry's raw pointer, and a
        // stale one is a use-after-free once `day_xaml_delete` runs below.
        COVERS.with(|c| c.borrow_mut().retain(|(w, _)| *w != h.0));
        COVER_IDS.with(|m| {
            m.borrow_mut().remove(&key);
        });
        // ONE sweep drops this handle from EVERY SideTable registered on this thread — the
        // textarea line bands today, and any table added later — so an element recycling the
        // freed address can't inherit the dead element's entries. (The explicit purges above
        // stay: they key by node id / value pairs, not this address.)
        day_spec::sidetable::sweep(key);
        unsafe { ffi::day_xaml_delete(h.0) };
    }

    fn insert(&mut self, parent: &WinHandle, child: &WinHandle, index: usize) {
        // Nav host: for a selector, page index 0 = sidebar (PaneHeader), the rest = detail. For a
        // stack, every page stacks in the content region.
        enum NavInsert {
            No,
            Done,
            /// A page landed in the content region: seed its frame. `stack` also re-syncs the
            /// stack's top-page visibility + back button.
            Content {
                node: NodeId,
                content_host: *mut c_void,
                stack: bool,
            },
        }
        let nav = NAV_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let Some(NavState::Split(s)) = m.get_mut(&(parent.0 as usize)) else {
                return NavInsert::No;
            };
            let node = NAV_PAGE_IDS
                .with(|ids| ids.borrow().get(&(child.0 as usize)).copied())
                .unwrap_or(NodeId(0));
            if !s.is_stack && index == 0 {
                // The sidebar page (day's logo/title header piece) → the NavigationView's
                // PaneHeader; clipped to a fixed height by set_frame.
                unsafe { ffi::day_xaml_nav_set_pane_header(s.nav_view, child.0) };
                s.sidebar_page = Some((child.0, node));
                SPLIT_SIDEBAR_PAGES.with(|p| p.borrow_mut().insert(child.0 as usize));
                NavInsert::Done
            } else {
                // Detail / stack page → nv.Content (day positions it by absolute frame).
                unsafe { ffi::day_xaml_add_child(s.content_host, child.0) };
                s.detail_pages.push((child.0, node, String::new()));
                NavInsert::Content {
                    node,
                    content_host: s.content_host,
                    stack: s.is_stack,
                }
            }
        });
        match nav {
            NavInsert::No => {}
            NavInsert::Done => return,
            NavInsert::Content {
                node,
                content_host,
                stack,
            } => {
                // The NavigationView content region is already sized, and adding a child won't
                // refire its SizeChanged — so seed the new page with the current content bounds
                // (else NavLayout would fall back to the split size). Emitted outside the NAV_STATE
                // borrow (FrameChanged re-enters the tree).
                let (mut w, mut h) = (0.0, 0.0);
                unsafe { ffi::day_xaml_widget_size(content_host, &mut w, &mut h) };
                if w > 0.0 && h > 0.0 {
                    emit(node, Event::FrameChanged(Size::new(w, h)));
                }
                if stack {
                    stack_sync(parent.0);
                }
                return;
            }
        }
        // Scroll host: children live in the inner content Canvas, not the ScrollViewer itself.
        let target = SCROLL_STATE
            .with(|m| m.borrow().get(&(parent.0 as usize)).copied())
            .unwrap_or(parent.0);
        unsafe { ffi::day_xaml_add_child(target, child.0) };
    }

    fn remove(&mut self, parent: &WinHandle, child: &WinHandle) {
        // Nav pages live in a pane / the NavigationView content — remove from wherever they landed.
        let removed = NAV_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let Some(NavState::Split(s)) = m.get_mut(&(parent.0 as usize)) else {
                return None;
            };
            s.detail_pages.retain(|&(p, _, _)| p != child.0);
            SPLIT_SIDEBAR_PAGES.with(|p| p.borrow_mut().remove(&(child.0 as usize)));
            unsafe { ffi::day_xaml_remove_child(s.content_host, child.0) };
            Some(s.is_stack)
        });
        match removed {
            Some(true) => {
                // A stack page popped: re-show the new top + refresh the back button.
                stack_sync(parent.0);
                return;
            }
            Some(false) => return,
            None => {}
        }
        let target = SCROLL_STATE
            .with(|m| m.borrow().get(&(parent.0 as usize)).copied())
            .unwrap_or(parent.0);
        unsafe { ffi::day_xaml_remove_child(target, child.0) };
    }

    fn move_child(&mut self, _parent: &WinHandle, _child: &WinHandle, _to: usize) {
        // Absolute frames don't overlap: sibling z-order is irrelevant.
    }

    fn measure(&mut self, h: &WinHandle, kind: PieceKind, p: Proposal) -> Size {
        match kind {
            kinds::LABEL => {
                let nat = natural(h.0);
                match p.width {
                    Some(pw) if nat.width > pw => {
                        // Height-for-width: re-measure wrapped at the proposed width.
                        let mut w = 0.0;
                        let mut hh = 0.0;
                        unsafe { ffi::day_xaml_measure(h.0, pw, -1.0, &mut w, &mut hh) };
                        Size::new(pw.ceil(), hh.ceil())
                    }
                    _ => Size::new(nat.width.ceil(), nat.height.ceil()),
                }
            }
            // Buttons hug their text like every other toolkit: the generic arm would take a
            // COLUMN's cross-axis width proposal and stretch the button across the full span.
            kinds::BUTTON => {
                let nat = natural(h.0);
                Size::new(nat.width.ceil(), nat.height.ceil())
            }
            kinds::SLIDER => Size::new(p.width.unwrap_or(180.0), natural(h.0).height.max(24.0)),
            kinds::PICKER => picker::measure_any(self, h, p),
            kinds::TEXT_AREA => textarea::measure_any(self, h, p),
            kinds::TEXT_FIELD => Size::new(p.width.unwrap_or(180.0), natural(h.0).height.max(28.0)),
            kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 1.0),
            // The list host fills whatever frame layout gives it; cells fill its width.
            kinds::LIST => Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0)),
            kinds::NAV_MENU => {
                // Split navs render the menu as the NavigationView's own pane, so the day node is an
                // invisible placeholder that must take no layout space.
                if NAV_MENU_HOST.with(|m| m.borrow().contains_key(&(h.0 as usize))) {
                    return Size::new(0.0, 0.0);
                }
                let rows =
                    NAV_MENU_ROWS.with(|m| m.borrow().get(&(h.0 as usize)).copied().unwrap_or(0));
                Size::new(
                    p.width.unwrap_or(220.0),
                    p.height.unwrap_or(rows as f64 * 40.0 + 8.0),
                )
            }
            kinds::PROGRESS => {
                // Determinate bar fills the proposed width; the indeterminate ring is square.
                let nat = natural(h.0);
                Size::new(p.width.unwrap_or(nat.width.max(20.0)), nat.height.max(6.0))
            }
            _ => {
                if let Some(measure) = self.registry.get(kind).and_then(|r| r.measure) {
                    return measure(self, h, p);
                }
                let nat = natural(h.0);
                Size::new(
                    p.width.unwrap_or(nat.width).ceil(),
                    p.height.unwrap_or(nat.height).ceil(),
                )
            }
        }
    }

    fn set_selectable(&mut self, h: &WinHandle, selectable: bool) -> Option<WinHandle> {
        // The shim try_as's to a TextBlock, so a non-label handle is a safe no-op (docs/text.md).
        unsafe { ffi::day_xaml_label_set_selectable(h.0, selectable as c_int) };
        None
    }

    // Animatable visual channels (DESIGN.md §8.4): cheap per-node opacity + transform that don't
    // relayout. `anim = Some` hands the target to XAML's compositor as a Storyboard; `None` sets it
    // outright. Day never ticks these frames itself (§0.3).
    fn set_opacity(&mut self, h: &WinHandle, opacity: f64, anim: Option<&AnimSpec>) {
        let (dur, curve) = xaml_anim_args(anim);
        unsafe { ffi::day_xaml_set_opacity(h.0, opacity, dur, curve) };
    }

    fn set_transform(&mut self, h: &WinHandle, t: Transform, _size: Size, anim: Option<&AnimSpec>) {
        // A CompositeTransform about the element's center — the same anchor AppKit's layer and
        // Qt's painter transform use, so a rotated/scaled box matches across backends.
        let (dur, curve) = xaml_anim_args(anim);
        unsafe {
            ffi::day_xaml_set_transform(h.0, t.tx, t.ty, t.sx, t.sy, t.rotate_deg, dur, curve)
        };
    }

    /// A `TextBlock` publishes `BaselineOffset`; every other control keeps its text inside a
    /// template that is not built until the control is in the visual tree, so the shim derives
    /// those from the control's font size and its own padding/border (docs/baseline.md). Hence
    /// `Emulated` — accurate for the Segoe faces XAML ships, not read off the platform.
    fn first_baseline(&mut self, h: &WinHandle, kind: PieceKind, size: Size) -> Option<f64> {
        if !day_spec::kind_has_baseline(kind) {
            return None;
        }
        let b = unsafe { ffi::day_xaml_baseline(h.0, size.height) };
        (b >= 0.0).then_some(b)
    }

    fn set_frame(&mut self, h: &WinHandle, frame: Rect, _anim: Option<&AnimSpec>) {
        // A split nav's sidebar page IS the NavigationView's PaneHeader: clip it to a fixed header
        // height (a Canvas has no desired size) so day's logo/title piece sits at the pane top; the
        // NavigationView owns everything below it. Width follows the frame day proposes.
        if SPLIT_SIDEBAR_PAGES.with(|m| m.borrow().contains(&(h.0 as usize))) {
            unsafe {
                ffi::day_xaml_set_geometry(
                    h.0,
                    0,
                    0,
                    frame.size.width.round() as c_int,
                    NAV_PANE_HEADER_H,
                )
            };
            return;
        }
        unsafe {
            ffi::day_xaml_set_geometry(
                h.0,
                frame.origin.x.round() as c_int,
                frame.origin.y.round() as c_int,
                frame.size.width.round() as c_int,
                frame.size.height.round() as c_int,
            )
        };
        // (Nav hosts are NavigationViews — they reflow their own regions, which report FrameChanged.)
        // List host framed: (re)fill its cells — but ONLY when the width actually changed, so the
        // set_frames a populate itself makes (on row content) don't schedule another forever.
        let framed = frame.size.width.round() as c_int;
        let width_changed = LIST_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let Some(st) = m.get_mut(&(h.0 as usize)) else {
                return false;
            };
            // Remember the assigned width for the populate this schedules: the host's own
            // `ActualWidth` is still the PREVIOUS pass's value at that point (see ListEntry).
            st.frame_width = framed;
            st.last_width != framed
        });
        if width_changed {
            schedule_list_populate(h.0 as usize);
        }
    }

    // Scroll (docs §7.6): size the ScrollViewer's inner content Canvas to the content extent so it
    // clips + scrolls; the offset/scroll-to operate on the ScrollViewer handle directly.
    fn set_scroll_content(&mut self, h: &WinHandle, content: Size) {
        if let Some(c) = SCROLL_STATE.with(|m| m.borrow().get(&(h.0 as usize)).copied()) {
            unsafe {
                ffi::day_xaml_scroll_set_content_size(
                    c,
                    content.width.round() as c_int,
                    content.height.round() as c_int,
                )
            };
        }
    }

    fn scroll_to(&mut self, h: &WinHandle, target: Rect, animated: bool) {
        unsafe {
            ffi::day_xaml_scroll_to(
                h.0,
                target.origin.y.round() as c_int,
                target.size.height.round() as c_int,
                animated as c_int,
            )
        };
    }

    fn scroll_offset(&mut self, h: &WinHandle) -> Point {
        let (mut x, mut y) = (0.0_f64, 0.0_f64);
        unsafe { ffi::day_xaml_scroll_offset(h.0, &mut x, &mut y) };
        Point::new(x, y)
    }

    fn enable_gesture(&mut self, h: &WinHandle, node: NodeId, kind: day_spec::GestureKind) {
        let k = match kind {
            day_spec::GestureKind::Tap => 0,
            day_spec::GestureKind::LongPress => 1,
            day_spec::GestureKind::Drag => 2,
        };
        // Idempotent per (handle, kind) — day-core may re-enable on rebuild.
        if !GESTURES.with(|g| g.borrow_mut().insert((h.0 as usize, k))) {
            return;
        }
        unsafe { ffi::day_xaml_enable_gesture(h.0, node.0, k, on_gesture) };
    }

    fn focus(&mut self, h: &WinHandle, _node: NodeId, focused: bool) {
        // The shim resigns to the window's invisible focus sink — system XAML has no
        // "focus nothing" — and only while this control still owns focus.
        unsafe { ffi::day_xaml_control_focus(h.0, focused as c_int) };
    }

    fn set_event_sink(&mut self, sink: EventSink) {
        SINK.with(|s| *s.borrow_mut() = Some(Rc::from(sink)));
    }

    fn set_a11y(&mut self, h: &WinHandle, a11y: &A11yProps) {
        if let Some(id) = &a11y.identifier {
            unsafe { ffi::day_xaml_set_name(h.0, cstr(id).as_ptr()) };
        }
    }

    fn attach_list(&mut self, host: &WinHandle, source: day_spec::ListSource) {
        LIST_STATE.with(|m| {
            if let Some(st) = m.borrow().get(&(host.0 as usize)) {
                *st.source.borrow_mut() = Some(source);
            }
        });
        // Deferred (see schedule_list_populate): populating re-enters with_tree via bind_row.
        schedule_list_populate(host.0 as usize);
    }

    fn set_context_menu(&mut self, h: &WinHandle, _node: NodeId, items: &[day_spec::MenuItem]) {
        // The RAW items, verbatim: a right-click menu is the app's own tree, never the menu
        // bar — `standard_menu_bar_for` here (a copy-paste from `set_app_menu`) injected
        // File/Edit/View/Help slot-filling into every context menu.
        let mut spec = String::new();
        serialize_menu_xaml(items, &mut spec);
        unsafe { ffi::day_xaml_set_context_menu(h.0, cstr(&spec).as_ptr()) };
    }

    fn set_toolbar(&mut self, h: &WinHandle, items: &[day_spec::ToolbarItem]) {
        self.install_toolbar(h, items);
    }

    fn update_toolbar(&mut self, h: &WinHandle, patch: &day_spec::ToolbarPatch) {
        self.patch_toolbar(h, patch);
    }

    fn set_app_menu(&mut self, items: &[day_spec::MenuItem]) {
        if self.window.is_null() {
            return;
        }
        // Windows' bar: File, Edit, View, the app's own menus, then Help. No Window menu (MDI
        // is long gone) and no app menu — Exit lives in File, About in Help.
        let items = day_core::menu::standard_menu_bar_for(
            day_core::menu::MenuBarStyle::Windows,
            items.to_vec(),
        );
        let mut spec = String::new();
        serialize_menu_xaml(&items, &mut spec);
        // Kept, because day's app menu is installed ONCE but Windows draws it per window: a
        // window opened later has to be given the menu that was set before it existed
        // (see `open_window`). Re-set on a locale change, so the stored spec stays current.
        self.menu_spec = spec.clone();
        let c = cstr(&spec);
        unsafe { ffi::day_xaml_set_app_menu(self.window, c.as_ptr()) };
        // Every window already open shows the same bar — the menu is app-level, not per-window.
        // A Preferences panel is the exception and stays bare.
        for w in self.secondary.iter().filter(|w| !w.menuless) {
            unsafe { ffi::day_xaml_window_set_menu2(w.win, c.as_ptr()) };
        }
    }

    fn present(&mut self, req: u64, spec: &day_spec::present::PresentSpec) {
        use day_spec::present::PresentSpec;
        match spec {
            PresentSpec::Dialog { .. } => unsafe {
                ffi::day_xaml_present_dialog(
                    req,
                    cstr(spec.title()).as_ptr(),
                    cstr(spec.message().unwrap_or("")).as_ptr(),
                    cstr(&spec.buttons_joined()).as_ptr(),
                    cstr(&spec.roles_joined()).as_ptr(),
                    self.window,
                )
            },
            PresentSpec::Prompt {
                placeholder,
                initial,
                ok,
                cancel,
                ..
            } => unsafe {
                ffi::day_xaml_present_prompt(
                    req,
                    cstr(spec.title()).as_ptr(),
                    cstr(spec.message().unwrap_or("")).as_ptr(),
                    cstr(placeholder).as_ptr(),
                    cstr(initial).as_ptr(),
                    cstr(ok).as_ptr(),
                    cstr(cancel).as_ptr(),
                    self.window,
                )
            },
            PresentSpec::OpenFile { .. } => unsafe {
                ffi::day_xaml_present_file_open(
                    req,
                    cstr(spec.title()).as_ptr(),
                    cstr(&spec.filters_joined()).as_ptr(),
                    self.window,
                )
            },
            PresentSpec::SaveFile { suggested_name, .. } => unsafe {
                // The pieces layer copies the staged bytes to the chosen path (docs/files.md).
                ffi::day_xaml_present_file_save(
                    req,
                    cstr(spec.title()).as_ptr(),
                    cstr(suggested_name).as_ptr(),
                    cstr(&spec.filters_joined()).as_ptr(),
                    self.window,
                )
            },
        }
    }

    fn dismiss(&mut self, req: u64) {
        unsafe { ffi::day_xaml_dismiss_present(req) };
    }

    fn open_url(&mut self, url: &str) {
        let c = cstr(url);
        unsafe { ffi::day_xaml_open_url(c.as_ptr()) };
    }

    fn adopt(&mut self, raw: day_spec::RawHandle) -> WinHandle {
        // A recycling-list cell (a plain Canvas) — Day builds/rebinds its row content in place.
        WinHandle(raw)
    }

    fn replay(&mut self, h: &WinHandle, ops: &[DrawOp], _size: Size) {
        let (nums, texts) = day_spec::encode_ops(ops);
        let joined = cstr(&texts.join("\u{1f}"));
        unsafe {
            ffi::day_xaml_canvas_set_ops(h.0, nums.as_ptr(), nums.len() as c_int, joined.as_ptr())
        };
    }

    fn open_window(
        &mut self,
        id: NodeId,
        options: &WindowOptions,
        kind: day_spec::WindowKind,
    ) -> day_spec::WindowOpenReply<WinHandle> {
        let fixed = (kind == day_spec::WindowKind::Preferences) as c_int;
        let win = unsafe {
            ffi::day_xaml_window_new2(
                cstr(&options.title).as_ptr(),
                options.size.width as c_int,
                options.size.height as c_int,
                id.0,
                fixed,
            )
        };
        if win.is_null() {
            return day_spec::WindowOpenReply::Unsupported;
        }
        let content = unsafe { ffi::day_xaml_window_content2(win) };
        // The app menu was installed before this window existed, and Windows draws one per
        // window — so replay it here rather than leaving the new window bare.
        //
        // Except into a Preferences window: that is a panel, not a second main window
        // (docs/windows.md), and a settings dialog carrying File/Edit/View is not an idiom
        // Windows has. It keeps the menu-less frame the shim gives it.
        let menuless = kind == day_spec::WindowKind::Preferences;
        if !menuless && !self.menu_spec.is_empty() {
            unsafe { ffi::day_xaml_window_set_menu2(win, cstr(&self.menu_spec).as_ptr()) };
        }
        self.secondary.push(XamlWin {
            win,
            content,
            menuless,
        });
        day_spec::WindowOpenReply::Open(WinHandle(content))
    }

    fn close_window(&mut self, host: &WinHandle) {
        if let Some(w) = self.secondary.iter().find(|w| w.content == host.0) {
            unsafe { ffi::day_xaml_window_close2(w.win) };
        }
    }

    fn quit_app(&mut self) {
        // Just the platform's exit: day-core has already disposed the other windows and
        // delivered WillTerminate (docs/windows.md close policy).
        unsafe { ffi::day_xaml_quit() };
    }

    fn focus_window(&mut self, host: &WinHandle) {
        if let Some(w) = self.secondary.iter().find(|w| w.content == host.0) {
            unsafe { ffi::day_xaml_window_raise2(w.win) };
        }
    }

    fn set_window_title(&mut self, host: &WinHandle, title: &str) {
        if let Some(w) = self.secondary.iter().find(|w| w.content == host.0) {
            unsafe { ffi::day_xaml_window_set_title2(w.win, cstr(title).as_ptr()) };
        }
    }

    /// A SECONDARY window's own pixels (docs/windows.md). Previously this kept the trait default,
    /// which captures the primary — so a `screenshot: { window: day.preferences }` step passed
    /// while writing an image of the main window, with nothing to indicate the target was ignored.
    fn snapshot_window_of(&mut self, host: &WinHandle) -> Result<Vec<u8>, String> {
        let Some(w) = self.secondary.iter().find(|w| w.content == host.0) else {
            // Not one of ours: the primary's own root arrives here too.
            return self.snapshot_window();
        };
        let win = w.win;
        snapshot_via(|path| unsafe { ffi::day_xaml_snapshot_png2(win, path) })
    }

    fn toggle_sidebar(&mut self) -> bool {
        // The same call the toolbar's own AppBarButton makes, so a dayscript walkthrough drives
        // the real path (docs/toolbars.md).
        unsafe { ffi::day_xaml_toggle_sidebar() != 0 }
    }

    /// The app's appearance override (docs/appearance.md) — what the showcase's Preferences
    /// Appearance picker drives. Without this the backend took the trait default, which ignores
    /// the call: picking Light or Dark changed nothing at all, while `Cap::Appearance` promised
    /// the opposite. `dark_mode` below answers from the same override, so day's palette and the
    /// native controls agree.
    fn set_appearance(&mut self, dark: Option<bool>) {
        let mode = match dark {
            None => 0,        // follow the system again
            Some(false) => 1, // light
            Some(true) => 2,  // dark
        };
        unsafe { ffi::day_xaml_set_appearance(mode) };
    }

    /// The system's light/dark setting (a `DAY_THEME` force wins). Previously this took the trait
    /// default, which reads DAY_THEME and nothing else — so on Windows every palette closure day
    /// evaluated resolved LIGHT no matter what the system was set to, while the XAML controls
    /// around it themed themselves correctly. That mismatch is what made a dark-mode window show
    /// app-painted surfaces in light colors.
    fn dark_mode(&mut self) -> bool {
        unsafe { ffi::day_xaml_is_dark() != 0 }
    }

    fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
        if self.window.is_null() {
            return Err("no window".into());
        }
        let win = self.window;
        snapshot_via(|path| unsafe { ffi::day_xaml_snapshot_png(win, path) })
    }
}

/// Run one of the shim's PNG writers into a temp file and hand back the bytes. The primary and
/// secondary captures differ only in which entry point they call, so the file dance lives here.
/// One name per process is enough: captures are sequential, and the file is read and removed
/// before the next one starts.
fn snapshot_via(capture: impl FnOnce(*const c_char) -> c_int) -> Result<Vec<u8>, String> {
    let path = std::env::temp_dir().join(format!("day-xaml-snap-{}.png", std::process::id()));
    let cpath = cstr(&path.to_string_lossy());
    let rc = capture(cpath.as_ptr());
    if rc != 0 {
        return Err(format!("snapshot failed (rc={rc})"));
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&path);
    Ok(bytes)
}

fn argb(c: day_spec::Color) -> u32 {
    let a = (c.a.clamp(0.0, 1.0) * 255.0) as u32;
    let r = (c.r.clamp(0.0, 1.0) * 255.0) as u32;
    let g = (c.g.clamp(0.0, 1.0) * 255.0) as u32;
    let b = (c.b.clamp(0.0, 1.0) * 255.0) as u32;
    (a << 24) | (r << 16) | (g << 8) | b
}

/// Per-row nav icon tints (docs/vectors.md) as one line-joined ARGB list, parallel to the rows.
/// `0` is the untinted row — fully transparent is not a color anyone can mean, and it keeps the
/// list positional so a row without a tint cannot shift the ones after it.
fn join_tints(tints: &[Option<day_spec::Color>]) -> String {
    tints
        .iter()
        .map(|t| t.map(argb).unwrap_or(0).to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A vector NAME's staged XAML geometry (docs/vectors.md), or `None` when the name is not a
/// vector or its art was outside the convertible subset — the caller then draws the raster.
pub(crate) fn vector_geometry(name: &str) -> Option<String> {
    let path = day_spec::resource::resolve_vector_xaml(name)?;
    std::fs::read_to_string(path).ok()
}

/// Per-row nav geometry, parallel to the rows. The specs are multi-line, and the FFI carries one
/// row per line, so each spec's newlines ride as `\x1f` (a unit separator cannot occur in path
/// data or a color) and the shim puts them back.
fn join_geoms(icons: &[Option<String>]) -> String {
    icons
        .iter()
        .map(|ic| {
            ic.as_deref()
                .and_then(vector_geometry)
                .map(|s| s.replace('\n', "\x1f"))
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Resolve an image NAME to a `file:///` URI the XAML `BitmapImage` can load (§18.3).
///
/// Unpackaged Win32 + XAML has no MRT/`.pri` resource store (that path is packaged/MSIX-only), so
/// images resolve to the loose files `day build` stages next to the exe under `images/` then
/// `assets/`. The shared [`resolve_image_file`](day_spec::resource::resolve_image_file) does that
/// lookup (probing `DAY_IMAGE_ROOT`/`DAY_ASSET_ROOT` for dev/`day launch` runs, then the exe-relative
/// dirs, inferring the extension), exactly as the AppKit/GTK/Qt backends do. An unresolved name
/// yields `""`, which `day_xaml_image_new` renders as an empty placeholder — the prior behavior.
/// A nav-icon name → the bundled file's NAME (e.g. "nav_controls" → "nav_controls.png"), which the
/// shim loads as `ms-appx:///images/<file>` (staged by `stage_bundled_images`). Empty if unresolved.
fn icon_file_name(name: &str) -> String {
    day_spec::resource::resolve_image_file(name)
        .as_deref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn image_uri(source: &str) -> String {
    // Pass the resolved NATIVE path to the shim, which reads the bytes and `SetSource`s a
    // BitmapImage — the system-XAML image loader does NOT accept `file://` (or bare-path) URIs (a
    // UWP restriction that carries into XAML Islands), so the old `file:///…` Uri silently loaded
    // nothing. An http(s) source (if ever resolved to one) is passed through and loaded as a Uri.
    match day_spec::resource::resolve_image_file(source) {
        Some(p) => p.to_string_lossy().into_owned(),
        None => String::new(),
    }
}

// Secondary-window event trampolines (docs/windows.md); px == points (the v1 100%-scale
// convention, same as `window_resized`).
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
/// The primary window's close — the same event a secondary window reports, addressed to the
/// root node day-core adopted it under (docs/windows.md close policy).
extern "C" fn primary_closed() {
    ffi_guard::contain((), || emit(day_spec::WINDOW_NODE, Event::WindowClosed));
}
/// The user flipped Windows between light and dark: re-read the setting into day's dark signal so
/// palette closures recolor live, the same way GTK's StyleManager `dark` notify drives it.
extern "C" fn appearance_changed() {
    ffi_guard::contain((), day_core::note_appearance_changed);
}
extern "C" fn win_focused(node: u64, active: c_int) {
    ffi_guard::contain((), || {
        emit(day_spec::NodeId(node), Event::WindowFocused(active != 0))
    });
}

extern "C" fn window_resized(w: c_int, h: c_int) {
    // Client rect is reported in pixels; day-xaml's v1 assumes a 100% scale factor
    // throughout (same convention as window creation).
    ffi_guard::contain((), || {
        let size = Size::new(w as f64, h as f64);
        LAST_WINDOW_SIZE.with(|c| c.set(size));
        emit(day_spec::WINDOW_NODE, Event::WindowResized(size));
        // Presented emulated covers track the content area (docs/cover.md).
        COVERS.with(|c| {
            for (cover, node) in c.borrow().iter() {
                unsafe {
                    ffi::day_xaml_set_geometry(
                        *cover,
                        0,
                        0,
                        size.width.round() as c_int,
                        size.height.round() as c_int,
                    )
                };
                emit(*node, Event::FrameChanged(size));
            }
        });
    });
}

thread_local! {
    /// Last reported window content size (seeds cover frames at Present).
    static LAST_WINDOW_SIZE: std::cell::Cell<Size> =
        const { std::cell::Cell::new(Size::new(0.0, 0.0)) };
    /// Presented emulated covers: (element, NodeId).
    static COVERS: RefCell<Vec<(*mut c_void, NodeId)>> = const { RefCell::new(Vec::new()) };
    /// Cover element → NodeId (set at realize).
    static COVER_IDS: RefCell<HashMap<usize, NodeId>> = RefCell::new(HashMap::new());
}

extern "C" fn run_posted(data: *mut c_void) {
    // The posted-closure trampoline runs arbitrary Rust (deferred emits, list populates) —
    // contained like every other FFI entry (day-spec's ffi_guard).
    let f: Box<Box<dyn FnOnce() + Send>> = unsafe { Box::from_raw(data as *mut _) };
    ffi_guard::contain((), f);
}

// A native modal answered (docs/dialogs.md): the shim reports (req, tag, index, text); decode into a
// PresentResult and route it to the window node, where day-core's executor resolves the future.
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
        emit(day_spec::WINDOW_NODE, Event::PresentResult { req, result });
    });
}

// A native pointer recognizer fired (docs/shapes.md). Phase codes match the shim's
// day_xaml_enable_gesture: 0 Tap, 1/2/3 Drag Began/Changed/Ended, 4 LongPress. `x,y` are the
// node-local location; `tx,ty` the cumulative drag translation.
extern "C" fn on_gesture(
    id: u64,
    phase: c_int,
    x: c_double,
    y: c_double,
    tx: c_double,
    ty: c_double,
) {
    use day_spec::DragPhase;
    ffi_guard::contain((), || {
        let at = Point::new(x, y);
        let ev = match phase {
            0 => Event::Tap(at),
            4 => Event::LongPress(at),
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
            _ => Event::Drag {
                phase: DragPhase::Changed,
                location: at,
                translation: Point::new(tx, ty),
            },
        };
        emit(NodeId(id), ev);
    });
}

impl Platform for Xaml {
    const TARGET: &'static str = "windows-xaml";
    const TOOLKIT: &'static str = "xaml";

    fn run(mut self, options: WindowOptions, ready: Box<dyn FnOnce(Self, WinHandle, Size)>) {
        unsafe {
            let (min_w, min_h) = options
                .min_size
                .map(|s| (s.width as c_int, s.height as c_int))
                .unwrap_or((0, 0));
            let win = ffi::day_xaml_window_new(
                cstr(&options.title).as_ptr(),
                options.size.width as c_int,
                options.size.height as c_int,
                min_w,
                min_h,
            );
            if win.is_null() {
                eprintln!("day-xaml: could not create the XAML window (see error above)");
                return;
            }
            self.window = win;
            // Bundled fonts (§18.4): stage every file into `<exe>/fonts/` before the app builds its
            // tree, so `Font::Custom` families resolve via `ms-appx:///fonts/…` inside XAML.
            stage_bundled_fonts();
            // Nav icons load as ms-appx BitmapIcons, so stage the project's images/ next to the exe.
            stage_bundled_images();
            // Taskbar/title icon (§18.2): the .ico `day launch` resolved from icons/windows/.
            if let Ok(icon) = std::env::var("DAY_APP_ICON") {
                ffi::day_xaml_set_app_icon(win, cstr(&icon).as_ptr());
            }
            ffi::day_xaml_set_menu_cb(on_menu_action);
            ffi::day_xaml_label_link_cb(on_link);
            ffi::day_xaml_set_toolbar_cb(toolbar::on_toolbar_value);
            ffi::day_xaml_set_lifecycle_cb(on_lifecycle);
            ffi::day_xaml_set_present_cb(present_cb);
            let root = ffi::day_xaml_window_root(win);
            self.primary_root = root;
            ready(self, WinHandle(root), options.size);
            ffi::day_xaml_window_on_resize(win, window_resized);
            ffi::day_xaml_set_primary_closed_cb(primary_closed);
            ffi::day_xaml_set_appearance_cb(appearance_changed);
            ffi::day_xaml_set_window_events_cb(win_resized, win_closed, win_focused);
            ffi::day_xaml_window_show(win);
            ffi::day_xaml_run(win);
        }
    }

    fn post(f: Box<dyn FnOnce() + Send>) {
        let data = Box::into_raw(Box::new(f)) as *mut c_void;
        unsafe { ffi::day_xaml_post(run_posted, data) };
    }
}
