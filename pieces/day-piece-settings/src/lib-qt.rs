// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Qt: KDE's settings idiom (docs/settings.md), the Preferences window's body.
//
// `window_shell` returns the structure `KConfigDialog` / `KPageDialog` in its list face are
// made of (the shape of Plasma's System Settings pages): a fixed-width `QListWidget` of
// icon+label page entries beside a `QStackedWidget` of pages under a title header, each page a
// column of `QGroupBox`es (one per section: the title as the box title, the footer a dim label
// under the rows) holding a `QFormLayout` of right-aligned labels and fields. One visible group
// is the page alone, with no list and no header. The widgets are the shim's
// (src/lib-qt-shim.cpp, compiled by build.rs); this file is the Day side: the declaration
// walked into shim calls, the slots, the patches, the events.
//
// Day mounts a piece's children into the renderer's widget and positions them from its own
// layout, which cannot know where a `QFormLayout` puts a field. So every control is wrapped in
// a SLOT, a one-child `QWidget` node of its own, and the slot is moved out of the host into a
// CELL the shim placed in the form (`day_settings_qt_slot_mount`); the cell takes the size Day
// measured for the slot (`day_settings_qt_cell_set_size`), since a layout overrides
// `setGeometry` on what it manages, and the form lays the cell out like any field. The control
// stays a Day node with its element id, its enabled binding and its scripting surface. A
// spanning cell (a custom row, a group's custom piece) reports the width the form granted, and
// Day re-lays the piece at that width. A row with no control but an element id (navigate, link,
// action) keeps the id on a 1×1 marker slot that never leaves the host; dayscript's `tap:` on
// the id is a Day event, so the marker runs the row's action like the native button does.
//
// The window itself is Day's `day.preferences` singleton (`open_preferences` focuses it,
// dayscript captures it by key), which day-qt opens fixed-size and `fit_window` shrinks to the
// shell's measured height. About stays the composed Day window.
// ---------------------------------------------------------------------------

use super::*;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};

use day_core::{Boundary, BuildCx, Flex, Layout, LayoutOps, RNode, with_tree};
use day_qt::{Qt, QtHandle};
use day_spec::{Event, NodeId, Proposal, Rect, Size, ffi_guard};

unsafe extern "C" {
    fn day_settings_qt_shell_new(
        node: u64,
        sidebar: c_int,
        on_select: extern "C" fn(u64, c_int),
        on_row: extern "C" fn(u64, u64),
        on_fill: extern "C" fn(u64, u64, f64),
    ) -> *mut c_void;
    fn day_settings_qt_shell_add_page(
        host: *mut c_void,
        title: *const c_char,
        icon_spec: *const c_char,
        icon_fallback: c_int,
    ) -> c_int;
    fn day_settings_qt_page_add_group(
        host: *mut c_void,
        page: c_int,
        title: *const c_char,
        footer: *const c_char,
    ) -> *mut c_void;
    fn day_settings_qt_group_add_row(
        host: *mut c_void,
        group: *mut c_void,
        key: u64,
        kind: c_int,
        label: *const c_char,
        text: *const c_char,
        caption: *const c_char,
        arrow: c_int,
        destructive: c_int,
        with_cell: c_int,
        enabled: c_int,
    ) -> *mut c_void;
    fn day_settings_qt_shell_done(host: *mut c_void, selected: c_int);
    fn day_settings_qt_slot_mount(cell: *mut c_void, slot: *mut c_void);
    fn day_settings_qt_cell_set_size(cell: *mut c_void, w: f64, h: f64);
    fn day_settings_qt_shell_measure(
        host: *mut c_void,
        for_w: f64,
        out_w: *mut f64,
        out_h: *mut f64,
    );
    fn day_settings_qt_shell_select(host: *mut c_void, page: c_int);
    fn day_settings_qt_shell_set_page_title(host: *mut c_void, page: c_int, title: *const c_char);
    fn day_settings_qt_group_set_title(group: *mut c_void, title: *const c_char);
    fn day_settings_qt_group_set_footer(group: *mut c_void, footer: *const c_char);
    fn day_settings_qt_row_set_text(host: *mut c_void, key: u64, which: c_int, text: *const c_char);
    fn day_settings_qt_row_set_enabled(host: *mut c_void, key: u64, on: c_int);
    // From day-qt-sys (already linked into the binary).
    fn day_qt_container_new() -> *mut c_void;
    fn day_qt_set_visible(w: *mut c_void, visible: c_int);
}

