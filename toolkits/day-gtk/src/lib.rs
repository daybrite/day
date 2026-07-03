//! day-gtk — the GTK 4 backend (linux-gtk / macos-gtk; DESIGN.md §9). gtk4-rs, pure Rust.
//!
//! `Handle = gtk4::Widget` (GObject-refcounted, `!Send`). Containers are `GtkFixed`; day's
//! layout positions children via `fixed.move_()` + `set_size_request` (hop's proven pattern).
//! Native signals connect once at realize, capturing the NodeId and emitting into the day sink.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use libadwaita as adw;
use linkme::distributed_slice;

use day_spec::props::*;
use day_spec::{
    A11yProps, AnimSpec, Cap, DrawOp, Event, EventSink, Font, NodeId, PieceKind, Platform,
    Proposal, Rect, Registry, Renderer, Size, Support, Toolkit, kinds,
};

pub type Handle = gtk4::Widget;

/// The day-core event sink (node-id keyed).
type Sink = Rc<dyn Fn(NodeId, Event)>;

thread_local! {
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
    static OPS: RefCell<HashMap<usize, Vec<DrawOp>>> = RefCell::new(HashMap::new());
}

fn cairo_set_color(cr: &gtk4::cairo::Context, bits: f64) {
    let v = bits as u32;
    cr.set_source_rgba(
        ((v >> 16) & 0xff) as f64 / 255.0,
        ((v >> 8) & 0xff) as f64 / 255.0,
        (v & 0xff) as f64 / 255.0,
        ((v >> 24) & 0xff) as f64 / 255.0,
    );
}

fn cairo_draw(cr: &gtk4::cairo::Context, ops: &[DrawOp]) {
    let (nums, texts) = day_spec::encode_ops(ops);
    let mut ti = 0;
    for chunk in nums.chunks(9) {
        let (k, a, b, c, d, e, f, g, col) = (
            chunk[0] as i32,
            chunk[1],
            chunk[2],
            chunk[3],
            chunk[4],
            chunk[5],
            chunk[6],
            chunk[7],
            chunk[8],
        );
        cairo_set_color(cr, col);
        match k {
            0 | 1 => {
                cr.rectangle(a, b, c, d);
                if k == 0 {
                    let _ = cr.fill();
                } else {
                    cr.set_line_width(g);
                    let _ = cr.stroke();
                }
            }
            2 => {
                cr.rectangle(a, b, c, d); // rounded post-MVP refinement
                let _ = cr.fill();
            }
            3 | 4 => {
                cr.save().ok();
                cr.translate(a + c / 2.0, b + d / 2.0);
                cr.scale(c / 2.0, d / 2.0);
                cr.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
                cr.restore().ok();
                if k == 3 {
                    let _ = cr.fill();
                } else {
                    cr.set_line_width(g);
                    let _ = cr.stroke();
                }
            }
            5 => {
                let (cx_, cy) = (a + c / 2.0, b + d / 2.0);
                let radius = c.min(d) / 2.0;
                let start = e.to_radians();
                let end = (e + f).to_radians();
                cr.set_line_width(g);
                cr.set_line_cap(gtk4::cairo::LineCap::Round);
                cr.arc(cx_, cy, radius, start, end);
                let _ = cr.stroke();
            }
            6 => {
                cr.set_line_width(g);
                cr.move_to(a, b);
                cr.line_to(c, d);
                let _ = cr.stroke();
            }
            7 => {
                let text = texts.get(ti).cloned().unwrap_or_default();
                ti += 1;
                cr.set_font_size(e);
                let (mut x, mut y) = (a, b);
                if f > 0.5
                    && let Ok(ext) = cr.text_extents(&text)
                {
                    x -= ext.width() / 2.0;
                    y += ext.height() / 2.0;
                }
                cr.move_to(x, y);
                let _ = cr.show_text(&text); // toy API; PangoCairo refinement is a TODO (§11)
            }
            _ => {}
        }
    }
}

/// Emit an event into day-core's queue (public for external Day Piece renderers).
pub fn emit(id: NodeId, ev: Event) {
    let sink = SINK.with(|s| s.borrow().clone());
    if let Some(sink) = sink {
        sink(id, ev);
    }
}

// ---------------------------------------------------------------------------
// Navigation (docs/navigation.md): libadwaita. selector(Sidebar) → AdwNavigationSplitView;
// stack → AdwNavigationView (push/pop). Each page's GtkFixed is wrapped in an
// AdwNavigationPage; day sizes content from the host width via FrameChanged (nav_report).
// ---------------------------------------------------------------------------

/// The sidebar's fixed width in the split view (day sizes detail content = host − this).
const NAV_SIDEBAR_W: f64 = day_spec::NAV_SIDEBAR_WIDTH;

/// selector(Sidebar) → AdwNavigationSplitView; stack → AdwNavigationView (push/pop).
enum NavPresent {
    Split(adw::NavigationSplitView),
    Stack(adw::NavigationView),
}

struct NavState {
    present: NavPresent,
    /// Sidebar+detail split (selector Sidebar) vs. a pure push/pop stack (`stack`).
    split: bool,
    /// (page GtkFixed key, node id, its AdwNavigationPage) in order (index 0 = sidebar/root).
    pages: Vec<(usize, NodeId, adw::NavigationPage)>,
    /// A programmatic pop is in flight: the `popped` handler must not re-emit NavBack.
    suppress: Rc<std::cell::Cell<bool>>,
}

