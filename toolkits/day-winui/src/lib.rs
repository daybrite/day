//! day-winui — the Windows backend (target `windows-winui`; DESIGN.md §1, §9), over the
//! day-winui-sys C++/WinRT XAML-Islands shim. `Handle = WinHandle(*mut UIElement)`; every Day
//! node is a real `Windows.UI.Xaml` control (TextBlock, Button, ToggleSwitch, Slider, TextBox,
//! ComboBox) hosted inside a `DesktopWindowXamlSource`. Day owns layout — containers are XAML
//! `Canvas`es and children are placed by absolute frame — exactly like the GTK/AppKit/Qt
//! backends. Native events (Click/Toggled/ValueChanged/TextChanged) funnel through the shim's
//! id-keyed callbacks into Day's event sink.

#![cfg(windows)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::rc::Rc;

use day_winui_sys as ffi;
use linkme::distributed_slice;

use day_spec::props::*;
use day_spec::{
    A11yProps, AnimSpec, Cap, DrawOp, Event, EventSink, Font, NodeId, PieceKind, Platform,
    Proposal, Rect, Registry, Renderer, Size, Support, Toolkit, WindowOptions, kinds,
};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct WinHandle(pub *mut c_void);

pub type Handle = WinHandle;

/// The day-core event sink (node-id keyed).
type Sink = Rc<dyn Fn(NodeId, Event)>;

thread_local! {
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
    /// Slider f64 range, keyed by node id (event callbacks) and handle ptr (patch application).
    static RANGES: RefCell<HashMap<u64, (f64, f64)>> = RefCell::new(HashMap::new());
    static RANGES_BY_PTR: RefCell<HashMap<usize, (f64, f64)>> = RefCell::new(HashMap::new());
    /// Tabs host ptr → (Pivot ptr, pages, initial). Pages reuse day.container.
    static TABS_STATE: RefCell<HashMap<usize, TabsState>> = RefCell::new(HashMap::new());
    static TABS_PAGE_IDS: RefCell<HashMap<usize, NodeId>> = RefCell::new(HashMap::new());
    static TABS_PAGE_TITLES: RefCell<HashMap<usize, String>> = RefCell::new(HashMap::new());
    /// Recycling-list host ptr → its ScrollViewer/content + cell pool (docs/list.md).
    static LIST_STATE: RefCell<HashMap<usize, ListEntry>> = RefCell::new(HashMap::new());
    /// NAV_MENU widget ptr → row count (for measure).
    static NAV_MENU_ROWS: RefCell<HashMap<usize, usize>> = RefCell::new(HashMap::new());
    /// NAV host ptr → its sidebar/detail panes + pages (docs/navigation.md).
    static NAV_STATE: RefCell<HashMap<usize, NavState>> = RefCell::new(HashMap::new());
}

// Navigation host: a Canvas holding two child Canvases — sidebar (fixed 240pt) + detail.
// day-core's NavLayout sizes each page to its pane (origin 0,0); this backend positions the
// panes side by side (NavLayout expects each page to live in its own positioned pane).
struct NavState {
    sidebar_pane: *mut c_void,
    detail_pane: *mut c_void,
    pages: Vec<*mut c_void>,
}

extern "C" fn nav_menu_changed(id: u64, index: c_int) {
    emit(NodeId(id), Event::SelectionChanged(index as i64));
}