/// The shell's kind under `day_qt::RENDERERS`.
const SHELL_KIND: &str = "day-piece-settings.panel";
/// A slot's kind.
const SLOT_KIND: &str = "day-piece-settings.slot";
/// The list picked a page: `text` is the group id.
const SELECT_TAG: &str = "day-piece-settings:select";
/// A native button or link was activated: `text` is the row key.
const ROW_TAG: &str = "day-piece-settings:row";
/// A spanning cell learned its width from the form: re-lay its piece.
const FILL_TAG: &str = "day-piece-settings:fill";

/// The width of a slider or a text field in its cell, the same on every row so a page of them
/// lines up.
const FIELD_CONTROL_W: f64 = 200.0;

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

/// How a slot is hosted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotKind {
    /// A control in a cell, at its natural size.
    Field,
    /// A piece that takes the form's width (a custom row, a group's custom piece).
    Fill,
    /// No widget: a 1×1 node carrying a row's element id.
    Marker,
}

/// What the Day layouts and the Qt side share about one slot.
struct SlotShared {
    key: u64,
    kind: SlotKind,
    /// The width the form last granted a spanning cell.
    width: Cell<Option<f64>>,
    /// The slot's node, for a relayout when that width changes.
    node: Cell<Option<RNode>>,
}

#[derive(Clone, Debug, PartialEq)]
struct SlotProps {
    key: u64,
    kind: SlotKind,
}

/// The field of a shell row.
#[derive(Clone, Debug, PartialEq)]
enum RowShape {
    /// A control cell under the row's title in the label column.
    Field { slot: u64 },
    /// A button titled with the row (an action, a navigate row); `slot` is a busy action's
    /// spinner cell beside it.
    Button {
        arrow: bool,
        destructive: bool,
        slot: Option<u64>,
    },
    /// A link titled with the row.
    Link,
    /// A footnote spanning the form.
    Note,
    /// A spanning cell.
    Fill { slot: u64 },
}

/// One row as the shell realizes it.
#[derive(Clone, Debug, PartialEq)]
struct ShellRow {
    key: u64,
    /// The label column (a control row's title) or the field's own text (a button, a link,
    /// a note).
    text: String,
    caption: String,
    enabled: bool,
    shape: RowShape,
}

#[derive(Clone, Debug, PartialEq)]
struct ShellGroup {
    key: u64,
    title: String,
    footer: String,
    rows: Vec<ShellRow>,
}

#[derive(Clone, Debug, PartialEq)]
struct ShellPage {
    id: String,
    title: String,
    /// The list entry's icon as `day_qt::icon_args` spells it.
    icon: (String, c_int),
    groups: Vec<ShellGroup>,
    /// The group's custom piece, as a spanning cell in a trailing untitled group.
    custom: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
struct ShellProps {
    pages: Vec<ShellPage>,
    selected: String,
}

#[derive(Clone, Debug, PartialEq)]
enum ShellPatch {
    /// Show this group's page (a programmatic selection: `open_at`, a route).
    Select(String),
    PageTitle(String, String),
    GroupTitle(u64, String),
    GroupFooter(u64, String),
    /// `which` as the shim reads it: 0 the label column, 1 the field's text, 2 the caption,
    /// 3 a note.
    RowText(u64, c_int, String),
    RowEnabled(u64, bool),
}

struct ShellState {
    node: NodeId,
    host: *mut c_void,
    /// Page ids in list order.
    pages: Vec<String>,
    groups: HashMap<u64, *mut c_void>,
    /// Each slot's cell by slot key.
    cells: HashMap<u64, *mut c_void>,
    /// Slots still to move into their cells.
    pending: RefCell<Vec<u64>>,
}

day_core::tls_group! {
    static NEXT_KEY: Cell<u64> = const { Cell::new(0) };
    /// The window shell behind each host widget, keyed by the host's address.
    static SHELLS: RefCell<HashMap<usize, Rc<ShellState>>> = RefCell::new(HashMap::new());
    /// Each shell slot's widget by key, while it is realized.
    static SLOT_WIDGETS: RefCell<HashMap<u64, *mut c_void>> = RefCell::new(HashMap::new());
    /// Each slot widget's key, for `release`.
    static SLOT_KEYS: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
    /// Each slot's cell by slot key, for the slot layout to size it.
    static SLOT_CELLS: RefCell<HashMap<u64, *mut c_void>> = RefCell::new(HashMap::new());
    /// The width the form last granted each spanning cell, by slot key.
    static FILL_WIDTHS: RefCell<HashMap<u64, f64>> = RefCell::new(HashMap::new());
}

fn next_key() -> u64 {
    NEXT_KEY.with(|k| {
        let v = k.get() + 1;
        k.set(v);
        v
    })
}

// --- the slot: a one-child widget, laid out by the form once it is in its cell ------------

/// Places the slot's one child at the origin: at its natural size, or at the width the form
/// granted a spanning cell. Hands the size to the cell, so the form lays it out at that size.
struct SlotLayout {
    shared: Rc<SlotShared>,
}

impl Layout for SlotLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let Some(&c) = children.first() else {
            // A marker: a node with a frame, so a script finds it visible.
            return Size::new(1.0, 1.0);
        };
        let width = match self.shared.kind {
            SlotKind::Fill => self.shared.width.get().or(p.width),
            _ => None,
        };
        let s = cx.measure_child(c, Proposal::new(width, None));
        let size = Size::new(width.unwrap_or(s.width), s.height);
        if let Some(cell) = SLOT_CELLS.with(|m| m.borrow().get(&self.shared.key).copied()) {
            // SAFETY: the cell is a live widget of a realized shell; `shell_release` drops the
            // entry before the shell's widgets go.
            unsafe { day_settings_qt_cell_set_size(cell, size.width, size.height) };
        }
        size
    }

    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        if let Some(&c) = children.first() {
            cx.place_child(
                c,
                Rect::new(0.0, 0.0, bounds.size.width, bounds.size.height),
            );
        }
    }
}