struct NavMenuState {
    listbox: gtk4::ListBox,
    rows: usize,
    /// Programmatic selection in flight: don't re-emit SelectionChanged.
    suppress: Rc<std::cell::Cell<bool>>,
}

thread_local! {
    static NAV_STATE: RefCell<HashMap<usize, NavState>> = RefCell::new(HashMap::new());
    /// NAV_PAGE widget → its day node id (recorded at realize, joined at insert).
    static NAV_PAGE_IDS: RefCell<HashMap<usize, NodeId>> = RefCell::new(HashMap::new());
    /// NAV_PAGE widget → its title (for the AdwNavigationPage).
    static NAV_PAGE_TITLES: RefCell<HashMap<usize, String>> = RefCell::new(HashMap::new());
    /// NAV_MENU widget → its list box + suppression flag.
    static NAV_MENUS: RefCell<HashMap<usize, NavMenuState>> = RefCell::new(HashMap::new());
}

fn widget_key(w: &Handle) -> usize {
    w.as_ptr() as usize
}

/// Emit each page's content size so NavLayout re-lays it (enqueue-only, §8.3). Split: the
/// sidebar is a fixed width and the detail fills the rest; stack: every page fills the host.
fn nav_report(host_key: usize) {
    let reports: Vec<(NodeId, Size)> = NAV_STATE.with(|m| {
        let m = m.borrow();
        let Some(state) = m.get(&host_key) else {
            return Vec::new();
        };
        let (hw, hh) = match &state.present {
            NavPresent::Split(sv) => (sv.width() as f64, sv.height() as f64),
            NavPresent::Stack(nv) => (nv.width() as f64, nv.height() as f64),
        };
        if hw <= 0.0 || hh <= 0.0 {
            return Vec::new();
        }
        state
            .pages
            .iter()
            .enumerate()
            .map(|(i, (_, id, _))| {
                let size = if state.split {
                    if i == 0 {
                        Size::new(NAV_SIDEBAR_W, hh)
                    } else {
                        Size::new((hw - NAV_SIDEBAR_W).max(0.0), hh)
                    }
                } else {
                    Size::new(hw, hh)
                };
                (*id, size)
            })
            .collect()
    });
    for (id, size) in reports {
        emit(id, Event::FrameChanged(size));
    }
}

// ---------------------------------------------------------------------------
// Tabs (docs/tabs.md): a GtkNotebook host holding GtkFixed page containers.
// ---------------------------------------------------------------------------

/// Approximate GtkNotebook tab-strip height, subtracted from the host to size page content.
const TAB_BAR_H: f64 = 40.0;

struct TabsState {
    notebook: gtk4::Notebook,
    /// (page widget, node id) in tab order.
    pages: Vec<(Handle, NodeId)>,
    /// Tab to select once its page exists (GtkNotebook shows the first page by default).
    initial: usize,
    /// Programmatic selection in flight: don't re-emit SelectionChanged.
    suppress: Rc<std::cell::Cell<bool>>,
}

thread_local! {
    static TABS_STATE: RefCell<HashMap<usize, TabsState>> = RefCell::new(HashMap::new());
    /// TABS_PAGE widget → its day node id (recorded at realize, joined at insert).
    static TABS_PAGE_IDS: RefCell<HashMap<usize, NodeId>> = RefCell::new(HashMap::new());
    /// TABS_PAGE widget → its tab label.
    static TABS_PAGE_TITLES: RefCell<HashMap<usize, String>> = RefCell::new(HashMap::new());
    /// TABS_PAGE widget keys (set_frame skips them — the notebook owns their layout).
    static TABS_PAGES: RefCell<std::collections::HashSet<usize>> =
        RefCell::new(std::collections::HashSet::new());
}

/// Report each tab page's content size (host minus the tab strip) so NavLayout re-lays it.
fn tabs_sync(host_key: usize) {
    let reports: Vec<(NodeId, Size)> = TABS_STATE.with(|m| {
        let m = m.borrow();
        let Some(state) = m.get(&host_key) else {
            return Vec::new();
        };
        let w = state.notebook.width() as f64;
        let h = state.notebook.height() as f64;
        if w <= 0.0 || h <= 0.0 {
            return Vec::new();
        }
        let content_h = (h - TAB_BAR_H).max(0.0);
        state
            .pages
            .iter()
            .map(|(_, id)| (*id, Size::new(w, content_h)))
            .collect()
    });
    for (id, size) in reports {
        emit(id, Event::FrameChanged(size));
    }
}

/// Renderers registered by external Day Piece crates (§8.2).
#[distributed_slice]
pub static RENDERERS: [fn() -> Renderer<Gtk>];

pub struct Gtk {
    registry: Registry<Gtk>,
    window_fixed: Option<gtk4::Fixed>,
}

impl Gtk {
    pub fn new() -> Self {
        let mut registry = Registry::default();
        for f in RENDERERS {
            registry.register(f());
        }
        Gtk {
            registry,
            window_fixed: None,
        }
    }
}

impl Default for Gtk {
    fn default() -> Self {
        Self::new()
    }
}

