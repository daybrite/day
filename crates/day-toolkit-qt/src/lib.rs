// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-toolkit-qt — the Qt 6 Widgets backend (linux-qt / macos-qt; DESIGN.md §9), over
//! the day-toolkit-qt-sys C++ shim. `Handle = QtHandle(*mut QWidget)`; absolute geometry; toggle is a
//! `QCheckBox` (Qt Widgets has no native switch — an explicitly documented divergence).

mod controls;

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_double, c_int, c_void};
use std::rc::Rc;

use day_toolkit_qt_sys as ffi;
use linkme::distributed_slice;

use day_spec::props::*;
use day_spec::{
    A11yProps, AnimSpec, Builtin, Cap, Cursor, Curve, DrawOp, Event, EventSink, Font, InputPurpose,
    InputTraits, NodeId, PieceKind, Platform, Proposal, Rect, Registry, Renderer, Role, Size,
    Support, Toolkit, Transform, WindowOptions, ffi_guard, kinds, props_of, sidetable::SideTable,
};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct QtHandle(pub *mut c_void);

/// An `AnimSpec` lowered for the shim's animation runs (`ffi::DayQtAnim`, docs/tween.md "The
/// animation contract"): the shim plays the delay as a `QPauseAnimation`, the cycles as a looped
/// `QVariantAnimation` (a forward-and-backward pair under autoreverse) whose key values are
/// these samples of one cycle, so a spring and every easing curve are drawn exactly.
#[derive(Clone, Debug, PartialEq)]
struct QtAnim {
    delay_ms: c_int,
    duration_ms: c_int,
    /// `-1` until the property is next set.
    cycles: c_int,
    autoreverse: bool,
    /// One forward cycle's progress at evenly spaced offsets (`AnimSpec::keyframes`); the two
    /// ends alone for a linear cycle, which Qt's interpolation already draws exactly.
    keys: Vec<f64>,
}

impl QtAnim {
    fn new(spec: &AnimSpec) -> Self {
        let ms = |v: u32| c_int::try_from(v).unwrap_or(c_int::MAX);
        let keys = if spec.curve == Curve::Linear {
            vec![0.0, 1.0]
        } else {
            spec.keyframes().iter().map(|k| k.progress).collect()
        };
        QtAnim {
            delay_ms: ms(spec.delay_ms),
            duration_ms: ms(spec.duration_ms),
            cycles: spec.cycles().map_or(-1, ms),
            autoreverse: spec.autoreverse,
            keys,
        }
    }

    /// The shim's view of this run, borrowing `keys`.
    fn raw(&self) -> ffi::DayQtAnim {
        ffi::DayQtAnim {
            delay_ms: self.delay_ms,
            duration_ms: self.duration_ms,
            cycles: self.cycles,
            autoreverse: c_int::from(self.autoreverse),
            keys: self.keys.as_ptr(),
            n_keys: c_int::try_from(self.keys.len()).unwrap_or(0),
        }
    }
}

/// Calls `f` with the shim's run for `anim`, or a null pointer (apply at once) for `None`. The
/// pointer and its samples are alive for the call.
fn with_qt_anim<R>(anim: Option<&AnimSpec>, f: impl FnOnce(*const ffi::DayQtAnim) -> R) -> R {
    match anim {
        None => f(std::ptr::null()),
        Some(spec) => {
            let run = QtAnim::new(spec);
            let raw = run.raw();
            f(&raw const raw)
        }
    }
}

/// An `InputPurpose` as the code `day_qt_lineedit_set_traits` takes (the shim's switch is the
/// other half of this table): 0 text, 1 name, 2 email, 3 url, 4 phone, 5 number, 6 decimal,
/// 7 username, 8 password, 9 new password, 10 one-time code.
fn qt_input_purpose(purpose: InputPurpose) -> c_int {
    match purpose {
        InputPurpose::Text => 0,
        InputPurpose::Name => 1,
        InputPurpose::Email => 2,
        InputPurpose::Url => 3,
        InputPurpose::Phone => 4,
        InputPurpose::Number => 5,
        InputPurpose::Decimal => 6,
        InputPurpose::Username => 7,
        InputPurpose::Password => 8,
        InputPurpose::NewPassword => 9,
        InputPurpose::OneTimeCode => 10,
    }
}

// Built-in leaf pieces split into modules (moved in from their satellite crates 2026-07).
#[cfg(target_os = "linux")]
mod linux;
mod picker;
mod textarea;
mod toolbar;
mod transfer;
mod tree;

pub type Handle = QtHandle;

pub mod ext;
pub use ext::*;
/// The icon spec a piece shim hands `day_qt_icon_new` (docs/extending.md): see
/// [`toolbar::icon_args`].
pub use toolbar::icon_args;

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

    /// Menu button widget ptr → the provider its click opens (`set_button_menu`, docs/menus.md
    /// "Menu buttons"). Swept on release with the widget.
    static BUTTON_MENUS: SideTable<day_spec::ContextMenuFn> = SideTable::new();
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
    /// Split host (the QSplitter, docs/split.md) → its state.
    static SPLIT_STATE: RefCell<HashMap<usize, SplitState>> = RefCell::new(HashMap::new());
    /// SPLIT_PANE widget → `(its NodeId, is-second)`, recorded at realize.
    static SPLIT_PANE_IDS: RefCell<HashMap<usize, (NodeId, bool)>> = RefCell::new(HashMap::new());

    /// Each label's own resolved point size, so a RELATIVE-size run can be written as the
    /// absolute `pt` Qt's rich-text CSS understands (a percentage is silently ignored there).
    ///
    /// A side table because the patch that carries runs does not carry the font: `LabelPatch`
    /// splits them, and by the time `Runs` arrives the `Font` it should scale against is
    /// whatever the last `Font` patch set. Swept with the widget in `release`.
    static LABEL_PT: SideTable<f64> = SideTable::new();

    static NAV_SUITES: RefCell<HashMap<usize, NavSuite>> = RefCell::new(HashMap::new());
    /// Suite page widgets, so a released one drops out of its suite. Unlike the `tabs()` piece's
    /// pages these are framed by Day: NavLayout computes exactly the content rect the tab widget
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
    /// Covers presented as half sheets (`CoverProps::half`, docs/cover.md): the bottom half
    /// of the window content.
    static COVER_HALF: RefCell<std::collections::HashSet<usize>> =
        RefCell::new(std::collections::HashSet::new());

    /// NAV_MENU widget → (row count, section heading count), for measure: the shim draws each
    /// heading as a band of its own above its row.
    static NAV_MENU_ROWS: RefCell<HashMap<usize, (usize, usize)>> = RefCell::new(HashMap::new());

    /// `BitmapId` → what the decode reported (docs/images.md). Only the METADATA lives here;
    /// the pixels are the shim's, in a `QImage` registry keyed by the same id.
    static BITMAP_INFO: RefCell<HashMap<u64, day_spec::BitmapInfo>> = RefCell::new(HashMap::new());

}

pub fn emit(id: NodeId, ev: Event) {
    let sink = SINK.with(|s| s.borrow().clone());
    if let Some(sink) = sink {
        sink(id, ev);
    }
}

/// Deliver an event to day-core on the next event-loop turn — a genuine "safe point" (§8.3) —
/// instead of inline. A native list selection rebuilds the sidebar detail *synchronously*
/// (dispose old widgets, create new ones); doing that inside the `QListView`'s own key/click
/// dispatch reparents widgets mid-event and reads freed memory (`QWidget::setParent` SIGSEGV).
/// Only `Send` data (id + event) is captured; the thread-local sink runs on the main thread.
fn emit_deferred(id: NodeId, ev: Event) {
    let boxed: Box<dyn FnOnce() + Send> = Box::new(move || emit(id, ev));
    let data = Box::into_raw(Box::new(boxed)).cast::<c_void>();
    unsafe { ffi::day_qt_post(run_posted, data) };
}

/// Pack a color as `0xAARRGGBB`, the form the shim reads.
fn argb(c: day_spec::Color) -> u32 {
    let f = |v: f64| (v.clamp(0.0, 1.0) * 255.0) as u32;
    (f(c.a) << 24) | (f(c.r) << 16) | (f(c.g) << 8) | f(c.b)
}

fn apply_button_content(
    h: *mut c_void,
    title: &str,
    icon: Option<&day_spec::Icon>,
    icon_only: bool,
) {
    let (name, fallback) = toolbar::icon_args(icon);
    unsafe {
        ffi::day_qt_button_set_content(
            h,
            cstr(title).as_ptr(),
            cstr(&name).as_ptr(),
            fallback,
            c_int::from(icon_only),
        );
    };
}

/// Hand a [`day_spec::props::ButtonStyleSpec`] to the shim, which styles the `QPushButton` in
/// place. The widget stays a `QPushButton`, so focus, Space/Enter activation and the a11y role
/// are unaffected whatever the style is.
fn apply_button_style(w: *mut c_void, style: day_spec::props::ButtonStyleSpec) {
    use day_spec::props::ButtonStyleSpec as S;
    let (kind, fill) = match style {
        // A QPushButton hugs a one-glyph title already, so Compact is the stock look.
        S::Automatic | S::Compact | S::Plain => (0, day_spec::Color::CLEAR),
        S::Bordered => (1, day_spec::Color::CLEAR),
        S::Prominent => (2, day_spec::Color::CLEAR),
        S::Tinted(c) => (3, c.resolved()),
    };
    // SAFETY: `w` is a live QPushButton created by `day_qt_button_new`; the shim only reads the
    // packed colors.
    unsafe { ffi::day_qt_button_set_style(w, kind, argb(fill), argb(S::on_tint(fill))) };
}

/// Re-encode a decoded bitmap through the shim (docs/images.md).
///
/// A free function rather than the duty's body so each refusal reads as an early return instead
/// of another copy of the same emit. The pixels never cross the boundary: the shim encodes and
/// hands back one heap buffer, which is copied out and released through its own free.
fn encode_bitmap(
    id: day_spec::BitmapId,
    spec: &day_spec::EncodeSpec,
) -> Result<Vec<u8>, day_spec::ImageError> {
    use day_spec::ImageFormat as F;
    let format = match spec.format {
        F::Png => "png",
        F::Jpeg => "jpeg",
        F::Tiff => "tiff",
        F::Bmp => "bmp",
        // Qt reads more than it writes; `encode_formats` says which before an app gets here.
        _ => return Err(day_spec::ImageError::Encode),
    };
    if !BITMAP_INFO.with(|m| m.borrow().contains_key(&id.0)) {
        return Err(day_spec::ImageError::Gone);
    }
    // Qt takes quality as 0..=100, with -1 meaning the format's own default.
    let quality = spec
        .quality
        .map_or(-1, |q| (q.clamp(0.0, 1.0) * 100.0).round() as c_int);
    let (fit_w, fit_h) = spec.fit.map_or((0.0, 0.0), |f| (f.width, f.height));
    let mut len: c_int = 0;
    // SAFETY: the shim returns either null or a malloc'd buffer of `len` bytes; it is copied out
    // and released through the shim's own free, and `len` is written before the pointer is used.
    let bytes = unsafe {
        let p = ffi::day_qt_image_encode(
            id.0,
            cstr(format).as_ptr(),
            quality,
            fit_w,
            fit_h,
            &raw mut len,
        );
        if p.is_null() || len <= 0 {
            return Err(day_spec::ImageError::Encode);
        }
        let out = std::slice::from_raw_parts(p, len as usize).to_vec();
        ffi::day_qt_bytes_free(p);
        out
    };
    Ok(bytes)
}

/// Encode every page, hand the shim borrowed views of them, and copy the document out.
fn render_pdf_document(
    pages: &[day_spec::PdfPage],
    info: &day_spec::PdfInfo,
) -> Result<Vec<u8>, day_spec::ExportError> {
    let encoded: Vec<(Vec<f64>, CString)> = pages
        .iter()
        .map(|page| {
            let (nums, texts) = day_spec::encode_ops(&page.ops);
            (nums, cstr(&texts.join("\u{1f}")))
        })
        .collect();
    let views: Vec<ffi::DayQtPdfPage> = pages
        .iter()
        .zip(&encoded)
        .map(|(page, (nums, texts))| ffi::DayQtPdfPage {
            w: page.size.width,
            h: page.size.height,
            nums: nums.as_ptr(),
            n: nums.len() as c_int,
            texts: texts.as_ptr(),
        })
        .collect();
    let title = info.title.as_deref().map(cstr);
    let author = info.author.as_deref().map(cstr);
    let mut len: c_int = 0;
    let mut err: *mut c_char = std::ptr::null_mut();
    // SAFETY: every page view borrows `encoded`, which outlives the call, and the strings are
    // null or NUL-terminated. The shim returns null (with `err` a heap string or null) or a
    // malloc'd buffer of `len` bytes; each is copied out and released through the shim's frees.
    unsafe {
        let p = ffi::day_qt_canvas_render_pdf(
            views.as_ptr(),
            views.len() as c_int,
            title.as_ref().map_or(std::ptr::null(), |t| t.as_ptr()),
            author.as_ref().map_or(std::ptr::null(), |a| a.as_ptr()),
            &raw mut len,
            &raw mut err,
        );
        if p.is_null() || len <= 0 {
            let why = if err.is_null() {
                "QPdfWriter failed".to_string()
            } else {
                let text = CStr::from_ptr(err).to_string_lossy().into_owned();
                ffi::day_qt_string_free(err);
                text
            };
            ffi::day_qt_bytes_free(p);
            return Err(day_spec::ExportError::Failed(why));
        }
        let out = std::slice::from_raw_parts(p, len as usize).to_vec();
        ffi::day_qt_bytes_free(p);
        Ok(out)
    }
}

pub(crate) fn cstr(s: &str) -> CString {
    // An interior NUL must not blank the whole string — a label, a menu item, a window title
    // would silently vanish. Strip the NULs and keep the visible text.
    CString::new(s).unwrap_or_else(|_| CString::new(s.replace('\0', "")).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// The list (docs/list.md, §10): a QListView over a count-only model (day-toolkit-qt-sys's
// `DayListView`). The view owns selection and its painting, the keyboard walker, focus and the
// tab stop, accessibility, the scroll bar and drag-and-drop; Day owns the rows. A row's cell is
// a plain QWidget placed over its item as a viewport child, and the cells live in a POOL
// bounded by the viewport plus overscan: a row scrolling out gives its cell back, a row
// scrolling in takes one and is rebound into it with a slot write, which is what
// `Cap::ListRecycling => Native` promises. The bookkeeping is `CellPool`, Qt-free so the
// tests at the end of this file exercise it without a display.
//
// Every entry into Day from a Qt callback is posted to the next loop turn: Qt delivers scroll,
// resize and layout synchronously inside its own handlers, where `bind_row`'s re-entry into
// `with_tree` cannot run (DESIGN.md §10). Deferred work looks the list up again, since it may
// have been released meanwhile.
// ---------------------------------------------------------------------------

/// Rows built beyond each edge of the viewport, so a flick has something to show before the
/// window callback lands. The pool holds at most a viewport of rows plus two of these.
const LIST_OVERSCAN: usize = 8;

/// Measured heights remembered per row token (`Automatic` lists). Past this many the cache
/// starts over rather than growing with every row ever visited: a row outside the window is
/// shown at the estimate until it is bound again, which is what happens to a row that was never
/// visited anyway.
const LIST_HEIGHT_CACHE: usize = 4096;

/// The pooled cells of one list: which row each shows, and the spares. Cells are opaque keys
/// (the native handle as `usize`), so none of this touches Qt.
#[derive(Default)]
struct CellPool {
    /// row → the cell showing it.
    bound: std::collections::BTreeMap<usize, usize>,
    /// cell → the row it shows (the inverse of `bound`).
    row_of: HashMap<usize, usize>,
    /// Cells with no row, hidden, ready for the next row that scrolls in.
    free: Vec<usize>,
    /// A sectioned list's spare cells that last held a header (docs/list.md "Sections"), kept
    /// apart from `free`: day-core rebuilds a cell that changes kind, so a header row only ever
    /// takes a header's cell and an item row an item's, the per-kind reuse pools of
    /// `UITableView` and `RecyclerView`. Each kind's spares stay as many as that kind ever had
    /// bound at once, so the pool is a window of items plus the headers a window holds.
    free_headers: Vec<usize>,
    /// The cells holding a header now, bound or spare.
    headers: std::collections::HashSet<usize>,
}

impl CellPool {
    /// Retarget the pool at `band`, the rows that should have a cell. Returns the cells that
    /// left a row (to park and recycle) and the rows that still need one, ascending.
    fn retarget(&mut self, band: std::ops::Range<usize>) -> (Vec<usize>, Vec<usize>) {
        let leaving: Vec<usize> = self
            .bound
            .range(..band.start)
            .chain(self.bound.range(band.end..))
            .map(|(row, _)| *row)
            .collect();
        let mut parked = Vec::with_capacity(leaving.len());
        for row in leaving {
            if let Some(cell) = self.bound.remove(&row) {
                self.row_of.remove(&cell);
                if self.headers.contains(&cell) {
                    self.free_headers.push(cell);
                } else {
                    self.free.push(cell);
                }
                parked.push(cell);
            }
        }
        let need: Vec<usize> = band.filter(|row| !self.bound.contains_key(row)).collect();
        (parked, need)
    }

    /// Bind `cell` to `row`. The cell must be spare (taken through [`Self::take_free`] or new).
    fn bind(&mut self, row: usize, cell: usize) {
        self.bound.insert(row, cell);
        self.row_of.insert(cell, row);
    }

    /// A spare cell that last held a row of the same kind (header or item), if any.
    fn take_free(&mut self, header: bool) -> Option<usize> {
        if header {
            self.free_headers.pop()
        } else {
            self.free.pop()
        }
    }

    /// `cell` now holds a header row (or an item row).
    fn set_kind(&mut self, cell: usize, header: bool) {
        if header {
            self.headers.insert(cell);
        } else {
            self.headers.remove(&cell);
        }
    }

    fn row_of(&self, cell: usize) -> Option<usize> {
        self.row_of.get(&cell).copied()
    }

    /// Every cell the pool owns, bound or spare (the tests' bound on the pool).
    #[cfg(test)]
    fn len(&self) -> usize {
        self.bound.len() + self.free.len() + self.free_headers.len()
    }
}

/// Row changes waiting for the next loop turn (docs/list.md "Keyed updates"): the patches
/// arrive inside a day-core borrow, and the model change that applies them re-enters Day
/// through `bind_row`. Everything one turn sends goes to the view in one flush.
#[derive(Debug, PartialEq)]
enum RowWork {
    /// Re-query the source: the count, and every bound row.
    Reload,
    /// Sequential deltas, in the order they arrived.
    Splice(Vec<RowDelta>),
}

impl RowWork {
    /// Fold `next` into the work already waiting: deltas after deltas stay sequential, and a
    /// reload on either side makes the deltas moot (it re-queries the final rows anyway).
    fn merge(waiting: Option<Self>, next: Self) -> Self {
        match (waiting, next) {
            (Some(Self::Splice(mut a)), Self::Splice(b)) => {
                a.extend(b);
                Self::Splice(a)
            }
            (None, next) => next,
            (Some(_), _) => Self::Reload,
        }
    }

    /// The deltas as the shim's `kind, a, b` triples (0 insert, 1 remove, 2 move).
    fn triples(deltas: &[RowDelta]) -> Vec<c_int> {
        let idx = |i: usize| c_int::try_from(i).unwrap_or(c_int::MAX);
        deltas
            .iter()
            .flat_map(|d| match d {
                RowDelta::Insert(i) => [0, idx(*i), 0],
                RowDelta::Remove(i) => [1, idx(*i), 0],
                RowDelta::Move(from, to) => [2, idx(*from), idx(*to)],
            })
            .collect()
    }
}

/// The supplementary rows of a list of `n` rows (docs/list.md "Header, footer and empty
/// state"). They only ever lead and trail the items (`[top padding] [header] ([empty state] |
/// items) [footer] [bottom padding]`), so the walk asks about the runs at both ends and stops
/// at the first item each way: a handful of calls however long the list.
fn supplement_rows(n: usize, is_supplement: &dyn Fn(usize) -> bool) -> Vec<usize> {
    let lead = (0..n).take_while(|i| is_supplement(*i)).count();
    let trail = (lead..n).rev().take_while(|i| is_supplement(*i)).count();
    (0..lead).chain(n - trail..n).collect()
}

/// `RowAlign` as the shim's number (0 Nearest, 1 Start, 2 Center, 3 End).
fn row_align_code(align: day_spec::RowAlign) -> c_int {
    match align {
        day_spec::RowAlign::Nearest => 0,
        day_spec::RowAlign::Start => 1,
        day_spec::RowAlign::Center => 2,
        day_spec::RowAlign::End => 3,
    }
}

struct ListEntry {
    host: *mut c_void,
    /// The row pitch: every row's height for a `Uniform` list, and for an `Automatic` one the
    /// first estimate for rows not measured yet.
    row_height: f64,
    /// `RowHeight::Automatic`: each built row is as tall as its content.
    automatic: bool,
    source: Rc<RefCell<Option<day_spec::ListSource>>>,
    pool: CellPool,
    /// `Automatic` lists: row token → the height Day laid that row at, so a row keeps its
    /// height across a reload or a reorder and a row scrolling back in starts at it.
    heights: HashMap<u64, f64>,
    /// The sum of `heights`, for the estimate without a walk.
    height_sum: f64,
    /// The height the model gives rows not measured yet: the running average of `heights`.
    estimate: c_int,
    /// Every bound row rebinds on the next sync: the source changed under the cells.
    rebind_all: bool,
    /// The height day's layout last framed the host at, so the first fill — which runs before
    /// Qt has laid the view out and it still reports no visible rows — has a window.
    frame_height: c_int,
    /// A window-driven sync is already posted: one flick emits a stream of window reports, and
    /// they would each post a pass that does the same work.
    fill_pending: bool,
    /// Last host width a populate ran at — so `set_frame` only repopulates on a real width change
    /// (a populate's own child `set_frame`s must not schedule another, or it loops forever).
    last_width: c_int,
    /// The width the cells were last laid out to: the VIEWPORT's, which a legacy scroll bar
    /// makes narrower than the host. A viewport resize that changes it repopulates.
    cell_width: c_int,
    node: u64,
    /// Whether several rows may be selected (`ListProps::multi_select`): decides which event
    /// a native selection change reports (docs/list.md).
    multi: bool,
    /// A sectioned list's pinned header cell (docs/list.md "Sections"; `Cap::ListSections` =
    /// Emulated), made on the first sync of a source with `is_header`.
    pin: Option<usize>,
    /// The header row the pinned cell was last bound to.
    pin_row: Option<usize>,
    /// The row count the view's header rows were last given for (`day_qt_list_set_headers`).
    headers_for: Option<usize>,
    /// The row count the view's supplementary rows were last given for
    /// (`day_qt_list_set_supplements`).
    supplements_for: Option<usize>,
    /// Supplementary row token → the height Day laid it at, so a header, footer, empty state
    /// or padding row keeps its height through a splice (which drops the view's per-row
    /// heights) even while it is out of the window. A list has a handful of them.
    supplement_heights: HashMap<u64, f64>,
    /// Row changes waiting for this turn's flush (`queue_rows`).
    pending: Option<RowWork>,
}

extern "C" fn on_list_activate(node: u64, row: c_int) {
    ffi_guard::contain((), || {
        if row >= 0 {
            emit_deferred(NodeId(node), Event::ListActivated(row as usize));
        }
    });
}

/// The view's selection changed under the user (a click, the keyboard walker, a range): report
/// it the way docs/list.md wants — the full set where several rows may be selected, the one
/// row otherwise, and an emptied selection as an empty set either way, since only the set can
/// say "nothing".
extern "C" fn on_list_selection(node: u64, rows: *const c_int, count: c_int) {
    // Every extern "C" trampoline in this backend runs its body through `ffi_guard::contain`:
    // a panic unwinding into the C++ shim frame is undefined behavior (day-spec's ffi_guard).
    ffi_guard::contain((), || {
        let rows: Vec<i64> = if rows.is_null() || count <= 0 {
            Vec::new()
        } else {
            // SAFETY: the shim hands a live array of `count` ints for the duration of the call.
            unsafe { std::slice::from_raw_parts(rows, count as usize) }
                .iter()
                .map(|r| i64::from(*r))
                .collect()
        };
        let multi = LIST_BY_NODE
            .with(|m| m.borrow().get(&node).copied())
            .and_then(|k| LIST_STATE.with(|m| m.borrow().get(&k).map(|st| st.multi)));
        let Some(multi) = multi else { return };
        let ev = match (multi, rows.as_slice()) {
            (false, [row]) => Event::SelectionChanged(*row),
            _ => Event::SelectionSet(rows),
        };
        emit(NodeId(node), ev);
    });
}

/// The reorder guard's verdict for a hovered drop (docs/list.md), called synchronously from the
/// view's drag-move handler: the accepted target index, or -1. The reorder Rc is cloned out of
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

/// Commit a drop the view accepted: rotate Day's snapshot through the sync seam (deferring the
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

/// A scroll piece with `track_offset` moved (docs/scroll.md "Offset and phase"): the shim
/// coalesces to one call per frame plus the resting one, from its own timer, outside any Day
/// borrow; the sink only enqueues.
extern "C" fn on_scroll_offset(node: u64, x: c_double, y: c_double) {
    ffi_guard::contain((), || {
        emit(
            NodeId(node),
            Event::ScrollChanged(day_spec::Point::new(x, y)),
        );
    });
}

/// Rebind every bound row on the next event-loop turn — NOT inline: a reload runs inside a
/// `with_tree` borrow, and `bind_row` re-enters `with_tree`, which would panic.
fn schedule_list_populate(host_key: usize) {
    Qt::post(Box::new(move || list_populate(host_key)));
}

/// The view's window moved (a scroll, a resize, a relayout): bring the pool to it. Deferred and
/// coalesced — the report arrives from inside Qt's own scroll handling, where `bind_row`'s
/// re-entry into `with_tree` cannot run.
extern "C" fn on_list_window(node: u64) {
    ffi_guard::contain((), || {
        // Even a visible-row query can make Qt lay out the view and report inside a sync's
        // LIST_STATE borrow. Defer the registry access too.
        Qt::post(Box::new(move || {
            if let Some(host_key) = LIST_BY_NODE.with(|m| m.borrow().get(&node).copied()) {
                schedule_list_fill(host_key);
            }
        }));
    });
}

/// Sync the pool to the window on the next loop turn, at most once per turn however many times
/// it is asked.
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
    Qt::post(Box::new(move || {
        LIST_STATE.with(|m| {
            if let Some(st) = m.borrow_mut().get_mut(&host_key) {
                st.fill_pending = false;
            }
        });
        list_sync(host_key);
    }));
}

/// Scroll the list to its bottom on the next event-loop turn — deferred so any pending
/// `list_populate` has sized the content first (posted callbacks run FIFO), matching Qt's
/// scrollToBottom semantics.
fn schedule_list_scroll_end(host_key: usize) {
    Qt::post(Box::new(move || {
        if let Some(host) = LIST_STATE.with(|m| m.borrow().get(&host_key).map(|st| st.host)) {
            unsafe { ffi::day_qt_list_scroll_to_end(host) };
        }
    }));
}

/// Scroll the list just far enough that `row` is in view (docs/list.md), on the next
/// event-loop turn — the same deferral as `schedule_list_scroll_end`.
fn schedule_list_scroll_row(host_key: usize, row: usize) {
    Qt::post(Box::new(move || {
        if let Some(host) = LIST_STATE.with(|m| m.borrow().get(&host_key).map(|st| st.host)) {
            unsafe { ffi::day_qt_list_scroll_to_row(host, row as c_int) };
        }
    }));
}

/// Queue a `ListPatch::Reload` or `ListPatch::Splice` for the flush on the next loop turn
/// (one flush per turn, posted by the first patch of the turn). A row scroll asked for after
/// the patch is posted after the flush, so it finds its row (docs/list.md "Programmatic
/// scrolling").
fn queue_rows(host_key: usize, work: RowWork) {
    let post = LIST_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let Some(st) = m.get_mut(&host_key) else {
            return false;
        };
        let first = st.pending.is_none();
        st.pending = Some(RowWork::merge(st.pending.take(), work));
        first
    });
    if post {
        Qt::post(Box::new(move || flush_rows(host_key)));
    }
}