struct Slot {
    shared: Rc<SlotShared>,
    child: Option<AnyPiece>,
    /// The action a marker runs on a (scripted) tap.
    on_tap: Option<Rc<dyn Fn()>>,
    /// The row's element id, when the slot node carries it (a marker, a busy action).
    id: Option<String>,
}

impl Piece for Slot {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Slot {
            shared,
            child,
            on_tap,
            id,
        } = self;
        let node = cx.native(
            SLOT_KIND,
            &SlotProps {
                key: shared.key,
                kind: shared.kind,
            },
            Rc::new(SlotLayout {
                shared: shared.clone(),
            }),
            Flex::default(),
            Boundary::No,
        );
        shared.node.set(Some(node));
        if let Some(id) = id {
            with_tree(|t| t.set_id(node, id));
        }
        if let Some(child) = child {
            cx.under(node, |cx| {
                let _ = child.build(cx);
            });
        }
        if let Some(on) = on_tap {
            cx.on(node, move |ev| {
                if let Event::Tap(_) = ev {
                    on();
                }
            });
        }
        node
    }
}

fn slot_make(_backend: &mut Qt, p: &SlotProps, _id: NodeId) -> QtHandle {
    // SAFETY: plain constructors on the main thread; the QApplication exists by now.
    let w = unsafe { day_qt_container_new() };
    if p.kind == SlotKind::Marker {
        // Never drawn, never a pointer target: the native button is the real control.
        // SAFETY: `w` is the widget just made.
        unsafe { day_qt_set_visible(w, 0) };
    }
    SLOT_WIDGETS.with(|m| m.borrow_mut().insert(p.key, w));
    SLOT_KEYS.with(|m| m.borrow_mut().insert(w as usize, p.key));
    QtHandle(w)
}

fn slot_update(_backend: &mut Qt, _h: &QtHandle, _patch: &()) {}

fn slot_release(_backend: &mut Qt, h: &QtHandle) {
    if let Some(k) = SLOT_KEYS.with(|m| m.borrow_mut().remove(&(h.0 as usize))) {
        SLOT_WIDGETS.with(|m| m.borrow_mut().remove(&k));
    }
}

// One `renderer!` per module: each shell kind registers from a module of its own.
mod slot_renderer {
    use super::*;
    day_pieces::renderer!(day_qt::RENDERERS, Qt,
        kind: SLOT_KIND, props: SlotProps, patch: (),
        make: slot_make, update: slot_update, measure: day_pieces::fill_measure, release: slot_release);
}

// --- the shell: pages, groups, rows ----------------------------------------------------------

extern "C" fn on_select(node: u64, index: c_int) {
    ffi_guard::contain((), || {
        let id = SHELLS.with(|m| {
            m.borrow()
                .values()
                .find(|st| st.node.0 == node)
                .and_then(|st| {
                    st.pages
                        .get(usize::try_from(index).unwrap_or(usize::MAX))
                        .cloned()
                })
        });
        if let Some(id) = id {
            day_qt::emit(NodeId(node), Event::custom(SELECT_TAG, id));
        }
    });
}

extern "C" fn on_row(node: u64, key: u64) {
    ffi_guard::contain((), || {
        day_qt::emit(NodeId(node), Event::custom(ROW_TAG, key.to_string()));
    });
}

extern "C" fn on_fill(node: u64, key: u64, width: f64) {
    ffi_guard::contain((), || {
        let changed = FILL_WIDTHS.with(|m| {
            let mut m = m.borrow_mut();
            if m.get(&key) == Some(&width) {
                false
            } else {
                m.insert(key, width);
                true
            }
        });
        if changed {
            day_qt::emit(NodeId(node), Event::custom(FILL_TAG, key.to_string()));
        }
    });
}