fn apply_font(label: &gtk4::Label, font: Font) {
    let (size_pt, bold) = match font {
        Font::Title => (24.0, true),
        Font::Headline => (15.0, true),
        Font::Body => (13.0, false),
        Font::Caption => (11.0, false),
        Font::System(pt) => (pt, false),
    };
    let weight = if bold { "bold" } else { "normal" };
    // Pango attributes via markup-free CSS is heavyweight; use attributes list.
    let attrs = gtk4::pango::AttrList::new();
    let mut size = gtk4::pango::AttrSize::new((size_pt * gtk4::pango::SCALE as f64) as i32);
    size.set_start_index(0);
    attrs.insert(size);
    if bold {
        let mut w = gtk4::pango::AttrInt::new_weight(gtk4::pango::Weight::Bold);
        w.set_start_index(0);
        attrs.insert(w);
    }
    let _ = weight;
    label.set_attributes(Some(&attrs));
}

/// If `parent` is a scrolled window, children go into its content fixed. NOTE: GTK4 auto-wraps
/// non-scrollable children in a GtkViewport, so `sw.child()` is the viewport, not our Fixed.
fn content_of(parent: &Handle) -> Handle {
    if let Some(sw) = parent.downcast_ref::<gtk4::ScrolledWindow>()
        && let Some(child) = sw.child()
    {
        if let Some(vp) = child.downcast_ref::<gtk4::Viewport>()
            && let Some(inner) = vp.child()
        {
            return inner;
        }
        return child;
    }
    parent.clone()
}

impl Toolkit for Gtk {
    type Handle = Handle;

    fn capability(&self, cap: Cap) -> Support {
        match cap {
            Cap::Snapshot | Cap::NavSplit | Cap::Dialogs => Support::Native,
            _ => Support::Unsupported,
        }
    }

