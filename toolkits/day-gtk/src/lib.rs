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

pub mod ext;
pub use ext::*;

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
            // Rounded rect (2 fill / 13 stroke): cairo has no primitive, so trace the four corner
            // arcs (radius clamped to half the short side).
            2 | 13 => {
                let r = e.min(c / 2.0).min(d / 2.0).max(0.0);
                use std::f64::consts::FRAC_PI_2;
                cr.new_sub_path();
                cr.arc(a + c - r, b + r, r, -FRAC_PI_2, 0.0);
                cr.arc(a + c - r, b + d - r, r, 0.0, FRAC_PI_2);
                cr.arc(a + r, b + d - r, r, FRAC_PI_2, 2.0 * FRAC_PI_2);
                cr.arc(a + r, b + r, r, 2.0 * FRAC_PI_2, 3.0 * FRAC_PI_2);
                cr.close_path();
                if k == 2 {
                    let _ = cr.fill();
                } else {
                    cr.set_line_width(g);
                    let _ = cr.stroke();
                }
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
            // Polygon (11 fill / 12 stroke): points ride the texts channel as "x,y x,y …".
            11 | 12 => {
                let pts = texts.get(ti).cloned().unwrap_or_default();
                ti += 1;
                let mut first = true;
                for pair in pts.split(' ') {
                    if let Some((x, y)) = pair.split_once(',')
                        && let (Ok(x), Ok(y)) = (x.parse::<f64>(), y.parse::<f64>())
                    {
                        if first {
                            cr.move_to(x, y);
                            first = false;
                        } else {
                            cr.line_to(x, y);
                        }
                    }
                }
                if !first {
                    cr.close_path();
                    if k == 11 {
                        let _ = cr.fill();
                    } else {
                        cr.set_line_width(g);
                        let _ = cr.stroke();
                    }
                }
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
// Menus (§ menus): render day's MenuItem model as a GMenu (GtkPopoverMenu for context menus,
// GtkPopoverMenuBar for the app menu). Custom items → SimpleActions under the "daymenu" prefix that
// emit Event::MenuAction; role items → the widget's stock action (clipboard.copy, …).
// ---------------------------------------------------------------------------

thread_local! {
    /// Keep each context-menu popover alive + parented to its widget (widget ptr → popover).
    static MENU_POPOVERS: RefCell<HashMap<usize, gtk4::PopoverMenu>> = RefCell::new(HashMap::new());
}

/// A day `MenuRole` → a GTK stock action targeting the focused widget's built-in behavior. `None` =
/// GTK has no widget action for it (the role item is then omitted from a context menu).
fn gtk_role_action(role: day_spec::MenuRole) -> Option<&'static str> {
    use day_spec::MenuRole as R;
    Some(match role {
        R::Cut => "clipboard.cut",
        R::Copy => "clipboard.copy",
        R::Paste => "clipboard.paste",
        R::SelectAll => "selection.select-all",
        R::Undo => "text.undo",
        R::Redo => "text.redo",
        // App/window-scoped standard commands. `app.quit` is registered on the GtkApplication in
        // `Platform::run`; `window.close`/`window.minimize` are GTK's built-in window actions.
        R::Quit => "app.quit",
        R::CloseWindow => "window.close",
        R::Minimize => "window.minimize",
        _ => return None,
    })
}

fn gtk_role_label(role: day_spec::MenuRole) -> &'static str {
    use day_spec::MenuRole as R;
    match role {
        R::Cut => "Cut",
        R::Copy => "Copy",
        R::Paste => "Paste",
        R::SelectAll => "Select All",
        R::Undo => "Undo",
        R::Redo => "Redo",
        R::Delete => "Delete",
        R::About => "About",
        R::Quit => "Quit",
        R::Preferences => "Preferences",
        R::Minimize => "Minimize",
        R::CloseWindow => "Close",
        R::Fullscreen => "Full Screen",
    }
}

/// GTK accelerator string, e.g. `<Primary>s`, `<Primary><Shift>s`.
fn accel_string(s: &day_spec::Shortcut) -> String {
    let mut acc = String::new();
    if s.primary {
        acc.push_str("<Primary>");
    }
    if s.shift {
        acc.push_str("<Shift>");
    }
    if s.alt {
        acc.push_str("<Alt>");
    }
    if s.control {
        acc.push_str("<Control>");
    }
    let key = match s.key.as_str() {
        "Return" | "Enter" => "Return".to_string(),
        "Delete" | "Backspace" => "Delete".to_string(),
        "Space" => "space".to_string(),
        k if k.chars().count() == 1 => k.to_lowercase(),
        k => k.to_string(),
    };
    format!("{acc}{key}")
}