/// Apply the turn's row changes. A splice goes to the view as model deltas, so selection and
/// the current row follow their rows (docs/list.md), but only when they fit what the view was
/// told: every index valid against the rows the earlier deltas left, and the count at the end
/// the source's. Anything else (a snapshot that drifted from the deltas) reloads with a model
/// reset that keeps the scroll position, never a delta the view cannot take. Either way the
/// bound rows rebind in place and the viewport is reported after.
fn flush_rows(host_key: usize) {
    let work = LIST_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let st = m.get_mut(&host_key)?;
        let work = st.pending.take()?;
        st.rebind_all = true;
        let len = st.source.borrow().as_ref().map(|s| (s.len)());
        Some((st.host, work, len))
    });
    let Some((host, work, len)) = work else {
        return;
    };
    if let (RowWork::Splice(deltas), Some(len)) = (work, len) {
        let triples = RowWork::triples(&deltas);
        let count = c_int::try_from(deltas.len()).unwrap_or(c_int::MAX);
        let len = c_int::try_from(len).unwrap_or(c_int::MAX);
        let applied = unsafe { ffi::day_qt_list_splice_all(host, triples.as_ptr(), count, len) };
        if applied == 0 {
            log::debug!("qt list: a splice did not fit the view's rows; reloaded instead");
        }
    }
    list_sync(host_key);
}

/// The viewport under a list resized (a legacy scroll bar came or went): the rows are laid
/// out to a width the viewport no longer has, so re-lay them. A viewport that still matches
/// the cells — the usual overlay-bar case, where every host resize also lands here — is left
/// alone, and so is a list that has not built a row yet.
extern "C" fn on_list_viewport_resized(host: *mut c_void) {
    ffi_guard::contain((), || {
        // Qt can resize the viewport synchronously while we realize cells. Wait until
        // that operation releases LIST_STATE; look up the host again in case it died.
        let host_key = host as usize;
        Qt::post(Box::new(move || {
            let state =
                LIST_STATE.with(|m| m.borrow().get(&host_key).map(|st| (st.host, st.cell_width)));
            let Some((host, cell_width)) = state else {
                return;
            };
            let vw = unsafe { ffi::day_qt_list_viewport_width(host) };
            if cell_width > 0 && vw > 0.0 && (vw as c_int) != cell_width {
                schedule_list_populate(host_key);
            }
        }));
    });
}

/// The reader moved the list themselves (docs/list.md "Following the end"): the view saw a
/// wheel step, a scroll bar action or a key move it (`DayListView::readerScrolled`). Reported
/// on the next loop turn, as the window reports are, and so ahead of the viewport report the
/// same movement brings (the view posts that one as the bar's value lands, after this).
extern "C" fn on_list_reader_scroll(node: u64) {
    ffi_guard::contain((), || {
        Qt::post(Box::new(move || {
            let report = LIST_BY_NODE
                .with(|m| m.borrow().get(&node).copied())
                .and_then(|host_key| {
                    LIST_STATE.with(|m| {
                        m.borrow()
                            .get(&host_key)?
                            .source
                            .borrow()
                            .as_ref()?
                            .reader_scroll
                            .clone()
                    })
                });
            if let Some(report) = report {
                report();
            }
        }));
    });
}

/// A source change under the cells: every bound row is showing the wrong row's data, so
/// rebind the window in place, top to bottom. Reload, reorder, and a width that re-lays every
/// row all come through here. The pool is untouched: a cell keeps its row, the row gets its
/// new content with one slot write.
fn list_populate(host_key: usize) {
    LIST_STATE.with(|m| {
        if let Some(st) = m.borrow_mut().get_mut(&host_key) {
            st.rebind_all = true;
        }
    });
    list_sync(host_key);
}

/// What one sync pass carries out of the `LIST_STATE` borrow into the binding phase.
struct SyncPass {
    host: *mut c_void,
    rowh: f64,
    source: day_spec::ListSource,
    /// `(row, cell)` to bind, ascending.
    work: Vec<(usize, *mut c_void)>,
    /// Cells that left their rows this pass.
    parked: Vec<usize>,
    width: c_int,
    host_width: c_int,
    first_visible: usize,
    automatic: bool,
    /// The header row to bind into the pinned cell, and the cell (sectioned lists).
    pin_bind: Option<(usize, *mut c_void)>,
}

/// Bring the pool to the rows the viewport shows (plus the overscan) and bind whatever needs
/// it. Idempotent and cheap when nothing moved, which is what lets every window report call it.
fn list_sync(host_key: usize) {
    // Phase 1 — under the LIST_STATE borrow: move the pool and snapshot what the binds need.
    // Nothing here re-enters Day; the view's own callbacks post.
    let Some(pass) = LIST_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let st = m.get_mut(&host_key)?;
        let source = st.source.borrow().clone()?;
        let n = (source.len)();
        let rowh = st.row_height.max(1.0);
        // The model holds the whole count, built or not: the scroll bar is how the user
        // reaches rows that do not exist yet. One delta, however many rows.
        unsafe { ffi::day_qt_list_set_count(st.host, n as c_int) };
        let rebind_all = std::mem::take(&mut st.rebind_all);
        // A sectioned list's header rows (docs/list.md "Sections") move only with the source:
        // a reload, or a count that changed before its reload arrived.
        let is_header = source.is_header.clone();
        if let Some(is_header) = &is_header
            && (rebind_all || st.headers_for != Some(n))
        {
            let rows: Vec<c_int> = (0..n)
                .filter(|i| is_header(*i))
                .map(|i| i as c_int)
                .collect();
            unsafe { ffi::day_qt_list_set_headers(st.host, rows.as_ptr(), rows.len() as c_int) };
            st.headers_for = Some(n);
        }
        // The list's own header, footer, empty state and padding rows (docs/list.md "Header,
        // footer and empty state") move the same way, and take back the heights Day laid them
        // at: a splice drops the view's per-row heights, and a padding row out of the window
        // would otherwise count at the item pitch.
        let is_supplement = source.is_supplement.clone();
        let mut supplements = Vec::new();
        if let Some(f) = &is_supplement
            && (rebind_all || st.supplements_for != Some(n))
        {
            supplements = supplement_rows(n, f.as_ref());
            let raw: Vec<c_int> = supplements.iter().map(|i| *i as c_int).collect();
            unsafe { ffi::day_qt_list_set_supplements(st.host, raw.as_ptr(), raw.len() as c_int) };
            st.supplements_for = Some(n);
        }
        let (mut w, mut h) = (0.0_f64, 0.0_f64);
        unsafe { ffi::day_qt_widget_size(st.host, &raw mut w, &raw mut h) };
        // Rows lay out to the VIEWPORT's width, not the host's: under a legacy scroll bar the
        // two differ by the bar, and a row framed to the host slides its trailing column under
        // it. The viewport reports nothing until Qt has laid the view out; the host stands in.
        let vw = unsafe { ffi::day_qt_list_viewport_width(st.host) };
        let width = if vw > 0.0 { vw } else { w }.max(1.0) as c_int;
        // The rows on screen. The view reports none until Qt has laid it out — the framed
        // height stands in until then.
        let (mut first, mut last) = (0 as c_int, -1 as c_int);
        unsafe { ffi::day_qt_list_visible_rows(st.host, &raw mut first, &raw mut last) };
        let (first, last) = if last >= first {
            (first.max(0) as usize, last.max(0) as usize + 1)
        } else {
            let vh = if st.frame_height > 0 {
                f64::from(st.frame_height)
            } else {
                // Never framed and never laid out: build a screen's worth so the first paint is
                // not blank, and let the frame that follows widen the window.
                600.0
            };
            (0, (vh / rowh).ceil() as usize)
        };
        let first_visible = first.min(n.saturating_sub(1));
        let band = first.saturating_sub(LIST_OVERSCAN)..(last + LIST_OVERSCAN).min(n);
        let (parked, need) = st.pool.retarget(band);
        for cell in &parked {
            unsafe { ffi::day_qt_list_park_cell(st.host, *cell as *mut c_void) };
        }
        if rebind_all && (st.automatic || is_header.is_some() || is_supplement.is_some()) {
            // The rows moved under the measured heights (a reload, a reorder): drop them and
            // re-seed the window from the token cache below, before anything is laid out. A
            // sectioned list's headers go too, and the supplementary rows: a reload can move
            // them.
            unsafe { ffi::day_qt_list_clear_heights(st.host) };
        }
        for row in supplements {
            if let Some(h) = st.supplement_heights.get(&(source.token_at)(row)) {
                unsafe {
                    ffi::day_qt_list_set_row_height(st.host, row as c_int, h.ceil() as c_int);
                };
            }
        }
        let mut work: Vec<(usize, *mut c_void)> = Vec::with_capacity(need.len());
        for row in need {
            let header = is_header.as_ref().is_some_and(|f| f(row));
            let cell = match st.pool.take_free(header) {
                Some(spare) => spare as *mut c_void,
                None => unsafe { ffi::day_qt_container_new() },
            };
            st.pool.bind(row, cell as usize);
            // The view owns the cell's place from here: positioned over its row through every
            // scroll and relayout, painted over the row's selection.
            unsafe { ffi::day_qt_list_place_cell(st.host, row as c_int, cell) };
            work.push((row, cell));
        }
        if rebind_all {
            work = st
                .pool
                .bound
                .iter()
                .map(|(row, cell)| (*row, *cell as *mut c_void))
                .collect();
        }
        work.sort_unstable_by_key(|(row, _)| *row);
        if let Some(f) = &is_header {
            for (row, cell) in &work {
                st.pool.set_kind(*cell as usize, f(*row));
            }
        }
        // The pinned header shows the header of the topmost row's section, rebound only when
        // that section changes or the rows moved under it; with no header yet it hides.
        let mut pin_bind = None;
        if is_header.is_some() {
            let host = st.host;
            let pin = *st
                .pin
                .get_or_insert_with(|| unsafe { ffi::day_qt_list_pin_cell(host) } as usize);
            let target = unsafe { ffi::day_qt_list_header_for(host, first_visible as c_int) };
            let target = usize::try_from(target).ok();
            if rebind_all || target != st.pin_row {
                st.pin_row = target;
                match target {
                    Some(row) => pin_bind = Some((row, pin as *mut c_void)),
                    None => unsafe { ffi::day_qt_list_pin_bound(host, -1, 0) },
                }
            }
        }
        if rebind_all && st.automatic {
            for (row, _) in &work {
                if let Some(h) = st.heights.get(&(source.token_at)(*row)) {
                    unsafe {
                        ffi::day_qt_list_set_row_height(st.host, *row as c_int, h.ceil() as c_int);
                    };
                }
            }
        }
        st.last_width = w.max(1.0) as c_int;
        st.cell_width = width;
        Some(SyncPass {
            host: st.host,
            rowh,
            source,
            work,
            parked,
            width,
            host_width: st.last_width,
            first_visible,
            automatic: st.automatic,
            pin_bind,
        })
    }) else {
        return;
    };
    let SyncPass {
        host,
        rowh,
        source,
        work,
        parked,
        width,
        host_width,
        first_visible,
        automatic,
        pin_bind,
    } = pass;
    // Phase 2 — no borrow held (bind_row re-enters with_tree, which may lay out + set_frame the
    // list host, taking LIST_STATE again).
    //
    // A parked cell's ids park with it (day-core's `recycle`), posted: the park arrived from a
    // pass that may sit inside a layout, and a cell this same turn binds again keeps its ids —
    // the re-check AppKit and UIKit both needed.
    for cell in parked {
        Qt::post(Box::new(move || {
            let recycle = LIST_STATE.with(|m| {
                let m = m.borrow();
                let st = m.get(&host_key)?;
                if st.pool.row_of(cell).is_some() {
                    return None; // bound again: its ids are the live row's
                }
                st.source.borrow().as_ref().map(|s| s.recycle.clone())
            });
            if let Some(recycle) = recycle {
                recycle(cell as *mut c_void);
            }
        }));
    }
    if source.viewport.is_some() {
        // What the list shows now (docs/list.md "Visible rows"), asked on the next turn: the
        // view lays out what this pass changed first, and a native layout may still hold the
        // core tree borrow. It supersedes `first_visible`, which is then never called.
        Qt::post(Box::new(move || list_report_viewport(host_key)));
    } else if source.first_visible.is_some() {
        // A viewport change can recompose and lay out a floating header. A
        // native layout may still hold the core tree borrow, so defer delivery.
        Qt::post(Box::new(move || {
            let source = LIST_STATE.with(|m| {
                m.borrow()
                    .get(&host_key)
                    .and_then(|st| st.source.borrow().clone())
            });
            if let Some(source) = source
                && let Some(report) = &source.first_visible
            {
                report(first_visible.min((source.len)().saturating_sub(1)));
            }
        }));
    }
    let mut measured = false;
    for (i, cell) in work {
        // The row's frame, so the bind lays the content out to the size it will be shown at.
        // The view places the cell itself; before its first layout pass the frame is empty and
        // the row pitch stands in.
        let (mut x, mut y, mut cw, mut ch) = (0 as c_int, 0 as c_int, 0 as c_int, 0 as c_int);
        unsafe {
            ffi::day_qt_list_cell_frame(
                host,
                i as c_int,
                &raw mut x,
                &raw mut y,
                &raw mut cw,
                &raw mut ch,
            );
        };
        if cw <= 0 || ch <= 0 {
            (x, y, cw, ch) = (0, (i as f64 * rowh) as c_int, width, rowh as c_int);
        }
        unsafe { ffi::day_qt_set_geometry(cell, x, y, cw, ch) };
        // First use builds the row; a pooled cell takes the new row with a slot write.
        (source.bind_row)(i, cell);
        // Day lays a bound row out at the list's own width — the host's, scroll bar included.
        // The cell is the viewport's width, so a legacy scroll bar covered the row's trailing
        // column (the sidebar's unread counts). Tell Day the cell's real width, the seam every
        // other backend uses (`ListSource::layout_cell`); it is remembered per cell, so later
        // binds start from it and this is a no-op once the widths agree.
        if cw != host_width {
            (source.layout_cell)(cell, f64::from(cw));
        }
        // A self-sizing row is as tall as Day laid it, padding included; the view re-lays its
        // items once per turn and moves the cells below. Later changes come as
        // `RowSizeInvalidated`. A section header sizes the same way in every list, `Uniform`
        // included (docs/list.md "Sections").
        // A supplementary row sizes to its content the same way (docs/list.md "Header, footer
        // and empty state").
        let header = source.is_header.as_ref().is_some_and(|f| f(i));
        let supplement = source.is_supplement.as_ref().is_some_and(|f| f(i));
        if automatic || header || supplement {
            let laid = (source.cell_height)(cell);
            if laid > 0.0 {
                let laid_px = laid.ceil() as c_int;
                unsafe {
                    ffi::day_qt_set_geometry(cell, x, y, cw, laid_px);
                    ffi::day_qt_list_set_row_height(host, i as c_int, laid_px);
                }
                // Headers and supplementary rows stay out of the items' estimate; the view
                // guesses headers apart, and a supplementary row keeps its own height.
                if supplement {
                    list_remember_supplement(host_key, (source.token_at)(i), laid);
                } else if !header {
                    list_remember_height(host_key, (source.token_at)(i), laid);
                    measured = true;
                }
            }
        }
    }
    if measured {
        list_update_estimate(host_key);
    }
    // The pinned copy of the current header: one more cell bound to that header's row, laid to
    // the viewport's width and placed by the view (`DayListView::placePin`).
    if let Some((row, pin)) = pin_bind {
        unsafe { ffi::day_qt_set_geometry(pin, 0, 0, width, rowh as c_int) };
        (source.bind_pinned)(row, pin);
        if width != host_width {
            (source.layout_cell)(pin, f64::from(width));
        }
        let laid = (source.cell_height)(pin);
        let height = if laid > 0.0 { laid.ceil() } else { rowh } as c_int;
        unsafe { ffi::day_qt_list_pin_bound(host, row as c_int, height) };
    }
}

/// Report what the list shows (docs/list.md "Visible rows"): the first and last rows in view and
/// the distance to the end, read from the view as it is laid now. Nothing with no rows, and
/// nothing while row changes wait for their flush: the source already has its new rows and the
/// view not yet, so the old end would be reported against the new row count, and day-core takes
/// a new count with a small distance as a fresh arrival at the end (`on_end_reached` would fire
/// again for rows that were just appended). The flush reports after it.
fn list_report_viewport(host_key: usize) {
    let found = LIST_STATE.with(|m| {
        let m = m.borrow();
        let st = m.get(&host_key)?;
        if st.pending.is_some() {
            return None;
        }
        let report = st.source.borrow().as_ref()?.viewport.clone()?;
        Some((st.host, report))
    });
    let Some((host, report)) = found else {
        return;
    };
    let (mut first, mut last, mut end) = (0 as c_int, 0 as c_int, 0.0_f64);
    if unsafe { ffi::day_qt_list_viewport(host, &raw mut first, &raw mut last, &raw mut end) } == 0
    {
        return;
    }
    report(day_spec::ListViewport {
        first: first.max(0) as usize,
        last: last.max(first).max(0) as usize,
        end_distance: end.max(0.0),
    });
}

/// Remember that the supplementary row with `token` was laid `height` tall.
fn list_remember_supplement(host_key: usize, token: u64, height: f64) {
    LIST_STATE.with(|m| {
        if let Some(st) = m.borrow_mut().get_mut(&host_key) {
            st.supplement_heights.insert(token, height);
        }
    });
}

/// Remember that the row with `token` was laid `height` tall (`Automatic` lists), keeping the
/// running sum current for the estimate.
fn list_remember_height(host_key: usize, token: u64, height: f64) {
    LIST_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let Some(st) = m.get_mut(&host_key) else {
            return;
        };
        if st.heights.len() >= LIST_HEIGHT_CACHE && !st.heights.contains_key(&token) {
            st.heights.clear();
            st.height_sum = 0.0;
        }
        if let Some(old) = st.heights.insert(token, height) {
            st.height_sum -= old;
        }
        st.height_sum += height;
    });
}

/// Give the model the running average of the measured heights for the rows it has not
/// measured, when it moved by a point or more: every change is one relayout.
fn list_update_estimate(host_key: usize) {
    let update = LIST_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let st = m.get_mut(&host_key)?;
        if st.heights.is_empty() {
            return None;
        }
        let avg = (st.height_sum / st.heights.len() as f64).round() as c_int;
        if avg <= 0 || avg == st.estimate {
            return None;
        }
        st.estimate = avg;
        Some((st.host, avg))
    });
    if let Some((host, avg)) = update {
        unsafe { ffi::day_qt_list_set_estimate(host, avg) };
    }
}

/// A self-sizing row Day re-laid at a new height (`ListPatch::RowSizeInvalidated`), applied on
/// the next event-loop turn: it arrives inside a day-core layout pass, and the view's relayout
/// must not run under it. The pool says which row the cell shows now.
fn schedule_list_row_height(host_key: usize, cell: usize, height: f64) {
    Qt::post(Box::new(move || {
        let found = LIST_STATE.with(|m| {
            let m = m.borrow();
            let st = m.get(&host_key)?;
            // The pinned header re-laid: only the pin changes height (docs/list.md "Sections").
            if st.pin == Some(cell) {
                let row = st.pin_row?;
                unsafe {
                    ffi::day_qt_list_pin_bound(st.host, row as c_int, height.ceil() as c_int);
                };
                return None;
            }
            let row = st.pool.row_of(cell)?;
            // A header's height is not an item's: it stays out of the estimate. A supplementary
            // row's (the empty state re-laid to a new list height) is remembered on its own.
            let kind = st.source.borrow().as_ref().map(|s| {
                let token = (s.token_at)(row);
                if s.is_supplement.as_ref().is_some_and(|f| f(row)) {
                    (Some(token), true)
                } else if s.is_header.as_ref().is_some_and(|f| f(row)) {
                    (None, false)
                } else {
                    (Some(token), false)
                }
            });
            Some((st.host, row, kind))
        });
        let Some((host, row, kind)) = found else {
            return;
        };
        unsafe { ffi::day_qt_list_set_row_height(host, row as c_int, height.ceil() as c_int) };
        match kind {
            Some((Some(token), true)) => list_remember_supplement(host_key, token, height),
            Some((Some(token), false)) => {
                list_remember_height(host_key, token, height);
                list_update_estimate(host_key);
            }
            _ => {}
        }
    }));
}

/// A right-click landed on a widget with a summon-time menu: ask the provider and hand Qt a
/// fresh `QMenu` (null when the provider answers empty). Runs synchronously inside Qt's
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

/// A menu button was clicked (`day_qt_button_menu_fn`; `key` is its widget pointer): ask the
/// provider now and hand Qt a fresh `QMenu`, or null when it answers empty.
extern "C" fn on_button_menu(
    key: u64,
    _x: std::os::raw::c_double,
    _y: std::os::raw::c_double,
) -> *mut c_void {
    ffi_guard::contain(std::ptr::null_mut(), || {
        let Some(f) = BUTTON_MENUS.with(|t| t.get(key as usize)) else {
            return std::ptr::null_mut();
        };
        // The provider may remove this button; then no menu opens (docs/menus.md).
        let Some(items) = day_spec::resolve_button_menu(&f, || {
            BUTTON_MENUS.with(|t| t.get(key as usize)).is_some()
        }) else {
            return std::ptr::null_mut();
        };
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
extern "C" fn on_text_selected(id: u64, start: c_int, end: c_int) {
    ffi_guard::contain((), || {
        let start = usize::try_from(start).unwrap_or(0);
        let end = usize::try_from(end).unwrap_or(0).max(start);
        emit(NodeId(id), Event::TextSelected { start, end });
    });
}
extern "C" fn on_text_scale() {
    ffi_guard::contain((), day_core::note_text_size_changed);
}
extern "C" fn on_link(id: u64, s: *const c_char) {
    ffi_guard::contain((), || {
        let url = unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned();
        emit(NodeId(id), Event::LinkActivated(url));
    });
}
extern "C" fn on_slider(id: u64, v: c_int, committed: c_int) {
    ffi_guard::contain((), || {
        let (min, max) = RANGES.with(|r| r.get(id as usize)).unwrap_or((0.0, 1.0));
        let value = min + (f64::from(v) / 1000.0) * (max - min);
        // The live value always; the settled one additionally, so a drag records once (the shim
        // decides which is which — see `day_qt_slider_new`).
        emit(NodeId(id), Event::ValueChanged(value));
        if committed != 0 {
            emit(NodeId(id), Event::ValueCommitted(value));
        }
    });
}
/// A key's phase from the shim's wire (docs/menus.md "Keys"): 0 down, 1 repeat (Qt's
/// `isAutoRepeat`), 2 up.
fn key_phase(phase: c_int) -> day_spec::KeyPhase {
    match phase {
        1 => day_spec::KeyPhase::Repeat,
        2 => day_spec::KeyPhase::Up,
        _ => day_spec::KeyPhase::Down,
    }
}

/// Key callback from the C++ key filter (docs/menus.md): `code` 0–3 is an arrow, 10–19 a
/// digit, `phase` 0 down, 1 repeat, 2 up. Answers whether the app claimed it: a canvas nobody
/// hung `.on_key` on keeps none of them, so an enclosing scroll area still scrolls with the
/// keyboard. An up comes only for a down the node claimed, so it is always the node's.
extern "C" fn on_key(id: u64, code: c_int, modifiers: c_int, phase: c_int) -> c_int {
    ffi_guard::contain(0, || {
        let key = match code {
            0 => "ArrowLeft",
            1 => "ArrowRight",
            2 => "ArrowUp",
            3 => "ArrowDown",
            10..=19 => day_spec::KeyEvent::DIGITS[(code - 10) as usize],
            _ => return 0,
        };
        let node = NodeId(id);
        let phase = key_phase(phase);
        if phase != day_spec::KeyPhase::Up && !day_spec::keys::handled(node) {
            return 0;
        }
        emit(
            node,
            Event::Key(day_spec::KeyEvent::named(key, modifiers as u8).with_phase(phase)),
        );
        1
    })
}

/// A line edit's Up, Down or Escape (docs/textfield.md "Keys"): claimed, and with `deliver`
/// reported, when the field has a key handler and no modifier is held. The release of a key
/// the field claimed comes as `phase` 2 and is the handler's whatever the modifiers.
extern "C" fn on_field_key(
    id: u64,
    name: *const c_char,
    modifiers: c_int,
    deliver: c_int,
    phase: c_int,
) -> c_int {
    ffi_guard::contain(0, || {
        // SAFETY: the shim passes one of its own NUL-terminated strings, valid for the call.
        let name = unsafe { std::ffi::CStr::from_ptr(name) }.to_string_lossy();
        let node = NodeId(id);
        let phase = key_phase(phase);
        if phase == day_spec::KeyPhase::Up {
            let up = day_spec::KeyEvent::named(name, modifiers as u8).with_phase(phase);
            emit(node, Event::Key(up));
            return 1;
        }
        if !day_spec::keys::field_claims(node, &name, modifiers as u8) {
            return 0;
        }
        if deliver != 0 {
            emit(
                node,
                Event::Key(day_spec::KeyEvent::named(name, 0).with_phase(phase)),
            );
        }
        1
    })
}

/// Whether `id` took the whole keyboard (`Decorate::keyboard`, docs/menus.md): the C++ key
/// filter asks before it names a letter.
extern "C" fn on_text_key_query(id: u64) -> c_int {
    ffi_guard::contain(0, || c_int::from(day_spec::keys::text(NodeId(id))))
}

/// A key on the full route (docs/menus.md "The full route"): `name` is the day name (empty
/// for text no single key is behind), `text` what it typed or null, both valid for the call,
/// `phase` 0 down, 1 repeat, 2 up (never with text). Answers whether the app claimed it.
extern "C" fn on_text_key(
    id: u64,
    name: *const c_char,
    text: *const c_char,
    modifiers: c_int,
    phase: c_int,
) -> c_int {
    ffi_guard::contain(0, || {
        // SAFETY: the shim passes NUL-terminated UTF-8 strings (or null) that outlive this call.
        let read = |p: *const c_char| {
            (!p.is_null()).then(|| {
                unsafe { std::ffi::CStr::from_ptr(p) }
                    .to_string_lossy()
                    .into_owned()
            })
        };
        let key = read(name).unwrap_or_default();
        let node = NodeId(id);
        let phase = key_phase(phase);
        if phase != day_spec::KeyPhase::Up && !day_spec::keys::handled(node) {
            return 0;
        }
        let modifiers = modifiers as u8;
        let event = match read(text) {
            Some(text) => day_spec::KeyEvent::typed_by(key, text, modifiers),
            None => day_spec::KeyEvent::named(key, modifiers),
        };
        emit(node, Event::Key(event.with_phase(phase)));
        1
    })
}

/// Escape over a presented sheet cover (docs/cover.md "Modal sheet"): the topmost cover hears
/// it as its back request, whichever sheet's shortcut fired.
extern "C" fn on_cover_escape(id: u64) {
    ffi_guard::contain((), || {
        let node = COVERS
            .with(|c| c.borrow().last().map(|(_, node)| *node))
            .unwrap_or(NodeId(id));
        emit(
            node,
            Event::NavBack {
                already_popped: false,
            },
        );
    });
}

/// Focus callback from the C++ event filter (docs/focus.md).
/// kind: 0 = lost, 1 = gained, 2 = submitted (line-edit return key).
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
/// 4..=6 pinch began/changed/ended (tx = cumulative scale); 7..=9 pan (tx/ty = delta);
/// 10..=12 hover; 13..=15 rotate began/changed/ended (tx = cumulative radians, clockwise);
/// 16 long press; 17 double tap; 18 press in; 19 press out.
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
            10..=12 => Event::Hover {
                phase: seq_phase(phase - 10),
                location: at,
            },
            13..=15 => Event::Rotate {
                phase: seq_phase(phase - 13),
                rotation: tx,
                location: at,
            },
            16 => Event::LongPress(at),
            17 => Event::DoubleTap(at),
            18 | 19 => Event::Press {
                pressed: phase == 18,
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

/// One contact list from a touches recognizer (docs/gestures.md "Touches"): six numbers per
/// contact, as `contacts` decodes them.
extern "C" fn on_touches(id: u64, data: *const c_double, count: c_int) {
    ffi_guard::contain((), || {
        let len = usize::try_from(count).unwrap_or(0) * 6;
        if data.is_null() || len == 0 {
            return;
        }
        // SAFETY: the shim passes `count` contacts of six doubles each, valid for the call.
        let numbers = unsafe { std::slice::from_raw_parts(data, len) };
        emit(NodeId(id), Event::Touches(contacts(numbers)));
    });
}

/// Decode the shim's contact numbers: id, phase (0 began, 1 moved, 2 stationary, 3 ended,
/// 4 cancelled), x, y, pressure, kind (0 touch, 1 mouse, 2 stylus) per contact.
fn contacts(numbers: &[f64]) -> Vec<day_spec::Touch> {
    use day_spec::{Point, Touch, TouchPhase};
    numbers
        .as_chunks::<6>()
        .0
        .iter()
        .map(|c| {
            let phase = match c[1] as i32 {
                0 => TouchPhase::Began,
                2 => TouchPhase::Stationary,
                3 => TouchPhase::Ended,
                4 => TouchPhase::Cancelled,
                _ => TouchPhase::Moved,
            };
            Touch::new(c[0] as u64, phase, Point::new(c[2], c[3]))
                .with_pressure(c[4])
                .with_kind(pointer_kind(c[5] as c_int))
        })
        .collect()
}

/// The shim's device code (0 touch, 1 mouse, 2 stylus) as a [`day_spec::PointerKind`].
fn pointer_kind(code: c_int) -> day_spec::PointerKind {
    match code {
        0 => day_spec::PointerKind::Touch,
        2 => day_spec::PointerKind::Stylus,
        _ => day_spec::PointerKind::Mouse,
    }
}

/// The kind of device behind the latest pointer input changed (docs/gestures.md "Input
/// kind"); the shim reports only a change.
extern "C" fn on_pointer_kind(code: c_int) {
    ffi_guard::contain((), || {
        emit(
            day_spec::WINDOW_NODE,
            Event::PointerKindChanged(pointer_kind(code)),
        );
    });
}

/// A realized container, label, canvas, image or divider: a view that takes no pointer input
/// until a recognizer, a menu, a drag, a drop or focus is attached, so a press on it reaches
/// whatever is behind it (docs/gestures.md "Pass-through").
fn passive(w: *mut c_void) -> QtHandle {
    // SAFETY: `w` is the widget the shim just made; the shim only sets a property on it.
    unsafe { ffi::day_qt_set_passive(w) };
    QtHandle(w)
}

fn slider_ticks(value: f64, min: f64, max: f64) -> c_int {
    if max <= min {
        return 0;
    }
    (((value - min) / (max - min)) * 1000.0).round() as c_int
}

/// A `0.0..=1.0` fraction as `QProgressBar` ticks (0..1000), clamped.
fn progress_ticks(fraction: f64) -> c_int {
    (fraction.clamp(0.0, 1.0) * 1000.0).round() as c_int
}

/// Renderers registered by external Day Piece crates (§8.2).
#[distributed_slice]
pub static RENDERERS: [fn() -> Renderer<Qt>];

/// A live secondary window (docs/windows.md): the day root node id, the `DayWindow`, and
/// its inner content widget (the adopted handle).
struct QtWin {
    #[cfg(target_os = "macos")]
    id: NodeId,
    #[cfg(target_os = "macos")]
    tab_group: Option<String>,
    win: *mut c_void,
    content: *mut c_void,
}

pub struct Qt {
    registry: Registry<Qt>,
    window: *mut c_void,
    secondary: Vec<QtWin>,
    /// Each window's toolbar as the shim holds it (docs/toolbars.md), by window pointer.
    toolbars: std::collections::HashMap<usize, day_spec::ToolbarMirror>,
    /// The status items shown (docs/status-item.md): each spec and its `QSystemTrayIcon`.
    trays: Vec<(day_spec::StatusItemSpec, *mut c_void)>,
}

impl Qt {
    #[must_use]
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
            toolbars: std::collections::HashMap::new(),
            trays: Vec::new(),
        }
    }

    /// The `DayWindow` whose content container is `host`: a secondary window by its content, or
    /// the primary, which `ready` handed out as the `DayWindow` itself.
    fn qt_window_for(&self, host: &QtHandle) -> Option<*mut c_void> {
        match self.secondary.iter().find(|w| w.content == host.0) {
            Some(w) => Some(w.win),
            None if !self.window.is_null() && host.0 == self.window => Some(self.window),
            None => None,
        }
    }

    /// The dialog parent (docs/windows.md): the ACTIVE Day window at present time,
    /// primary as the fallback.
    fn present_parent(&self) -> *mut c_void {
        self.secondary
            .iter()
            .find(|w| unsafe { ffi::day_qt_window_is_active(w.win) } != 0)
            .map_or(self.window, |w| w.win)
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
    let ptr = unsafe { ffi::day_qt_resource_data(cstr(&respath).as_ptr(), &raw mut len) };
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { day_spec::resource::Resource::from_raw(ptr.cast::<u8>(), len, Box::new(())) })
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
#[must_use]
#[expect(
    clippy::match_same_arms,
    reason = "one row per text style, in the order of the type scale it approximates"
)]
pub fn qt_style(f: Font) -> (f64, day_spec::FontWeight) {
    use day_spec::FontWeight::{Regular, Semibold};
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
        Font::System(pt) | Font::Custom(_, pt) => (pt, Regular),
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
    let html = qt_label_markup(text, runs, base_pt);
    unsafe { ffi::day_qt_label_set_rich_text(w, cstr(&html).as_ptr()) };
}

/// Qt's spelling of a decoration style in its rich-text CSS (`text-underline-style`). Qt has no
/// double rule, so `Double` draws a single one.
fn qt_underline_style(style: day_spec::DecorationStyle) -> &'static str {
    match style {
        day_spec::DecorationStyle::Single | day_spec::DecorationStyle::Double => "solid",
        day_spec::DecorationStyle::Dotted => "dotted",
        day_spec::DecorationStyle::Wavy => "wave",
    }
}