    fn realize(&mut self, kind: PieceKind, props: &dyn std::any::Any, id: NodeId) -> Handle {
        match kind {
            kinds::CONTAINER => gtk4::Fixed::new().upcast(),
            kinds::NAV => {
                let is_split = props
                    .downcast_ref::<NavProps>()
                    .map(|p| p.split)
                    .unwrap_or(true);
                let suppress = Rc::new(std::cell::Cell::new(false));
                let (host, present): (Handle, NavPresent) = if is_split {
                    // AdwNavigationSplitView: sidebar + detail, the idiomatic GNOME paradigm.
                    let sv = adw::NavigationSplitView::new();
                    sv.set_min_sidebar_width(NAV_SIDEBAR_W);
                    sv.set_max_sidebar_width(NAV_SIDEBAR_W);
                    (sv.clone().upcast(), NavPresent::Split(sv))
                } else {
                    // AdwNavigationView: a genuine push/pop stack with back gesture.
                    let nv = adw::NavigationView::new();
                    let s = suppress.clone();
                    nv.connect_popped(move |_view, _page| {
                        // A native back gesture / Escape popped a page (not a day-driven pop).
                        if !s.get() {
                            emit(
                                id,
                                Event::NavBack {
                                    already_popped: true,
                                },
                            );
                        }
                    });
                    (nv.clone().upcast(), NavPresent::Stack(nv))
                };
                let key = widget_key(&host);
                NAV_STATE.with(|m| {
                    m.borrow_mut().insert(
                        key,
                        NavState {
                            present,
                            split: is_split,
                            pages: Vec::new(),
                            suppress,
                        },
                    )
                });
                host
            }
            kinds::NAV_PAGE => {
                let title = props
                    .downcast_ref::<NavPageProps>()
                    .map(|p| p.title.clone())
                    .unwrap_or_default();
                let page: Handle = gtk4::Fixed::new().upcast();
                let key = widget_key(&page);
                NAV_PAGE_IDS.with(|m| m.borrow_mut().insert(key, id));
                NAV_PAGE_TITLES.with(|m| m.borrow_mut().insert(key, title));
                page
            }
            kinds::TABS => {
                let p = props.downcast_ref::<TabsProps>().unwrap();
                let notebook = gtk4::Notebook::new();
                notebook.set_scrollable(true);
                let host: Handle = notebook.clone().upcast();
                let key = widget_key(&host);
                let suppress = Rc::new(std::cell::Cell::new(false));
                {
                    let suppress = suppress.clone();
                    notebook.connect_switch_page(move |_, _, page_num| {
                        if !suppress.get() {
                            emit(id, Event::SelectionChanged(page_num as i64));
                        }
                    });
                }
                TABS_STATE.with(|m| {
                    m.borrow_mut().insert(
                        key,
                        TabsState {
                            notebook,
                            pages: Vec::new(),
                            initial: p.selected,
                            suppress,
                        },
                    )
                });
                host
            }
            kinds::TABS_PAGE => {
                let p = props.downcast_ref::<TabsPageProps>().unwrap();
                let page: Handle = gtk4::Fixed::new().upcast();
                let key = widget_key(&page);
                TABS_PAGE_IDS.with(|m| m.borrow_mut().insert(key, id));
                TABS_PAGE_TITLES.with(|m| m.borrow_mut().insert(key, p.title.clone()));
                TABS_PAGES.with(|s| s.borrow_mut().insert(key));
                page
            }
            kinds::NAV_MENU => {
                let p = props.downcast_ref::<NavMenuProps>().unwrap();
                let listbox = gtk4::ListBox::new();
                // The standard GNOME sidebar treatment.
                listbox.add_css_class("navigation-sidebar");
                listbox.set_selection_mode(gtk4::SelectionMode::Single);
                for item in &p.items {
                    let label = gtk4::Label::new(Some(item));
                    label.set_halign(gtk4::Align::Start);
                    listbox.append(&label);
                }
                let suppress = Rc::new(std::cell::Cell::new(false));
                {
                    let suppress = suppress.clone();
                    listbox.connect_row_selected(move |_, row| {
                        if suppress.get() {
                            return;
                        }
                        if let Some(row) = row {
                            emit(id, Event::SelectionChanged(row.index() as i64));
                        }
                    });
                }
                if let Some(sel) = p.selected {
                    suppress.set(true);
                    listbox.select_row(listbox.row_at_index(sel as i32).as_ref());
                    suppress.set(false);
                }
                let handle: Handle = listbox.clone().upcast();
                NAV_MENUS.with(|m| {
                    m.borrow_mut().insert(
                        widget_key(&handle),
                        NavMenuState {
                            listbox,
                            rows: p.items.len(),
                            suppress,
                        },
                    )
                });
                handle
            }
            kinds::SCROLL => {
                let sw = gtk4::ScrolledWindow::new();
                sw.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
                sw.set_child(Some(&gtk4::Fixed::new()));
                sw.upcast()
            }
            kinds::LABEL => {
                let p = props.downcast_ref::<LabelProps>().unwrap();
                let label = gtk4::Label::new(Some(&p.text));
                label.set_xalign(0.0);
                label.set_yalign(0.0);
                label.set_wrap(true);
                label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
                apply_font(&label, p.font);
                label.upcast()
            }
            kinds::BUTTON => {
                let p = props.downcast_ref::<ButtonProps>().unwrap();
                let btn = gtk4::Button::with_label(&p.title);
                btn.connect_clicked(move |_| emit(id, Event::Pressed));
                btn.upcast()
            }
            kinds::TOGGLE => {
                let p = props.downcast_ref::<ToggleProps>().unwrap();
                let sw = gtk4::Switch::new();
                sw.set_active(p.on);
                sw.connect_active_notify(move |s| emit(id, Event::ToggleChanged(s.is_active())));
                sw.upcast()
            }
            kinds::SLIDER => {
                let p = props.downcast_ref::<SliderProps>().unwrap();
                let step = p.step.unwrap_or((p.max - p.min) / 1000.0).max(1e-9);
                let scale =
                    gtk4::Scale::with_range(gtk4::Orientation::Horizontal, p.min, p.max, step);
                scale.set_value(p.value);
                scale.set_draw_value(false);
                scale.connect_value_changed(move |s| emit(id, Event::ValueChanged(s.value())));
                scale.upcast()
            }
            kinds::TEXT_FIELD => {
                let p = props.downcast_ref::<TextFieldProps>().unwrap();
                let entry = gtk4::Entry::new();
                entry.set_text(&p.text);
                entry.set_placeholder_text(Some(&p.placeholder));
                entry.connect_changed(move |e| emit(id, Event::TextChanged(e.text().to_string())));
                entry.upcast()
            }
            kinds::DIVIDER => gtk4::Separator::new(gtk4::Orientation::Horizontal).upcast(),
            kinds::PROGRESS => {
                let p = props.downcast_ref::<ProgressProps>().unwrap();
                match p.value {
                    Some(v) => {
                        let bar = gtk4::ProgressBar::new();
                        bar.set_fraction(v);
                        bar.upcast()
                    }
                    None => {
                        let spin = gtk4::Spinner::new();
                        spin.start();
                        spin.upcast()
                    }
                }
            }
            kinds::CANVAS => {
                let area = gtk4::DrawingArea::new();
                area.set_draw_func(|area, cr, _w, _h| {
                    let ptr = area.as_ptr() as usize;
                    let ops = OPS
                        .with(|m| m.borrow().get(&ptr).cloned())
                        .unwrap_or_default();
                    cairo_draw(cr, &ops);
                });
                area.upcast()
            }
            kinds::IMAGE => {
                let p = props.downcast_ref::<ImageProps>().unwrap();
                let pic = gtk4::Picture::new();
                if let Ok(root) = std::env::var("DAY_ASSET_ROOT") {
                    let path = std::path::Path::new(&root).join(&p.source);
                    if path.exists() {
                        pic.set_filename(Some(&path));
                    }
                }
                pic.upcast()
            }
            _ => {
                if let Some(make) = self.registry.get(kind).map(|r| r.make) {
                    return make(self, props, id);
                }
                gtk4::Label::new(Some(&format!("⟨{kind}⟩"))).upcast()
            }
        }
    }

