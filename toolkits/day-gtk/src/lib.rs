//! day-gtk — the GTK 4 backend (linux-gtk / macos-gtk; DESIGN.md §9). gtk4-rs, pure Rust.
//!
//! `Handle = gtk4::Widget` (GObject-refcounted, `!Send`). Containers are `GtkFixed`; Day's
//! layout positions children via `fixed.move_()` + `set_size_request` (hop's proven pattern).
//! Native signals connect once at realize, capturing the NodeId and emitting into the Day sink.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use libadwaita as adw;
// AdwApplicationWindow / AdwToolbarView / AdwAlertDialog / AdwDialog / AdwViewStack methods live
// on extension traits (unlike the final Adw*Navigation* widgets, whose methods are inherent).
use adw::prelude::*;
use linkme::distributed_slice;

use day_spec::props::*;
use day_spec::{
    A11yProps, AnimSpec, Cap, DrawOp, Event, EventSink, Font, ListSource, NodeId, PieceKind,
    Platform, Proposal, RawHandle, Rect, Registry, Renderer, Size, Support, Toolkit, kinds,
};

pub type Handle = gtk4::Widget;

/// The day-core event sink (node-id keyed).
type Sink = Rc<dyn Fn(NodeId, Event)>;

thread_local! {
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
    static OPS: RefCell<HashMap<usize, Vec<DrawOp>>> = RefCell::new(HashMap::new());
    /// (widget_ptr, is_drag) pairs already wired, so enable_gesture is idempotent.
    static GESTURES: RefCell<std::collections::HashSet<(usize, bool)>> =
        RefCell::new(std::collections::HashSet::new());
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
            8 => {
                cr.save().ok();
            }
            9 => {
                cr.restore().ok();
            }
            10 => {
                // Packed affine (a,b,c,d,tx,ty); cairo Matrix is (xx,yx,xy,yy,x0,y0) with the
                // same row-vector meaning as day_geometry::Affine.
                let m = gtk4::cairo::Matrix::new(a, b, c, d, e, f);
                cr.transform(m);
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
// AdwNavigationPage; Day sizes content from the host width via FrameChanged (nav_report).
// ---------------------------------------------------------------------------

/// The sidebar's fixed width in the split view (Day sizes detail content = host − this).
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
    /// NAV_PAGE widget → its Day node id (recorded at realize, joined at insert).
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
// Tabs (docs/tabs.md): the Adwaita view-switching pattern — an AdwViewSwitcher above
// an AdwViewStack (the libadwaita counterpart to a stock GtkNotebook), wrapped in a box.
// Day's tabs are label-only, so the switcher is a `.linked` row of grouped toggle buttons —
// the Adwaita segmented-control idiom — rather than an icon-oriented AdwViewSwitcher.
// ---------------------------------------------------------------------------

struct TabsState {
    /// The AdwViewStack holding the page containers (the box also carries the switcher above it).
    stack: adw::ViewStack,
    /// The `.linked` box of grouped toggle buttons above the stack (the segmented switcher).
    switcher: gtk4::Box,
    /// One grouped toggle button per tab, in tab order (drives + reflects the selection).
    toggles: Vec<gtk4::ToggleButton>,
    /// The tabs host node id (a toggle emits SelectionChanged against it).
    host_id: NodeId,
    /// (page widget, node id) in tab order.
    pages: Vec<(Handle, NodeId)>,
    /// Tab to select once its page exists (the stack shows the first added page by default).
    initial: usize,
    /// Programmatic selection in flight: don't re-emit SelectionChanged.
    suppress: Rc<std::cell::Cell<bool>>,
}

thread_local! {
    static TABS_STATE: RefCell<HashMap<usize, TabsState>> = RefCell::new(HashMap::new());
    /// TABS_PAGE widget → its Day node id (recorded at realize, joined at insert).
    static TABS_PAGE_IDS: RefCell<HashMap<usize, NodeId>> = RefCell::new(HashMap::new());
    /// TABS_PAGE widget → its tab label.
    static TABS_PAGE_TITLES: RefCell<HashMap<usize, String>> = RefCell::new(HashMap::new());
    /// TABS_PAGE widget keys (set_frame skips them — the view stack owns their layout).
    static TABS_PAGES: RefCell<std::collections::HashSet<usize>> =
        RefCell::new(std::collections::HashSet::new());
}

// ---------------------------------------------------------------------------
// Native recycling list (docs/list.md, §10): GtkListView + GtkSignalListItemFactory. The model
// (a GtkStringList) supplies only the row COUNT; Day fills each recycled cell's content on bind.
// ---------------------------------------------------------------------------

struct ListEntry {
    /// Backing model — sized to the row count; content comes from `bind_row`, not the strings.
    model: gtk4::StringList,
    /// The row-pull source, injected by `attach_list` and read by the factory's `bind` handler.
    source: Rc<RefCell<Option<ListSource>>>,
}

thread_local! {
    /// LIST scrolled-window key → its model + source holder.
    static LIST_STATE: RefCell<HashMap<usize, ListEntry>> = RefCell::new(HashMap::new());
}

/// Resize the backing model to `n` rows (content is irrelevant — bind_row provides it).
fn list_resize(model: &gtk4::StringList, n: usize) {
    let cur = model.n_items();
    let blanks: Vec<&str> = vec![""; n];
    model.splice(0, cur, &blanks);
}

/// Resize the model on the next main-loop turn. CRUCIAL: `splice` makes GtkListView bind visible
/// cells SYNCHRONOUSLY (unlike NSTableView's deferred reloadData), and a reload is driven from
/// inside a `with_tree` borrow — so resizing inline would re-enter `with_tree` (bind_row) and
/// panic. Deferring to an idle runs the bind after the borrow is released.
fn schedule_list_resize(model: gtk4::StringList, source: Rc<RefCell<Option<ListSource>>>) {
    gtk4::glib::idle_add_local_once(move || {
        let n = source.borrow().as_ref().map(|s| (s.len)()).unwrap_or(0);
        list_resize(&model, n);
    });
}

/// Report each tab page's content size (host minus the tab strip) so NavLayout re-lays it.
fn tabs_sync(host_key: usize) {
    let reports: Vec<(NodeId, Size)> = TABS_STATE.with(|m| {
        let m = m.borrow();
        let Some(state) = m.get(&host_key) else {
            return Vec::new();
        };
        // The switcher sits above the stack in the box, so the stack's own allocation is
        // already the content area — no tab-strip height to subtract (unlike GtkNotebook).
        let w = state.stack.width() as f64;
        let h = state.stack.height() as f64;
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

/// Point size + the style's inherent weight for a logical [`Font`]. GTK has no semantic text styles,
/// so we approximate the platform typographic scale (matching the Apple text-style sizes for cross-
/// platform consistency). Pango point sizes are rendered through the Xft DPI, which GNOME's
/// text-scaling-factor (Settings ▸ Accessibility ▸ Large Text) feeds — so these scale for accessibility.
fn gtk_style(f: Font) -> (f64, day_spec::FontWeight) {
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

fn pango_weight(w: day_spec::FontWeight) -> gtk4::pango::Weight {
    use day_spec::FontWeight as W;
    use gtk4::pango::Weight;
    match w {
        W::UltraLight => Weight::Ultralight,
        W::Thin => Weight::Thin,
        W::Light => Weight::Light,
        W::Regular => Weight::Normal,
        W::Medium => Weight::Medium,
        W::Semibold => Weight::Semibold,
        W::Bold => Weight::Bold,
        W::Heavy => Weight::Heavy,
        W::Black => Weight::Ultraheavy,
    }
}

fn apply_font(label: &gtk4::Label, spec: day_spec::FontSpec) {
    use gtk4::pango;
    let (size_pt, inherent) = gtk_style(spec.style);
    let weight = spec.weight.unwrap_or(inherent);
    // Pango attribute list (markup-free): size, weight, and italic style.
    let attrs = pango::AttrList::new();
    let mut size = pango::AttrSize::new((size_pt * pango::SCALE as f64) as i32);
    size.set_start_index(0);
    attrs.insert(size);
    let mut w = pango::AttrInt::new_weight(pango_weight(weight));
    w.set_start_index(0);
    attrs.insert(w);
    if spec.italic {
        let mut it = pango::AttrInt::new_style(pango::Style::Italic);
        it.set_start_index(0);
        attrs.insert(it);
    }
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
                // Adwaita segmented switcher: a `.linked` row of grouped toggle buttons above an
                // AdwViewStack. Toggle buttons are wired per page in `insert`.
                let stack = adw::ViewStack::new();
                stack.set_vexpand(true);
                let switcher = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
                switcher.add_css_class("linked");
                switcher.set_halign(gtk4::Align::Center);
                switcher.set_margin_top(6);
                switcher.set_margin_bottom(6);
                let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
                container.append(&switcher);
                container.append(&stack);
                let host: Handle = container.upcast();
                let key = widget_key(&host);
                let suppress = Rc::new(std::cell::Cell::new(false));
                TABS_STATE.with(|m| {
                    m.borrow_mut().insert(
                        key,
                        TabsState {
                            stack,
                            switcher,
                            toggles: Vec::new(),
                            host_id: id,
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
            kinds::LIST => {
                let p = props.downcast_ref::<ListProps>().unwrap();
                let model = gtk4::StringList::new(&[]);
                let factory = gtk4::SignalListItemFactory::new();
                factory.connect_setup(|_, item| {
                    if let Some(li) = item.downcast_ref::<gtk4::ListItem>() {
                        // Each physical cell is a GtkFixed; Day fills it via bind_row.
                        let cell = gtk4::Fixed::new();
                        cell.set_overflow(gtk4::Overflow::Visible);
                        li.set_child(Some(&cell));
                    }
                });
                let source: Rc<RefCell<Option<ListSource>>> = Rc::new(RefCell::new(None));
                factory.connect_bind({
                    let source = source.clone();
                    move |_, item| {
                        let Some(li) = item.downcast_ref::<gtk4::ListItem>() else {
                            return;
                        };
                        let pos = li.position() as usize;
                        if let Some(cell) = li.child()
                            && let Some(src) = source.borrow().as_ref()
                        {
                            (src.bind_row)(pos, cell.as_ptr() as RawHandle);
                        }
                    }
                });
                let listview = if p.selectable {
                    let sel = gtk4::SingleSelection::new(Some(model.clone()));
                    sel.set_autoselect(false);
                    sel.set_can_unselect(true);
                    sel.connect_selected_notify(move |s| {
                        let i = s.selected();
                        if i != gtk4::INVALID_LIST_POSITION {
                            emit(id, Event::SelectionChanged(i as i64));
                        }
                    });
                    gtk4::ListView::new(Some(sel), Some(factory))
                } else {
                    gtk4::ListView::new(
                        Some(gtk4::NoSelection::new(Some(model.clone()))),
                        Some(factory),
                    )
                };
                let sw = gtk4::ScrolledWindow::new();
                sw.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
                sw.set_child(Some(&listview));
                sw.set_vexpand(true);
                let host: Handle = sw.upcast();
                LIST_STATE.with(|m| {
                    m.borrow_mut()
                        .insert(widget_key(&host), ListEntry { model, source })
                });
                host
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
                        if let Some(state) = m.borrow().get(&widget_key(h))
                            && let Some(toggle) = state.toggles.get(*i)
                            && let Some((w, _)) = state.pages.get(*i)
                        {
                            state.suppress.set(true);
                            toggle.set_active(true);
                            state.stack.set_visible_child(w);
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
            kinds::LIST => {
                if let Some(ListPatch::Reload) = patch.downcast_ref::<ListPatch>() {
                    LIST_STATE.with(|m| {
                        if let Some(e) = m.borrow().get(&widget_key(h)) {
                            // Deferred: this runs inside a with_tree borrow (see schedule_list_resize).
                            schedule_list_resize(e.model.clone(), e.source.clone());
                        }
                    });
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
        LIST_STATE.with(|m| {
            m.borrow_mut().remove(&key);
        });
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
        GESTURES.with(|g| {
            let mut g = g.borrow_mut();
            g.remove(&(key, false));
            g.remove(&(key, true));
        });
        // A tab page detaches from its AdwViewStack; a nav page is owned by its AdwNavigationPage
        // (already detached in `remove`); everything else lives in a GtkFixed parent.
        if let Some(stack) = h.parent().and_then(|p| p.downcast::<adw::ViewStack>().ok()) {
            stack.remove(&h);
        } else if let Some(parent) = h.parent()
            && let Some(fixed) = parent.downcast_ref::<gtk4::Fixed>()
        {
            fixed.remove(&h);
        }
    }

    fn insert(&mut self, parent: &Handle, child: &Handle, index: usize) {
        let host_key = widget_key(parent);
        // Tabs host: insert the page into the view stack + a toggle into the switcher; the stack
        // owns the page's layout, so Day sizes the page content from tabs_sync's FrameChanged reports.
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
            let at = index.min(state.pages.len());
            // The page content lives in the view stack…
            state
                .stack
                .add_titled(child, Some(&format!("tab{index}")), &title);
            state.pages.insert(at, (child.clone(), id));
            // …and a grouped toggle button (radio behaviour) into the `.linked` switcher.
            let toggle = gtk4::ToggleButton::with_label(&title);
            if let Some(first) = state.toggles.first() {
                toggle.set_group(Some(first));
            }
            {
                let suppress = state.suppress.clone();
                let key = host_key;
                toggle.connect_toggled(move |t| {
                    if !t.is_active() || suppress.get() {
                        return;
                    }
                    // Resolve this toggle's index, show its page, and report the selection.
                    let hit = TABS_STATE.with(|m| {
                        let m = m.borrow();
                        let s = m.get(&key)?;
                        let i = s.toggles.iter().position(|x| x == t)?;
                        s.stack.set_visible_child(&s.pages[i].0);
                        Some((s.host_id, i))
                    });
                    if let Some((host_id, i)) = hit {
                        emit(host_id, Event::SelectionChanged(i as i64));
                    }
                });
            }
            state.switcher.append(&toggle);
            state.toggles.insert(at, toggle.clone());
            if index == state.initial {
                // Suppress the echo: activating the initial toggle must not write back.
                state.suppress.set(true);
                toggle.set_active(true);
                state.stack.set_visible_child(child);
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
                .unwrap_or_else(|| "Day".to_string());
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
            // The recycling list fills the space it is offered (its scroll owns overflow).
            kinds::LIST => Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0)),
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
        // Tab pages / nav pages are laid out by their native container, not by Day; skip them.
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

    fn enable_gesture(&mut self, h: &Handle, node: NodeId, kind: day_spec::GestureKind) {
        use day_spec::{DragPhase, GestureKind, Point};
        let is_drag = matches!(kind, GestureKind::Drag);
        let key = (h.as_ptr() as usize, is_drag);
        if !GESTURES.with(|g| g.borrow_mut().insert(key)) {
            return; // already wired
        }
        match kind {
            GestureKind::Drag => {
                let drag = gtk4::GestureDrag::new();
                let start = Rc::new(std::cell::Cell::new((0.0f64, 0.0f64)));
                drag.connect_drag_begin({
                    let start = start.clone();
                    move |_, x, y| {
                        start.set((x, y));
                        emit(
                            node,
                            Event::Drag {
                                phase: DragPhase::Began,
                                location: Point::new(x, y),
                                translation: Point::ZERO,
                            },
                        );
                    }
                });
                drag.connect_drag_update({
                    let start = start.clone();
                    move |_, ox, oy| {
                        let (sx, sy) = start.get();
                        emit(
                            node,
                            Event::Drag {
                                phase: DragPhase::Changed,
                                location: Point::new(sx + ox, sy + oy),
                                translation: Point::new(ox, oy),
                            },
                        );
                    }
                });
                drag.connect_drag_end({
                    let start = start.clone();
                    move |_, ox, oy| {
                        let (sx, sy) = start.get();
                        emit(
                            node,
                            Event::Drag {
                                phase: DragPhase::Ended,
                                location: Point::new(sx + ox, sy + oy),
                                translation: Point::new(ox, oy),
                            },
                        );
                    }
                });
                h.add_controller(drag);
            }
            _ => {
                let click = gtk4::GestureClick::new();
                click.connect_released(move |_, _n, x, y| {
                    emit(node, Event::Tap(Point::new(x, y)));
                });
                h.add_controller(click);
            }
        }
    }

    fn attach_list(&mut self, host: &Handle, source: ListSource) {
        LIST_STATE.with(|m| {
            if let Some(e) = m.borrow().get(&widget_key(host)) {
                *e.source.borrow_mut() = Some(source);
                // Deferred off this with_tree borrow: splice binds cells synchronously (see
                // schedule_list_resize), which would otherwise re-enter with_tree via bind_row.
                schedule_list_resize(e.model.clone(), e.source.clone());
            }
        });
    }

    fn adopt(&mut self, raw: RawHandle) -> Handle {
        // A recycling GtkListView cell (a GtkFixed) — Day fills/rebinds its row content in place.
        unsafe { gtk4::glib::translate::from_glib_none(raw as *mut gtk4::ffi::GtkWidget) }
    }

    fn set_a11y(&mut self, h: &Handle, a11y: &A11yProps) {
        use gtk4::accessible::{Property, State};
        if let Some(id) = &a11y.identifier {
            h.set_widget_name(id); // GtkInspector-visible automation id (§13's honest table)
        }
        // Real GtkAccessible properties → AT-SPI (screen readers on Linux; no AT bridge on macOS,
        // §13). GtkWidget's accessible-role is fixed at construction, so Day sets label/description/
        // value here and leaves role to the widget (canvas role-setting is a follow-up).
        let mut props: Vec<Property> = Vec::new();
        if let Some(label) = &a11y.label {
            props.push(Property::Label(label.as_str()));
        }
        if let Some(hint) = &a11y.hint {
            props.push(Property::Description(hint.as_str()));
        }
        if let Some(value) = &a11y.value {
            props.push(Property::ValueText(value.as_str()));
        }
        if !props.is_empty() {
            h.update_property(&props);
        }
        if a11y.hidden {
            h.update_state(&[State::Hidden(true)]);
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
        use day_spec::present::{ButtonRole, PresentResult, PresentSpec};
        // AdwDialog presents relative to any widget inside its AdwApplicationWindow.
        let parent = self.window_fixed.clone();
        match spec {
            PresentSpec::Dialog {
                title,
                message,
                buttons,
                ..
            } => {
                let dialog = adw::AlertDialog::new(Some(title), message.as_deref());
                for (i, b) in buttons.iter().enumerate() {
                    let rid = i.to_string();
                    dialog.add_response(&rid, &b.label);
                    match b.role {
                        ButtonRole::Destructive => dialog
                            .set_response_appearance(&rid, adw::ResponseAppearance::Destructive),
                        ButtonRole::Default => {
                            dialog
                                .set_response_appearance(&rid, adw::ResponseAppearance::Suggested);
                            dialog.set_default_response(Some(&rid));
                        }
                        // Esc / tap-outside resolves to the cancel button (as on the other backends).
                        ButtonRole::Cancel => dialog.set_close_response(&rid),
                    }
                }
                let finish = dialog_finisher(req, dialog.clone());
                {
                    let finish = finish.clone();
                    dialog.connect_response(None, move |_, resp| {
                        let result = resp
                            .parse::<i64>()
                            .map(PresentResult::Button)
                            .unwrap_or(PresentResult::Dismissed);
                        finish(result);
                    });
                }
                NAV_DIALOGS.with(|m| m.borrow_mut().insert(req, DialogHandle { finish }));
                dialog.present(parent.as_ref());
            }
            PresentSpec::Prompt {
                title,
                message,
                placeholder,
                initial,
                ok,
                cancel,
            } => {
                // The Adwaita text prompt: an AdwAlertDialog with the entry as its extra child.
                let dialog = adw::AlertDialog::new(Some(title), message.as_deref());
                let entry = gtk4::Entry::new();
                entry.set_placeholder_text(Some(placeholder));
                entry.set_text(initial);
                entry.set_activates_default(true);
                dialog.set_extra_child(Some(&entry));
                dialog.add_response("cancel", cancel);
                dialog.set_close_response("cancel");
                dialog.add_response("ok", ok);
                dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
                dialog.set_default_response(Some("ok"));
                let finish = dialog_finisher(req, dialog.clone());
                {
                    let finish = finish.clone();
                    let entry = entry.clone();
                    dialog.connect_response(None, move |_, resp| {
                        let result = if resp == "ok" {
                            PresentResult::Text(entry.text().to_string())
                        } else {
                            PresentResult::Dismissed
                        };
                        finish(result);
                    });
                }
                NAV_DIALOGS.with(|m| m.borrow_mut().insert(req, DialogHandle { finish }));
                dialog.present(parent.as_ref());
            }
        }
    }

    fn dismiss(&mut self, req: u64) {
        // Programmatic dismissal yields `Dismissed`; the finisher's guard makes the AdwDialog's
        // own close-response (fired by `close()`) a no-op, so no button result leaks out.
        let handle = NAV_DIALOGS.with(|m| m.borrow_mut().remove(&req));
        if let Some(handle) = handle {
            (handle.finish)(day_spec::present::PresentResult::Dismissed);
        }
    }
}

/// A live modal's resolver: emits the first result only, then closes the AdwDialog (whose
/// close-response re-enters the finisher guarded, and no-ops).
struct DialogHandle {
    finish: Rc<dyn Fn(day_spec::present::PresentResult)>,
}

fn dialog_finisher(
    req: u64,
    dialog: adw::AlertDialog,
) -> Rc<dyn Fn(day_spec::present::PresentResult)> {
    let answered = Rc::new(std::cell::Cell::new(false));
    Rc::new(move |result| {
        if answered.replace(true) {
            return;
        }
        emit(day_spec::WINDOW_NODE, Event::PresentResult { req, result });
        NAV_DIALOGS.with(|m| {
            m.borrow_mut().remove(&req);
        });
        dialog.close();
    })
}

thread_local! {
    /// Live modals keyed by request id (for programmatic dismissal).
    static NAV_DIALOGS: RefCell<HashMap<u64, DialogHandle>> = RefCell::new(HashMap::new());
}

/// Adwaita's default header-bar height, used to size Day's content area before the header is
/// first allocated (`report_content_size` reads the real height thereafter).
const HEADER_H: f64 = 47.0;

/// Report Day's content area (the window minus its AdwHeaderBar) on every window resize.
fn report_content_size(w: &adw::ApplicationWindow, header: &adw::HeaderBar) {
    let hb = header.height();
    let hb = if hb > 0 { hb as f64 } else { HEADER_H };
    emit(
        day_spec::WINDOW_NODE,
        Event::WindowResized(Size::new(
            w.default_width() as f64,
            (w.default_height() as f64 - hb).max(0.0),
        )),
    );
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
            .application_id("dev.daybrite.day.app")
            .build();
        let state = RefCell::new(Some((self, ready, options)));
        // Take on first activate (FnOnce payload inside an Fn handler).
        app.connect_activate(move |app| {
            let Some((mut backend, ready, options)) = state.borrow_mut().take() else {
                return;
            };
            let window = adw::ApplicationWindow::new(app);
            window.set_title(Some(&options.title));
            window.set_default_size(options.size.width as i32, options.size.height as i32);
            let fixed = gtk4::Fixed::new();
            // A GtkFixed reports its children's bounding box as its MINIMUM size, which
            // would pin the window at the content size. A scroll wrapper with External
            // policy breaks that propagation (no scrollbars are ever shown — Day sizes
            // the content to the window on every resize).
            let wrapper = gtk4::ScrolledWindow::new();
            wrapper.set_policy(gtk4::PolicyType::External, gtk4::PolicyType::External);
            wrapper.set_child(Some(&fixed));
            // AdwApplicationWindow carries no titlebar of its own; an AdwToolbarView supplies
            // an AdwHeaderBar (window controls, drag handle, and the window title) above Day's
            // content — the standard Adwaita window structure, and the AdwDialog host that
            // AdwAlertDialog needs.
            let header = adw::HeaderBar::new();
            let toolbar = adw::ToolbarView::new();
            toolbar.add_top_bar(&header);
            toolbar.set_content(Some(&wrapper));
            window.set_content(Some(&toolbar));
            backend.window_fixed = Some(fixed.clone());
            // Day's content area is the window height minus the header bar; estimate it until
            // the header is allocated (report_content_size reads the real height thereafter).
            ready(
                backend,
                fixed.upcast(),
                Size::new(
                    options.size.width,
                    (options.size.height - HEADER_H).max(0.0),
                ),
            );
            // GTK4 keeps default-width/height tracking the live size of a resizable window —
            // the only public resize signal it offers.
            {
                let header = header.clone();
                window.connect_default_width_notify(move |w| report_content_size(w, &header));
            }
            {
                let header = header.clone();
                window.connect_default_height_notify(move |w| report_content_size(w, &header));
            }
            window.present();
        });
        app.run_with_args::<&str>(&[]);
    }

    fn post(f: Box<dyn FnOnce() + Send>) {
        gtk4::glib::MainContext::default().invoke(f);
    }
}

use day_spec::WindowOptions;