fn shell_make(_backend: &mut Qt, p: &ShellProps, id: NodeId) -> QtHandle {
    let sidebar = p.pages.len() > 1;
    // SAFETY (every shim call in this function): constructors and setters on widgets this
    // function just made, on the main thread.
    let host = unsafe {
        day_settings_qt_shell_new(id.0, c_int::from(sidebar), on_select, on_row, on_fill)
    };
    let mut groups = HashMap::new();
    let mut cells = HashMap::new();
    let mut pending = Vec::new();
    let mut selected = 0;
    for (ix, page) in p.pages.iter().enumerate() {
        if page.id == p.selected {
            selected = ix as c_int;
        }
        let index = unsafe {
            day_settings_qt_shell_add_page(
                host,
                cstr(&page.title).as_ptr(),
                cstr(&page.icon.0).as_ptr(),
                page.icon.1,
            )
        };
        let add_group = |title: &str, footer: &str| unsafe {
            day_settings_qt_page_add_group(host, index, cstr(title).as_ptr(), cstr(footer).as_ptr())
        };
        for g in &page.groups {
            let group = add_group(&g.title, &g.footer);
            groups.insert(g.key, group);
            for r in &g.rows {
                let (kind, arrow, destructive, with_cell, slot) = match &r.shape {
                    RowShape::Field { slot } => (0, false, false, false, Some(*slot)),
                    RowShape::Button {
                        arrow,
                        destructive,
                        slot,
                    } => (1, *arrow, *destructive, slot.is_some(), *slot),
                    RowShape::Link => (2, false, false, false, None),
                    RowShape::Note => (3, false, false, false, None),
                    RowShape::Fill { slot } => (4, false, false, false, Some(*slot)),
                };
                let (label, text) = match &r.shape {
                    RowShape::Field { .. } => (r.text.as_str(), ""),
                    _ => ("", r.text.as_str()),
                };
                let cell = unsafe {
                    day_settings_qt_group_add_row(
                        host,
                        group,
                        r.key,
                        kind,
                        cstr(label).as_ptr(),
                        cstr(text).as_ptr(),
                        cstr(&r.caption).as_ptr(),
                        c_int::from(arrow),
                        c_int::from(destructive),
                        c_int::from(with_cell),
                        c_int::from(r.enabled),
                    )
                };
                if let Some(slot) = slot
                    && !cell.is_null()
                {
                    cells.insert(slot, cell);
                    pending.push(slot);
                }
            }
        }
        if let Some(slot) = page.custom {
            let group = add_group("", "");
            let cell = unsafe {
                day_settings_qt_group_add_row(
                    host,
                    group,
                    slot,
                    4,
                    cstr("").as_ptr(),
                    cstr("").as_ptr(),
                    cstr("").as_ptr(),
                    0,
                    0,
                    0,
                    1,
                )
            };
            if !cell.is_null() {
                cells.insert(slot, cell);
                pending.push(slot);
            }
        }
    }
    unsafe { day_settings_qt_shell_done(host, selected) };
    SLOT_CELLS.with(|m| m.borrow_mut().extend(cells.iter().map(|(k, v)| (*k, *v))));
    let state = Rc::new(ShellState {
        node: id,
        host,
        pages: p.pages.iter().map(|pg| pg.id.clone()).collect(),
        groups,
        cells,
        pending: RefCell::new(pending),
    });
    SHELLS.with(|m| m.borrow_mut().insert(host as usize, state));
    QtHandle(host)
}

/// Move the slots Day mounted into the host into their cells. Runs at every measure and
/// update, so a slot mounted later still finds its cell.
fn settle(st: &ShellState) {
    let mut pending = st.pending.borrow_mut();
    pending.retain(|k| {
        let Some(slot) = SLOT_WIDGETS.with(|m| m.borrow().get(k).copied()) else {
            return true;
        };
        let Some(&cell) = st.cells.get(k) else {
            return false;
        };
        // SAFETY: both are live widgets of this shell (the slot realized and mounted under
        // the host by Day, the cell made by `shell_make`).
        unsafe { day_settings_qt_slot_mount(cell, slot) };
        false
    });
}