    fn update(
        &mut self,
        h: &Handle,
        kind: PieceKind,
        patch: &dyn std::any::Any,
        _anim: Option<&AnimSpec>,
    ) {
        match kind {
            kinds::NAV_MENU => {
                if let Some(NavMenuPatch::Selected(sel)) = patch.downcast_ref::<NavMenuPatch>() {
                    NAV_MENUS.with(|m| {
                        let m = m.borrow();
                        let Some(state) = m.get(&widget_key(h)) else {
                            return;
                        };
                        state.suppress.set(true);
                        match sel {
                            Some(i) => state
                                .listbox
                                .select_row(state.listbox.row_at_index(*i as i32).as_ref()),
                            None => state.listbox.unselect_all(),
                        }
                        state.suppress.set(false);
                    });
                }
            }
            kinds::TABS => {
                if let Some(TabsPatch::Selected(i)) = patch.downcast_ref::<TabsPatch>() {
                    TABS_STATE.with(|m| {
                        if let Some(state) = m.borrow().get(&widget_key(h)) {
                            state.suppress.set(true);
                            state.notebook.set_current_page(Some(*i as u32));
                            state.suppress.set(false);
                        }
                    });
                }
            }
            kinds::NAV => {
                if let Some(p) = patch.downcast_ref::<NavPatch>() {
                    NAV_STATE.with(|m| {
                        let m = m.borrow();
                        let Some(state) = m.get(&widget_key(h)) else {
                            return;
                        };
                        // Structure (sidebar / content / push) is driven from insert & remove;
                        // Popped drives the stack's day-initiated pop (suppressing its echo).
                        if let (NavPatch::Popped, NavPresent::Stack(nv)) = (p, &state.present) {
                            state.suppress.set(true);
                            nv.pop();
                            state.suppress.set(false);
                        }
                    });
                }
            }
            kinds::LABEL => {
                if let (Some(p), Some(label)) = (
                    patch.downcast_ref::<LabelPatch>(),
                    h.downcast_ref::<gtk4::Label>(),
                ) {
                    match p {
                        LabelPatch::Text(t) => {
                            if label.text() != t.as_str() {
                                label.set_text(t);
                            }
                        }
                        LabelPatch::Font(f) => apply_font(label, *f),
                        LabelPatch::Color(_) => {}
                    }
                }
            }
            kinds::BUTTON => {
                if let (Some(p), Some(btn)) = (
                    patch.downcast_ref::<ButtonPatch>(),
                    h.downcast_ref::<gtk4::Button>(),
                ) {
                    match p {
                        ButtonPatch::Title(t) => btn.set_label(t),
                        ButtonPatch::Enabled(e) => btn.set_sensitive(*e),
                    }
                }
            }
            kinds::TOGGLE => {
                if let (Some(p), Some(sw)) = (
                    patch.downcast_ref::<TogglePatch>(),
                    h.downcast_ref::<gtk4::Switch>(),
                ) {
                    match p {
                        TogglePatch::On(on) => {
                            if sw.is_active() != *on {
                                sw.set_active(*on);
                            }
                        }
                        TogglePatch::Enabled(e) => sw.set_sensitive(*e),
                    }
                }
            }
            kinds::SLIDER => {
                if let (Some(p), Some(scale)) = (
                    patch.downcast_ref::<SliderPatch>(),
                    h.downcast_ref::<gtk4::Scale>(),
                ) {
                    match p {
                        SliderPatch::Value(v) => {
                            if (scale.value() - v).abs() > 0.001 {
                                scale.set_value(*v);
                            }
                        }
                        SliderPatch::Enabled(e) => scale.set_sensitive(*e),
                    }
                }
            }
            kinds::PROGRESS => {
                if let Some(ProgressPatch::Value(Some(v))) = patch.downcast_ref::<ProgressPatch>()
                    && let Some(bar) = h.downcast_ref::<gtk4::ProgressBar>()
                    && (bar.fraction() - v).abs() > 0.0001
                {
                    bar.set_fraction(*v);
                }
            }
            kinds::TEXT_FIELD => {
                if let (Some(p), Some(entry)) = (
                    patch.downcast_ref::<TextFieldPatch>(),
                    h.downcast_ref::<gtk4::Entry>(),
                ) {
                    match p {
                        TextFieldPatch::Text { text, from_native } => {
                            if !*from_native && entry.text() != text.as_str() {
                                entry.set_text(text);
                            }
                        }
                        TextFieldPatch::Placeholder(t) => entry.set_placeholder_text(Some(t)),
                        TextFieldPatch::Enabled(e) => entry.set_sensitive(*e),
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

    fn release(&mut self, h: Handle) {
        let key = widget_key(&h);
        NAV_MENUS.with(|m| {
            m.borrow_mut().remove(&key);
        });
        TABS_STATE.with(|m| {
            m.borrow_mut().remove(&key);
        });
        TABS_PAGE_IDS.with(|m| {
            m.borrow_mut().remove(&key);
        });
        TABS_PAGE_TITLES.with(|m| {
            m.borrow_mut().remove(&key);
        });
        TABS_PAGES.with(|s| {
            s.borrow_mut().remove(&key);
        });
        NAV_STATE.with(|m| {
            m.borrow_mut().remove(&key);
        });
        NAV_PAGE_IDS.with(|m| {
            m.borrow_mut().remove(&key);
        });
        NAV_PAGE_TITLES.with(|m| {
            m.borrow_mut().remove(&key);
        });
        // A tab page detaches from its notebook; a nav page is owned by its AdwNavigationPage
        // (already detached in `remove`); everything else lives in a GtkFixed parent.
        if let Some(notebook) = h.parent().and_then(|p| p.downcast::<gtk4::Notebook>().ok()) {
            if let Some(n) = notebook.page_num(&h) {
                notebook.remove_page(Some(n));
            }
        } else if let Some(parent) = h.parent()
            && let Some(fixed) = parent.downcast_ref::<gtk4::Fixed>()
        {
            fixed.remove(&h);
        }
    }

    fn insert(&mut self, parent: &Handle, child: &Handle, index: usize) {
        let host_key = widget_key(parent);
        // Tabs host: insert the page into the notebook with its label; the notebook owns the
        // page's layout, so day sizes the page content from tabs_sync's FrameChanged reports.
        let tabs_handled = TABS_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let Some(state) = m.get_mut(&host_key) else {
                return false;
            };
            let id = TABS_PAGE_IDS
                .with(|ids| ids.borrow().get(&widget_key(child)).copied())
                .unwrap_or(NodeId(0));
            let title = TABS_PAGE_TITLES
                .with(|t| t.borrow().get(&widget_key(child)).cloned())
                .unwrap_or_default();
            let label = gtk4::Label::new(Some(&title));
            state
                .notebook
                .insert_page(child, Some(&label), Some(index as u32));
            let at = index.min(state.pages.len());
            state.pages.insert(at, (child.clone(), id));
            if index == state.initial {
                state.suppress.set(true);
                state.notebook.set_current_page(Some(index as u32));
                state.suppress.set(false);
            }
            true
        });
        if tabs_handled {
            gtk4::glib::idle_add_local_once(move || tabs_sync(host_key));
            return;
        }
        // Nav host: wrap the page's GtkFixed in an AdwNavigationPage. Split → set sidebar
        // (index 0) / content; stack → push onto the navigation view.
        let handled = NAV_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let Some(state) = m.get_mut(&host_key) else {
                return false;
            };
            let id = NAV_PAGE_IDS
                .with(|ids| ids.borrow().get(&widget_key(child)).copied())
                .unwrap_or(NodeId(0));
            let title = NAV_PAGE_TITLES
                .with(|t| t.borrow().get(&widget_key(child)).cloned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "day".to_string());
            let nav_page = adw::NavigationPage::new(child, &title);
            match &state.present {
                NavPresent::Split(sv) => {
                    if state.split && index == 0 {
                        sv.set_sidebar(Some(&nav_page));
                    } else {
                        sv.set_content(Some(&nav_page));
                    }
                }
                NavPresent::Stack(nv) => {
                    state.suppress.set(true);
                    nv.push(&nav_page);
                    state.suppress.set(false);
                }
            }
            state.pages.push((widget_key(child), id, nav_page));
            true
        });
        if handled {
            gtk4::glib::idle_add_local_once(move || nav_report(host_key));
        } else if let Some(fixed) = content_of(parent).downcast_ref::<gtk4::Fixed>() {
            fixed.put(child, 0.0, 0.0);
        }
    }

    fn remove(&mut self, parent: &Handle, child: &Handle) {
        let handled = NAV_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let Some(state) = m.get_mut(&widget_key(parent)) else {
                return false;
            };
            let key = widget_key(child);
            if let Some(pos) = state.pages.iter().position(|(k, _, _)| *k == key) {
                let (_, _, nav_page) = state.pages.remove(pos);
                match &state.present {
                    // The content page is being replaced; clear it (a new one follows).
                    NavPresent::Split(sv) => sv.set_content(None::<&adw::NavigationPage>),
                    // The stack pop already removed it (day-driven pop or native gesture);
                    // dropping our ref is enough.
                    NavPresent::Stack(_) => {
                        let _ = nav_page;
                    }
                }
            }
            true
        });
        if !handled && let Some(fixed) = content_of(parent).downcast_ref::<gtk4::Fixed>() {
            fixed.remove(child);
        }
    }

    fn move_child(&mut self, _parent: &Handle, _child: &Handle, _to: usize) {
        // Absolute layout: z-order = insertion order; nothing to do for non-overlapping frames.
    }

    fn measure(&mut self, h: &Handle, kind: PieceKind, p: Proposal) -> Size {
        match kind {
            kinds::NAV_MENU => {
                let rows =
                    NAV_MENUS.with(|m| m.borrow().get(&widget_key(h)).map(|s| s.rows).unwrap_or(0));
                Size::new(
                    p.width.unwrap_or(220.0),
                    p.height.unwrap_or(rows as f64 * 36.0 + 8.0),
                )
            }
            kinds::LABEL => {
                // Height-for-width through GTK's measure protocol.
                let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
                let w = match p.width {
                    Some(pw) => (nat_w as f64).min(pw),
                    None => nat_w as f64,
                };
                let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, w.round() as i32);
                Size::new(w.ceil(), nat_h as f64)
            }
            kinds::SLIDER => {
                let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
                Size::new(p.width.unwrap_or(180.0), (nat_h as f64).max(24.0))
            }
            kinds::TEXT_FIELD => {
                let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
                Size::new(p.width.unwrap_or(180.0), (nat_h as f64).max(24.0))
            }
            kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 1.0),
            kinds::PROGRESS => {
                if h.downcast_ref::<gtk4::Spinner>().is_some() {
                    Size::new(20.0, 20.0)
                } else {
                    let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
                    Size::new(p.width.unwrap_or(180.0), (nat_h as f64).max(6.0))
                }
            }
            _ => {
                if let Some(measure) = self.registry.get(kind).and_then(|r| r.measure) {
                    return measure(self, h, p);
                }
                let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
                let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
                Size::new(nat_w as f64, nat_h as f64)
            }
        }
    }

    fn set_frame(&mut self, h: &Handle, frame: Rect, _anim: Option<&AnimSpec>) {
        let key = widget_key(h);
        // Tab pages / nav pages are laid out by their native container, not by day; skip them.
        if TABS_PAGES.with(|s| s.borrow().contains(&key))
            || NAV_PAGE_IDS.with(|m| m.borrow().contains_key(&key))
        {
            return;
        }
        if let Some(parent) = h.parent()
            && let Some(fixed) = parent.downcast_ref::<gtk4::Fixed>()
        {
            fixed.move_(h, frame.origin.x, frame.origin.y);
        }
        h.set_size_request(
            frame.size.width.round() as i32,
            frame.size.height.round() as i32,
        );
        // Nav / tabs host resized (window resize): re-report page sizes for relayout.
        // GTK allocates asynchronously — defer one idle so size/position settle.
        let is_nav = NAV_STATE.with(|m| m.borrow().contains_key(&key));
        if is_nav {
            gtk4::glib::idle_add_local_once(move || nav_report(key));
        }
        let is_tabs = TABS_STATE.with(|m| m.borrow().contains_key(&key));
        if is_tabs {
            gtk4::glib::idle_add_local_once(move || tabs_sync(key));
        }
    }

    fn set_scroll_content(&mut self, h: &Handle, content: Size) {
        let inner = content_of(h);
        if inner.as_ptr() != h.as_ptr() {
            inner.set_size_request(content.width.round() as i32, content.height.round() as i32);
        }
    }

    fn set_event_sink(&mut self, sink: EventSink) {
        SINK.with(|s| *s.borrow_mut() = Some(Rc::from(sink)));
    }

    fn set_a11y(&mut self, h: &Handle, a11y: &A11yProps) {
        if let Some(id) = &a11y.identifier {
            h.set_widget_name(id); // GtkInspector-visible only (§13's honest table)
        }
        if let Some(label) = &a11y.label {
            // Full GtkAccessible property plumbing lands with M6; tooltip is the M3 stopgap.
            h.set_tooltip_text(Some(label));
        }
    }

    fn replay(&mut self, h: &Handle, ops: &[DrawOp], _size: Size) {
        OPS.with(|m| m.borrow_mut().insert(h.as_ptr() as usize, ops.to_vec()));
        h.queue_draw();
    }

    fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
        let fixed = self.window_fixed.as_ref().ok_or("no window")?;
        let widget: &gtk4::Widget = fixed.upcast_ref();
        let paintable = gtk4::WidgetPaintable::new(Some(widget));
        let w = widget.width() as f64;
        let h = widget.height() as f64;
        if w <= 0.0 || h <= 0.0 {
            return Err("zero-size window".into());
        }
        let snapshot = gtk4::Snapshot::new();
        use gtk4::gdk::prelude::PaintableExt;
        paintable.snapshot(&snapshot, w, h);
        let node = snapshot.to_node().ok_or("empty render node")?;
        let native = widget.native().ok_or("no native")?;
        let renderer = native.renderer().ok_or("no renderer")?;
        let texture = renderer.render_texture(&node, None);
        Ok(texture.save_to_png_bytes().to_vec())
    }

    fn present(&mut self, req: u64, spec: &day_spec::present::PresentSpec) {
        use day_spec::present::{PresentResult, PresentSpec};
        let parent = self
            .window_fixed
            .as_ref()
            .and_then(|f| f.root())
            .and_downcast::<gtk4::Window>();
        match spec {
            PresentSpec::Dialog {
                title,
                message,
                buttons,
                ..
            } => {
                let dialog = gtk4::AlertDialog::builder().modal(true).build();
                dialog.set_message(title);
                if let Some(m) = message {
                    dialog.set_detail(m);
                }
                let labels: Vec<&str> = buttons.iter().map(|b| b.label.as_str()).collect();
                dialog.set_buttons(&labels);
                if let Some(i) = buttons
                    .iter()
                    .position(|b| b.role == day_spec::present::ButtonRole::Cancel)
                {
                    dialog.set_cancel_button(i as i32);
                }
                let cancellable = gtk4::gio::Cancellable::new();
                NAV_DIALOGS.with(|m| {
                    m.borrow_mut()
                        .insert(req, DismissHandle::Alert(cancellable.clone()))
                });
                dialog.choose(
                    parent.as_ref(),
                    Some(&cancellable),
                    move |res: Result<i32, gtk4::glib::Error>| {
                        let result = match res {
                            Ok(i) => PresentResult::Button(i as i64),
                            Err(_) => PresentResult::Dismissed,
                        };
                        emit(day_spec::WINDOW_NODE, Event::PresentResult { req, result });
                        NAV_DIALOGS.with(|m| {
                            m.borrow_mut().remove(&req);
                        });
                    },
                );
            }
            PresentSpec::Prompt {
                title,
                message,
                placeholder,
                initial,
                ok,
                cancel,
            } => {
                // GTK has no native text prompt — a small modal window with an entry.
                let win = gtk4::Window::builder()
                    .modal(true)
                    .title(title)
                    .default_width(320)
                    .build();
                if let Some(p) = &parent {
                    win.set_transient_for(Some(p));
                }
                let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
                vbox.set_margin_top(12);
                vbox.set_margin_bottom(12);
                vbox.set_margin_start(12);
                vbox.set_margin_end(12);
                if let Some(m) = message {
                    vbox.append(&gtk4::Label::new(Some(m)));
                }
                let entry = gtk4::Entry::new();
                entry.set_placeholder_text(Some(placeholder));
                entry.set_text(initial);
                vbox.append(&entry);
                let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
                hbox.set_halign(gtk4::Align::End);
                let cancel_btn = gtk4::Button::with_label(cancel);
                let ok_btn = gtk4::Button::with_label(ok);
                hbox.append(&cancel_btn);
                hbox.append(&ok_btn);
                vbox.append(&hbox);
                win.set_child(Some(&vbox));

                let answered = Rc::new(std::cell::Cell::new(false));
                let finish = {
                    let (win, answered) = (win.clone(), answered.clone());
                    move |result: PresentResult| {
                        if answered.replace(true) {
                            return;
                        }
                        emit(day_spec::WINDOW_NODE, Event::PresentResult { req, result });
                        NAV_DIALOGS.with(|m| {
                            m.borrow_mut().remove(&req);
                        });
                        win.close();
                    }
                };
                {
                    let (finish, entry) = (finish.clone(), entry.clone());
                    ok_btn.connect_clicked(move |_| {
                        finish(PresentResult::Text(entry.text().to_string()))
                    });
                }
                {
                    let finish = finish.clone();
                    cancel_btn.connect_clicked(move |_| finish(PresentResult::Dismissed));
                }
                {
                    let finish = finish.clone();
                    win.connect_close_request(move |_| {
                        finish(PresentResult::Dismissed);
                        gtk4::glib::Propagation::Proceed
                    });
                }
                NAV_DIALOGS.with(|m| {
                    m.borrow_mut()
                        .insert(req, DismissHandle::Prompt(win.clone()))
                });
                win.present();
            }
        }
    }

    fn dismiss(&mut self, req: u64) {
        if let Some(handle) = NAV_DIALOGS.with(|m| m.borrow_mut().remove(&req)) {
            match handle {
                DismissHandle::Alert(c) => c.cancel(),
                DismissHandle::Prompt(w) => w.close(),
            }
        }
    }
}