/// A label's runs as Qt rich text: the shared serializer's markup, with what Qt's rich text
/// takes beyond it.
///
/// The shared markup underlines with `<u>`, which Qt reads as a single line and which overrides
/// any style around it. A run whose underline has a style of its own is therefore serialized
/// without the underline, inside a span carrying Qt's `text-underline-style` and
/// `text-decoration-color` (both parsed by Qt's rich text into the `QTextCharFormat` underline
/// style and color). Qt strikes through in one style and the text color, so a strikethrough is
/// written plain (`<s>`) rather than leaving its color to recolor an underline. A run's small
/// capitals are dropped (see below); the label's own `small_caps` is the face's `smcp`. Every other run is the shared markup verbatim:
/// serializing one run at a time writes exactly what one call over the whole text writes.
fn qt_label_markup(text: &str, runs: &[day_spec::TextRun], base_pt: f64) -> String {
    use std::fmt::Write as _;
    let dialect = day_spec::MarkupDialect::QtHtml;
    let mut out = String::with_capacity(text.len() + runs.len() * 32);
    let mut at = 0usize;
    for r in runs {
        let Some(styled) = text.get(r.range.clone()) else {
            continue;
        };
        if r.range.start > at
            && let Some(plain) = text.get(at..r.range.start)
        {
            out.push_str(&day_spec::runs_to_markup(plain, &[], dialect, base_pt));
        }
        let mut inner = r.clone();
        inner.range = 0..styled.len();
        // Qt strikes through in one style and the text color only.
        if inner.strikethrough.is_some() {
            inner.strikethrough = Some(day_spec::Decoration::SINGLE);
        }
        // Qt's rich text cannot ask a face for `smcp`: its `font-variant:small-caps` draws
        // scaled capitals, which Day never synthesizes, and measures them inconsistently (a
        // label laid out at its own natural width wrapped its last word on Linux). The run
        // keeps its ordinary lowercase.
        inner.font.small_caps = false;
        match r
            .underline
            .filter(|d| d.style != day_spec::DecorationStyle::Single)
        {
            Some(d) => {
                inner.underline = None;
                let _ = write!(
                    out,
                    "<span style=\"text-underline-style:{};",
                    qt_underline_style(d.style)
                );
                if let Some(c) = d.color {
                    let _ = write!(out, "text-decoration-color:{};", css_hex(c.resolved()));
                }
                out.push_str("\">");
                out.push_str(&day_spec::runs_to_markup(
                    styled,
                    &[inner],
                    dialect,
                    base_pt,
                ));
                out.push_str("</span>");
            }
            None => {
                out.push_str(&day_spec::runs_to_markup(
                    styled,
                    &[inner],
                    dialect,
                    base_pt,
                ));
            }
        }
        at = r.range.end;
    }
    if let Some(tail) = text.get(at..) {
        out.push_str(&day_spec::runs_to_markup(tail, &[], dialect, base_pt));
    }
    out
}

/// `#rrggbb` for a color in Qt's rich-text CSS.
fn css_hex(c: day_spec::Color) -> String {
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b))
}

/// Remember a label's resolved point size for [`set_label_runs`].
fn remember_label_pt(w: *mut c_void, f: day_spec::FontSpec) {
    LABEL_PT.with(|t| t.insert(w as usize, f.resolved_points(qt_style(f.style).0)));
}

/// Apply a `Font::Custom` family on top of `day_qt_label_set_font` (which set size/weight/italic).
/// The family was registered with the `QFontDatabase` in `run()`; an unknown name falls back to the
/// default family inside Qt.
unsafe fn apply_custom_family(w: *mut c_void, spec: day_spec::FontSpec) {
    if let Font::Custom(family, _) = spec.style {
        unsafe { ffi::day_qt_label_set_font_family(w, cstr(family).as_ptr()) };
    }
}

/// Day weight → `QFont::Weight` numeric value (Thin=100 … Black=900).
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

/// A label's base font as the C++ shim takes it.
struct FontParams {
    /// The resolved size: the style's (or the explicit) points times `FontSpec::scale`.
    pt: f64,
    weight: c_int,
    italic: c_int,
    tabular: c_int,
    small_caps: c_int,
    oldstyle: c_int,
}

fn font_params(spec: day_spec::FontSpec) -> FontParams {
    let (pt, inherent) = qt_style(spec.style);
    FontParams {
        pt: spec.resolved_points(pt),
        weight: qt_weight(spec.weight.unwrap_or(inherent)),
        italic: c_int::from(spec.italic),
        tabular: c_int::from(spec.tabular),
        small_caps: c_int::from(spec.small_caps),
        oldstyle: c_int::from(spec.oldstyle_figures),
    }
}

/// Set a label's base font (size, weight, italic and the font features).
fn apply_label_font(w: *mut c_void, spec: day_spec::FontSpec) {
    let f = font_params(spec);
    // SAFETY: `w` is a live label widget the caller holds; the shim only reads its font.
    unsafe {
        ffi::day_qt_label_set_font(
            w,
            f.pt,
            f.weight,
            f.italic,
            f.tabular,
            f.small_caps,
            f.oldstyle,
        );
    }
}

/// Apply the paragraph attributes (docs/text.md) at realize and on `LabelPatch::Paragraph`.
fn apply_paragraph(w: *mut c_void, p: &day_spec::props::LabelParagraph) {
    let truncation = match p.truncation {
        day_spec::props::Truncation::Tail => 0,
        day_spec::props::Truncation::Head => 1,
        day_spec::props::Truncation::Middle => 2,
    };
    let max_lines = c_int::try_from(p.max_lines).unwrap_or(c_int::MAX);
    // SAFETY: `w` is a live label widget; the shim ignores any other widget.
    unsafe {
        ffi::day_qt_label_set_paragraph(w, max_lines, truncation, p.letter_spacing, p.line_height);
    };
}

/// A label's `TextAlign` as the shim's code.
fn label_align(a: day_spec::props::TextAlign) -> c_int {
    match a {
        day_spec::props::TextAlign::Leading => 0,
        day_spec::props::TextAlign::Center => 1,
        day_spec::props::TextAlign::Trailing => 2,
        day_spec::props::TextAlign::Justified => 3,
    }
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
    /// Sidebar+detail split (nav host Sidebar) vs. a pure push/pop stack (`nav_stack`).
    split: bool,
    /// Whether the sidebar pane is showing. Tracked rather than read back because the shim
    /// exposes `day_qt_set_visible` and no getter — what the sidebar toggle flips.
    sidebar_shown: bool,
    /// The content-list pane (docs/navigation.md), splitter pane 1. It exists on every host
    /// and is hidden on one that declared no content list; `list_width` is `Some` only where
    /// the app asked for the pane (`NavProps::list_width`). The pane PERSISTS through every
    /// presentation (`Cap::NavContentList` = Native): a narrow window collapses the sidebar
    /// into the stack and keeps the list beside the detail, as a narrow Mail.app does.
    list_pane: *mut std::os::raw::c_void,
    list_width: Option<f64>,
    /// The `Pane::List` page, once it has one — sized by its pane, never a member of the
    /// detail stack.
    list_page: Option<QtHandle>,
    /// Whether the pane is showing (`NavProps::list_visible`, then `NavPatch::ListVisible`).
    list_shown: bool,
    /// Whether the pane has been given the app's requested width yet. A pane hidden since
    /// construction keeps the constructor's placeholder size, so the first reveal places it.
    list_positioned: bool,
    /// Stack presentation: title per level (index 0 = root) for the back header — desktop has
    /// no system back affordance, so the header gives a pushed page its way out.
    titles: Vec<String>,
}

/// Show/hide the sidebar of the navigation host `host` — what the sidebar affordance a
/// nav host contributes for itself drives (`day_spec::SIDEBAR_TOGGLE_ID`, docs/toolbars.md).
/// Per host, so a second window's button toggles that window's own pane. `false` when the
/// host is not split (nothing to toggle).
pub(crate) fn toggle_sidebar(host: *mut std::os::raw::c_void) -> bool {
    // The pane flips outside the borrow: the sibling panes' resize reports read `NAV_STATE`.
    let flip = NAV_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let st = m.get_mut(&(host as usize))?;
        if !st.split {
            return None;
        }
        st.sidebar_shown = !st.sidebar_shown;
        Some((st.sidebar_pane, st.sidebar_shown))
    });
    match flip {
        Some((pane, shown)) => {
            unsafe { ffi::day_qt_set_visible(pane, i32::from(shown)) };
            true
        }
        None => false,
    }
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
        );
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
        ffi::day_qt_nav_header_update(host, c_int::from(visible), cstr(&title).as_ptr());
    }
    nav_sync_panes(host);
}

/// Is this page the host's sidebar pane?
fn is_sidebar_page(page: *mut std::os::raw::c_void) -> bool {
    PAGE_PANE
        .with(|m| m.borrow().get(&(page as usize)).copied() == Some(day_spec::props::Pane::Sidebar))
}

/// Is this page the host's content-list pane? Never part of the detail stack, at any
/// presentation (docs/navigation.md).
fn is_list_page(page: *mut std::os::raw::c_void) -> bool {
    PAGE_PANE
        .with(|m| m.borrow().get(&(page as usize)).copied() == Some(day_spec::props::Pane::List))
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
            // Only the top detail page shows; the sidebar page, when split, sits outside that,
            // and the list page always does.
            let detail: Vec<QtHandle> = s
                .pages
                .iter()
                .filter(|(p, _)| !(split && is_sidebar_page(p.0) || is_list_page(p.0)))
                .map(|(p, _)| *p)
                .collect();
            let last = detail.len().saturating_sub(1);
            for (i, page) in detail.iter().enumerate() {
                unsafe { ffi::day_qt_set_visible(page.0, i32::from(i == last)) };
            }
        }
    });
    // Deferred one turn, like the pop path: hiding a QSplitter pane does not resize its sibling
    // until Qt runs its own layout pass, so reading the pane sizes now would report the old
    // detail width and leave the page laid out for a window that still had a sidebar.
    let host_addr = host as usize;
    <Qt as Platform>::post(Box::new(move || {
        nav_sync_header(host_addr as *mut std::os::raw::c_void);
    }));
}

/// Report both pane sizes so `NavLayout` re-lays page content (enqueue-only, §8.3).
fn nav_sync_panes(host: *mut std::os::raw::c_void) {
    let reports: Vec<(NodeId, Size)> = NAV_STATE.with(|m| {
        let m = m.borrow();
        let Some(state) = m.get(&(host as usize)) else {
            return Vec::new();
        };
        let (mut sw, mut sh, mut lw, mut lh, mut dw, mut dh) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        unsafe {
            ffi::day_qt_widget_size(state.sidebar_pane, &raw mut sw, &raw mut sh);
            ffi::day_qt_widget_size(state.list_pane, &raw mut lw, &raw mut lh);
            ffi::day_qt_widget_size(state.detail_pane, &raw mut dw, &raw mut dh);
        }
        if sh <= 0.0 && dh <= 0.0 {
            return Vec::new();
        }
        state
            .pages
            .iter()
            .map(|(page, id)| {
                // A page in the sidebar pane is sized by it, the list page by its pane;
                // everything else fills the detail.
                let size = if state.split && is_sidebar_page(page.0) {
                    Size::new(sw, sh)
                } else if is_list_page(page.0) {
                    Size::new(lw, lh)
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
    ffi_guard::contain((), || {
        nav_sync_panes(host);
        // The window toolbar's columns follow the panes (docs/toolbars.md).
        unsafe { ffi::day_qt_toolbar_sync_columns(host) };
    });
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
            ffi::day_qt_widget_size(state.content_pane, &raw mut cw, &raw mut ch);
            ffi::day_qt_widget_size(state.panel_pane, &raw mut pw, &raw mut ph);
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
// Split (docs/split.md): a QSplitter of two plain panes, either orientation, the divider
// placed by the bound share and a drag reported back as the new share.
// ---------------------------------------------------------------------------

struct SplitState {
    first_pane: *mut std::os::raw::c_void,
    second_pane: *mut std::os::raw::c_void,
    node: NodeId,
    /// `(pane NodeId, is-second)` in attach order, for frame reports.
    panes: Vec<(NodeId, bool)>,
    /// The first pane's share as last patched or reported.
    share: f64,
    min_pane: f64,
    /// The divider has been placed once; before that the splitter has no size to place it in.
    placed: bool,
    /// Day is moving the divider itself: the move that follows is not a report.
    suppress: bool,
}

/// Report both pane sizes so `SplitLayout` re-lays content (`inspector_sync_panes`'s
/// counterpart).
fn split_sync_panes(host: *mut std::os::raw::c_void) {
    let reports: Vec<(NodeId, Size)> = SPLIT_STATE.with(|m| {
        let m = m.borrow();
        let Some(state) = m.get(&(host as usize)) else {
            return Vec::new();
        };
        let (mut aw, mut ah, mut bw, mut bh) = (0.0, 0.0, 0.0, 0.0);
        unsafe {
            ffi::day_qt_widget_size(state.first_pane, &raw mut aw, &raw mut ah);
            ffi::day_qt_widget_size(state.second_pane, &raw mut bw, &raw mut bh);
        }
        if aw <= 0.0 || ah <= 0.0 {
            return Vec::new();
        }
        state
            .panes
            .iter()
            .map(|(id, second)| {
                let size = if *second {
                    Size::new(bw, bh)
                } else {
                    Size::new(aw, ah)
                };
                (*id, size)
            })
            .collect()
    });
    for (id, size) in reports {
        emit(id, Event::FrameChanged(size));
    }
}

/// A layout pass on the splitter: the first one with a size places the divider by the share;
/// every one re-lays the panes' content.
extern "C" fn split_resized(host: *mut std::os::raw::c_void) {
    ffi_guard::contain((), || {
        let place = SPLIT_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let state = m.get_mut(&(host as usize))?;
            let (mut w, mut h) = (0.0, 0.0);
            unsafe { ffi::day_qt_widget_size(host, &raw mut w, &raw mut h) };
            if state.placed || w <= 0.0 || h <= 0.0 {
                return None;
            }
            state.placed = true;
            state.suppress = true;
            Some(state.share)
        });
        if let Some(share) = place {
            unsafe { ffi::day_qt_split_set_fraction(host, share) };
            SPLIT_STATE.with(|m| {
                if let Some(state) = m.borrow_mut().get_mut(&(host as usize)) {
                    state.suppress = false;
                }
            });
        }
        split_sync_panes(host);
    });
}

/// The divider moved: a drag reports the new share; Day's own placement does not.
extern "C" fn split_moved(host: *mut std::os::raw::c_void) {
    ffi_guard::contain((), || {
        let share = unsafe { ffi::day_qt_split_fraction(host) }.clamp(0.0, 1.0);
        let report = SPLIT_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let state = m.get_mut(&(host as usize))?;
            if state.suppress || !state.placed || (share - state.share).abs() <= 1e-4 {
                return None;
            }
            state.share = share;
            Some(state.node)
        });
        if let Some(node) = report {
            emit(node, Event::ValueChanged(share));
        }
        split_sync_panes(host);
    });
}

// ---------------------------------------------------------------------------
// The navigation suite (docs/navigation.md): a QTabWidget host that owns its page widgets.
// ---------------------------------------------------------------------------

/// The navigation suite (`NavPresentation::Tabs`): a `QTabWidget` whose bar IS the host's row list.
struct NavSuite {
    /// The host's own node — what the `QTabWidget`'s signal carries, so a click can find its suite
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
    keys: Vec<String>,
}

/// Put the rows on the bar, for as many tabs as currently exist.
///
/// Called from both sides, because either can arrive first: the pages are inserted as the tree is
/// built, and the rows come with the nav menu nested somewhere inside the sidebar page. Whichever
/// lands last completes the bar.
fn suite_apply_rows(host: *mut std::os::raw::c_void) {
    NAV_SUITES.with(|m| {
        let m = m.borrow();
        let Some(suite) = m.get(&(host as usize)) else {
            return;
        };
        for (i, title) in suite.titles.iter().enumerate() {
            // Row i is tab i: the sidebar page is parked under the widget, never a tab.
            if i >= suite.pages.len() {
                break;
            }
            let at = i as c_int;
            unsafe { ffi::day_qt_tabs_set_title(host, at, cstr(title).as_ptr()) };
            let icon = suite
                .icons
                .get(i)
                .and_then(|o| o.as_deref())
                .filter(|n| !n.is_empty())
                .map_or_default(icon_file_path);
            if !icon.is_empty() {
                unsafe { ffi::day_qt_tabs_set_icon(host, at, cstr(&icon).as_ptr()) };
            }
        }
    });
}

/// Report each destination's content size so `NavLayout` re-lays it (enqueue-only, §8.3) — the
/// suite's counterpart of the list's own sizing, and for the same reason: `QTabWidget` owns page
/// geometry.
fn nav_suite_sync(host: *mut std::os::raw::c_void) {
    let reports: Vec<(NodeId, Size)> = NAV_SUITES.with(|m| {
        let m = m.borrow();
        let Some(suite) = m.get(&(host as usize)) else {
            return Vec::new();
        };
        let (mut w, mut h) = (0.0, 0.0);
        unsafe { ffi::day_qt_tabs_content_size(host, &raw mut w, &raw mut h) };
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
type NavRow = (NodeId, Vec<String>, Vec<Option<String>>, Vec<String>);

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
            Event::SelectionChanged(i64::from(index)),
        );
    });
}

extern "C" fn document_tab_event(id: u64, action: c_int, index: c_int, to: c_int) {
    ffi_guard::contain((), || {
        let key = NAV_SUITES
            .with(|m| {
                m.borrow()
                    .values()
                    .find(|s| s.host.0 == id)
                    .and_then(|s| s.keys.get(index.saturating_sub(1) as usize))
                    .cloned()
            })
            .unwrap_or_default();
        let tag = match action {
            0 => "day:tab-new",
            1 => "day:tab-close",
            _ => "day:tab-move",
        };
        emit(
            NodeId(id),
            Event::Custom {
                tag,
                num: f64::from((to - 1).max(0)),
                text: key,
            },
        );
    });
}

/// Report each tab page's content size so `NavLayout` re-lays it (enqueue-only, §8.3).
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
            Event::WindowResized(Size::new(f64::from(w), f64::from(h))),
        );
    });
}
extern "C" fn win_closed(node: u64) {
    // Node 0 is the primary, which reports its close only while the app keeps running
    // (docs/windows.md "Keeping the app running").
    let target = if node == 0 {
        day_spec::WINDOW_NODE
    } else {
        day_spec::NodeId(node)
    };
    ffi_guard::contain((), || emit(target, Event::WindowClosed));
}
extern "C" fn win_focused(node: u64, active: c_int) {
    ffi_guard::contain((), || {
        emit(day_spec::NodeId(node), Event::WindowFocused(active != 0));
    });
}

/// A window's display state changed (docs/windows.md "Window properties"). The shim dedupes
/// and keys it by node id, 0 standing for the primary window.
extern "C" fn win_state(node: u64, code: c_int) {
    ffi_guard::contain((), || {
        let state = day_spec::WindowState::from_code(code);
        log::debug!("qt: window state now {state:?}");
        let target = if node == 0 {
            day_spec::WINDOW_NODE
        } else {
            day_spec::NodeId(node)
        };
        emit(target, Event::WindowStateChanged(state));
    });
}

extern "C" fn window_resized(w: c_int, h: c_int) {
    ffi_guard::contain((), || {
        let size = Size::new(f64::from(w), f64::from(h));
        LAST_WINDOW_SIZE.with(|c| c.set(size));
        emit(day_spec::WINDOW_NODE, Event::WindowResized(size));
        // Presented emulated covers track the content area (docs/cover.md: the frame is
        // native-owned while presented).
        COVERS.with(|c| {
            for (cover, node) in c.borrow().iter() {
                let (y, size) = cover_frame(*cover, size);
                unsafe {
                    ffi::day_qt_set_geometry(
                        *cover,
                        0,
                        y as c_int,
                        size.width as c_int,
                        size.height as c_int,
                    );
                };
                emit(*node, Event::FrameChanged(size));
            }
        });
    });
}

/// A presented cover's place in a window content `area` large: the whole area, or the bottom
/// half for a half sheet (docs/cover.md). Answers the y offset and the cover's own size.
fn cover_frame(cover: *mut c_void, area: Size) -> (f64, Size) {
    if COVER_HALF.with(|s| s.borrow().contains(&(cover as usize))) {
        let h = (area.height * 0.5).floor();
        (area.height - h, Size::new(area.width, h))
    } else {
        (0.0, area)
    }
}