fn shell_update(_backend: &mut Qt, h: &QtHandle, patch: &ShellPatch) {
    let Some(st) = SHELLS.with(|m| m.borrow().get(&(h.0 as usize)).cloned()) else {
        return;
    };
    settle(&st);
    // SAFETY (every shim call below): `st.host` is this live shell; the group pointers are
    // its own, alive as long as it is.
    unsafe {
        match patch {
            ShellPatch::Select(id) => {
                if let Some(ix) = st.pages.iter().position(|p| p == id) {
                    day_settings_qt_shell_select(st.host, ix as c_int);
                }
            }
            ShellPatch::PageTitle(id, t) => {
                if let Some(ix) = st.pages.iter().position(|p| p == id) {
                    day_settings_qt_shell_set_page_title(st.host, ix as c_int, cstr(t).as_ptr());
                }
            }
            ShellPatch::GroupTitle(k, t) => {
                if let Some(&g) = st.groups.get(k) {
                    day_settings_qt_group_set_title(g, cstr(t).as_ptr());
                }
            }
            ShellPatch::GroupFooter(k, t) => {
                if let Some(&g) = st.groups.get(k) {
                    day_settings_qt_group_set_footer(g, cstr(t).as_ptr());
                }
            }
            ShellPatch::RowText(k, which, t) => {
                day_settings_qt_row_set_text(st.host, *k, *which, cstr(t).as_ptr());
            }
            ShellPatch::RowEnabled(k, on) => {
                day_settings_qt_row_set_enabled(st.host, *k, c_int::from(*on));
            }
        }
    }
}

/// The shell's natural size: the list beside the widest page, and the tallest page's content at
/// the proposed width. The window fits that at open (`size_to_fit`, up to its ceiling), so every
/// page fits without scrolling; a page taller than the ceiling scrolls inside its own
/// `QScrollArea`.
fn shell_measure(_backend: &mut Qt, h: &QtHandle, p: Proposal) -> Size {
    let Some(st) = SHELLS.with(|m| m.borrow().get(&(h.0 as usize)).cloned()) else {
        return Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0));
    };
    settle(&st);
    let (mut w, mut hh) = (0.0, 0.0);
    // SAFETY: `st.host` is this live shell; `w`/`hh` are the two doubles the shim writes.
    unsafe { day_settings_qt_shell_measure(st.host, p.width.unwrap_or(0.0), &mut w, &mut hh) };
    Size::new(p.width.unwrap_or(w), hh)
}

fn shell_release(_backend: &mut Qt, h: &QtHandle) {
    if let Some(st) = SHELLS.with(|m| m.borrow_mut().remove(&(h.0 as usize))) {
        SLOT_CELLS.with(|m| {
            let mut m = m.borrow_mut();
            for k in st.cells.keys() {
                m.remove(k);
            }
        });
        FILL_WIDTHS.with(|m| {
            let mut m = m.borrow_mut();
            for k in st.cells.keys() {
                m.remove(k);
            }
        });
    }
}

mod shell_renderer {
    use super::*;
    day_pieces::renderer!(day_qt::RENDERERS, Qt,
        kind: SHELL_KIND, props: ShellProps, patch: ShellPatch,
        make: shell_make, update: shell_update, measure: shell_measure, release: shell_release);
}

/// The host fills the window; its height is the shell's natural height, clamped to the
/// proposal so a tall page scrolls inside the window instead of running past it.
struct ShellLayout;

impl Layout for ShellLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        // The slots first: their sizes become the cells' sizes, which the form's own measure
        // then includes.
        for &c in children {
            let _ = cx.measure_child(c, Proposal::new(None, None));
        }
        let leaf = cx.measure_leaf(p);
        let height = match p.height {
            Some(h) => h.min(leaf.height),
            None => leaf.height,
        };
        Size::new(p.width.unwrap_or(leaf.width), height)
    }

    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], _bounds: Rect) {
        // Each slot's on-screen frame is the form's (it sits in its cell); Day records the size.
        for &c in children {
            let s = cx.measure_child(c, Proposal::new(None, None));
            cx.place_child_native(c, Rect::new(0.0, 0.0, s.width, s.height));
        }
    }
}

// --- the piece: the declaration as pages, groups, rows and slots ---------------------------

/// A localized text the shell re-reads on locale change, and where it goes.
enum DynText {
    PageTitle(String),
    GroupTitle(u64),
    GroupFooter(u64),
    /// A row text, with the shim's `which`.
    Row(u64, c_int),
}

impl DynText {
    fn patch(&self, t: String) -> ShellPatch {
        match self {
            DynText::PageTitle(id) => ShellPatch::PageTitle(id.clone(), t),
            DynText::GroupTitle(k) => ShellPatch::GroupTitle(*k, t),
            DynText::GroupFooter(k) => ShellPatch::GroupFooter(*k, t),
            DynText::Row(k, which) => ShellPatch::RowText(*k, *which, t),
        }
    }
}

/// What `Shell::build` accumulates while it walks the declaration.
#[derive(Default)]
struct ShellBuild {
    slots: Vec<Slot>,
    actions: HashMap<u64, Rc<dyn Fn()>>,
    dyns: Vec<(TextSource, DynText)>,
    enabled: Vec<(u64, Reactive<bool>)>,
}