enum DismissHandle {
    Alert(gtk4::gio::Cancellable),
    Prompt(gtk4::Window),
}

thread_local! {
    /// Live modals keyed by request id (for programmatic dismissal).
    static NAV_DIALOGS: RefCell<HashMap<u64, DismissHandle>> = RefCell::new(HashMap::new());
}

impl Platform for Gtk {
    const TARGET: &'static str = if cfg!(target_os = "macos") {
        "macos-gtk"
    } else {
        "linux-gtk"
    };
    const TOOLKIT: &'static str = "gtk";

    fn run(self, options: WindowOptions, ready: Box<dyn FnOnce(Self, Handle, Size)>) {
        // AdwApplication initialises libadwaita and loads the Adwaita stylesheet, so
        // AdwNavigationSplitView / AdwNavigationView render with the GNOME treatment.
        let app = adw::Application::builder()
            .application_id("dev.day.app")
            .build();
        let state = RefCell::new(Some((self, ready, options)));
        // Take on first activate (FnOnce payload inside an Fn handler).
        app.connect_activate(move |app| {
            let Some((mut backend, ready, options)) = state.borrow_mut().take() else {
                return;
            };
            let window = gtk4::ApplicationWindow::new(app);
            window.set_title(Some(&options.title));
            window.set_default_size(options.size.width as i32, options.size.height as i32);
            let fixed = gtk4::Fixed::new();
            // A GtkFixed reports its children's bounding box as its MINIMUM size, which
            // would pin the window at the content size. A scroll wrapper with External
            // policy breaks that propagation (no scrollbars are ever shown — day sizes
            // the content to the window on every resize).
            let wrapper = gtk4::ScrolledWindow::new();
            wrapper.set_policy(gtk4::PolicyType::External, gtk4::PolicyType::External);
            wrapper.set_child(Some(&fixed));
            window.set_child(Some(&wrapper));
            backend.window_fixed = Some(fixed.clone());
            ready(backend, fixed.upcast(), options.size);
            // GTK4 keeps default-width/height tracking the live size of a resizable
            // window — the only public resize signal it offers.
            let report = |w: &gtk4::ApplicationWindow| {
                emit(
                    day_spec::WINDOW_NODE,
                    Event::WindowResized(Size::new(
                        w.default_width() as f64,
                        w.default_height() as f64,
                    )),
                );
            };
            window.connect_default_width_notify(report);
            window.connect_default_height_notify(report);
            window.present();
        });
        app.run_with_args::<&str>(&[]);
    }

    fn post(f: Box<dyn FnOnce() + Send>) {
        gtk4::glib::MainContext::default().invoke(f);
    }
}

use day_spec::WindowOptions;