extern "C" fn nav_menu_changed(id: u64, row: std::os::raw::c_int) {
    // -1 = cleared (programmatic unselect fires nothing thanks to blockSignals; a clear
    // reaching here means the widget emptied — ignore). Deferred: selecting a sidebar item
    // rebuilds the detail, which must not run inside the list's own event dispatch.
    ffi_guard::contain((), || {
        if row >= 0 {
            emit_deferred(NodeId(id), Event::SelectionChanged(i64::from(row)));
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
        .map_or_default(|p| p.to_string_lossy().into_owned())
}

/// Build the nav list's per-row context menus (docs/menus.md) and hand them to the shim:
/// one `QMenu` per row with entries (null where a row has none), the same
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

/// The nav menu's section headings as a U+001F-joined list parallel to its rows (empty entry =
/// the row continues the current group), with how many headings there are.
fn nav_sections_joined(sections: &[Option<String>]) -> (String, usize) {
    let headings = sections.iter().filter(|s| s.is_some()).count();
    let joined = sections
        .iter()
        .map(|s| s.as_deref().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\u{1f}");
    (joined, headings)
}

/// Per-row nav icon tints as a U+001F-joined list of "#rrggbb" entries (empty entry = no
/// row tint, the shim falls back to the palette text color), parallel to the icon list
/// (docs/vectors.md).
/// A color as the `"#aarrggbb"` the shim's `QColor` parses. The alpha is kept: the shim's
/// `SourceIn` fill multiplies it into the glyph's own coverage, as every other toolkit's tint does.
fn hex_argb(c: day_spec::Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}{:02x}",
        (c.a * 255.0).round() as u8,
        (c.r * 255.0) as u8,
        (c.g * 255.0) as u8,
        (c.b * 255.0) as u8
    )
}

/// The image shim's tint channel (docs/vectors.md): a `#aarrggbb`, `currentColor` for a
/// template glyph that follows the palette, or empty for the authored colors.
fn image_tint(tint: Option<day_spec::Color>, template: bool) -> String {
    match tint {
        Some(c) => hex_argb(c),
        None if template => "currentColor".to_owned(),
        None => String::new(),
    }
}

/// Whether a container asks for what only the painted surface gives (docs/surfaces.md): a
/// gradient, a border, a shadow, or rounding on some corners only. A plain color with all four
/// corners rounded stays on the stylesheet path it has always taken.
fn is_styled_surface(p: &ContainerProps) -> bool {
    matches!(
        p.fill,
        Some(day_spec::Paint::Linear(_) | day_spec::Paint::Radial(_) | day_spec::Paint::Conic(_))
    ) || p.border.is_some()
        || p.shadow.is_some()
        || (p.corner_radius > 0.0 && p.corners != day_spec::Corners::ALL)
}

/// The color of a solid fill in the current appearance; `None` for no fill or a gradient.
fn solid_fill(fill: Option<&day_spec::Paint>) -> Option<day_spec::Color> {
    match fill {
        Some(day_spec::Paint::Solid(ink)) => Some(ink.resolved()),
        _ => None,
    }
}

/// A styled surface as the shim's `day_qt_widget_set_styled_surface` takes it: the 24-number
/// spec, and the gradient's stops as (offset, r, g, b, a) groups, every ink resolved against
/// the current appearance. The gradient kinds are the shim's: 1 linear (start and end unit
/// points), 2 radial (center and radius, unit space), 3 conic (center and start angle in
/// degrees, clockwise from +x).
fn surface_spec(p: &ContainerProps) -> ([f64; 24], Vec<f64>) {
    use day_spec::Paint;
    let rgba = |c: day_spec::Color| [c.r, c.g, c.b, c.a];
    let mut spec = [0.0; 24];
    spec[0..4].copy_from_slice(&rgba(
        solid_fill(p.fill.as_ref()).unwrap_or(day_spec::Color::CLEAR),
    ));
    spec[4] = p.corner_radius;
    spec[5] = f64::from(p.corners.bits());
    spec[6] = f64::from(u8::from(p.clips));
    if let Some(border) = p.border {
        spec[7..11].copy_from_slice(&rgba(border.color.resolved()));
        spec[11] = border.width;
    }
    if let Some(shadow) = p.shadow {
        spec[12..16].copy_from_slice(&rgba(shadow.color.resolved()));
        spec[16] = shadow.blur;
        spec[17] = shadow.dx;
        spec[18] = shadow.dy;
    }
    let gradient = match &p.fill {
        Some(Paint::Linear(g)) => Some((1.0, [g.start.x, g.start.y, g.end.x, g.end.y], &g.stops)),
        Some(Paint::Radial(g)) => Some((2.0, [g.center.x, g.center.y, g.radius, 0.0], &g.stops)),
        Some(Paint::Conic(g)) => Some((
            3.0,
            [g.center.x, g.center.y, g.start.degrees(), 0.0],
            &g.stops,
        )),
        Some(Paint::Solid(_)) | None => None,
    };
    let mut stops = Vec::new();
    if let Some((kind, numbers, ink_stops)) = gradient {
        spec[19] = kind;
        spec[20..24].copy_from_slice(&numbers);
        for (offset, color) in day_spec::resolved_stops(ink_stops) {
            stops.push(offset);
            stops.extend_from_slice(&rgba(color));
        }
    }
    (spec, stops)
}

/// Whether any ink of a container's surface depends on the appearance.
fn surface_is_dynamic(p: &ContainerProps) -> bool {
    p.fill.as_ref().is_some_and(day_spec::Paint::is_dynamic)
        || p.border.is_some_and(|b| b.color.is_dynamic())
        || p.shadow.is_some_and(|s| s.color.is_dynamic())
}

/// Apply every styling field of a container (docs/surfaces.md), resolved against the current
/// appearance: at realize (`fresh`), for a reactive surface (`ContainerPatch::Surface`), and
/// again after an appearance switch. A role is the realize-time one; its surface follows the
/// palette by itself and only its radius changes later. A container that once painted its
/// surface keeps painting it, so a border going away leaves no stale stylesheet behind.
///
/// # Safety
/// `w` is a live container widget Day realized.
unsafe fn apply_surface(w: *mut c_void, p: &ContainerProps, fresh: bool) {
    // SAFETY: the caller's guarantee; every call reads or writes only `w` and the numbers
    // passed, which outlive the calls.
    unsafe {
        match p.role {
            Some(day_spec::SurfaceRole::SectionCard) => {
                // A translucent neutral over the window color: subtle in any palette, and it
                // follows the platform / forced (DAY_THEME) color scheme.
                ffi::day_qt_widget_set_section_card(w, p.corner_radius);
            }
            Some(day_spec::SurfaceRole::Window) => {
                ffi::day_qt_widget_set_window_surface(w, p.corner_radius, c_int::from(p.clips));
            }
            _ if is_styled_surface(p) || ffi::day_qt_widget_has_styled_surface(w) != 0 => {
                // A gradient, a border, a shadow or a choice of corners: the container paints
                // these itself, since a stylesheet cannot follow the frame or cast a shadow.
                let (spec, stops) = surface_spec(p);
                ffi::day_qt_widget_set_styled_surface(
                    w,
                    spec.as_ptr(),
                    stops.as_ptr(),
                    (stops.len() / 5) as c_int,
                );
            }
            _ if !fresh || p.fill.is_some() || p.corner_radius > 0.0 || p.clips => {
                let bg = solid_fill(p.fill.as_ref()).unwrap_or(day_spec::Color::CLEAR);
                ffi::day_qt_widget_set_surface(
                    w,
                    bg.r,
                    bg.g,
                    bg.b,
                    bg.a,
                    p.corner_radius,
                    c_int::from(p.clips),
                );
            }
            _ => {}
        }
    }
}

// ---- inks that follow the appearance (docs/color.md "How a toolkit follows the appearance") ----

/// The appearance-dependent inks one widget shows, kept so that an appearance switch can apply
/// each again, resolved against the new palette. Qt takes concrete colors only (a stylesheet, a
/// painter, rich text, a recolored glyph), and Day sends no patch on a switch: an adaptive ink is
/// the same value before and after it. Only dynamic inks are kept (`Ink::is_dynamic`); a
/// container's surface lives in `SURFACES` instead.
#[derive(Clone, Default)]
struct Inks {
    label_color: Option<day_spec::Ink>,
    label_runs: Option<(String, Vec<day_spec::TextRun>)>,
    button: Option<day_spec::props::ButtonStyleSpec>,
    image_tint: Option<day_spec::Ink>,
    nav: Option<NavRows>,
    cover: Option<day_spec::Ink>,
}

impl Inks {
    fn is_empty(&self) -> bool {
        self.label_color.is_none()
            && self.label_runs.is_none()
            && self.button.is_none()
            && self.image_tint.is_none()
            && self.nav.is_none()
            && self.cover.is_none()
    }
}

/// A sidebar's rows as the shim's row calls take them, kept while a row's tint depends on the
/// appearance.
#[derive(Clone, Default)]
struct NavRows {
    items: Vec<String>,
    icons: Vec<Option<String>>,
    tints: Vec<Option<day_spec::Ink>>,
    badge_icons: Vec<Option<String>>,
    badge_tints: Vec<Option<day_spec::Ink>>,
    badges: Vec<Option<String>>,
}

impl NavRows {
    fn is_dynamic(&self) -> bool {
        self.tints
            .iter()
            .chain(&self.badge_tints)
            .any(|t| t.is_some_and(day_spec::Ink::is_dynamic))
    }

    /// The unit-separator joined lists the shim's row calls take: titles, icon files, tints,
    /// badge icon files, badge tints, counts.
    fn joined(&self) -> [CString; 6] {
        let files = |icons: &[Option<String>]| {
            icons
                .iter()
                .map(|ic| ic.as_deref().map_or_default(icon_file_path))
                .collect::<Vec<_>>()
                .join("\u{1f}")
        };
        [
            cstr(&self.items.join("\u{1f}")),
            cstr(&files(&self.icons)),
            cstr(&nav_tints_joined(&self.tints)),
            cstr(&files(&self.badge_icons)),
            cstr(&nav_tints_joined(&self.badge_tints)),
            cstr(&nav_counts_joined(&self.badges)),
        ]
    }
}

thread_local! {
    /// Appearance-dependent inks by widget pointer (`Inks`); swept with the widget.
    static INKS: SideTable<Inks> = SideTable::new();
    /// The surface each styled container shows now, by widget pointer: a fill patch changes
    /// one field of it, a surface patch replaces it, an appearance switch applies it again.
    static SURFACES: SideTable<ContainerProps> = SideTable::new();
}

/// Change the inks kept for `w`; an entry left with nothing is dropped.
fn remember_inks(w: *mut c_void, f: impl FnOnce(&mut Inks)) {
    INKS.with(|t| {
        let mut inks = t.take(w as usize).unwrap_or_default();
        f(&mut inks);
        if !inks.is_empty() {
            t.insert(w as usize, inks);
        }
    });
}

/// A dynamic ink, or `None` for a fixed one (which never needs applying again).
fn dynamic(ink: Option<day_spec::Ink>) -> Option<day_spec::Ink> {
    ink.filter(|i| i.is_dynamic())
}

/// A label's text color in the current appearance (`None`: the palette's text color).
///
/// # Safety
/// `w` is a live label Day realized.
unsafe fn apply_label_color(w: *mut c_void, color: Option<day_spec::Ink>) {
    let c = color.map(day_spec::Ink::resolved);
    let (r, g, b, a) = c.map_or((0.0, 0.0, 0.0, 0.0), |c| (c.r, c.g, c.b, c.a));
    // SAFETY: the caller's guarantee; the shim reads only the numbers.
    unsafe { ffi::day_qt_label_set_color(w, r, g, b, a, c_int::from(c.is_some())) };
    remember_inks(w, |i| i.label_color = dynamic(color));
}

/// A label's runs, kept for an appearance switch when one of their inks depends on it.
fn apply_label_runs(w: *mut c_void, text: &str, runs: &[day_spec::TextRun]) {
    set_label_runs(w, text, runs);
    let dynamic_runs = runs.iter().any(|r| {
        r.color.is_some_and(day_spec::Ink::is_dynamic)
            || r.background.is_some_and(day_spec::Ink::is_dynamic)
            || r.underline
                .and_then(|d| d.color)
                .is_some_and(day_spec::Ink::is_dynamic)
            || r.strikethrough
                .and_then(|d| d.color)
                .is_some_and(day_spec::Ink::is_dynamic)
    });
    remember_inks(w, |i| {
        i.label_runs = dynamic_runs.then(|| (text.to_owned(), runs.to_vec()));
    });
}

/// A button's style, kept for an appearance switch when its tint depends on it.
fn apply_button_style_ink(w: *mut c_void, style: day_spec::props::ButtonStyleSpec) {
    apply_button_style(w, style);
    let tinted = matches!(style, day_spec::props::ButtonStyleSpec::Tinted(ink) if ink.is_dynamic());
    remember_inks(w, |i| i.button = tinted.then_some(style));
}

/// A sidebar's rows set or updated in place: `rows` the Day rows to update, or `None` to replace
/// every row (the sections then come too).
///
/// # Safety
/// `w` is a live sidebar list Day realized.
unsafe fn apply_nav_rows(
    w: *mut c_void,
    rows: &NavRows,
    update: Option<&[usize]>,
    sections: &[Option<String>],
) -> usize {
    let [joined, icons, tints, badge_icons, badge_tints, counts] = rows.joined();
    let mut headings = 0;
    // SAFETY: the caller's guarantee; every string outlives the call.
    unsafe {
        if let Some(update) = update {
            let rows_joined = update
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join(",");
            ffi::day_qt_navlist_update_rows(
                w,
                cstr(&rows_joined).as_ptr(),
                joined.as_ptr(),
                icons.as_ptr(),
                tints.as_ptr(),
                badge_icons.as_ptr(),
                badge_tints.as_ptr(),
                counts.as_ptr(),
            );
        } else {
            let (sections_joined, n) = nav_sections_joined(sections);
            headings = n;
            ffi::day_qt_navlist_set_items(
                w,
                joined.as_ptr(),
                icons.as_ptr(),
                tints.as_ptr(),
                badge_icons.as_ptr(),
                badge_tints.as_ptr(),
                counts.as_ptr(),
                cstr(&sections_joined).as_ptr(),
            );
        }
    }
    remember_inks(w, |i| i.nav = rows.is_dynamic().then(|| rows.clone()));
    headings
}

/// The appearance changed: Day reads the new palette, then every ink Qt was handed as a
/// concrete color is applied again, resolved against it (docs/color.md "How a toolkit follows
/// the appearance"). Palette-driven parts (the stock controls, a template glyph, a section card)
/// have already followed. Canvases re-record on Day's side.
fn refresh_appearance() {
    day_core::note_appearance_changed();
    reapply_inks();
}

/// Every kept ink applied again, resolved against `Palette::current()`.
fn reapply_inks() {
    let mut surfaces = Vec::new();
    SURFACES.with(|t| {
        t.for_each(|w, p| {
            if surface_is_dynamic(p) {
                surfaces.push((w, p.clone()));
            }
        });
    });
    for (w, p) in surfaces {
        // SAFETY: an entry lives exactly as long as its widget (swept on release).
        unsafe { apply_surface(w as *mut c_void, &p, false) };
    }
    let mut inks = Vec::new();
    INKS.with(|t| t.for_each(|w, i| inks.push((w, i.clone()))));
    for (w, i) in inks {
        let w = w as *mut c_void;
        // SAFETY: as above; each call re-applies to the widget the entry was made for.
        unsafe {
            if let Some(c) = i.label_color {
                apply_label_color(w, Some(c));
            }
            if let Some((text, runs)) = &i.label_runs {
                set_label_runs(w, text, runs);
            }
            if let Some(style) = i.button {
                apply_button_style(w, style);
            }
            if let Some(tint) = i.image_tint {
                let tint = image_tint(Some(tint.resolved()), false);
                ffi::day_qt_image_set_tint(w, cstr(&tint).as_ptr());
            }
            if let Some(rows) = &i.nav {
                let all: Vec<usize> = (0..rows.items.len()).collect();
                apply_nav_rows(w, rows, Some(&all), &[]);
            }
            if let Some(bg) = i.cover {
                let bg = bg.resolved();
                ffi::day_qt_widget_set_bg(w, bg.r, bg.g, bg.b, bg.a, std::ptr::null());
            }
        }
    }
}

/// The shim's appearance callback (`day_qt_on_appearance`).
extern "C" fn on_appearance() {
    ffi_guard::contain((), refresh_appearance);
}

/// Per-row counts for the shim, unit-separator joined (empty = no count).
fn nav_counts_joined(badges: &[Option<String>]) -> String {
    badges
        .iter()
        .map(|b| b.as_deref().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\u{1f}")
}

/// Per-row tints for the shim, each resolved against the current appearance.
fn nav_tints_joined(tints: &[Option<day_spec::Ink>]) -> String {
    tints
        .iter()
        .map(|t| match t.map(day_spec::Ink::resolved) {
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
#[must_use]
pub const fn lifecycle_supported(phase: day_spec::Lifecycle) -> bool {
    phase.is_universal()
}

/// Phase codes (from the Qt shim's `QApplication` signals) → day lifecycle events.
extern "C" fn on_lifecycle(code: c_int) {
    use day_spec::Lifecycle::{DidBecomeActive, WillResignActive, WillTerminate};
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
    use day_spec::MenuRole::{
        About, CheckForUpdates, CloseWindow, Copy, Cut, Delete, Fullscreen, Minimize, NewWindow,
        Paste, Preferences, Quit, Redo, SelectAll, Undo,
    };
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
        CheckForUpdates => "Check for Updates…",
    }
}

/// Default portable shortcut for a role (Qt maps `Ctrl` → ⌘ on macOS).
fn qt_role_shortcut(role: day_spec::MenuRole) -> Option<&'static str> {
    use day_spec::MenuRole::{
        About, CheckForUpdates, CloseWindow, Copy, Cut, Delete, Fullscreen, Minimize, NewWindow,
        Paste, Preferences, Quit, Redo, SelectAll, Undo,
    };
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
        Delete | About | Fullscreen | CheckForUpdates => return None,
    })
}

/// Walk the day-neutral menu tree, issuing flat builder calls against a `QMenu` pointer.
pub(crate) fn build_qt_menu(menu: *mut c_void, items: &[day_spec::MenuItem]) {
    for item in items {
        match item {
            day_spec::MenuItem::Separator => unsafe { ffi::day_qt_menu_add_separator(menu) },
            day_spec::MenuItem::Submenu { label, items, .. } => {
                let sub = unsafe { ffi::day_qt_menu_add_submenu(menu, cstr(label).as_ptr()) };
                build_qt_menu(sub, items);
            }
            day_spec::MenuItem::Action {
                action,
                label,
                shortcut,
                enabled,
                checked,
                role,
                icon,
                ..
            } => {
                // A nonzero id always wins (the appkit precedence, docs/menus.md): the item
                // dispatches the day action, with the role only supplying label/shortcut
                // defaults — the auto Preferences item and `MenuRole::NewWindow` arrive
                // this way. Routing them through the role-only path dropped the dispatch
                // id, leaving visible-but-dead items (the macos-qt bug).
                if *action != 0 {
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
                            *action,
                            cstr(&sc).as_ptr(),
                            c_int::from(*enabled),
                            cstr(&ic).as_ptr(),
                            ic_fallback,
                            checked.map_or(-1, c_int::from),
                        );
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
                        );
                    };
                }
            }
        }
    }
}

/// Warn once per kind that this backend has no registered renderer for `kind`, before falling back to
/// a visible placeholder. A missing renderer usually means the piece's `qt` feature wasn't enabled
/// (Tier A.2 derives it automatically under `day build`). Deduped per kind so a placeholder rendered
/// every frame doesn't spam the log.
fn warn_missing_renderer(kind: PieceKind) {
    day_spec::placeholder::report(kind, "qt");
}

/// The visible placeholder a realize arm degrades to when its props payload has the wrong type
/// (`props_of` has already reported the mismatch) — the same label the missing-renderer arm shows.
pub(crate) fn placeholder_handle(kind: PieceKind) -> QtHandle {
    QtHandle(unsafe { ffi::day_qt_label_new(cstr(&format!("⟨{kind}⟩")).as_ptr(), 1) })
}

/// `Qt::CursorShape` for a [`Cursor`], or -1 to unset (docs/cursor.md).
fn qt_cursor_shape(c: &Cursor) -> c_int {
    match c {
        Cursor::Default => -1,
        Cursor::Pointer => 13,                    // PointingHandCursor
        Cursor::Text | Cursor::VerticalText => 4, // IBeamCursor
        Cursor::Crosshair | Cursor::Cell => 2,    // CrossCursor
        Cursor::Move => 9,                        // SizeAllCursor
        Cursor::Grab => 17,                       // OpenHandCursor
        Cursor::Grabbing => 18,                   // ClosedHandCursor
        Cursor::NotAllowed => 14,                 // ForbiddenCursor
        Cursor::Wait => 3,                        // WaitCursor
        Cursor::Progress => 16,                   // BusyCursor
        Cursor::Help => 15,                       // WhatsThisCursor
        Cursor::ContextMenu | Cursor::ZoomIn | Cursor::ZoomOut => 0, // ArrowCursor
        Cursor::Copy => 19,                       // DragCopyCursor
        Cursor::Alias => 21,                      // DragLinkCursor
        Cursor::None => 10,                       // BlankCursor
        Cursor::NsResize => 5,                    // SizeVerCursor
        Cursor::EwResize => 6,                    // SizeHorCursor
        Cursor::NeswResize => 7,                  // SizeBDiagCursor
        Cursor::NwseResize => 8,                  // SizeFDiagCursor
        Cursor::ColResize => 12,                  // SplitHCursor
        Cursor::RowResize => 11,                  // SplitVCursor
        Cursor::Native(name) => match &**name {
            "UpArrowCursor" => 1,
            "BlankCursor" => 10,
            "SplitVCursor" => 11,
            "SplitHCursor" => 12,
            "WhatsThisCursor" => 15,
            "BusyCursor" => 16,
            "DragMoveCursor" => 20,
            _ => -1,
        },
    }
}

impl Toolkit for Qt {
    type Handle = QtHandle;

    fn set_presented(&mut self, handle: &Self::Handle, visible: bool) {
        unsafe {
            day_toolkit_qt_sys::day_qt_set_visible(handle.0, i32::from(visible));
        }
    }