impl ShellBuild {
    fn slot(
        &mut self,
        kind: SlotKind,
        child: Option<AnyPiece>,
        on_tap: Option<Rc<dyn Fn()>>,
        id: Option<String>,
    ) -> u64 {
        let key = next_key();
        self.slots.push(Slot {
            shared: Rc::new(SlotShared {
                key,
                kind,
                width: Cell::new(None),
                node: Cell::new(None),
            }),
            child,
            on_tap,
            id,
        });
        key
    }
    fn text(&mut self, src: &TextSource, target: DynText) {
        if matches!(src, TextSource::Dyn(_)) {
            self.dyns.push((src.clone(), target));
        }
    }
}

/// One row of the declaration as a shell row, with its control (if any) as a slot.
fn shell_row(inst: Rc<Installed>, r: &Row, b: &mut ShellBuild) -> ShellRow {
    let key = next_key();
    let id = r.id.clone();
    let enabled = r.enabled.clone();
    let with_id = |p: AnyPiece| match &id {
        Some(id) => p.id(id.clone()).any(),
        None => p,
    };
    let enabled_fn = move || enabled.as_ref().is_none_or(|e| e.get());
    // What the row starts as, for the native row\'s first draw; the control tracks the rest.
    let enabled_now = enabled_fn();
    if let Some(sig) = r.enabled.clone() {
        b.enabled.push((key, sig));
    }
    // A control row's title is the label column (0); a button's, a link's or a note's is the
    // field's own text (1 / 3). The caption is the dim line under the field (2).
    let title_which = match &r.kind {
        RowKind::Action { .. } | RowKind::Link(_) | RowKind::Navigate(_) => 1,
        RowKind::Note => 3,
        _ => 0,
    };
    b.text(&r.title, DynText::Row(key, title_which));
    if let Some(c) = &r.caption {
        b.text(c, DynText::Row(key, 2));
    }
    let mk_row = |shape: RowShape| ShellRow {
        key,
        text: r.title.initial(),
        caption: r
            .caption
            .as_ref()
            .map(TextSource::initial)
            .unwrap_or_default(),
        enabled: enabled_now,
        shape,
    };
    match &r.kind {
        RowKind::Toggle(sig) => {
            let ctl = with_id(toggle(*sig).enabled(enabled_fn).any());
            let slot = b.slot(SlotKind::Field, Some(ctl), None, None);
            mk_row(RowShape::Field { slot })
        }
        RowKind::Choice {
            labels,
            index,
            style,
        } => {
            let p = choice_picker(labels, *index, *style == ChoiceStyle::Segmented, enabled_fn);
            let slot = b.slot(SlotKind::Field, Some(with_id(p.any())), None, None);
            mk_row(RowShape::Field { slot })
        }
        RowKind::Slider {
            value,
            range,
            step,
            value_text,
        } => {
            let mut s = slider(*value).range(range.clone()).enabled(enabled_fn);
            if let Some(st) = step {
                s = s.step(*st);
            }
            let ctl = match &id {
                Some(i) => s
                    .id(i.clone())
                    .min_width(FIELD_CONTROL_W)
                    .max_width(FIELD_CONTROL_W)
                    .any(),
                None => s
                    .min_width(FIELD_CONTROL_W)
                    .max_width(FIELD_CONTROL_W)
                    .any(),
            };
            let ctl = match value_text {
                Some(f) => {
                    let f = f.clone();
                    let v = *value;
                    let value_label = label(move || f(v.get()))
                        .tabular()
                        .secondary()
                        .single_line()
                        .align(day_spec::props::TextAlign::Trailing);
                    let value_label = match &id {
                        Some(i) => value_label
                            .id(day_core::id!(pref_value, i.as_str()))
                            .min_width(VALUE_WIDTH)
                            .max_width(VALUE_WIDTH)
                            .any(),
                        None => value_label
                            .min_width(VALUE_WIDTH)
                            .max_width(VALUE_WIDTH)
                            .any(),
                    };
                    row((ctl, value_label))
                        .spacing(8.0)
                        .align(VAlign::Center)
                        .any()
                }
                None => ctl,
            };
            let slot = b.slot(SlotKind::Field, Some(ctl), None, None);
            mk_row(RowShape::Field { slot })
        }
        RowKind::Stepper { value, range, step } => {
            let v = *value;
            let (dec, inc) = stepper_buttons(v, range, *step, enabled_fn);
            let ctl = stepper_control(v, dec, inc, id.as_ref());
            let slot = b.slot(SlotKind::Field, Some(ctl), None, None);
            mk_row(RowShape::Field { slot })
        }
        RowKind::Text {
            value,
            placeholder,
            secure,
        } => {
            let mut f = if *secure {
                secure_field(*value)
            } else {
                text_field(*value)
            };
            if let Some(ph) = placeholder {
                f = f.placeholder(text_source_fn(ph));
            }
            let f = f.enabled(enabled_fn);
            let ctl = match &id {
                Some(i) => f
                    .id(i.clone())
                    .min_width(FIELD_CONTROL_W)
                    .max_width(FIELD_CONTROL_W)
                    .any(),
                None => f
                    .min_width(FIELD_CONTROL_W)
                    .max_width(FIELD_CONTROL_W)
                    .any(),
            };
            let slot = b.slot(SlotKind::Field, Some(ctl), None, None);
            mk_row(RowShape::Field { slot })
        }
        RowKind::Action {
            on,
            destructive,
            confirm,
            confirm_label,
            busy,
        } => {
            // The command button KDE forms use, titled with the command, its caption under it;
            // a destructive one draws in the negative text color. The confirm dialog and the
            // busy state are the composed row's.
            let on = on.clone();
            let title = text_source_fn(&r.title);
            let confirm = confirm.clone();
            let confirm_label = confirm_label.clone();
            let busy = busy.clone();
            let (enabled_run, busy_run) = (enabled_fn.clone(), busy.clone());
            let run: Rc<dyn Fn()> = Rc::new(move || {
                if !enabled_run() || busy_run.as_ref().is_some_and(|b| b.get_untracked()) {
                    return;
                }
                let on = on.clone();
                match &confirm {
                    Some(message) => {
                        let message = message.initial();
                        let title = title();
                        let label = confirm_label.as_ref().map(TextSource::initial);
                        day_core::task(async move {
                            if crate::confirm_dialog(&title, &message, label).await {
                                on();
                            }
                        });
                    }
                    None => on(),
                }
            });
            b.actions.insert(key, run.clone());
            // The id goes on a slot node that runs the command on a scripted tap: a marker,
            // or the spinner's cell while the row can be busy. A busy row is disabled.
            let slot = match (&id, busy.clone()) {
                (None, None) => None,
                (_, Some(b_sig)) => Some(b.slot(
                    SlotKind::Field,
                    Some(when(move || b_sig.get(), spinner).any()),
                    Some(run.clone()),
                    id.clone(),
                )),
                (Some(_), None) => {
                    Some(b.slot(SlotKind::Marker, None, Some(run.clone()), id.clone()))
                }
            };
            let can_be_busy = busy.is_some();
            if let Some(b_sig) = busy {
                let sig = Signal::global(enabled_fn() && !b_sig.get_untracked());
                day_reactive::Scope::root().enter(|| {
                    watch(
                        move || enabled_fn() && !b_sig.get(),
                        move |v, _| sig.set_if_changed(*v),
                    );
                });
                b.enabled.retain(|(k, _)| *k != key);
                b.enabled.push((key, sig.into_reactive()));
            }
            mk_row(RowShape::Button {
                arrow: false,
                destructive: *destructive,
                slot: slot.filter(|_| can_be_busy),
            })
        }
        RowKind::Link(url) => {
            let url = url.clone();
            let open: Rc<dyn Fn()> = Rc::new(move || day_core::open_url(&url));
            b.actions.insert(key, open.clone());
            if id.is_some() {
                b.slot(SlotKind::Marker, None, Some(open.clone()), id.clone());
            }
            mk_row(RowShape::Link)
        }
        RowKind::Navigate(build) => {
            let build = build.clone();
            let title = text_source_fn(&r.title);
            let open: Rc<dyn Fn()> = Rc::new(move || {
                push_subpage(inst.clone(), title(), build.clone());
            });
            b.actions.insert(key, open.clone());
            if id.is_some() {
                b.slot(SlotKind::Marker, None, Some(open.clone()), id.clone());
            }
            mk_row(RowShape::Button {
                arrow: true,
                destructive: false,
                slot: None,
            })
        }
        RowKind::Info { value, copyable } => {
            let v = text_label(value).secondary();
            let v = if *copyable {
                let text = value.clone();
                row((
                    v,
                    button("")
                        .icon(day_spec::Symbol::Copy)
                        .icon_only()
                        .plain()
                        .action(move || {
                            day_part_clipboard::set_text(&text.initial());
                        }),
                ))
                .spacing(4.0)
                .align(VAlign::Center)
                .any()
            } else {
                v.any()
            };
            let slot = b.slot(SlotKind::Field, Some(with_id(v)), None, None);
            mk_row(RowShape::Field { slot })
        }
        RowKind::Note => mk_row(RowShape::Note),
        RowKind::Custom(build) => {
            let slot = b.slot(SlotKind::Fill, Some(with_id(build())), None, None);
            mk_row(RowShape::Fill { slot })
        }
    }
}