fn build_gio_menu(
    items: &[day_spec::MenuItem],
    group: &gtk4::gio::SimpleActionGroup,
) -> gtk4::gio::Menu {
    use day_spec::MenuItem as MI;
    use gtk4::glib::variant::ToVariant;
    use gtk4::prelude::*;
    let menu = gtk4::gio::Menu::new();
    let mut section = gtk4::gio::Menu::new();
    for item in items {
        match item {
            MI::Separator => {
                if section.n_items() > 0 {
                    menu.append_section(None, &section);
                    section = gtk4::gio::Menu::new();
                }
            }
            MI::Submenu { label, items } => {
                section.append_submenu(Some(label), &build_gio_menu(items, group));
            }
            MI::Action {
                id,
                label,
                shortcut,
                enabled,
                role,
            } => {
                if *id != 0 {
                    let name = format!("a{id}");
                    let action = gtk4::gio::SimpleAction::new(&name, None);
                    action.set_enabled(*enabled);
                    let aid = *id;
                    action.connect_activate(move |_, _| {
                        emit(day_spec::WINDOW_NODE, Event::MenuAction(aid));
                    });
                    group.add_action(&action);
                    let mi =
                        gtk4::gio::MenuItem::new(Some(label), Some(&format!("daymenu.{name}")));
                    if let Some(sc) = shortcut {
                        mi.set_attribute_value("accel", Some(&accel_string(sc).to_variant()));
                    }
                    section.append_item(&mi);
                } else if let Some(r) = role {
                    let lbl = if label.is_empty() {
                        gtk_role_label(*r)
                    } else {
                        label.as_str()
                    };
                    if let Some(act) = gtk_role_action(*r) {
                        section.append(Some(lbl), Some(act));
                    }
                }
            }
        }
    }
    if section.n_items() > 0 {
        menu.append_section(None, &section);
    }
    menu
}