/// Lay the sidebar + detail panes across the nav host of the given size.
fn nav_layout_panes(host: *mut c_void, w: f64, h: f64) {
    NAV_STATE.with(|m| {
        if let Some(state) = m.borrow().get(&(host as usize)) {
            let sidebar = day_spec::NAV_SIDEBAR_WIDTH;
            let detail_x = sidebar + 1.0;
            unsafe {
                ffi::day_winui_set_geometry(state.sidebar_pane, 0, 0, sidebar as c_int, h as c_int);
                ffi::day_winui_set_geometry(
                    state.detail_pane,
                    detail_x as c_int,
                    0,
                    (w - detail_x).max(0.0) as c_int,
                    h as c_int,
                );
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
}

/// Populate/refresh a list's cells on the next loop turn — NOT inline: a reload runs inside a
/// `with_tree` borrow, and `bind_row` re-enters `with_tree`, which would panic.
fn schedule_list_populate(host_key: usize) {
    let boxed: Box<dyn FnOnce() + Send> = Box::new(move || list_populate(host_key));
    let data = Box::into_raw(Box::new(boxed)) as *mut c_void;
    unsafe { ffi::day_winui_post(run_posted, data) };
}

fn list_populate(host_key: usize) {
    // Phase 1 — under the LIST_STATE borrow: grow the cell pool + snapshot what we need.
    let Some((content, rowh, source, cells, n, width)) = LIST_STATE.with(|m| {
        let mut m = m.borrow_mut();
        let st = m.get_mut(&host_key)?;
        let source = st.source.borrow().clone()?;
        let (mut w, mut h) = (0.0_f64, 0.0_f64);
        unsafe { ffi::day_winui_widget_size(st.host, &mut w, &mut h) };
        let width = w.max(1.0) as c_int;
        let n = (source.len)();
        while st.cells.len() < n {
            let cell = unsafe { ffi::day_winui_container_new() };
            unsafe { ffi::day_winui_add_child(st.content, cell) };
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
            ffi::day_winui_set_geometry(cell, 0, (i as f64 * rowh) as c_int, width, rowh as c_int);
            ffi::day_winui_set_visible(cell, 1);
        }
        (source.bind_row)(i, cell);
    }
    for &cell in cells.iter().skip(n) {
        unsafe { ffi::day_winui_set_visible(cell, 0) };
    }
    unsafe { ffi::day_winui_list_set_content_size(content, width, (n as f64 * rowh) as c_int) };
}

struct TabsState {
    tabs: *mut c_void,
    pages: Vec<(WinHandle, NodeId)>,
    initial: usize,
}

extern "C" fn tabs_changed(id: u64, index: c_int) {
    emit(NodeId(id), Event::SelectionChanged(index as i64));
}

fn tabs_sync(host: *mut c_void) {
    let reports: Vec<(NodeId, Size)> = TABS_STATE.with(|m| {
        let m = m.borrow();
        let Some(state) = m.get(&(host as usize)) else {
            return Vec::new();
        };
        let (mut w, mut h) = (0.0, 0.0);
        unsafe { ffi::day_winui_tabs_content_size(state.tabs, &mut w, &mut h) };
        if w <= 0.0 || h <= 0.0 {
            return Vec::new();
        }
        state
            .pages
            .iter()
            .map(|(_, id)| (*id, Size::new(w, h)))
            .collect()
    });
    for (id, size) in reports {
        emit(id, Event::FrameChanged(size));
    }
}

/// Emit an event into day-core's queue (public for external Day Piece renderers).
pub fn emit(id: NodeId, ev: Event) {
    let sink = SINK.with(|s| s.borrow().clone());
    if let Some(sink) = sink {
        sink(id, ev);
    }
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

extern "C" fn on_press(id: u64) {
    emit(NodeId(id), Event::Pressed);
}
extern "C" fn on_toggle(id: u64, on: c_int) {
    emit(NodeId(id), Event::ToggleChanged(on != 0));
}
extern "C" fn on_text(id: u64, s: *const c_char) {
    let text = unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned();
    emit(NodeId(id), Event::TextChanged(text));
}
extern "C" fn on_slider(id: u64, v: c_int) {
    let (min, max) = RANGES.with(|r| r.borrow().get(&id).copied().unwrap_or((0.0, 1.0)));
    let value = min + (v as f64 / 1000.0) * (max - min);
    emit(NodeId(id), Event::ValueChanged(value));
}

fn slider_ticks(value: f64, min: f64, max: f64) -> c_int {
    if max <= min {
        return 0;
    }
    (((value - min) / (max - min)) * 1000.0).round() as c_int
}

/// A `0.0..=1.0` fraction as ProgressBar ticks (0..1000), clamped.
fn progress_ticks(fraction: f64) -> c_int {
    (fraction.clamp(0.0, 1.0) * 1000.0).round() as c_int
}

/// Renderers registered by external Day Piece crates (§8.2).
#[distributed_slice]
pub static RENDERERS: [fn() -> Renderer<WinUi>];

pub struct WinUi {
    registry: Registry<WinUi>,
    window: *mut c_void,
}

impl WinUi {
    pub fn new() -> Self {
        let mut registry = Registry::default();
        for f in RENDERERS {
            registry.register(f());
        }
        WinUi {
            registry,
            window: std::ptr::null_mut(),
        }
    }
}

impl Default for WinUi {
    fn default() -> Self {
        Self::new()
    }
}

/// Day font intents → (XAML FontSize in DIPs, bold).
/// Point size + the style's inherent weight for a logical [`Font`]. WinUI's `TextBlock.FontSize`
/// auto-scales with the OS text-scale-factor (Settings ▸ Accessibility ▸ Text size), so these sizes
/// honor accessibility. Aligned with the desktop scale used by the GTK/Qt backends.
fn winui_style(f: Font) -> (f64, day_spec::FontWeight) {
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
    }
}

/// Day weight → Windows.UI.Text.FontWeight numeric value (Thin=100 … Black=900).
fn winui_weight(w: day_spec::FontWeight) -> c_int {
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
fn font_params(spec: day_spec::FontSpec) -> (f64, c_int, c_int) {
    let (pt, inherent) = winui_style(spec.style);
    let weight = winui_weight(spec.weight.unwrap_or(inherent));
    (pt, weight, spec.italic as c_int)
}

/// Natural (unconstrained) desired size from the shim's XAML Measure.
fn natural(h: *mut c_void) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { ffi::day_winui_measure(h, -1.0, -1.0, &mut w, &mut hh) };
    Size::new(w, hh)
}

impl Toolkit for WinUi {
    type Handle = WinHandle;

    fn capability(&self, cap: Cap) -> Support {
        match cap {
            Cap::Snapshot => Support::Native,
            Cap::ListRecycling => Support::Emulated,
            // Present `nav()` as split panes: NAV/NAV_PAGE are plain Canvases and day-core's
            // NavLayout positions the sidebar + detail (no native split control needed).
            Cap::NavSplit => Support::Native,
            _ => Support::Unsupported,
        }
    }

    fn realize(&mut self, kind: PieceKind, props: &dyn std::any::Any, id: NodeId) -> WinHandle {
        unsafe {
            match kind {
                kinds::CONTAINER => {
                    let h = ffi::day_winui_container_new();
                    if let Some(p) = props.downcast_ref::<ContainerProps>()
                        && let Some(bg) = p.background
                    {
                        ffi::day_winui_container_set_bg(h, argb(bg));
                    }
                    WinHandle(h)
                }
                kinds::SCROLL => WinHandle(ffi::day_winui_scroll_new()),
                kinds::CANVAS => WinHandle(ffi::day_winui_canvas_new()),
                kinds::NAV => {
                    let host = ffi::day_winui_container_new();
                    let sidebar_pane = ffi::day_winui_container_new();
                    let detail_pane = ffi::day_winui_container_new();
                    ffi::day_winui_add_child(host, sidebar_pane);
                    ffi::day_winui_add_child(host, detail_pane);
                    NAV_STATE.with(|m| {
                        m.borrow_mut().insert(
                            host as usize,
                            NavState {
                                sidebar_pane,
                                detail_pane,
                                pages: Vec::new(),
                            },
                        )
                    });
                    WinHandle(host)
                }
                kinds::NAV_PAGE => WinHandle(ffi::day_winui_container_new()),
                kinds::NAV_MENU => {
                    let p = props.downcast_ref::<NavMenuProps>().unwrap();
                    let w = ffi::day_winui_navlist_new(id.0, nav_menu_changed);
                    ffi::day_winui_navlist_set_items(w, cstr(&p.items.join("\n")).as_ptr());
                    ffi::day_winui_navlist_set_selected(
                        w,
                        p.selected.map(|i| i as c_int).unwrap_or(-1),
                    );
                    NAV_MENU_ROWS.with(|m| m.borrow_mut().insert(w as usize, p.items.len()));
                    WinHandle(w)
                }
                kinds::LABEL => {
                    let p = props.downcast_ref::<LabelProps>().unwrap();
                    let h = ffi::day_winui_label_new(cstr(&p.text).as_ptr());
                    let (pt, weight, italic) = font_params(p.font);
                    ffi::day_winui_label_set_font(h, pt, weight, italic);
                    WinHandle(h)
                }
                kinds::BUTTON => {
                    let p = props.downcast_ref::<ButtonProps>().unwrap();
                    let h = ffi::day_winui_button_new(cstr(&p.title).as_ptr(), id.0, on_press);
                    ffi::day_winui_set_enabled(h, p.enabled as c_int);
                    WinHandle(h)
                }
                kinds::TOGGLE => {
                    let p = props.downcast_ref::<ToggleProps>().unwrap();
                    let h = ffi::day_winui_toggle_new(p.on as c_int, id.0, on_toggle);
                    ffi::day_winui_set_enabled(h, p.enabled as c_int);
                    WinHandle(h)
                }
                kinds::SLIDER => {
                    let p = props.downcast_ref::<SliderProps>().unwrap();
                    RANGES.with(|r| r.borrow_mut().insert(id.0, (p.min, p.max)));
                    let h = ffi::day_winui_slider_new(
                        slider_ticks(p.value, p.min, p.max),
                        id.0,
                        on_slider,
                    );
                    RANGES_BY_PTR.with(|r| r.borrow_mut().insert(h as usize, (p.min, p.max)));
                    ffi::day_winui_set_enabled(h, p.enabled as c_int);
                    WinHandle(h)
                }
                kinds::TEXT_FIELD => {
                    let p = props.downcast_ref::<TextFieldProps>().unwrap();
                    let h = ffi::day_winui_textbox_new(
                        cstr(&p.text).as_ptr(),
                        cstr(&p.placeholder).as_ptr(),
                        id.0,
                        on_text,
                    );
                    ffi::day_winui_set_enabled(h, p.enabled as c_int);
                    WinHandle(h)
                }
                kinds::DIVIDER => WinHandle(ffi::day_winui_divider_new()),
                kinds::LIST => {
                    let p = props.downcast_ref::<ListProps>().unwrap();
                    let mut content: *mut c_void = std::ptr::null_mut();
                    let host = ffi::day_winui_list_new(&mut content);
                    let row_height = match p.row_height {
                        RowHeight::Uniform(h) => h,
                        RowHeight::Automatic => 44.0,
                    };
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
                            },
                        )
                    });
                    WinHandle(host)
                }
                kinds::PROGRESS => {
                    let p = props.downcast_ref::<ProgressProps>().unwrap();
                    match p.value {
                        Some(v) => WinHandle(ffi::day_winui_progress_new(1, progress_ticks(v))),
                        None => WinHandle(ffi::day_winui_progress_new(0, 0)),
                    }
                }
                kinds::TABS => {
                    let p = props.downcast_ref::<TabsProps>().unwrap();
                    let w = ffi::day_winui_tabs_new(id.0, tabs_changed);
                    TABS_STATE.with(|m| {
                        m.borrow_mut().insert(
                            w as usize,
                            TabsState {
                                tabs: w,
                                pages: Vec::new(),
                                initial: p.selected,
                            },
                        )
                    });
                    WinHandle(w)
                }
                kinds::TABS_PAGE => {
                    let p = props.downcast_ref::<TabsPageProps>().unwrap();
                    let page = WinHandle(ffi::day_winui_container_new());
                    TABS_PAGE_IDS.with(|m| m.borrow_mut().insert(page.0 as usize, id));
                    TABS_PAGE_TITLES
                        .with(|m| m.borrow_mut().insert(page.0 as usize, p.title.clone()));
                    page
                }
                kinds::IMAGE => {
                    let p = props.downcast_ref::<ImageProps>().unwrap();
                    WinHandle(ffi::day_winui_image_new(
                        cstr(&image_uri(&p.source)).as_ptr(),
                    ))
                }
                _ => {
                    if let Some(make) = self.registry.get(kind).map(|r| r.make) {
                        return make(self, props, id);
                    }
                    WinHandle(ffi::day_winui_label_new(
                        cstr(&format!("⟨{kind}⟩")).as_ptr(),
                    ))
                }
            }
        }
    }

    fn update(
        &mut self,
        h: &WinHandle,
        kind: PieceKind,
        patch: &dyn std::any::Any,
        _anim: Option<&AnimSpec>,
    ) {
        unsafe {
            match kind {
                kinds::LABEL => {
                    if let Some(p) = patch.downcast_ref::<LabelPatch>() {
                        match p {
                            LabelPatch::Text(t) => {
                                ffi::day_winui_label_set_text(h.0, cstr(t).as_ptr())
                            }
                            LabelPatch::Font(f) => {
                                let (pt, weight, italic) = font_params(*f);
                                ffi::day_winui_label_set_font(h.0, pt, weight, italic);
                            }
                            LabelPatch::Color(_) => {} // XAML Foreground token is a follow-up
                        }
                    }
                }
                kinds::BUTTON => {
                    if let Some(p) = patch.downcast_ref::<ButtonPatch>() {
                        match p {
                            ButtonPatch::Title(t) => {
                                ffi::day_winui_button_set_title(h.0, cstr(t).as_ptr())
                            }
                            ButtonPatch::Enabled(e) => ffi::day_winui_set_enabled(h.0, *e as c_int),
                        }
                    }
                }
                kinds::TOGGLE => {
                    if let Some(p) = patch.downcast_ref::<TogglePatch>() {
                        match p {
                            TogglePatch::On(on) => ffi::day_winui_toggle_set(h.0, *on as c_int),
                            TogglePatch::Enabled(e) => ffi::day_winui_set_enabled(h.0, *e as c_int),
                        }
                    }
                }
                kinds::SLIDER => {
                    if let Some(p) = patch.downcast_ref::<SliderPatch>() {
                        match p {
                            SliderPatch::Value(v) => {
                                let (min, max) = RANGES_BY_PTR
                                    .with(|r| r.borrow().get(&(h.0 as usize)).copied())
                                    .unwrap_or((0.0, 1.0));
                                ffi::day_winui_slider_set(h.0, slider_ticks(*v, min, max));
                            }
                            SliderPatch::Enabled(e) => ffi::day_winui_set_enabled(h.0, *e as c_int),
                        }
                    }
                }
                kinds::PROGRESS => {
                    if let Some(ProgressPatch::Value(Some(v))) =
                        patch.downcast_ref::<ProgressPatch>()
                    {
                        ffi::day_winui_progress_set(h.0, progress_ticks(*v));
                    }
                }
                kinds::TABS => {
                    if let Some(TabsPatch::Selected(i)) = patch.downcast_ref::<TabsPatch>() {
                        TABS_STATE.with(|m| {
                            if let Some(state) = m.borrow().get(&(h.0 as usize)) {
                                ffi::day_winui_tabs_set_current(state.tabs, *i as c_int);
                            }
                        });
                    }
                }
                kinds::LIST => {
                    if let Some(ListPatch::Reload) = patch.downcast_ref::<ListPatch>() {
                        schedule_list_populate(h.0 as usize);
                    }
                }
                kinds::NAV_MENU => {
                    if let Some(NavMenuPatch::Selected(sel)) = patch.downcast_ref::<NavMenuPatch>()
                    {
                        ffi::day_winui_navlist_set_selected(
                            h.0,
                            sel.map(|i| i as c_int).unwrap_or(-1),
                        );
                    }
                }
                // NAV Pushed/Popped/Title need no native work — NavLayout re-places the pages.
                kinds::NAV => {}
                kinds::TEXT_FIELD => {
                    if let Some(p) = patch.downcast_ref::<TextFieldPatch>() {
                        match p {
                            TextFieldPatch::Text { text, from_native } => {
                                if !*from_native {
                                    ffi::day_winui_textbox_set_text(h.0, cstr(text).as_ptr());
                                }
                            }
                            TextFieldPatch::Placeholder(t) => {
                                ffi::day_winui_textbox_set_placeholder(h.0, cstr(t).as_ptr())
                            }
                            TextFieldPatch::Enabled(e) => {
                                ffi::day_winui_set_enabled(h.0, *e as c_int)
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

    fn release(&mut self, h: WinHandle) {
        let key = h.0 as usize;
        RANGES_BY_PTR.with(|r| r.borrow_mut().remove(&key));
        TABS_STATE.with(|m| m.borrow_mut().remove(&key));
        TABS_PAGE_IDS.with(|m| m.borrow_mut().remove(&key));
        TABS_PAGE_TITLES.with(|m| m.borrow_mut().remove(&key));
        NAV_MENU_ROWS.with(|m| m.borrow_mut().remove(&key));
        if let Some(state) = NAV_STATE.with(|m| m.borrow_mut().remove(&key)) {
            unsafe {
                ffi::day_winui_delete(state.sidebar_pane);
                ffi::day_winui_delete(state.detail_pane);
            }
        }
        // day-core never releases the adopted cell handles (their anchors are detached), so the
        // list host owns cell + content cleanup.
        if let Some(st) = LIST_STATE.with(|m| m.borrow_mut().remove(&key)) {
            for cell in st.cells {
                unsafe { ffi::day_winui_delete(cell) };
            }
            unsafe { ffi::day_winui_delete(st.content) };
        }
        unsafe { ffi::day_winui_delete(h.0) };
    }

    fn insert(&mut self, parent: &WinHandle, child: &WinHandle, index: usize) {
        // Tabs host: add the page to the Pivot with its label; the Pivot owns page layout.
        let tabs_handled = TABS_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let Some(state) = m.get_mut(&(parent.0 as usize)) else {
                return false;
            };
            let id = TABS_PAGE_IDS
                .with(|ids| ids.borrow().get(&(child.0 as usize)).copied())
                .unwrap_or(NodeId(0));
            let title = TABS_PAGE_TITLES
                .with(|t| t.borrow().get(&(child.0 as usize)).cloned())
                .unwrap_or_default();
            unsafe {
                ffi::day_winui_tabs_add_page(
                    state.tabs,
                    child.0,
                    cstr(&title).as_ptr(),
                    index as c_int,
                )
            };
            let at = index.min(state.pages.len());
            state.pages.insert(at, (*child, id));
            if index == state.initial {
                unsafe { ffi::day_winui_tabs_set_current(state.tabs, index as c_int) };
            }
            true
        });
        if tabs_handled {
            tabs_sync(parent.0);
            return;
        }
        // Nav host: page index 0 = sidebar pane, the rest = detail pane.
        let nav_handled = NAV_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let Some(state) = m.get_mut(&(parent.0 as usize)) else {
                return false;
            };
            let pane = if index == 0 {
                state.sidebar_pane
            } else {
                state.detail_pane
            };
            unsafe { ffi::day_winui_add_child(pane, child.0) };
            state.pages.push(child.0);
            true
        });
        if nav_handled {
            return;
        }
        unsafe { ffi::day_winui_add_child(parent.0, child.0) };
    }

    fn remove(&mut self, parent: &WinHandle, child: &WinHandle) {
        // Nav pages live in a pane, not directly under the host — remove from whichever pane.
        let panes = NAV_STATE.with(|m| {
            let mut m = m.borrow_mut();
            m.get_mut(&(parent.0 as usize)).map(|state| {
                state.pages.retain(|&p| p != child.0);
                (state.sidebar_pane, state.detail_pane)
            })
        });
        match panes {
            Some((sidebar, detail)) => unsafe {
                ffi::day_winui_remove_child(sidebar, child.0);
                ffi::day_winui_remove_child(detail, child.0);
            },
            None => unsafe { ffi::day_winui_remove_child(parent.0, child.0) },
        }
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
                        unsafe { ffi::day_winui_measure(h.0, pw, -1.0, &mut w, &mut hh) };
                        Size::new(pw.ceil(), hh.ceil())
                    }
                    _ => Size::new(nat.width.ceil(), nat.height.ceil()),
                }
            }
            kinds::SLIDER => Size::new(p.width.unwrap_or(180.0), natural(h.0).height.max(24.0)),
            kinds::TEXT_FIELD => Size::new(p.width.unwrap_or(180.0), natural(h.0).height.max(28.0)),
            kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 1.0),
            // The list host fills whatever frame layout gives it; cells fill its width.
            kinds::LIST => Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0)),
            kinds::NAV_MENU => {
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

    fn set_frame(&mut self, h: &WinHandle, frame: Rect, _anim: Option<&AnimSpec>) {
        // Tab pages are laid out by the Pivot, not by Day; skip them.
        if TABS_PAGE_IDS.with(|m| m.borrow().contains_key(&(h.0 as usize))) {
            return;
        }
        unsafe {
            ffi::day_winui_set_geometry(
                h.0,
                frame.origin.x.round() as c_int,
                frame.origin.y.round() as c_int,
                frame.size.width.round() as c_int,
                frame.size.height.round() as c_int,
            )
        };
        if TABS_STATE.with(|m| m.borrow().contains_key(&(h.0 as usize))) {
            tabs_sync(h.0);
        }
        // Nav host framed (or window resized): re-lay the sidebar + detail panes.
        if NAV_STATE.with(|m| m.borrow().contains_key(&(h.0 as usize))) {
            nav_layout_panes(h.0, frame.size.width, frame.size.height);
        }
        // List host framed: (re)fill its cells — but ONLY when the width actually changed, so the
        // set_frames a populate itself makes (on row content) don't schedule another forever.
        let width_changed = LIST_STATE.with(|m| {
            m.borrow()
                .get(&(h.0 as usize))
                .map(|st| st.last_width != frame.size.width.round() as c_int)
                .unwrap_or(false)
        });
        if width_changed {
            schedule_list_populate(h.0 as usize);
        }
    }

    fn set_event_sink(&mut self, sink: EventSink) {
        SINK.with(|s| *s.borrow_mut() = Some(Rc::from(sink)));
    }

    fn set_a11y(&mut self, h: &WinHandle, a11y: &A11yProps) {
        if let Some(id) = &a11y.identifier {
            unsafe { ffi::day_winui_set_name(h.0, cstr(id).as_ptr()) };
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

    fn adopt(&mut self, raw: day_spec::RawHandle) -> WinHandle {
        // A recycling-list cell (a plain Canvas) — Day builds/rebinds its row content in place.
        WinHandle(raw)
    }

    fn replay(&mut self, h: &WinHandle, ops: &[DrawOp], _size: Size) {
        let (nums, texts) = day_spec::encode_ops(ops);
        let joined = cstr(&texts.join("\n"));
        unsafe {
            ffi::day_winui_canvas_set_ops(h.0, nums.as_ptr(), nums.len() as c_int, joined.as_ptr())
        };
    }

    fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
        if self.window.is_null() {
            return Err("no window".into());
        }
        let path = std::env::temp_dir().join(format!("day-winui-snap-{}.png", std::process::id()));
        let cpath = cstr(&path.to_string_lossy());
        let rc = unsafe { ffi::day_winui_snapshot_png(self.window, cpath.as_ptr()) };
        if rc != 0 {
            return Err(format!("snapshot failed (rc={rc})"));
        }
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&path);
        Ok(bytes)
    }
}