struct Shell {
    inst: Rc<Installed>,
}

impl Piece for Shell {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let inst = self.inst;
        let mut b = ShellBuild::default();
        let mut pages = Vec::new();
        for g in inst.visible_groups() {
            let mut groups = Vec::new();
            for s in &g.sections {
                let rows: Vec<&Row> = s.rows.iter().filter(|r| r.is_available()).collect();
                if rows.is_empty() {
                    continue;
                }
                let gkey = next_key();
                if let Some(t) = &s.title {
                    b.text(t, DynText::GroupTitle(gkey));
                }
                if let Some(f) = &s.footer {
                    b.text(f, DynText::GroupFooter(gkey));
                }
                let rows = rows
                    .into_iter()
                    .map(|r| shell_row(inst.clone(), r, &mut b))
                    .collect();
                groups.push(ShellGroup {
                    key: gkey,
                    title: s
                        .title
                        .as_ref()
                        .map(TextSource::initial)
                        .unwrap_or_default(),
                    footer: s
                        .footer
                        .as_ref()
                        .map(TextSource::initial)
                        .unwrap_or_default(),
                    rows,
                });
            }
            let custom = g
                .custom
                .as_ref()
                .map(|build| b.slot(SlotKind::Fill, Some(build()), None, None));
            let icon = match &g.icon {
                Some(GroupIcon::Symbol(s)) => day_qt::icon_args(Some(&day_spec::Icon::Symbol(*s))),
                Some(GroupIcon::Image(n)) => {
                    day_qt::icon_args(Some(&day_spec::Icon::Image(n.as_str().to_string())))
                }
                None => (String::new(), -1),
            };
            b.text(&g.title, DynText::PageTitle(g.id.to_string()));
            pages.push(ShellPage {
                id: g.id.to_string(),
                title: g.title.initial(),
                icon,
                groups,
                custom,
            });
        }
        let ShellBuild {
            slots,
            actions,
            dyns,
            enabled,
        } = b;
        let fills: Vec<Rc<SlotShared>> = slots
            .iter()
            .filter(|s| s.shared.kind == SlotKind::Fill)
            .map(|s| s.shared.clone())
            .collect();
        let node = cx.native(
            SHELL_KIND,
            &ShellProps {
                pages,
                selected: inst.selected.get_untracked(),
            },
            Rc::new(ShellLayout),
            Flex {
                grow_w: true,
                grow_h: true,
                ..Default::default()
            },
            Boundary::Yes,
        );
        cx.under(node, |cx| {
            for s in slots {
                let _ = s.build(cx);
            }
        });
        let i = inst.clone();
        cx.on(node, move |ev| {
            if let Event::Custom { tag, text, .. } = ev {
                match *tag {
                    SELECT_TAG => i.selected.set_if_changed(text.clone()),
                    ROW_TAG => {
                        if let Some(run) = text.parse::<u64>().ok().and_then(|k| actions.get(&k)) {
                            run();
                        }
                    }
                    FILL_TAG => {
                        let k = text.parse::<u64>().ok();
                        for s in &fills {
                            if k.is_some_and(|k| k != s.key) {
                                continue;
                            }
                            let w = FILL_WIDTHS.with(|m| m.borrow().get(&s.key).copied());
                            s.width.set(w);
                            if let Some(n) = s.node.get() {
                                day_core::invalidate_size(n);
                            }
                        }
                    }
                    _ => {}
                }
            }
        });
        // A selection made elsewhere (`open_at`, a route) shows that page too.
        let i = inst.clone();
        watch(
            move || i.selected.get(),
            move |id, _| {
                let id = id.clone();
                with_tree(|t| t.patch(node, Box::new(ShellPatch::Select(id)), false));
            },
        );
        // Localized titles, captions, section texts and notes follow the locale.
        for (src, target) in dyns {
            if let TextSource::Dyn(f) = src {
                watch(
                    move || f(),
                    move |t, prev| {
                        if prev.is_some() {
                            let affects_size = true;
                            with_tree(|tree| {
                                tree.patch(node, Box::new(target.patch(t.clone())), affects_size)
                            });
                        }
                    },
                );
            }
        }
        for (k, sig) in enabled {
            watch(
                move || sig.get(),
                move |on, _| {
                    with_tree(|t| t.patch(node, Box::new(ShellPatch::RowEnabled(k, *on)), false));
                },
            );
        }
        node
    }
}

/// The preferences window's body: the real preferences-dialog structure around the visible
/// groups. `None` with nothing to show.
pub(super) fn window_shell(inst: Rc<Installed>) -> Option<AnyPiece> {
    let groups = inst.visible_groups();
    if groups.is_empty() {
        return None;
    }
    let selected = inst.selected.get_untracked();
    if !groups.iter().any(|g| g.id == selected) {
        inst.selected.set(groups[0].id.to_string());
    }
    Some(AnyPiece::new(Shell { inst }))
}