/// Register app-menu accelerators on the GtkApplication so shortcuts fire even without opening the bar.
fn set_menu_accels(app: &gtk4::Application, items: &[day_spec::MenuItem]) {
    use day_spec::MenuItem as MI;
    use gtk4::prelude::*;
    for item in items {
        match item {
            MI::Submenu { items, .. } => set_menu_accels(app, items),
            MI::Action {
                id,
                shortcut: Some(sc),
                ..
            } if *id != 0 => {
                let accel = accel_string(sc);
                app.set_accels_for_action(&format!("daymenu.a{id}"), &[accel.as_str()]);
            }
            _ => {}
        }
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
    Split(gtk4::Paned),
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

thread_local! {
    /// Per-widget CSS provider for `background`/`corner_radius` surfaces, keyed by widget ptr, so
    /// a reactive background repaints by reloading the SAME provider (no provider accumulation).
    static SURFACE: RefCell<HashMap<usize, gtk4::CssProvider>> = RefCell::new(HashMap::new());
}

/// Apply a `background`/`corner_radius` surface to a container widget via a scoped CSS provider
/// (a unique `.day-surface-N` class added to just this widget). `overflow: hidden` rounds the
/// child clip. Idempotent — reuses the provider on a reactive background patch.
fn apply_surface(w: &Handle, bg: Option<day_spec::Color>, corner_radius: f64, clips: bool) {
    let key = widget_key(w);
    let class = format!("day-surface-{key}");
    let provider = SURFACE.with(|m| {
        m.borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                let p = gtk4::CssProvider::new();
                if let Some(display) = gtk4::gdk::Display::default() {
                    gtk4::style_context_add_provider_for_display(
                        &display,
                        &p,
                        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
                    );
                }
                w.add_css_class(&class);
                p
            })
            .clone()
    });
    let mut body = String::new();
    if let Some(c) = bg {
        body.push_str(&format!(
            "background-color: rgba({},{},{},{});",
            (c.r * 255.0).round() as u32,
            (c.g * 255.0).round() as u32,
            (c.b * 255.0).round() as u32,
            c.a
        ));
    }
    if corner_radius > 0.0 {
        body.push_str(&format!("border-radius: {corner_radius}px;"));
    }
    provider.load_from_data(&format!(".{class} {{ {body} }}"));
    if clips || corner_radius > 0.0 {
        w.set_overflow(gtk4::Overflow::Hidden);
    }
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
            NavPresent::Split(paned) => (paned.width() as f64, paned.height() as f64),
            NavPresent::Stack(nv) => (nv.width() as f64, nv.height() as f64),
        };
        if hw <= 0.0 || hh <= 0.0 {
            return Vec::new();
        }
        // Split: the divider is user-draggable, so report the paned's CURRENT position (falling
        // back to the default width before the first allocation) — Day re-lays each pane's
        // content to the reported size on every drag.
        let sidebar_w = match &state.present {
            NavPresent::Split(paned) => {
                let pos = paned.position() as f64;
                if pos > 0.0 { pos } else { NAV_SIDEBAR_W }
            }
            NavPresent::Stack(_) => 0.0,
        };
        state
            .pages
            .iter()
            .enumerate()
            .map(|(i, (_, id, _))| {
                let size = if state.split {
                    if i == 0 {
                        Size::new(sidebar_w, hh)
                    } else {
                        Size::new((hw - sidebar_w).max(0.0), hh)
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
    /// The app menu bar, if installed — kept so a re-`set_app_menu` can replace it.
    menu_bar: Option<gtk4::PopoverMenuBar>,
}

impl Gtk {
    pub fn new() -> Self {
        register_resources();
        let mut registry = Registry::default();
        for f in RENDERERS {
            registry.register(f());
        }
        Gtk {
            registry,
            window_fixed: None,
            menu_bar: None,
        }
    }
}

/// Apply the app icon `day launch` resolved from the project's `icons/` (§18.2) to the dock /
/// taskbar. GTK4 window icons are themed-name only, so on Linux the launcher stages a hicolor
/// layout (`DAY_ICON_THEME_DIR` + `DAY_ICON_NAME`) that is added to the display's icon-theme search
/// path; on macOS GTK has no Dock integration at all, so the icon is applied straight through
/// AppKit's `NSApplication.applicationIconImage` (`DAY_APP_ICON`).
fn apply_app_icon(window: &adw::ApplicationWindow) {
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        if let Ok(icon) = std::env::var("DAY_APP_ICON")
            && let Some(mtm) = objc2::MainThreadMarker::new()
        {
            use objc2::AllocAnyThread as _;
            if let Some(img) = objc2_app_kit::NSImage::initWithContentsOfFile(
                objc2_app_kit::NSImage::alloc(),
                &objc2_foundation::NSString::from_str(&icon),
            ) {
                let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
                unsafe { app.setApplicationIconImage(Some(&img)) };
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        use gtk4::prelude::*;
        if let (Ok(dir), Ok(name)) = (
            std::env::var("DAY_ICON_THEME_DIR"),
            std::env::var("DAY_ICON_NAME"),
        ) {
            let display = gtk4::gdk::Display::default();
            if let Some(display) = display {
                let theme = gtk4::IconTheme::for_display(&display);
                theme.add_search_path(&dir);
                window.set_icon_name(Some(&name));
            }
        }
    }
}

/// Register the app's native GResource blob (§18.3) — `day build` compiles images + data into it and
/// `day launch` points `DAY_GRESOURCE` at it. Once registered, data reads go through
/// `g_resources_lookup_data` (zero-copy from the mmapped blob) via [`open_resource`], and images load
/// with `gtk_picture_new_for_resource`.
fn register_resources() {
    let Ok(path) = std::env::var("DAY_GRESOURCE") else {
        return;
    };
    let Ok(res) = gtk4::gio::Resource::load(&path) else {
        return;
    };
    gtk4::gio::resources_register(&res);
    day_spec::resource::set_resource_opener(open_resource);
}

/// `resource("name")` → the `/day/assets/<name>` GResource entry, borrowed zero-copy (the `GBytes`
/// points into the mmapped, uncompressed blob and is held as the guard).
fn open_resource(name: &str) -> Option<day_spec::resource::Resource> {
    let path = format!("/day/assets/{name}");
    let bytes =
        gtk4::gio::resources_lookup_data(&path, gtk4::gio::ResourceLookupFlags::NONE).ok()?;
    let slice: &[u8] = &bytes;
    let (ptr, len) = (slice.as_ptr(), slice.len());
    // Safety: `bytes` keeps the GResource data alive for the returned view.
    Some(unsafe { day_spec::resource::Resource::from_raw(ptr, len, Box::new(bytes)) })
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
        Font::Custom(_, pt) => (pt, Regular),
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

thread_local! {
    /// Per-label (font, color) state, keyed by widget ptr. Both render through ONE Pango
    /// attribute list (set_attributes replaces the whole list), but a `LabelPatch` carries only
    /// the half that changed — so each patch updates its half here and re-applies the pair.
    /// Entries drop in `release`.
    static LABEL_STYLE: RefCell<HashMap<usize, (day_spec::FontSpec, Option<day_spec::Color>)>> =
        RefCell::new(HashMap::new());
}

/// Rebuild a label's full Pango attribute list: font family/size/weight/style + foreground color.
fn apply_text_attrs(label: &gtk4::Label, spec: day_spec::FontSpec, color: Option<day_spec::Color>) {
    use gtk4::pango;
    let (size_pt, inherent) = gtk_style(spec.style);
    let weight = spec.weight.unwrap_or(inherent);
    // Pango attribute list (markup-free): size, weight, italic style, and foreground.
    let attrs = pango::AttrList::new();
    // A bundled family (§18.4), registered with the platform font system in run(). Pango falls
    // back to the default family if the name doesn't resolve.
    if let Font::Custom(family, _) = spec.style {
        let mut fam = pango::AttrString::new_family(family);
        fam.set_start_index(0);
        attrs.insert(fam);
    }
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
    if let Some(c) = color {
        let ch = |x: f64| (x.clamp(0.0, 1.0) * 65535.0).round() as u16;
        let mut fg = pango::AttrColor::new_foreground(ch(c.r), ch(c.g), ch(c.b));
        fg.set_start_index(0);
        attrs.insert(fg);
        if c.a < 1.0 {
            let mut alpha = pango::AttrInt::new_foreground_alpha(ch(c.a).max(1));
            alpha.set_start_index(0);
            attrs.insert(alpha);
        }
    }
    label.set_attributes(Some(&attrs));
}

/// Update one half of a label's remembered (font, color) pair and re-apply both.
fn update_text_attrs(
    label: &gtk4::Label,
    font: Option<day_spec::FontSpec>,
    color: Option<Option<day_spec::Color>>,
) {
    let key = label.clone().upcast::<gtk4::Widget>().as_ptr() as usize;
    let (spec, col) = LABEL_STYLE.with(|m| {
        let mut m = m.borrow_mut();
        let entry = m.entry(key).or_default();
        if let Some(f) = font {
            entry.0 = f;
        }
        if let Some(c) = color {
            entry.1 = c;
        }
        *entry
    });
    apply_text_attrs(label, spec, col);
}

/// Register bundled fonts (§18.4) with whatever font system Pango draws from on this OS, so
/// `Font::Custom` family names resolve: fontconfig on Linux; on macOS BOTH CoreText and
/// fontconfig (Homebrew Pango may use either fontmap depending on how it was built); GDI
/// private fonts on Windows (MSYS2 Pango, best effort). Failures log and move on — the family
/// simply won't resolve and Pango falls back to the default face.
fn register_bundled_fonts() {
    let fonts = day_spec::fonts::bundled_fonts();
    if fonts.is_empty() {
        return;
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        #[link(name = "fontconfig")]
        unsafe extern "C" {
            fn FcConfigAppFontAddFile(
                config: *mut std::ffi::c_void,
                file: *const std::ffi::c_char,
            ) -> std::ffi::c_int;
        }
        for path in &fonts {
            let Ok(c) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else {
                continue;
            };
            // NULL config = the current default configuration.
            if unsafe { FcConfigAppFontAddFile(std::ptr::null_mut(), c.as_ptr()) } == 0 {
                eprintln!("day: could not register bundled font {}", path.display());
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            fn CFURLCreateFromFileSystemRepresentation(
                alloc: *const std::ffi::c_void,
                buffer: *const u8,
                buf_len: isize,
                is_directory: bool,
            ) -> *const std::ffi::c_void;
            fn CFRelease(cf: *const std::ffi::c_void);
        }
        #[link(name = "CoreText", kind = "framework")]
        unsafe extern "C" {
            fn CTFontManagerRegisterFontsForURL(
                font_url: *const std::ffi::c_void,
                scope: u32, // kCTFontManagerScopeProcess = 1
                error: *mut *const std::ffi::c_void,
            ) -> bool;
        }
        for path in &fonts {
            let bytes = path.as_os_str().as_encoded_bytes();
            unsafe {
                let url = CFURLCreateFromFileSystemRepresentation(
                    std::ptr::null(),
                    bytes.as_ptr(),
                    bytes.len() as isize,
                    false,
                );
                if !url.is_null() {
                    // Duplicate registration (hot relaunch) fails harmlessly; fontconfig above
                    // is the loud path, so no second log line here.
                    let _ = CTFontManagerRegisterFontsForURL(url, 1, std::ptr::null_mut());
                    CFRelease(url);
                }
            }
        }
    }
    #[cfg(windows)]
    {
        #[link(name = "gdi32")]
        unsafe extern "system" {
            fn AddFontResourceExW(
                name: *const u16,
                fl: u32, // FR_PRIVATE = 0x10
                res: *mut std::ffi::c_void,
            ) -> std::ffi::c_int;
        }
        use std::os::windows::ffi::OsStrExt as _;
        for path in &fonts {
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
            if unsafe { AddFontResourceExW(wide.as_ptr(), 0x10, std::ptr::null_mut()) } == 0 {
                eprintln!("day: could not register bundled font {}", path.display());
            }
        }
    }
}

/// Verify every bundled font family resolved into the Pango fontmap GTK is actually using —
/// the loud half of §18.4's degrade-loudly rule. Pango fontmaps enumerate families at creation
/// (see the `register_bundled_fonts` call at the top of `run`), so a family missing HERE means
/// registration ran too late (or failed) and labels will silently render in the default face.
fn check_bundled_fonts(widget: &impl gtk4::prelude::IsA<gtk4::Widget>) {
    use gtk4::prelude::WidgetExt as _;
    let fonts = day_spec::fonts::bundled_fonts();
    if fonts.is_empty() {
        return;
    }
    let families: Vec<String> = widget
        .as_ref()
        .pango_context()
        .list_families()
        .iter()
        .map(|f| f.name().to_string())
        .collect();
    for path in fonts {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Some(names) = day_spec::fonts::parse_font_names(&bytes) else {
            continue;
        };
        if !families
            .iter()
            .any(|f| f.eq_ignore_ascii_case(&names.family))
        {
            eprintln!(
                "day: bundled font family {:?} ({}) did not register with Pango — labels using \
                 it will fall back to the default face",
                names.family,
                path.display()
            );
        }
    }
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

/// Warn ONCE per kind that this backend has no registered renderer for `kind`, before falling back to
/// a visible placeholder. A missing renderer usually means the piece's `gtk` feature wasn't enabled
/// (Tier A.2 derives it automatically under `day build`). Deduped per kind so a placeholder rendered
/// every frame doesn't spam the log.
fn warn_missing_renderer(kind: PieceKind) {
    static SEEN: std::sync::Mutex<Option<std::collections::HashSet<&'static str>>> =
        std::sync::Mutex::new(None);
    let Ok(mut guard) = SEEN.lock() else { return };
    if guard
        .get_or_insert_with(std::collections::HashSet::new)
        .insert(kind)
    {
        eprintln!(
            "day: no renderer for piece kind \"{kind}\" on gtk \
             — is the piece's gtk feature enabled? (rendering a placeholder)"
        );
    }
}

impl Toolkit for Gtk {
    type Handle = Handle;

    fn capability(&self, cap: Cap) -> Support {
        match cap {
            Cap::Snapshot | Cap::NavSplit | Cap::Dialogs | Cap::FileDialogs => Support::Native,
            _ => Support::Unsupported,
        }
    }

    fn realize(&mut self, kind: PieceKind, props: &dyn std::any::Any, id: NodeId) -> Handle {
        match kind {
            kinds::CONTAINER => {
                let w: Handle = gtk4::Fixed::new().upcast();
                if let Some(p) = props.downcast_ref::<ContainerProps>()
                    && (p.background.is_some() || p.corner_radius > 0.0 || p.clips)
                {
                    apply_surface(&w, p.background, p.corner_radius, p.clips);
                }
                w
            }
            kinds::NAV => {
                let is_split = props
                    .downcast_ref::<NavProps>()
                    .map(|p| p.split)
                    .unwrap_or(true);
                let suppress = Rc::new(std::cell::Cell::new(false));
                let (host, present): (Handle, NavPresent) = if is_split {
                    // GtkPaned: sidebar + detail with a USER-DRAGGABLE divider (the AppKit
                    // NSSplitView counterpart). AdwNavigationSplitView pins its sidebar width by
                    // design (GNOME HIG has no draggable sidebars), so a paned is the native way to
                    // honor divider adjustment; the sidebar list keeps the `.navigation-sidebar`
                    // treatment. Day re-lays each pane's content from the sizes reported on drag.
                    let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
                    paned.set_position(NAV_SIDEBAR_W as i32);
                    // Window resizes go to the detail pane; the sidebar holds its width.
                    paned.set_resize_start_child(false);
                    paned.set_resize_end_child(true);
                    // Day frames each pane's content to EXACTLY the last reported size, which
                    // becomes that pane's GTK minimum — with shrink forbidden the divider would
                    // be pinned in place. Allow shrinking; Day re-lays content to the new size
                    // reported on every drag (position notify → nav_report).
                    paned.set_shrink_start_child(true);
                    paned.set_shrink_end_child(true);
                    let host_key_for_report = Rc::new(std::cell::Cell::new(0usize));
                    {
                        let hk = host_key_for_report.clone();
                        paned.connect_position_notify(move |_| {
                            let key = hk.get();
                            if key != 0 {
                                gtk4::glib::idle_add_local_once(move || nav_report(key));
                            }
                        });
                    }
                    let handle: Handle = paned.clone().upcast();
                    host_key_for_report.set(widget_key(&handle));
                    (handle, NavPresent::Split(paned))
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
                // The list scrolls WITHIN the sidebar (its own scrolled window, like AppKit's
                // NSOutlineView-in-NSScrollView). Without this, the bare ListBox's minimum height
                // (all rows) propagates up to the window's sizing wrapper and a wheel over the
                // sidebar scrolls the ENTIRE window.
                let sw = gtk4::ScrolledWindow::new();
                sw.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
                sw.set_child(Some(&listbox));
                let handle: Handle = sw.upcast();
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
                update_text_attrs(&label, Some(p.font), Some(p.color));
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
                // Scaling (§18.3): GtkPicture content-fit — Contain (fit) / Cover (fill) / Fill.
                pic.set_content_fit(match p.content_mode {
                    ContentMode::Fit => gtk4::ContentFit::Contain,
                    ContentMode::Fill => gtk4::ContentFit::Cover,
                    ContentMode::Stretch => gtk4::ContentFit::Fill,
                });
                // Prefer the native GResource entry `/day/images/<name>` (§18.3); else a loose file.
                let res_path = format!("/day/images/{}", p.source);
                if gtk4::gio::resources_lookup_data(&res_path, gtk4::gio::ResourceLookupFlags::NONE)
                    .is_ok()
                {
                    pic.set_resource(Some(&res_path));
                } else if let Some(path) = day_spec::resource::resolve_image_file(&p.source) {
                    pic.set_filename(Some(&path));
                }
                pic.upcast()
            }
            _ => {
                if let Some(make) = self.registry.get(kind).map(|r| r.make) {
                    return make(self, props, id);
                }
                warn_missing_renderer(kind);
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
            kinds::CONTAINER => {
                if let Some(ContainerPatch::Background(c)) = patch.downcast_ref::<ContainerPatch>()
                {
                    apply_surface(h, *c, 0.0, false);
                }
            }
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
                        LabelPatch::Font(f) => update_text_attrs(label, Some(*f), None),
                        LabelPatch::Color(c) => update_text_attrs(label, None, Some(*c)),
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
            kinds::LIST => match patch.downcast_ref::<ListPatch>() {
                Some(ListPatch::Reload) => {
                    LIST_STATE.with(|m| {
                        if let Some(e) = m.borrow().get(&widget_key(h)) {
                            // Deferred: this runs inside a with_tree borrow (see schedule_list_resize).
                            schedule_list_resize(e.model.clone(), e.source.clone());
                        }
                    });
                }
                Some(ListPatch::ScrollToEnd) => {
                    // GtkListView::scroll_to needs v4_12; we target v4_10, so drive the scrolled
                    // window's vertical adjustment to its maximum instead. Deferred past this
                    // with_tree borrow AND past any pending reload splice so the freshly bound rows
                    // are allocated before we read the extent.
                    if let Some(sw) = h.downcast_ref::<gtk4::ScrolledWindow>() {
                        let adj = sw.vadjustment();
                        gtk4::glib::idle_add_local_once(move || {
                            adj.set_value(adj.upper() - adj.page_size());
                        });
                    }
                }
                _ => {}
            },
            _ => {
                if let Some(update) = self.registry.get(kind).map(|r| r.update) {
                    update(self, h, patch);
                }
            }
        }
    }

    fn release(&mut self, h: Handle) {
        let key = widget_key(&h);
        LABEL_STYLE.with(|m| {
            m.borrow_mut().remove(&key);
        });
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
                NavPresent::Split(paned) => {
                    if state.split && index == 0 {
                        // libadwaita's split-sidebar background treatment on the paned child.
                        nav_page.add_css_class("sidebar-pane");
                        paned.set_start_child(Some(&nav_page));
                    } else {
                        paned.set_end_child(Some(&nav_page));
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
                    NavPresent::Split(paned) => paned.set_end_child(None::<&gtk4::Widget>),
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
                // Measure the TEXT, not the widget. GtkFixed children are sized through
                // `set_size_request` (see set_frame), and `gtk_widget_measure` never reports
                // less than the current size request — so measuring the widget RATCHETS: after
                // a narrow layout requests a tall wrapped height, re-measuring at a wider width
                // keeps returning that tall height and content below never moves back up. A
                // fresh Pango layout on the label's own (styled) context measures exactly the
                // text the label renders, free of any request state.
                let label = h.downcast_ref::<gtk4::Label>().expect("label widget");
                let layout = gtk4::pango::Layout::new(&label.pango_context());
                layout.set_text(&label.text());
                layout.set_attributes(label.attributes().as_ref());
                layout.set_wrap(gtk4::pango::WrapMode::WordChar);
                let (nat_w, _) = layout.pixel_size();
                let w = match p.width {
                    Some(pw) => (nat_w as f64).min(pw),
                    None => nat_w as f64,
                };
                layout.set_width((w * gtk4::pango::SCALE as f64).round() as i32);
                let (_, nat_h) = layout.pixel_size();
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

    fn set_context_menu(&mut self, h: &Handle, _node: NodeId, items: &[day_spec::MenuItem]) {
        // Remove any prior context menu (popover + gesture) for this widget.
        MENU_POPOVERS.with(|m| {
            if let Some(pop) = m.borrow_mut().remove(&widget_key(h)) {
                pop.unparent();
            }
        });
        if items.is_empty() {
            return;
        }
        let group = gtk4::gio::SimpleActionGroup::new();
        let model = build_gio_menu(items, &group);
        h.insert_action_group("daymenu", Some(&group));
        // The label/target must be able to receive pointer events for the gesture to fire.
        h.set_can_target(true);
        let popover = gtk4::PopoverMenu::from_model(Some(&model));
        popover.set_parent(h);
        popover.set_has_arrow(false);
        let popup_at = {
            let pop = popover.clone();
            move |x: f64, y: f64| {
                pop.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                pop.popup();
            }
        };
        // Secondary (right) click on desktop…
        let click = gtk4::GestureClick::new();
        click.set_button(3);
        let f = popup_at.clone();
        click.connect_pressed(move |_, _n, x, y| f(x, y));
        h.add_controller(click);
        // …and long-press for touch/mobile.
        let long = gtk4::GestureLongPress::new();
        let f = popup_at.clone();
        long.connect_pressed(move |_, x, y| f(x, y));
        h.add_controller(long);
        MENU_POPOVERS.with(|m| m.borrow_mut().insert(widget_key(h), popover));
    }

    fn set_app_menu(&mut self, items: &[day_spec::MenuItem]) {
        let Some(app) = self
            .window_fixed
            .as_ref()
            .and_then(|f| f.root())
            .and_then(|r| r.downcast::<gtk4::Window>().ok())
            .and_then(|w| w.application())
        else {
            return;
        };
        let group = gtk4::gio::SimpleActionGroup::new();
        let model = build_gio_menu(items, &group);
        // Actions live under the "daymenu" prefix on the window; accelerators are set on the app.
        if let Some(win) = app.active_window() {
            win.insert_action_group("daymenu", Some(&group));
        }
        set_menu_accels(&app, items);
        // The Adwaita window is Window → AdwToolbarView[ AdwHeaderBar, ScrolledWindow[ fixed ] ].
        // The app menu bar is another top bar of the toolbar view, sitting under the header.
        let toolbar = self
            .window_fixed
            .as_ref()
            .and_then(|f| f.parent()) // ScrolledWindow (the sizing wrapper)
            .and_then(|w| w.parent()) // AdwToolbarView
            .and_then(|w| w.downcast::<adw::ToolbarView>().ok());
        let Some(toolbar) = toolbar else { return };
        // Replace any previously-installed bar (set_app_menu may be called again).
        if let Some(old) = self.menu_bar.take() {
            toolbar.remove(&old);
        }
        let bar = gtk4::PopoverMenuBar::from_model(Some(&model));
        toolbar.add_top_bar(&bar);
        self.menu_bar = Some(bar);
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
            // GtkFileDialog (GTK 4.10+): async open/save with a native GTK picker. The chosen
            // GFile's local path crosses back; a Cancellable lets dismiss() cancel it.
            // Presenting a modal GtkFileDialog pumps the GTK main loop (mapping its window — a
            // synchronous round-trip under a headless/xvfb display). But day-core calls this
            // `present()` WHILE holding the tree borrow (`with_tree(|t| t.present(..))` in
            // present.rs), so a loop-spin here re-enters Day (e.g. the on-main dayscript engine's
            // next step) and its `with_tree` panics "already borrowed" — inside a GTK C callback,
            // which aborts rather than unwinds. So DEFER the actual open/save to an idle: by then
            // `present()` has returned and the borrow is released, making any re-entry safe. (Same
            // reasoning as `schedule_list_resize`.)
            PresentSpec::OpenFile { title, filters } => {
                let dialog = gtk4::FileDialog::builder()
                    .title(title.as_str())
                    .modal(true)
                    .build();
                apply_gtk_filters(&dialog, filters);
                let cancellable = gtk4::gio::Cancellable::new();
                FILE_DIALOGS.with(|m| m.borrow_mut().insert(req, cancellable.clone()));
                let window = file_dialog_window(&parent);
                gtk4::glib::idle_add_local_once(move || {
                    dialog.open(window.as_ref(), Some(&cancellable), move |res| {
                        emit_file_result(req, res.map(|f| f.path()))
                    });
                });
            }
            PresentSpec::SaveFile {
                title,
                suggested_name,
                ..
            } => {
                let dialog = gtk4::FileDialog::builder()
                    .title(title.as_str())
                    .initial_name(suggested_name.as_str())
                    .modal(true)
                    .build();
                apply_gtk_filters(&dialog, spec.filters());
                let cancellable = gtk4::gio::Cancellable::new();
                FILE_DIALOGS.with(|m| m.borrow_mut().insert(req, cancellable.clone()));
                let window = file_dialog_window(&parent);
                // The pieces layer copies the staged bytes to the chosen local path.
                gtk4::glib::idle_add_local_once(move || {
                    dialog.save(window.as_ref(), Some(&cancellable), move |res| {
                        emit_file_result(req, res.map(|f| f.path()))
                    });
                });
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
        // GtkFileDialog: cancel the in-flight operation (its callback then no-ops in day-core).
        if let Some(c) = FILE_DIALOGS.with(|m| m.borrow_mut().remove(&req)) {
            c.cancel();
        }
    }
}

/// The GtkWindow to anchor a file picker on: the fixed content's toplevel.
fn file_dialog_window(parent: &Option<gtk4::Fixed>) -> Option<gtk4::Window> {
    parent
        .as_ref()
        .and_then(|f| f.root())
        .and_then(|r| r.downcast::<gtk4::Window>().ok())
}

/// Apply a file dialog's extension filters as GtkFileFilters (`*.ext` glob patterns).
fn apply_gtk_filters(dialog: &gtk4::FileDialog, filters: &[day_spec::present::FileFilter]) {
    if filters.is_empty() {
        return;
    }
    let store = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
    for f in filters {
        let ff = gtk4::FileFilter::new();
        ff.set_name(Some(&f.name));
        for ext in &f.extensions {
            ff.add_pattern(&format!("*.{ext}"));
        }
        store.append(&ff);
    }
    dialog.set_filters(Some(&store));
}

/// Turn a GtkFileDialog result into a `PresentResult` and enqueue it.
fn emit_file_result(req: u64, res: Result<Option<std::path::PathBuf>, gtk4::glib::Error>) {
    let result = match res {
        Ok(Some(path)) => {
            day_spec::present::PresentResult::Files(vec![path.to_string_lossy().into_owned()])
        }
        _ => day_spec::present::PresentResult::Dismissed,
    };
    emit(day_spec::WINDOW_NODE, Event::PresentResult { req, result });
    FILE_DIALOGS.with(|m| {
        m.borrow_mut().remove(&req);
    });
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
    /// In-flight GtkFileDialog operations keyed by request id (cancelled on dismiss).
    static FILE_DIALOGS: RefCell<HashMap<u64, gtk4::gio::Cancellable>> =
        RefCell::new(HashMap::new());
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

/// Which lifecycle phases this desktop backend delivers (docs/lifecycle.md): the universal set
/// (launch / activation / termination). GTK desktop apps have no background/foreground or
/// memory-warning concept. `const` so `day::require_lifecycle!` can reject unsupported phases at
/// compile time. Must match [`Gtk::supports_lifecycle`].
pub const fn lifecycle_supported(phase: day_spec::Lifecycle) -> bool {
    phase.is_universal()
}

impl Platform for Gtk {
    const TARGET: &'static str = if cfg!(target_os = "macos") {
        "macos-gtk"
    } else {
        "linux-gtk"
    };
    const TOOLKIT: &'static str = "gtk";

    fn run(self, options: WindowOptions, ready: Box<dyn FnOnce(Self, Handle, Size)>) {
        // Bundled custom fonts (§18.4) must be registered BEFORE any GTK/Pango initialization:
        // Pango's fontmaps (CoreText on macOS, fontconfig on Linux) enumerate the available
        // families when the fontmap is created and do NOT re-scan, so a font registered after
        // GTK init silently falls back to the default family. `check_bundled_fonts` (in
        // activate, below) verifies the families actually resolved and warns loudly if not.
        register_bundled_fonts();

        // AdwApplication initialises libadwaita and loads the Adwaita stylesheet, so
        // AdwNavigationSplitView / AdwNavigationView render with the GNOME treatment.
        let app = adw::Application::builder()
            .application_id("dev.daybrite.day.app")
            .build();

        // Standard app-level "quit" action so `MenuRole::Quit` (and the platform quit shortcut
        // ⌘Q / Ctrl+Q) actually exits — GTK provides no default quit action (docs/menus.md fix).
        // Calling `app.quit()` tears the app down, which fires `shutdown` → `WillTerminate` below,
        // so every quit path (menu item, accelerator, last-window-close) runs the same handlers.
        let quit = gtk4::gio::SimpleAction::new("quit", None);
        {
            let app = app.clone();
            quit.connect_activate(move |_, _| app.quit());
        }
        app.add_action(&quit);
        app.set_accels_for_action("app.quit", &["<Primary>q"]);

        // Lifecycle: GApplication `shutdown` is the single point every quit funnels through
        // (docs/lifecycle.md). Emit WillTerminate synchronously so handlers run before teardown.
        app.connect_shutdown(|_| {
            emit(
                day_spec::WINDOW_NODE,
                Event::Lifecycle(day_spec::Lifecycle::WillTerminate),
            );
        });

        let state = RefCell::new(Some((self, ready, options)));
        // Take on first activate (FnOnce payload inside an Fn handler).
        app.connect_activate(move |app| {
            let Some((mut backend, ready, options)) = state.borrow_mut().take() else {
                return;
            };
            let window = adw::ApplicationWindow::new(app);
            check_bundled_fonts(&window);
            window.set_title(Some(&options.title));
            window.set_default_size(options.size.width as i32, options.size.height as i32);
            apply_app_icon(&window);
            let fixed = gtk4::Fixed::new();
            // A GtkFixed reports its children's bounding box as its MINIMUM size, which
            // would pin the window at the content size. A scroll wrapper with External
            // policy breaks that propagation (no scrollbars are ever shown — Day sizes
            // the content to the window on every resize).
            let wrapper = gtk4::ScrolledWindow::new();
            wrapper.set_policy(gtk4::PolicyType::External, gtk4::PolicyType::External);
            wrapper.set_child(Some(&fixed));
            // The wrapper exists ONLY to break min-size propagation — it must never actually
            // scroll (Day sizes the content to the window). If any child's native minimum
            // exceeds the window, a wheel would otherwise pan the whole UI; pin both axes.
            for adj in [wrapper.hadjustment(), wrapper.vadjustment()] {
                adj.connect_value_changed(|a| {
                    if a.value() != 0.0 {
                        a.set_value(0.0);
                    }
                });
            }
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
            // Lifecycle activation (docs/lifecycle.md): the window's focus tracks foreground/active.
            window.connect_is_active_notify(|w| {
                let phase = if w.is_active() {
                    day_spec::Lifecycle::DidBecomeActive
                } else {
                    day_spec::Lifecycle::WillResignActive
                };
                emit(day_spec::WINDOW_NODE, Event::Lifecycle(phase));
            });
            window.present();
        });
        app.run_with_args::<&str>(&[]);
    }

    fn post(f: Box<dyn FnOnce() + Send>) {
        gtk4::glib::MainContext::default().invoke(f);
    }
}

use day_spec::WindowOptions;