fn argb(c: day_spec::Color) -> u32 {
    let a = (c.a.clamp(0.0, 1.0) * 255.0) as u32;
    let r = (c.r.clamp(0.0, 1.0) * 255.0) as u32;
    let g = (c.g.clamp(0.0, 1.0) * 255.0) as u32;
    let b = (c.b.clamp(0.0, 1.0) * 255.0) as u32;
    (a << 24) | (r << 16) | (g << 8) | b
}

/// Resolve an asset name to a `file:///` URI the XAML `BitmapImage` can load (§18.2).
fn image_uri(source: &str) -> String {
    let path = std::env::var("DAY_ASSET_ROOT")
        .map(|r| std::path::Path::new(&r).join(source))
        .ok()
        .filter(|p| p.exists());
    match path {
        Some(p) => format!("file:///{}", p.to_string_lossy().replace('\\', "/")),
        None => String::new(),
    }
}

extern "C" fn window_resized(w: c_int, h: c_int) {
    // Client rect is reported in pixels; day-winui's v1 assumes a 100% scale factor
    // throughout (same convention as window creation).
    emit(
        day_spec::WINDOW_NODE,
        Event::WindowResized(Size::new(w as f64, h as f64)),
    );
}

extern "C" fn run_posted(data: *mut c_void) {
    let f: Box<Box<dyn FnOnce() + Send>> = unsafe { Box::from_raw(data as *mut _) };
    f();
}

impl Platform for WinUi {
    const TARGET: &'static str = "windows-winui";
    const TOOLKIT: &'static str = "winui";

    fn run(mut self, options: WindowOptions, ready: Box<dyn FnOnce(Self, WinHandle, Size)>) {
        unsafe {
            let win = ffi::day_winui_window_new(
                cstr(&options.title).as_ptr(),
                options.size.width as c_int,
                options.size.height as c_int,
            );
            if win.is_null() {
                eprintln!("day-winui: could not create the XAML window (see error above)");
                return;
            }
            self.window = win;
            let root = ffi::day_winui_window_root(win);
            ready(self, WinHandle(root), options.size);
            ffi::day_winui_window_on_resize(win, window_resized);
            ffi::day_winui_window_show(win);
            ffi::day_winui_run(win);
        }
    }

    fn post(f: Box<dyn FnOnce() + Send>) {
        let data = Box::into_raw(Box::new(f)) as *mut c_void;
        unsafe { ffi::day_winui_post(run_posted, data) };
    }
}