    #[expect(
        clippy::match_same_arms,
        reason = "one arm per capability, each documented with the Qt mechanism behind it"
    )]
    fn capability(&self, cap: Cap) -> Support {
        match cap {
            // A reordering sidebar is Day's own list standing in for the native one: the rows
            // drag natively, the sidebar does not (docs/navigation.md).
            Cap::NavReorder => Support::Emulated,
            // `set_appearance` is `QStyleHints::setColorScheme`, which Qt has from 6.8. Every
            // ink follows a system switch on any version (`refresh_appearance`).
            Cap::Appearance => {
                // SAFETY: a compile-time answer from the shim; no arguments.
                if unsafe { ffi::day_qt_appearance_settable() } != 0 {
                    Support::Native
                } else {
                    Support::Unsupported
                }
            }
            Cap::SearchField => Support::Native,
            Cap::ComboBox => Support::Native,
            Cap::Stepper => Support::Native,
            Cap::DateTimePicker => Support::Native,
            Cap::ColorPicker => Support::Native,
            Cap::PullRefresh => Support::Emulated,
            Cap::Pager => Support::Emulated,
            Cap::PullRefreshGesture => Support::Unsupported,
            // `QWidget::setCursor` per widget; several CSS shapes take their nearest Qt shape
            // (docs/cursor.md).
            Cap::Cursor => Support::Emulated,
            // OS window tabs exist on macOS alone, through `day_platform_macos` (docs/windows.md).
            Cap::WindowTabbing if cfg!(target_os = "macos") => Support::Native,
            // `QFontDatabase::families()` + `styles()` (docs/fonts.md).
            Cap::FontList => Support::Native,
            // Qt's animation framework plays the whole AnimSpec contract on opacity, transform,
            // frame and a container's fill (a QSequentialAnimationGroup per widget and channel:
            // QPauseAnimation for the delay, looped QVariantAnimations keyed with the curve's
            // samples), and `read_displayed` reads the values in flight (docs/tween.md).
            Cap::Animation => Support::Native,
            // `QImage::loadFromData` reads every format Qt's image plugins handle, and
            // `QImage::save` writes back PNG/JPEG/TIFF/BMP (docs/images.md). `Cap::ImageProperties`
            // is deliberately NOT here: Qt exposes no metadata reader on this path, and an empty
            // struct would read as "this file records nothing" rather than "nobody looked".
            Cap::ImageDecode | Cap::ImageEncode => Support::Native,
            // The canvas widget's painter replays into a QImage and onto QPdfWriter pages
            // (QtGui, no extra module), so both exports are native (docs/canvas.md "Exporting").
            Cap::CanvasBitmap | Cap::CanvasPdf => Support::Native,
            // QListView has no sticky group rows: header rows are unselectable, disabled model
            // rows sized to their content, and the list pins a copy of the current section's
            // header over the top of its viewport (docs/list.md "Sections").
            Cap::ListSections => Support::Emulated,
            // `ScrollSnap` comes to rest by a glide to the nearest stop once the user's
            // scrolling has been quiet for 150 ms: Qt's scroll areas have resting points only
            // under a QScroller, which Day's do not grab (docs/scroll.md).
            Cap::ScrollSnap => Support::Emulated,
            // `Cap::ScrollPhases` stays Unsupported (the default arm): with no QScroller, Qt
            // tells no drag/coast/idle moments, so the scroll piece derives them from the
            // offset reports. `Cap::ScrollZoom` too (no zooming scroll area), and
            // `Cap::ScrollKeyboardDismiss` (a desktop toolkit with no software keyboard).
            // `Cap::PagerProgress` as well: the pager is the composed one (`Cap::Pager`
            // Emulated), whose progress the piece keeps at rest.
            //
            // The viewport report: the vertical bar's value and range and `indexAt` at the
            // viewport's edges, after every scroll, resize, splice and relayout (docs/list.md
            // "Visible rows"). Row scrolls aimed per pixel on the vertical bar, every
            // alignment, gliding with a QPropertyAnimation (`Toolkit::list_scroll`). The
            // offset: the bars' `valueChanged`, one report per frame plus the resting one.
            Cap::ListViewport | Cap::ListScrollAlign | Cap::ScrollOffset => Support::Native,
            // The scrolls the reader starts: the vertical bar's `actionTriggered` (only ever the
            // user's: a wheel step, which the view hands to the bar, a thumb drag, a page click)
            // when it moves the bar, and a key whose cursor move scrolled the list.
            Cap::ListReaderScroll => Support::Native,
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
            // The same QSplitter's middle pane, with a real draggable divider on each side;
            // it persists through every presentation, the AppKit shape (docs/navigation.md).
            | Cap::NavContentList
            | Cap::Dialogs
            | Cap::FileDialogs
            // QFileDialog in `Directory` mode (docs/files.md).
            | Cap::FolderDialog
            | Cap::TextEditable
            | Cap::TextSelectable
            // Qt's own QDrag pipeline: grabbed-cell pixmap, insertion line, no-drop cursor.
            | Cap::DragDrop | Cap::DragExternalImport | Cap::DragExternalExport | Cap::DragMultipleItems | Cap::DragFileReferences | Cap::ListReorder
            // A QListView with Day's cells recycled through a bounded pool as rows scroll in
            // and out (docs/list.md): the view scrolls, Day fills the cell it is handed.
            | Cap::ListRecycling
            // Real DayWindows on the shared QApplication (docs/windows.md).
            | Cap::MultiWindow
            // showMinimized / showMaximized / showFullScreen / showNormal, reported back from
            // the DayWindow's WindowStateChange events (docs/windows.md "Window properties").
            // Wayland never reports minimized; the shim's `assumedMinimized` stands in for it.
            // `Cap::ContentProtection` stays Unsupported: no Linux display server lets a client
            // opt out of capture.
            | Cap::WindowStates
            | Cap::WindowFullscreen
            // `Qt::FramelessWindowHint`, resizable from its edges through
            // `QWindow::startSystemResize` (docs/window-chrome.md).
            | Cap::FramelessWindow
            // `QWindow::startSystemMove` on a press of the region's own background.
            | Cap::DragRegion
            // `resize`, `setMinimumSize`/`setMaximumSize`, `setFixedSize`.
            | Cap::WindowGeometry
            // `Qt::WindowStaysOnTopHint`, and `Qt::Tool` for no taskbar entry. Every workspace
            // has no Qt API and is ignored.
            | Cap::WindowLevel
            // `QApplication::alert`.
            | Cap::RequestAttention
            // `QGuiApplication::screens()`, with each screen's available geometry.
            | Cap::Monitors
            // A QTabWidget, which is Qt's own one-of-N container and already the shape Day
            // wants: it owns its pages and shows one at a time (docs/navigation.md).
            //
            // `Cap::NavTabsAdaptive` is deliberately not here: a Qt app may PIN a tab bar, but a
            // narrowing window hides its sidebar and pushes rather than growing one.
            | Cap::NavTabs
            // A real QToolBar under the menu bar (docs/toolbars.md).
            | Cap::AppMenu
            | Cap::Toolbar
            // The bar holds a native search field (docs/search.md).
            | Cap::ToolbarSearch
            // The nav QSplitter mirrored: panel pane trailing, divider draggable
            // (docs/inspector.md — not a QDockWidget; DayWindow is no QMainWindow).
            | Cap::Inspector
            // A QSplitter of two plain panes, either orientation (docs/split.md).
            | Cap::Split => Support::Native,
            // A focusable view joins Qt's tab chain (StrongFocus), focuses on a press before
            // its recognizers hear it, reports focus both ways, takes its keys with phases, and
            // shows the style's focus indicator when the keyboard brought focus
            // (docs/focus.md). Repeats are `QKeyEvent::isAutoRepeat`, releases `KeyRelease`
            // (docs/menus.md "Keys").
            Cap::Focusable | Cap::KeyPhases => Support::Native,
            // Day's own routing of the press, since Qt has no per-widget hit test to override
            // (docs/gestures.md "Hit slop").
            Cap::HitSlop => Support::Emulated,
            // QLabel has no selection signal; the label compares `selectionStart` and
            // `selectedText` after each mouse, key and focus event that can move it and reports
            // a change (docs/text.md "Hearing about the selection").
            Cap::LabelSelection => Support::Native,
            // A topmost child of the window content — not a system modal (docs/cover.md).
            Cap::Cover => Support::Emulated,
            // Native QTreeWidget disclosure and selection, with Day-owned row widgets.
            // Native tree drag-to-move is not wired yet.
            Cap::Tree | Cap::TreeSections => Support::Native,
            // Derived from QFontMetrics — Qt publishes no baseline of its own
            // (docs/baseline.md).
            Cap::BaselineAlignment => Support::Emulated,
            // `QAccessibleAnnouncementEvent` (docs/accessibility.md) is Qt 6.8; the shim
            // answers for the Qt it is built against.
            Cap::Announce => {
                // SAFETY: a constant read; no arguments and no pointers cross the boundary.
                if unsafe { ffi::day_qt_can_announce() } != 0 {
                    Support::Native
                } else {
                    Support::Unsupported
                }
            }
            // Qt publishes no system reduce-motion read (docs/accessibility.md): QStyleHints and
            // QGuiApplication carry nothing, `QStyle::SH_Widget_Animation_Duration` is the active
            // style's own constant rather than the user's setting, and
            // `QApplication::isEffectEnabled` answers the menu/tooltip effect switches. So
            // `Toolkit::reduce_motion` keeps its default, and `DAY_REDUCE_MOTION=1` is the one
            // way on.
            Cap::ReduceMotion => Support::Unsupported,
            // `move()` places a window on X11, Windows and macOS; Wayland lets no client place
            // its windows (docs/windows.md).
            Cap::WindowPosition => {
                // SAFETY: a read of the platform name.
                if unsafe { ffi::day_qt_is_wayland() } != 0 {
                    Support::Unsupported
                } else {
                    Support::Native
                }
            }
            // The Unity LauncherEntry signal, which docks may or may not read.
            // Written as a plain arm so the coverage matrix reads it as decided per platform (a
            // guard arm reads as unsupported).
            Cap::AppProgress | Cap::AppBadgeCount => {
                if cfg!(target_os = "linux") {
                    Support::Emulated
                } else {
                    Support::Unsupported
                }
            }
            // `WA_TranslucentBackground`. On X11 it needs a compositing window manager.
            Cap::TransparentWindow => Support::Native,
            // A QSystemTrayIcon host: a StatusNotifierWatcher or an XEmbed tray on Linux.
            Cap::StatusItem => {
                // SAFETY: a query of the running QApplication.
                if unsafe { ffi::day_qt_tray_available() } != 0 {
                    Support::Native
                } else {
                    Support::Unsupported
                }
            }
            // Deliberately Unsupported (the default arm): `OverlayTitleBar` (Qt draws no title
            // bar of its own to run content under; the window keeps its standard frame),
            // `WindowMaterial`, `WindowAppearance` (palette and scheme are app-wide),
            // `ContentProtection`, `DynamicShortcuts` (a .desktop file's actions are fixed).
            _ => Support::Unsupported,
        }
    }

    fn realize(&mut self, kind: PieceKind, props: &dyn std::any::Any, id: NodeId) -> QtHandle {
        unsafe {
            match Builtin::from_key(kind) {
                Some(Builtin::Inspector) => {
                    let (visible, width, leading) =
                        props
                            .downcast_ref::<InspectorProps>()
                            .map_or((false, 280.0, false), |p| {
                                (
                                    p.visible,
                                    p.width,
                                    p.edge == day_spec::props::PaneEdge::Leading,
                                )
                            });
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
                Some(Builtin::Split) => {
                    let (axis, fraction, min_pane) = props
                        .downcast_ref::<SplitProps>()
                        .map_or((day_spec::props::SplitAxis::Horizontal, 0.5, 80.0), |p| {
                            (p.axis, p.fraction, p.min_pane)
                        });
                    let vertical = axis == day_spec::props::SplitAxis::Vertical;
                    let host = ffi::day_qt_split_new(c_int::from(vertical), min_pane);
                    let first_pane = ffi::day_qt_splitter_pane(host, 0);
                    let second_pane = ffi::day_qt_splitter_pane(host, 1);
                    ffi::day_qt_splitter_on_moved(host, split_moved);
                    ffi::day_qt_splitter_on_resized(host, split_resized);
                    SPLIT_STATE.with(|m| {
                        m.borrow_mut().insert(
                            host as usize,
                            SplitState {
                                first_pane,
                                second_pane,
                                node: id,
                                panes: Vec::new(),
                                share: fraction.clamp(0.0, 1.0),
                                min_pane,
                                placed: false,
                                suppress: false,
                            },
                        )
                    });
                    QtHandle(host)
                }
                Some(Builtin::SplitPane) => {
                    let w = ffi::day_qt_container_new();
                    let second = props
                        .downcast_ref::<SplitPaneProps>()
                        .is_some_and(|p| p.second);
                    SPLIT_PANE_IDS.with(|m| m.borrow_mut().insert(w as usize, (id, second)));
                    QtHandle(w)
                }
                Some(Builtin::InspectorPane) => {
                    let w = ffi::day_qt_container_new();
                    let panel = props
                        .downcast_ref::<InspectorPaneProps>()
                        .is_some_and(|p| p.panel);
                    INSPECTOR_PANE_IDS.with(|m| m.borrow_mut().insert(w as usize, (id, panel)));
                    QtHandle(w)
                }
                Some(Builtin::Container) => {
                    let w = ffi::day_qt_container_new();
                    if let Some(p) = props.downcast_ref::<ContainerProps>() {
                        apply_surface(w, p, true);
                        if *p != ContainerProps::default() {
                            SURFACES.with(|t| t.insert(w as usize, p.clone()));
                        }
                    }
                    passive(w)
                }
                Some(Builtin::Nav) => {
                    let nav_props = props.downcast_ref::<NavProps>();
                    // Where the rows are the CHROME, the host is a QTabWidget — Qt's own
                    // one-of-N container, and already the shape Day wants here: it owns its page
                    // widgets and shows one at a time, so the pages stay resident and
                    // `NavPatch::Select` drives the bar instead of push/pop.
                    //
                    // A desktop only reaches this by PINNING `NavStyle::Tabs`;
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
                                    keys: Vec::new(),
                                },
                            )
                        });
                        if let Some(config) = nav_props.and_then(|p| p.documents.as_ref()) {
                            ffi::day_qt_tabs_documents(
                                w,
                                id.0,
                                cstr(&config.new_label).as_ptr(),
                                cstr(&config.close_label).as_ptr(),
                                c_int::from(config.can_add),
                                document_tab_event,
                            );
                        }
                        return QtHandle(w);
                    }
                    let is_split = nav_props.is_none_or(|p| p.presentation.is_split());
                    // The content-list pane (docs/navigation.md): pane 1 of the same splitter,
                    // hidden on a host that declared none — the same shape either way, so the
                    // header and the pane indices never move.
                    let list_width = nav_props.and_then(|p| p.list_width);
                    let list_visible = nav_props.is_none_or(|p| p.list_visible);
                    let host = ffi::day_qt_splitter_new(
                        list_width.unwrap_or(0.0),
                        day_spec::NAV_SIDEBAR_MIN_W,
                        day_spec::NAV_LIST_MIN_W,
                    );
                    let sidebar_pane = ffi::day_qt_splitter_pane(host, 0);
                    let list_pane = ffi::day_qt_splitter_pane(host, 1);
                    let mut detail_pane = ffi::day_qt_splitter_pane(host, 2);
                    // Collapsed for a destination that spans the whole detail area
                    // (`content_list_for`); `NavPatch::ListVisible` brings it back.
                    if list_width.is_some() && !list_visible {
                        ffi::day_qt_set_visible(list_pane, 0);
                    }
                    ffi::day_qt_splitter_on_moved(host, nav_splitter_moved);
                    ffi::day_qt_splitter_on_resized(host, nav_splitter_moved);
                    // The back header goes in for both presentations, hidden until a stack
                    // actually pushes. Installing it lazily would mean restructuring the detail
                    // pane's layout under live, absolutely-positioned pages the first time a
                    // window narrowed (docs/size-classes.md) — this way the widget tree is the
                    // same shape either way and a re-present only flips visibility.
                    let pages = ffi::day_qt_nav_header_install(host, id.0, nav_back_clicked);
                    if !pages.is_null() {
                        detail_pane = pages;
                    }
                    // Depth titles for the back header. Recorded in both presentations, so a
                    // window that narrows into a stack already knows what to put in the header
                    // (docs/size-classes.md); the header simply stays hidden while split.
                    let titles = vec![nav_props.map_or_default(|p| p.title.clone())];
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
                                list_pane,
                                list_width,
                                list_page: None,
                                list_shown: list_visible,
                                // Sized at construction when it opened showing; a hidden
                                // pane is placed on its first reveal.
                                list_positioned: list_visible,
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
                    let p = props_of::<CoverProps>(kind, "qt", props);
                    if p.is_some_and(|p| p.half) {
                        COVER_HALF.with(|s| s.borrow_mut().insert(cover.0 as usize));
                    }
                    if p.is_some_and(|p| (p.sheet || p.large) && !p.half) {
                        ffi::day_qt_cover_escape(cover.0, id.0, on_cover_escape);
                    }
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
                        m.borrow_mut().insert(
                            w as usize,
                            (
                                id,
                                p.items.clone(),
                                p.icons.clone(),
                                p.document_keys.clone(),
                            ),
                        )
                    });
                    // Parallel per-row icon file paths (empty entry = no icon): the bundled image
                    // NAME resolves to a loadable file path on the Rust side; the shim loads a
                    // QPixmap, tints it to the palette text color, and setIcon()s it. The
                    // trailing status glyph rides the same resolve-then-tint path as the leading
                    // icon; the shim's delegate paints it at the row's far edge.
                    let rows = NavRows {
                        items: p.items.clone(),
                        icons: p.icons.clone(),
                        tints: p.tints.clone(),
                        badge_icons: p.badge_icons.clone(),
                        badge_tints: p.badge_tints.clone(),
                        badges: p.badges.clone(),
                    };
                    let headings = apply_nav_rows(w, &rows, None, &p.sections);
                    navlist_apply_row_menus(w, &p.menus);
                    ffi::day_qt_navlist_set_selected(w, p.selected.map_or(-1, |i| i as c_int));
                    NAV_MENU_ROWS
                        .with(|m| m.borrow_mut().insert(w as usize, (p.items.len(), headings)));
                    QtHandle(w)
                }
                Some(Builtin::Scroll) => {
                    let p = props
                        .downcast_ref::<day_spec::props::ScrollProps>()
                        .cloned()
                        .unwrap_or_default();
                    let sa = ffi::day_qt_scroll_new(c_int::from(p.horizontal));
                    // The preferences (docs/scroll.md): hidden bars, and the emulated snap.
                    // `bounce` needs nothing: a Qt scroll area stops dead at its edges
                    // (`Some(false)` is how it always behaves) and has no overscroll effect to
                    // turn on. `zoom` and `keyboard_dismiss` are Unsupported; `track_phase` is
                    // left to the piece's emulation (no QScroller, so no native phases).
                    let (snap, interval) = match p.snap {
                        day_spec::props::ScrollSnap::None => (0, 0.0),
                        day_spec::props::ScrollSnap::Page => (1, 0.0),
                        day_spec::props::ScrollSnap::Interval(points) => (2, points),
                    };
                    let hidden = p.indicators == day_spec::props::ScrollIndicators::Hidden;
                    ffi::day_qt_scroll_configure(sa, c_int::from(hidden), snap, interval);
                    if p.track_offset {
                        ffi::day_qt_scroll_track_offset(sa, id.0, on_scroll_offset);
                    }
                    QtHandle(sa)
                }
                Some(Builtin::Label) => {
                    let Some(p) = props_of::<LabelProps>(kind, "qt", props) else {
                        return placeholder_handle(kind);
                    };
                    let w = ffi::day_qt_label_new(cstr(&p.text).as_ptr(), c_int::from(p.wraps));
                    ffi::day_qt_label_set_align(w, label_align(p.align));
                    apply_label_font(w, p.font);
                    apply_custom_family(w, p.font);
                    if p.font.monospace {
                        ffi::day_qt_label_set_monospace(w);
                    }
                    if p.color.is_some() {
                        apply_label_color(w, p.color);
                    }
                    remember_label_pt(w, p.font);
                    apply_paragraph(w, &p.paragraph());
                    apply_label_runs(w, &p.text, &p.runs);
                    if p.reports_selection {
                        ffi::day_qt_label_on_selection(w, id.0, on_text_selected);
                    }
                    // Wired unconditionally: runs can arrive later by patch, and connecting a
                    // signal on a label that never gets a link costs nothing.
                    ffi::day_qt_label_on_link(w, id.0, on_link);
                    passive(w)
                }
                Some(Builtin::Button) => {
                    let Some(p) = props_of::<ButtonProps>(kind, "qt", props) else {
                        return placeholder_handle(kind);
                    };
                    let w = ffi::day_qt_button_new(cstr(&p.title).as_ptr(), id.0, on_press);
                    ffi::day_qt_set_enabled(w, c_int::from(p.enabled));
                    ffi::day_qt_button_set_selected(w, c_int::from(p.selected));
                    apply_button_style_ink(w, p.style);
                    if p.icon.is_some() {
                        apply_button_content(w, &p.title, p.icon.as_ref(), p.icon_only);
                    }
                    ffi::day_qt_enable_focus(w, id.0, on_focus);
                    QtHandle(w)
                }
                Some(Builtin::Toggle) => {
                    let Some(p) = props_of::<ToggleProps>(kind, "qt", props) else {
                        return placeholder_handle(kind);
                    };
                    let w = ffi::day_qt_checkbox_new(c_int::from(p.on), id.0, on_toggle);
                    ffi::day_qt_set_enabled(w, c_int::from(p.enabled));
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
                    ffi::day_qt_lineedit_enable_field_keys(w, id.0, on_field_key);
                    QtHandle(w)
                }
                Some(Builtin::Divider) => {
                    let vertical = day_spec::props_of::<DividerProps>(kind, "qt", props)
                        .is_some_and(|p| p.vertical);
                    passive(ffi::day_qt_separator_new(c_int::from(vertical)))
                }
                Some(Builtin::Progress) => {
                    let Some(p) = props_of::<ProgressProps>(kind, "qt", props) else {
                        return placeholder_handle(kind);
                    };
                    if p.value.is_none() {
                        return crate::controls::spinner::make(self, p, id);
                    }
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
                    passive(w)
                }
                Some(Builtin::Image) => {
                    let Some(p) = props_of::<ImageProps>(kind, "qt", props) else {
                        return placeholder_handle(kind);
                    };
                    // Scaling: 0=fit, 1=fill (crop), 2=stretch.
                    let mode = match p.content_mode {
                        ContentMode::Fit => 0,
                        ContentMode::Fill => 1,
                        ContentMode::Stretch => 2,
                    };
                    let image = match &p.source {
                        // The staged-asset path: only a NAMED source resolves to a file, and only
                        // a file can be tinted — a recolor re-renders the glyph from its source.
                        day_spec::ImageSource::Named(named) => {
                            // Prefer vector geometry; ordinary images keep their native Qt resource path.
                            let res_path = format!(":/day/images/{named}");
                            let path = if let Some(svg) =
                                day_spec::resource::resolve_vector_svg(named)
                            {
                                svg.to_string_lossy().into_owned()
                            } else if ffi::day_qt_resource_exists(cstr(&res_path).as_ptr()) != 0 {
                                res_path
                            } else {
                                // The staged glyph SVG first (docs/vectors.md), the raster cache
                                // after — `icon_file_path`'s rule, so the piece and the icon
                                // channels agree.
                                icon_file_path(named)
                            };
                            // Vector-glyph tint (docs/vectors.md): the same SourceIn recolor the
                            // nav rows use, over a glyph the SVG engine renders at size. A
                            // template with no tint asks for the palette's text color
                            // (`currentColor`), which the shim re-applies on a palette change.
                            let tint = image_tint(p.tint.map(day_spec::Ink::resolved), p.template);
                            passive(ffi::day_qt_image_new(
                                cstr(&path).as_ptr(),
                                mode,
                                cstr(&tint).as_ptr(),
                            ))
                        }
                        // Bytes the app already holds — a download, a picked file, a paste — with
                        // no staged resource behind them (docs/images.md).
                        // An SVG among them takes the tint a staged glyph would (a template
                        // follows the palette); a raster is recolored by an explicit tint and
                        // otherwise draws as authored.
                        day_spec::ImageSource::Bytes(bytes) => {
                            let tint = image_tint(p.tint.map(day_spec::Ink::resolved), p.template);
                            passive(ffi::day_qt_image_new_bytes(
                                bytes.as_ptr(),
                                bytes.len() as c_int,
                                mode,
                                cstr(&tint).as_ptr(),
                            ))
                        }
                        // Already decoded: share the shim's one QImage rather than parsing twice.
                        day_spec::ImageSource::Decoded(id) => {
                            let tint = image_tint(p.tint.map(day_spec::Ink::resolved), false);
                            passive(ffi::day_qt_image_new_bitmap(
                                id.0,
                                mode,
                                cstr(&tint).as_ptr(),
                            ))
                        }
                    };
                    remember_inks(image.0, |i| i.image_tint = dynamic(p.tint));
                    image
                }
                Some(Builtin::Tree) => {
                    let Some(p) = props_of::<TreeProps>(kind, "qt", props) else {
                        return placeholder_handle(kind);
                    };
                    QtHandle(tree::new(id, p))
                }
                Some(Builtin::List) => {
                    let Some(p) = props_of::<ListProps>(kind, "qt", props) else {
                        return placeholder_handle(kind);
                    };
                    let (row_height, automatic) = match p.row_height {
                        RowHeight::Uniform(h) => (h, false),
                        RowHeight::Automatic => (44.0, true),
                    };
                    // A QListView over a count-only model (docs/list.md): selection, the
                    // keyboard walker and the drag-and-drop machinery are the view's; the
                    // guard and the commit of a reorder still run through Day's seam
                    // (`on_list_can_move`, `on_list_move`) so the app's own verdict decides
                    // every drop. Rows are bound as the window moves (`on_list_window`).
                    let separators = match p.separators {
                        None => -1,
                        Some(false) => 0,
                        Some(true) => 1,
                    };
                    let host = ffi::day_qt_list_new(
                        id.0,
                        row_height as c_int,
                        c_int::from(p.selectable),
                        c_int::from(p.multi_select),
                        c_int::from(p.reorderable),
                        separators,
                        on_list_selection,
                        on_list_activate,
                        on_list_can_move,
                        on_list_move,
                        on_list_window,
                    );
                    // A legacy scroll bar appearing or leaving resizes the viewport under
                    // the rows without the host moving: re-lay them to the width they have.
                    ffi::day_qt_list_on_viewport_resized(host, on_list_viewport_resized);
                    // The scrolls the reader starts (docs/list.md "Following the end").
                    ffi::day_qt_list_on_reader_scroll(host, on_list_reader_scroll);
                    // A sectioned list's headers size to their content even in a `Uniform`
                    // list (docs/list.md "Sections"), so its rows are not uniform either.
                    if automatic || p.sections {
                        ffi::day_qt_list_set_automatic(host);
                    }
                    LIST_STATE.with(|m| {
                        m.borrow_mut().insert(
                            host as usize,
                            ListEntry {
                                host,
                                row_height,
                                automatic,
                                source: Rc::new(RefCell::new(None)),
                                pool: CellPool::default(),
                                heights: HashMap::new(),
                                height_sum: 0.0,
                                estimate: row_height as c_int,
                                rebind_all: false,
                                frame_height: -1,
                                fill_pending: false,
                                last_width: -1,
                                cell_width: -1,
                                node: id.0,
                                multi: p.multi_select,
                                pin: None,
                                pin_row: None,
                                headers_for: None,
                                supplements_for: None,
                                supplement_heights: HashMap::new(),
                                pending: None,
                            },
                        )
                    });
                    LIST_BY_NODE.with(|m| m.borrow_mut().insert(id.0, host as usize));
                    QtHandle(host)
                }
                // A recycled list cell is ADOPTED from the native list, never realized
                // through this path; anything else is an extension piece.
                Some(Builtin::ListCell | Builtin::ListItem) | None => {
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
        anim: Option<&AnimSpec>,
    ) {
        if let Some(patch) = patch.downcast_ref::<TreePatch>() {
            tree::patch(h.0 as usize, patch);
            return;
        }
        unsafe {
            match kind {
                kinds::IMAGE => {
                    if let Some(p) = patch.downcast_ref::<day_spec::props::ImagePatch>() {
                        match p {
                            day_spec::props::ImagePatch::Tint(c) => {
                                // The shim remembers whether the glyph is a template, so a
                                // cleared tint lands back on the palette color rather than the
                                // authored art.
                                let tint = image_tint((*c).map(day_spec::Ink::resolved), false);
                                ffi::day_qt_image_set_tint(h.0, cstr(&tint).as_ptr());
                                remember_inks(h.0, |i| i.image_tint = dynamic(*c));
                            }
                            // A source swap repaints the same widget (docs/images.md), so an
                            // `image()` bound to a signal shows new pixels without rebuilding
                            // its subtree.
                            day_spec::props::ImagePatch::Source(source) => match source {
                                day_spec::ImageSource::Named(named) => {
                                    let res_path = format!(":/day/images/{named}");
                                    let path = if let Some(svg) =
                                        day_spec::resource::resolve_vector_svg(named)
                                    {
                                        svg.to_string_lossy().into_owned()
                                    } else if ffi::day_qt_resource_exists(cstr(&res_path).as_ptr())
                                        != 0
                                    {
                                        res_path
                                    } else {
                                        icon_file_path(named)
                                    };
                                    // The tint setter re-renders from the widget's recorded path,
                                    // so pointing it at the new file is the whole swap.
                                    ffi::day_qt_image_set_path(h.0, cstr(&path).as_ptr());
                                }
                                day_spec::ImageSource::Bytes(bytes) => {
                                    ffi::day_qt_image_set_bytes(
                                        h.0,
                                        bytes.as_ptr(),
                                        bytes.len() as c_int,
                                    );
                                }
                                day_spec::ImageSource::Decoded(id) => {
                                    ffi::day_qt_image_set_bitmap(h.0, id.0);
                                }
                            },
                        }
                    }
                }
                kinds::CONTAINER => {
                    let key = h.0 as usize;
                    match patch.downcast_ref::<ContainerPatch>() {
                        Some(ContainerPatch::Fill(fill)) => {
                            let mut p = SURFACES.with(|t| t.take(key)).unwrap_or_default();
                            p.fill.clone_from(fill);
                            match solid_fill(fill.as_ref()).filter(|_| p.role.is_none()) {
                                // Preserve the corner radius set at realize — a reactive
                                // `.background` must not square off a rounded surface.
                                // Animated from the fill shown when both ends are a fill
                                // (rule 8); a fill appearing or going away lands at once in
                                // the shim.
                                Some(bg) => with_qt_anim(anim, |anim| {
                                    ffi::day_qt_widget_set_bg(h.0, bg.r, bg.g, bg.b, bg.a, anim);
                                }),
                                // No fill, or a gradient: the whole surface, at once.
                                None => apply_surface(h.0, &p, false),
                            }
                            SURFACES.with(|t| t.insert(key, p));
                        }
                        // A reactive surface or a frame-relative radius (docs/surfaces.md
                        // "Reactive surfaces"): every field at once, applied at once; the
                        // role stays the realized one.
                        Some(ContainerPatch::Surface(next)) => {
                            let mut p = (**next).clone();
                            p.role = SURFACES.with(|t| t.get(key)).and_then(|old| old.role);
                            apply_surface(h.0, &p, false);
                            SURFACES.with(|t| t.insert(key, p));
                        }
                        None => {}
                    }
                }
                kinds::NAV_MENU => {
                    if let Some(NavMenuPatch::Items {
                        document_keys,
                        items,
                        icons,
                        tints,
                        menus,
                        badges,
                        badge_icons,
                        badge_tints,
                        sections,
                        selected,
                        rows_changed,
                        // `movable` is for toolkits whose own sidebar drags (Cap::NavReorder).
                        ..
                    }) = patch.downcast_ref::<NavMenuPatch>()
                    {
                        NAV_SUITE_ROWS.with(|m| {
                            if let Some(row) = m.borrow_mut().get_mut(&(h.0 as usize)) {
                                row.1.clone_from(items);
                                row.2.clone_from(icons);
                                row.3 = document_keys.to_vec();
                            }
                        });
                        let host = ffi::day_qt_enclosing_tabs(h.0);
                        if !host.is_null() {
                            NAV_SUITES.with(|m| {
                                if let Some(s) = m.borrow_mut().get_mut(&(host as usize)) {
                                    s.titles.clone_from(items);
                                    s.icons.clone_from(icons);
                                    s.keys = document_keys.to_vec();
                                }
                            });
                            suite_apply_rows(host);
                        }
                        // Data-driven rows: rebuild the QListWidget from the new labels/icons
                        // (the same shim call realize uses), then apply the selection; the same
                        // rows with new words or counts update those rows in place.
                        let rows = NavRows {
                            items: items.clone(),
                            icons: icons.clone(),
                            tints: tints.clone(),
                            badge_icons: badge_icons.clone(),
                            badge_tints: badge_tints.clone(),
                            badges: badges.clone(),
                        };
                        if let Some(changed) = rows_changed {
                            apply_nav_rows(h.0, &rows, Some(changed), sections);
                            return;
                        }
                        let headings = apply_nav_rows(h.0, &rows, None, sections);
                        navlist_apply_row_menus(h.0, menus);
                        ffi::day_qt_navlist_set_selected(h.0, selected.map_or(-1, |i| i as c_int));
                        NAV_MENU_ROWS
                            .with(|m| m.borrow_mut().insert(h.0 as usize, (items.len(), headings)));
                    } else if let Some(NavMenuPatch::Selected(sel)) =
                        patch.downcast_ref::<NavMenuPatch>()
                    {
                        ffi::day_qt_navlist_set_selected(h.0, sel.map_or(-1, |i| i as c_int));
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
                                if let Some(bg) = background.map(day_spec::Ink::resolved) {
                                    ffi::day_qt_widget_set_bg(
                                        h.0,
                                        bg.r,
                                        bg.g,
                                        bg.b,
                                        bg.a,
                                        std::ptr::null(),
                                    );
                                }
                                remember_inks(h.0, |i| i.cover = dynamic(*background));
                                ffi::day_qt_add_child(self.window, h.0);
                                // A clear cover (a composed sheet's) lets the page show through.
                                let opaque = background.is_none_or(|bg| bg.resolved().a > 0.0);
                                ffi::day_qt_cover_top(h.0, c_int::from(opaque));
                                let (y, size) =
                                    cover_frame(h.0, LAST_WINDOW_SIZE.with(std::cell::Cell::get));
                                ffi::day_qt_set_geometry(
                                    h.0,
                                    0,
                                    y as c_int,
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
                kinds::SPLIT => {
                    if let Some(p) = patch.downcast_ref::<SplitPatch>() {
                        let apply = SPLIT_STATE.with(|m| {
                            let mut m = m.borrow_mut();
                            let state = m.get_mut(&(h.0 as usize))?;
                            match p {
                                SplitPatch::Fraction(f) => state.share = f.clamp(0.0, 1.0),
                                SplitPatch::Axis(_) => {}
                            }
                            state.suppress = true;
                            Some((state.placed, state.share, state.min_pane))
                        });
                        if let Some((placed, share, min_pane)) = apply {
                            if let SplitPatch::Axis(axis) = p {
                                let vertical = *axis == day_spec::props::SplitAxis::Vertical;
                                ffi::day_qt_split_set_orientation(
                                    h.0,
                                    c_int::from(vertical),
                                    min_pane,
                                );
                            }
                            if placed {
                                ffi::day_qt_split_set_fraction(h.0, share);
                            }
                            SPLIT_STATE.with(|m| {
                                if let Some(state) = m.borrow_mut().get_mut(&(h.0 as usize)) {
                                    state.suppress = false;
                                }
                            });
                            split_sync_panes(h.0);
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
                            inspector_sync_panes(host_addr as *mut std::os::raw::c_void);
                        }));
                    }
                }
                kinds::NAV => {
                    if let Some(p) = patch.downcast_ref::<day_spec::props::DocumentTabsPatch>() {
                        ffi::day_qt_tabs_chrome(h.0, c_int::from(p.native));
                        nav_suite_sync(h.0);
                        return;
                    }
                    if let Some(NavPatch::Reorder(order)) = patch.downcast_ref::<NavPatch>() {
                        NAV_SUITES.with(|m| {
                            let mut m = m.borrow_mut();
                            if let Some(s) = m.get_mut(&(h.0 as usize)) {
                                let old = s.pages.clone();
                                s.pages =
                                    order.iter().filter_map(|i| old.get(*i).copied()).collect();
                                for (i, (page, _)) in s.pages.iter().enumerate() {
                                    ffi::day_qt_tabs_move_page(h.0, page.0, i as c_int);
                                }
                            }
                        });
                        suite_apply_rows(h.0);
                        return;
                    }
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
                    // A navigation suite's resident-page switch (docs/navigation.md): the host
                    // is a QTabWidget with no `NAV_STATE` entry, so it is answered here, ahead
                    // of the split/stack block below — which returns early for it. (Until
                    // 2026-09-12 the suite branch sat inside that block and never ran, so a
                    // programmatic selection — a route, a deep link — left the bar on the
                    // first tab.) Destination i is tab i: the sidebar page is parked, never a
                    // tab. The newly shown page gets its size report at once.
                    if let Some(NavPatch::Select(i)) = patch.downcast_ref::<NavPatch>()
                        && NAV_SUITES.with(|m| m.borrow().contains_key(&(h.0 as usize)))
                    {
                        ffi::day_qt_tabs_set_current(h.0, *i as c_int);
                        nav_suite_sync(h.0);
                        return;
                    }
                    if let Some(p) = patch.downcast_ref::<NavPatch>() {
                        // A pane shown or hidden outside the borrow below: Qt resizes the
                        // sibling panes synchronously, the pane filter reports them, and
                        // that report reads `NAV_STATE` — a re-entrant borrow otherwise.
                        let mut list_apply: Option<(*mut c_void, bool, Option<f64>)> = None;
                        NAV_STATE.with(|m| {
                            let mut m = m.borrow_mut();
                            let Some(state) = m.get_mut(&(h.0 as usize)) else {
                                return;
                            };
                            // Split: the sidebar's page is not part of the detail stack.
                            // Stack: every page participates, its root included. The list
                            // page never is; its pane persists at every presentation.
                            let split_now = state.split;
                            let detail: Vec<(QtHandle, NodeId)> = state
                                .pages
                                .iter()
                                .filter(|(p, _)| {
                                    !(split_now && is_sidebar_page(p.0) || is_list_page(p.0))
                                })
                                .copied()
                                .collect();
                            let detail = &detail[..];
                            match p {
                                NavPatch::Pushed { title, .. } => {
                                    let last = detail.len().saturating_sub(1);
                                    for (i, (page, _)) in detail.iter().enumerate() {
                                        ffi::day_qt_set_visible(page.0, (i == last).into());
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
                                        last.clone_from(t);
                                    }
                                }
                                // Custom back header → always NavBack{already_popped:false};
                                // no native auto-pop to suppress (docs/navigation.md).
                                #[expect(
                                    clippy::match_same_arms,
                                    reason = "each group of no-op patches keeps its own arm and the comment saying why"
                                )]
                                NavPatch::GuardTop(_) | NavPatch::Reorder(_) => {}
                                // Handled before this borrow (it re-parents widgets).
                                #[expect(
                                    clippy::match_same_arms,
                                    reason = "each group of no-op patches keeps its own arm and the comment saying why"
                                )]
                                NavPatch::Presentation(_) => {}
                                // The resident-page switch (docs/navigation.md): the app moved the
                                // selection, so the suite shows that destination.
                                NavPatch::Select(i) => {
                                    // The resident-page switch (docs/navigation.md) on a
                                    // split — which is also what a ROUNDED `Rail` lands on
                                    // here, Qt having no rail widget: show destination `i`
                                    // and hide its siblings. A suite's switch was answered
                                    // above, before this block.
                                    for (n, (page, _)) in detail.iter().enumerate() {
                                        ffi::day_qt_set_visible(page.0, c_int::from(n == *i));
                                    }
                                }
                                // Show or collapse the content-list pane (docs/navigation.md).
                                // The sibling panes take their new widths on Qt's next layout
                                // pass, so the frame report is deferred one turn below, like
                                // a pop's.
                                NavPatch::ListVisible(v) => {
                                    state.list_shown = *v;
                                    if state.list_width.is_some() {
                                        // First time on screen: the width the app asked for.
                                        // The constructor sized a pane that was showing; one
                                        // hidden since then still holds its placeholder.
                                        let place = if *v && !state.list_positioned {
                                            state.list_positioned = true;
                                            state.list_width
                                        } else {
                                            None
                                        };
                                        list_apply = Some((state.list_pane, *v, place));
                                    }
                                }
                                // Never arrives: the pane persists through every presentation
                                // (`Cap::NavContentList` = Native), so the pieces layer never
                                // asks for the merged-stack shape.
                                NavPatch::ListInStack(_) => {}
                            }
                        });
                        if let Some((pane, shown, place)) = list_apply {
                            ffi::day_qt_set_visible(pane, c_int::from(shown));
                            if let Some(w) = place {
                                ffi::day_qt_splitter_set_pane_width(h.0, 1, w);
                            }
                        }
                        // Header visibility follows the depth after the pop completes (the
                        // popped page leaves `pages` via remove()); defer one turn so the
                        // header + page sizes settle against the final stack. The raw QWidget
                        // pointer crosses the (main-thread-only) post as usize. The same
                        // deferral gives a shown or hidden list pane its settled sizes.
                        let host = h.0 as usize;
                        <Qt as Platform>::post(Box::new(move || {
                            nav_sync_header(host as *mut std::os::raw::c_void);
                        }));
                    }
                }
                kinds::LABEL => {
                    if let Some(p) = patch.downcast_ref::<LabelPatch>() {
                        match p {
                            LabelPatch::Paragraph(p) => apply_paragraph(h.0, p),
                            LabelPatch::Text(t) => {
                                ffi::day_qt_label_set_text(h.0, cstr(t).as_ptr());
                                remember_inks(h.0, |i| i.label_runs = None);
                            }
                            LabelPatch::Font(f) => {
                                apply_label_font(h.0, *f);
                                apply_custom_family(h.0, *f);
                                remember_label_pt(h.0, *f);
                            }
                            LabelPatch::Runs(text, runs) => apply_label_runs(h.0, text, runs),
                            LabelPatch::Color(c) => apply_label_color(h.0, *c),
                        }
                    }
                }
                kinds::BUTTON => {
                    if let Some(p) = patch.downcast_ref::<ButtonPatch>() {
                        match p {
                            ButtonPatch::Content(c) => {
                                apply_button_content(h.0, &c.title, c.icon.as_ref(), c.icon_only);
                            }
                            ButtonPatch::Title(t) => {
                                ffi::day_qt_button_set_title(h.0, cstr(t).as_ptr());
                            }
                            ButtonPatch::Enabled(e) => {
                                ffi::day_qt_set_enabled(h.0, c_int::from(*e));
                            }
                            ButtonPatch::Selected(on) => {
                                ffi::day_qt_button_set_selected(h.0, c_int::from(*on));
                            }
                            ButtonPatch::Style(s) => apply_button_style_ink(h.0, *s),
                        }
                    }
                }
                kinds::TOGGLE => {
                    if let Some(p) = patch.downcast_ref::<TogglePatch>() {
                        match p {
                            TogglePatch::On(on) => ffi::day_qt_checkbox_set(h.0, c_int::from(*on)),
                            TogglePatch::Enabled(e) => {
                                ffi::day_qt_set_enabled(h.0, c_int::from(*e));
                            }
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
                            SliderPatch::Enabled(e) => {
                                ffi::day_qt_set_enabled(h.0, c_int::from(*e));
                            }
                        }
                    }
                }
                kinds::PROGRESS => {
                    if let Some(p @ ProgressPatch::Animating(_)) =
                        patch.downcast_ref::<ProgressPatch>()
                    {
                        crate::controls::spinner::update(self, h, p);
                        return;
                    }

                    if let Some(ProgressPatch::Value(Some(v))) =
                        patch.downcast_ref::<ProgressPatch>()
                    {
                        ffi::day_qt_progress_set(h.0, progress_ticks(*v));
                    }
                }
                kinds::PICKER => picker::update_any(self, *h, patch),
                kinds::TEXT_AREA => textarea::update_any(self, *h, patch),
                kinds::TEXT_FIELD => {
                    if let Some(p) = patch.downcast_ref::<TextFieldPatch>() {
                        match p {
                            TextFieldPatch::Text { text, from_native } => {
                                if !*from_native {
                                    ffi::day_qt_lineedit_set_text(h.0, cstr(text).as_ptr());
                                }
                            }
                            TextFieldPatch::Placeholder(t) => {
                                ffi::day_qt_lineedit_set_placeholder(h.0, cstr(t).as_ptr());
                            }
                            TextFieldPatch::Enabled(e) => {
                                ffi::day_qt_set_enabled(h.0, c_int::from(*e));
                            }
                            TextFieldPatch::Select(sel) => {
                                let at = |n: usize| n.min(c_int::MAX as usize) as c_int;
                                ffi::day_qt_lineedit_select(h.0, at(sel.start), at(sel.end));
                            }
                        }
                    }
                }
                kinds::LIST => match patch.downcast_ref::<ListPatch>() {
                    Some(ListPatch::Reload) => queue_rows(h.0 as usize, RowWork::Reload),
                    // Row deltas as model deltas: selection and the current row follow.
                    Some(ListPatch::Splice(deltas)) => {
                        queue_rows(h.0 as usize, RowWork::Splice(deltas.clone()));
                    }
                    Some(ListPatch::ScrollToEnd) => schedule_list_scroll_end(h.0 as usize),
                    Some(ListPatch::ScrollToRow(row)) => {
                        schedule_list_scroll_row(h.0 as usize, *row);
                    }
                    Some(ListPatch::Selected(rows)) => {
                        // Programmatic selection sync (empty = clear): the view repaints and
                        // does not report it back.
                        let rows: Vec<c_int> = rows.iter().map(|r| *r as c_int).collect();
                        ffi::day_qt_list_set_selected(h.0, rows.as_ptr(), rows.len() as c_int);
                    }
                    Some(ListPatch::RowSizeInvalidated { cell, height }) => {
                        schedule_list_row_height(h.0 as usize, *cell, *height);
                    }
                    // No editing mode (Cap::ListEditing is Unsupported).
                    Some(ListPatch::Editing(_)) | None => {}
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
        // Now destroy the whole DayWindow (deleteLater — after this event dispatch), never
        // before, so day's child-widget releases can't touch freed memory.
        let toolbars = &mut self.toolbars;
        self.secondary.retain(|w| {
            if w.content == h.0 {
                #[cfg(target_os = "macos")]
                day_platform_macos::window_tabs::unregister(w.id);
                toolbars.remove(&(w.win as usize));
                unsafe { ffi::day_qt_window_destroy(w.win) };
                false
            } else {
                true
            }
        });
        let key = h.0 as usize;
        tree::release(key);
        if let Some(entry) = LIST_STATE.with(|m| m.borrow_mut().remove(&key)) {
            LIST_BY_NODE.with(|m| {
                m.borrow_mut().remove(&entry.node);
            });
            // The pinned header's cell is Day's row like any other (docs/list.md "Sections"):
            // give it back; its widget goes with the view.
            let source = entry.source.borrow().clone();
            if let (Some(pin), Some(source)) = (entry.pin, source) {
                (source.release)(pin as *mut c_void);
            }
        }
        // A disposed nav host / page must drop its NAV_STATE / NAV_PAGE_IDS entry — otherwise a
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
        SPLIT_STATE.with(|m| {
            m.borrow_mut().remove(&key);
        });
        SPLIT_PANE_IDS.with(|m| {
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
        COVER_HALF.with(|s| s.borrow_mut().remove(&(h.0 as usize)));
        COVER_IDS.with(|m| {
            m.borrow_mut().remove(&key);
        });
        // One sweep drops this handle from every SideTable registered on this thread — the
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
        // An emulated cover (docs/cover.md) is parked hidden at realize and surfaces only
        // through `CoverPatch::Present`, which re-homes it onto the window. `day_qt_add_child`
        // shows whatever it parents, so a cover inserted like any other child came back at
        // Qt's 640x480 default geometry over the page's top-left corner: nothing to see (no
        // fill), but in front of the sidebar, taking every click meant for it. Parent it for
        // ownership only, and leave it parked.
        if COVER_IDS.with(|m| m.borrow().contains_key(&(child.0 as usize))) {
            unsafe {
                ffi::day_qt_add_child(content_of(parent), child.0);
                ffi::day_qt_set_visible(child.0, 0);
            }
            return;
        }
        // Navigation suite: every destination becomes a tab. Day's nav host keeps its menu as
        // child 0, inside the SIDEBAR page whose rows became the bar; that page is parked under
        // the widget (so the menu keeps a parent chain up to the suite) and never a tab, because
        // drawing the rows again as a list would be the same navigation twice. So destination
        // i, Day's child i + 1, is tab i.
        let suite_handled = NAV_SUITES.with(|m| {
            let mut m = m.borrow_mut();
            let Some(suite) = m.get_mut(&(parent.0 as usize)) else {
                return false;
            };
            unsafe {
                if index == 0 {
                    ffi::day_qt_tabs_park(parent.0, child.0);
                } else {
                    ffi::day_qt_tabs_add_page(
                        parent.0,
                        child.0,
                        cstr("").as_ptr(),
                        (index - 1) as c_int,
                    );
                    let id = NAV_PAGE_IDS
                        .with(|ids| ids.borrow().get(&(child.0 as usize)).copied())
                        .unwrap_or(NodeId(0));
                    suite.pages.push((*child, id));
                    // The first destination claims the content area (insertion is silent).
                    if suite.pages.len() == 1 {
                        ffi::day_qt_tabs_set_current(parent.0, 0);
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
        if let Some((node, titles, icons, keys)) =
            NAV_SUITE_ROWS.with(|m| m.borrow().get(&(child.0 as usize)).cloned())
        {
            let suite = unsafe { ffi::day_qt_enclosing_tabs(parent.0) };
            if !suite.is_null() {
                NAV_SUITES.with(|m| {
                    if let Some(s) = m.borrow_mut().get_mut(&(suite as usize)) {
                        s.menu_node = node;
                        s.titles = titles;
                        s.icons = icons;
                        s.keys = keys;
                    }
                });
                suite_apply_rows(suite);
            }
        }
        // Nav host: pages land by their PANE, not their position (docs/size-classes.md).
        let sidebar = is_sidebar_page(child.0);
        let list = is_list_page(child.0);
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
            if list {
                state.list_page = Some(*child);
            }
            // The content-list page fills its own pane at every presentation
            // (docs/navigation.md) — never a member of the detail stack.
            let pane = if list {
                state.list_pane
            } else if state.split && sidebar {
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
        // Split host: the pane's own record says which pane it is (docs/split.md).
        let split_pane = SPLIT_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let Some(state) = m.get_mut(&(parent.0 as usize)) else {
                return false;
            };
            let (pane_id, second) = SPLIT_PANE_IDS
                .with(|ids| ids.borrow().get(&(child.0 as usize)).copied())
                .unwrap_or((NodeId(0), index == 1));
            let pane = if second {
                state.second_pane
            } else {
                state.first_pane
            };
            unsafe { ffi::day_qt_add_child(pane, child.0) };
            state.panes.push((pane_id, second));
            true
        });
        if split_pane {
            split_sync_panes(parent.0);
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
                if state.list_page.map(|p| p.0) == Some(child.0) {
                    state.list_page = None;
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
        // A label's sizeHint() is a full rich-text layout at Qt's own wrap heuristic, and the
        // label arm below never uses it except as a last resort: ask only then.
        if kind != kinds::LABEL {
            unsafe { ffi::day_qt_size_hint(h.0, &raw mut w, &raw mut hh) };
        }
        match kind {
            kinds::NAV_MENU => {
                let (rows, headings) = NAV_MENU_ROWS
                    .with(|m| m.borrow().get(&(h.0 as usize)).copied().unwrap_or((0, 0)));
                // A heading band is its caption plus the gap above it (shim.cpp,
                // day_qt_navlist_set_items); left uncounted, a sectioned sidebar under-reports by
                // a band per group.
                Size::new(
                    p.width.unwrap_or(220.0),
                    p.height
                        .unwrap_or(rows as f64 * 34.0 + headings as f64 * 30.0 + 8.0),
                )
            }
            kinds::LABEL => {
                // Natural width from font metrics — QLabel::sizeHint() suggests a narrow
                // "readable column" for word-wrapped labels, which is not day's contract
                // (natural = unwrapped). Height always via heightForWidth at the width day
                // actually grants; sizeHint's heuristic height is never mixed in.
                let nat_w = f64::from(unsafe { ffi::day_qt_label_natural_width(h.0) });
                let width = match p.width {
                    Some(pw) => nat_w.min(pw),
                    None => nat_w,
                };
                let hfw = unsafe {
                    ffi::day_qt_label_height_for_width(h.0, width.round().max(1.0) as c_int)
                };
                if hfw > 0 {
                    Size::new(width.ceil(), f64::from(hfw))
                } else {
                    unsafe { ffi::day_qt_size_hint(h.0, &raw mut w, &raw mut hh) };
                    Size::new(width.ceil(), hh.ceil())
                }
            }
            // Buttons hug their text (sizeHint = content + chrome) like every other
            // toolkit: the generic arm would swallow a COLUMN's cross-axis width proposal
            // and stretch the button across the full content span.
            kinds::BUTTON => Size::new(w.ceil(), hh.ceil()),
            kinds::SLIDER => Size::new(p.width.unwrap_or(180.0), hh.max(20.0)),
            kinds::PICKER => picker::measure_any(self, *h, p),
            kinds::TEXT_AREA => textarea::measure_any(self, *h, p),
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
        // Opacity + transform share one custom QGraphicsEffect per widget (see the shim); an
        // animated change plays as a run on Qt's animation framework (`QtAnim`).
        // SAFETY: `h` is a live widget Day realized; the run is alive for the call.
        with_qt_anim(anim, |anim| unsafe {
            ffi::day_qt_set_opacity(h.0, opacity, anim);
        });
    }

    fn set_transform(&mut self, h: &QtHandle, t: Transform, _size: Size, anim: Option<&AnimSpec>) {
        // The effect re-draws the widget through a painter transformed about its center, spilling
        // beyond the widget's own frame (§8.4) so a moved/scaled/rotated box isn't clipped.
        // SAFETY: as in `set_opacity`.
        with_qt_anim(anim, |anim| unsafe {
            ffi::day_qt_set_transform(h.0, t.tx, t.ty, t.sx, t.sy, t.rotation.degrees(), anim);
        });
    }

    /// What the widget shows now: the shim's runs write the `DayEffect`, the geometry and the
    /// fill on every tick, so these are the in-flight values; at rest, the model's.
    fn read_displayed(&self, h: &QtHandle) -> day_spec::Displayed {
        if h.0.is_null() {
            return day_spec::Displayed::default();
        }
        let mut d = ffi::DayQtDisplayed::default();
        // SAFETY: `h` is a live widget Day realized; the shim writes every field of `d`.
        unsafe { ffi::day_qt_read_displayed(h.0, &raw mut d) };
        day_spec::Displayed {
            opacity: Some(d.opacity),
            frame: (d.frame_ok != 0).then(|| Rect::new(d.x, d.y, d.w, d.h)),
            translation: Some(day_spec::Point::new(d.tx, d.ty)),
            background: (d.fill_ok != 0).then(|| day_spec::Color::rgba(d.r, d.g, d.b, d.a)),
        }
    }

    fn set_cursor(&mut self, h: &QtHandle, cursor: Cursor) {
        // `QWidget::setCursor(Qt::CursorShape)`, or `unsetCursor` for Default (docs/cursor.md).
        // Qt has no zoom, cell, copy-drop, context-menu, or vertical-text shape; each takes its
        // nearest, which is why `Cap::Cursor` answers Emulated.
        unsafe { ffi::day_qt_widget_set_cursor(h.0, qt_cursor_shape(&cursor)) };
    }

    fn set_input_traits(&mut self, h: &QtHandle, traits: &InputTraits) -> Option<QtHandle> {
        // One QLineEdit serves every trait as a property (docs/textfield.md), so the widget,
        // its signal wiring, its focus and its caret all stay. The shim qobject_casts to
        // QLineEdit, so a non-field handle is a safe no-op.
        //
        // Two members stay on the Rust side. `submit_label`: QLineEdit has no setter for the
        // enter-key type (only Qt Quick exposes one). `max_length`: `QLineEdit::setMaxLength`
        // counts UTF-16 code units where day-pieces counts characters, so a native limit would
        // refuse text the bound allows (five emoji are ten units); day-pieces holds it alone.
        unsafe {
            ffi::day_qt_lineedit_set_traits(
                h.0,
                c_int::from(traits.secure),
                c_int::from(traits.read_only),
                qt_input_purpose(traits.purpose),
            );
        };
        None
    }

    fn set_selectable(&mut self, h: &QtHandle, selectable: bool) -> Option<QtHandle> {
        // The shim casts to Day's label, so a non-label handle is a safe no-op (docs/text.md).
        unsafe { ffi::day_qt_label_set_selectable(h.0, c_int::from(selectable)) };
        None
    }

    /// The screen's logical DPI against the platform's reference: what the desktop's text-size
    /// setting (GNOME's text-scaling-factor, KDE's forced font DPI, both through `Xft.dpi`)
    /// does to Day's point-sized fonts. 1.0 on macOS, which has no such setting.
    fn text_scale(&mut self, _style: Font) -> f64 {
        // SAFETY: reads the primary screen's DPI; called on the UI thread after `run` built the
        // QApplication (and answers 1.0 when there is no screen).
        unsafe { ffi::day_qt_text_scale() }
    }

    /// `QWidget::setLayoutDirection` on the widget itself. Day's containers have no Qt layout,
    /// so the direction never moves a child Day placed; hosts whose Qt widget mirrors its own
    /// coordinates (scroll areas, splitters, lists, trees) are skipped, and so is the canvas,
    /// which draws in its own coordinates.
    fn set_layout_direction(
        &mut self,
        h: &QtHandle,
        kind: PieceKind,
        direction: day_spec::LayoutDirection,
    ) {
        if matches!(
            kind,
            kinds::CONTAINER
                | kinds::SCROLL
                | kinds::CANVAS
                | kinds::IMAGE
                | kinds::NAV
                | kinds::NAV_PAGE
                | kinds::NAV_MENU
                | kinds::LIST
                | kinds::LIST_CELL
                | kinds::TREE
                | kinds::COVER
                | kinds::INSPECTOR
                | kinds::INSPECTOR_PANE
                | kinds::SPLIT
                | kinds::SPLIT_PANE
        ) || h.0.is_null()
        {
            return;
        }
        let rtl = c_int::from(direction == day_spec::LayoutDirection::Rtl);
        // SAFETY: `h` is a live widget Day realized.
        unsafe { ffi::day_qt_widget_set_direction(h.0, rtl) };
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

    fn set_frame(&mut self, h: &QtHandle, frame: Rect, anim: Option<&AnimSpec>) {
        // A host whose panes or rows are laid out from its native size (nav, split, inspector,
        // list) re-reports them below from the size Qt gives it, so it takes its frame at once;
        // everything else animates its geometry under `anim` (a run in the shim).
        let key = h.0 as usize;
        let native_host = NAV_STATE.with(|m| m.borrow().contains_key(&key))
            || NAV_SUITES.with(|m| m.borrow().contains_key(&key))
            || SPLIT_STATE.with(|m| m.borrow().contains_key(&key))
            || INSPECTOR_STATE.with(|m| m.borrow().contains_key(&key))
            || LIST_STATE.with(|m| m.borrow().contains_key(&key));
        let anim = if native_host { None } else { anim };
        // SAFETY: `h` is a live widget Day realized; the run is alive for the call.
        with_qt_anim(anim, |anim| unsafe {
            ffi::day_qt_set_frame(
                h.0,
                frame.origin.x.round() as c_int,
                frame.origin.y.round() as c_int,
                frame.size.width.round() as c_int,
                frame.size.height.round() as c_int,
                anim,
            );
        });
        // Nav host resized (window resize): re-report page sizes for relayout.
        if NAV_STATE.with(|m| m.borrow().contains_key(&(h.0 as usize))) {
            nav_sync_panes(h.0);
        }
        if NAV_SUITES.with(|m| m.borrow().contains_key(&(h.0 as usize))) {
            nav_suite_sync(h.0);
        }
        // A split host framed: its pane frames are native-owned too (docs/split.md).
        if SPLIT_STATE.with(|m| m.borrow().contains_key(&(h.0 as usize))) {
            split_sync_panes(h.0);
        }
        // Same for an inspector split: its pane frames are native-owned too.
        if INSPECTOR_STATE.with(|m| m.borrow().contains_key(&(h.0 as usize))) {
            inspector_sync_panes(h.0);
        }
        // List host framed: (re)fill its cells — but only when the width actually changed, so the
        // set_frame calls a populate itself makes (on row content) don't schedule another forever.
        let width_changed = LIST_STATE.with(|m| {
            m.borrow()
                .get(&(h.0 as usize))
                .is_some_and(|st| st.last_width != frame.size.width.round() as c_int)
        });
        // A taller host shows more rows, and the ones it grew into are not built yet — so a
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

    fn scroll_bar_inset(&mut self, h: &QtHandle, vertical: bool) -> f64 {
        f64::from(unsafe { ffi::day_qt_scroll_bar_inset(h.0, c_int::from(vertical)) })
    }

    fn set_scroll_content(&mut self, h: &QtHandle, content: Size) {
        unsafe {
            ffi::day_qt_scroll_set_content_size(
                h.0,
                content.width.round() as c_int,
                content.height.round() as c_int,
            );
        };
    }

    /// A Qt item view row: compact, 8px sides and 28px tall.
    fn list_row_metrics(&mut self, _h: &QtHandle) -> day_spec::RowMetrics {
        day_spec::RowMetrics {
            insets: day_spec::Insets {
                top: 4.0,
                leading: 8.0,
                bottom: 4.0,
                trailing: 8.0,
            },
            min_height: 28.0,
        }
    }

    fn scroll_to(&mut self, h: &QtHandle, target: Rect, animated: bool) {
        // Qt's scroll bars have no animation of their own: an animated scroll glides the bars'
        // values for 250 ms with an ease-out cubic, and lands where the jump would (docs/scroll.md).
        unsafe {
            ffi::day_qt_scroll_to_rect(
                h.0,
                target.origin.x.round() as c_int,
                target.origin.y.round() as c_int,
                target.size.width.round() as c_int,
                target.size.height.round() as c_int,
                c_int::from(animated),
            );
        };
    }

    fn scroll_offset(&mut self, h: &QtHandle) -> day_spec::Point {
        let (mut x, mut y) = (0.0_f64, 0.0_f64);
        unsafe { ffi::day_qt_scroll_offset(h.0, &raw mut x, &raw mut y) };
        day_spec::Point::new(x, y)
    }

    fn list_scroll(&mut self, h: &QtHandle, to: day_spec::ListScroll) -> bool {
        // QListView scrolls per pixel, so the shim aims the vertical bar itself: the row's
        // aligned position (`QAbstractItemView::scrollTo` has no "nearest" that keeps a fully
        // visible row still, nor a pinned-header inset), a jump or a 250 ms glide, re-aimed
        // while rows it reveals are measured. Posted: the request arrives in the same turn as
        // the splice that made its row, and the row flush posted first runs first.
        let host_key = h.0 as usize;
        let row = to
            .row
            .map_or(-1, |r| c_int::try_from(r).unwrap_or(c_int::MAX));
        let align = row_align_code(to.align);
        let animated = c_int::from(to.animated);
        Qt::post(Box::new(move || {
            if let Some(host) = LIST_STATE.with(|m| m.borrow().get(&host_key).map(|st| st.host)) {
                unsafe { ffi::day_qt_list_scroll(host, row, align, animated) };
            }
        }));
        true
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
                );
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
                );
            },
            PresentSpec::OpenFile { multiple, .. } => unsafe {
                ffi::day_qt_present_file_open(
                    req,
                    cstr(spec.title()).as_ptr(),
                    cstr(&spec.filters_joined()).as_ptr(),
                    c_int::from(*multiple),
                    parent,
                );
            },
            PresentSpec::OpenFolder { .. } => unsafe {
                ffi::day_qt_present_folder_open(req, cstr(spec.title()).as_ptr(), parent);
            },
            PresentSpec::SaveFile { suggested_name, .. } => unsafe {
                // The pieces layer copies the staged bytes to the chosen path.
                ffi::day_qt_present_file_save(
                    req,
                    cstr(spec.title()).as_ptr(),
                    cstr(suggested_name).as_ptr(),
                    cstr(&spec.filters_joined()).as_ptr(),
                    parent,
                );
            },
        }
    }

    fn dismiss(&mut self, req: u64) {
        unsafe { ffi::day_qt_dismiss_present(req) };
    }

    #[cfg(target_os = "macos")]
    fn share_support(&self) -> day_spec::Support {
        day_spec::Support::Native
    }
    #[cfg(target_os = "macos")]
    fn share(&mut self, spec: &day_spec::share::ShareSpec, _anchor: Option<&Self::Handle>) -> bool {
        unsafe { ffi::day_qt_share_url(cstr(&spec.url).as_ptr(), cstr(&spec.title).as_ptr()) }
    }
    fn open_url(&mut self, url: &str) {
        if let Ok(c) = std::ffi::CString::new(url) {
            unsafe { ffi::day_qt_open_url(c.as_ptr()) };
        }
    }

    fn set_focusable(&mut self, h: &QtHandle, node: NodeId, focusable: bool) {
        if focusable {
            unsafe {
                ffi::day_qt_enable_keys(h.0, node.0, on_key);
                ffi::day_qt_enable_focus(h.0, node.0, on_focus);
                ffi::day_qt_set_focusable(h.0);
            }
        }
    }

    fn focus(&mut self, h: &QtHandle, _node: NodeId, focused: bool) {
        // The shim clears only while this widget still owns focus, so a stale release
        // can't blur a sibling.
        unsafe { ffi::day_qt_widget_focus(h.0, c_int::from(focused)) };
    }

    fn set_event_sink(&mut self, sink: EventSink) {
        SINK.with(|s| *s.borrow_mut() = Some(Rc::from(sink)));
    }

    fn enable_gesture(&mut self, h: &QtHandle, node: NodeId, kind: day_spec::GestureKind) {
        use day_spec::GestureKind as K;
        let key = (h.0 as usize, kind);
        if !GESTURES.with(|g| g.borrow_mut().insert(key)) {
            return; // already wired
        }
        let code: c_int = match kind {
            K::Tap => 0,
            K::Drag => 1,
            K::Pinch => 2,
            K::Pan => 3,
            K::Hover => 4,
            K::Rotate => 5,
            K::Press => 6,
            K::LongPress => 7,
            K::DoubleTap => 8,
            K::Touches => 9,
        };
        unsafe { ffi::day_qt_enable_gesture(h.0, node.0, code, on_gesture) };
    }

    // The shim's own recognizers (docs/gestures.md "Recognizers"): a press is the left
    // button's or the first finger's, a long press Qt's `mousePressAndHoldInterval` held within
    // its `startDragDistance`, a double tap Qt's `MouseButtonDblClick`. Touches need screen
    // contacts: Qt's touch events where the platform has a touch screen (X11's XInput 2,
    // Windows), with the mouse as one contact; a Mac's trackpad fingers are not screen
    // contacts, so there day-core derives the single contact.
    fn gesture_support(&self, kind: day_spec::GestureKind) -> Support {
        match kind {
            day_spec::GestureKind::Touches if cfg!(target_os = "macos") => Support::Unsupported,
            _ => Support::Native,
        }
    }

    fn set_hit_slop(&mut self, h: &QtHandle, _node: NodeId, slop: day_spec::Insets) {
        unsafe {
            ffi::day_qt_set_hit_slop(h.0, slop.top, slop.leading, slop.bottom, slop.trailing);
        }
    }

    fn set_drag_axis(&mut self, h: &QtHandle, _node: NodeId, axis: Option<day_spec::DragAxis>) {
        let code = match axis {
            None => 0,
            Some(day_spec::DragAxis::Horizontal) => 1,
            Some(day_spec::DragAxis::Vertical) => 2,
        };
        unsafe { ffi::day_qt_set_drag_axis(h.0, code) };
    }

    // `QWidget::setEnabled` cascades through the subtree: the controls dim, leave the focus
    // chain (focus inside moves on and reports its loss), take no input and read as disabled
    // to assistive technology; the shim counts nested calls (docs/forms.md).
    fn set_subtree_enabled(&mut self, h: &QtHandle, _node: NodeId, enabled: bool) {
        unsafe { ffi::day_qt_subtree_enabled(h.0, c_int::from(enabled)) };
    }

    fn edit_toolbar(&mut self, h: &QtHandle, ops: &[day_spec::ToolbarOp]) -> bool {
        self.edit_toolbar(h, ops)
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
        // The shim's mask uses the wire bits (`Modifiers::bits`).
        day_spec::Modifiers::from_bits((m & 7) as u8)
    }

    fn set_drag_source(&mut self, h: &QtHandle, source: day_spec::transfer::Source) {
        transfer::source(h, source);
    }
    fn set_drop_target(&mut self, h: &QtHandle, target: day_spec::transfer::Target) {
        transfer::target(h, target);
    }
    fn set_context_menu_fn(&mut self, h: &QtHandle, node: NodeId, f: day_spec::ContextMenuFn) {
        CTX_MENU_FNS.with(|m| m.borrow_mut().insert(node.0, f));
        unsafe { ffi::day_qt_context_menu_fn(h.0, node.0, on_context_menu_summon) };
    }

    fn set_button_menu(&mut self, h: &QtHandle, _node: NodeId, f: day_spec::ContextMenuFn) {
        BUTTON_MENUS.with(|t| t.insert(h.0 as usize, f));
        // SAFETY: `h` is a live widget; the shim checks it is a QPushButton (qobject_cast)
        // before connecting to its click, and parents each menu to it.
        unsafe { ffi::day_qt_button_menu_fn(h.0, h.0 as u64, on_button_menu) };
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

    fn attach_tree(&mut self, host: &QtHandle, source: day_spec::TreeSource) {
        tree::attach(host.0 as usize, source);
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
            // Role, value and hidden go to the shim's own QAccessibleInterface, which Qt's
            // stock ones cannot carry for a widget they type themselves. The codes follow
            // `Role`'s declaration order (the table sits with the declaration in day-toolkit-qt-sys).
            let (role, level) = match a11y.role {
                Role::None => (0, 0),
                Role::Button => (1, 0),
                Role::Toggle => (2, 0),
                Role::Slider => (3, 0),
                Role::TextInput => (4, 0),
                Role::Heading(level) => (5, c_int::from(level.max(1))),
                Role::Image => (6, 0),
                Role::Meter => (7, 0),
                Role::Group => (8, 0),
                Role::Tree => (9, 0),
                Role::TreeItem => (10, 0),
                Role::Tab => (11, 0),
                Role::TabList => (12, 0),
            };
            let value = a11y.value.as_deref().map(cstr);
            if role != 0 || value.is_some() || a11y.hidden || a11y.selected.is_some() {
                ffi::day_qt_set_a11y_traits(
                    h.0,
                    role,
                    level,
                    value.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
                    c_int::from(a11y.hidden),
                    a11y.selected.map_or(-1, c_int::from),
                );
            }
        }
    }

    /// The widget's own state (docs/testing.md), all of it read in the shim from the `QWidget` and
    /// its `QAccessibleInterface`. Slider and progress ticks go back through the range Day built
    /// them from; a slider whose range was never recorded reports no number.
    fn read_native(&self, h: &QtHandle) -> day_spec::NativeSnapshot {
        if h.0.is_null() {
            return day_spec::NativeSnapshot::default();
        }
        let mut n = std::mem::MaybeUninit::<ffi::DayQtNative>::zeroed();
        // SAFETY: the handle is a live QWidget Day realized, and the shim initializes every
        // field of `n` before returning.
        let mut n = unsafe {
            ffi::day_qt_read_native(h.0, n.as_mut_ptr());
            n.assume_init()
        };
        // SAFETY: each pointer is NULL or a NUL-terminated heap string the shim wrote, alive
        // until `day_qt_native_free` below.
        let string = |p: *mut c_char| {
            (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
        };
        let role = match n.role {
            1 => Role::Button,
            2 => Role::Toggle,
            3 => Role::Slider,
            4 => Role::TextInput,
            5 => Role::Heading(u8::try_from(n.level).unwrap_or(1).max(1)),
            6 => Role::Image,
            7 => Role::Meter,
            8 => Role::Group,
            9 => Role::Tree,
            10 => Role::TreeItem,
            11 => Role::Tab,
            12 => Role::TabList,
            _ => Role::None,
        };
        let ticks = f64::from(n.number) / 1000.0;
        let number = match n.number_kind {
            1 => RANGES_BY_PTR
                .with(|r| r.get(h.0 as usize))
                .map(|(min, max)| {
                    // `slider_ticks` maps an empty range to tick 0, which is `min`.
                    if max <= min {
                        min
                    } else {
                        min + ticks * (max - min)
                    }
                }),
            2 => Some(ticks),
            _ => None,
        };
        let found = n.found != 0;
        let snapshot = day_spec::NativeSnapshot {
            found,
            role: if found { role } else { Role::None },
            label: string(n.label),
            value: string(n.value),
            identifier: string(n.identifier),
            // A picker's text comes from its own unit (see `picker::selected_text`).
            text: match crate::picker::selected_text(h.0) {
                Some(picked) => picked,
                None => string(n.text),
            },
            number,
            checked: (n.checked >= 0).then_some(n.checked != 0),
            enabled: Some(n.enabled != 0),
            visible: Some(n.visible != 0),
            frame: (n.frame_ok != 0).then(|| Rect::new(n.x, n.y, n.w, n.h)),
        };
        // SAFETY: `n` holds the strings the shim allocated above, released exactly once.
        unsafe { ffi::day_qt_native_free(&raw mut n) };
        snapshot
    }

    fn announce(&mut self, text: &str, urgent: bool) {
        if self.window.is_null() {
            return;
        }
        // SAFETY: the primary DayWindow lives until the process exits; the text outlives the
        // call, and the shim copies it.
        unsafe { ffi::day_qt_announce(self.window, cstr(text).as_ptr(), c_int::from(urgent)) };
    }

    fn request_frame(
        &mut self,
        host: &QtHandle,
        cb: day_spec::FrameCallback,
    ) -> day_spec::CancelFrame {
        let token = day_core::frame::native::register(cb);
        unsafe { ffi::day_qt_request_frame(host.0, token, run_frame) };
        Box::new(move || {
            day_core::frame::native::cancel(token);
            unsafe { ffi::day_qt_cancel_frame(token) };
        })
    }

    fn replay(&mut self, h: &QtHandle, ops: &[DrawOp], _size: Size) {
        let (nums, texts) = day_spec::encode_ops(ops);
        let joined = cstr(&texts.join("\u{1f}"));
        unsafe {
            ffi::day_qt_canvas_set_ops(h.0, nums.as_ptr(), nums.len() as c_int, joined.as_ptr());
        };
    }

    /// A role from the application palette (docs/color.md "Roles"): `QPalette::Accent` (6.6+,
    /// else `Highlight`), `WindowText` for text (at 0.6, 0.38 and 0.15 for secondary, tertiary
    /// and separators), `PlaceholderText`, `Link`, `Button`, `Window`, `Base` (also for an
    /// elevated surface) and `Highlight` at 0.35 for a selection. The palette has no
    /// destructive, success or warning color: Day's documented pairs answer those.
    fn system_color(&mut self, role: day_spec::SystemColor) -> day_spec::Color {
        let mut rgba = [0.0f64; 4];
        // SAFETY: `rgba` has the four slots the shim writes; nothing else crosses.
        if unsafe { ffi::day_qt_system_color(c_int::from(role.wire()), rgba.as_mut_ptr()) } != 0 {
            day_spec::Color::rgba(rgba[0], rgba[1], rgba[2], rgba[3])
        } else {
            role.fallback(self.dark_mode())
        }
    }

    /// `QStyleHints::setColorScheme` (Qt 6.8+), for this application only: Qt re-polishes
    /// every widget with the scheme's palette, and the palette change re-applies Day's inks.
    fn set_appearance(&mut self, dark: Option<bool>) {
        let mode = match dark {
            None => -1,
            Some(false) => 0,
            Some(true) => 1,
        };
        // SAFETY: a plain request on the application's style hints.
        if unsafe { ffi::day_qt_set_appearance(mode) } == 0 {
            return;
        }
        // Qt has re-polished by now: take its new palette and apply the inks again before
        // anything paints (day-core reads the same palette as soon as this returns). A palette
        // Qt delivers later still reaches `refresh_appearance`.
        day_spec::Palette::set_current(day_spec::Palette {
            dark: self.dark_mode(),
            colors: day_spec::SystemColor::ALL.map(|role| self.system_color(role)),
        });
        reapply_inks();
    }

    fn dark_mode(&mut self) -> bool {
        // Ask Qt, not the `DAY_THEME` default: Qt Widgets follow the system scheme on their
        // own, so the default's "light unless DAY_THEME says otherwise" left app-painted
        // surfaces light while every native control around them rendered dark.
        // SAFETY: a plain palette read; no arguments and no pointers cross the boundary.
        unsafe { ffi::day_qt_dark_mode() != 0 }
    }

    fn toggle_sidebar(&mut self, host: &QtHandle) -> bool {
        crate::toggle_sidebar(host.0)
    }

    /// `QFontDatabase::families()` + `styles()`, decoded from the shim's list text
    /// (docs/fonts.md).
    fn font_families(&mut self) -> Vec<day_spec::FontFamilyInfo> {
        // SAFETY: the shim returns a NUL-terminated heap string (or null); it is read once and
        // released through the shim's own free.
        unsafe {
            let p = ffi::day_qt_font_families();
            if p.is_null() {
                return Vec::new();
            }
            let text = std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned();
            ffi::day_qt_string_free(p);
            day_spec::parse_font_list(&text)
        }
    }

    /// `QFontMetricsF` of the font the canvas draws with (docs/fonts.md).
    fn measure_text(
        &mut self,
        text: &str,
        size: f64,
        font: &day_spec::CanvasFont,
    ) -> Option<day_spec::TextMetrics> {
        let text = cstr(text);
        let family = cstr(font.family_str());
        let mut out = [0.0f64; 8];
        // SAFETY: both strings outlive the call and `out` has the eight slots the shim fills.
        unsafe {
            ffi::day_qt_measure_text(
                text.as_ptr(),
                size,
                c_int::from(font.css_weight()),
                c_int::from(font.italic),
                family.as_ptr(),
                out.as_mut_ptr(),
            );
        };
        Some(day_spec::TextMetrics::from_slots(&out))
    }

    /// Decode bytes with `QImage::loadFromData` (docs/images.md), which reads every format Qt's
    /// image plugins handle. The pixels stay in the shim; only what the decode reported is kept
    /// on this side.
    fn decode_image(&mut self, req: u64, id: day_spec::BitmapId, bytes: &[u8]) {
        let mut out = [0.0f64; 3];
        // SAFETY: the shim reads `len` bytes from the borrowed span and copies what it keeps,
        // and writes exactly three doubles into `out`.
        let ok = unsafe {
            ffi::day_qt_image_decode(id.0, bytes.as_ptr(), bytes.len() as c_int, out.as_mut_ptr())
        };
        let event = if ok != 0 {
            let info = day_spec::BitmapInfo {
                pixels: Size::new(out[0], out[1]),
                // Decoded bytes carry no density — a PNG is simply its pixels.
                scale: 1.0,
                format: day_spec::ImageFormat::sniff(bytes),
                has_alpha: out[2] > 0.5,
            };
            BITMAP_INFO.with(|m| m.borrow_mut().insert(id.0, info));
            Event::ImageDecoded {
                req,
                result: Ok(info),
            }
        } else {
            Event::ImageDecoded {
                req,
                result: Err(day_spec::ImageError::Decode),
            }
        };
        emit(day_spec::WINDOW_NODE, event);
    }

    fn image_info(&mut self, id: day_spec::BitmapId) -> Option<day_spec::BitmapInfo> {
        BITMAP_INFO.with(|m| m.borrow().get(&id.0).copied())
    }

    fn encode_image(&mut self, req: u64, id: day_spec::BitmapId, spec: &day_spec::EncodeSpec) {
        let result = encode_bitmap(id, spec);
        emit(day_spec::WINDOW_NODE, Event::ImageEncoded { req, result });
    }

    /// What `QImage::save` WRITES. Qt reads more than this — GIF and, with the right plugins,
    /// WebP — and the asymmetry is Qt's own, which is why this duty exists rather than letting
    /// `Cap::ImageEncode` imply that everything decodable is also encodable.
    fn encode_formats(&mut self) -> Vec<day_spec::ImageFormat> {
        use day_spec::ImageFormat::{Bmp, Jpeg, Png, Tiff};
        vec![Png, Jpeg, Tiff, Bmp]
    }

    fn release_image(&mut self, id: day_spec::BitmapId) {
        release_bitmap(id);
    }

    /// The canvas widget's painter (`day_paint_ops`) replayed into a premultiplied ARGB32
    /// `QImage` whose device pixel ratio is `scale`, stored in the same registry
    /// `decode_image` fills (docs/canvas.md "Exporting").
    fn render_bitmap(
        &mut self,
        req: u64,
        id: day_spec::BitmapId,
        ops: &[DrawOp],
        size: Size,
        scale: f64,
    ) {
        let (pw, ph) = day_spec::export_pixels(size, scale);
        let (nums, texts) = day_spec::encode_ops(ops);
        let joined = cstr(&texts.join("\u{1f}"));
        let result = match (c_int::try_from(pw), c_int::try_from(ph)) {
            // SAFETY: both op channels outlive the call; the shim copies what it keeps.
            (Ok(w), Ok(h)) => {
                if unsafe {
                    ffi::day_qt_canvas_render_image(
                        id.0,
                        nums.as_ptr(),
                        nums.len() as c_int,
                        joined.as_ptr(),
                        w,
                        h,
                        scale,
                    )
                } == 0
                {
                    Err(day_spec::ExportError::Failed(format!(
                        "QImage could not allocate {pw} × {ph} pixels"
                    )))
                } else {
                    let info = day_spec::BitmapInfo {
                        pixels: Size::new(f64::from(pw), f64::from(ph)),
                        scale,
                        format: None,
                        has_alpha: true,
                    };
                    BITMAP_INFO.with(|m| m.borrow_mut().insert(id.0, info));
                    Ok(info)
                }
            }
            _ => Err(day_spec::ExportError::TooLarge {
                width: pw,
                height: ph,
            }),
        };
        emit(day_spec::WINDOW_NODE, Event::BitmapRendered { req, result });
    }

    /// One `QPdfWriter` document, a page per `PdfPage`, each painted by `day_paint_ops` at
    /// resolution 72 so a unit is a PDF point (docs/canvas.md "Exporting").
    fn render_pdf(&mut self, req: u64, pages: &[day_spec::PdfPage], info: &day_spec::PdfInfo) {
        let result = render_pdf_document(pages, info);
        emit(day_spec::WINDOW_NODE, Event::PdfRendered { req, result });
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

    fn native_window_tabs(&self) -> bool {
        cfg!(target_os = "macos")
    }
    #[cfg(target_os = "macos")]
    fn group_windows(&mut self, hosts: &[QtHandle]) {
        // Core applies window flags after open_window. Qt can recreate its NSWindow
        // while applying those flags, so obtain the native window only after that step.
        for host in hosts {
            if let Some(w) = self.secondary.iter().find(|w| w.content == host.0)
                && let Some(group) = &w.tab_group
            {
                unsafe {
                    ffi::day_qt_window_show(w.win);
                    let view = ffi::day_qt_window_nsview(w.win);
                    let raw = day_platform_macos::window_tabs::window_from_view(view);
                    day_platform_macos::window_tabs::register(raw, w.id, group);
                }
            }
        }
        let ids = hosts
            .iter()
            .filter_map(|h| {
                self.secondary
                    .iter()
                    .find(|w| w.content == h.0)
                    .map(|w| w.id)
            })
            .collect::<Vec<_>>();
        day_platform_macos::window_tabs::group(&ids);
    }
    #[cfg(target_os = "macos")]
    fn window_tab_order(&self, host: &QtHandle) -> Vec<NodeId> {
        self.secondary
            .iter()
            .find(|w| w.content == host.0)
            .map_or_default(|w| day_platform_macos::window_tabs::order(w.id))
    }
    fn native_document_tabs(&self) -> bool {
        true
    }

    fn open_window(
        &mut self,
        id: NodeId,
        options: &WindowOptions,
        kind: day_spec::WindowKind,
    ) -> day_spec::WindowOpenReply<QtHandle> {
        let fixed = c_int::from(kind.is_panel());
        let win = unsafe {
            ffi::day_qt_window_new2(
                cstr(&options.title).as_ptr(),
                options.size.width as c_int,
                options.size.height as c_int,
                id.0,
                fixed,
            )
        };
        apply_open_options(win, options, fixed == 0);
        if !options.start_hidden {
            unsafe { ffi::day_qt_window_show(win) };
        }
        let content = unsafe { ffi::day_qt_window_content(win) };
        self.secondary.push(QtWin {
            #[cfg(target_os = "macos")]
            id,
            #[cfg(target_os = "macos")]
            tab_group: match &options.tabbing {
                day_spec::WindowTabbing::Group(g) | day_spec::WindowTabbing::Preferred(g) => {
                    Some(g.clone())
                }
                _ => None,
            },
            win,
            content,
        });
        day_spec::WindowOpenReply::Open(QtHandle(content))
    }

    fn close_window(&mut self, host: &QtHandle) {
        if let Some(w) = self.secondary.iter().find(|w| w.content == host.0) {
            // closeEvent confirms with WindowClosed; the window HIDES — destruction is
            // anchored to day-core releasing the content handle (`release`, below).
            #[cfg(target_os = "macos")]
            day_platform_macos::window_tabs::unregister(w.id);
            unsafe { ffi::day_qt_window_close(w.win) };
        }
    }

    fn focus_window(&mut self, host: &QtHandle) {
        if let Some(w) = self.secondary.iter().find(|w| w.content == host.0) {
            unsafe { ffi::day_qt_window_raise(w.win) };
        }
    }

    fn fit_window(&mut self, host: &QtHandle, size: Size) {
        // A panel that fits its content (docs/windows.md `size_to_fit`): the shim adds the
        // chrome strip back and moves a fixed-size window's pin; the resize event then reports
        // the new content area to day-core through `relayoutChrome`.
        if let Some(win) = self.qt_window_for(host) {
            // SAFETY: `win` is a live DayWindow (see `apply_window`).
            unsafe { ffi::day_qt_window_fit(win, size.width, size.height) };
        }
    }

    fn set_window_title(&mut self, host: &QtHandle, title: &str) {
        // The primary is an ordinary window (docs/windows.md): `day::window_title` in the first
        // window's shell lands here with the primary's own root, and searching only the
        // secondary list would drop it.
        let win = match self.secondary.iter().find(|w| w.content == host.0) {
            Some(w) => w.win,
            None => self.window,
        };
        if !win.is_null() {
            unsafe { ffi::day_qt_window_set_title(win, cstr(title).as_ptr()) };
        }
    }

    fn apply_window(&mut self, host: &QtHandle, change: &day_spec::WindowChange) {
        use day_spec::WindowChange as C;
        // A secondary window by its content, or the primary, which `ready` handed out as the
        // DayWindow itself.
        let Some(win) = self.qt_window_for(host) else {
            return;
        };
        match change {
            // SAFETY: `win` is a live DayWindow: the primary lives until the process exits, and
            // a secondary stays in `self.secondary` until day-core releases its content.
            C::State(state) => unsafe { ffi::day_qt_window_set_state(win, state.code()) },
            // No Linux display server (X11, Wayland) lets a client keep its surface out of
            // screenshots or screen sharing, so `Cap::ContentProtection` is Unsupported and the
            // request is ignored. (Qt has no portable switch for it on macOS or Windows either.)
            C::ContentProtected(_) => {}
            other => apply_property(win, other),
        }
    }

    fn window_frame(&self, host: &QtHandle) -> Option<day_spec::Rect> {
        let win = self.qt_window_for(host)?;
        let mut out = [0.0f64; 4];
        // SAFETY: `win` is a live DayWindow (see `apply_window`); `out` holds the four doubles
        // the shim writes.
        let known = unsafe { ffi::day_qt_window_frame(win, out.as_mut_ptr()) } != 0;
        known.then(|| day_spec::Rect::new(out[0], out[1], out[2], out[3]))
    }

    fn monitors(&self) -> Vec<day_spec::Monitor> {
        qt_monitors()
    }

    fn set_drag_region(&mut self, h: &QtHandle, drag: bool) {
        // SAFETY: `h` is a live widget Day realized.
        unsafe { ffi::day_qt_set_drag_region(h.0, c_int::from(drag)) };
    }

    fn set_status_items(&mut self, items: &[day_spec::StatusItemSpec]) {
        self.trays.retain(|(spec, tray)| {
            let keep = items.iter().any(|i| i.id == spec.id);
            if !keep {
                // SAFETY: a tray this backend made and has not deleted.
                unsafe { ffi::day_qt_tray_delete(*tray) };
            }
            keep
        });
        for spec in items {
            let tray = match self.trays.iter_mut().find(|(s, _)| s.id == spec.id) {
                Some((s, _)) if s == spec => continue,
                Some((s, tray)) => {
                    *s = spec.clone();
                    *tray
                }
                None => {
                    // SAFETY: a constructor; the QApplication exists by the time any duty runs.
                    let tray = unsafe { ffi::day_qt_tray_new() };
                    self.trays.push((spec.clone(), tray));
                    tray
                }
            };
            update_tray(tray, spec);
        }
    }

    fn set_app_progress(&mut self, progress: day_spec::AppProgress) {
        #[cfg(target_os = "linux")]
        linux::set_progress(progress);
        #[cfg(not(target_os = "linux"))]
        let _ = progress;
    }

    fn set_app_badge(&mut self, badge: &day_spec::AppBadge) {
        #[cfg(target_os = "linux")]
        linux::set_badge(badge);
        #[cfg(not(target_os = "linux"))]
        let _ = badge;
    }

    fn set_keep_running(&mut self, keep: bool) {
        // SAFETY: a flag write.
        unsafe { ffi::day_qt_set_keep_running(c_int::from(keep)) };
    }

    fn quit_app(&mut self) {
        // SAFETY: called on the UI thread with the QApplication running.
        unsafe { ffi::day_qt_quit() };
    }
}

/// The chrome, size limits and placement a window opens with, applied before it is first shown
/// (docs/window-chrome.md). `resizable` is false for a Preferences window, which keeps its
/// fixed size whatever the options say.
fn apply_open_options(win: *mut c_void, options: &WindowOptions, resizable: bool) {
    let frameless = options.chrome == day_spec::WindowChrome::Frameless;
    let transparent = options.background == day_spec::WindowBackground::Transparent;
    let resizable = resizable && options.resizable;
    // SAFETY (every call below): `win` is the DayWindow just created, not yet shown.
    unsafe {
        if options.min_size.is_some() || options.max_size.is_some() {
            let min = options.min_size.unwrap_or(Size::new(-1.0, -1.0));
            let max = options.max_size.unwrap_or(Size::new(-1.0, -1.0));
            ffi::day_qt_window_set_limits(win, min.width, min.height, max.width, max.height);
        }
        ffi::day_qt_window_set_chrome(
            win,
            c_int::from(frameless),
            c_int::from(transparent),
            c_int::from(options.shadow),
            c_int::from(resizable),
        );
        match options.placement {
            day_spec::WindowPlacement::Automatic => {}
            day_spec::WindowPlacement::Centered => ffi::day_qt_window_place(win, 1, 0.0, 0.0),
            day_spec::WindowPlacement::At(p) => ffi::day_qt_window_place(win, 2, p.x, p.y),
        }
    }
}

/// Every [`day_spec::WindowChange`] but the display state and content protection
/// (docs/windows.md "Window properties"). Per-window appearance is not Qt's to give (its palette
/// and color scheme are app-wide), and every workspace has no Qt API; both are ignored.
fn apply_property(win: *mut c_void, change: &day_spec::WindowChange) {
    use day_spec::WindowChange as C;
    let switch = |which: c_int, on: bool| {
        // SAFETY: `win` is a live DayWindow (see `apply_window`).
        unsafe { ffi::day_qt_window_set_switch(win, which, c_int::from(on)) }
    };
    // SAFETY (the calls below): as above.
    match change {
        C::Frame { origin, size } => unsafe {
            ffi::day_qt_window_set_frame(
                win,
                c_int::from(origin.is_some()),
                origin.map_or(0.0, |p| p.x),
                origin.map_or(0.0, |p| p.y),
                c_int::from(size.is_some()),
                size.map_or(0.0, |s| s.width),
                size.map_or(0.0, |s| s.height),
            );
        },
        C::Limits { min, max } => unsafe {
            let min = min.unwrap_or(Size::new(-1.0, -1.0));
            let max = max.unwrap_or(Size::new(-1.0, -1.0));
            ffi::day_qt_window_set_limits(win, min.width, min.height, max.width, max.height);
        },
        C::Level(level) => switch(0, *level == day_spec::WindowLevel::Floating),
        C::SkipTaskbar(skip) => switch(1, *skip),
        C::Minimizable(on) => switch(2, *on),
        C::Maximizable(on) => switch(3, *on),
        C::Closable(on) => switch(4, *on),
        C::Resizable(on) => switch(5, *on),
        C::Visible(on) => switch(6, *on),
        C::RequestAttention(a) => unsafe {
            ffi::day_qt_window_alert(win, c_int::from(*a == day_spec::Attention::Critical));
        },
        _ => {}
    }
}

/// Every attached display, from `QGuiApplication::screens()`: geometry and available geometry
/// in device-independent pixels (Day's points), the device pixel ratio, Qt's primary screen.
fn qt_monitors() -> Vec<day_spec::Monitor> {
    // SAFETY: a read of the QGuiApplication's screen list on the UI thread.
    let n = unsafe { ffi::day_qt_screen_count() };
    (0..n)
        .filter_map(|i| {
            let mut out = [0.0f64; 9];
            let mut name = [0 as std::os::raw::c_char; 256];
            let mut id = [0 as std::os::raw::c_char; 128];
            // SAFETY: the buffers are as long as the lengths passed, and the shim
            // NUL-terminates within them.
            let primary = unsafe {
                ffi::day_qt_screen_info(
                    i,
                    out.as_mut_ptr(),
                    name.as_mut_ptr(),
                    name.len() as c_int,
                    id.as_mut_ptr(),
                    id.len() as c_int,
                )
            };
            if primary < 0 {
                return None;
            }
            // SAFETY: NUL-terminated by the shim (above).
            let text = |b: &[std::os::raw::c_char]| unsafe {
                std::ffi::CStr::from_ptr(b.as_ptr())
                    .to_string_lossy()
                    .into_owned()
            };
            Some(day_spec::Monitor {
                id: text(&id),
                name: text(&name),
                frame: day_spec::Rect::new(out[0], out[1], out[2], out[3]),
                work_area: day_spec::Rect::new(out[4], out[5], out[6], out[7]),
                scale: out[8],
                primary: primary == 1,
            })
        })
        .collect()
}

/// Apply a status item's spec to its `QSystemTrayIcon`: icon, hover text, menu, click action.
fn update_tray(tray: *mut c_void, spec: &day_spec::StatusItemSpec) {
    let (icon, fallback) = crate::toolbar::icon_args(spec.icon.as_ref());
    let tip = if spec.tooltip.is_empty() {
        spec.title.as_str()
    } else {
        spec.tooltip.as_str()
    };
    // SAFETY: a fresh QMenu, filled by the same builder the app menu uses; the tray takes it.
    let menu = unsafe { ffi::day_qt_menu_new() };
    build_qt_menu(menu, &spec.menu);
    let mask = spec.template || matches!(spec.icon, Some(day_spec::Icon::Symbol(_)));
    // SAFETY: `tray` is live (see `set_status_items`); the strings outlive the call.
    unsafe {
        ffi::day_qt_tray_update(
            tray,
            cstr(&icon).as_ptr(),
            fallback,
            c_int::from(mask),
            cstr(tip).as_ptr(),
            menu,
            spec.activate,
        );
    };
    log::debug!("qt: status item {} shown", spec.id);
}

/// Grab any widget to PNG through the shim (the dayscript screenshot seam).
fn snapshot_qt_widget(widget: *mut c_void) -> Result<Vec<u8>, String> {
    let path = std::env::temp_dir().join(format!("day-qt-snap-{}.png", std::process::id()));
    let cpath = cstr(path.to_str().unwrap_or("/tmp/day-qt-snap.png"));
    // A scripted run's stated capture scale (Day.toml `[screenshots]`), else the widget's own.
    let scale = day_spec::capture_scale().unwrap_or(0.0);
    let rc = unsafe { ffi::day_qt_snapshot_png(widget, cpath.as_ptr(), scale) };
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
    let f: Box<Box<dyn FnOnce() + Send>> = unsafe { Box::from_raw(data.cast()) };
    ffi_guard::contain((), f);
}

/// A window update opportunity delivered by Qt on the application thread.
extern "C" fn run_frame(token: u64, timestamp: f64) {
    day_core::frame::native::deliver(token, day_spec::FrameStamp::new(timestamp));
}

impl Platform for Qt {
    const TARGET: &'static str = if cfg!(target_os = "macos") {
        "macos-qt"
    } else {
        "linux-qt"
    };
    const TOOLKIT: &'static str = "qt";

    fn run(mut self, options: WindowOptions, ready: Box<dyn FnOnce(Self, QtHandle, Size)>) {
        // One process per app id on Linux: a second launch forwards its arguments and exits
        // before it opens anything. Held until `run` returns, which is the app's end.
        #[cfg(target_os = "linux")]
        let _instance = linux::claim_instance();
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
            // The desktop's text size (docs/text.md "Text scale") follows the screen's DPI.
            ffi::day_qt_on_text_scale(on_text_scale);
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
            ffi::day_qt_set_text_keys(on_text_key_query, on_text_key);
            ffi::day_qt_set_touches_cb(on_touches);
            ffi::day_qt_install_input_router(on_pointer_kind);
            ffi::day_qt_set_toolbar_cb(toolbar::on_toolbar_value);
            ffi::day_qt_set_lifecycle_cb(on_lifecycle);
            // A palette or color scheme change re-applies every appearance-dependent ink.
            ffi::day_qt_on_appearance(on_appearance);
            ready(self, QtHandle(window), options.size);
            ffi::day_qt_window_on_resize(window, window_resized);
            ffi::day_qt_set_window_events_cb(win_resized, win_closed, win_focused);
            ffi::day_qt_set_window_state_cb(win_state);
            apply_open_options(window, &options, true);
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
        let data = Box::into_raw(Box::new(f)).cast::<c_void>();
        unsafe { ffi::day_qt_post(run_posted, data) };
    }

    fn post_delayed(ms: u32, f: Box<dyn FnOnce() + Send>) {
        // The timer door behind `day::sleep` (docs/async.md): `QTimer::singleShot` on the
        // application object, which Qt runs in the application thread wherever it was armed.
        // No helper thread. Qt's delay is an `int`; a longer one runs as a chain of timers,
        // so the whole delay is honored.
        const STEP: u32 = c_int::MAX as u32;
        if ms > STEP {
            Self::post_delayed(STEP, Box::new(move || Self::post_delayed(ms - STEP, f)));
            return;
        }
        let data = Box::into_raw(Box::new(f)).cast::<c_void>();
        unsafe { ffi::day_qt_post_delayed(ms as c_int, run_posted, data) };
    }
}

// Bitmap handles in reactive state can outlive the toolkit's thread-local registry at Quit.
// Keep normal releases strict about borrow conflicts, but tolerate a destroyed registry.
fn release_bitmap(id: day_spec::BitmapId) {
    let _ = BITMAP_INFO.try_with(|m| {
        m.borrow_mut().remove(&id.0);
        // SAFETY: release a known or already-absent id while toolkit TLS is still alive.
        // Do not call into the C++ registry once toolkit teardown has begun.
        unsafe { ffi::day_qt_image_release(id.0) };
    });
}

/// Qt's macOS `QFileOpenEvent`; command-line activations on other OSes use day-core.
///
/// # Safety
/// `url` is null or a NUL-terminated C string that stays readable for the length of this call.
/// The shim passes the event's URL, which Qt owns and outlives the callback; nothing here keeps
/// the pointer past the copy.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn day_qt_open_file(url: *const std::ffi::c_char) {
    day_spec::ffi_guard::contain((), || {
        if !url.is_null() {
            let file = unsafe { std::ffi::CStr::from_ptr(url) }
                .to_string_lossy()
                .into_owned();
            day_core::request_open_files(vec![file]);
        }
    });
}

#[cfg(test)]
mod bitmap_teardown_tests {
    use super::*;

    #[test]
    fn releasing_during_bitmap_registry_teardown_does_not_abort() {
        const CHILD: &str = "DAY_QT_BITMAP_TEARDOWN_CHILD";
        if std::env::var_os(CHILD).is_some() {
            struct LateRelease(std::sync::mpsc::Sender<()>);
            impl Drop for LateRelease {
                fn drop(&mut self) {
                    assert!(BITMAP_INFO.try_with(|_| ()).is_err());
                    release_bitmap(day_spec::BitmapId(1));
                    self.0.send(()).unwrap();
                }
            }
            let (sent, received) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let held = LateRelease(sent);
                // The sink and bitmap registry share one TLS group. Dropping its captures
                // exercises unavailable TLS without relying on ordering of unrelated keys.
                SINK.with(|sink| {
                    *sink.borrow_mut() = Some(Rc::new(move |_, _| {
                        let _ = &held;
                    }));
                });
            })
            .join()
            .unwrap();
            received
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "bitmap_teardown_tests::releasing_during_bitmap_registry_teardown_does_not_abort",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(test)]
mod input_tests {
    use super::{contacts, key_phase, pointer_kind};
    use day_spec::{KeyPhase, Point, PointerKind, TouchPhase};

    #[test]
    fn contact_numbers_decode_in_order() {
        let numbers = [
            7.0, 0.0, 10.0, 20.0, 0.5, 0.0, // a finger going down at half force
            9.0, 2.0, 1.0, 2.0, 1.0, 2.0, // a pen held still
            0.0, 4.0, 3.0, 4.0, 1.0, 1.0, // the mouse, cancelled
            1.0, 3.0, // an incomplete trailing record is ignored
        ];
        let touches = contacts(&numbers);
        assert_eq!(touches.len(), 3);
        assert_eq!(touches[0].id, 7);
        assert_eq!(touches[0].phase, TouchPhase::Began);
        assert_eq!(touches[0].location, Point::new(10.0, 20.0));
        assert!((touches[0].pressure - 0.5).abs() < f64::EPSILON);
        assert_eq!(touches[0].kind, PointerKind::Touch);
        assert_eq!(touches[1].phase, TouchPhase::Stationary);
        assert_eq!(touches[1].kind, PointerKind::Stylus);
        assert_eq!(touches[2].phase, TouchPhase::Cancelled);
        assert_eq!(touches[2].kind, PointerKind::Mouse);
    }

    #[test]
    fn wire_codes_name_phases_and_devices() {
        assert_eq!(key_phase(0), KeyPhase::Down);
        assert_eq!(key_phase(1), KeyPhase::Repeat);
        assert_eq!(key_phase(2), KeyPhase::Up);
        assert_eq!(pointer_kind(0), PointerKind::Touch);
        assert_eq!(pointer_kind(1), PointerKind::Mouse);
        assert_eq!(pointer_kind(2), PointerKind::Stylus);
    }
}

#[cfg(test)]
mod surface_tests {
    use super::{dynamic, is_styled_surface, surface_is_dynamic, surface_spec};
    use day_spec::props::ContainerProps;
    use day_spec::{Border, Color, Corners, LinearGradient, Shadow, UnitPoint};

    /// A plain colored, rounded surface keeps the stylesheet path; each new field leaves it.
    #[test]
    fn only_new_fields_take_the_painted_path() {
        let plain = ContainerProps {
            fill: Some(Color::rgb(1.0, 0.0, 0.0).into()),
            corner_radius: 8.0,
            clips: true,
            ..ContainerProps::default()
        };
        assert!(!is_styled_surface(&plain));
        // A corner choice without a radius rounds nothing, so it changes nothing.
        assert!(!is_styled_surface(&ContainerProps {
            corner_radius: 0.0,
            corners: Corners::TOP,
            ..plain.clone()
        }));
        assert!(is_styled_surface(&ContainerProps {
            corners: Corners::TOP,
            ..plain.clone()
        }));
        assert!(is_styled_surface(&ContainerProps {
            border: Some(Border::new(Color::WHITE, 1.0)),
            ..plain.clone()
        }));
        assert!(is_styled_surface(&ContainerProps {
            shadow: Some(Shadow::new(Color::WHITE, 4.0)),
            ..plain.clone()
        }));
        assert!(is_styled_surface(&ContainerProps {
            fill: Some(LinearGradient::vertical(Color::WHITE, Color::WHITE).into()),
            ..plain.clone()
        }));
        // Every gradient kind paints; a solid fill keeps the stylesheet.
        assert!(is_styled_surface(&ContainerProps {
            fill: Some(
                day_spec::RadialGradient::new(UnitPoint::CENTER, 0.5, [(0.0, Color::WHITE)]).into()
            ),
            ..plain.clone()
        }));
        assert!(is_styled_surface(&ContainerProps {
            fill: Some(
                day_spec::ConicGradient::centered(day_spec::Angle::ZERO, [(0.0, Color::WHITE)])
                    .into()
            ),
            ..plain
        }));
    }

    /// A radial fill sends its center and radius, a conic one its center and start angle in
    /// degrees; stops go resolved, in order.
    #[test]
    fn radial_and_conic_fills_reach_the_shim() {
        let red = Color::rgb(1.0, 0.0, 0.0);
        let blue = Color::rgb(0.0, 0.0, 1.0);
        let radial = ContainerProps {
            fill: Some(
                day_spec::RadialGradient::new(
                    UnitPoint::new(0.25, 0.75),
                    0.4,
                    [(0.0, red), (1.0, blue)],
                )
                .into(),
            ),
            ..ContainerProps::default()
        };
        let (spec, stops) = surface_spec(&radial);
        assert_eq!(spec[19..24], [2.0, 0.25, 0.75, 0.4, 0.0]);
        assert_eq!(stops, [0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0]);
        let conic = ContainerProps {
            fill: Some(
                day_spec::ConicGradient::new(
                    UnitPoint::CENTER,
                    day_spec::Angle::rad(std::f64::consts::FRAC_PI_2),
                    [(0.0, red), (0.5, blue)],
                )
                .into(),
            ),
            ..ContainerProps::default()
        };
        let (spec, stops) = surface_spec(&conic);
        assert_eq!(spec[19..22], [3.0, 0.5, 0.5]);
        assert!((spec[22] - 90.0).abs() < 1e-9);
        assert_eq!(stops.len(), 10);
        assert_eq!(spec[0..4], [0.0, 0.0, 0.0, 0.0]);
    }

    /// Only an ink that depends on the appearance is kept for a switch.
    #[test]
    fn a_surface_follows_the_appearance_only_with_a_dynamic_ink() {
        let fixed = ContainerProps {
            fill: Some(Color::WHITE.into()),
            border: Some(Border::new(Color::WHITE, 1.0)),
            ..ContainerProps::default()
        };
        assert!(!surface_is_dynamic(&fixed));
        let pair = day_spec::Ink::adaptive(Color::WHITE, Color::BLACK);
        assert!(surface_is_dynamic(&ContainerProps {
            fill: Some(pair.into()),
            ..fixed.clone()
        }));
        assert!(surface_is_dynamic(&ContainerProps {
            border: Some(Border::new(day_spec::SystemColor::Separator, 1.0)),
            ..fixed.clone()
        }));
        assert!(surface_is_dynamic(&ContainerProps {
            fill: Some(LinearGradient::vertical(Color::WHITE, pair).into()),
            ..fixed
        }));
        assert!(dynamic(Some(Color::WHITE.into())).is_none());
        assert_eq!(dynamic(Some(pair)), Some(pair));
    }

    /// The spec's slots are the ones shim.cpp's `day_qt_widget_set_styled_surface` reads.
    #[test]
    fn spec_slots_match_the_shim() {
        // A fill is a color or a gradient, never both: the gradient leaves the color slots clear.
        let props = ContainerProps {
            fill: Some(
                LinearGradient::vertical(Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0))
                    .into(),
            ),
            corner_radius: 12.0,
            corners: Corners::TOP_LEFT | Corners::BOTTOM_RIGHT,
            border: Some(Border::new(Color::rgba(0.5, 0.6, 0.7, 0.8), 2.0)),
            shadow: Some(Shadow::new(Color::rgba(0.0, 0.0, 0.0, 0.25), 16.0).offset(1.0, 6.0)),
            clips: false,
            role: None,
        };
        let (spec, stops) = surface_spec(&props);
        assert_eq!(spec[0..4], [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(spec[4..7], [12.0, 5.0, 0.0]);
        assert_eq!(spec[7..12], [0.5, 0.6, 0.7, 0.8, 2.0]);
        assert_eq!(spec[12..19], [0.0, 0.0, 0.0, 0.25, 16.0, 1.0, 6.0]);
        let Some(day_spec::Paint::Linear(g)) = &props.fill else {
            unreachable!("the gradient set above")
        };
        assert_eq!(spec[19..24], [1.0, g.start.x, g.start.y, g.end.x, g.end.y]);
        assert_eq!(stops.len(), 10);
        assert_eq!(stops[0..5], [0.0, 1.0, 0.0, 0.0, 1.0]);
        assert_eq!(stops[5..10], [1.0, 0.0, 0.0, 1.0, 1.0]);

        // Nothing set: every slot zero but the corner bits, and no stops.
        let (spec, stops) = surface_spec(&ContainerProps::default());
        assert_eq!(stops, [] as [f64; 0]);
        assert!(
            spec.iter()
                .enumerate()
                .all(|(i, v)| (i == 5) == (*v != 0.0))
        );
        assert_eq!(spec[5], 15.0);
    }
}

#[cfg(test)]
mod list_rows_tests {
    use super::{RowWork, row_align_code, supplement_rows};
    use day_spec::props::RowDelta;
    use std::os::raw::c_int;

    /// One turn's patches fold into one flush: deltas stay in order, and a reload on either
    /// side of them wins.
    #[test]
    fn row_work_merges_in_arrival_order() {
        let a = RowWork::Splice(vec![RowDelta::Insert(0)]);
        let b = RowWork::Splice(vec![RowDelta::Move(3, 0), RowDelta::Remove(2)]);
        assert_eq!(
            RowWork::merge(Some(a), b),
            RowWork::Splice(vec![
                RowDelta::Insert(0),
                RowDelta::Move(3, 0),
                RowDelta::Remove(2)
            ])
        );
        let splice = || RowWork::Splice(vec![RowDelta::Insert(1)]);
        assert_eq!(RowWork::merge(None, splice()), splice());
        assert_eq!(
            RowWork::merge(Some(RowWork::Reload), splice()),
            RowWork::Reload
        );
        assert_eq!(
            RowWork::merge(Some(splice()), RowWork::Reload),
            RowWork::Reload
        );
        assert_eq!(RowWork::merge(None, RowWork::Reload), RowWork::Reload);
    }

    /// The shim's triples: kind 0 insert, 1 remove, 2 move, the move's target last.
    #[test]
    fn row_work_triples() {
        let deltas = [
            RowDelta::Insert(4),
            RowDelta::Remove(1),
            RowDelta::Move(5, 2),
        ];
        assert_eq!(RowWork::triples(&deltas), vec![0, 4, 0, 1, 1, 0, 2, 5, 2]);
        assert_eq!(RowWork::triples(&[]), Vec::<c_int>::new());
    }

    /// The supplementary rows lead and trail the items; the walk finds both runs, an all-
    /// supplementary list (header, empty state, footer) whole, and none in a plain list.
    #[test]
    fn supplement_rows_at_both_ends() {
        // [top padding] [header] items 2..8 [footer] [bottom padding]
        let rows = supplement_rows(10, &|i| !(2..8).contains(&i));
        assert_eq!(rows, vec![0, 1, 8, 9]);
        assert_eq!(supplement_rows(3, &|_| true), vec![0, 1, 2]);
        assert_eq!(supplement_rows(5, &|_| false), Vec::<usize>::new());
        assert_eq!(supplement_rows(0, &|_| true), Vec::<usize>::new());
        // Only a trailing footer.
        assert_eq!(supplement_rows(4, &|i| i == 3), vec![3]);
        // The walk never asks about the items between the runs.
        let asked = std::cell::Cell::new(0);
        let rows = supplement_rows(1_000_000, &|i| {
            asked.set(asked.get() + 1);
            i == 0 || i == 999_999
        });
        assert_eq!(rows, vec![0, 999_999]);
        assert!(asked.get() <= 4);
    }

    #[test]
    fn row_align_codes_match_the_shim() {
        use day_spec::RowAlign;
        assert_eq!(row_align_code(RowAlign::Nearest), 0);
        assert_eq!(row_align_code(RowAlign::Start), 1);
        assert_eq!(row_align_code(RowAlign::Center), 2);
        assert_eq!(row_align_code(RowAlign::End), 3);
    }
}

#[cfg(test)]
mod cell_pool_tests {
    use super::{CellPool, LIST_OVERSCAN};

    /// Drive the pool through a band the way `list_sync` does: park what left, bind what
    /// entered, spares first, new cells (fresh keys from `next`) only when none is spare.
    fn sync(pool: &mut CellPool, band: std::ops::Range<usize>, next: &mut usize) -> Vec<usize> {
        let (parked, need) = pool.retarget(band);
        for row in need {
            let cell = pool.take_free(false).unwrap_or_else(|| {
                *next += 1;
                *next
            });
            pool.bind(row, cell);
        }
        parked
    }

    /// `sync` for a sectioned list whose header rows are the multiples of `every`.
    fn sync_sectioned(
        pool: &mut CellPool,
        band: std::ops::Range<usize>,
        every: usize,
        next: &mut usize,
    ) -> (Vec<usize>, usize) {
        let (parked, need) = pool.retarget(band);
        let mut flips = 0;
        for row in need {
            let header = row % every == 0;
            let cell = pool.take_free(header).unwrap_or_else(|| {
                *next += 1;
                *next
            });
            if *next != cell && pool.headers.contains(&cell) != header {
                flips += 1;
            }
            pool.bind(row, cell);
            pool.set_kind(cell, header);
        }
        (parked, flips)
    }

    #[test]
    fn a_scroll_reuses_the_cells_that_left() {
        let mut pool = CellPool::default();
        let mut next = 0;
        assert_eq!(sync(&mut pool, 0..10, &mut next), [] as [usize; 0]);
        assert_eq!(pool.len(), 10);
        // Five rows scroll out at the top, five come in at the bottom: no new cell.
        let parked = sync(&mut pool, 5..15, &mut next);
        assert_eq!(parked.len(), 5);
        assert_eq!(pool.len(), 10);
        assert_eq!(next, 10);
        for row in 5..15 {
            assert!(pool.bound.contains_key(&row), "row {row} has a cell");
        }
        for cell in parked {
            assert!(
                pool.row_of(cell).is_some(),
                "the parked cell was re-bound to a new row"
            );
        }
    }

    #[test]
    fn the_pool_stays_a_window_wide_across_a_long_scroll() {
        let mut pool = CellPool::default();
        let mut next = 0;
        let window = 12 + 2 * LIST_OVERSCAN;
        let mut top = 0usize;
        while top + window < 100_000 {
            sync(&mut pool, top..top + window, &mut next);
            assert!(pool.len() <= window, "pool grew to {} cells", pool.len());
            top += 7;
        }
        // A far jump reuses the same cells rather than allocating a second set.
        sync(&mut pool, 90_000..90_000 + window, &mut next);
        assert_eq!(pool.len(), window);
        assert_eq!(next, window);
    }

    #[test]
    fn a_shrunk_band_parks_the_rest_and_keeps_them_spare() {
        let mut pool = CellPool::default();
        let mut next = 0;
        sync(&mut pool, 0..20, &mut next);
        let parked = sync(&mut pool, 0..5, &mut next);
        assert_eq!(parked.len(), 15);
        assert_eq!(pool.bound.len(), 5);
        assert_eq!(pool.free.len(), 15);
        for cell in &parked {
            assert_eq!(pool.row_of(*cell), None);
        }
        // Growing again takes the spares, newest first, and allocates nothing.
        sync(&mut pool, 0..20, &mut next);
        assert_eq!(pool.free.len(), 0);
        assert_eq!(next, 20);
    }

    #[test]
    fn header_rows_take_header_cells_and_item_rows_item_cells() {
        let mut pool = CellPool::default();
        let mut next = 0;
        let every = 10;
        let window = 12 + 2 * LIST_OVERSCAN;
        // A window holds at most this many headers, and the pool at most that many more cells
        // than a window: each kind keeps its own spares.
        let headers = window.div_ceil(every) + 1;
        let mut top = 0usize;
        while top + window < 10_000 {
            let (_, flips) = sync_sectioned(&mut pool, top..top + window, every, &mut next);
            assert_eq!(flips, 0, "a cell changed kind at top {top}");
            assert!(
                pool.len() <= window + headers,
                "pool grew to {} cells",
                pool.len()
            );
            top += 3;
        }
        // A far jump reuses the same cells.
        let before = next;
        sync_sectioned(&mut pool, 9_000..9_000 + window, every, &mut next);
        assert_eq!(next, before);
    }

    #[test]
    fn retarget_is_idempotent() {
        let mut pool = CellPool::default();
        let mut next = 0;
        sync(&mut pool, 3..9, &mut next);
        let before: Vec<(usize, usize)> = pool.bound.iter().map(|(r, c)| (*r, *c)).collect();
        let parked = sync(&mut pool, 3..9, &mut next);
        assert_eq!(parked, [] as [usize; 0]);
        let after: Vec<(usize, usize)> = pool.bound.iter().map(|(r, c)| (*r, *c)).collect();
        assert_eq!(before, after);
    }
}

#[cfg(test)]
mod text_tests {
    use super::{css_hex, font_params, label_align, qt_label_markup};
    use day_spec::props::TextAlign;
    use day_spec::{Color, Decoration, DecorationStyle, Font, FontSpec, MarkupDialect, TextRun};

    fn run(range: std::ops::Range<usize>) -> TextRun {
        TextRun {
            range,
            ..TextRun::default()
        }
    }

    #[test]
    fn runs_without_a_styled_underline_are_the_shared_markup() {
        let text = "a < b and c & d\nnext";
        let mut bold = run(2..5);
        bold.font.weight = Some(day_spec::FontWeight::Bold);
        let mut under = run(10..11);
        under.underline = Some(Decoration::SINGLE.color(Color::rgb(1.0, 0.0, 0.0)));
        let runs = [bold, under];
        assert_eq!(
            qt_label_markup(text, &runs, 13.0),
            day_spec::runs_to_markup(text, &runs, MarkupDialect::QtHtml, 13.0)
        );
    }

    #[test]
    fn a_styled_underline_travels_as_qt_css_around_the_run() {
        let text = "Check unknown now";
        let mut wavy = run(6..13);
        wavy.underline = Some(Decoration::wavy().color(Color::rgb(1.0, 0.0, 0.0)));
        let html = qt_label_markup(text, &[wavy], 13.0);
        assert_eq!(
            html,
            "Check <span style=\"text-underline-style:wave;text-decoration-color:#ff0000;\">\
             unknown</span> now"
        );
        let mut dotted = run(0..5);
        dotted.underline = Some(Decoration::dotted());
        let mut double = run(6..13);
        double.underline = Some(Decoration::double());
        let html = qt_label_markup(text, &[dotted, double], 13.0);
        assert!(html.starts_with("<span style=\"text-underline-style:dotted;\">Check</span> "));
        assert!(html.contains("<span style=\"text-underline-style:solid;\">unknown</span>"));
        assert!(!html.contains("<u>"));
    }

    #[test]
    fn a_strikethrough_color_is_dropped_and_the_line_kept() {
        let mut struck = run(0..6);
        struck.strikethrough =
            Some(Decoration::new(DecorationStyle::Double).color(Color::rgb(1.0, 0.0, 0.0)));
        let html = qt_label_markup("Struck", &[struck], 13.0);
        assert_eq!(html, "<s>Struck</s>");
        assert_eq!(css_hex(Color::rgb(0.0, 0.5, 1.0)), "#007fff");
    }

    #[test]
    fn a_run_asks_for_no_synthesized_small_caps() {
        let mut caps = run(0..4);
        caps.font.small_caps = true;
        assert_eq!(qt_label_markup("Caps", &[caps], 13.0), "Caps");
    }

    #[test]
    fn a_run_that_misses_the_text_is_skipped() {
        let html = qt_label_markup("short", &[run(2..40)], 13.0);
        assert_eq!(html, "short");
    }

    #[test]
    fn the_font_carries_scale_and_features() {
        let spec = FontSpec {
            style: Font::System(20.0),
            scale: 1.5,
            tabular: true,
            small_caps: true,
            oldstyle_figures: true,
            ..FontSpec::default()
        };
        let f = font_params(spec);
        assert!((f.pt - 30.0).abs() < 1e-9);
        assert_eq!((f.tabular, f.small_caps, f.oldstyle), (1, 1, 1));
        assert_eq!(label_align(TextAlign::Justified), 3);
        assert_eq!(label_align(TextAlign::Trailing), 2);
    }
}

#[cfg(test)]
mod anim_tests {
    use super::*;

    #[test]
    fn a_spec_lowers_to_its_delay_cycles_and_direction() {
        let run = QtAnim::new(&AnimSpec::linear(600).delay(900));
        assert_eq!((run.delay_ms, run.duration_ms, run.cycles), (900, 600, 1));
        assert!(!run.autoreverse);
        // A linear cycle needs only its two ends: Qt interpolates between key values linearly.
        assert_eq!(run.keys, vec![0.0, 1.0]);
        let run = QtAnim::new(&AnimSpec::linear(800).repeat(2, true));
        assert_eq!((run.cycles, run.autoreverse), (3, true));
        let run = QtAnim::new(&AnimSpec::linear(600).repeat_forever(true));
        assert_eq!(run.cycles, -1);
    }

    #[test]
    fn a_spring_carries_its_overshooting_samples() {
        let spec = AnimSpec::spring(0.9, 0.25);
        let run = QtAnim::new(&spec);
        assert_eq!(run.keys.len(), spec.keyframes().len());
        assert_eq!(run.keys.first().copied(), Some(0.0));
        assert_eq!(run.keys.last().copied(), Some(1.0));
        // The loose spring passes its target by more than a third on the first swing.
        assert!(run.keys.iter().copied().fold(0.0, f64::max) > 1.35);
    }

    #[test]
    fn an_easing_curve_is_sampled_exactly() {
        let spec = AnimSpec::ease_in_out(1000);
        let run = QtAnim::new(&spec);
        let n = run.keys.len() - 1;
        let mid = run.keys[n / 2];
        // Day's EaseInOut is smoothstep, which Qt's InOutQuad is not away from the middle.
        assert!((mid - spec.cycle_progress(0.5)).abs() < 1e-12);
        let quarter = run.keys[n / 4];
        let t = (n / 4) as f64 / n as f64;
        assert!((quarter - t * t * (3.0 - 2.0 * t)).abs() < 1e-12);
    }

    #[test]
    fn the_shim_sees_the_samples_and_none_is_null() {
        let spec = AnimSpec::spring(0.4, 0.8).delay(50);
        let run = QtAnim::new(&spec);
        let raw = run.raw();
        assert_eq!(raw.n_keys as usize, run.keys.len());
        assert_eq!(raw.keys, run.keys.as_ptr());
        assert_eq!(raw.delay_ms, 50);
        assert!(with_qt_anim(None, <*const ffi::DayQtAnim>::is_null));
        assert!(!with_qt_anim(Some(&spec), <*const ffi::DayQtAnim>::is_null));
    }
}
