// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-gtk — the GTK 4 backend (linux-gtk / macos-gtk; DESIGN.md §9). gtk4-rs, pure Rust.
//!
//! `Handle = gtk4::Widget` (GObject-refcounted, `!Send`). Containers are `GtkFixed`; Day's
//! layout positions children via `fixed.move_()` + `set_size_request` (hop's proven pattern).
//! The one exception is a recycling list's cell, a [`DayCell`]: a custom container that takes
//! part in GTK's size negotiation instead — asks for no width, places its children itself from
//! the frames Day recorded — because inside a `GtkListView` a `set_size_request` floor would
//! drag the list wider than its frame (day#35). Measurement of a native leaf is likewise done
//! with its request cleared, or the last placement becomes a floor on the next measure.
//! Native signals connect once at realize, capturing the NodeId and emitting into the Day sink.

mod controls;

#[cfg(target_os = "macos")]
mod snapshot_macos;

mod vector;

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
use day_spec::sidetable::SideTable;
use day_spec::{
    A11yProps, AnimSpec, Animatable, Builtin, Cap, Cursor, Curve, DrawOp, Event, EventSink, Font,
    InputPurpose, InputTraits, ListSource, NodeId, PieceKind, Platform, Proposal, RawHandle, Rect,
    Registry, Renderer, Size, Support, Toolkit, Transform, TreeSource, ffi_guard, kinds, props_of,
};

pub type Handle = gtk4::Widget;

/// The GTK input purpose and hints for a field's purpose and secrecy together
/// (docs/textfield.md). GTK has one purpose slot, and input methods read `Password`/`Pin` as
/// "stay out of this field", so a secure field takes one of those whatever it collects, and
/// showing the characters again gives the app's own purpose back.
fn gtk_input_purpose(
    purpose: InputPurpose,
    secure: bool,
) -> (gtk4::InputPurpose, gtk4::InputHints) {
    use gtk4::InputHints as H;
    use gtk4::InputPurpose as P;
    // What every field of exact characters shares: nothing to correct, nothing to decorate.
    let exact = H::NO_SPELLCHECK | H::NO_EMOJI;
    let (native, hints) = match purpose {
        InputPurpose::Text => (P::FreeForm, H::NONE),
        InputPurpose::Name => (P::Name, H::NO_SPELLCHECK | H::UPPERCASE_WORDS),
        InputPurpose::Email => (P::Email, exact | H::LOWERCASE),
        InputPurpose::Url => (P::Url, exact | H::LOWERCASE),
        InputPurpose::Phone => (P::Phone, exact),
        InputPurpose::Number => (P::Digits, exact),
        InputPurpose::Decimal => (P::Number, exact),
        // No LOWERCASE here: the hint asks the input method to convert, and an account name
        // can be case-sensitive. Without an UPPERCASE hint nothing capitalizes it either.
        InputPurpose::Username => (P::FreeForm, exact),
        InputPurpose::Password | InputPurpose::NewPassword => (P::Password, exact | H::PRIVATE),
        // A code can carry letters, so it is not `Pin`.
        InputPurpose::OneTimeCode => (P::FreeForm, exact),
    };
    if !secure {
        return (native, hints);
    }
    let native = if purpose == InputPurpose::Number {
        P::Pin
    } else {
        P::Password
    };
    // The app's capitalization hints go too: nothing may rewrite hidden characters.
    (native, exact | H::PRIVATE)
}

// Built-in leaf pieces split into modules (moved in from their satellite crates 2026-07).
mod picker;
mod shell;
mod textarea;
mod toolbar;
mod transfer;

pub mod ext;
pub use ext::*;
/// For external piece renderers that build native chrome of their own (the settings shell's
/// view switcher): the window's header bar and themed icon names for Day's icon vocabulary.
pub use toolbar::{header_of, icon_name_for};

/// The day-core event sink (node-id keyed).
type Sink = Rc<dyn Fn(NodeId, Event)>;

day_core::tls_group! {
    static BUTTON_CONTENT: day_spec::sidetable::SideTable<day_spec::props::ButtonContent> = day_spec::sidetable::SideTable::new();
    static BUTTON_INK: day_spec::sidetable::SideTable<day_spec::Color> = day_spec::sidetable::SideTable::new();
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
    /// Canvas ptr → its display list, ALREADY ENCODED (`replay` writes, the draw func reads). A
    /// [`SideTable`]: the release sweep drops a dead canvas's list, so a recycled address can't
    /// briefly draw the previous canvas's ops.
    ///
    /// Encoded once per replay rather than once per draw. GTK raises a draw for reasons that have
    /// nothing to do with the content — a resize, an occlusion change, a window damage event —
    /// and re-encoding there meant formatting every path segment into a string again for a
    /// drawing that had not moved, in-process, with no boundary to cross.
    static CONTAINER_KEYS: day_spec::sidetable::SideTable<bool> = day_spec::sidetable::SideTable::new();
    static OPS: SideTable<(Vec<f64>, Vec<String>)> = SideTable::new();
    /// (widget_ptr, kind) pairs already wired, so enable_gesture is idempotent.
    static GESTURES: RefCell<std::collections::HashSet<(usize, day_spec::GestureKind)>> =
        RefCell::new(std::collections::HashSet::new());

    static TINT_PROVIDERS: RefCell<std::collections::HashSet<String>> =
        RefCell::new(std::collections::HashSet::new());

    /// Keep each context-menu popover alive + parented to its widget (widget ptr → popover).
    static MENU_POPOVERS: RefCell<HashMap<usize, gtk4::PopoverMenu>> = RefCell::new(HashMap::new());

    static NAV_STATE: RefCell<HashMap<usize, NavState>> = RefCell::new(HashMap::new());
    /// NAV_PAGE widget → its Day node id (recorded at realize, joined at insert).
    static NAV_PAGE_IDS: RefCell<HashMap<usize, NodeId>> = RefCell::new(HashMap::new());
    /// NAV_PAGE widget → its title (for the AdwNavigationPage).
    static NAV_PAGE_TITLES: RefCell<HashMap<usize, String>> = RefCell::new(HashMap::new());
    /// The Day window that was active most recently (`Toolkit::active_window`).
    static LAST_ACTIVE_WINDOW: RefCell<Option<gtk4::glib::WeakRef<gtk4::Window>>> = const { RefCell::new(None) };
    /// NAV_PAGE widget → the pane it belongs to (which `NavSplit` cell takes it).
    static NAV_PAGE_PANES: RefCell<HashMap<usize, day_spec::props::Pane>> = RefCell::new(HashMap::new());
    /// A pushed stack page's filling cell (under its header bar), keyed by the NAV_PAGE
    /// widget: the page's content is the cell's allocation, not the whole host.
    static STACK_PAGE_CELLS: RefCell<HashMap<usize, DayCell>> = RefCell::new(HashMap::new());

    /// NAV_MENU widget → its list box + suppression flag.
    static NAV_MENUS: RefCell<HashMap<usize, NavMenuState>> = RefCell::new(HashMap::new());

    static GTK_ANIMS: RefCell<HashMap<usize, GtkAnim>> = RefCell::new(HashMap::new());
    // Day's laid-out top-left origin per GtkFixed child. GTK positions a Fixed child *via* its
    // child transform, which is the same slot the animation transform uses — so the two must be
    // composed (see apply_gtk_transform) rather than overwrite each other.
    static NODE_ORIGIN: RefCell<HashMap<usize, (f32, f32)>> = RefCell::new(HashMap::new());

    /// Each realized image's bundled source NAME and how it is painted, so a tint patch or a
    /// source swap can re-render it. The widget holds a texture, not a path, and re-reading the
    /// file is what a recolor needs. A [`SideTable`], so the entry goes with the widget in
    /// `release`'s sweep.
    static IMAGE_SOURCE: SideTable<ImageEntry> = SideTable::new();

    /// `BitmapId` → the decoded image (docs/images.md). Keyed by the id day-core minted rather
    /// than by a widget, because one bitmap outlives any view and may be drawn by several at
    /// once; entries leave only through `release_image`.
    static BITMAPS: RefCell<HashMap<u64, GtkBitmap>> = RefCell::new(HashMap::new());

    /// Per-listbox context popovers for the nav rows (docs/menus.md), keyed by listbox ptr —
    /// unparented before every row rebuild so popovers never outlive their rows.
    static NAV_ROW_POPOVERS: RefCell<HashMap<usize, Vec<gtk4::PopoverMenu>>> =
        RefCell::new(HashMap::new());
    /// NAV_MENU ListBox → its rows' section titles by index (`NavMenuProps::sections`), read
    /// by the box's header func (`nav_menu_headers`).
    static NAV_MENU_SECTIONS: RefCell<HashMap<usize, Vec<Option<String>>>> =
        RefCell::new(HashMap::new());

    /// Per-widget CSS provider for `background`/`corner_radius` surfaces, keyed by widget ptr, so
    /// a reactive background repaints by reloading the same provider (no provider accumulation).
    /// Each provider is registered on the DISPLAY (`apply_surface`), so the teardown — run by
    /// `release`'s sweep — must take it back off, or display-global providers pile up for the
    /// life of the process, one per decorated widget that ever existed.
    static SURFACE: SideTable<gtk4::CssProvider> = SideTable::with_teardown(|p| {
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::style_context_remove_provider_for_display(&display, &p);
        }
    });

        static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };

    /// Inspector split key → its state. [`SideTable`]s, so the release sweep drops them.
    static INSPECTOR_STATE: SideTable<Rc<RefCell<InspectorState>>> = SideTable::new();
    /// Inspector pane key → `(its NodeId, is-panel)`, recorded at realize and consumed when
    /// the pane is inserted into its split.
    static INSPECTOR_PANES: SideTable<(NodeId, bool)> = SideTable::new();
    /// Split host key → its state (docs/split.md); a [`SideTable`], swept with the host.
    static SPLIT_STATE: SideTable<Rc<RefCell<SplitState>>> = SideTable::new();
    /// Split pane key → `(its NodeId, is-second)`, recorded at realize.
    static SPLIT_PANES: SideTable<(NodeId, bool)> = SideTable::new();

    /// LIST scrolled-window key → its model + source holder.
    static LIST_STATE: RefCell<HashMap<usize, ListEntry>> = RefCell::new(HashMap::new());
    /// TREE host scrolled-window key → its live state (docs/tree.md).
    static TREE_STATE: RefCell<HashMap<usize, Rc<TreeEntry>>> = RefCell::new(HashMap::new());
    /// Widget key → its summon-time context-menu provider (docs/menus.md).
    static CTX_MENU_FNS: RefCell<HashMap<usize, day_spec::ContextMenuFn>> =
        RefCell::new(HashMap::new());
    /// Widgets that already carry the summon gesture (attach once; providers replace).
    static CTX_MENU_WIRED: RefCell<std::collections::HashSet<usize>> =
        RefCell::new(std::collections::HashSet::new());
    /// TREE cell widget key → the token its row currently shows (for row context menus).
    static TREE_CELL_TOKENS: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
    /// Physical cell (GtkFixed ptr) → the row it is currently bound to, for drag-to-reorder:
    /// a cell's row changes on every recycle, so the DragSource reads it at drag time. A
    /// [`SideTable`]: recycling overwrites live entries, but only the release sweep drops a
    /// destroyed cell's — an address GTK reuses must not inherit a stale row.
    static LIST_CELL_ROWS: SideTable<usize> = SideTable::new();
    /// The in-flight reorder drag: (list scrolled-window key, source row). One drag exists at a
    /// time and day lists only accept their OWN rows, so a thread-local carries what GTK's
    /// value-based content would only hand back asynchronously.
    static DRAG_FROM: std::cell::Cell<Option<(usize, usize)>> = const { std::cell::Cell::new(None) };
    /// A realized NAV_MENU's rows, by widget key: `(node, titles, icon names)`.
    ///
    /// Recorded at realize, where the props are, and handed to a navigation suite at INSERT — the
    /// first moment the menu has ancestors to walk. Where there is no suite above it (every
    /// presentation but `Tabs`) the handover finds nothing and the rows stay a list.
    static DOCUMENT_MENU_KEYS: RefCell<HashMap<usize, Vec<String>>> = RefCell::new(HashMap::new());
    static NAV_MENU_ROWS: RefCell<HashMap<usize, NavRow>> = RefCell::new(HashMap::new());

    /// Per-label style state, keyed by widget ptr. Font and color render through one Pango
    /// attribute list (set_attributes replaces the whole list), but a `LabelPatch` carries only
    /// the half that changed — so each patch updates its half here and re-applies the whole.
    /// Entries drop in `release`.
    static LABEL_STYLE: RefCell<HashMap<usize, LabelStyle>> = RefCell::new(HashMap::new());

    /// Live modals keyed by request id (for programmatic dismissal).
    static NAV_DIALOGS: RefCell<HashMap<u64, DialogHandle>> = RefCell::new(HashMap::new());
    /// In-flight GtkFileDialog requests. Membership IS the state: a request in the set is one
    /// day is still waiting on, and `end_file_dialog` drops it rather than cancelling anything
    /// (see there for why cancelling crashes GTK 4.14).
    static FILE_DIALOGS: RefCell<std::collections::HashSet<u64>> =
        RefCell::new(std::collections::HashSet::new());

    /// Presented emulated covers: (cover widget, its NodeId). `report_content_size` re-sizes
    /// each on window resize and re-reports FrameChanged (the frame is native-owned while
    /// presented — docs/cover.md).
    static COVERS: RefCell<Vec<(gtk4::Fixed, NodeId)>> = const { RefCell::new(Vec::new()) };
    /// Cover widget key → NodeId (set at realize; consumed by the CoverPatch arms). A
    /// [`SideTable`], so the release sweep drops it with the cover widget.
    static COVER_IDS: SideTable<NodeId> = SideTable::new();
    /// Covers presented as half sheets (`CoverProps::half`, docs/cover.md): the bottom half
    /// of the content area.
    static COVER_HALF: SideTable<()> = SideTable::new();

    /// Whether ANY Day window was active at the last activation check — the app-level
    /// lifecycle debounce (docs/windows.md): focus moving BETWEEN Day windows must not
    /// emit a resign/become pair.
    static ANY_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };

}

/// Render a label's runs as Pango markup (docs/text-runs.md).
///
/// MARKUP rather than a `pango::AttrList`, even though the attribute list is the tidier API:
/// Pango's attributes cannot express a LINK, which only exists in the markup dialect as
/// `<a href>`. One path for both keeps the escaping in one place — and the escaping is the whole
/// risk here, since `set_markup` on a string containing a stray `&` renders nothing at all.
fn set_label_runs(label: &gtk4::Label, text: &str, runs: &[day_spec::TextRun]) {
    use gtk4::prelude::*;
    let key = label.clone().upcast::<gtk4::Widget>().as_ptr() as usize;
    let style = LABEL_STYLE.with(|m| {
        let mut m = m.borrow_mut();
        let entry = m.entry(key).or_default();
        entry.rich = (!runs.is_empty()).then(|| (text.to_string(), runs.to_vec()));
        entry.clone()
    });
    let Some((text, runs)) = &style.rich else {
        // Plain text through the plain setter: no markup parse, no escaping to get wrong. The
        // attribute list comes back, since it no longer has runs to override.
        if label.text() != text {
            label.set_text(text);
        }
        apply_text_attrs(label, style.font, style.color);
        return;
    };
    label.set_attributes(None);
    label.set_markup(&rich_markup(text, runs, &style));
}

// Tint colors whose CSS provider is already installed on the display, keyed by `rrggbb`.
//
// GTK 4.10 deprecated per-widget providers, and the replacement is DISPLAY-wide — so the rule
// has to be selective rather than the widget. Each color gets its own class and its own
// provider, installed once; a second button in the same color reuses it.

fn apply_button_content(
    btn: &gtk4::Button,
    title: &str,
    icon: Option<&day_spec::Icon>,
    icon_only: bool,
) {
    let ink = BUTTON_INK.with(|m| m.get(btn.as_ptr() as usize));
    let image = match icon {
        Some(day_spec::Icon::Symbol(sym)) => toolbar::icon_name_for(*sym)
            .filter(|name| gtk4::IconTheme::for_display(&btn.display()).has_icon(name))
            .map(gtk4::Image::from_icon_name)
            .or_else(|| {
                day_spec::resource::stage_symbol_svg(*sym)
                    .and_then(|path| tinted_template_icon(&path.to_string_lossy(), ink))
            }),
        Some(day_spec::Icon::Image(name)) => tinted_template_icon(name, ink),
        None => None,
    };
    BUTTON_CONTENT.with(|m| {
        m.insert(
            btn.as_ptr() as usize,
            day_spec::props::ButtonContent {
                title: title.into(),
                icon: icon.cloned(),
                icon_only,
            },
        );
    });
    let hides_title = icon_only && image.is_some();
    if let Some(image) = image {
        image.set_pixel_size(20);
        if icon_only {
            btn.set_child(Some(&image));
        } else {
            let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            content.set_halign(gtk4::Align::Center);
            content.append(&image);
            content.append(&gtk4::Label::new(Some(title)));
            btn.set_child(Some(&content));
        }
    } else {
        btn.set_label(title);
    }
    btn.set_tooltip_text(hides_title.then_some(title));
    btn.update_property(&[gtk4::accessible::Property::Label(title)]);
}

/// The title a button shows, read off its GtkLabel: the child itself for a text button, the
/// label beside the image for an icon-and-title one (`apply_button_content`). `gtk_label_get_text`
/// already drops mnemonic underscores and markup. `None` for an icon-only button.
fn button_text(btn: &gtk4::Button) -> Option<String> {
    let child = btn.child()?;
    if let Some(label) = child.downcast_ref::<gtk4::Label>() {
        return Some(label.text().to_string());
    }
    let row = child.downcast_ref::<gtk4::Box>()?;
    let mut cur = row.first_child();
    while let Some(w) = cur {
        if let Some(label) = w.downcast_ref::<gtk4::Label>() {
            return Some(label.text().to_string());
        }
        cur = w.next_sibling();
    }
    None
}

/// Whether `h` is on screen, for `read_native`. Once the window is mapped, GTK's drawable
/// state (visible and mapped, so every ancestor shown and no unselected stack page above it)
/// answers. Before that, only a hidden widget or ancestor answers (`false`); otherwise `None`.
fn native_visible(h: &Handle) -> Option<bool> {
    if h.root().is_some_and(|r| r.is_mapped()) {
        return Some(h.is_drawable());
    }
    let mut cur = Some(h.clone());
    while let Some(w) = cur {
        if !w.is_visible() || !w.is_child_visible() {
            return Some(false);
        }
        cur = w.parent();
    }
    None
}

/// The frame of `h` in points, relative to its window's content area (the `DayCell` wrapper
/// `build_day_window` sets as the AdwToolbarView's content, below the header bar). `None` while
/// the widget is unmapped (its allocation may be stale) or outside a Day window.
fn native_frame(h: &Handle) -> Option<Rect> {
    if !h.is_mapped() {
        return None;
    }
    let mut cur = h.parent();
    let content = loop {
        let w = cur?;
        if let Some(view) = w.downcast_ref::<adw::ToolbarView>() {
            break view.content()?;
        }
        cur = w.parent();
    };
    let b = h.compute_bounds(&content)?;
    Some(Rect::new(
        f64::from(b.x()),
        f64::from(b.y()),
        f64::from(b.width()),
        f64::from(b.height()),
    ))
}

/// Put a [`day_spec::props::ButtonStyleSpec`] on a `GtkButton`, keeping it a GtkButton.
///
/// Prominent is Adwaita's own `suggested-action`. A tint sets `background-image` (which is what
/// Adwaita's button styling uses, so a plain `background-color` would be painted over) and the
/// label color. GTK keeps drawing the button, so `:hover`, `:active`, `:focus` and `:disabled`
/// still come from the theme.
/// Put `ButtonProps::selected` on a plain `gtk4::Button`: the `:checked` state flag, which the
/// theme draws exactly as a toggle button's checked look, without swapping in a
/// `GtkToggleButton` that would flip itself on every click. AT-SPI hears a button that has been
/// selected as pressed or not; a button that never selects keeps its plain button semantics.
fn set_button_selected(btn: &gtk4::Button, on: bool) {
    let was = btn.state_flags().contains(gtk4::StateFlags::CHECKED);
    if on {
        btn.set_state_flags(gtk4::StateFlags::CHECKED, false);
    } else {
        btn.unset_state_flags(gtk4::StateFlags::CHECKED);
    }
    if on || was {
        let pressed = if on {
            gtk4::AccessibleTristate::True
        } else {
            gtk4::AccessibleTristate::False
        };
        btn.update_state(&[gtk4::accessible::State::Pressed(pressed)]);
    }
}

fn apply_button_style(btn: &gtk4::Button, style: day_spec::props::ButtonStyleSpec) {
    use day_spec::props::ButtonStyleSpec as S;
    use gtk4::prelude::*;
    let hex = |x: day_spec::Color| {
        format!(
            "{:02x}{:02x}{:02x}",
            (x.r.clamp(0.0, 1.0) * 255.0) as u8,
            (x.g.clamp(0.0, 1.0) * 255.0) as u8,
            (x.b.clamp(0.0, 1.0) * 255.0) as u8
        )
    };
    btn.remove_css_class("suggested-action");
    btn.remove_css_class("flat");
    // Drop any tint class from a previous style, so a patch cannot leave two fills fighting.
    for c in btn.css_classes() {
        if c.starts_with("day-tint-") {
            btn.remove_css_class(&c);
        }
    }
    match style {
        S::Prominent => btn.add_css_class("suggested-action"),
        S::Tinted(c) => {
            let key = hex(c);
            let class = format!("day-tint-{key}");
            TINT_PROVIDERS.with(|set| {
                if set.borrow().contains(&key) {
                    return;
                }
                let Some(display) = gtk4::gdk::Display::default() else {
                    return;
                };
                let provider = gtk4::CssProvider::new();
                provider.load_from_data(&format!(
                    "button.{class} {{ background-image: image(#{key}); color: #{}; }}",
                    hex(S::on_tint(c))
                ));
                gtk4::style_context_add_provider_for_display(
                    &display,
                    &provider,
                    gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
                set.borrow_mut().insert(key);
            });
            btn.add_css_class(&class);
        }
        // A GtkButton hugs a one-glyph title already.
        S::Plain => btn.add_css_class("flat"),
        S::Bordered | S::Automatic | S::Compact => {}
    }
    let key = btn.as_ptr() as usize;
    let ink = match style {
        S::Tinted(c) => S::on_tint(c),
        S::Prominent => day_spec::Color::WHITE,
        _ => {
            if adw::StyleManager::default().is_dark() {
                day_spec::Color::WHITE
            } else {
                day_spec::Color::hex(0x1a1a1a)
            }
        }
    };
    BUTTON_INK.with(|m| {
        m.insert(key, ink);
    });
    if let Some(c) = BUTTON_CONTENT.with(|m| m.get(key)) {
        apply_button_content(btn, &c.title, c.icon.as_ref(), c.icon_only);
    }
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

/// A decoded kind-14 record (set-gradient), waiting for its fill-shape record. Unit geometry
/// resolves against the shape's bounding box; stops are (offset, 0xAARRGGBB).
struct PendingGradient {
    /// Encoded gradient type: 0 = linear (a,b→c,d unit points), 1 = radial (a,b center, c radius).
    kind: u8,
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    stops: Vec<(f64, u32)>,
}

impl PendingGradient {
    /// Install the gradient as the cairo source, resolved against the shape's bbox.
    fn set_source(&self, cr: &gtk4::cairo::Context, x: f64, y: f64, w: f64, h: f64) {
        let stops = |g: &gtk4::cairo::Gradient| {
            for (o, bits) in &self.stops {
                g.add_color_stop_rgba(
                    *o,
                    ((bits >> 16) & 0xff) as f64 / 255.0,
                    ((bits >> 8) & 0xff) as f64 / 255.0,
                    (bits & 0xff) as f64 / 255.0,
                    ((bits >> 24) & 0xff) as f64 / 255.0,
                );
            }
        };
        if self.kind == 1 {
            // Radial, elliptical-to-bounds: a circular gradient in the unit space of the
            // bounds, mapped by the PATTERN matrix (user → pattern space, so the inverse of
            // the unit→bounds map).
            let rg = gtk4::cairo::RadialGradient::new(self.a, self.b, 0.0, self.a, self.b, self.c);
            stops(rg.as_ref());
            let m = gtk4::cairo::Matrix::new(1.0 / w, 0.0, 0.0, 1.0 / h, -x / w, -y / h);
            rg.set_matrix(m);
            let _ = cr.set_source(&rg);
        } else {
            let lg = gtk4::cairo::LinearGradient::new(
                x + self.a * w,
                y + self.b * h,
                x + self.c * w,
                y + self.d * h,
            );
            stops(lg.as_ref());
            let _ = cr.set_source(&lg);
        }
    }
}

/// Trace an encoded path ("M x y L x y Q .. C .. Z", see `day_spec::encode_path`) into the
/// cairo context, returning its bounding box for gradient resolution.
///
/// Tolerant by construction: a malformed token is skipped rather than aborting the page, because
/// this data crosses an encode/decode boundary and a half-drawn frame beats a blank one.
fn cairo_trace_path(cr: &gtk4::cairo::Context, spec: &str) -> (f64, f64, f64, f64) {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    let mut it = spec.split_whitespace();
    let num = |it: &mut std::str::SplitWhitespace| it.next().and_then(|s| s.parse::<f64>().ok());
    // cairo has no quadratic primitive, so quads are elevated to cubics exactly.
    let mut cur = (0.0f64, 0.0f64);
    while let Some(tok) = it.next() {
        match tok {
            "M" => {
                if let (Some(x), Some(y)) = (num(&mut it), num(&mut it)) {
                    cr.move_to(x, y);
                    cur = (x, y);
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
            }
            "L" => {
                if let (Some(x), Some(y)) = (num(&mut it), num(&mut it)) {
                    cr.line_to(x, y);
                    cur = (x, y);
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
            }
            "Q" => {
                if let (Some(cx), Some(cy), Some(x), Some(y)) =
                    (num(&mut it), num(&mut it), num(&mut it), num(&mut it))
                {
                    let c1 = (
                        cur.0 + 2.0 / 3.0 * (cx - cur.0),
                        cur.1 + 2.0 / 3.0 * (cy - cur.1),
                    );
                    let c2 = (x + 2.0 / 3.0 * (cx - x), y + 2.0 / 3.0 * (cy - y));
                    cr.curve_to(c1.0, c1.1, c2.0, c2.1, x, y);
                    cur = (x, y);
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
            }
            "C" => {
                if let (Some(ax), Some(ay), Some(bx), Some(by), Some(x), Some(y)) = (
                    num(&mut it),
                    num(&mut it),
                    num(&mut it),
                    num(&mut it),
                    num(&mut it),
                    num(&mut it),
                ) {
                    cr.curve_to(ax, ay, bx, by, x, y);
                    cur = (x, y);
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
            }
            "Z" => cr.close_path(),
            _ => {}
        }
    }
    if x0 > x1 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    (x0, y0, x1 - x0, y1 - y0)
}

/// The dash/cap/join a kind-18 record carries, waiting for the stroke it applies to.
#[derive(Default)]
struct PendingStroke {
    cap: f64,
    join: f64,
    miter: f64,
    phase: f64,
    dashes: Vec<f64>,
}

impl PendingStroke {
    fn apply(&self, cr: &gtk4::cairo::Context) {
        use gtk4::cairo::{LineCap, LineJoin};
        cr.set_line_cap(match self.cap as i32 {
            1 => LineCap::Round,
            2 => LineCap::Square,
            _ => LineCap::Butt,
        });
        cr.set_line_join(match self.join as i32 {
            1 => LineJoin::Round,
            2 => LineJoin::Bevel,
            _ => LineJoin::Miter,
        });
        cr.set_miter_limit(self.miter);
        cr.set_dash(&self.dashes, self.phase);
    }
}

/// The family/weight/slant a kind-19 record carries, waiting for the text it applies to.
#[derive(Default)]
struct PendingFont {
    weight: f64,
    italic: bool,
    family: String,
}

/// The Pango description canvas text draws and measures with (docs/fonts.md): the requested
/// family or, for the default, the family of GTK's own UI font, so canvas text matches labels;
/// an ABSOLUTE size, so a point stays a cairo user unit whatever the Xft DPI says.
fn canvas_font_desc(size: f64, font: &PendingFont) -> gtk4::pango::FontDescription {
    use gtk4::pango;
    let mut desc = if font.family.is_empty() {
        default_ui_font_desc()
    } else {
        let mut d = pango::FontDescription::new();
        d.set_family(&font.family);
        d
    };
    let weight = if font.weight > 0.0 {
        day_spec::FontWeight::from_css(font.weight as u16)
    } else {
        day_spec::FontWeight::Regular
    };
    desc.set_weight(pango_weight(weight));
    desc.set_style(if font.italic {
        pango::Style::Italic
    } else {
        pango::Style::Normal
    });
    desc.set_absolute_size(size * f64::from(pango::SCALE));
    desc
}

/// The family of the GTK UI font (`gtk-font-name`), as a fresh description carrying only
/// the family — size, weight and slant come from the canvas request.
fn default_ui_font_desc() -> gtk4::pango::FontDescription {
    use gtk4::pango;
    let mut d = pango::FontDescription::new();
    if let Some(name) = gtk4::Settings::default().and_then(|s| s.gtk_font_name())
        && let Some(family) = pango::FontDescription::from_string(&name).family()
    {
        d.set_family(&family);
    }
    d
}

/// Pango's numeric weight (100 … 1000) to the nearest Day rung.
fn day_weight(w: gtk4::pango::Weight) -> day_spec::FontWeight {
    use gtk4::glib::translate::IntoGlib as _;
    day_spec::FontWeight::from_css(w.into_glib().clamp(100, 900) as u16)
}

/// Put the stroke state back to Day's defaults, so a styled stroke never leaks into the next one.
fn reset_stroke(cr: &gtk4::cairo::Context) {
    cr.set_line_cap(gtk4::cairo::LineCap::Butt);
    cr.set_line_join(gtk4::cairo::LineJoin::Miter);
    cr.set_miter_limit(10.0);
    cr.set_dash(&[], 0.0);
}

fn cairo_draw(cr: &gtk4::cairo::Context, nums: &[f64], texts: &[String]) {
    let mut ti = 0;
    let mut pending: Option<PendingGradient> = None;
    let mut pending_stroke: Option<PendingStroke> = None;
    let mut pending_font: Option<PendingFont> = None;
    // A decoded kind-20 record (stamp): the positions the next shape record is drawn at, once
    // each. Empty means the ordinary one-shape-one-record case (docs/canvas.md "Stamping").
    let mut stamp_at: Vec<(f64, f64)> = Vec::new();
    let mut stamp_n = 0usize;
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
        // Stroke kinds, in encode_ops order: rect, ellipse, arc, line, polygon, rrect, path.
        let is_stroke = matches!(k, 1 | 4 | 5 | 6 | 12 | 13 | 16);
        if is_stroke && let Some(st) = pending_stroke.take() {
            st.apply(cr);
        }
        if k != 19 && k != 7 {
            pending_font = None;
        }
        // Stamp prefix and its coordinate records: collected, never drawn on their own.
        if k == 20 {
            stamp_at.clear();
            stamp_n = a as usize;
            stamp_at.reserve(stamp_n);
            continue;
        }
        if k == 21 {
            // Four points per record; the last record of a run is padded with zeros, so the
            // header's count is what says where the real ones stop.
            for pair in [(a, b), (c, d), (e, f), (g, col)] {
                if stamp_at.len() < stamp_n {
                    stamp_at.push(pair);
                }
            }
            continue;
        }
        // The template is replayed once per position under a translated CTM. `ti` is rewound
        // each time so a template with a texts payload (a polygon, a path) reads the same entry
        // every repetition and consumes it exactly once overall.
        let reps = stamp_at.len().max(1);
        let ti_start = ti;
        for rep in 0..reps {
            ti = ti_start;
            if let Some((dx, dy)) = stamp_at.get(rep) {
                cr.save().ok();
                cr.translate(*dx, *dy);
            }
            match k {
                0 | 1 => {
                    cr.rectangle(a, b, c, d);
                    if k == 0 {
                        if let Some(gr) = pending.take() {
                            gr.set_source(cr, a, b, c, d);
                        }
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
                        if let Some(gr) = pending.take() {
                            gr.set_source(cr, a, b, c, d);
                        }
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
                        if let Some(gr) = pending.take() {
                            gr.set_source(cr, a, b, c, d);
                        }
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
                // Text (7): a Pango layout, drawn from its top-left — which IS the `LEADING`
                // anchor — offset by whatever the packed anchor asks for, using the layout's own
                // logical extents and baseline. The layout picks up the cairo CTM, so it rotates and
                // scales with the drawing.
                7 => {
                    let text = texts.get(ti).cloned().unwrap_or_default();
                    ti += 1;
                    let font = pending_font.take().unwrap_or_default();
                    let layout = pangocairo::functions::create_layout(cr);
                    layout.set_font_description(Some(&canvas_font_desc(e, &font)));
                    layout.set_single_paragraph_mode(true);
                    layout.set_text(&text);
                    let anchor = day_spec::TextAnchor::unpack(f);
                    let (mut x, mut y) = (a, b);
                    if anchor != day_spec::TextAnchor::LEADING {
                        let (_, logical) = layout.extents();
                        let scale = f64::from(gtk4::pango::SCALE);
                        let (dx, dy) = anchor.offset(
                            f64::from(logical.width()) / scale,
                            f64::from(logical.height()) / scale,
                            f64::from(layout.baseline()) / scale,
                        );
                        x += dx;
                        y += dy;
                    }
                    cr.move_to(x, y);
                    pangocairo::functions::show_layout(cr, &layout);
                    // The layout leaves cairo's current point where the text ended; the next
                    // arc/ellipse would draw a line from there to its start. Clear it.
                    cr.new_path();
                }
                // Font (19): applies to the next text record only.
                19 => {
                    let family = texts.get(ti).cloned().unwrap_or_default();
                    ti += 1;
                    pending_font = Some(PendingFont {
                        weight: a,
                        italic: b > 0.5,
                        family,
                    });
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
                    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                    for pair in pts.split(' ') {
                        if let Some((x, y)) = pair.split_once(',')
                            && let (Ok(x), Ok(y)) = (x.parse::<f64>(), y.parse::<f64>())
                        {
                            (x0, y0) = (x0.min(x), y0.min(y));
                            (x1, y1) = (x1.max(x), y1.max(y));
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
                            if let Some(gr) = pending.take() {
                                gr.set_source(cr, x0, y0, x1 - x0, y1 - y0);
                            }
                            let _ = cr.fill();
                        } else {
                            cr.set_line_width(g);
                            let _ = cr.stroke();
                        }
                    }
                }
                // Path (15 fill / 16 stroke): segments ride the texts channel; slot f is the fill
                // rule (0 non-zero, 1 even-odd) and slot g the stroke width.
                15 | 16 => {
                    let spec = texts.get(ti).cloned().unwrap_or_default();
                    ti += 1;
                    let (bx, by, bw, bh) = cairo_trace_path(cr, &spec);
                    if k == 15 {
                        cr.set_fill_rule(if f > 0.5 {
                            gtk4::cairo::FillRule::EvenOdd
                        } else {
                            gtk4::cairo::FillRule::Winding
                        });
                        if let Some(gr) = pending.take() {
                            gr.set_source(cr, bx, by, bw, bh);
                        }
                        let _ = cr.fill();
                        cr.set_fill_rule(gtk4::cairo::FillRule::Winding);
                    } else {
                        cr.set_line_width(g);
                        if let Some(gr) = pending.take() {
                            gr.set_source(cr, bx, by, bw, bh);
                        }
                        let _ = cr.stroke();
                    }
                }
                // Clip (17): slot f names the shape (0 rect, 1 rounded rect, 2 ellipse, 3 path,
                // 4 polygon); geometry in a..d, corner radius or fill rule in e.
                17 => {
                    match f as i32 {
                        1 => {
                            // Same corner trace as the rounded-rect fill above.
                            let r = e.min(c / 2.0).min(d / 2.0);
                            let (x, y, w, h) = (a, b, c, d);
                            cr.new_sub_path();
                            cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
                            cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
                            cr.arc(
                                x + r,
                                y + h - r,
                                r,
                                std::f64::consts::FRAC_PI_2,
                                std::f64::consts::PI,
                            );
                            cr.arc(
                                x + r,
                                y + r,
                                r,
                                std::f64::consts::PI,
                                3.0 * std::f64::consts::FRAC_PI_2,
                            );
                            cr.close_path();
                        }
                        2 => {
                            cr.save().ok();
                            cr.translate(a + c / 2.0, b + d / 2.0);
                            cr.scale(c / 2.0, d / 2.0);
                            cr.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
                            cr.restore().ok();
                        }
                        3 => {
                            let spec = texts.get(ti).cloned().unwrap_or_default();
                            ti += 1;
                            cairo_trace_path(cr, &spec);
                            cr.set_fill_rule(if e > 0.5 {
                                gtk4::cairo::FillRule::EvenOdd
                            } else {
                                gtk4::cairo::FillRule::Winding
                            });
                        }
                        4 => {
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
                            }
                        }
                        _ => cr.rectangle(a, b, c, d),
                    }
                    cr.clip();
                    cr.set_fill_rule(gtk4::cairo::FillRule::Winding);
                }
                // Stroke style (18): applies to the next stroke record only.
                18 => {
                    let raw = texts.get(ti).cloned().unwrap_or_default();
                    ti += 1;
                    pending_stroke = Some(PendingStroke {
                        cap: a,
                        join: b,
                        miter: c,
                        phase: d,
                        dashes: raw
                            .split(' ')
                            .filter_map(|s| s.parse::<f64>().ok())
                            .collect(),
                    });
                }
                10 => {
                    // Packed affine (a,b,c,d,tx,ty); cairo Matrix is (xx,yx,xy,yy,x0,y0) with the
                    // same row-vector meaning as day_geometry::Affine.
                    let m = gtk4::cairo::Matrix::new(a, b, c, d, e, f);
                    cr.transform(m);
                }
                // Set-gradient (f = type: 0 linear a,b→c,d; 1 radial a,b center + c radius);
                // "offset,aarrggbb …" stops ride the texts channel. Applies to the next
                // fill-shape record (encode_ops contract).
                14 => {
                    let raw = texts.get(ti).cloned().unwrap_or_default();
                    ti += 1;
                    let stops = raw
                        .split(' ')
                        .filter_map(|s| {
                            let (o, c) = s.split_once(',')?;
                            Some((o.parse::<f64>().ok()?, u32::from_str_radix(c, 16).ok()?))
                        })
                        .collect();
                    pending = Some(PendingGradient {
                        kind: f as u8,
                        a,
                        b,
                        c,
                        d,
                        stops,
                    });
                }
                // Image (22): a,b origin · c,d size · e the BitmapId · f opacity (docs/images.md).
                // A released bitmap draws nothing rather than a placeholder: a canvas re-records
                // on every tracked read, so a handle can be dropped between record and replay.
                22 => {
                    let pixbuf = BITMAPS
                        .with(|m| m.borrow().get(&(e as u64)).and_then(|b| b.pixbuf.clone()));
                    if let Some(pixbuf) = pixbuf {
                        let (pw, ph) = (f64::from(pixbuf.width()), f64::from(pixbuf.height()));
                        if pw > 0.0 && ph > 0.0 && c > 0.0 && d > 0.0 {
                            use gtk4::gdk::prelude::GdkCairoContextExt as _;
                            cr.save().ok();
                            cr.translate(a, b);
                            cr.scale(c / pw, d / ph);
                            cr.set_source_pixbuf(&pixbuf, 0.0, 0.0);
                            let _ = cr.paint_with_alpha(f.clamp(0.0, 1.0));
                            cr.restore().ok();
                        }
                    }
                }
                _ => {}
            }
            if stamp_at.get(rep).is_some() {
                cr.restore().ok();
            }
        }
        stamp_at.clear();
        if is_stroke {
            reset_stroke(cr);
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

/// The day name for a keyval, or `None` for every key the route does not carry
/// (docs/menus.md).
fn key_name(key: gtk4::gdk::Key, state: gtk4::gdk::ModifierType) -> Option<&'static str> {
    use gtk4::gdk::{Key, ModifierType as M};
    match key {
        Key::Left | Key::KP_Left => Some("ArrowLeft"),
        Key::Right | Key::KP_Right => Some("ArrowRight"),
        Key::Up | Key::KP_Up => Some("ArrowUp"),
        Key::Down | Key::KP_Down => Some("ArrowDown"),
        // Not the delete keys: this backend has a menu bar, whose accelerators own them
        // (docs/menus.md).
        //
        // A digit, main row or keypad, named by what it types (the keyval already carries
        // Shift). Never under Control, Alt, or a Mac's ⌘ (Meta/Super): those belong to
        // accelerators, and claiming one would starve the menu the same way.
        _ if state.intersects(M::CONTROL_MASK | M::ALT_MASK | M::META_MASK | M::SUPER_MASK) => None,
        _ => key.to_unicode().and_then(day_spec::KeyEvent::digit_name),
    }
}

/// GDK's modifier state as day's mask.
fn key_modifiers(state: gtk4::gdk::ModifierType) -> u8 {
    let mut m = 0u8;
    if state.contains(gtk4::gdk::ModifierType::SHIFT_MASK) {
        m |= day_spec::KeyEvent::SHIFT;
    }
    if state.contains(gtk4::gdk::ModifierType::CONTROL_MASK) {
        m |= day_spec::KeyEvent::PRIMARY;
    }
    if state.contains(gtk4::gdk::ModifierType::ALT_MASK) {
        m |= day_spec::KeyEvent::ALT;
    }
    m
}

/// Report focus gain/loss into the sink (docs/focus.md). `EventControllerFocus` tracks
/// focus-within, so a `GtkEntry`'s inner `GtkText` grabbing focus still counts as the entry.
fn wire_focus(w: &impl IsA<gtk4::Widget>, id: NodeId) {
    let focus = gtk4::EventControllerFocus::new();
    focus.connect_enter(move |_| ffi_guard::contain((), || emit(id, Event::FocusChanged(true))));
    focus.connect_leave(move |_| ffi_guard::contain((), || emit(id, Event::FocusChanged(false))));
    w.add_controller(focus);
}

/// GTK 4.24 defers focus removal until after paint when a widget is unparented. On Win32,
/// GtkIMContextIME then cannot find the detached widget's native surface to unregister its
/// display message filter. Disposing the input leaves that filter pointing at freed memory.
/// Resign while the focused descendant is still attached; leave another subtree's focus alone.
fn resign_focus_before_detach(widget: &gtk4::Widget) {
    if !cfg!(target_os = "windows") {
        return;
    }
    let Some(root) = widget.root() else {
        return;
    };
    if let Some(focus) = root.focus()
        && (focus == *widget || focus.is_ancestor(widget))
    {
        root.set_focus(None::<&gtk4::Widget>);
    }
}

// ---------------------------------------------------------------------------
// Menus (§ menus): render day's MenuItem model as a GMenu (GtkPopoverMenu for context menus,
// GtkPopoverMenuBar for the app menu). Custom items → SimpleActions under the "daymenu" prefix that
// emit Event::MenuAction; role items → the widget's stock action (clipboard.copy, …).
// ---------------------------------------------------------------------------

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
        R::NewWindow => "New Window",
        R::CheckForUpdates => "Check for Updates…",
    }
}

/// GTK accelerator string, e.g. `<Primary>s`, `<Primary><Shift>s`.
fn accel_string(s: &day_spec::Shortcut) -> String {
    let mut acc = String::new();
    if s.primary {
        // GTK4 dropped GTK3's Primary→Command mapping: <Primary> is a plain <Control> alias
        // now, so on macOS the conventional command modifier is the Command key — which the
        // macos GDK backend reports as META.
        acc.push_str(if cfg!(target_os = "macos") {
            "<Meta>"
        } else {
            "<Primary>"
        });
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
        k if k.chars().count() == 1 => {
            // Punctuation must be spelled by keysym name — GTK rejects the literal
            // ("Unable to parse accelerator '<Primary>/'") and then installs NO accelerators
            // for that menu at all. Ask GDK for the name rather than keeping a hand-written
            // table, which only ever covers the punctuation someone already hit.
            match k.chars().next() {
                Some(ch) if !ch.is_ascii_alphanumeric() => {
                    let keyval = gtk4::gdk::unicode_to_keyval(ch as u32);
                    // SAFETY: `Key` is a transparent newtype over a keyval and validates
                    // nothing; `keyval` is one GDK just minted for this char. gdk4 relies on
                    // the same reasoning in its own `FromValue for Key`.
                    let key: gtk4::gdk::Key =
                        unsafe { gtk4::glib::translate::FromGlib::from_glib(keyval) };
                    key.name()
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| k.to_lowercase())
                }
                _ => k.to_lowercase(),
            }
        }
        k => k.to_string(),
    };
    format!("{acc}{key}")
}

/// A menu item's glyph, as the GMenuModel "icon" attribute GtkPopoverMenu renders
/// (docs/menus.md). A symbol becomes the theme's own named icon; a bundled image becomes a
/// file icon on the staged asset. A name the theme lacks is simply left off — a menu item
/// reads by its label, and a broken-image placeholder would read worse than no icon.
fn set_menu_icon(mi: &gtk4::gio::MenuItem, icon: Option<&day_spec::Icon>) {
    use gtk4::gio::prelude::*;
    let gicon: Option<gtk4::gio::Icon> = match icon {
        Some(day_spec::Icon::Symbol(s)) => crate::toolbar::icon_name_for(*s)
            .map(|n| gtk4::gio::ThemedIcon::new(n).upcast::<gtk4::gio::Icon>()),
        Some(day_spec::Icon::Image(name)) => day_spec::resource::resolve_vector_svg(name)
            .or_else(|| day_spec::resource::resolve_image_file(name))
            .map(|path| {
                gtk4::gio::FileIcon::new(&gtk4::gio::File::for_path(path))
                    .upcast::<gtk4::gio::Icon>()
            }),
        None => None,
    };
    if let Some(v) = gicon.and_then(|g| g.serialize()) {
        mi.set_attribute_value("icon", Some(&v));
    }
}

pub(crate) fn build_gio_menu(
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
            MI::Submenu { label, items, .. } => {
                section.append_submenu(Some(label), &build_gio_menu(items, group));
            }
            MI::Action {
                action: dispatch,
                label,
                shortcut,
                enabled,
                checked,
                role,
                icon,
                ..
            } => {
                if *dispatch != 0 {
                    let name = format!("a{dispatch}");
                    // A checkable item is a STATEFUL boolean GAction: GTK draws the check from the
                    // action's state, so the mark and the model cannot drift apart. The state is
                    // set from the app's value on every install and never toggled here — day's
                    // rule is that the app owns it (docs/menus.md).
                    let action = match checked {
                        Some(on) => {
                            gtk4::gio::SimpleAction::new_stateful(&name, None, &(*on).to_variant())
                        }
                        None => gtk4::gio::SimpleAction::new(&name, None),
                    };
                    action.set_enabled(*enabled);
                    if checked.is_some() {
                        // Swallow GTK's own toggle. A stateful boolean GAction flips its state on
                        // activation unless something handles `change-state`, which would let the
                        // mark move even when the app's action decides not to — the one way the
                        // menu could end up disagreeing with the app. Connecting an empty handler
                        // leaves the state settable only by the next install.
                        action.connect_change_state(|_, _| {});
                    }
                    let aid = *dispatch;
                    action.connect_activate(move |_, _| {
                        ffi_guard::contain((), || {
                            emit(day_spec::WINDOW_NODE, Event::MenuAction(aid));
                        });
                    });
                    group.add_action(&action);
                    // An id+role item with no label (the auto Preferences item) reads from
                    // the role table — verbatim "" rendered an invisible blank entry.
                    let lbl = match role {
                        Some(r) if label.is_empty() => gtk_role_label(*r),
                        _ => label.as_str(),
                    };
                    let mi = gtk4::gio::MenuItem::new(Some(lbl), Some(&format!("daymenu.{name}")));
                    if let Some(sc) = shortcut {
                        mi.set_attribute_value("accel", Some(&accel_string(sc).to_variant()));
                    }
                    set_menu_icon(&mi, icon.as_ref());
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
                action,
                shortcut: Some(sc),
                ..
            } if *action != 0 => {
                let accel = accel_string(sc);
                app.set_accels_for_action(&format!("daymenu.a{action}"), &[accel.as_str()]);
            }
            _ => {}
        }
    }
}

/// Register the conventional `app.preferences` GAction when the lowered model carries a
/// Preferences item. On macOS this is what enables the stock "Preferences…" entry GTK's
/// quartz backend places in the global app menu; on GNOME it matches the shell convention.
fn register_app_prefs_action(app: &gtk4::Application, items: &[day_spec::MenuItem]) {
    use day_spec::MenuItem as MI;
    use gtk4::prelude::*;
    fn find_prefs_id(items: &[day_spec::MenuItem]) -> Option<u64> {
        items.iter().find_map(|item| match item {
            MI::Submenu { items, .. } => find_prefs_id(items),
            MI::Action { action, role, .. }
                if *action != 0 && *role == Some(day_spec::MenuRole::Preferences) =>
            {
                Some(*action)
            }
            _ => None,
        })
    }
    let Some(id) = find_prefs_id(items) else {
        return;
    };
    let action = gtk4::gio::SimpleAction::new("preferences", None);
    action.connect_activate(move |_, _| {
        ffi_guard::contain((), || {
            emit(day_spec::WINDOW_NODE, Event::MenuAction(id));
        });
    });
    app.add_action(&action);
}

// ---------------------------------------------------------------------------
// Navigation (docs/navigation.md): nav host(Sidebar) → two nested GtkPaneds (`NavSplit`);
// stack → AdwNavigationView (push/pop). A split pane's page sits in a filling `DayCell`; a
// stack page's GtkFixed is wrapped in an AdwNavigationPage. Day sizes content from the pane
// widths via FrameChanged (nav_report).
// ---------------------------------------------------------------------------

/// The sidebar's opening width; the divider moves it between `NAV_SIDEBAR_MIN_W` and
/// `NAV_SIDEBAR_MAX_W`.
const NAV_SIDEBAR_W: f64 = day_spec::NAV_SIDEBAR_WIDTH;

/// The detail pane's minimum: the room a window must keep for the destination beside its
/// panes. Below it the panes' honest minimums no longer fit, and the sidebar collapses to make
/// room (`NavSplit::fit`).
const NAV_DETAIL_MIN_W: f64 = 200.0;

/// The room a collapsed sidebar waits for before coming back, past its own width: a resize
/// that stops exactly at the threshold flips neither way, and one that crosses it does not
/// oscillate.
const NAV_REVEAL_SLACK: i32 = 24;

/// A sidebar host's three panes — sidebar | list | detail — as two nested `GtkPaned`s, each
/// with a user-draggable divider: the AppKit `NSSplitView` shape (docs/navigation.md).
///
/// Decision (2026-09): this replaced `AdwOverlaySplitView`, and no flag keeps it around.
/// libadwaita pins sidebar widths by design (the GNOME HIG has no draggable sidebars), so the
/// Adw split could never give Day's desktop apps the adjustable columns AppKit and Qt have,
/// and its pane sizing — a min == max sidebar, a content pane whose minimum is its own Day
/// frame — fought every relayout (issue #19's collapse animation, the External-policy scroll
/// windows it took to break minimum propagation). A paned is the toolkit's own resizable
/// split; with a filling `DayCell` as each pane's child, the only minimum a pane has is the
/// honest one this backend sets. What the Adw split supplied that the paned does not — a
/// slide animation on the sidebar toggle, and the adaptive `collapsed` breakpoint — is either
/// not wanted (dayscript screenshots the instant a toggle returns) or done here instead
/// (`fit`, the AppKit-style collapse when the window has no room).
///
/// The inner paned exists on every host, list or not: with no list page, or a collapsed one,
/// GTK gives the detail the whole inner allocation and draws no handle, so the shape never
/// changes under live pages (Qt keeps its three splitter panes for the same reason).
struct NavSplit {
    /// sidebar | `inner`.
    outer: gtk4::Paned,
    /// list | detail.
    inner: gtk4::Paned,
    /// The pane cells: filling, so a page's Day-laid frame never becomes a pane minimum; each
    /// carries its pane's minimum width as its size request instead.
    sidebar: DayCell,
    list: DayCell,
    detail: DayCell,
    /// The sidebar's width while showing — what a reveal restores. Follows the divider, but
    /// never a position GTK clamped for want of room.
    sidebar_w: std::cell::Cell<i32>,
    /// The list pane's width, likewise.
    list_w: std::cell::Cell<i32>,
    /// The toolbar's sidebar toggle hid the sidebar; only the toggle brings it back.
    sidebar_hidden: std::cell::Cell<bool>,
    /// `fit` hid the sidebar for want of room; it comes back when there is room again.
    sidebar_fitted: std::cell::Cell<bool>,
    /// The host declared a content-list pane (`NavProps::list_width`) and it is showing
    /// (`NavProps::list_visible`, then `NavPatch::ListVisible`).
    list_shown: std::cell::Cell<bool>,
}

impl NavSplit {
    /// The width of one paned handle: Adwaita's `paned > separator` is a 1px line.
    const HANDLE: i32 = 1;

    /// Whether the sidebar should be on screen now.
    fn sidebar_showing(&self) -> bool {
        !self.sidebar_hidden.get() && !self.sidebar_fitted.get()
    }

    /// Show or hide the sidebar per the two flags, restoring its width on a reveal. GTK gives
    /// the inner paned the whole outer allocation while the start child is hidden, and draws
    /// no handle — the collapsed shape, with nothing to drag.
    fn apply_sidebar(&self) {
        let show = self.sidebar_showing();
        if self.sidebar.is_visible() != show {
            self.sidebar.set_visible(show);
        }
        if show {
            self.outer.set_position(self.sidebar_w.get());
        }
    }

    /// Show or collapse the content-list pane (`NavPatch::ListVisible`).
    fn apply_list(&self) {
        let show = self.list_shown.get();
        if self.list.is_visible() != show {
            self.list.set_visible(show);
        }
        if show {
            self.inner.set_position(self.list_w.get());
        }
    }

    /// The width the panes beside the sidebar need: the inner paned's honest minimum (the
    /// list's minimum while it shows, plus the detail's).
    fn rest_min(&self) -> i32 {
        self.inner.measure(gtk4::Orientation::Horizontal, -1).0
    }

    /// Fit the panes to `width`, the host's allocation — the AppKit rule: when the window has
    /// no room for the sidebar beside the other panes' minimums, the sidebar collapses; when
    /// room returns, it comes back at the width it had. A sidebar the toggle hid stays hidden
    /// either way. Runs inside the host's allocation, so the flip itself is deferred: showing
    /// or hiding a pane mid-allocation would queue a resize from within one.
    fn fit(self: &Rc<Self>, width: i32) {
        if self.sidebar_hidden.get() || width <= 0 {
            return;
        }
        let rest = self.rest_min() + Self::HANDLE;
        let flip = if self.sidebar_fitted.get() {
            width >= rest + self.sidebar_w.get() + NAV_REVEAL_SLACK
        } else {
            width < rest + self.sidebar.measure(gtk4::Orientation::Horizontal, -1).0
        };
        if flip {
            self.sidebar_fitted.set(!self.sidebar_fitted.get());
            let sp = self.clone();
            gtk4::glib::idle_add_local_once(move || ffi_guard::contain((), || sp.apply_sidebar()));
        }
    }
}

/// nav host(Sidebar) → `NavSplit`; stack → AdwNavigationView (push/pop).
enum NavPresent {
    Documents(Rc<DocumentHost>),
    /// Sidebar, content list, and detail in nested GtkPaneds with draggable dividers.
    Split(Rc<NavSplit>),
    /// `NavPresentation::Tabs`: the Adwaita view-switching idiom — an `AdwViewSwitcher` over
    /// an `AdwViewStack` of resident pages, the switcher a libadwaita app puts in its header
    /// bar. Drawn above the pages here: this host is usually nested inside a sidebar
    /// destination, and the window's header bar is not its to take. (Until 2026-09-12 the
    /// switcher was a hand-built `.linked` row of toggles docked at the foot, and the pages'
    /// own Day frames pushed it below the pane's clipped edge — no picker was ever visible.)
    ///
    /// Each page sits in a filling `DayCell` and reports its size from the cell's allocation,
    /// the split panes' rule, so the pages can never grow the stack past the host.
    ///
    /// A desktop only ever gets here by PINNING `NavStyle::Tabs` — `Cap::NavTabsAdaptive` is
    /// off here, so a narrowing window hides the sidebar and pushes instead of growing a bar.
    Suite {
        stack: adw::ViewStack,
        /// The rows the switcher shows — titles and icon names, index-aligned with the
        /// destination pages (`p1`, `p2`, …). They arrive from the NAV_MENU, which is built
        /// After the pages' host and may be rebuilt (`NavMenuPatch::Items`), so a page joining
        /// before or after them takes its title from here.
        rows: Rc<RefCell<SuiteRows>>,
        /// The nav menu's node: a switch reports against it, exactly as a sidebar row click
        /// does, so a tab and a row are one event above this backend.
        menu_node: Rc<std::cell::Cell<u64>>,
    },
    Stack(adw::NavigationView),
}

/// Document tabs use libadwaita's document container, not its section switcher.
struct DocumentHost {
    bar: adw::TabBar,
    view: adw::TabView,
    sidebar: gtk4::Box,
    keys: RefCell<Vec<String>>,
    titles: RefCell<Vec<String>>,
    page_keys: Rc<RefCell<HashMap<usize, String>>>,
    menu: Rc<std::cell::Cell<u64>>,
    suppress: Rc<std::cell::Cell<bool>>,
}
impl DocumentHost {
    fn new(
        id: NodeId,
        config: &DocumentTabsConfig,
        suppress: Rc<std::cell::Cell<bool>>,
    ) -> (Handle, Rc<Self>) {
        let view = adw::TabView::new();
        view.set_vexpand(true);
        let bar = adw::TabBar::new();
        bar.set_view(Some(&view));
        bar.set_autohide(false);
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        let sidebar = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        sidebar.set_visible(false);
        container.append(&sidebar);
        container.append(&bar);
        container.append(&view);
        let menu = Rc::new(std::cell::Cell::new(0));
        let page_keys: Rc<RefCell<HashMap<usize, String>>> = Rc::default();
        if config.can_add {
            let button = gtk4::Button::from_icon_name("list-add-symbolic");
            button.set_tooltip_text(Some(&config.new_label));
            let label = config.new_label.clone();
            button.update_property(&[gtk4::accessible::Property::Label(&label)]);
            button.connect_clicked(move |_| {
                emit(
                    id,
                    Event::Custom {
                        tag: "day:tab-new",
                        num: 0.0,
                        text: String::new(),
                    },
                )
            });
            bar.set_end_action_widget(Some(&button));
        }
        {
            let (s, m) = (suppress.clone(), menu.clone());
            view.connect_selected_page_notify(move |view| {
                if !s.get()
                    && m.get() != 0
                    && let Some(page) = view.selected_page()
                {
                    emit(
                        NodeId(m.get()),
                        Event::SelectionChanged(view.page_position(&page) as i64),
                    );
                }
            });
        }
        {
            let keys = page_keys.clone();
            let s = suppress.clone();
            view.connect_close_page(move |view, page| {
                if s.get() {
                    return gtk4::glib::Propagation::Proceed;
                }
                view.close_page_finish(page, false);
                if !s.get()
                    && let Some(key) = keys.borrow().get(&widget_key(&page.child())).cloned()
                {
                    emit(
                        id,
                        Event::Custom {
                            tag: "day:tab-close",
                            num: 0.0,
                            text: key,
                        },
                    );
                }
                // Cancel native removal; the app accepts later by removing the model key.
                gtk4::glib::Propagation::Stop
            });
        }
        {
            let keys = page_keys.clone();
            let s = suppress.clone();
            view.connect_page_reordered(move |_, page, index| {
                if !s.get()
                    && let Some(key) = keys.borrow().get(&widget_key(&page.child())).cloned()
                {
                    emit(
                        id,
                        Event::Custom {
                            tag: "day:tab-move",
                            num: index as f64,
                            text: key,
                        },
                    );
                }
            });
        }
        let host = Rc::new(Self {
            bar,
            view,
            sidebar,
            keys: RefCell::new(Vec::new()),
            titles: RefCell::new(Vec::new()),
            page_keys,
            menu,
            suppress,
        });
        (container.upcast(), host)
    }
    fn rows(&self, node: NodeId, keys: &[String], titles: &[String]) {
        self.menu.set(node.0);
        *self.keys.borrow_mut() = keys.to_vec();
        *self.titles.borrow_mut() = titles.to_vec();
        for i in 0..self.view.n_pages() {
            let page = self.view.nth_page(i);
            if let Some(key) = self.page_keys.borrow().get(&widget_key(&page.child()))
                && let Some(at) = keys.iter().position(|k| k == key)
                && let Some(title) = titles.get(at)
            {
                page.set_title(title);
            }
        }
    }
}

/// Show/hide the sidebar of this process's `nav(Sidebar)` host — what a
/// [`day_spec::ToolbarItemKind::SidebarToggle`] item drives (docs/toolbars.md). `false` when
/// there is no split host to toggle, which is how the item knows to render disabled.
///
/// Per HOST: the item's action names the host it was built for, so a second window's button
/// toggles that window's own sidebar.
pub(crate) fn toggle_sidebar(host: &Handle) -> bool {
    let key = widget_key(host);
    let split = NAV_STATE.with(|m| {
        let m = m.borrow();
        match m.get(&key).map(|st| &st.present) {
            Some(NavPresent::Split(sp)) => Some(sp.clone()),
            _ => None,
        }
    });
    let Some(sp) = split else {
        return false;
    };
    // The toggle overrides a fit: a user asking for the sidebar on a narrow window gets it,
    // and the next allocation folds it again only if there is still no room.
    sp.sidebar_hidden.set(!sp.sidebar_hidden.get());
    sp.sidebar_fitted.set(false);
    sp.apply_sidebar();
    gtk4::glib::idle_add_local_once(move || ffi_guard::contain((), || nav_report(key)));
    true
}

struct NavState {
    present: NavPresent,
    /// Sidebar+detail split (nav host Sidebar) vs. a pure push/pop stack (`nav_stack`).
    split: bool,
    /// (page GtkFixed key, node id, its AdwNavigationPage) in order (index 0 = sidebar/root).
    /// A split pane's page has no AdwNavigationPage: its cell holds it directly.
    pages: Vec<(usize, NodeId, Option<adw::NavigationPage>)>,
    /// A programmatic pop is in flight: the `popped` handler must not re-emit NavBack.
    suppress: Rc<std::cell::Cell<bool>>,
}

struct NavMenuState {
    listbox: gtk4::ListBox,
    rows: usize,
    /// Programmatic selection in flight: don't re-emit SelectionChanged.
    suppress: Rc<std::cell::Cell<bool>>,
    /// The row index last selected, by Day or by the user: a `row-selected` naming it again is
    /// an echo and is not reported.
    current: Rc<std::cell::Cell<Option<i32>>>,
}

thread_local! {
    /// The newest pending scroll per list host (`scroll_list_when_laid`): a request that has to
    /// wait for layout is superseded by the next one for the same host, so a burst of jumps
    /// ends where the last of them pointed.
    static LIST_SCROLL_SERIAL: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
}

/// Scroll a list host to `row`, or to its end for `None` (`ListPatch::ScrollToRow` /
/// `ScrollToEnd`). `GtkListView::scroll_to` needs v4_12 and Day targets v4_10 (and on GTK 4.20
/// it realized the row without moving the view), so this drives the scrolled window's vertical
/// adjustment instead, at the row's offset by the uniform pitch (the extent over the count;
/// docs/list.md notes the Automatic-height approximation).
///
/// At once when the list is laid out, which is the common case. Otherwise on the frame clock:
/// the request can arrive before the deferred reload (`schedule_list_resize`) has filled the
/// model, or before the list has laid out the rows it filled, and either way the extent would
/// say there is nothing to scroll; it then waits until the model has rows and the extent covers
/// them, a few frames at most. Deferring every request was wrong: GTK runs a frame's tick
/// callbacks newest first, and a burst of jumps that all waited for one frame landed on the
/// oldest, so a `scroll_to_row` could end at the previous target.
///
/// A sectioned list (`pinned`) scrolls a row it has laid out to its real position rather than
/// the uniform-pitch estimate (its headers are not one pitch tall), and stops it just below the
/// pinned header copy instead of under it.
fn scroll_list_when_laid(
    sw: &gtk4::ScrolledWindow,
    row: Option<usize>,
    pinned: Option<Rc<PinnedHeader>>,
) {
    /// The row count of the host's list, or 0 before a model is set.
    fn count(sw: &gtk4::ScrolledWindow) -> usize {
        sw.child()
            .and_then(|c| c.downcast::<gtk4::ListView>().ok())
            .and_then(|lv| lv.model())
            .map(|m| m.n_items() as usize)
            .unwrap_or(0)
    }
    /// The vertical offset of `row` (its end for `None`) by the uniform pitch, or for a sectioned
    /// list by where the row really is.
    fn offset(
        adj: &gtk4::Adjustment,
        n: usize,
        row: Option<usize>,
        pinned: Option<&PinnedHeader>,
    ) -> f64 {
        let end = adj.upper() - adj.page_size();
        let Some(row) = row else {
            return end;
        };
        let estimate = (adj.upper() / n as f64) * row as f64;
        let Some(pinned) = pinned else {
            return estimate.min(end);
        };
        let top = pinned.laid_top(row).map_or(estimate, |y| adj.value() + y);
        (top - pinned.inset_for(row)).clamp(0.0, end.max(0.0))
    }
    let key = widget_key(sw.upcast_ref());
    let adj = sw.vadjustment();
    if count(sw) > 0 && adj.upper() > adj.page_size() {
        // Laid out: on the next main-loop turn, in request order. Not here: the patch arrives
        // inside day-core's tree borrow, and the view's reaction to the new offset reaches
        // `recycle` for the rows that left, which needs that borrow to park their ids. A request
        // still waiting for this host's layout is superseded.
        LIST_SCROLL_SERIAL.with(|m| m.borrow_mut().remove(&key));
        let sw = sw.clone();
        gtk4::glib::idle_add_local_full(gtk4::glib::Priority::DEFAULT, move || {
            let n = count(&sw);
            if n > 0 {
                let adj = sw.vadjustment();
                adj.set_value(offset(&adj, n, row, pinned.as_deref()));
                if let (Some(pinned), Some(row)) = (&pinned, row) {
                    pinned.settle_scroll(&sw, row);
                }
            }
            gtk4::glib::ControlFlow::Break
        });
        return;
    }
    let serial = LIST_SCROLL_SERIAL.with(|m| {
        let mut m = m.borrow_mut();
        let serial = m.get(&key).map_or(1, |s| s + 1);
        m.insert(key, serial);
        serial
    });
    // The callback is an `Fn`, so the frame count lives in a cell.
    let frames = std::cell::Cell::new(0u32);
    sw.add_tick_callback(move |sw, _| {
        frames.set(frames.get() + 1);
        let frames = frames.get();
        let sw = match sw.downcast_ref::<gtk4::ScrolledWindow>() {
            Some(sw) => sw,
            None => return gtk4::glib::ControlFlow::Break,
        };
        // A newer request for this host took over.
        if LIST_SCROLL_SERIAL.with(|m| m.borrow().get(&key) != Some(&serial)) {
            return gtk4::glib::ControlFlow::Break;
        }
        let n = count(sw);
        let adj = sw.vadjustment();
        // Laid out: the rows overflow the viewport, or have had a few frames to (a list that
        // fits has nothing to scroll, and waiting longer would not change that).
        let laid = n > 0 && (adj.upper() > adj.page_size() || frames >= 4);
        if !laid {
            return if frames < 60 {
                gtk4::glib::ControlFlow::Continue
            } else {
                LIST_SCROLL_SERIAL.with(|m| m.borrow_mut().remove(&key));
                gtk4::glib::ControlFlow::Break
            };
        }
        LIST_SCROLL_SERIAL.with(|m| m.borrow_mut().remove(&key));
        adj.set_value(offset(&adj, n, row, pinned.as_deref()));
        if let (Some(pinned), Some(row)) = (&pinned, row) {
            pinned.settle_scroll(sw, row);
        }
        gtk4::glib::ControlFlow::Break
    });
}

fn widget_key(w: &Handle) -> usize {
    w.as_ptr() as usize
}

/// Per-widget live animations (§8.4). Held so libadwaita's frame-clock-driven animations aren't
/// dropped mid-flight; a new animation for the same channel replaces (cancels) the previous one.
/// `cur_transform` is the last transform target, so the next transform tween lerps from it.
#[derive(Default)]
struct GtkAnim {
    opacity: Option<adw::Animation>,
    transform: Option<adw::Animation>,
    cur_transform: Transform,
}

/// Apply Day's layout origin AND the animation transform `t` to `widget` as its `GtkFixed` child
/// transform, about the widget's center. GTK positions a Fixed child *through* its child transform,
/// so the laid-out origin and the animation transform share one slot and must be composed here —
/// otherwise setting the transform would strand the widget at the fixed's (0,0) corner.
fn apply_gtk_transform(fixed: &gtk4::Fixed, widget: &gtk4::Widget, t: Transform, size: Size) {
    let (ox, oy) = NODE_ORIGIN
        .with(|m| m.borrow().get(&widget_key(widget)).copied())
        .unwrap_or((0.0, 0.0));
    if t.is_identity() {
        // Just the laid-out position — but as an explicit translation (not `None`), so it can't be
        // left at (0,0) by racing a later set_child_transform.
        let translate = gtk4::gsk::Transform::new().translate(&gtk4::graphene::Point::new(ox, oy));
        fixed.set_child_transform(widget, Some(&translate));
        return;
    }
    // The laid-out size (from Day) is reliable; `widget.width()` can be 0 before allocation, which
    // would pivot scale/rotation on the top-left corner instead of the center.
    let w = if size.width > 0.0 {
        size.width as f32
    } else {
        widget.width() as f32
    };
    let hgt = if size.height > 0.0 {
        size.height as f32
    } else {
        widget.height() as f32
    };
    let cx = w * t.anchor_x as f32;
    let cy = hgt * t.anchor_y as f32;
    // GSK applies the FIRST-chained op first to a point, so to rotate/scale about the center and
    // then translate: move to origin + pivot, scale, rotate, move back. Folding the laid-out
    // origin (ox, oy) into the outer translation places the transformed box at its layout position.
    let transform = gtk4::gsk::Transform::new()
        .translate(&gtk4::graphene::Point::new(
            ox + cx + t.tx as f32,
            oy + cy + t.ty as f32,
        ))
        .rotate(t.rotate_deg as f32)
        .scale(t.sx as f32, t.sy as f32)
        .translate(&gtk4::graphene::Point::new(-cx, -cy));
    fixed.set_child_transform(widget, Some(&transform));
}

/// Build a libadwaita animation from `from`→`to` matching the `AnimSpec` curve — a spring
/// (`SpringParams` from response/damping) or a timed easing.
fn gtk_animation(
    widget: &gtk4::Widget,
    from: f64,
    to: f64,
    a: &AnimSpec,
    target: adw::CallbackAnimationTarget,
) -> adw::Animation {
    use adw::prelude::*;
    match a.curve {
        Curve::Spring { .. } => {
            // A fixed-duration overshoot (EaseOutBack) over exactly `duration_ms`, so the timing
            // matches the other toolkits (a physics AdwSpringAnimation would settle on its own
            // schedule, not the requested duration).
            let anim = adw::TimedAnimation::new(widget, from, to, a.duration_ms, target);
            anim.set_easing(adw::Easing::EaseOutBack);
            anim.upcast()
        }
        curve => {
            let anim = adw::TimedAnimation::new(widget, from, to, a.duration_ms, target);
            anim.set_easing(match curve {
                Curve::Linear => adw::Easing::Linear,
                Curve::EaseIn => adw::Easing::EaseInCubic,
                Curve::EaseOut => adw::Easing::EaseOutCubic,
                _ => adw::Easing::EaseInOutCubic,
            });
            anim.upcast()
        }
    }
}

/// A nav row's title: fills the row, ellipsized. Ellipsize, or a long feed title makes the row
/// wider than the sidebar: the ListBox's natural width then wins over its allocation and GTK
/// slides every row leftwards, past the window edge, leaving only the tails of the longest names
/// visible. An ellipsizing label reports a one-ellipsis minimum instead, so the list fits the
/// pane it is given.
fn nav_row_label(item: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(item));
    label.set_halign(gtk4::Align::Fill);
    label.set_xalign(0.0);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    label.set_hexpand(true);
    label
}

/// A decorated nav row's content, into `row`: the leading icon, the title, the count and the
/// trailing status glyph. Shared by the first fill and by an in-place refresh (`rows_changed`),
/// which empties the same box and fills it again so the row, its selection and its context menu
/// stay.
fn fill_nav_row(
    row: &gtk4::Box,
    item: &str,
    icon: Option<&str>,
    tint: Option<day_spec::Color>,
    badge: Option<&str>,
    badge_icon: Option<&str>,
    badge_tint: Option<day_spec::Color>,
) {
    if let Some(image) = icon.and_then(|name| tinted_template_icon(name, tint)) {
        image.set_margin_start(2);
        row.append(&image);
    }
    row.append(&nav_row_label(item));
    if let Some(text) = badge {
        let b = gtk4::Label::new(Some(text));
        // `dim-label` is the GNOME treatment for secondary text; numeric alignment keeps a
        // column of counts from jittering as digits change.
        b.add_css_class("dim-label");
        b.add_css_class("numeric");
        b.set_halign(gtk4::Align::End);
        row.append(&b);
    }
    // The trailing status glyph, tinted where the app gave it a meaning-bearing color — the same
    // `tinted_template_icon` path the leading icon takes, so a symbol resolves and recolors
    // identically at either end of the row.
    if let Some(g) = badge_icon.and_then(|name| tinted_template_icon(name, badge_tint)) {
        g.set_halign(gtk4::Align::End);
        row.append(&g);
    }
}

/// Refresh row `i` in place with new words or decorations; `false` when its shape changed (it
/// gained or lost its decorations), which needs the full rebuild.
#[allow(clippy::too_many_arguments)]
fn refresh_nav_row(
    listbox: &gtk4::ListBox,
    i: usize,
    item: &str,
    icon: Option<&str>,
    tint: Option<day_spec::Color>,
    badge: Option<&str>,
    badge_icon: Option<&str>,
    badge_tint: Option<day_spec::Color>,
) -> bool {
    let Some(child) = listbox.row_at_index(i as i32).and_then(|r| r.child()) else {
        return false;
    };
    let decorated = icon.is_some() || badge.is_some() || badge_icon.is_some();
    match (decorated, child.downcast::<gtk4::Box>()) {
        (true, Ok(row)) => {
            while let Some(c) = row.first_child() {
                row.remove(&c);
            }
            fill_nav_row(&row, item, icon, tint, badge, badge_icon, badge_tint);
            true
        }
        (false, Err(child)) => match child.downcast_ref::<gtk4::Label>() {
            Some(label) => {
                label.set_label(item);
                true
            }
            None => false,
        },
        _ => false,
    }
}

/// Build a nav-menu ListBox's rows (an optional template icon left of the label). Shared by the
/// NAV_MENU realize and the data-driven `NavMenuPatch::Items` rebuild.
// The parameters are `NavMenuProps`, minus `selected`: index-aligned per-row decoration arrays.
// Taking the props struct instead would tie this to one caller — `NavMenuPatch::Items` carries
// the same arrays without a props value to hand over.
#[allow(clippy::too_many_arguments)]
fn fill_nav_menu(
    listbox: &gtk4::ListBox,
    items: &[String],
    icons: &[Option<String>],
    badges: &[Option<String>],
    badge_icons: &[Option<String>],
    badge_tints: &[Option<day_spec::Color>],
    sections: &[Option<String>],
    tints: &[Option<day_spec::Color>],
    menus: &[Vec<day_spec::MenuItem>],
) {
    // Idempotent: a caller that clears the ListBox itself must call this before doing so (the
    // popovers are parented to the rows it is about to destroy), and the map entry is taken, so
    // running it twice is a no-op.
    unparent_nav_popovers(listbox);
    for (i, item) in items.iter().enumerate() {
        let decorated = icons.get(i).is_some_and(|o| o.is_some())
            || badges.get(i).is_some_and(|o| o.is_some())
            || badge_icons.get(i).is_some_and(|o| o.is_some());
        let row_widget: gtk4::Widget = if decorated {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            fill_nav_row(
                &row,
                item,
                icons.get(i).and_then(|o| o.as_deref()),
                tints.get(i).copied().flatten(),
                badges.get(i).and_then(|o| o.as_deref()),
                badge_icons.get(i).and_then(|o| o.as_deref()),
                badge_tints.get(i).copied().flatten(),
            );
            listbox.append(&row);
            row.upcast()
        } else {
            let label = nav_row_label(item);
            listbox.append(&label);
            label.upcast()
        };
        // Per-row context menu (docs/menus.md): the same popover + gesture treatment as the
        // piece decorator, on this row's widget.
        if let Some(row_menu) = menus.get(i).filter(|m| !m.is_empty()) {
            let pop = attach_row_context_menu(&row_widget, row_menu);
            NAV_ROW_POPOVERS.with(|m| {
                m.borrow_mut()
                    .entry(listbox.as_ptr() as usize)
                    .or_default()
                    .push(pop);
            });
        }
    }
    // Section headers ride ON the row via GtkListBox's header slot, so they never become rows
    // of their own — indices stay 1:1 with day's items and selection needs no map. The slot is
    // only honored from the box's own header func (`nav_menu_headers`): a header set from
    // outside it is measured but never parented, so it drew as a blank gap. The titles go in
    // the map the func reads, and the rows already appended are asked again.
    NAV_MENU_SECTIONS.with(|m| {
        m.borrow_mut()
            .insert(listbox.as_ptr() as usize, sections.to_vec())
    });
    listbox.invalidate_headers();
}

/// The nav menu's `header-func`: the section title recorded for this row's index, as a dim
/// heading above it, or nothing. Installed once per ListBox at realize.
///
/// GTK calls it from a C trampoline, including while a box is being emptied during teardown,
/// so like every other callback it runs under [`ffi_guard::contain`]: a panic that crossed the
/// trampoline would abort the process. The map is read with `try_with` for the same reason: at
/// thread exit this crate's thread-locals may be gone before the widgets are.
fn nav_menu_headers(row: &gtk4::ListBoxRow, listbox_key: usize) {
    ffi_guard::contain((), || {
        let index = row.index();
        let title = NAV_MENU_SECTIONS
            .try_with(|m| {
                m.borrow()
                    .get(&listbox_key)
                    .and_then(|s| usize::try_from(index).ok().and_then(|i| s.get(i).cloned()))
                    .flatten()
            })
            .ok()
            .flatten();
        let Some(title) = title else {
            row.set_header(None::<&gtk4::Widget>);
            return;
        };
        // Reuse the label GTK already parented for this row when only its text moved.
        if let Some(label) = row.header().and_downcast::<gtk4::Label>() {
            if label.text() != title.as_str() {
                label.set_text(&title);
            }
            return;
        }
        let header = gtk4::Label::new(Some(&title));
        header.add_css_class("heading");
        header.add_css_class("dim-label");
        header.set_xalign(0.0);
        header.set_margin_top(if index == 0 { 2 } else { 10 });
        header.set_margin_bottom(2);
        header.set_margin_start(4);
        row.set_header(Some(&header));
    });
}

/// The bundled glyph `source`, recolored to `t` with its alpha kept as the mask — the recolor
/// both the image piece and the sidebar template icons use (docs/vectors.md "Tint").
/// A decoded bitmap (docs/images.md), held until day-core drops its last handle.
///
/// The pixbuf is kept BESIDE the texture deliberately: cairo draws from a pixbuf
/// (`set_source_pixbuf` — there is no texture source), the pixbuf is what re-encodes to JPEG and
/// BMP, and it is the only cheap place to ask whether the image carries alpha. Downloading the
/// texture's pixels on every canvas redraw is precisely the cost this avoids.
struct GtkBitmap {
    texture: gtk4::gdk::Texture,
    pixbuf: Option<gtk4::gdk_pixbuf::Pixbuf>,
    info: day_spec::BitmapInfo,
}

/// Point a `GtkPicture` at an [`day_spec::ImageSource`] (docs/images.md).
///
/// Shared by realize and the `Source` patch, so a swapped source loads exactly what a fresh
/// realize would have.
fn set_picture_source(pic: &gtk4::Picture, source: &day_spec::ImageSource) {
    match source {
        day_spec::ImageSource::Named(name) if !name.is_empty() => {
            if let Some(path) = day_spec::resource::resolve_vector_svg(name)
                && let Some(svg) = vector::SvgPaintable::load(&path, None, None)
            {
                pic.set_paintable(Some(&svg));
                return;
            }
            // Prefer the native GResource entry `/day/images/<name>` (§18.3); else a loose file.
            let res_path = format!("/day/images/{name}");
            if gtk4::gio::resources_lookup_data(&res_path, gtk4::gio::ResourceLookupFlags::NONE)
                .is_ok()
            {
                pic.set_resource(Some(&res_path));
            } else if let Some(path) = day_spec::resource::resolve_image_file(name) {
                pic.set_filename(Some(&path));
            }
        }
        day_spec::ImageSource::Named(_) => {}
        // Bytes the app already holds, with no staged resource behind them.
        day_spec::ImageSource::Bytes(bytes) => {
            let data = gtk4::glib::Bytes::from_owned(bytes.as_ref().clone());
            if let Ok(texture) = gtk4::gdk::Texture::from_bytes(&data) {
                pic.set_paintable(Some(&texture));
            }
        }
        // Already decoded: share the one texture rather than parsing the bytes twice.
        day_spec::ImageSource::Decoded(id) => {
            let texture = BITMAPS.with(|m| m.borrow().get(&id.0).map(|b| b.texture.clone()));
            if let Some(texture) = texture {
                pic.set_paintable(Some(&texture));
            }
        }
    }
}

/// The body of `render_bitmap`: `ops` painted at `scale` onto a transparent ARGB32 surface of
/// `export_pixels(size, scale)`, stored in `BITMAPS` under `id`. Errors carry cairo's text.
fn render_bitmap_to(
    id: day_spec::BitmapId,
    ops: &[DrawOp],
    size: Size,
    scale: f64,
) -> Result<day_spec::BitmapInfo, String> {
    use gtk4::cairo;
    let (pw, ph) = day_spec::export_pixels(size, scale);
    // day-core caps a side at 16,384 pixels before calling, so these always fit.
    let w = i32::try_from(pw).map_err(|e| e.to_string())?;
    let h = i32::try_from(ph).map_err(|e| e.to_string())?;
    let mut surface =
        cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).map_err(|e| e.to_string())?;
    {
        // A new image surface is cleared to transparent black, the export's background.
        let cr = cairo::Context::new(&surface).map_err(|e| e.to_string())?;
        cr.scale(scale, scale);
        let (nums, texts) = day_spec::encode_ops(ops);
        cairo_draw(&cr, &nums, &texts);
        cr.status().map_err(|e| e.to_string())?;
    }
    surface.flush();
    let stride = usize::try_from(surface.stride()).map_err(|e| e.to_string())?;
    let bytes = surface.data().map_err(|e| e.to_string())?.to_vec();
    // CAIRO_FORMAT_ARGB32 is premultiplied ARGB in NATIVE-endian 32-bit words: B, G, R, A in
    // memory on a little-endian machine — GDK_MEMORY_DEFAULT, spelled out per byte order.
    let format = if cfg!(target_endian = "little") {
        gtk4::gdk::MemoryFormat::B8g8r8a8Premultiplied
    } else {
        gtk4::gdk::MemoryFormat::A8r8g8b8Premultiplied
    };
    let texture: gtk4::gdk::Texture =
        gtk4::gdk::MemoryTexture::new(w, h, format, &gtk4::glib::Bytes::from_owned(bytes), stride)
            .upcast();
    // The pixbuf twin is what `DrawOp::Image` paints and the JPEG/BMP encoders read, exactly as
    // for a decoded bitmap.
    let pixbuf = gtk4::gdk::pixbuf_get_from_texture(&texture);
    let info = day_spec::BitmapInfo {
        pixels: Size::new(f64::from(pw), f64::from(ph)),
        scale,
        format: None,
        has_alpha: true,
    };
    BITMAPS.with(|m| {
        m.borrow_mut().insert(
            id.0,
            GtkBitmap {
                texture,
                pixbuf,
                info,
            },
        )
    });
    Ok(info)
}

/// The body of `render_pdf`: every page through `cairo_draw` onto one `PdfSurface` writing into
/// memory, with the title and author in the document's info dictionary.
fn render_pdf_bytes(
    pages: &[day_spec::PdfPage],
    info: &day_spec::PdfInfo,
) -> Result<Vec<u8>, String> {
    let first = pages.first().map_or(Size::new(1.0, 1.0), |p| p.size);
    let surface = cairo::PdfSurface::for_stream(first.width, first.height, Vec::<u8>::new())
        .map_err(|e| e.to_string())?;
    if let Some(title) = &info.title {
        surface
            .set_metadata(cairo::PdfMetadata::Title, title)
            .map_err(|e| e.to_string())?;
    }
    if let Some(author) = &info.author {
        surface
            .set_metadata(cairo::PdfMetadata::Author, author)
            .map_err(|e| e.to_string())?;
    }
    for page in pages {
        // Takes effect for the page about to start: cairo reads the size at its first drawing.
        surface
            .set_size(page.size.width, page.size.height)
            .map_err(|e| e.to_string())?;
        let cr = cairo::Context::new(&surface).map_err(|e| e.to_string())?;
        let (nums, texts) = day_spec::encode_ops(&page.ops);
        cairo_draw(&cr, &nums, &texts);
        cr.show_page().map_err(|e| e.to_string())?;
    }
    // Finishing flushes the trailer and hands the Vec back; it can only panic on a second call
    // or a stream type that re-enters the surface, and a Vec does neither.
    let stream = surface
        .finish_output_stream()
        .map_err(|e| e.error.to_string())?;
    stream
        .downcast::<Vec<u8>>()
        .map(|bytes| *bytes)
        .map_err(|_| "the PDF stream was not the buffer it was created with".to_string())
}

/// Re-encode a decoded bitmap (docs/images.md).
///
/// PNG and TIFF come from the texture's own writers where nothing was rescaled; everything else
/// goes through the pixbuf, which is what gdk-pixbuf can actually write.
fn encode_bitmap(
    id: day_spec::BitmapId,
    spec: &day_spec::EncodeSpec,
) -> Result<Vec<u8>, day_spec::ImageError> {
    use day_spec::ImageFormat as F;
    let held = BITMAPS.with(|m| {
        m.borrow()
            .get(&id.0)
            .map(|b| (b.texture.clone(), b.pixbuf.clone(), b.info))
    });
    let (texture, pixbuf, info) = held.ok_or(day_spec::ImageError::Gone)?;
    if spec.fit.is_none() {
        match spec.format {
            F::Png => return Ok(texture.save_to_png_bytes().to_vec()),
            F::Tiff => return Ok(texture.save_to_tiff_bytes().to_vec()),
            _ => {}
        }
    }
    let kind = match spec.format {
        F::Png => "png",
        F::Tiff => "tiff",
        F::Jpeg => "jpeg",
        F::Bmp => "bmp",
        // gdk-pixbuf ships no WebP, GIF or HEIF writer; `encode_formats` says so up front.
        _ => return Err(day_spec::ImageError::Encode),
    };
    let pixbuf = pixbuf.ok_or(day_spec::ImageError::Encode)?;
    // `fit` scales the longest side down first. A box LARGER than the original is ignored:
    // upscaling on an export path inflates the bytes without adding any detail.
    let pixbuf = match spec.fit {
        Some(fit) if info.pixels.width > 0.0 && info.pixels.height > 0.0 => {
            let s = (fit.width / info.pixels.width)
                .min(fit.height / info.pixels.height)
                .min(1.0);
            if s < 1.0 {
                pixbuf
                    .scale_simple(
                        ((info.pixels.width * s).round() as i32).max(1),
                        ((info.pixels.height * s).round() as i32).max(1),
                        gtk4::gdk_pixbuf::InterpType::Bilinear,
                    )
                    .ok_or(day_spec::ImageError::Encode)?
            } else {
                pixbuf
            }
        }
        _ => pixbuf,
    };
    let quality;
    let opts: Vec<(&str, &str)> = match (spec.format, spec.quality) {
        (F::Jpeg, Some(q)) => {
            quality = format!("{}", (q.clamp(0.0, 1.0) * 100.0).round() as i32);
            vec![("quality", quality.as_str())]
        }
        _ => Vec::new(),
    };
    pixbuf
        .save_to_bufferv(kind, &opts)
        .map_err(|_| day_spec::ImageError::Encode)
}

fn tinted_image_texture(source: &str, t: day_spec::Color) -> Option<gtk4::gdk::Texture> {
    let path = day_spec::resource::resolve_image_file(source)?;
    let pixbuf = gtk4::gdk_pixbuf::Pixbuf::from_file(&path).ok()?;
    let pixbuf = if pixbuf.has_alpha() {
        pixbuf
    } else {
        pixbuf.add_alpha(false, 0, 0, 0).ok()?
    };
    recolor_pixbuf(
        &pixbuf,
        (t.r * 255.0) as u8,
        (t.g * 255.0) as u8,
        (t.b * 255.0) as u8,
    );
    Some(gtk4::gdk::Texture::for_pixbuf(&pixbuf))
}

/// A realized image's source and paint (`IMAGE_SOURCE`): the bundled NAME (empty for bytes or
/// a decode, which cannot be recolored), the tint on it now, whether it is a template glyph
/// (docs/vectors.md "Template"), and the picture itself — weakly, so a theme switch can
/// recolor every untinted template to the new foreground without keeping any alive.
#[derive(Clone)]
struct ImageEntry {
    name: String,
    tint: Option<day_spec::Color>,
    template: bool,
    picture: gtk4::glib::WeakRef<gtk4::Picture>,
}

/// The theme foreground a template glyph with no tint of its own draws in (docs/vectors.md
/// "Template"): the same near-white / near-black the sidebar's template icons use.
fn template_foreground() -> day_spec::Color {
    if adw::StyleManager::default().is_dark() {
        day_spec::Color::hex(0xFFFFFF)
    } else {
        day_spec::Color::hex(0x1A1A1A)
    }
}

/// Paint a picture from its recorded source with `tint`, or — for a template — with the theme
/// foreground; a plain image with no tint reloads the file as authored.
fn repaint_image(pic: &gtk4::Picture, source: &str, tint: Option<day_spec::Color>, template: bool) {
    let tint = tint.or_else(|| template.then(template_foreground));
    if let Some(path) = day_spec::resource::resolve_vector_svg(source) {
        if let Some(svg) = pic.paintable().and_downcast::<vector::SvgPaintable>()
            && svg.source_is(&path)
        {
            svg.set_tint(tint);
            return;
        }
        if let Some(svg) = vector::SvgPaintable::load(&path, tint, None) {
            pic.set_paintable(Some(&svg));
            return;
        }
    }
    match tint.and_then(|t| tinted_image_texture(source, t)) {
        Some(texture) => pic.set_paintable(Some(&texture)),
        None => set_picture_source(pic, &day_spec::ImageSource::Named(source.to_owned())),
    }
}

/// A theme switch: every untinted template glyph takes the new foreground. The recolor is
/// baked into the texture, so nothing else would repaint it.
fn retint_template_images() {
    let mut live = Vec::new();
    IMAGE_SOURCE.with(|t| {
        t.for_each(|_, entry| {
            if entry.template
                && entry.tint.is_none()
                && let Some(pic) = entry.picture.upgrade()
            {
                live.push((pic, entry.name.clone()));
            }
        })
    });
    for (pic, name) in live {
        repaint_image(&pic, &name, None, true);
    }
}

/// Release this listbox's row popovers, and do it before the rows themselves go.
///
/// A `PopoverMenu` is parented to its row's label, and GTK4 requires a popover to be unparented
/// while that parent is still alive. Destroying the rows first leaves each popover pointing at
/// freed memory — GTK says so at the time ("Finalizing GtkLabel, but it still has children left:
/// GtkPopoverMenu") — and the later `unparent()` then walks a dead parent chain inside
/// `gtk_widget_unparent` → `gtk_accessible_update_children`, which is where the
/// `gtk_widget_is_ancestor`/`gtk_accessible_get_accessible_role` criticals came from.
///
/// Taking the entry makes this idempotent, so callers can be defensive without double-unparenting.
fn unparent_nav_popovers(listbox: &gtk4::ListBox) {
    NAV_ROW_POPOVERS.with(|m| {
        for pop in m
            .borrow_mut()
            .remove(&(listbox.as_ptr() as usize))
            .unwrap_or_default()
        {
            pop.unparent();
        }
    });
}

/// Attach a context menu to one nav row (docs/menus.md): the same gio-menu + PopoverMenu +
/// secondary-click/long-press gestures the piece decorator uses, minus its per-widget
/// bookkeeping (the caller owns the popover's lifetime via [`NAV_ROW_POPOVERS`]).
fn attach_row_context_menu(w: &gtk4::Widget, items: &[day_spec::MenuItem]) -> gtk4::PopoverMenu {
    let group = gtk4::gio::SimpleActionGroup::new();
    let model = build_gio_menu(items, &group);
    w.insert_action_group("daymenu", Some(&group));
    w.set_can_target(true);
    let popover = gtk4::PopoverMenu::from_model(Some(&model));
    popover.set_parent(w);
    popover.set_has_arrow(false);
    let popup_at = {
        let pop = popover.clone();
        move |x: f64, y: f64| {
            pop.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            pop.popup();
        }
    };
    let click = gtk4::GestureClick::new();
    click.set_button(3);
    let f = popup_at.clone();
    click.connect_pressed(move |_, _n, x, y| f(x, y));
    w.add_controller(click);
    let long = gtk4::GestureLongPress::new();
    long.connect_pressed(move |_, x, y| popup_at(x, y));
    w.add_controller(long);
    popover
}

/// Recolor a pixbuf in place: every pixel's RGB becomes (`fr`,`fg`,`fb`), alpha untouched — the
/// glyph's shape and antialiasing survive as the mask. Shared by the sidebar template icons and
/// the `vector(…)` piece's tint (docs/vectors.md).
fn recolor_pixbuf(pixbuf: &gtk4::gdk_pixbuf::Pixbuf, fr: u8, fg: u8, fb: u8) {
    let width = pixbuf.width() as usize;
    let height = pixbuf.height() as usize;
    let rowstride = pixbuf.rowstride() as usize;
    let n_channels = pixbuf.n_channels() as usize;
    if n_channels < 4 {
        return;
    }
    // SAFETY: the returned slice aliases the pixbuf's own pixel store, which we exclusively
    // own here; we only touch bytes inside the documented rowstride/width bounds.
    let pixels = unsafe { pixbuf.pixels() };
    for y in 0..height {
        let row = y * rowstride;
        for x in 0..width {
            let i = row + x * n_channels;
            if i + 3 < pixels.len() {
                pixels[i] = fr;
                pixels[i + 1] = fg;
                pixels[i + 2] = fb;
                // pixels[i + 3] (alpha) is left untouched — it is the glyph mask.
            }
        }
    }
}

/// Load a bundled template image (black glyph on transparent) and tint it to the current theme's
/// foreground so it's visible in both light and dark mode — a raw black PNG vanishes on a
/// dark-mode sidebar or toolbar. Every RGB pixel is recolored to the foreground; the source ALPHA
/// is kept as the mask, so the glyph's shape and antialiasing survive. Returns a ~20px `GtkImage`
/// or `None` if the name doesn't resolve / the file can't be decoded. Used by the sidebar rows and
/// by a toolbar button whose icon is a bundled `Icon::Image` (docs/toolbars.md).
/// Display size of a sidebar / toolbar template glyph, in points.
const ICON_PX: i32 = 20;

/// Day's own drawing of a [`Symbol`], tinted to the theme foreground.
///
/// The fallback for a symbol the ICON THEME does not have. `view-filter-symbolic` and friends
/// ship with GNOME and with nothing else, so a GTK app run anywhere but a GNOME desktop drew
/// toolbar items with no icon at all (docs/toolbars.md). The outline comes from day-spec, so the
/// shape is the same one every other backend falls back to.
pub(crate) fn symbol_outline_icon(sym: day_spec::Symbol) -> Option<gtk4::Image> {
    let svg = sym.outline_svg()?;
    let stream = gtk4::gio::MemoryInputStream::from_bytes(&gtk4::glib::Bytes::from(svg.as_bytes()));
    let pixbuf = gtk4::gdk_pixbuf::Pixbuf::from_stream_at_scale(
        &stream,
        ICON_PX * 2,
        ICON_PX * 2,
        true,
        gtk4::gio::Cancellable::NONE,
    )
    .ok()?;
    let pixbuf = if pixbuf.has_alpha() {
        pixbuf
    } else {
        pixbuf.add_alpha(false, 0, 0, 0).ok()?
    };
    let (r, g, b) = if adw::StyleManager::default().is_dark() {
        (0xffu8, 0xffu8, 0xffu8)
    } else {
        (0x1au8, 0x1au8, 0x1au8)
    };
    recolor_pixbuf(&pixbuf, r, g, b);
    let texture = gtk4::gdk::Texture::for_pixbuf(&pixbuf);
    let image = gtk4::Image::from_paintable(Some(&texture));
    image.set_pixel_size(ICON_PX);
    Some(image)
}

thread_local! {
    /// Each realized list item's current props, by its widget: a patch updates one part and the
    /// item is re-dressed from the whole (docs/list.md "List items").
    static LIST_ITEMS: std::cell::RefCell<std::collections::HashMap<usize, day_spec::props::ListItemProps>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// A list row's standard content (docs/list.md "List items"), as a GNOME list row lays it out:
/// a 16px icon slot, the title over a dim caption subtitle, a dim trailing value, and the
/// selection check (`object-select-symbolic`) in the accent color, 12px apart, 8px above and
/// below and at least 44px tall.
fn make_list_item(p: &day_spec::props::ListItemProps) -> gtk4::Widget {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    row.set_margin_top(8);
    row.set_margin_bottom(8);
    let slot = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    slot.set_valign(gtk4::Align::Center);
    row.append(&slot);
    let texts = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    texts.set_valign(gtk4::Align::Center);
    texts.set_hexpand(true);
    let title = gtk4::Label::new(None);
    title.set_xalign(0.0);
    title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    let subtitle = gtk4::Label::new(None);
    subtitle.set_xalign(0.0);
    subtitle.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    subtitle.add_css_class("dim-label");
    subtitle.add_css_class("caption");
    texts.append(&title);
    texts.append(&subtitle);
    row.append(&texts);
    let detail = gtk4::Label::new(None);
    detail.add_css_class("dim-label");
    detail.add_css_class("numeric");
    row.append(&detail);
    let check = gtk4::Image::from_icon_name("object-select-symbolic");
    check.add_css_class("accent");
    row.append(&check);
    let w: gtk4::Widget = row.upcast();
    LIST_ITEMS.with(|m| m.borrow_mut().insert(widget_key(&w), p.clone()));
    dress_list_item(&w, p);
    w
}

/// Show `p` on a list item's row box (see [`make_list_item`]).
fn dress_list_item(w: &gtk4::Widget, p: &day_spec::props::ListItemProps) {
    let Some(slot) = w.first_child().and_downcast::<gtk4::Box>() else {
        return;
    };
    while let Some(c) = slot.first_child() {
        slot.remove(&c);
    }
    let image = match &p.icon {
        Some(day_spec::Icon::Image(name)) => tinted_template_icon(name, p.icon_tint),
        Some(day_spec::Icon::Symbol(sym)) => symbol_icon_name(*sym)
            .map(|n| gtk4::Image::from_icon_name(&n))
            .or_else(|| symbol_outline_icon(*sym)),
        None => None,
    };
    if let Some(img) = &image {
        img.set_pixel_size(16);
        slot.append(img);
    }
    slot.set_visible(image.is_some());
    let Some(texts) = slot.next_sibling().and_downcast::<gtk4::Box>() else {
        return;
    };
    if let Some(title) = texts.first_child().and_downcast::<gtk4::Label>() {
        title.set_label(&p.title);
        if let Some(subtitle) = title.next_sibling().and_downcast::<gtk4::Label>() {
            subtitle.set_label(&p.subtitle);
            subtitle.set_visible(!p.subtitle.is_empty());
        }
    }
    if let Some(detail) = texts.next_sibling().and_downcast::<gtk4::Label>() {
        detail.set_label(&p.detail);
        detail.set_visible(!p.detail.is_empty());
        if let Some(check) = detail.next_sibling() {
            check.set_visible(p.checked);
        }
    }
    let words = [&p.title, &p.subtitle, &p.detail]
        .into_iter()
        .filter(|w| !w.is_empty())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    w.update_property(&[gtk4::accessible::Property::Label(&words)]);
    w.update_state(&[gtk4::accessible::State::Selected(Some(p.checked))]);
}

fn tinted_template_icon(name: &str, tint: Option<day_spec::Color>) -> Option<gtk4::Image> {
    let svg_path = day_spec::resource::resolve_vector_svg(name).or_else(|| {
        let path = std::path::Path::new(name);
        (path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
            && path.is_file())
        .then(|| path.to_path_buf())
    });
    if let Some(path) = svg_path {
        let svg = vector::SvgPaintable::load(
            &path,
            Some(tint.unwrap_or_else(template_foreground)),
            Some(f64::from(ICON_PX)),
        )?;
        let image = gtk4::Image::from_paintable(Some(&svg));
        image.set_halign(gtk4::Align::Center);
        image.set_valign(gtk4::Align::Center);
        return Some(image);
    }

    // Ordinary bitmap icons keep their pixel recolor path. SVGs above never use a
    // fixed-size texture, including when parsing fails.
    let path = day_spec::resource::resolve_image_file(name)?;
    let pixbuf = gtk4::gdk_pixbuf::Pixbuf::from_file(&path).ok()?;
    let pixbuf = if pixbuf.has_alpha() {
        pixbuf
    } else {
        pixbuf.add_alpha(false, 0, 0, 0).ok()?
    };

    // The row's own tint when given (docs/vectors.md); else the theme foreground —
    // near-white in dark mode, near-black in light mode.
    let (fr, fg, fb) = match tint {
        Some(t) => (
            (t.r * 255.0) as u8,
            (t.g * 255.0) as u8,
            (t.b * 255.0) as u8,
        ),
        None => {
            if adw::StyleManager::default().is_dark() {
                (0xffu8, 0xffu8, 0xffu8)
            } else {
                (0x1au8, 0x1au8, 0x1au8)
            }
        }
    };

    recolor_pixbuf(&pixbuf, fr, fg, fb);

    let texture = gtk4::gdk::Texture::for_pixbuf(&pixbuf);
    let image = gtk4::Image::from_paintable(Some(&texture));
    image.set_pixel_size(ICON_PX);
    Some(image)
}

/// Install (once) the CSS that makes Day scroll viewports transparent, so a backdrop layered
/// behind a scroll (zstack) shows through — matching AppKit's `setDrawsBackground(false)`.
fn scroll_transparent_css() {
    if DONE.with(|c| c.replace(true)) {
        return;
    }
    let p = gtk4::CssProvider::new();
    p.load_from_data(
        "scrolledwindow.day-scroll, scrolledwindow.day-scroll > viewport { background: none; }",
    );
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &p,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// Apply a `background`/`corner_radius` surface to a container widget via a scoped CSS provider
/// (a unique `.day-surface-N` class added to just this widget). `overflow: hidden` rounds the
/// child clip. Idempotent — reuses the provider on a reactive background patch.
fn apply_surface(w: &Handle, bg: Option<day_spec::Color>, corner_radius: f64, clips: bool) {
    let key = widget_key(w);
    let class = format!("day-surface-{key}");
    let provider = match SURFACE.with(|t| t.get(key)) {
        Some(p) => p,
        None => {
            let p = gtk4::CssProvider::new();
            if let Some(display) = gtk4::gdk::Display::default() {
                gtk4::style_context_add_provider_for_display(
                    &display,
                    &p,
                    gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
            }
            w.add_css_class(&class);
            SURFACE.with(|t| t.insert(key, p.clone()));
            p
        }
    };
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

/// Emit each page's content size so NavLayout re-lays it (enqueue-only, §8.3). Split: each
/// pane's page gets its pane's width — the dividers are user-draggable, so this runs on every
/// drag and every allocation; stack: every page fills the host.
fn nav_report(host_key: usize) {
    let reports: Vec<(NodeId, Size)> = NAV_STATE.with(|m| {
        let m = m.borrow();
        let Some(state) = m.get(&host_key) else {
            return Vec::new();
        };
        let (hw, hh) = match &state.present {
            NavPresent::Split(sp) => (sp.outer.width() as f64, sp.outer.height() as f64),
            NavPresent::Stack(nv) => (nv.width() as f64, nv.height() as f64),
            NavPresent::Documents(doc) => (doc.view.width() as f64, doc.view.height() as f64),
            NavPresent::Suite { stack, .. } => (stack.width() as f64, stack.height() as f64),
        };
        if hw <= 0.0 || hh <= 0.0 {
            return Vec::new();
        }
        // A pane's ALLOCATED width once it has one, its remembered width until then — and
        // while hidden, so a reveal never re-lays the pane's content up from zero (the
        // inspector's rule). Echoing an allocation is safe here: the pane is a filling
        // `DayCell`, so what Day lays into it never feeds back as a GTK minimum.
        let pane_w = |cell: &DayCell, remembered: i32| {
            if cell.is_visible() && cell.width() > 0 {
                cell.width() as f64
            } else {
                f64::from(remembered)
            }
        };
        let split = match &state.present {
            NavPresent::Split(sp) if state.split => Some(sp),
            _ => None,
        };
        state
            .pages
            .iter()
            .map(|(key, id, _)| {
                let size = match split {
                    Some(sp) => {
                        let pane = NAV_PAGE_PANES
                            .with(|m| m.borrow().get(key).copied())
                            .unwrap_or_default();
                        match pane {
                            day_spec::props::Pane::Sidebar => {
                                Size::new(pane_w(&sp.sidebar, sp.sidebar_w.get()), hh)
                            }
                            day_spec::props::Pane::List => {
                                Size::new(pane_w(&sp.list, sp.list_w.get()), hh)
                            }
                            day_spec::props::Pane::Detail => {
                                // Before the first allocation: what the other panes leave.
                                let taken = if sp.sidebar_showing() {
                                    sp.sidebar_w.get() + NavSplit::HANDLE
                                } else {
                                    0
                                } + if sp.list_shown.get() {
                                    sp.list_w.get() + NavSplit::HANDLE
                                } else {
                                    0
                                };
                                let fallback = (hw - f64::from(taken)).max(0.0);
                                Size::new(pane_w(&sp.detail, fallback as i32), hh)
                            }
                        }
                    }
                    // A stack, a suite, or a `nav_stack`: every page fills the host — a
                    // pushed stack page its cell under the header bar, once allocated (its
                    // own allocation reports it live; this is the whole-host moment).
                    None => STACK_PAGE_CELLS
                        .with(|m| m.borrow().get(key).cloned())
                        .filter(|cell| cell.width() > 0 && cell.height() > 0)
                        .map(|cell| Size::new(cell.width() as f64, cell.height() as f64))
                        .unwrap_or(Size::new(hw, hh)),
                };
                (*id, size)
            })
            .collect()
    });
    for (id, size) in reports {
        emit(id, Event::FrameChanged(size));
    }
}

/// A split pane's LIVE report: its page's size, emitted from the pane cell's own allocation —
/// before the cell allocates the page — so Day lays the page to the width the divider is at
/// in this very pass (`DayCell::on_allocate` explains). `nav_report` is the whole-host
/// counterpart for the moments a pane is not being allocated (a page joining, a toggle).
fn nav_pane_report(host_key: usize, pane: day_spec::props::Pane, width: i32, height: i32) {
    if width <= 0 || height <= 0 {
        return;
    }
    // Collected first: the dispatch may insert or remove a page, which borrows `NAV_STATE`.
    let ids: Vec<NodeId> = NAV_STATE.with(|m| {
        let m = m.borrow();
        let Some(state) = m.get(&host_key) else {
            return Vec::new();
        };
        state
            .pages
            .iter()
            .filter(|(key, ..)| {
                NAV_PAGE_PANES
                    .with(|p| p.borrow().get(key).copied())
                    .unwrap_or_default()
                    == pane
            })
            .map(|(_, id, _)| *id)
            .collect()
    });
    let size = Size::new(f64::from(width), f64::from(height));
    for id in ids {
        emit(id, Event::FrameChanged(size));
    }
}

// ---------------------------------------------------------------------------
// Inspector (docs/inspector.md): a GtkPaned with the panel at the trailing (or leading) edge
// — the nav split's device, mirrored. The divider is draggable, as Qt's is; the panel keeps
// the width it was dragged to across a hide and a reveal.
// ---------------------------------------------------------------------------

/// The narrowest an inspector panel can be dragged.
const INSPECTOR_MIN_W: f64 = 160.0;

struct InspectorState {
    paned: gtk4::Paned,
    /// The two pane cells: filling, so a pane's Day-laid content never becomes its minimum
    /// (a 280-wide form inside a pane briefly measured narrower would otherwise inflate the
    /// pane's minimum until the panel has no room to show).
    content: DayCell,
    panel: DayCell,
    /// The panel's width: the piece's preference, then wherever the divider was dragged.
    width: f64,
    /// Panel at the trailing edge (the paned's END child) or the leading one (its START).
    trailing: bool,
    /// The panel is showing (`InspectorProps::visible`, then `InspectorPatch::Visible`).
    shown: bool,
    /// Each attached pane: `(pane NodeId, is-panel, the pane's own GtkFixed)`, for frame
    /// reports.
    panes: Vec<(NodeId, bool, Handle)>,
}

impl InspectorState {
    /// Put the divider where the panel takes `width`: a trailing panel's position is measured
    /// from the far side of the host, so it needs the host's width — `None` before the first
    /// allocation, when GTK's own natural-size split places it from the cell's request.
    fn place(&self, host_w: i32) {
        let width = self.width.round() as i32;
        if self.trailing {
            if host_w > 0 {
                self.paned
                    .set_position((host_w - width - NavSplit::HANDLE).max(0));
            }
        } else {
            self.paned.set_position(width);
        }
    }
}

// ---------------------------------------------------------------------------
// Split (docs/split.md): a GtkPaned of two filling cells, either orientation, both resizing
// with the host (proportionally, which is what keeps a dragged share through a window
// resize), the divider placed by the bound share and a drag reported back as the new share.
// ---------------------------------------------------------------------------

struct SplitState {
    paned: gtk4::Paned,
    first: DayCell,
    second: DayCell,
    node: NodeId,
    /// The first pane's share as last patched or reported.
    share: f64,
    min_pane: f64,
    /// The divider has been placed once; before that the paned has no size to place it in.
    placed: bool,
    /// Day is moving the divider itself: the position-notify that follows is not a report.
    suppress: bool,
    /// Each attached pane: `(pane NodeId, is-second, the pane's own GtkFixed)`.
    panes: Vec<(NodeId, bool, Handle)>,
}

impl SplitState {
    fn stacked(&self) -> bool {
        self.paned.orientation() == gtk4::Orientation::Vertical
    }
    /// The length the two panes share: the paned's extent along its axis, minus the handle.
    fn usable(&self) -> f64 {
        let len = if self.stacked() {
            self.paned.height()
        } else {
            self.paned.width()
        };
        f64::from(len - NavSplit::HANDLE).max(0.0)
    }
    /// Put the divider where the first pane takes the share, silently, from the paned's own
    /// allocated length.
    fn place(&mut self) {
        let len = if self.stacked() {
            self.paned.height()
        } else {
            self.paned.width()
        };
        self.place_along(len);
    }
    /// [`Self::place`] for a length the paned is about to be allocated: the host's hook runs
    /// before the paned is allocated (`DayCell::size_allocate`), when the paned still measures
    /// zero, so the first placement takes the host's length instead. GtkPaned keeps a
    /// position set before its allocation and clamps it to the panes' minimums then.
    fn place_along(&mut self, len: i32) {
        let usable = f64::from(len - NavSplit::HANDLE).max(0.0);
        if usable <= 0.0 {
            return;
        }
        self.suppress = true;
        self.paned
            .set_position((self.share * usable).round() as i32);
        self.suppress = false;
        self.placed = true;
    }
    /// The minimum a pane may be dragged to, along the current axis only.
    fn request_minimums(&self) {
        let m = self.min_pane.round() as i32;
        let (w, h) = if self.stacked() { (-1, m) } else { (m, -1) };
        self.first.set_size_request(w, h);
        self.second.set_size_request(w, h);
    }
}

/// Emit each inspector pane's content size so `InspectorLayout` re-lays it (the nav_report
/// counterpart). The panel reports its width even while hidden, so revealing it never re-lays
/// the panel's content from zero.
fn inspector_report(host_key: usize) {
    let reports: Vec<(NodeId, Size)> = INSPECTOR_STATE
        .with(|t| t.get(host_key))
        .map(|state| {
            let state = state.borrow();
            let (hw, hh) = (state.paned.width() as f64, state.paned.height() as f64);
            if hw <= 0.0 || hh <= 0.0 {
                return Vec::new();
            }
            // The panel's allocation once it has one (the divider may have moved it), its
            // remembered width until then and while hidden; the content gets the rest.
            let panel_w = if state.shown && state.panel.width() > 0 {
                state.panel.width() as f64
            } else {
                state.width
            };
            let shown = if state.shown {
                panel_w + f64::from(NavSplit::HANDLE)
            } else {
                0.0
            };
            state
                .panes
                .iter()
                .map(|(id, panel, _)| {
                    let size = if *panel {
                        Size::new(panel_w, hh)
                    } else {
                        Size::new((hw - shown).max(0.0), hh)
                    };
                    (*id, size)
                })
                .collect()
        })
        .unwrap_or_default();
    for (id, size) in reports {
        emit(id, Event::FrameChanged(size));
    }
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
    selection: Option<ListSelection>,
    /// The cells GTK has bound right now, for a reload to re-bind in place.
    bound: BoundCells,
    /// A sectioned list's pinned header copy (docs/list.md "Sections"); `None` on a plain list.
    pinned: Option<Rc<PinnedHeader>>,
}

/// The pinned copy of a sectioned list's current header (docs/list.md "Sections";
/// `Cap::ListSections` is Emulated here).
///
/// GtkListView's own section headers (GtkSectionModel + header factory, GTK 4.12) scroll away with
/// their rows, so a sectioned list's headers are ordinary rows of the flat model, and the list's
/// host is a GtkOverlay over its scrolled window. One extra `DayCell` above it holds the header of
/// the section the topmost visible row belongs to, bound through `bind_pinned` (a row bind that
/// leaves the copy without element ids), with an opaque strip behind it so the rows do not show
/// through. The overlay's `get-child-position` places both: GTK runs it right after it allocated
/// the scrolled window in the same pass, so the row positions it reads are this frame's, and the
/// copy moves in step with the scroll, pushed up as the next header arrives. A scroll queues the
/// overlay's allocation for exactly that.
struct PinnedHeader {
    overlay: gtk4::Overlay,
    listview: gtk4::ListView,
    /// The opaque background, the list's full width.
    strip: gtk4::Box,
    /// The header copy, at the rows' own cell inset and width.
    cell: DayCell,
    source: Rc<RefCell<Option<ListSource>>>,
    bound: BoundCells,
    /// The display row the copy holds, `None` before its first bind and after a reload.
    row: std::cell::Cell<Option<usize>>,
    /// The row a posted rebind will bind, while one is pending.
    wanted: std::cell::Cell<Option<usize>>,
    /// The last `(topmost row, its header)`, so a scroll only scans the rows it moved across.
    last: std::cell::Cell<Option<(usize, usize)>>,
    /// This pass's `(strip, cell)` placement, worked out for the strip (the first overlay child)
    /// and reused for the cell.
    placement: std::cell::Cell<Option<(gtk4::gdk::Rectangle, gtk4::gdk::Rectangle)>>,
}

impl PinnedHeader {
    /// Wrap `sw` (the list's scrolled window around `listview`) in the overlay that pins the
    /// header copy. The overlay is the list's host.
    fn new(
        sw: &gtk4::ScrolledWindow,
        listview: &gtk4::ListView,
        source: Rc<RefCell<Option<ListSource>>>,
        bound: BoundCells,
    ) -> Rc<Self> {
        let overlay = gtk4::Overlay::new();
        overlay.set_child(Some(sw));
        // A copy pushed up past the top, or parked above it while hidden, is clipped here.
        overlay.set_overflow(gtk4::Overflow::Hidden);
        let strip = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        strip.add_css_class("day-list-pinned");
        let cell = DayCell::new();
        cell.set_overflow(gtk4::Overflow::Hidden);
        // Assistive technology reads the real header row; the copy is only paint.
        cell.update_state(&[gtk4::accessible::State::Hidden(true)]);
        strip.update_state(&[gtk4::accessible::State::Hidden(true)]);
        for w in [strip.upcast_ref::<gtk4::Widget>(), cell.upcast_ref()] {
            overlay.add_overlay(w);
            // The copy sits beside the scrolled window, not in it, so the wheel and touchpad
            // over it would scroll nothing: hand their deltas to the list.
            forward_scroll(w, sw);
        }
        let pinned = Rc::new(Self {
            overlay: overlay.clone(),
            listview: listview.clone(),
            strip,
            cell,
            source,
            bound,
            row: std::cell::Cell::new(None),
            wanted: std::cell::Cell::new(None),
            last: std::cell::Cell::new(None),
            placement: std::cell::Cell::new(None),
        });
        // Weak: the overlay owns the handler, and the handler must not own the overlay.
        let weak = Rc::downgrade(&pinned);
        overlay.connect_get_child_position(move |_, child| {
            let pinned = weak.upgrade()?;
            ffi_guard::contain(None, || {
                let place = if child == pinned.strip.upcast_ref::<gtk4::Widget>() {
                    let place = pinned.place();
                    pinned.placement.set(Some(place));
                    place
                } else {
                    pinned.placement.take().unwrap_or_else(|| pinned.place())
                };
                Some(if child == pinned.strip.upcast_ref::<gtk4::Widget>() {
                    place.0
                } else {
                    place.1
                })
            })
        });
        let adj = sw.vadjustment();
        for signal in ["value-changed", "changed"] {
            let overlay = overlay.downgrade();
            adj.connect_local(signal, false, move |_| {
                if let Some(overlay) = overlay.upgrade() {
                    overlay.queue_allocate();
                }
                None
            });
        }
        pinned
    }

    /// Where the strip and the copy go this pass: over the top of the viewport while the topmost
    /// visible row's section header has scrolled (even partly) out of view, pushed up by the next
    /// header, and parked out of sight above the list otherwise. Also reports the first visible
    /// row, which a list of mixed row heights cannot work out from the scroll offset.
    fn place(self: &Rc<Self>) -> (gtk4::gdk::Rectangle, gtk4::gdk::Rectangle) {
        let width = self.overlay.width();
        let parked = |h: i32| {
            let r = gtk4::gdk::Rectangle::new(0, -h - 1, width.max(1), h.max(1));
            (r, r)
        };
        let Some(src) = self.source.borrow().clone() else {
            return parked(1);
        };
        let Some(is_header) = src.is_header.clone() else {
            return parked(1);
        };
        // The rows GTK has laid out, in the overlay's (= the viewport's) coordinates.
        let laid: Vec<(usize, gtk4::graphene::Rect, gtk4::graphene::Rect)> = self
            .bound
            .borrow()
            .values()
            .filter_map(|li| {
                let li = li.upgrade()?;
                let pos = li.position();
                if pos == gtk4::INVALID_LIST_POSITION {
                    return None;
                }
                let cell = li.child()?;
                let row = cell.parent()?;
                if !row.is_visible() || !row.is_child_visible() {
                    return None;
                }
                Some((
                    pos as usize,
                    row.compute_bounds(&self.overlay)?,
                    cell.compute_bounds(&self.overlay)?,
                ))
            })
            .collect();
        // The topmost visible row: the one across the viewport's top edge.
        let Some(&(top_row, top_bounds, top_cell)) = laid
            .iter()
            .filter(|(_, b, _)| b.y() <= 0.5 && b.y() + b.height() > 0.5)
            .min_by_key(|(pos, ..)| *pos)
        else {
            return parked(self.cell.height());
        };
        if let Some(report) = &src.first_visible {
            report(top_row);
        }
        // Its section's header: the nearest header at or above it, scanning only what the last
        // pass did not already settle.
        let header = match self.last.get() {
            Some((t, h)) if h <= top_row && top_row <= t => Some(h),
            Some((t, h)) if t < top_row => {
                (t + 1..=top_row).rev().find(|&r| is_header(r)).or(Some(h))
            }
            _ => (0..=top_row).rev().find(|&r| is_header(r)),
        };
        let Some(header) = header else {
            self.last.set(None);
            return parked(self.cell.height());
        };
        self.last.set(Some((top_row, header)));
        // The real header row, fully in view at the top, needs no copy over it.
        if header == top_row && top_bounds.y() >= -0.5 {
            return parked(self.cell.height());
        }
        // The copy must hold this header first; until it does, nothing is shown.
        if self.row.get() != Some(header) {
            self.rebind_later(header);
            return parked(self.cell.height());
        }
        let cell_w = top_cell.width().round() as i32;
        let (height, ..) = self.cell.measure(gtk4::Orientation::Vertical, cell_w);
        // Pushed up by the next section's header once it reaches the copy's bottom edge.
        let next_top = laid
            .iter()
            .filter(|(pos, ..)| *pos > top_row && is_header(*pos))
            .min_by_key(|(pos, ..)| *pos)
            .map(|(_, b, _)| b.y());
        let y = next_top.map_or(0, |top| (top.round() as i32 - height).min(0));
        let list = self.listview.compute_bounds(&self.overlay);
        let (list_x, list_w) = list.map_or((0, width), |b| {
            (b.x().round() as i32, b.width().round() as i32)
        });
        (
            gtk4::gdk::Rectangle::new(list_x, y, list_w.max(1), height.max(1)),
            gtk4::gdk::Rectangle::new(top_cell.x().round() as i32, y, cell_w.max(1), height.max(1)),
        )
    }

    /// The top of display row `row` in the viewport, when GTK has laid it out. A bound row GTK
    /// keeps out of view is not child-visible and has no allocation to read.
    fn laid_top(&self, row: usize) -> Option<f64> {
        let li = self.bound.borrow().values().find_map(|li| {
            let li = li.upgrade()?;
            (li.position() as usize == row).then_some(li)
        })?;
        let widget = li.child()?.parent()?;
        if !widget.is_child_visible() {
            return None;
        }
        Some(f64::from(widget.compute_bounds(&self.listview)?.y()))
    }

    /// After a jump to `row` by the estimate: once GTK has laid the row out (a frame or two
    /// later), move it to its real place below the pinned copy. Gives up after a few frames.
    fn settle_scroll(self: &Rc<Self>, sw: &gtk4::ScrolledWindow, row: usize) {
        let pinned = Rc::clone(self);
        let frames = std::cell::Cell::new(0u32);
        sw.add_tick_callback(move |sw, _| {
            frames.set(frames.get() + 1);
            let Some(sw) = sw.downcast_ref::<gtk4::ScrolledWindow>() else {
                return gtk4::glib::ControlFlow::Break;
            };
            let Some(y) = pinned.laid_top(row) else {
                return if frames.get() < 8 {
                    gtk4::glib::ControlFlow::Continue
                } else {
                    gtk4::glib::ControlFlow::Break
                };
            };
            let adj = sw.vadjustment();
            let end = (adj.upper() - adj.page_size()).max(0.0);
            adj.set_value((adj.value() + y - pinned.inset_for(row)).clamp(0.0, end));
            gtk4::glib::ControlFlow::Break
        });
    }

    /// How far below the viewport's top a scroll should stop row `row`: clear of the pinned
    /// copy, which covers the top of every section but a header's own row. The copy's height,
    /// or before its first bind any laid header row's.
    fn inset_for(&self, row: usize) -> f64 {
        let Some(is_header) = self
            .source
            .borrow()
            .as_ref()
            .and_then(|src| src.is_header.clone())
        else {
            return 0.0;
        };
        if is_header(row) {
            return 0.0;
        }
        if self.row.get().is_some() && self.cell.height() > 0 {
            return f64::from(self.cell.height());
        }
        self.bound
            .borrow()
            .values()
            .find_map(|li| {
                let li = li.upgrade()?;
                let pos = li.position();
                if pos == gtk4::INVALID_LIST_POSITION || !is_header(pos as usize) {
                    return None;
                }
                let widget = li.child()?.parent()?;
                (widget.is_child_visible() && widget.height() > 0)
                    .then(|| f64::from(widget.height()))
            })
            .unwrap_or(0.0)
    }

    /// Bind header `row` into the copy on the next main-loop turn: `bind_row` builds Day's
    /// subtree, which must not happen inside the allocation pass that found the need for it.
    fn rebind_later(self: &Rc<Self>, row: usize) {
        if self.wanted.replace(Some(row)).is_some() {
            return; // already posted; it binds the newest wanted row
        }
        let weak = Rc::downgrade(self);
        gtk4::glib::idle_add_local_once(move || {
            let Some(pinned) = weak.upgrade() else {
                return;
            };
            // Contained: bind_row runs day-core and the app's header builder.
            ffi_guard::contain((), || {
                let Some(row) = pinned.wanted.take() else {
                    return;
                };
                let Some(src) = pinned.source.borrow().clone() else {
                    return;
                };
                if row >= (src.len)() {
                    return;
                }
                // `bind_pinned`: the copy carries no element ids, so a lookup of the header
                // finds the real row in the list.
                (src.bind_pinned)(row, pinned.cell.as_ptr() as RawHandle);
                pinned
                    .cell
                    .bind_layout(src.layout_cell.clone(), src.cell_height.clone());
                pinned.row.set(Some(row));
                pinned.cell.queue_resize();
                pinned.overlay.queue_allocate();
            });
        });
    }

    /// The rows were reloaded: the header a row index names may have changed, so the copy binds
    /// afresh. Posted behind the reload's own deferred resize (`schedule_list_resize`), so it
    /// places against the new rows.
    fn reset_later(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        gtk4::glib::idle_add_local_once(move || {
            if let Some(pinned) = weak.upgrade() {
                pinned.row.set(None);
                pinned.last.set(None);
                pinned.overlay.queue_allocate();
            }
        });
    }
}

/// Scroll `sw` by the wheel or touchpad deltas that land on `w`, a widget floating over it.
/// GTK's own wheel step is the page size to the 2/3 power; a touchpad reports pixels.
fn forward_scroll(w: &gtk4::Widget, sw: &gtk4::ScrolledWindow) {
    let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
    let adj = sw.vadjustment();
    scroll.connect_scroll(move |ctrl, _dx, dy| {
        let step = if ctrl.unit() == gtk4::gdk::ScrollUnit::Wheel {
            adj.page_size().powf(2.0 / 3.0)
        } else {
            1.0
        };
        adj.set_value(adj.value() + dy * step);
        gtk4::glib::Propagation::Stop
    });
    w.add_controller(scroll);
}

/// A list host's scrolled window: the host itself, or a sectioned list's, under the overlay that
/// pins its header (`PinnedHeader`).
fn list_scroller(h: &Handle) -> Option<gtk4::ScrolledWindow> {
    if let Some(sw) = h.downcast_ref::<gtk4::ScrolledWindow>() {
        return Some(sw.clone());
    }
    h.downcast_ref::<gtk4::Overlay>()?
        .child()?
        .downcast::<gtk4::ScrolledWindow>()
        .ok()
}

/// Mark row `pos`'s list item as a section header or an ordinary row (docs/list.md "Sections"):
/// a header is not selectable, activatable or focusable, so it takes no click, no Enter and no
/// selection, and keyboard navigation steps over it. Adwaita draws hover and press feedback only
/// on an `.activatable` row, which GTK takes off an unactivatable item itself, so the header row
/// needs no style of its own (a CSS class added to the row widget from `bind` crashed GTK 4.20's
/// item manager in a later splice). `focusable` is GTK 4.12's (the binding is pinned at 4.10), so
/// it is set by name where the running library has it.
fn mark_list_header(li: &gtk4::ListItem, header: bool) {
    li.set_selectable(!header);
    li.set_activatable(!header);
    if li.find_property("focusable").is_some() {
        li.set_property("focusable", !header);
    }
}

/// A list's bound cells: physical cell (DayCell ptr) → the GtkListItem it fills. Written by the
/// factory's bind, cleared by its unbind; weak, so a disposed list item is simply skipped.
type BoundCells = Rc<RefCell<HashMap<usize, gtk4::glib::WeakRef<gtk4::ListItem>>>>;

#[derive(Clone)]
struct ListSelection {
    model: gtk4::SingleSelection,
    requested: Rc<std::cell::Cell<u32>>,
    suppress: Rc<std::cell::Cell<bool>>,
}
impl ListSelection {
    fn apply(&self) {
        let row = self.requested.get();
        self.model.set_selected(if row < self.model.n_items() {
            row
        } else {
            gtk4::INVALID_LIST_POSITION
        });
    }
}

/// `DayCell` — the widget a recycling list's cell is made of: a container that takes part in GTK's
/// own size negotiation instead of dictating to it, the way a custom container in a C GTK app does.
///
/// Day owns layout, so everywhere else a Day subtree sits in a `GtkFixed` and states its size with
/// `set_size_request` (see the module doc). That makes Day's computed width a FLOOR on the widget
/// around it — wrong inside a `GtkListView`, which wraps each cell in a themed `row` node that pads
/// it: a row laid at the list's width inflates the cell, the cell reports that width back as its
/// minimum, and the whole list is dragged wider than the frame Day gave it, painting over whatever
/// sits alongside (day#35).
///
/// GTK's answer — and the one AppKit (`DayTreeCell`), UIKit and Qt already use — runs the other
/// way: ask for nothing, then lay out inside whatever you are given.
///
/// * `measure` reports a zero minimum width, so the cell can never inflate its row; its height is
///   whatever Day last laid the row at (`RowHeight`).
/// * `size_allocate` hands the width GTK actually granted to Day through
///   [`ListSource::layout_cell`] (the seam trees use for indentation) before placing the children,
///   then puts each child at the frame Day recorded for it — so a row laid at the list's full
///   width never reaches the screen.
///
/// It derives from `GtkWidget` directly, not `GtkFixed`: GTK never calls the `measure` and
/// `size_allocate` of a widget that has a layout manager — it asks the manager instead — and
/// `GtkFixed` installs one. So this cell owns its children outright (`set_parent` / `unparent`,
/// released in `dispose`), and `set_frame` RECORDS a child's frame here instead of calling
/// `set_size_request`, which would put the floor straight back.
mod day_cell {
    use super::{RawHandle, Rect, RefCell, ffi_guard};
    use gtk4::glib;
    use gtk4::prelude::*;
    use gtk4::subclass::prelude::*;
    use std::collections::HashMap;

    /// A cell's re-layout seam, `(cell handle, width)`: `ListSource::layout_cell` or
    /// `TreeSource::layout_cell`.
    type LayoutCell = std::rc::Rc<dyn Fn(RawHandle, f64)>;
    /// A cell's laid-height query, `ListSource::cell_height` or `TreeSource::cell_height`.
    type CellHeight = std::rc::Rc<dyn Fn(RawHandle) -> f64>;
    /// A filling cell's allocation observer, `(width, height)`.
    type AllocateHook = std::rc::Rc<dyn Fn(i32, i32)>;

    #[derive(Default)]
    pub struct DayCellImp {
        /// The row's seam, planted at bind time; `None` until the first bind, and always for a
        /// filling cell.
        layout: RefCell<Option<LayoutCell>>,
        /// The row's laid height, asked of day-core (`ListSource::cell_height`): the row's own
        /// height, its bottom padding included, where the children's frames alone end at the
        /// last child.
        height: RefCell<Option<CellHeight>>,
        /// The width Day last laid this cell's row at, so an unchanged allocation costs nothing.
        laid: std::cell::Cell<i32>,
        /// Each child's frame as Day last laid it, keyed by the child's pointer.
        frames: RefCell<HashMap<usize, Rect>>,
        /// Set while `size_allocate` runs: frames Day records then are placed by that same pass,
        /// so they must not queue another.
        allocating: std::cell::Cell<bool>,
        /// Filling: the cell asks for nothing in either direction and every child gets the whole
        /// allocation. That is what the `External`-policy ScrolledWindows did around a nav pane's
        /// or the window's content — break minimum-size propagation — minus a scroll window that
        /// must never scroll.
        fill: std::cell::Cell<bool>,
        /// A filling cell's allocation observer, `(width, height)`: GTK4 has no size-allocate
        /// signal, and the nav and inspector panes need one. Runs before the children are
        /// allocated, so a Day relayout it triggers — a pane reporting `FrameChanged`, which
        /// day-core dispatches at once when its tree is free — lands in this same pass, and
        /// the page follows a divider drag live, as an AppKit `setFrameSize:` report does.
        on_allocate: RefCell<Option<AllocateHook>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DayCellImp {
        const NAME: &'static str = "DayCell";
        type Type = super::DayCell;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for DayCellImp {
        fn dispose(&self) {
            // A custom container releases its own children.
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for DayCellImp {
        fn measure(&self, orientation: gtk4::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            if self.fill.get() || orientation == gtk4::Orientation::Horizontal {
                // Ask for nothing: a row gives the cell whatever the list's width leaves after
                // its own padding, a pane gives it the pane, and Day lays the content to fit
                // that in `size_allocate`.
                return (0, 0, -1, -1);
            }
            // The row is as tall as Day laid it — `RowHeight`, or a measured automatic row —
            // padding included. Before the first layout (or inside a day-core borrow) the
            // children's frames stand in.
            let laid = self
                .height
                .borrow()
                .as_ref()
                .map(|h| h(self.obj().as_ptr() as RawHandle))
                .unwrap_or(0.0);
            let height = if laid > 0.0 {
                laid.ceil() as i32
            } else {
                self.frames
                    .borrow()
                    .values()
                    .map(|f| (f.origin.y + f.size.height).ceil() as i32)
                    .max()
                    .unwrap_or(0)
                    .max(0)
            };
            (height, height, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let obj = self.obj();
            if self.fill.get() {
                // The whole allocation to every child — at least its minimum, as GTK insists;
                // anything past the edge is clipped, exactly as the scroll window it replaces
                // clipped it. The observer goes first (its field explains), cloned out so it
                // may install another.
                let hook = self.on_allocate.borrow().clone();
                if let Some(hook) = hook {
                    hook(width, height);
                }
                let mut child = obj.first_child();
                while let Some(c) = child {
                    child = c.next_sibling();
                    let (min_w, ..) = c.measure(gtk4::Orientation::Horizontal, -1);
                    let (min_h, ..) = c.measure(gtk4::Orientation::Vertical, -1);
                    c.allocate(width.max(min_w), height.max(min_h), -1, None);
                }
                return;
            }
            // Lay the row at the width the row actually granted, before any child is placed.
            // `layout_cell` re-enters day-core, whose `set_frame` calls land in `set_child_frame`
            // below; it skips quietly if a day-core borrow is already held.
            if width > 0
                && self.laid.get() != width
                && let Some(layout) = self.layout.borrow().clone()
            {
                self.laid.set(width);
                self.allocating.set(true);
                let cell = obj.as_ptr() as RawHandle;
                ffi_guard::contain((), || layout(cell, f64::from(width)));
                self.allocating.set(false);
            }
            let frames = self.frames.borrow().clone();
            let mut child = obj.first_child();
            while let Some(c) = child {
                child = c.next_sibling();
                let (x, y, w, h) = frames
                    .get(&(c.as_ptr() as usize))
                    .map(|f| (f.origin.x, f.origin.y, f.size.width, f.size.height))
                    .unwrap_or_default();
                // GTK insists a child gets at least its minimum. Day's frames come from its own
                // measure of the child, so this only matters for a child not laid out yet.
                let (min_w, ..) = c.measure(gtk4::Orientation::Horizontal, -1);
                let (min_h, ..) = c.measure(gtk4::Orientation::Vertical, -1);
                let transform = gtk4::gsk::Transform::new()
                    .translate(&gtk4::graphene::Point::new(x as f32, y as f32));
                c.allocate(
                    (w.round() as i32).max(min_w),
                    (h.round() as i32).max(min_h),
                    -1,
                    Some(transform),
                );
            }
        }
    }

    impl DayCellImp {
        /// Plant the row's seam, then lay the row that was just bound at the width this cell
        /// already has. `bind_row` laid it at the HOST's width a moment ago — wider than the cell
        /// by the row's padding, or its indentation — and correcting it now keeps that width off
        /// the screen. A cell GTK has not allocated yet has no width to offer; its first
        /// `size_allocate` does it.
        fn bind(&self, layout: LayoutCell, height: CellHeight) {
            *self.layout.borrow_mut() = Some(layout.clone());
            *self.height.borrow_mut() = Some(height);
            let width = self.obj().width();
            if width <= 0 {
                self.laid.set(0);
                return;
            }
            self.laid.set(width);
            let cell = self.obj().as_ptr() as RawHandle;
            ffi_guard::contain((), || layout(cell, f64::from(width)));
        }
    }

    impl super::DayCell {
        /// A filling cell: the minimum-size breaker around a nav pane's or the window's content
        /// (the `fill` field explains). Clipped, as the scroll window it replaces was.
        pub(crate) fn filling() -> Self {
            let cell: Self = glib::Object::new();
            cell.imp().fill.set(true);
            cell.set_overflow(gtk4::Overflow::Hidden);
            cell
        }

        /// Observe a filling cell's allocations (the `on_allocate` field explains). One
        /// observer; a second call replaces the first.
        pub(crate) fn on_allocate(&self, hook: impl Fn(i32, i32) + 'static) {
            *self.imp().on_allocate.borrow_mut() = Some(std::rc::Rc::new(hook));
        }

        /// Called from a factory's `bind` handler once day-core has built or rebound the row,
        /// with `ListSource::layout_cell` or `TreeSource::layout_cell`.
        pub(crate) fn bind_layout(&self, layout: LayoutCell, height: CellHeight) {
            self.imp().bind(layout, height);
        }

        /// Adopt `child`: this cell places it at the frame `set_child_frame` records for it.
        pub(crate) fn add_child(&self, child: &gtk4::Widget) {
            child.set_parent(self);
        }

        pub(crate) fn remove_child(&self, child: &gtk4::Widget) {
            self.imp()
                .frames
                .borrow_mut()
                .remove(&(child.as_ptr() as usize));
            if child.parent().as_ref() == Some(self.upcast_ref()) {
                super::resign_focus_before_detach(child);
                child.unparent();
            }
        }

        /// Record where Day laid `child`. Placed by the next allocation — or by the one running,
        /// when the record comes from the row layout that `size_allocate` itself asked for.
        pub(crate) fn set_child_frame(&self, child: &gtk4::Widget, frame: Rect) {
            let imp = self.imp();
            imp.frames
                .borrow_mut()
                .insert(child.as_ptr() as usize, frame);
            if !imp.allocating.get() {
                // The row's height is derived from these frames, so a resize, not just a
                // re-allocation.
                self.queue_resize();
            }
        }
    }
}

gtk4::glib::wrapper! {
    pub struct DayCell(ObjectSubclass<day_cell::DayCellImp>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl DayCell {
    fn new() -> Self {
        gtk4::glib::Object::new()
    }
}

/// Day's row height is the row's height. GtkListView's themed `row` node pads each row
/// vertically (2px under Adwaita), which was added on top of every `RowHeight` — a 56pt row drew
/// 60 — and grew an automatic row past what Day measured for it. The horizontal padding stays
/// (the cell fits inside it; see `DayCell`); vertically, Day's policy is the one that counts.
/// One global provider serves every Day list and tree.
fn day_list_rows(listview: &gtk4::ListView) {
    thread_local! {
        static ROW_CSS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    ROW_CSS.with(|done| {
        if !done.get()
            && let Some(display) = gtk4::gdk::Display::default()
        {
            let provider = gtk4::CssProvider::new();
            provider.load_from_data(concat!(
                "listview.day-list > row { padding-top: 0; padding-bottom: 0; }",
                // A sectioned list's pinned header strip (docs/list.md "Sections"): the list's own
                // background, so rows scrolling under it stay hidden, and a hairline below, as a
                // floating header draws.
                ".day-list-pinned { background-color: @view_bg_color; \
                 box-shadow: 0 1px alpha(currentColor, 0.12); }",
            ));
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            done.set(true);
        }
    });
    listview.add_css_class("day-list");
}

/// A navigation suite's switcher rows: `(titles, icon names)`, index-aligned with the
/// destination pages.
type SuiteRows = (Vec<String>, Vec<Option<String>>);

/// A realized nav menu's rows: `(node, titles, icon names)`.
type NavRow = (NodeId, Vec<String>, Vec<Option<String>>);

// ---------------------------------------------------------------------------
// Native hierarchical tree (docs/tree.md): GtkListView + GtkTreeListModel + GtkTreeExpander.
// Rows are keyed by TOKEN (a StringObject holding its decimal form); children come lazily
// from the injected `TreeSource` through the tree model's create func. A Reload REBUILDS the
// whole model (deferred to an idle — GTK binds synchronously, and reloads arrive inside a
// `with_tree` borrow, the same rule `schedule_list_resize` documents) and then re-applies
// the recorded expansion and selection top-down, so both survive by token.
// ---------------------------------------------------------------------------

struct TreeEntry {
    node: NodeId,
    listview: gtk4::ListView,
    source: Rc<RefCell<Option<TreeSource>>>,
    /// Disclosure by token, as last patched or natively toggled — what a rebuild restores.
    expanded: Rc<RefCell<std::collections::HashSet<u64>>>,
    /// Selection by token, as last patched — what a rebuild restores.
    selected: Rc<RefCell<Vec<u64>>>,
    /// Programmatic changes in flight: don't echo them back as events.
    suppress: Rc<std::cell::Cell<bool>>,
    multi: bool,
}

thread_local! {
    /// The `notify::expanded` handler each bound row holds while bound, keyed by ListItem ptr.
    static TREE_ROW_HANDLERS: RefCell<HashMap<usize, (gtk4::TreeListRow, gtk4::glib::SignalHandlerId)>> =
        RefCell::new(HashMap::new());
}

fn tree_row_token(row: &gtk4::TreeListRow) -> Option<u64> {
    row.item()?
        .downcast_ref::<gtk4::StringObject>()?
        .string()
        .parse()
        .ok()
}

/// The children of `parent` as a StringList of tokens, from the source's snapshot.
fn tree_children_model(src: &TreeSource, parent: Option<u64>) -> gtk4::StringList {
    let list = gtk4::StringList::new(&[]);
    let n = (src.children_len)(parent);
    for i in 0..n {
        list.append(&(src.child_token)(parent, i).to_string());
    }
    list
}

/// Rebuild the whole model from the source's snapshot, then restore disclosure and
/// selection — always deferred to an idle (see the module comment).
fn schedule_tree_rebuild(entry: Rc<TreeEntry>) {
    gtk4::glib::idle_add_local_once(move || {
        ffi_guard::contain((), || {
            let root = {
                let src = entry.source.borrow();
                let Some(src) = src.as_ref() else { return };
                tree_children_model(src, None)
            };
            let create = {
                let source = entry.source.clone();
                move |obj: &gtk4::glib::Object| -> Option<gtk4::gio::ListModel> {
                    ffi_guard::contain(None, || {
                        let tok: u64 = obj
                            .downcast_ref::<gtk4::StringObject>()?
                            .string()
                            .parse()
                            .ok()?;
                        let src = source.borrow();
                        let src = src.as_ref()?;
                        // Expandability is the app's branch rule: a childless BRANCH still
                        // discloses (to an empty level); a leaf returns no model at all,
                        // which is what hides the expander arrow.
                        if !(src.expandable)(tok) {
                            return None;
                        }
                        Some(tree_children_model(src, Some(tok)).upcast())
                    })
                }
            };
            let tlm = gtk4::TreeListModel::new(root, false, false, create);
            let model: gtk4::gio::ListModel = tlm.upcast();
            let sel_model: gtk4::SelectionModel = if entry.multi {
                let m = gtk4::MultiSelection::new(Some(model));
                m.upcast()
            } else {
                let m = gtk4::SingleSelection::new(Some(model));
                m.set_autoselect(false);
                m.set_can_unselect(true);
                m.upcast()
            };
            {
                // Report the FULL selected token set (docs/tree.md). Captures only what it
                // needs — an Rc cycle through the entry would leak a model per rebuild.
                let (node, suppress) = (entry.node, entry.suppress.clone());
                sel_model.connect_selection_changed(move |m, _, _| {
                    ffi_guard::contain((), || {
                        if suppress.get() {
                            return;
                        }
                        emit(node, Event::TreeSelection(tree_selected_tokens(m)));
                    });
                });
            }
            entry.listview.set_model(Some(&sel_model));
            tree_apply_expansion(&entry);
            tree_apply_selection(&entry);
        });
    });
}

/// Every currently selected row's token, in visible order.
fn tree_selected_tokens(m: &gtk4::SelectionModel) -> Vec<u64> {
    let mut out = Vec::new();
    let bits = m.selection();
    if !bits.is_empty() {
        for i in bits.minimum()..=bits.maximum() {
            if m.is_selected(i)
                && let Some(row) = m
                    .item(i)
                    .and_then(|o| o.downcast::<gtk4::TreeListRow>().ok())
                && let Some(tok) = tree_row_token(&row)
            {
                out.push(tok);
            }
        }
    }
    out
}

/// Walk the VISIBLE rows top-down and set each one's disclosure to the recorded state —
/// expanding at `i` inserts children right after it, which the loop then visits, so a
/// recorded deep disclosure re-opens ancestors-first in one pass. Suppressed: a restore
/// must not echo as `Event::TreeExpanded`.
fn tree_apply_expansion(entry: &TreeEntry) {
    let Some(model) = entry.listview.model() else {
        return;
    };
    entry.suppress.set(true);
    let mut i = 0;
    while i < model.n_items() {
        if let Some(row) = model
            .item(i)
            .and_then(|o| o.downcast::<gtk4::TreeListRow>().ok())
            && let Some(tok) = tree_row_token(&row)
        {
            let want = entry.expanded.borrow().contains(&tok);
            if row.is_expandable() && row.is_expanded() != want {
                row.set_expanded(want);
            }
        }
        i += 1;
    }
    entry.suppress.set(false);
}

/// Sync the native selection to the recorded tokens (suppressed — no event echo). A token
/// under a collapsed ancestor has no row; the piece re-applies after expansion changes.
fn tree_apply_selection(entry: &TreeEntry) {
    let Some(model) = entry.listview.model() else {
        return;
    };
    entry.suppress.set(true);
    model.unselect_all();
    for i in 0..model.n_items() {
        if let Some(row) = model
            .item(i)
            .and_then(|o| o.downcast::<gtk4::TreeListRow>().ok())
            && let Some(tok) = tree_row_token(&row)
            && entry.selected.borrow().contains(&tok)
        {
            model.select_item(i, false);
        }
    }
    entry.suppress.set(false);
}

fn tree_entry(key: usize) -> Option<Rc<TreeEntry>> {
    TREE_STATE.with(|m| m.borrow().get(&key).cloned())
}

/// Show `items` as a one-summon popover anchored at `(x, y)` on `w` — built fresh per
/// summon (docs/menus.md "Dynamic context menus") and unparented once it closes.
fn show_menu_popover(w: &Handle, items: &[day_spec::MenuItem], x: f64, y: f64) {
    let group = gtk4::gio::SimpleActionGroup::new();
    let model = build_gio_menu(items, &group);
    w.insert_action_group("daymenu", Some(&group));
    let popover = gtk4::PopoverMenu::from_model(Some(&model));
    popover.set_parent(w);
    popover.set_has_arrow(false);
    popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.connect_closed(|pop| {
        // Unparent outside the close handler — GTK is still walking the popover's state.
        let pop = pop.clone();
        gtk4::glib::idle_add_local_once(move || pop.unparent());
    });
    popover.popup();
}

/// Bring the backing model to `n` rows (content is irrelevant — bind_row provides it), then
/// re-bind the rows GTK already shows, in place.
///
/// Only the count difference is spliced. Replacing every item would make GtkListView destroy
/// each row widget and set up a new one (it keeps a widget only for an item that stays in the
/// model), so a same-size reload would build every bound row a second time while day-core
/// still holds the first set, with its effects and tasks still running. A row that keeps its
/// position re-binds instead, the way AppKit's reloadData refreshes its visible cells.
fn list_resize(
    model: &gtk4::StringList,
    n: usize,
    source: &Rc<RefCell<Option<ListSource>>>,
    bound: &BoundCells,
) {
    let cur = model.n_items() as usize;
    // Snapshot first: an append binds its new rows synchronously, and they need no second bind.
    // Top to bottom, so the rows a reader sees first are refreshed (and start any work) first.
    let mut kept: Vec<gtk4::ListItem> = bound
        .borrow()
        .values()
        .filter_map(|li| li.upgrade())
        .collect();
    kept.sort_by_key(|li| li.position());
    let (cur32, n32) = (cur as u32, n as u32);
    if n > cur {
        let blanks: Vec<&str> = vec![""; n - cur];
        model.splice(cur32, 0, &blanks);
    } else if n < cur {
        model.splice(n32, cur32 - n32, &[]);
    }
    let Some(src) = source.borrow().clone() else {
        return;
    };
    for li in kept {
        let pos = li.position();
        // A row past the new end was unbound by the splice above.
        if pos == gtk4::INVALID_LIST_POSITION || pos >= n32 {
            continue;
        }
        let Some(cell) = li.child() else { continue };
        LIST_CELL_ROWS.with(|t| t.insert(cell.as_ptr() as usize, pos as usize));
        if let Some(is_header) = &src.is_header {
            mark_list_header(&li, is_header(pos as usize));
        }
        (src.bind_row)(pos as usize, cell.as_ptr() as RawHandle);
        if let Ok(cell) = cell.downcast::<DayCell>() {
            cell.bind_layout(src.layout_cell.clone(), src.cell_height.clone());
            // New content may need a new height (an Automatic row); GTK measured the old one.
            cell.queue_resize();
        }
    }
}

/// Resize the model on the next main-loop turn. CRUCIAL: `splice` makes GtkListView bind visible
/// cells SYNCHRONOUSLY (unlike NSTableView's deferred reloadData), and a reload is driven from
/// inside a `with_tree` borrow — so resizing inline would re-enter `with_tree` (bind_row) and
/// panic. Deferring to an idle runs the bind after the borrow is released.
fn schedule_list_resize(
    model: gtk4::StringList,
    source: Rc<RefCell<Option<ListSource>>>,
    selection: Option<ListSelection>,
    bound: BoundCells,
) {
    gtk4::glib::idle_add_local_once(move || {
        // Contained: `len` is app code and the splice re-binds through day-core (bind_row).
        ffi_guard::contain((), || {
            let n = source.borrow().as_ref().map(|s| (s.len)()).unwrap_or(0);
            if let Some(selection) = &selection {
                selection.suppress.set(true);
            }
            list_resize(&model, n, &source, &bound);
            if let Some(selection) = &selection {
                selection.apply();
                selection.suppress.set(false);
            }
        });
    });
}

/// The directory of symbolic icons staged for the icon theme (`vector_icon_name`), created
/// and put on the theme's search path once per process; `None` where there is no cache
/// directory to write to.
fn symbolic_icon_dir() -> Option<std::path::PathBuf> {
    thread_local! {
        static DIR: RefCell<Option<Option<std::path::PathBuf>>> = const { RefCell::new(None) };
    }
    DIR.with(|d| {
        if let Some(known) = d.borrow().as_ref() {
            return known.clone();
        }
        let made = (|| {
            // XDG on every platform; one directory per executable, so two apps' vectors of
            // one name never meet.
            let root = std::env::var_os("XDG_CACHE_HOME")
                .map(std::path::PathBuf::from)
                .filter(|p| p.is_absolute())
                .or_else(|| {
                    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache"))
                })?;
            let key = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                std::env::current_exe().ok()?.hash(&mut h);
                h.finish()
            };
            let dir = root
                .join("day")
                .join("symbolic")
                .join(format!("{key:016x}"));
            std::fs::create_dir_all(&dir).ok()?;
            let display = gtk4::gdk::Display::default()?;
            gtk4::IconTheme::for_display(&display).add_search_path(&dir);
            Some(dir)
        })();
        *d.borrow_mut() = Some(made.clone());
        made
    })
}

/// A bundled vector as an icon-theme name, for widgets that take only `icon-name`
/// (`AdwViewSwitcher`): `<name>-symbolic`, so GTK recolors it to the widget's foreground at
/// render time — the tab's glyph then matches its label in either theme, the way every
/// symbolic icon in a GNOME app does. Staged as a symbolic PNG (`<name>-symbolic.symbolic.png`,
/// the shape in the alpha channel over black), which GTK recolors with a color matrix: a
/// symbolic SVG would go through GTK's own `GtkSvg` parser instead, which draws only a subset
/// of SVG and turned a Material glyph's quadratic curves into the broken-image box. Rendered
/// through the same librsvg path the sidebar rows use, at 4× the display size for HiDPI, and
/// refreshed when the vector is newer. `None` for a name no vector answers to, so the caller
/// falls back to the blank glyph rather than the broken-image box.
pub fn vector_icon_name(name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    let svg = day_spec::resource::resolve_vector_svg(name)?;
    let dir = symbolic_icon_dir()?;
    let png = dir.join(format!("{name}-symbolic.symbolic.png"));
    let stale = match (std::fs::metadata(&svg), std::fs::metadata(&png)) {
        (Ok(src), Ok(dst)) => match (src.modified(), dst.modified()) {
            (Ok(s), Ok(d)) => s > d,
            _ => true,
        },
        _ => true,
    };
    if stale {
        let px = ICON_PX * 4;
        let pixbuf = vector::symbolic_pixbuf(&std::fs::read(&svg).ok()?, px)?;
        let pixbuf = if pixbuf.has_alpha() {
            pixbuf
        } else {
            pixbuf.add_alpha(false, 0, 0, 0).ok()?
        };
        // Black under the alpha: the color matrix reads red, green, and blue as the
        // success, warning, and error tints, so any color in the source would bleed those.
        recolor_pixbuf(&pixbuf, 0, 0, 0);
        pixbuf.savev(&png, "png", &[]).ok()?;
        icon_theme_rescan();
    }
    let symbolic = format!("{name}-symbolic");
    let display = gtk4::gdk::Display::default()?;
    gtk4::IconTheme::for_display(&display)
        .has_icon(&symbolic)
        .then_some(symbolic)
}

/// Make the icon theme see a file just written into a directory it has already listed: GTK
/// scans a search-path directory once and keeps the listing, so a symbolic icon staged after
/// the first lookup is invisible until the theme is told its paths changed. Re-setting the
/// search path to itself is that signal; it costs a rescan, paid only when a vector is staged
/// for the first time on this machine.
fn icon_theme_rescan() {
    if let Some(display) = gtk4::gdk::Display::default() {
        let theme = gtk4::IconTheme::for_display(&display);
        let paths = theme.search_path();
        let paths: Vec<&std::path::Path> = paths.iter().map(std::path::PathBuf::as_path).collect();
        theme.set_search_path(&paths);
    }
}

/// A themed icon name for a [`Symbol`]: the running theme's own glyph where it has one, else
/// Day's outline staged as a symbolic icon (`day-<symbol>-symbolic`) the way
/// [`vector_icon_name`] stages a bundled vector, so a widget that takes only an icon name (an
/// `AdwViewStackPage`) draws the same glyph a toolbar button would. `None` where nothing can be
/// staged (no cache directory, no display).
pub fn symbol_icon_name(sym: day_spec::Symbol) -> Option<String> {
    if let Some(name) = toolbar::icon_name_for(sym) {
        return Some(name.to_string());
    }
    let svg = sym.outline_svg()?;
    let dir = symbolic_icon_dir()?;
    let name = format!("day-{}-symbolic", format!("{sym:?}").to_lowercase());
    let png = dir.join(format!("{name}.symbolic.png"));
    if !png.exists() {
        let px = ICON_PX * 4;
        let stream =
            gtk4::gio::MemoryInputStream::from_bytes(&gtk4::glib::Bytes::from(svg.as_bytes()));
        let pixbuf = gtk4::gdk_pixbuf::Pixbuf::from_stream_at_scale(
            &stream,
            px,
            px,
            true,
            gtk4::gio::Cancellable::NONE,
        )
        .ok()?;
        let pixbuf = if pixbuf.has_alpha() {
            pixbuf
        } else {
            pixbuf.add_alpha(false, 0, 0, 0).ok()?
        };
        // Black under the alpha, for the same reason `vector_icon_name` recolors.
        recolor_pixbuf(&pixbuf, 0, 0, 0);
        pixbuf.savev(&png, "png", &[]).ok()?;
        icon_theme_rescan();
    }
    let display = gtk4::gdk::Display::default()?;
    gtk4::IconTheme::for_display(&display)
        .has_icon(&name)
        .then_some(name)
}

/// A fully transparent symbolic icon, for a switcher row with no glyph: libadwaita's switcher
/// button shows `image-missing` for a page with no icon name, and a blank is what "no icon"
/// should look like.
fn blank_icon_name() -> Option<String> {
    let dir = symbolic_icon_dir()?;
    let png = dir.join("day-blank-symbolic.symbolic.png");
    if !png.exists() {
        let pixbuf = gtk4::gdk_pixbuf::Pixbuf::new(
            gtk4::gdk_pixbuf::Colorspace::Rgb,
            true,
            8,
            ICON_PX,
            ICON_PX,
        )?;
        pixbuf.fill(0);
        pixbuf.savev(&png, "png", &[]).ok()?;
        icon_theme_rescan();
    }
    let display = gtk4::gdk::Display::default()?;
    gtk4::IconTheme::for_display(&display)
        .has_icon("day-blank-symbolic")
        .then(|| "day-blank-symbolic".to_string())
}

/// Apply the suite's rows to its pages: the switcher shows each page's title and icon, and
/// destination `i` is the page named `p{i + 1}` (`p0` is the hidden rows page).
fn suite_apply_rows(stack: &adw::ViewStack, titles: &[String], icons: &[Option<String>]) {
    for (i, title) in titles.iter().enumerate() {
        if let Some(child) = stack.child_by_name(&format!("p{}", i + 1)) {
            let page = stack.page(&child);
            page.set_title(Some(title));
            let icon = icons
                .get(i)
                .and_then(|o| o.as_deref())
                .and_then(vector_icon_name)
                .or_else(blank_icon_name);
            page.set_icon_name(icon.as_deref());
        }
    }
}

/// A nav menu's rows reached a navigation suite: walking up from `from`, the first Suite host
/// takes them as its switcher's rows (the menu lives inside that host's rows page). Run when
/// the menu is inserted and whenever its rows are rebuilt.
fn suite_rows_changed(
    from: &Handle,
    node: NodeId,
    titles: &[String],
    icons: &[Option<String>],
    keys: &[String],
) {
    let mut up = Some(from.clone());
    while let Some(w) = up {
        let taken = NAV_STATE.with(|m| {
            let m = m.borrow();
            if let Some(NavState {
                present: NavPresent::Documents(doc),
                ..
            }) = m.get(&widget_key(&w))
            {
                doc.rows(node, keys, titles);
                return true;
            }
            let Some(NavState {
                present:
                    NavPresent::Suite {
                        stack,
                        rows,
                        menu_node,
                    },
                ..
            }) = m.get(&widget_key(&w))
            else {
                return false;
            };
            menu_node.set(node.0);
            *rows.borrow_mut() = (titles.to_vec(), icons.to_vec());
            suite_apply_rows(stack, titles, icons);
            true
        });
        if taken {
            return;
        }
        up = w.parent();
    }
}

/// Renderers registered by external Day Piece crates (§8.2).
#[distributed_slice]
pub static RENDERERS: [fn() -> Renderer<Gtk>];

/// Whether `widget`'s window holds a native surface a piece hosts above GTK's drawing (on
/// macOS, a Cocoa view in the window's content view: day-piece-webview's `WKWebView`). Such
/// a view draws over everything GTK draws in that window, an `AdwDialog` included, so a
/// dialog presented within the window would sit under it unseen; the callers present the
/// dialog as a window instead. Elsewhere a web view draws inside GTK's scene, so nothing
/// covers a dialog.
fn has_native_surface(widget: &gtk4::Widget) -> bool {
    #[cfg(target_os = "macos")]
    {
        snapshot_macos::has_native_surface(widget)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = widget;
        false
    }
}

/// Where to present an alert dialog for `parent`: within its window, or, while a native
/// surface would cover it there (`has_native_surface`), as a window of its own (`None`). An
/// alert belongs to the window it interrupts, so only the cover lifts it out.
fn dialog_parent(parent: &gtk4::Widget) -> Option<&gtk4::Widget> {
    if has_native_surface(parent) {
        log::info!("day-gtk: presenting the dialog as a window: its window hosts a native surface");
        None
    } else {
        Some(parent)
    }
}

/// Where to present a panel (Preferences, About) for `parent`. On Linux, within its window:
/// GNOME's preferences dialog, a sheet in a wide window and a bottom sheet in a narrow one.
/// On macOS always as a window of its own (`None`): that is where a Mac keeps Preferences
/// (AppKit's convention), and it never depends on what the window holds — a Cocoa view
/// (`has_native_surface`) would cover an in-window sheet, whether it is there when the
/// panel opens or arrives after. The dialog is the same `AdwPreferencesDialog` either way.
fn panel_parent(parent: &gtk4::Widget) -> Option<&gtk4::Widget> {
    if cfg!(target_os = "macos") {
        None
    } else {
        dialog_parent(parent)
    }
}

/// Remember a window that just became active (`LAST_ACTIVE_WINDOW`).
fn note_last_active(w: &gtk4::Window) {
    if w.is_active() {
        LAST_ACTIVE_WINDOW.with(|l| *l.borrow_mut() = Some(w.downgrade()));
    }
}

/// A live secondary window (docs/windows.md).
struct GtkWin {
    window: adw::ApplicationWindow,
    fixed: gtk4::Fixed,
}

/// A panel (`WindowKind::Preferences`, `WindowKind::About`) presented as an `AdwDialog` over
/// the app window rather than as a window of its own (docs/windows.md): a preferences or About
/// surface in a GNOME app is a dialog, floating in a wide window and a bottom sheet in a narrow
/// one. The record keeps a window's content-container contract: Day mounts into `fixed`, and
/// every window duty day-core calls with that handle lands on the dialog instead.
///
/// The dialog itself is made lazily, because what it should be depends on the content. The
/// settings shell (day-piece-settings) claims an `AdwPreferencesDialog` through
/// [`preferences_dialog_of`] while it realizes and adds its pages to it; any other content (the
/// composed About, a settings sub-page opened on its own) gets a plain `AdwDialog` with a header
/// bar over Day's tree the moment its first child mounts. Presenting waits for the next main-loop
/// turn, so the dialog appears built and at its fitted size.
struct DialogHost {
    /// The window's root node, where the dialog's events go.
    node: NodeId,
    kind: day_spec::WindowKind,
    /// Day's content container.
    fixed: gtk4::Fixed,
    /// The window the dialog presents over.
    parent: gtk4::Window,
    dialog: RefCell<Option<adw::Dialog>>,
    title: RefCell<String>,
    /// The content size to show: the options' size until `fit_window` says otherwise. The
    /// dialog's content height counts its header bar, which is added when applied.
    size: std::cell::Cell<Size>,
    presented: std::cell::Cell<bool>,
    /// When it was presented: a capture waits out the sheet's entrance animation.
    presented_at: std::cell::Cell<Option<std::time::Instant>>,
    /// Closed (by day-core, or by the user) — nothing further is presented or applied.
    closed: std::cell::Cell<bool>,
}

/// How long libadwaita animates a dialog in; a capture before that shows it mid-fade.
const DIALOG_ENTRANCE: std::time::Duration = std::time::Duration::from_millis(400);

thread_local! {
    /// Every open dialog-hosted panel. An entry goes when its dialog reports `closed`.
    static DIALOGS: RefCell<Vec<Rc<DialogHost>>> = const { RefCell::new(Vec::new()) };
}

/// The dialog host whose content container is `host`.
fn dialog_host_for(host: &Handle) -> Option<Rc<DialogHost>> {
    DIALOGS.with(|d| {
        d.borrow()
            .iter()
            .find(|h| h.fixed.upcast_ref::<gtk4::Widget>() == host)
            .cloned()
    })
}

/// The dialog host whose window root is `node`.
fn dialog_host_of_node(node: NodeId) -> Option<Rc<DialogHost>> {
    DIALOGS.with(|d| d.borrow().iter().find(|h| h.node == node).cloned())
}

/// The first header bar under `w`: a dialog's own, for the content height it takes.
fn find_header_bar(w: &gtk4::Widget) -> Option<adw::HeaderBar> {
    if let Some(h) = w.downcast_ref::<adw::HeaderBar>() {
        return Some(h.clone());
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        if let Some(h) = find_header_bar(&c) {
            return Some(h);
        }
        child = c.next_sibling();
    }
    None
}

/// The height libadwaita's header bar takes when it cannot be measured yet.
const HEADER_BAR_FALLBACK_H: f64 = 47.0;

impl DialogHost {
    /// A plain `AdwDialog`: a header bar with the title over Day's content, which fills the
    /// dialog and reports its allocation as the window size. No-op once a dialog exists.
    fn materialize_plain(self: &Rc<Self>) {
        if self.dialog.borrow().is_some() {
            return;
        }
        let dialog = adw::Dialog::new();
        dialog.set_title(&self.title.borrow());
        let header = adw::HeaderBar::new();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        // The same minimum-size breaker a window's content has (`build_day_window`), plus the
        // allocation report GTK offers no signal for.
        let cell = DayCell::filling();
        cell.add_child(self.fixed.upcast_ref());
        let node = self.node;
        cell.on_allocate(move |w, h| {
            ffi_guard::contain((), || {
                emit(
                    node,
                    Event::WindowResized(Size::new(f64::from(w), f64::from(h))),
                )
            })
        });
        toolbar.set_content(Some(&cell));
        dialog.set_child(Some(&toolbar));
        self.install(dialog);
    }

    /// The `AdwPreferencesDialog` the settings shell fills: made on the first claim, answered
    /// again on later ones. `None` once a plain dialog holds the content.
    fn claim_preferences(self: &Rc<Self>) -> Option<adw::PreferencesDialog> {
        if let Some(d) = self.dialog.borrow().clone() {
            return d.downcast::<adw::PreferencesDialog>().ok();
        }
        let dialog = adw::PreferencesDialog::new();
        dialog.set_title(&self.title.borrow());
        self.install(dialog.clone().upcast());
        Some(dialog)
    }

    /// Keep `dialog`, wire its close to day-core's confirm path, and present it on the next
    /// main-loop turn.
    fn install(self: &Rc<Self>, dialog: adw::Dialog) {
        *self.dialog.borrow_mut() = Some(dialog.clone());
        let node = self.node;
        let parent = self.parent.clone();
        let host = Rc::downgrade(self);
        dialog.connect_closed(move |d| {
            ffi_guard::contain((), || {
                if let Some(h) = host.upgrade() {
                    h.closed.set(true);
                }
                DIALOGS.with(|v| {
                    v.borrow_mut()
                        .retain(|h| h.dialog.borrow().as_ref() != Some(d))
                });
                // The window underneath is key again: an in-window dialog never changed the
                // toplevel's active state, so nothing else reports it.
                emit(node, Event::WindowFocused(false));
                if let Some(track) = state_track(&parent) {
                    emit(track.target, Event::WindowFocused(true));
                }
                // day-core tears down on a deferred hop, never inside this signal.
                emit(node, Event::WindowClosed);
            })
        });
        let host = Rc::downgrade(self);
        gtk4::glib::idle_add_local_once(move || {
            ffi_guard::contain((), || {
                if let Some(h) = host.upgrade() {
                    h.present();
                }
            })
        });
    }

    /// Show the dialog over its window at the size it was fitted to.
    fn present(&self) {
        if self.closed.get() || self.presented.replace(true) {
            return;
        }
        let Some(dialog) = self.dialog.borrow().clone() else {
            return;
        };
        dialog.present(panel_parent(self.parent.upcast_ref()));
        self.presented_at.set(Some(std::time::Instant::now()));
        self.apply_size();
    }

    /// The widget a capture of the dialog renders: the sheet libadwaita lays the dialog's
    /// content in (floating, or the bottom sheet of a narrow window), which paints the
    /// background, corners and shadow. The dialog widget itself spans the whole window: it
    /// holds the dimming layer too.
    fn capture_widget(&self) -> Option<gtk4::Widget> {
        fn sheet_under(w: &gtk4::Widget) -> Option<gtk4::Widget> {
            if w.css_name() == "sheet" {
                return Some(w.clone());
            }
            let mut child = w.first_child();
            while let Some(c) = child {
                if let Some(s) = sheet_under(&c) {
                    return Some(s);
                }
                child = c.next_sibling();
            }
            None
        }
        let dialog: gtk4::Widget = self.dialog.borrow().clone()?.upcast();
        Some(sheet_under(&dialog).unwrap_or(dialog))
    }

    /// Whether the dialog is on screen and done animating in.
    fn settled(&self) -> bool {
        self.presented_at
            .get()
            .is_some_and(|t| t.elapsed() >= DIALOG_ENTRANCE)
    }

    /// Size the dialog to its content: `size` plus the header bar, which the dialog's content
    /// height includes. Nothing before presentation; `present` applies it then.
    fn apply_size(&self) {
        if !self.presented.get() || self.closed.get() {
            return;
        }
        let Some(dialog) = self.dialog.borrow().clone() else {
            return;
        };
        let bar = find_header_bar(dialog.upcast_ref())
            .map(|h| {
                let h = h.measure(gtk4::Orientation::Vertical, -1).1;
                if h > 0 {
                    f64::from(h)
                } else {
                    HEADER_BAR_FALLBACK_H
                }
            })
            .unwrap_or(0.0);
        let size = self.size.get();
        // Never narrower than libadwaita's own preferences dialog (640px): the header's view
        // switcher is laid out for that width, and narrower it truncates every page title
        // ("Gen…") before its breakpoint moves the switcher to a bottom bar.
        dialog.set_content_width(size.width.max(ADW_PREFERENCES_WIDTH).round() as i32);
        dialog.set_content_height((size.height + bar).round() as i32);
    }

    /// Close the dialog; `closed` confirms to day-core. A dialog not presented yet (closed in
    /// the turn it opened) has nothing to animate, so the confirm goes straight out.
    fn close(self: &Rc<Self>) {
        if self.closed.get() {
            return;
        }
        let dialog = self.dialog.borrow().clone();
        match dialog {
            Some(d) if self.presented.get() => {
                d.close();
            }
            _ => {
                self.closed.set(true);
                let node = self.node;
                DIALOGS.with(|v| v.borrow_mut().retain(|h| h.node != node));
                gtk4::glib::idle_add_local_once(move || {
                    ffi_guard::contain((), || emit(node, Event::WindowClosed))
                });
            }
        }
    }
}

/// The width libadwaita gives an `AdwPreferencesDialog` (its `content-width` default).
const ADW_PREFERENCES_WIDTH: f64 = 640.0;

/// The `AdwPreferencesDialog` hosting the panel whose window root is `root`, for the settings
/// shell to fill with its `AdwPreferencesPage`s (docs/settings.md). Made on the first call,
/// which must come before any content mounts into the window (the shell's realize does); `None`
/// when `root` is not a dialog-hosted `WindowKind::Preferences` panel, or when its content
/// already took a plain dialog.
pub fn preferences_dialog_of(root: NodeId) -> Option<adw::PreferencesDialog> {
    dialog_host_of_node(root)
        .filter(|h| h.kind == day_spec::WindowKind::Preferences)
        .and_then(|h| h.claim_preferences())
}

/// GTK 4.14's `gtk_accessible_announce(GtkAccessible *, const char *, GtkAccessibleAnnouncementPriority)`.
type AnnounceFn =
    unsafe extern "C" fn(*mut gtk4::ffi::GtkAccessible, *const std::ffi::c_char, std::ffi::c_int);

/// `gtk_accessible_announce`, if the GTK this process loaded has it (4.14; the binding is
/// pinned at v4_10, so the symbol is looked up rather than linked). Windows builds link GTK
/// through MSYS2 and have no `dlsym`; they answer `None` until the binding catches up.
fn announce_symbol() -> Option<AnnounceFn> {
    #[cfg(unix)]
    {
        // SAFETY: `RTLD_DEFAULT` searches the images already loaded, the name is a NUL-terminated
        // literal, and the pointer is only used after a null check, as the type the GTK header
        // declares.
        let sym = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"gtk_accessible_announce".as_ptr()) };
        if sym.is_null() {
            return None;
        }
        // SAFETY: a non-null `dlsym` result for this name is the function GTK exports under it.
        Some(unsafe { std::mem::transmute::<*mut std::ffi::c_void, AnnounceFn>(sym) })
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Speak `text` through the screen reader via `widget`'s accessible (docs/accessibility.md).
///
/// Only when the widget's AT context is GTK's AT-SPI one, which GTK makes only after the
/// accessibility bus answered: without a bus GTK stands a context that has no announcement,
/// and 4.14 reaches the call through it anyway, which ends the process. With no screen reader
/// nobody would hear it, so staying silent there loses nothing.
fn announce_through(widget: &gtk4::Widget, text: &str, urgent: bool) {
    let Some(announce) = announce_symbol() else {
        return;
    };
    if widget.at_context().type_().name() != "GtkAtSpiContext" {
        return;
    }
    let Ok(text) = std::ffi::CString::new(text) else {
        return;
    };
    // GTK_ACCESSIBLE_ANNOUNCEMENT_PRIORITY_MEDIUM waits its turn; HIGH interrupts.
    let priority = if urgent { 2 } else { 1 };
    // SAFETY: `widget` is a live GtkWidget, which implements GtkAccessible, and `text` outlives
    // the call; the function is the one GTK exports (see `announce_symbol`).
    unsafe {
        announce(
            widget.as_ptr().cast::<gtk4::ffi::GtkAccessible>(),
            text.as_ptr(),
            priority,
        )
    };
}

pub struct Gtk {
    #[cfg(target_os = "macos")]
    sharing: Option<objc2::rc::Retained<objc2::runtime::AnyObject>>,

    registry: Registry<Gtk>,
    window_fixed: Option<gtk4::Fixed>,
    /// The application, retained so `open_window` can create windows after activate.
    app: Option<adw::Application>,
    secondary: Vec<GtkWin>,
    /// The app menu bar, if installed — kept so a re-`set_app_menu` can replace it.
    /// In-window presentation (Linux/Windows); macOS renders the model in the GLOBAL
    /// menu bar instead (`set_menubar` — the quartz backend maps it natively).
    menu_bar: Option<gtk4::PopoverMenuBar>,
    /// The current app-menu action group, inserted on every Day window ("daymenu."
    /// resolves against the focused window, so the macOS global menubar keeps working
    /// while a secondary window is key).
    menu_group: Option<gtk4::gio::SimpleActionGroup>,
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
            #[cfg(target_os = "macos")]
            sharing: None,
            app: None,
            secondary: Vec::new(),
            menu_bar: None,
            menu_group: None,
        }
    }

    /// The window a dialog presents over: the active Day window, else the primary (the same
    /// rule `present` uses for alerts). `None` before the first window exists.
    /// The window a panel (a dialog) presents over: the active window, else the window that
    /// was active most recently and is still up, else the secondary window opened last, else
    /// the primary. The fallbacks matter on macOS, where choosing from the global menu bar
    /// leaves no window `is-active` for the moment the action runs: without them a Settings…
    /// chosen while a reader window was in front landed on the primary window BEHIND it, and
    /// showed only once the reader window closed.
    fn active_window(&self) -> Option<gtk4::Window> {
        let primary = self
            .window_fixed
            .as_ref()
            .and_then(|f| f.root())
            .and_then(|r| r.downcast::<gtk4::Window>().ok());
        self.secondary
            .iter()
            .find(|w| w.window.is_active())
            .map(|w| w.window.clone().upcast())
            .or_else(|| primary.clone().filter(|w| w.is_active()))
            .or_else(|| {
                LAST_ACTIVE_WINDOW
                    .with(|w| w.borrow().as_ref().and_then(|w| w.upgrade()))
                    .filter(|w| w.is_visible())
            })
            .or_else(|| {
                self.secondary
                    .iter()
                    .rev()
                    .find(|w| w.window.is_visible())
                    .map(|w| w.window.clone().upcast())
            })
            .or(primary)
    }

    /// The GtkWindow whose content container is `host`: a secondary window's, or the
    /// primary's (`window_fixed`, which `ready` handed out).
    fn gtk_window_for(&self, host: &Handle) -> Option<gtk4::Window> {
        if let Some(w) = self
            .secondary
            .iter()
            .find(|w| w.fixed.upcast_ref::<gtk4::Widget>() == host)
        {
            return Some(w.window.clone().upcast());
        }
        self.window_fixed
            .as_ref()
            .filter(|f| f.upcast_ref::<gtk4::Widget>() == host)
            .and_then(|f| f.root())
            .and_then(|r| r.downcast::<gtk4::Window>().ok())
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

/// Point size + the style's inherent weight for a logical [`Font`]. Public for standalone pieces
/// (docs/extending.md), which have to resolve the same scale their labels do. GTK has no semantic text styles,
/// so we approximate the platform typographic scale (matching the Apple text-style sizes for cross-
/// platform consistency). Pango point sizes are rendered through the Xft DPI, which GNOME's
/// text-scaling-factor (Settings ▸ Accessibility ▸ Large Text) feeds — so these scale for accessibility.
pub fn gtk_style(f: Font) -> (f64, day_spec::FontWeight) {
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

/// A Day weight as a Pango one. Public for standalone pieces that build their own Pango
/// attributes or text tags (docs/extending.md).
pub fn pango_weight(w: day_spec::FontWeight) -> gtk4::pango::Weight {
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

/// A label's remembered style: the base font, its color, and — for a label with styled runs —
/// the text and runs the markup is built from.
#[derive(Default, Clone)]
struct LabelStyle {
    font: day_spec::FontSpec,
    color: Option<day_spec::Color>,
    /// `Some` once `.runs()` has put styled runs on this label; the pair rebuilds the markup
    /// whenever the base font or color is patched.
    rich: Option<(String, Vec<day_spec::TextRun>)>,
}

/// The base font and color as a Pango markup span that WRAPS a label's run markup.
///
/// A `GtkLabel`'s attribute list OVERRIDES the attributes its markup parsed, so a base weight
/// attribute spanning the whole label silently defeats a `<b>` run — bold text rendered at the
/// body weight while italic, color and the monospace family (which set no base attribute) came
/// through. A rich label therefore carries NO attribute list, and its base font arrives as this
/// wrapping span. Inside one markup parse a nested tag wins over an enclosing span, which is the
/// ordering the run tags need.
fn base_span(spec: day_spec::FontSpec, color: Option<day_spec::Color>) -> String {
    use gtk4::pango;
    let (size_pt, inherent) = gtk_style(spec.style);
    let weight = spec.weight.unwrap_or(inherent);
    let mut s = String::from("<span");
    // Monospace wins over a bundled family, as it does on Qt: the label asked for code type.
    if spec.monospace {
        s.push_str(" font_family=\"monospace\"");
    } else if let Font::Custom(family, _) = spec.style {
        s.push_str(&format!(" font_family=\"{family}\""));
    }
    // Pango markup sizes are in 1024ths of a point, the same unit as `AttrSize`.
    s.push_str(&format!(
        " size=\"{}\"",
        (size_pt * pango::SCALE as f64) as i32
    ));
    s.push_str(&format!(
        " weight=\"{}\"",
        gtk4::glib::translate::IntoGlib::into_glib(pango_weight(weight))
    ));
    if spec.italic {
        s.push_str(" style=\"italic\"");
    }
    if spec.tabular {
        s.push_str(" font_features=\"tnum 1\"");
    }
    if let Some(c) = color {
        let ch = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        s.push_str(&format!(
            " foreground=\"#{:02x}{:02x}{:02x}\"",
            ch(c.r),
            ch(c.g),
            ch(c.b)
        ));
        if c.a < 1.0 {
            s.push_str(&format!(
                " alpha=\"{}\"",
                (c.a.clamp(0.0, 1.0) * 65535.0) as u16
            ));
        }
    }
    s.push('>');
    s
}

/// The full markup for a rich label: its base font as a wrapping span, run tags inside.
fn rich_markup(text: &str, runs: &[day_spec::TextRun], style: &LabelStyle) -> String {
    let mut m = base_span(style.font, style.color);
    m.push_str(&day_spec::runs_to_markup(
        text,
        runs,
        day_spec::MarkupDialect::Pango,
        gtk_style(style.font.style).0,
    ));
    m.push_str("</span>");
    m
}

/// Rebuild a label's full Pango attribute list: font family/size/weight/style + foreground color.
fn apply_text_attrs(label: &gtk4::Label, spec: day_spec::FontSpec, color: Option<day_spec::Color>) {
    use gtk4::pango;
    let (size_pt, inherent) = gtk_style(spec.style);
    let weight = spec.weight.unwrap_or(inherent);
    // Pango attribute list (markup-free): size, weight, italic style, and foreground.
    let attrs = pango::AttrList::new();
    // A bundled family (§18.4), registered with the platform font system in run(). Pango falls
    // back to the default family if the name doesn't resolve. Monospace wins over it, as it does
    // on Qt, through Pango's `monospace` alias for the system's fixed-width face.
    let family = if spec.monospace {
        Some("monospace")
    } else if let Font::Custom(family, _) = spec.style {
        Some(family)
    } else {
        None
    };
    if let Some(family) = family {
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
    // Tabular figures come from the OpenType feature rather than a different family, so the face
    // is untouched and only the digits change metrics. A font without `tnum` simply ignores it.
    if spec.tabular {
        let mut f = pango::AttrFontFeatures::new("tnum 1");
        f.set_start_index(0);
        attrs.insert(f);
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

/// Update one part of a label's remembered style and re-apply the whole of it.
///
/// A label with styled runs re-renders its markup instead of taking an attribute list, since the
/// list would override the runs (see [`base_span`]).
fn update_text_attrs(
    label: &gtk4::Label,
    font: Option<day_spec::FontSpec>,
    color: Option<Option<day_spec::Color>>,
) {
    use gtk4::prelude::*;
    let key = label.clone().upcast::<gtk4::Widget>().as_ptr() as usize;
    let style = LABEL_STYLE.with(|m| {
        let mut m = m.borrow_mut();
        let entry = m.entry(key).or_default();
        if let Some(f) = font {
            entry.font = f;
        }
        if let Some(c) = color {
            entry.color = c;
        }
        entry.clone()
    });
    match &style.rich {
        Some((text, runs)) => {
            label.set_attributes(None);
            label.set_markup(&rich_markup(text, runs, &style));
        }
        None => apply_text_attrs(label, style.font, style.color),
    }
}

/// Register bundled fonts (§18.4) with whatever font system Pango draws from on this OS, so
/// `Font::Custom` family names resolve: fontconfig on Linux; on macOS both CoreText and
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
                log::warn!("could not register bundled font {}", path.display());
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
                log::warn!("could not register bundled font {}", path.display());
            }
        }
    }
}

/// Verify every bundled font family resolved into the Pango fontmap GTK is actually using —
/// the loud half of §18.4's degrade-loudly rule. Pango fontmaps enumerate families at creation
/// (see the `register_bundled_fonts` call at the top of `run`), so a family missing here means
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
            log::warn!(
                "bundled font family {:?} ({}) did not register with Pango — labels using \
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

/// Warn once per kind that this backend has no registered renderer for `kind`, before falling back to
/// a visible placeholder. A missing renderer usually means the piece's `gtk` feature wasn't enabled
/// (Tier A.2 derives it automatically under `day build`). Deduped per kind so a placeholder rendered
/// every frame doesn't spam the log.
fn warn_missing_renderer(kind: PieceKind) {
    day_spec::placeholder::report(kind, "gtk");
}

/// The visible stand-in a realize arm degrades to: for a missing renderer, and for a props
/// payload that is not the type the arm expects (`props_of` reports the mismatch through the
/// same per-kind dedup channel `warn_missing_renderer` uses).
pub(crate) fn placeholder_label(kind: PieceKind) -> Handle {
    gtk4::Label::new(Some(&format!("⟨{kind}⟩"))).upcast()
}

impl Toolkit for Gtk {
    type Handle = Handle;

    fn dark_mode(&mut self) -> bool {
        // libadwaita's StyleManager already folds together the system preference, the
        // DAY_THEME startup force, and any set_appearance override.
        adw::StyleManager::default().is_dark()
    }

    fn set_appearance(&mut self, dark: Option<bool>) {
        adw::StyleManager::default().set_color_scheme(match dark {
            Some(true) => adw::ColorScheme::ForceDark,
            Some(false) => adw::ColorScheme::ForceLight,
            None => adw::ColorScheme::Default,
        });
    }

    fn reduce_motion(&mut self) -> bool {
        // `gtk-enable-animations` is the one switch GNOME's "Animations" toggle and its
        // reduce-motion accessibility setting both drive (docs/accessibility.md); libadwaita's
        // own transitions read it too. No GtkSettings before `startup` means no display yet,
        // which is not a request for less motion.
        gtk4::Settings::default().is_some_and(|s| !s.is_gtk_enable_animations())
    }

    fn attach_refresh(&mut self, handle: &Self::Handle, host: NodeId) {
        controls::refresh_gesture::attach(handle, host);
    }

    fn set_presented(&mut self, handle: &Self::Handle, visible: bool) {
        handle.set_visible(visible);
    }

    fn capability(&self, cap: Cap) -> Support {
        match cap {
            Cap::SearchField => Support::Native,
            Cap::ComboBox => Support::Native,
            Cap::Stepper => Support::Native,
            Cap::DateTimePicker => Support::Emulated,
            Cap::ColorPicker => Support::Native,
            Cap::PullRefresh => Support::Emulated,
            Cap::Pager => Support::Emulated,
            Cap::PullRefreshGesture => Support::Emulated,
            // `gdk::Cursor::from_name` takes the CSS vocabulary as it is (docs/cursor.md).
            Cap::Cursor => Support::Native,
            // Opacity and transform tween on libadwaita's frame clock (`gtk_animation`); a
            // frame applies instantly.
            Cap::Animation => Support::Native,
            // `gtk-enable-animations`, watched through its notify (docs/accessibility.md).
            Cap::ReduceMotion => Support::Native,
            // Pango's font map lists every fontconfig family and face (docs/fonts.md).
            Cap::FontList => Support::Native,
            // A GNOME list row's content (docs/list.md "List items").
            Cap::ListItem => Support::Native,
            // Headers are rows of the flat model, unselectable and unfocusable, and a copy of
            // the current one is pinned over the top in a GtkOverlay (`PinnedHeader`):
            // GtkListView's own section headers (GTK 4.12) scroll away (docs/list.md "Sections").
            Cap::ListSections => Support::Emulated,
            // `GdkTexture::from_bytes` reads what gdk-pixbuf's loaders read, and pixbuf writes
            // back PNG/JPEG/TIFF/BMP (docs/images.md). `Cap::ImageProperties` is deliberately NOT
            // here: there is no metadata reader on this path, and answering an empty struct would
            // read as "this file records nothing" rather than "nobody looked".
            Cap::ImageDecode | Cap::ImageEncode => Support::Native,
            // Canvas export (docs/canvas.md "Exporting"): `cairo_draw` paints a cairo image
            // surface for a bitmap and a cairo PDF surface for a document. The PDF symbols live in
            // libcairo itself; Homebrew, Debian/Ubuntu and MSYS2's mingw-w64-cairo (the
            // windows-gtk toolchain, which ships cairo-pdf.h and cairo-pdf.pc) all build that
            // backend, so every GTK target links and answers it.
            Cap::CanvasBitmap | Cap::CanvasPdf => Support::Native,
            // GtkTextView is editable-toggleable; it's always selectable and ships no spell-check,
            // so TextSelectable / TextSpellCheck stay Unsupported (the default arm).
            Cap::TextRuns
            | Cap::TextLinks
            | Cap::Snapshot
            | Cap::NavSplit
            // The Adwaita view-switching idiom: an AdwViewStack of resident pages under a
            // `.linked` row of grouped toggle buttons, which is GNOME's segmented one-of-N
            // switch (docs/navigation.md).
            //
            // `Cap::NavTabsAdaptive` is deliberately not here: a GNOME app may PIN a tab bar,
            // but a narrowing window collapses its sidebar and pushes rather than growing one.
            | Cap::NavTabs
            | Cap::Dialogs
            | Cap::FileDialogs
            // GtkFileDialog's `select_folder` (docs/files.md).
            | Cap::FolderDialog
            | Cap::TextEditable
            // GTK's own DnD framework (DragSource/DropTarget) drives row reorder; the drop gap
            // indicator is the drag icon + forbidden cursor (docs/list.md has the nuance).
            | Cap::DragDeferredReceipt | Cap::DragDrop | Cap::DragExternalImport | Cap::DragExternalExport | Cap::DragMultipleItems | Cap::DragFileReferences | Cap::NavReorder | Cap::ListReorder
            // GtkListView's factory binds and unbinds Day's `DayCell`s as rows come and go,
            // and across a far jump it tears item widgets down and sets up new ones; the
            // factory's `teardown` releases the row Day built in a discarded cell, so the
            // rows alive stay bounded by the view's own pool (docs/list.md).
            | Cap::ListRecycling
            // GtkListView + GtkTreeListModel + GtkTreeExpander host day-built rows natively
            // (docs/tree.md). `Cap::TreeMove` is deliberately NOT here yet: the native drag
            // half lands after the seam parity — dayscript's `tree_move:` drives the seam
            // regardless.
            | Cap::Tree
            | Cap::TreeSections
            // Real AdwApplicationWindows on the shared GtkApplication (docs/windows.md).
            | Cap::MultiWindow
            // `gtk_window_minimize` / `maximize` / `fullscreen` and their inverses, reported
            // back from the window's `maximized` / `fullscreened` properties and the toplevel
            // surface's state (`watch_window_state`). Wayland never reports minimized; see
            // `StateTrack` for how this backend stands in for it there. `Cap::ContentProtection`
            // stays Unsupported: no Linux display server lets a client opt out of capture.
            | Cap::WindowStates
            | Cap::WindowFullscreen
            // No title bar (`set_decorated(false)` without the shadow; the client-side frame
            // keeps its resize edges otherwise), or GtkWindowControls over the content's top
            // band for an overlay title bar (docs/window-chrome.md).
            | Cap::FramelessWindow
            | Cap::OverlayTitleBar
            // GtkWindowHandle's behavior through `gdk::Toplevel::begin_move`.
            | Cap::DragRegion
            // `set_default_size` and size requests; no maximum size in GTK 4.
            | Cap::WindowGeometry
            // `gdk::Display::monitors()`: geometry and scale, no work area (it reads as the whole
            // display).
            | Cap::Monitors
            // The window's AdwHeaderBar — GNOME's toolbar (docs/toolbars.md).
            | Cap::Toolbar
            // The bar holds a native search field (docs/search.md).
            | Cap::ToolbarSearch
            | Cap::AppMenu
            | Cap::Appearance
            // gtk_widget_measure reports baselines itself (docs/baseline.md).
            | Cap::BaselineAlignment
            // The middle pane of the nav host's nested GtkPaneds (`NavSplit`): a real pane at
            // every presentation, with a draggable divider on each side (docs/navigation.md).
            | Cap::NavContentList
            // A GtkPaned with the panel at the trailing (or leading) edge (docs/inspector.md).
            | Cap::Inspector
            // A GtkPaned of two filling cells, either orientation (docs/split.md).
            | Cap::Split => Support::Native,
            // A topmost child of the window's root Fixed — not a system modal (docs/cover.md).
            Cap::Cover => Support::Emulated,
            // The library on the machine decides: `gtk_accessible_announce` arrived in 4.14,
            // and the binding is pinned below it (docs/accessibility.md).
            Cap::Announce => {
                if announce_symbol().is_some() {
                    Support::Native
                } else {
                    Support::Unsupported
                }
            }
            // An RGBA visual on a composited display; bare X11 has none.
            Cap::TransparentWindow => {
                if shell::can_be_transparent() {
                    Support::Native
                } else {
                    Support::Unsupported
                }
            }
            // A StatusNotifierWatcher on the session bus (docs/status-item.md).
            Cap::StatusItem => shell::status_item_support(),
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
            // Deliberately Unsupported (the default arm): `WindowPosition`, `WindowLevel`,
            // `RequestAttention` (GTK 4 removed positioning, keep-above, stick, skip-taskbar and
            // the urgency hint), `WindowAppearance` (the color scheme is app-wide),
            // `WindowMaterial` (no blur behind a window on Linux), `ContentProtection`,
            // `DynamicShortcuts` (a .desktop file's actions are fixed at install).
            _ => Support::Unsupported,
        }
    }

    fn realize(&mut self, kind: PieceKind, props: &dyn std::any::Any, id: NodeId) -> Handle {
        match Builtin::from_key(kind) {
            Some(Builtin::Container) => {
                let w: Handle = gtk4::Fixed::new().upcast();
                if let Some(p) = props.downcast_ref::<ContainerProps>() {
                    if p.role == Some(day_spec::SurfaceRole::SectionCard) {
                        // libadwaita's own grouped-card treatment — rounded, elevated, and
                        // theme-adaptive (follows the Adwaita light/dark stylesheet).
                        w.add_css_class("card");
                    } else if p.role == Some(day_spec::SurfaceRole::Window) {
                        // Adwaita's `background` class, the theme window background an opaque
                        // cover paints; the rounding and clip ride the surface provider.
                        w.add_css_class("background");
                        if p.corner_radius > 0.0 || p.clips {
                            apply_surface(&w, None, p.corner_radius, p.clips);
                        }
                    } else if p.background.is_some() || p.corner_radius > 0.0 || p.clips {
                        apply_surface(&w, p.background, p.corner_radius, p.clips);
                    }
                }
                w
            }
            Some(Builtin::Inspector) => {
                let (visible, width, edge) = props
                    .downcast_ref::<InspectorProps>()
                    .map(|p| (p.visible, p.width, p.edge))
                    .unwrap_or((false, 280.0, PaneEdge::Trailing));
                // A GtkPaned, the nav split's device (`NavSplit` records why not the Adw
                // split): the panel is a filling `DayCell` whose size request is the honest
                // minimum, the content another whose request is nothing, and neither may
                // shrink below it. The pane's side follows the piece: Trailing is the classic
                // inspector, Leading a utility pane like a layer panel (docs/tree.md).
                let trailing = matches!(edge, PaneEdge::Trailing);
                let panel = DayCell::filling();
                panel.set_size_request(INSPECTOR_MIN_W.min(width) as i32, -1);
                // Hidden before it joins the paned (the nav list pane's rule).
                panel.set_visible(visible);
                let content = DayCell::filling();
                let paned = gtk4::Paned::new(gtk4::Orientation::Horizontal);
                if trailing {
                    paned.set_start_child(Some(&content));
                    paned.set_end_child(Some(&panel));
                } else {
                    paned.set_start_child(Some(&panel));
                    paned.set_end_child(Some(&content));
                }
                // Window resizes go to the content; the panel holds its width.
                paned.set_resize_start_child(!trailing);
                paned.set_resize_end_child(trailing);
                paned.set_shrink_start_child(false);
                paned.set_shrink_end_child(false);
                // The host: a filling cell around the paned, for the allocation observer a
                // trailing panel's placement needs (its divider position is measured from the
                // host's far side).
                let host = DayCell::filling();
                host.add_child(paned.upcast_ref());
                let handle: Handle = host.clone().upcast();
                let key = widget_key(&handle);
                let state = Rc::new(RefCell::new(InspectorState {
                    paned: paned.clone(),
                    content,
                    panel,
                    width,
                    trailing,
                    shown: visible,
                    panes: Vec::new(),
                }));
                if !trailing {
                    state.borrow().place(0);
                }
                {
                    // First allocation: place the divider for the panel's width — before the
                    // paned is allocated, so it lands in this pass. Thereafter the paned's
                    // resize flags keep the panel's width through window resizes.
                    let st = state.clone();
                    let placed = std::cell::Cell::new(false);
                    host.on_allocate(move |width, _| {
                        if !placed.replace(true) {
                            st.borrow().place(width);
                        }
                    });
                }
                // Each pane reports its page's size as it is allocated, so a divider drag
                // re-lays the page in the same pass (the nav panes' rule, `nav_pane_report`).
                for is_panel in [false, true] {
                    let st = state.clone();
                    let cell = {
                        let s = st.borrow();
                        if is_panel {
                            s.panel.clone()
                        } else {
                            s.content.clone()
                        }
                    };
                    let last = std::cell::Cell::new((0, 0));
                    cell.on_allocate(move |width, height| {
                        if width <= 0
                            || height <= 0
                            || last.replace((width, height)) == (width, height)
                        {
                            return;
                        }
                        // Collected first: the dispatch may attach a pane, which borrows.
                        let ids: Vec<NodeId> = st
                            .borrow()
                            .panes
                            .iter()
                            .filter(|(_, panel, _)| *panel == is_panel)
                            .map(|(id, ..)| *id)
                            .collect();
                        ffi_guard::contain((), || {
                            for id in ids {
                                emit(
                                    id,
                                    Event::FrameChanged(Size::new(
                                        f64::from(width),
                                        f64::from(height),
                                    )),
                                );
                            }
                        });
                    });
                }
                {
                    // A divider drag: the panel keeps the dragged width across a hide and a
                    // reveal (the re-lay is the pane cells', above).
                    let st = state.clone();
                    paned.connect_position_notify(move |_| {
                        if let Ok(mut st) = st.try_borrow_mut()
                            && st.shown
                            && st.panel.width() > 0
                        {
                            st.width = f64::from(st.panel.width());
                        }
                    });
                }
                INSPECTOR_STATE.with(|t| t.insert(key, state));
                handle
            }
            Some(Builtin::Split) => {
                let (axis, fraction, min_pane) = props
                    .downcast_ref::<SplitProps>()
                    .map(|p| (p.axis, p.fraction, p.min_pane))
                    .unwrap_or((day_spec::props::SplitAxis::Horizontal, 0.5, 80.0));
                let stacked = axis == day_spec::props::SplitAxis::Vertical;
                let first = DayCell::filling();
                let second = DayCell::filling();
                let paned = gtk4::Paned::new(if stacked {
                    gtk4::Orientation::Vertical
                } else {
                    gtk4::Orientation::Horizontal
                });
                paned.set_start_child(Some(&first));
                paned.set_end_child(Some(&second));
                // Both panes resize with the host, proportionally, so a dragged share holds
                // through a window resize; neither shrinks under its minimum.
                paned.set_resize_start_child(true);
                paned.set_resize_end_child(true);
                paned.set_shrink_start_child(false);
                paned.set_shrink_end_child(false);
                let host = DayCell::filling();
                host.add_child(paned.upcast_ref());
                let handle: Handle = host.clone().upcast();
                let key = widget_key(&handle);
                let state = Rc::new(RefCell::new(SplitState {
                    paned: paned.clone(),
                    first,
                    second,
                    node: id,
                    share: fraction.clamp(0.0, 1.0),
                    min_pane,
                    placed: false,
                    suppress: false,
                    panes: Vec::new(),
                }));
                state.borrow().request_minimums();
                {
                    // First allocation: place the divider by the share, once.
                    let st = state.clone();
                    host.on_allocate(move |width, height| {
                        if let Ok(mut st) = st.try_borrow_mut()
                            && !st.placed
                        {
                            let len = if st.stacked() { height } else { width };
                            st.place_along(len);
                        }
                    });
                }
                // Each pane reports its page's size as it is allocated (the inspector's rule).
                for is_second in [false, true] {
                    let st = state.clone();
                    let cell = {
                        let s = st.borrow();
                        if is_second {
                            s.second.clone()
                        } else {
                            s.first.clone()
                        }
                    };
                    let last = std::cell::Cell::new((0, 0));
                    cell.on_allocate(move |width, height| {
                        if width <= 0
                            || height <= 0
                            || last.replace((width, height)) == (width, height)
                        {
                            return;
                        }
                        let ids: Vec<NodeId> = st
                            .borrow()
                            .panes
                            .iter()
                            .filter(|(_, second, _)| *second == is_second)
                            .map(|(id, ..)| *id)
                            .collect();
                        ffi_guard::contain((), || {
                            for id in ids {
                                emit(
                                    id,
                                    Event::FrameChanged(Size::new(
                                        f64::from(width),
                                        f64::from(height),
                                    )),
                                );
                            }
                        });
                    });
                }
                {
                    // A divider drag: report the new share (not Day's own placement).
                    let st = state.clone();
                    paned.connect_position_notify(move |p| {
                        let report = {
                            let Ok(mut st) = st.try_borrow_mut() else {
                                return;
                            };
                            if st.suppress || !st.placed {
                                return;
                            }
                            let usable = st.usable();
                            if usable <= 0.0 {
                                return;
                            }
                            let share = (f64::from(p.position()) / usable).clamp(0.0, 1.0);
                            if (share - st.share).abs() <= 1e-4 {
                                return;
                            }
                            st.share = share;
                            (st.node, share)
                        };
                        ffi_guard::contain((), || emit(report.0, Event::ValueChanged(report.1)));
                    });
                }
                SPLIT_STATE.with(|t| t.insert(key, state));
                handle
            }
            Some(Builtin::SplitPane) => {
                let w: Handle = gtk4::Fixed::new().upcast();
                let second = props
                    .downcast_ref::<SplitPaneProps>()
                    .map(|p| p.second)
                    .unwrap_or(false);
                SPLIT_PANES.with(|t| t.insert(widget_key(&w), (id, second)));
                w
            }
            Some(Builtin::InspectorPane) => {
                let w: Handle = gtk4::Fixed::new().upcast();
                let panel = props
                    .downcast_ref::<InspectorPaneProps>()
                    .map(|p| p.panel)
                    .unwrap_or(false);
                INSPECTOR_PANES.with(|t| t.insert(widget_key(&w), (id, panel)));
                w
            }
            Some(Builtin::Nav) => {
                let presentation = props
                    .downcast_ref::<NavProps>()
                    .map(|p| p.presentation)
                    .unwrap_or(day_spec::props::NavPresentation::Split);
                let is_split = presentation.is_split();
                let suppress = Rc::new(std::cell::Cell::new(false));
                if let Some(config) = props
                    .downcast_ref::<NavProps>()
                    .and_then(|p| p.documents.as_ref())
                {
                    let (host, doc) = DocumentHost::new(id, config, suppress.clone());
                    NAV_STATE.with(|m| {
                        m.borrow_mut().insert(
                            widget_key(&host),
                            NavState {
                                present: NavPresent::Documents(doc),
                                split: false,
                                pages: Vec::new(),
                                suppress,
                            },
                        )
                    });
                    return host;
                }
                if presentation.rows_are_chrome() {
                    // The GNOME view-switching idiom (`NavPresent::Suite` explains).
                    let stack = adw::ViewStack::new();
                    stack.set_vexpand(true);
                    let switcher = adw::ViewSwitcher::new();
                    switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
                    switcher.set_stack(Some(&stack));
                    switcher.set_halign(gtk4::Align::Center);
                    switcher.set_margin_top(6);
                    switcher.set_margin_bottom(6);
                    let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
                    container.append(&switcher);
                    container.append(&stack);
                    let host: Handle = container.upcast();
                    let menu_node = Rc::new(std::cell::Cell::new(0u64));
                    {
                        // A click on the switcher moves the stack: report it against the
                        // NAV_MENU's node, exactly as a sidebar row click is reported — a tab
                        // and a row are one event above this backend. A Day-driven move
                        // (`NavPatch::Select`) is suppressed, per the from-native echo rule.
                        let (s, mn) = (suppress.clone(), menu_node.clone());
                        stack.connect_visible_child_name_notify(move |stack| {
                            ffi_guard::contain((), || {
                                if s.get() || mn.get() == 0 {
                                    return;
                                }
                                if let Some(i) = stack
                                    .visible_child_name()
                                    .and_then(|n| n.strip_prefix('p')?.parse::<usize>().ok())
                                    && i >= 1
                                {
                                    emit(NodeId(mn.get()), Event::SelectionChanged((i - 1) as i64));
                                }
                            });
                        });
                    }
                    NAV_STATE.with(|m| {
                        m.borrow_mut().insert(
                            widget_key(&host),
                            NavState {
                                present: NavPresent::Suite {
                                    stack,
                                    rows: Rc::new(RefCell::new((Vec::new(), Vec::new()))),
                                    menu_node,
                                },
                                split: false,
                                pages: Vec::new(),
                                suppress,
                            },
                        )
                    });
                    return host;
                }
                let (host, present): (Handle, NavPresent) = if is_split {
                    // Two nested GtkPaneds with USER-DRAGGABLE dividers (the AppKit NSSplitView
                    // counterpart; `NavSplit` records why not libadwaita's split). Each pane is
                    // a filling `DayCell` carrying that pane's honest minimum as its size
                    // request, and no child may shrink below it: the divider stops at the
                    // minimum, and a window with no room for all three folds the sidebar
                    // (`NavSplit::fit`) rather than squeezing a pane. Day re-lays each pane's
                    // content to the width reported on every drag and allocation.
                    let nav_props = props.downcast_ref::<NavProps>();
                    let list_width = nav_props.and_then(|p| p.list_width);
                    let list_visible = nav_props.is_none_or(|p| p.list_visible);
                    let sidebar = DayCell::filling();
                    // libadwaita's sidebar background treatment, on the pane itself.
                    sidebar.add_css_class("sidebar-pane");
                    sidebar.set_size_request(day_spec::NAV_SIDEBAR_MIN_W as i32, -1);
                    let list = DayCell::filling();
                    // A host that asked for a narrower list than the shared minimum gets it
                    // (AppKit's rule for the same pane).
                    let list_min = list_width.map_or(day_spec::NAV_LIST_MIN_W, |w| {
                        day_spec::NAV_LIST_MIN_W.min(w)
                    });
                    list.set_size_request(list_min as i32, -1);
                    // No list page, or one collapsed for the opening destination
                    // (`content_list_for`): hidden before it joins the paned, so the host never
                    // opens with an empty pane beside a page that owns none.
                    list.set_visible(list_width.is_some() && list_visible);
                    let detail = DayCell::filling();
                    detail.set_size_request(NAV_DETAIL_MIN_W as i32, -1);
                    let inner = gtk4::Paned::new(gtk4::Orientation::Horizontal);
                    inner.set_start_child(Some(&list));
                    inner.set_end_child(Some(&detail));
                    // Window resizes go to the detail; the list and the sidebar hold their
                    // widths, as on AppKit.
                    inner.set_resize_start_child(false);
                    inner.set_resize_end_child(true);
                    inner.set_shrink_start_child(false);
                    inner.set_shrink_end_child(false);
                    let list_w = list_width.unwrap_or(day_spec::NAV_LIST_WIDTH) as i32;
                    inner.set_position(list_w);
                    let outer = gtk4::Paned::new(gtk4::Orientation::Horizontal);
                    outer.set_start_child(Some(&sidebar));
                    outer.set_end_child(Some(&inner));
                    outer.set_resize_start_child(false);
                    outer.set_resize_end_child(true);
                    outer.set_shrink_start_child(false);
                    outer.set_shrink_end_child(false);
                    outer.set_position(NAV_SIDEBAR_W as i32);
                    let split = Rc::new(NavSplit {
                        outer: outer.clone(),
                        inner: inner.clone(),
                        sidebar,
                        list,
                        detail,
                        sidebar_w: std::cell::Cell::new(NAV_SIDEBAR_W as i32),
                        list_w: std::cell::Cell::new(list_w),
                        sidebar_hidden: std::cell::Cell::new(false),
                        sidebar_fitted: std::cell::Cell::new(false),
                        list_shown: std::cell::Cell::new(list_width.is_some() && list_visible),
                    });
                    // The host: a filling cell around the outer paned, for the allocation
                    // observer `fit` needs (GTK4 has no size-allocate signal). Its own
                    // minimum is nothing, like every Day container's, so the WINDOW can always
                    // narrow past the panes — which is what lets the sidebar fold instead of
                    // the window refusing the resize.
                    let host = DayCell::filling();
                    host.add_child(outer.upcast_ref());
                    let handle: Handle = host.clone().upcast();
                    let key = widget_key(&handle);
                    {
                        let sp = split.clone();
                        host.on_allocate(move |width, _| sp.fit(width));
                    }
                    // Each pane reports its page's size as it is allocated, so the page
                    // follows the divider in the same pass (`nav_pane_report`). Gated on a
                    // change: an allocation at the same size is GTK converging after the
                    // relayout's own size requests, and re-laying then would be wasted work.
                    for (cell, pane) in [
                        (&split.sidebar, day_spec::props::Pane::Sidebar),
                        (&split.list, day_spec::props::Pane::List),
                        (&split.detail, day_spec::props::Pane::Detail),
                    ] {
                        let last = std::cell::Cell::new((0, 0));
                        cell.on_allocate(move |width, height| {
                            if last.replace((width, height)) != (width, height) {
                                ffi_guard::contain((), || {
                                    nav_pane_report(key, pane, width, height)
                                });
                            }
                        });
                    }
                    // A divider drag: remember the width (unless GTK clamped it for want of
                    // room — that is `fit`'s to handle; a position that exactly fills the
                    // room is indistinguishable from a clamp, so it is not recorded either)
                    // and cap it at the pane's maximum. The cap is applied from an idle: the
                    // notify can arrive from inside the paned's own allocation, where a
                    // position write would queue a resize mid-pass. The re-lay itself is the
                    // pane cells' (above): the drag allocates them.
                    {
                        let sp = split.clone();
                        outer.connect_position_notify(move |paned| {
                            let pos = paned.position();
                            let max = day_spec::NAV_SIDEBAR_MAX_W as i32;
                            if pos > max {
                                let paned = paned.clone();
                                gtk4::glib::idle_add_local_once(move || {
                                    paned.set_position(max);
                                });
                            } else if pos >= day_spec::NAV_SIDEBAR_MIN_W as i32
                                && sp.sidebar_showing()
                                && paned.width() > pos + NavSplit::HANDLE + sp.rest_min()
                            {
                                sp.sidebar_w.set(pos);
                            }
                        });
                    }
                    {
                        let sp = split.clone();
                        inner.connect_position_notify(move |paned| {
                            let pos = paned.position();
                            let max =
                                day_spec::NAV_LIST_MAX_W.max(f64::from(sp.list_w.get())) as i32;
                            if pos > max {
                                let paned = paned.clone();
                                gtk4::glib::idle_add_local_once(move || {
                                    paned.set_position(max);
                                });
                            } else if pos >= list_min as i32
                                && sp.list_shown.get()
                                && paned.width()
                                    > pos
                                        + NavSplit::HANDLE
                                        + sp.detail.measure(gtk4::Orientation::Horizontal, -1).0
                            {
                                sp.list_w.set(pos);
                            }
                        });
                    }
                    (handle, NavPresent::Split(split))
                } else {
                    // AdwNavigationView: a genuine push/pop stack with back gesture.
                    let nv = adw::NavigationView::new();
                    let s = suppress.clone();
                    nv.connect_popped(move |_view, _page| {
                        // A native back gesture / Escape popped a page (not a day-driven pop).
                        ffi_guard::contain((), || {
                            if !s.get() {
                                emit(
                                    id,
                                    Event::NavBack {
                                        already_popped: true,
                                    },
                                );
                            }
                        });
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
            Some(Builtin::NavPage) => {
                let (title, pane) = props
                    .downcast_ref::<NavPageProps>()
                    .map(|p| (p.title.clone(), p.pane))
                    .unwrap_or_default();
                let page: Handle = gtk4::Fixed::new().upcast();
                let key = widget_key(&page);
                NAV_PAGE_IDS.with(|m| m.borrow_mut().insert(key, id));
                NAV_PAGE_TITLES.with(|m| m.borrow_mut().insert(key, title));
                NAV_PAGE_PANES.with(|m| m.borrow_mut().insert(key, pane));
                page
            }
            // Emulated fullscreen cover (docs/cover.md): parked hidden; CoverPatch::Present
            // re-homes it onto the window's root Fixed, topmost, sized to the content area.
            Some(Builtin::Cover) => {
                let cover = gtk4::Fixed::new();
                cover.set_visible(false);
                COVER_IDS.with(|t| t.insert(widget_key(&cover.clone().upcast()), id));
                if props_of::<CoverProps>(kind, "gtk", props).is_some_and(|p| p.half) {
                    COVER_HALF.with(|t| t.insert(widget_key(&cover.clone().upcast()), ()));
                }
                cover.upcast()
            }
            Some(Builtin::NavMenu) => {
                let Some(p) = props_of::<NavMenuProps>(kind, "gtk", props) else {
                    return placeholder_label(kind);
                };
                let listbox = gtk4::ListBox::new();
                // The standard GNOME sidebar treatment.
                listbox.add_css_class("navigation-sidebar");
                // Breathing room at the ends: flush against the window edge the first row's
                // ascenders touch the chrome and the selection pill has nowhere to sit.
                listbox.set_margin_top(4);
                listbox.set_margin_bottom(4);
                listbox.set_selection_mode(gtk4::SelectionMode::Single);
                {
                    let key = listbox.as_ptr() as usize;
                    listbox.set_header_func(move |row, _before| nav_menu_headers(row, key));
                }
                fill_nav_menu(
                    &listbox,
                    &p.items,
                    &p.icons,
                    &p.badges,
                    &p.badge_icons,
                    &p.badge_tints,
                    &p.sections,
                    &p.tints,
                    &p.menus,
                );
                let suppress = Rc::new(std::cell::Cell::new(false));
                let current = Rc::new(std::cell::Cell::new(p.selected.map(|i| i as i32)));
                {
                    let (suppress, current) = (suppress.clone(), current.clone());
                    listbox.connect_row_selected(move |_, row| {
                        ffi_guard::contain((), || {
                            if suppress.get() {
                                return;
                            }
                            // Only a CHANGE is a navigation. After a rebuild the box selects the
                            // row it was already on once more, outside the `suppress` window, and
                            // reporting it would rewrite the nav's selection with the same key —
                            // which an app watching that signal reads as the user picking the
                            // row again (Day News closed its open article when an unread badge
                            // changed).
                            if let Some(row) = row
                                && current.replace(Some(row.index())) != Some(row.index())
                            {
                                emit(id, Event::SelectionChanged(row.index() as i64));
                            }
                        });
                    });
                }
                if let Some(sel) = p.selected {
                    suppress.set(true);
                    listbox.select_row(listbox.row_at_index(sel as i32).as_ref());
                    suppress.set(false);
                }
                // The list scrolls within the sidebar (its own scrolled window, like AppKit's
                // NSOutlineView-in-NSScrollView). Without this, the bare ListBox's minimum height
                // (all rows) propagates up to the window's sizing wrapper and a wheel over the
                // sidebar scrolls the entire window.
                let sw = gtk4::ScrolledWindow::new();
                sw.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
                sw.set_child(Some(&listbox));
                let handle: Handle = sw.upcast();
                DOCUMENT_MENU_KEYS.with(|m| {
                    m.borrow_mut()
                        .insert(widget_key(&handle), p.document_keys.to_vec());
                });
                NAV_MENU_ROWS.with(|m| {
                    m.borrow_mut()
                        .insert(widget_key(&handle), (id, p.items.clone(), p.icons.clone()))
                });
                NAV_MENUS.with(|m| {
                    m.borrow_mut().insert(
                        widget_key(&handle),
                        NavMenuState {
                            listbox,
                            rows: p.items.len(),
                            suppress,
                            current,
                        },
                    )
                });
                handle
            }
            Some(Builtin::Scroll) => {
                let horizontal = props
                    .downcast_ref::<day_spec::props::ScrollProps>()
                    .map(|p| p.horizontal)
                    .unwrap_or(false);
                let sw = gtk4::ScrolledWindow::new();
                if horizontal {
                    sw.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Never);
                } else {
                    sw.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
                }
                sw.set_child(Some(&gtk4::Fixed::new()));
                // Transparent like AppKit's `setDrawsBackground(false)` scroll: content layered
                // Behind the viewport (e.g. a gradient backdrop in a zstack) must show through.
                scroll_transparent_css();
                sw.add_css_class("day-scroll");
                sw.upcast()
            }
            Some(Builtin::Label) => {
                let Some(p) = props_of::<LabelProps>(kind, "gtk", props) else {
                    return placeholder_label(kind);
                };
                let label = gtk4::Label::new(Some(&p.text));
                // Both halves of alignment. `xalign` places the text block inside the label's
                // allocation; `justify` aligns the WRAPPED LINES against each other. A centered
                // paragraph needs the second — with xalign alone the block sits centered while
                // its lines stay ragged-right, which is not what `TextAlign::Center` means.
                let (xalign, justify) = match p.align {
                    day_spec::props::TextAlign::Center => (0.5, gtk4::Justification::Center),
                    day_spec::props::TextAlign::Trailing => (1.0, gtk4::Justification::Right),
                    day_spec::props::TextAlign::Leading => (0.0, gtk4::Justification::Left),
                };
                label.set_xalign(xalign);
                label.set_justify(justify);
                label.set_yalign(0.0);
                label.set_wrap(p.wraps);
                if !p.wraps {
                    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                }
                label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
                update_text_attrs(&label, Some(p.font), Some(p.color));
                // GTK ships the de-emphasized look as a style class, so the theme decides the
                // actual color and it follows light/dark the way the rest of the window does —
                // which is the whole reason for asking by role rather than naming a grey.
                if p.role == day_spec::props::TextRole::Secondary {
                    use gtk4::prelude::*;
                    label.add_css_class("dim-label");
                }
                set_label_runs(&label, &p.text, &p.runs);
                // A link run is an `<a href>` in the markup, and GtkLabel hit-tests those itself
                // (Cap::TextLinks). Stopping the signal keeps GTK from also handing the URI to
                // the desktop's URL handler — day-core decides that, from the app's `.on_link()`.
                label.connect_activate_link(move |_, uri| {
                    emit(id, Event::LinkActivated(uri.to_string()));
                    gtk4::glib::Propagation::Stop
                });
                label.upcast()
            }
            Some(Builtin::Button) => {
                let Some(p) = props_of::<ButtonProps>(kind, "gtk", props) else {
                    return placeholder_label(kind);
                };
                let btn = gtk4::Button::with_label(&p.title);
                apply_button_style(&btn, p.style);
                if p.icon.is_some() {
                    apply_button_content(&btn, &p.title, p.icon.as_ref(), p.icon_only);
                }
                btn.set_sensitive(p.enabled);
                set_button_selected(&btn, p.selected);
                btn.connect_clicked(move |_| ffi_guard::contain((), || emit(id, Event::Pressed)));
                wire_focus(&btn, id);
                btn.upcast()
            }
            Some(Builtin::Toggle) => {
                let Some(p) = props_of::<ToggleProps>(kind, "gtk", props) else {
                    return placeholder_label(kind);
                };
                let sw = gtk4::Switch::new();
                sw.set_active(p.on);
                sw.set_sensitive(p.enabled);
                sw.connect_active_notify(move |s| {
                    ffi_guard::contain((), || emit(id, Event::ToggleChanged(s.is_active())))
                });
                wire_focus(&sw, id);
                sw.upcast()
            }
            Some(Builtin::Slider) => {
                let Some(p) = props_of::<SliderProps>(kind, "gtk", props) else {
                    return placeholder_label(kind);
                };
                let step = p.step.unwrap_or((p.max - p.min) / 1000.0).max(1e-9);
                let scale =
                    gtk4::Scale::with_range(gtk4::Orientation::Horizontal, p.min, p.max, step);
                scale.set_value(p.value);
                scale.set_draw_value(false);
                scale.connect_value_changed(move |s| {
                    ffi_guard::contain((), || emit(id, Event::ValueChanged(s.value())))
                });
                // GtkScale has no "drag ended" signal — `value-changed` fires on every motion
                // step — so the settled value comes from the interactions that END one: the
                // pointer coming up, and a key being released after an arrow/Page step. Both
                // controllers run in the CAPTURE phase so the scale's own internal gesture
                // (which claims the sequence) cannot swallow them first.
                let released = gtk4::GestureClick::new();
                released.set_propagation_phase(gtk4::PropagationPhase::Capture);
                {
                    let scale = scale.clone();
                    released.connect_released(move |_, _n, _x, _y| {
                        ffi_guard::contain((), || emit(id, Event::ValueCommitted(scale.value())))
                    });
                }
                scale.add_controller(released);
                let keys = gtk4::EventControllerKey::new();
                keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
                {
                    let scale = scale.clone();
                    keys.connect_key_released(move |_, _k, _c, _m| {
                        ffi_guard::contain((), || emit(id, Event::ValueCommitted(scale.value())))
                    });
                }
                scale.add_controller(keys);
                wire_focus(&scale, id);
                scale.upcast()
            }
            Some(Builtin::Picker) => picker::realize_any(self, props, id),
            Some(Builtin::TextArea) => textarea::realize_any(self, props, id),
            Some(Builtin::TextField) => {
                let Some(p) = props_of::<TextFieldProps>(kind, "gtk", props) else {
                    return placeholder_label(kind);
                };
                let entry = gtk4::Entry::new();
                entry.set_text(&p.text);
                entry.set_placeholder_text(Some(&p.placeholder));
                entry.connect_changed(move |e| {
                    ffi_guard::contain((), || emit(id, Event::TextChanged(e.text().to_string())))
                });
                entry.connect_activate(move |_| {
                    ffi_guard::contain((), || emit(id, Event::Submitted))
                });
                wire_focus(&entry, id);
                entry.upcast()
            }
            Some(Builtin::Divider) => {
                let vertical =
                    props_of::<DividerProps>(kind, "gtk", props).is_some_and(|p| p.vertical);
                let orientation = if vertical {
                    gtk4::Orientation::Vertical
                } else {
                    gtk4::Orientation::Horizontal
                };
                gtk4::Separator::new(orientation).upcast()
            }
            Some(Builtin::Progress) => {
                let Some(p) = props_of::<ProgressProps>(kind, "gtk", props) else {
                    return placeholder_label(kind);
                };
                if p.value.is_none() {
                    return crate::controls::spinner::make(self, p, id);
                }
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
            Some(Builtin::Canvas) => {
                let area = gtk4::DrawingArea::new();
                area.set_draw_func(|area, cr, _w, _h| {
                    // Contained: the ops decode runs app-supplied data inside GTK's C draw
                    // dispatch, where an unwound panic is an abort.
                    ffi_guard::contain((), || {
                        let ptr = area.as_ptr() as usize;
                        let (nums, texts) = OPS.with(|t| t.get(ptr)).unwrap_or_default();
                        cairo_draw(cr, &nums, &texts);
                    });
                });
                // Focus, and with it the keyboard (docs/menus.md). A DrawingArea is not
                // focusable by default — nothing would ever make it the focus widget — so
                // arrow keys aimed at what an app DRAWS need this and the click below.
                area.set_focusable(true);
                area.set_can_focus(true);
                wire_focus(&area, id);
                let click = gtk4::GestureClick::new();
                click.connect_pressed(|g, _, _, _| {
                    ffi_guard::contain((), || {
                        if let Some(w) = g.widget() {
                            w.grab_focus();
                        }
                    })
                });
                area.add_controller(click);
                let keys = gtk4::EventControllerKey::new();
                keys.connect_key_pressed(move |_, key, _, state| {
                    ffi_guard::contain(gtk4::glib::Propagation::Proceed, || {
                        let Some(name) = key_name(key, state) else {
                            return gtk4::glib::Propagation::Proceed;
                        };
                        // An arrow nobody asked for keeps traveling, so a scrolled window
                        // around the canvas still scrolls with the keyboard.
                        if !day_spec::keys::handled(id) {
                            return gtk4::glib::Propagation::Proceed;
                        }
                        emit(
                            id,
                            Event::Key(day_spec::KeyEvent {
                                key: name.to_string(),
                                modifiers: key_modifiers(state),
                            }),
                        );
                        gtk4::glib::Propagation::Stop
                    })
                });
                area.add_controller(keys);
                area.upcast()
            }
            Some(Builtin::Tree) => {
                let Some(p) = props_of::<TreeProps>(kind, "gtk", props) else {
                    return placeholder_label(kind);
                };
                let source: Rc<RefCell<Option<TreeSource>>> = Rc::new(RefCell::new(None));
                let suppress = Rc::new(std::cell::Cell::new(false));
                let factory = gtk4::SignalListItemFactory::new();
                factory.connect_setup(|_, item| {
                    if let Some(li) = item.downcast_ref::<gtk4::ListItem>() {
                        // The expander draws the indent + arrow and wraps the day cell.
                        let expander = gtk4::TreeExpander::new();
                        expander.set_indent_for_icon(true);
                        // A `DayCell`, as for list rows: it asks for no width and lays Day's row
                        // at the width the expander leaves after the indentation, through
                        // `TreeSource::layout_cell` — so an indented row fits its cell instead of
                        // overflowing it. The expander's box gives a child only its natural width
                        // unless it expands, and this cell's natural width is nothing.
                        let cell = DayCell::new();
                        cell.set_hexpand(true);
                        cell.set_overflow(gtk4::Overflow::Hidden);
                        expander.set_child(Some(&cell));
                        li.set_child(Some(&expander));
                    }
                });
                factory.connect_bind({
                    let source = source.clone();
                    let suppress = suppress.clone();
                    // Contained: bind_row runs day-core (and the app's row builder).
                    move |_, item| {
                        ffi_guard::contain((), || {
                            let Some(li) = item.downcast_ref::<gtk4::ListItem>() else {
                                return;
                            };
                            let Some(row) = li
                                .item()
                                .and_then(|o| o.downcast::<gtk4::TreeListRow>().ok())
                            else {
                                return;
                            };
                            let Some(expander) = li
                                .child()
                                .and_then(|c| c.downcast::<gtk4::TreeExpander>().ok())
                            else {
                                return;
                            };
                            expander.set_list_row(Some(&row));
                            let Some(tok) = tree_row_token(&row) else {
                                return;
                            };
                            let heading = source
                                .borrow()
                                .as_ref()
                                .is_some_and(|s| (s.section_header)(tok));
                            li.set_selectable(!heading);
                            li.set_activatable(!heading);
                            // The user's disclosure click reports through the row's expanded
                            // property; a programmatic restore is suppressed. Held only
                            // while bound (see unbind).
                            {
                                let suppress = suppress.clone();
                                let handler = row.connect_expanded_notify(move |r| {
                                    ffi_guard::contain((), || {
                                        if suppress.get() {
                                            return;
                                        }
                                        let Some(tok) = tree_row_token(r) else { return };
                                        // The record lives with the entry, found via the
                                        // expander's scroller at event time (the closure
                                        // must not hold the entry — Rc cycle).
                                        TREE_STATE.with(|m| {
                                            for e in m.borrow().values() {
                                                if e.listview.model().is_some_and(|mm| {
                                                    mm.item(r.position()).is_some_and(|it| {
                                                        it.downcast_ref::<gtk4::TreeListRow>()
                                                            .is_some_and(|rr| rr == r)
                                                    })
                                                }) {
                                                    if r.is_expanded() {
                                                        e.expanded.borrow_mut().insert(tok);
                                                    } else {
                                                        e.expanded.borrow_mut().remove(&tok);
                                                    }
                                                    emit(
                                                        e.node,
                                                        Event::TreeExpanded {
                                                            token: tok,
                                                            expanded: r.is_expanded(),
                                                        },
                                                    );
                                                    break;
                                                }
                                            }
                                        });
                                    });
                                });
                                TREE_ROW_HANDLERS.with(|t| {
                                    if let Some((old_row, old_h)) = t
                                        .borrow_mut()
                                        .insert(li.as_ptr() as usize, (row.clone(), handler))
                                    {
                                        old_row.disconnect(old_h);
                                    }
                                });
                            }
                            if let Some(cell) = expander.child()
                                && let Some(src) = source.borrow().as_ref()
                            {
                                TREE_CELL_TOKENS.with(|m| {
                                    m.borrow_mut().insert(widget_key(cell.upcast_ref()), tok)
                                });
                                (src.bind_row)(tok, cell.as_ptr() as RawHandle);
                                // `bind_row` laid the row at the TREE's width; the cell knows
                                // the width the expander actually left it and re-lays to that.
                                if let Ok(cell) = cell.downcast::<DayCell>() {
                                    cell.bind_layout(
                                        src.layout_cell.clone(),
                                        src.cell_height.clone(),
                                    );
                                }
                            }
                        });
                    }
                });
                factory.connect_unbind({
                    let source = source.clone();
                    move |_, item| {
                        if let Some(li) = item.downcast_ref::<gtk4::ListItem>() {
                            TREE_ROW_HANDLERS.with(|t| {
                                if let Some((row, h)) =
                                    t.borrow_mut().remove(&(li.as_ptr() as usize))
                                {
                                    row.disconnect(h);
                                }
                            });
                            if let Some(expander) = li
                                .child()
                                .and_then(|c| c.downcast::<gtk4::TreeExpander>().ok())
                            {
                                expander.set_list_row(None::<&gtk4::TreeListRow>);
                                // The cell left its row (a collapse, a model swap): clear
                                // its day element ids so a hidden row stops answering
                                // `find_by_id` — the next bind re-sets the live row's
                                // (docs/tree.md; the same rule AppKit wires through
                                // didRemoveRowView).
                                if let (Some(cell), Some(src)) =
                                    (expander.child(), source.borrow().as_ref())
                                {
                                    TREE_CELL_TOKENS.with(|m| {
                                        m.borrow_mut().remove(&widget_key(cell.upcast_ref()))
                                    });
                                    ffi_guard::contain((), || {
                                        (src.recycle)(cell.as_ptr() as RawHandle);
                                    });
                                }
                            }
                        }
                    }
                });
                // No model until the first rebuild fills one from the injected source.
                let listview = gtk4::ListView::new(None::<gtk4::SelectionModel>, Some(factory));
                listview.set_single_click_activate(false);
                listview.connect_activate(move |_, row| {
                    ffi_guard::contain((), || emit(id, Event::ListActivated(row as usize)))
                });
                day_list_rows(&listview);
                // Summon-time ROW context menus (docs/menus.md, docs/tree.md): right-click
                // (and long-press) picks the row under the pointer and asks the tree's
                // `row_menu` provider.
                {
                    let show = {
                        let (lv, source) = (listview.clone(), source.clone());
                        move |x: f64, y: f64| {
                            let row_menu =
                                source.borrow().as_ref().and_then(|s| s.row_menu.clone());
                            let Some(row_menu) = row_menu else { return };
                            let Some(mut picked) = lv.pick(x, y, gtk4::PickFlags::DEFAULT) else {
                                return;
                            };
                            let token = loop {
                                if let Some(tok) = TREE_CELL_TOKENS
                                    .with(|m| m.borrow().get(&widget_key(&picked)).copied())
                                {
                                    break Some(tok);
                                }
                                match picked.parent() {
                                    Some(p) => picked = p,
                                    None => break None,
                                }
                            };
                            let Some(token) = token else { return };
                            // Guarded: the provider is app code.
                            ffi_guard::contain((), || {
                                let items = row_menu(token);
                                if !items.is_empty() {
                                    show_menu_popover(&lv.clone().upcast(), &items, x, y);
                                }
                            });
                        }
                    };
                    let click = gtk4::GestureClick::new();
                    click.set_button(3);
                    let f = show.clone();
                    click.connect_pressed(move |_, _n, x, y| f(x, y));
                    listview.add_controller(click);
                    let long = gtk4::GestureLongPress::new();
                    let f = show.clone();
                    long.connect_pressed(move |_, x, y| f(x, y));
                    listview.add_controller(long);
                }
                let sw = gtk4::ScrolledWindow::new();
                sw.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
                sw.set_child(Some(&listview));
                sw.set_vexpand(true);
                let host: Handle = sw.upcast();
                TREE_STATE.with(|m| {
                    m.borrow_mut().insert(
                        widget_key(&host),
                        Rc::new(TreeEntry {
                            node: id,
                            listview,
                            source,
                            expanded: Rc::new(RefCell::new(std::collections::HashSet::new())),
                            selected: Rc::new(RefCell::new(Vec::new())),
                            suppress,
                            multi: p.multi_select,
                        }),
                    );
                });
                host
            }
            Some(Builtin::List) => {
                let Some(p) = props_of::<ListProps>(kind, "gtk", props) else {
                    return placeholder_label(kind);
                };
                let model = gtk4::StringList::new(&[]);
                let factory = gtk4::SignalListItemFactory::new();
                // The reorder DragSource needs the host list's key, which doesn't exist until
                // the ScrolledWindow below is created — carry it through a shared cell.
                let host_key: Rc<std::cell::Cell<usize>> = Rc::new(std::cell::Cell::new(0));
                let reorderable = p.reorderable;
                // Each list item's cell, by the item: GTK detaches the child before it emits
                // `teardown`, so the teardown handler finds it here, not on the item.
                let cells: Rc<RefCell<HashMap<usize, DayCell>>> = Rc::default();
                factory.connect_setup({
                    let host_key = host_key.clone();
                    let cells = cells.clone();
                    move |_, item| {
                        if let Some(li) = item.downcast_ref::<gtk4::ListItem>() {
                            // Each physical cell is a `DayCell` — a GtkFixed that asks for no width
                            // and lays Day's row out at whatever the row grants it, so the cell can
                            // never drag the list wider than its frame (day#35). Day fills it via
                            // bind_row as before.
                            let cell = DayCell::new();
                            cells
                                .borrow_mut()
                                .insert(li.as_ptr() as usize, cell.clone());
                            // Clipped, not visible: a row is laid at the list's width for the
                            // instant between `bind_row` and the correction, and that must not
                            // paint over the pane next door.
                            cell.set_overflow(gtk4::Overflow::Hidden);
                            if reorderable {
                                // Native GTK drag (docs/list.md): the drag carries the row it
                                // left from; the icon is the row itself (a WidgetPaintable).
                                let drag = gtk4::DragSource::new();
                                drag.set_actions(gtk4::gdk::DragAction::MOVE);
                                drag.connect_prepare({
                                    let cell = cell.clone();
                                    let host_key = host_key.clone();
                                    move |ds, x, y| {
                                        let row = LIST_CELL_ROWS
                                            .with(|t| t.get(cell.as_ptr() as usize))?;
                                        // A section header never lifts (docs/list.md
                                        // "Sections"); its cell may hold an item row later.
                                        let header = LIST_STATE.with(|m| {
                                            let m = m.borrow();
                                            let source = m.get(&host_key.get())?.source.borrow();
                                            source.as_ref()?.is_header.clone()
                                        });
                                        if header.is_some_and(|is_header| is_header(row)) {
                                            return None;
                                        }
                                        DRAG_FROM.with(|d| d.set(Some((host_key.get(), row))));
                                        ds.set_icon(
                                            Some(&gtk4::WidgetPaintable::new(Some(&cell))),
                                            x as i32,
                                            y as i32,
                                        );
                                        Some(gtk4::gdk::ContentProvider::for_value(
                                            &(row as u64).to_value(),
                                        ))
                                    }
                                });
                                drag.connect_drag_end(|_, _, _| {
                                    DRAG_FROM.with(|d| d.set(None));
                                });
                                cell.add_controller(drag);
                            }
                            li.set_child(Some(&cell));
                        }
                    }
                });
                let source: Rc<RefCell<Option<ListSource>>> = Rc::new(RefCell::new(None));
                let bound: BoundCells = Rc::default();
                factory.connect_bind({
                    let source = source.clone();
                    let bound = bound.clone();
                    // Contained: bind_row runs day-core (and through it the app's row builder).
                    move |_, item| {
                        ffi_guard::contain((), || {
                            let Some(li) = item.downcast_ref::<gtk4::ListItem>() else {
                                return;
                            };
                            let pos = li.position() as usize;
                            if let Some(cell) = li.child()
                                && let Some(src) = source.borrow().as_ref()
                            {
                                LIST_CELL_ROWS.with(|t| t.insert(cell.as_ptr() as usize, pos));
                                bound
                                    .borrow_mut()
                                    .insert(cell.as_ptr() as usize, li.downgrade());
                                // A sectioned list's header rows (docs/list.md "Sections").
                                // GtkListView pools one item type, so a recycled item can turn
                                // from a header into a row and back: mark it on every bind.
                                if let Some(is_header) = &src.is_header {
                                    mark_list_header(li, is_header(pos));
                                }
                                (src.bind_row)(pos, cell.as_ptr() as RawHandle);
                                // `bind_row` laid the row at the LIST's width; the cell knows the
                                // width its row actually granted and re-lays the content to that.
                                if let Ok(cell) = cell.downcast::<DayCell>() {
                                    cell.bind_layout(
                                        src.layout_cell.clone(),
                                        src.cell_height.clone(),
                                    );
                                }
                            }
                        });
                    }
                });
                factory.connect_unbind({
                    let source = source.clone();
                    let bound = bound.clone();
                    // The cell left its row (a source that shrank, a scroll): clear its day
                    // element ids so a hidden row stops answering lookups. The next bind
                    // restores the live row's (docs/list.md; the tree's unbind does the same).
                    move |_, item| {
                        ffi_guard::contain((), || {
                            let Some(li) = item.downcast_ref::<gtk4::ListItem>() else {
                                return;
                            };
                            if let Some(cell) = li.child() {
                                bound.borrow_mut().remove(&(cell.as_ptr() as usize));
                            }
                            if let Some(cell) = li.child()
                                && let Some(src) = source.borrow().as_ref()
                            {
                                (src.recycle)(cell.as_ptr() as RawHandle);
                            }
                        });
                    }
                });
                factory.connect_teardown({
                    let source = source.clone();
                    let bound = bound.clone();
                    let cells = cells.clone();
                    // GTK is done with this list item for good (a far jump, a shrink, the
                    // list going away): its `DayCell` goes with it, so release the row Day
                    // built there, or that row's scope, nodes and ids outlive the cell in
                    // day-core, one dead row per discarded cell (docs/list.md). day-core
                    // holds the cell until the release runs, so the pointer stays this cell's;
                    // a teardown inside a day-core borrow (a reload or a removal Day started)
                    // is held for the borrow's end rather than dropped.
                    move |_, item| {
                        ffi_guard::contain((), || {
                            let Some(li) = item.downcast_ref::<gtk4::ListItem>() else {
                                return;
                            };
                            let Some(cell) = cells.borrow_mut().remove(&(li.as_ptr() as usize))
                            else {
                                return;
                            };
                            let key = cell.as_ptr() as usize;
                            bound.borrow_mut().remove(&key);
                            LIST_CELL_ROWS.with(|t| {
                                t.remove(key);
                            });
                            if li.child().is_some() {
                                li.set_child(None::<&gtk4::Widget>);
                            }
                            if let Some(src) = source.borrow().as_ref() {
                                (src.release)(cell.as_ptr() as RawHandle);
                            }
                        });
                    }
                });
                let selection = p.selectable.then(|| ListSelection {
                    model: gtk4::SingleSelection::new(Some(model.clone())),
                    requested: Rc::new(std::cell::Cell::new(gtk4::INVALID_LIST_POSITION)),
                    suppress: Rc::new(std::cell::Cell::new(false)),
                });
                let listview = if let Some(selection) = &selection {
                    let sel = &selection.model;
                    sel.set_autoselect(false);
                    sel.set_can_unselect(true);
                    let suppress = selection.suppress.clone();
                    let requested = selection.requested.clone();
                    let header_guard = selection.clone();
                    let source = source.clone();
                    sel.connect_selected_notify(move |s| {
                        if suppress.get() {
                            return;
                        }
                        ffi_guard::contain((), || {
                            let i = s.selected();
                            // A header is never selected (docs/list.md "Sections"). Its item
                            // is unselectable, but keyboard navigation on GTK before 4.12
                            // (no item `focusable`) can still land on it: put the row the
                            // selection held back, past this notify.
                            let header = i != gtk4::INVALID_LIST_POSITION
                                && source.borrow().as_ref().is_some_and(|src| {
                                    src.is_header.as_ref().is_some_and(|f| f(i as usize))
                                });
                            if header {
                                let selection = header_guard.clone();
                                gtk4::glib::idle_add_local_once(move || {
                                    selection.suppress.set(true);
                                    selection.apply();
                                    selection.suppress.set(false);
                                });
                                return;
                            }
                            requested.set(i);
                            emit(
                                id,
                                Event::SelectionChanged(if i == gtk4::INVALID_LIST_POSITION {
                                    -1
                                } else {
                                    i as i64
                                }),
                            );
                        });
                    });
                    gtk4::ListView::new(Some(sel.clone()), Some(factory))
                } else {
                    gtk4::ListView::new(
                        Some(gtk4::NoSelection::new(Some(model.clone()))),
                        Some(factory),
                    )
                };
                day_list_rows(&listview);
                // Activation (docs/list.md): the view's own `activate` — a double-click or
                // Enter on a row — reaches the app as `ListActivated`, as the tree's does.
                listview.set_single_click_activate(false);
                listview.connect_activate(move |_, row| {
                    ffi_guard::contain((), || emit(id, Event::ListActivated(row as usize)))
                });
                // Host-drawn row separators (docs/list.md): a border on the ListView's own
                // `row` CSS nodes, which sit exactly at the row boundary — aligned with the
                // native selection. One global provider serves every separated list.
                if p.separators == Some(true) {
                    thread_local! {
                        static SEP_CSS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
                    }
                    SEP_CSS.with(|done| {
                        if !done.get()
                            && let Some(display) = gtk4::gdk::Display::default()
                        {
                            let provider = gtk4::CssProvider::new();
                            provider.load_from_data(
                                "listview.day-separators > row { \
                                     border-bottom: 1px solid alpha(currentColor, 0.18); \
                                 }",
                            );
                            gtk4::style_context_add_provider_for_display(
                                &display,
                                &provider,
                                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
                            );
                            done.set(true);
                        }
                    });
                    listview.add_css_class("day-separators");
                }
                let sw = gtk4::ScrolledWindow::new();
                sw.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
                sw.set_child(Some(&listview));
                sw.set_vexpand(true);
                let vadj = sw.vadjustment();
                {
                    let source = source.clone();
                    let pitch = match p.row_height {
                        RowHeight::Uniform(height) => height,
                        RowHeight::Automatic => 44.0,
                    }
                    .max(1.0);
                    // A sectioned list's headers are not one pitch tall: its `PinnedHeader`
                    // reports the first visible row from the laid rows instead.
                    if !p.sections {
                        vadj.connect_value_changed(move |adjustment| {
                            let source = source.borrow().clone();
                            if let Some(source) = source
                                && let Some(report) = &source.first_visible
                            {
                                report(
                                    ((adjustment.value().max(0.0) / pitch) as usize)
                                        .min((source.len)().saturating_sub(1)),
                                );
                            }
                        });
                    }
                }
                // A sectioned list pins its current header over the rows (docs/list.md
                // "Sections"): the host is the overlay that holds the copy above the scroller.
                let pinned = p
                    .sections
                    .then(|| PinnedHeader::new(&sw, &listview, source.clone(), bound.clone()));
                let host: Handle = match &pinned {
                    Some(pinned) => pinned.overlay.clone().upcast(),
                    None => sw.upcast(),
                };
                host_key.set(widget_key(&host));
                if p.reorderable {
                    // The drop half (docs/list.md): every hovered slot is vetted through the
                    // app's guard — a denied slot answers no action, so GTK shows the forbidden
                    // cursor live; the drop commits through the sync seam and re-binds.
                    let (row_h, automatic) = match p.row_height {
                        RowHeight::Uniform(h) => (h, false),
                        RowHeight::Automatic => (44.0, true),
                    };
                    let slot_at = {
                        let source = source.clone();
                        let bound = bound.clone();
                        let view = listview.downgrade();
                        move |y: f64| {
                            let n = source.borrow().as_ref().map(|s| (s.len)()).unwrap_or(0);
                            if n == 0 {
                                return None;
                            }
                            // Self-sizing rows have no pitch to divide by: the slot is the bound
                            // row under the pointer, its themed row widget's bounds in the
                            // view's own (scrolled) coordinates, the same as `y`.
                            if automatic && let Some(view) = view.upgrade() {
                                let hit = bound.borrow().values().find_map(|li| {
                                    let li = li.upgrade()?;
                                    let row = li.child()?.parent()?;
                                    let b = row.compute_bounds(&view)?;
                                    let (top, bottom) =
                                        (f64::from(b.y()), f64::from(b.y() + b.height()));
                                    (y >= top && y < bottom).then(|| li.position() as usize)
                                });
                                if let Some(row) = hit {
                                    return Some(row.min(n - 1));
                                }
                            }
                            let abs = y + vadj.value();
                            Some(((abs / row_h) as usize).min(n - 1))
                        }
                    };
                    let key = widget_key(&host);
                    let verdict = {
                        let source = source.clone();
                        let slot_at = slot_at.clone();
                        move |y: f64| -> Option<usize> {
                            let (from_key, from) = DRAG_FROM.with(|d| d.get())?;
                            if from_key != key {
                                return None; // a drag from some other list — never accepted
                            }
                            let slot = slot_at(y)?;
                            let r = source.borrow().as_ref().and_then(|s| s.reorder.clone())?;
                            let accepted = (r.can_move)(from, slot);
                            (accepted >= 0).then_some(accepted as usize)
                        }
                    };
                    let dt = gtk4::DropTarget::new(
                        gtk4::glib::types::Type::U64,
                        gtk4::gdk::DragAction::MOVE,
                    );
                    dt.connect_motion({
                        let verdict = verdict.clone();
                        // Contained with "no action": the verdict runs the app's can_move guard,
                        // and denying the slot is the safe answer to a panic there.
                        move |_, _x, y| {
                            ffi_guard::contain(gtk4::gdk::DragAction::empty(), || {
                                match verdict(y) {
                                    Some(_) => gtk4::gdk::DragAction::MOVE,
                                    None => gtk4::gdk::DragAction::empty(),
                                }
                            })
                        }
                    });
                    dt.connect_drop({
                        let source = source.clone();
                        let model = model.clone();
                        let selection = selection.clone();
                        let bound = bound.clone();
                        // Contained as a refused drop: move_row is app code.
                        move |_, _value, _x, y| {
                            ffi_guard::contain(false, || {
                                let Some((_, from)) = DRAG_FROM.with(|d| d.get()) else {
                                    return false;
                                };
                                let Some(accepted) = verdict(y) else {
                                    return false;
                                };
                                if accepted != from
                                    && let Some(r) =
                                        source.borrow().as_ref().and_then(|s| s.reorder.clone())
                                {
                                    (r.move_row)(from, accepted);
                                    // Re-bind the visible cells in the new order (same count).
                                    schedule_list_resize(
                                        model.clone(),
                                        source.clone(),
                                        selection.clone(),
                                        bound.clone(),
                                    );
                                }
                                true
                            })
                        }
                    });
                    listview.add_controller(dt);
                }
                LIST_STATE.with(|m| {
                    m.borrow_mut().insert(
                        widget_key(&host),
                        ListEntry {
                            model,
                            source,
                            selection,
                            bound,
                            pinned,
                        },
                    )
                });
                host
            }
            Some(Builtin::Image) => {
                let Some(p) = props_of::<ImageProps>(kind, "gtk", props) else {
                    return placeholder_label(kind);
                };
                let pic = gtk4::Picture::new();
                // Scaling (§18.3): GtkPicture content-fit — Contain (fit) / Cover (fill) / Fill.
                pic.set_content_fit(match p.content_mode {
                    ContentMode::Fit => gtk4::ContentFit::Contain,
                    ContentMode::Fill => gtk4::ContentFit::Cover,
                    ContentMode::Stretch => gtk4::ContentFit::Fill,
                });
                // Vector-glyph tint (docs/vectors.md): recolor every pixel to the tint,
                // keeping alpha as the mask — same recolor the sidebar template icons use.
                // Bytes and shared decodes arrive through `decode_image` (docs/images.md); this
                // arm is the staged-asset path, unchanged.
                // Only a NAMED source can be tinted: a recolor re-reads the file, and bytes have
                // no file to re-read. The name is recorded either way so a later tint patch
                // knows whether it has one.
                let named = match &p.source {
                    day_spec::ImageSource::Named(name) => name.clone(),
                    _ => String::new(),
                };
                IMAGE_SOURCE.with(|t| {
                    t.insert(
                        widget_key(pic.upcast_ref()),
                        ImageEntry {
                            name: named.clone(),
                            tint: p.tint,
                            template: p.template,
                            picture: pic.downgrade(),
                        },
                    )
                });
                // A template with no tint (docs/vectors.md "Template") recolors to the theme
                // foreground, as the sidebar's own icons do, and is re-painted on a switch.
                if !named.is_empty() {
                    repaint_image(&pic, &named, p.tint, p.template);
                } else {
                    set_picture_source(&pic, &p.source);
                }
                pic.upcast()
            }
            Some(Builtin::ListItem) => match props.downcast_ref::<day_spec::props::ListItemProps>()
            {
                Some(p) => make_list_item(p),
                None => placeholder_label(kind),
            },
            // A recycled list cell is ADOPTED from the native list, never realized
            // through this path; anything else is an extension piece.
            Some(Builtin::ListCell) | None => {
                if let Some(make) = self.registry.get(kind).map(|r| r.make) {
                    return make(self, props, id);
                }
                warn_missing_renderer(kind);
                placeholder_label(kind)
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
            kinds::LIST_ITEM => {
                let Some(p) = patch.downcast_ref::<day_spec::props::ListItemPatch>() else {
                    return;
                };
                let props = LIST_ITEMS.with(|m| {
                    let mut m = m.borrow_mut();
                    let props = m.get_mut(&widget_key(h))?;
                    use day_spec::props::ListItemPatch as P;
                    match p {
                        P::Title(t) => props.title = t.clone(),
                        P::Subtitle(t) => props.subtitle = t.clone(),
                        P::Detail(t) => props.detail = t.clone(),
                        P::Icon(i) => props.icon = i.clone(),
                        P::IconTint(c) => props.icon_tint = *c,
                        P::Checked(on) => props.checked = *on,
                    }
                    Some(props.clone())
                });
                if let Some(props) = props {
                    dress_list_item(h, &props);
                }
            }
            kinds::SPLIT => {
                if let Some(p) = patch.downcast_ref::<SplitPatch>()
                    && let Some(state) = SPLIT_STATE.with(|t| t.get(widget_key(h)))
                {
                    let mut st = state.borrow_mut();
                    match p {
                        SplitPatch::Fraction(f) => {
                            st.share = f.clamp(0.0, 1.0);
                            if st.placed {
                                st.place();
                            }
                        }
                        SplitPatch::Axis(axis) => {
                            st.paned.set_orientation(
                                if *axis == day_spec::props::SplitAxis::Vertical {
                                    gtk4::Orientation::Vertical
                                } else {
                                    gtk4::Orientation::Horizontal
                                },
                            );
                            st.request_minimums();
                            if st.placed {
                                st.place();
                            }
                        }
                    }
                }
            }
            kinds::INSPECTOR => {
                if let Some(InspectorPatch::Visible(v)) = patch.downcast_ref::<InspectorPatch>() {
                    let key = widget_key(h);
                    if let Some(state) = INSPECTOR_STATE.with(|t| t.get(key)) {
                        // Applied at once, no transition: a dayscript screenshot right after
                        // a toggle must see the settled shape (AppKit's no-animator rule).
                        // Nothing native hides this panel, so there is no echo to suppress:
                        // `Event::InspectorChanged` never originates here.
                        let mut state = state.borrow_mut();
                        state.shown = *v;
                        state.panel.set_visible(*v);
                        if *v {
                            state.place(h.width());
                        }
                    }
                    gtk4::glib::idle_add_local_once(move || {
                        ffi_guard::contain((), || inspector_report(key))
                    });
                }
            }
            // Emulated cover (docs/cover.md): present = re-home onto the window's root Fixed
            // at the content size, topmost; dismiss = hide + report `CoverHidden` at once (no
            // transition on this tier). No interactive dismissal exists on this backend.
            kinds::COVER => {
                if let (Some(p), Ok(cover)) = (
                    patch.downcast_ref::<CoverPatch>(),
                    h.clone().downcast::<gtk4::Fixed>(),
                ) {
                    let node = COVER_IDS
                        .with(|t| t.get(widget_key(h)))
                        .unwrap_or(day_spec::WINDOW_NODE);
                    match p {
                        CoverPatch::Present { background, .. } => {
                            // Occlude the window: an explicit color via the surface provider,
                            // else Adwaita's `background` class (the theme window background).
                            // The widget outlives one presentation, and a composed sheet's cover
                            // is clear while a full-window one is not, so each present resets
                            // whichever treatment the last one left.
                            match background {
                                Some(_) => {
                                    cover.remove_css_class("background");
                                    apply_surface(h, *background, 0.0, false);
                                }
                                None => {
                                    apply_surface(h, None, 0.0, false);
                                    cover.add_css_class("background");
                                }
                            }
                            if let Some(parent) = cover.parent()
                                && let Ok(old) = parent.downcast::<gtk4::Fixed>()
                            {
                                old.remove(&cover);
                            }
                            if let Some(root) = self.window_fixed.as_ref() {
                                root.put(&cover, 0.0, 0.0);
                                // The filling wrapper owns the content allocation; GtkFixed's
                                // own minimum can grow with children outside the viewport.
                                let area = root.parent().unwrap_or_else(|| root.clone().upcast());
                                let size = Size::new(area.width() as f64, area.height() as f64);
                                let (y, size) = cover_frame(&cover, size);
                                root.move_(&cover, 0.0, y);
                                cover.set_size_request(size.width as i32, size.height as i32);
                                cover.set_visible(true);
                                COVERS.with(|c| {
                                    c.borrow_mut().push((cover.clone(), node));
                                });
                                emit(node, Event::FrameChanged(size));
                            }
                        }
                        CoverPatch::DismissDisabled(_) => {}
                        CoverPatch::Dismiss => {
                            cover.set_visible(false);
                            if let Some(parent) = cover.parent()
                                && let Ok(old) = parent.downcast::<gtk4::Fixed>()
                            {
                                old.remove(&cover);
                            }
                            COVERS.with(|c| {
                                c.borrow_mut().retain(|(w, _)| w != &cover);
                            });
                            emit(node, Event::CoverHidden);
                        }
                    }
                }
            }
            kinds::IMAGE => {
                if let (Some(p), Some(pic)) = (
                    patch.downcast_ref::<day_spec::props::ImagePatch>(),
                    h.downcast_ref::<gtk4::Picture>(),
                ) {
                    match p {
                        day_spec::props::ImagePatch::Tint(c) => {
                            // Back to the authored colors when the tint goes — or, for a
                            // template glyph, to the theme foreground.
                            let entry = IMAGE_SOURCE.with(|t| {
                                t.with(widget_key(h), |e| {
                                    e.tint = *c;
                                    e.clone()
                                })
                            });
                            if let Some(entry) = entry {
                                repaint_image(pic, &entry.name, *c, entry.template);
                            }
                        }
                        // A source swap repaints the same widget (docs/images.md), so an `image()`
                        // bound to a signal shows new pixels without rebuilding its subtree. A
                        // tinted or template glyph is re-rendered from the new name in the same
                        // paint, so a recycled sidebar row shows the icon it was rebound to.
                        day_spec::props::ImagePatch::Source(source) => {
                            let named = match source {
                                day_spec::ImageSource::Named(name) => name.clone(),
                                _ => String::new(),
                            };
                            let entry = IMAGE_SOURCE.with(|t| {
                                t.with(widget_key(h), |e| {
                                    e.name = named.clone();
                                    e.clone()
                                })
                            });
                            match entry {
                                Some(e)
                                    if !named.is_empty() && (e.tint.is_some() || e.template) =>
                                {
                                    repaint_image(pic, &named, e.tint, e.template)
                                }
                                _ => set_picture_source(pic, source),
                            }
                        }
                    }
                }
            }
            kinds::CONTAINER => {
                if let Some(ContainerPatch::Background(c)) = patch.downcast_ref::<ContainerPatch>()
                {
                    apply_surface(h, *c, 0.0, false);
                }
            }
            kinds::NAV_MENU => {
                if let Some(NavMenuPatch::Items {
                    document_keys,
                    items,
                    icons,
                    badges,
                    badge_icons,
                    badge_tints,
                    sections,
                    tints,
                    menus,
                    selected,
                    rows_changed,
                }) = patch.downcast_ref::<NavMenuPatch>()
                {
                    DOCUMENT_MENU_KEYS.with(|m| {
                        m.borrow_mut().insert(widget_key(h), document_keys.to_vec());
                    });
                    NAV_MENUS.with(|m| {
                        let mut m = m.borrow_mut();
                        let Some(state) = m.get_mut(&widget_key(h)) else {
                            return;
                        };
                        state.suppress.set(true);
                        // The same rows with new words or counts: refresh those rows where they
                        // stand, keeping the selection and every other row.
                        if let Some(rows) = rows_changed
                            && rows.iter().all(|&i| {
                                refresh_nav_row(
                                    &state.listbox,
                                    i,
                                    &items[i],
                                    icons.get(i).and_then(|o| o.as_deref()),
                                    tints.get(i).copied().flatten(),
                                    badges.get(i).and_then(|o| o.as_deref()),
                                    badge_icons.get(i).and_then(|o| o.as_deref()),
                                    badge_tints.get(i).copied().flatten(),
                                )
                            })
                        {
                            state.suppress.set(false);
                            return;
                        }
                        // Before the rows go, not after: their labels own the popovers.
                        unparent_nav_popovers(&state.listbox);
                        while let Some(row) = state.listbox.first_child() {
                            state.listbox.remove(&row);
                        }
                        fill_nav_menu(
                            &state.listbox,
                            items,
                            icons,
                            badges,
                            badge_icons,
                            badge_tints,
                            sections,
                            tints,
                            menus,
                        );
                        state.rows = items.len();
                        match selected {
                            Some(i) => state
                                .listbox
                                .select_row(state.listbox.row_at_index(*i as i32).as_ref()),
                            None => state.listbox.unselect_all(),
                        }
                        state.current.set(selected.map(|i| i as i32));
                        state.suppress.set(false);
                    });
                    // The rows are a navigation suite's switcher where the menu sits in one
                    // (docs/navigation.md): keep the recorded rows current and re-title the
                    // suite's pages.
                    let node = NAV_MENU_ROWS.with(|m| {
                        let mut m = m.borrow_mut();
                        let entry = m.get_mut(&widget_key(h))?;
                        entry.1 = items.clone();
                        entry.2 = icons.clone();
                        Some(entry.0)
                    });
                    if let Some(node) = node
                        && let Some(parent) = h.parent()
                    {
                        suite_rows_changed(&parent, node, items, icons, document_keys);
                    }
                } else if let Some(NavMenuPatch::Selected(sel)) =
                    patch.downcast_ref::<NavMenuPatch>()
                {
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
                        state.current.set(sel.map(|i| i as i32));
                        state.suppress.set(false);
                    });
                }
            }
            kinds::NAV => {
                if let Some(p) = patch.downcast_ref::<day_spec::props::DocumentTabsPatch>() {
                    NAV_STATE.with(|m| {
                        if let Some(state) = m.borrow().get(&widget_key(h))
                            && let NavPresent::Documents(doc) = &state.present
                        {
                            doc.bar.set_visible(p.native);
                        }
                    });
                    return;
                }
                if let Some(NavPatch::Reorder(order)) = patch.downcast_ref::<NavPatch>() {
                    NAV_STATE.with(|m| {
                        let mut m = m.borrow_mut();
                        if let Some(state) = m.get_mut(&widget_key(h)) {
                            // `pages` holds documents alone (the sidebar page is not in it),
                            // so each entry of `order` is an index into it.
                            let old = state.pages.clone();
                            state.pages =
                                order.iter().filter_map(|i| old.get(*i).cloned()).collect();
                            if let NavPresent::Documents(doc) = &state.present {
                                doc.suppress.set(true);
                                for (i, (key, _, _)) in state.pages.iter().enumerate() {
                                    for n in 0..doc.view.n_pages() {
                                        let page = doc.view.nth_page(n);
                                        if page
                                            .child()
                                            .first_child()
                                            .is_some_and(|c| widget_key(&c) == *key)
                                        {
                                            doc.view.reorder_page(&page, i as i32);
                                            break;
                                        }
                                    }
                                }
                                doc.suppress.set(false);
                            }
                        }
                    });
                    return;
                }
                if let Some(p) = patch.downcast_ref::<NavPatch>() {
                    NAV_STATE.with(|m| {
                        let m = m.borrow();
                        let Some(state) = m.get(&widget_key(h)) else {
                            return;
                        };
                        if let (NavPatch::Select(i), NavPresent::Documents(doc)) =
                            (p, &state.present)
                            && *i < doc.view.n_pages() as usize
                        {
                            doc.suppress.set(true);
                            doc.view.set_selected_page(&doc.view.nth_page(*i as i32));
                            doc.suppress.set(false);
                        }
                        // Structure (sidebar / content / push) is driven from insert & remove;
                        // Popped drives the stack's day-initiated pop (suppressing its echo).
                        if let (NavPatch::Popped, NavPresent::Stack(nv)) = (p, &state.present) {
                            state.suppress.set(true);
                            nv.pop();
                            state.suppress.set(false);
                        }
                        // Back guard (docs/navigation.md): AdwNavigationView has no pre-pop veto
                        // signal, so a guarded top page sets `can-pop = false` — the swipe/Escape
                        // back is disabled, and the app drives the back through its own control
                        // (which routes to the GUARDED nav_back()). The guard still runs; it just
                        // isn't reachable by gesture here.
                        if let NavPatch::GuardTop(on) = p
                            && let Some((_, _, Some(page))) = state.pages.last()
                        {
                            page.set_can_pop(!on);
                        }
                        // The resident-page switch (docs/navigation.md): the app moved the
                        // selection, so the suite shows that destination and sets its toggle
                        // Without reporting the move back as a click.
                        if let (NavPatch::Select(i), NavPresent::Suite { stack, .. }) =
                            (p, &state.present)
                            && let Some(child) = stack.child_by_name(&format!("p{}", i + 1))
                        {
                            state.suppress.set(true);
                            stack.set_visible_child(&child);
                            state.suppress.set(false);
                        }
                        // Show or collapse the content-list pane (docs/navigation.md). Applied
                        // at once, no transition: a dayscript screenshot right after the
                        // patch must see the settled shape. The sibling panes take their new
                        // widths on the next allocation, which reports them (`on_allocate`).
                        if let (NavPatch::ListVisible(v), NavPresent::Split(sp)) =
                            (p, &state.present)
                        {
                            sp.list_shown.set(*v);
                            sp.apply_list();
                        }
                        // `NavPatch::Presentation` is deliberately not handled: this backend
                        // answers `Cap::NavRepresent = Unsupported`, so the pieces layer never
                        // sends it. Unlike the other desktops the two presentations are different
                        // WIDGETS here (the `NavSplit` paneds vs AdwNavigationView), and Day
                        // holds the host handle — so morphing means re-homing the pages into a
                        // stack under one host, a real restructure (docs/size-classes.md). What
                        // a narrowing window does get is the AppKit-style fold: the sidebar
                        // collapses when there is no room for it (`NavSplit::fit`).
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
                        LabelPatch::Runs(text, runs) => set_label_runs(label, text, runs),
                    }
                }
            }
            kinds::BUTTON => {
                if let (Some(p), Some(btn)) = (
                    patch.downcast_ref::<ButtonPatch>(),
                    h.downcast_ref::<gtk4::Button>(),
                ) {
                    match p {
                        ButtonPatch::Content(c) => {
                            apply_button_content(btn, &c.title, c.icon.as_ref(), c.icon_only)
                        }
                        ButtonPatch::Title(t) => btn.set_label(t),
                        ButtonPatch::Enabled(e) => btn.set_sensitive(*e),
                        ButtonPatch::Selected(on) => set_button_selected(btn, *on),
                        ButtonPatch::Style(s) => apply_button_style(btn, *s),
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
                if let Some(p @ ProgressPatch::Animating(_)) = patch.downcast_ref::<ProgressPatch>()
                {
                    crate::controls::spinner::update(self, h, p);
                    return;
                }

                if let Some(ProgressPatch::Value(Some(v))) = patch.downcast_ref::<ProgressPatch>()
                    && let Some(bar) = h.downcast_ref::<gtk4::ProgressBar>()
                    && (bar.fraction() - v).abs() > 0.0001
                {
                    bar.set_fraction(*v);
                }
            }
            kinds::PICKER => picker::update_any(self, h, patch),
            kinds::TEXT_AREA => textarea::update_any(self, h, patch),
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
                Some(ListPatch::Reload) | Some(ListPatch::Splice(_)) => {
                    LIST_STATE.with(|m| {
                        if let Some(e) = m.borrow().get(&widget_key(h)) {
                            // Deferred: this runs inside a with_tree borrow (see schedule_list_resize).
                            schedule_list_resize(
                                e.model.clone(),
                                e.source.clone(),
                                e.selection.clone(),
                                e.bound.clone(),
                            );
                            if let Some(pinned) = &e.pinned {
                                pinned.reset_later();
                            }
                        }
                    });
                }
                Some(ListPatch::ScrollToEnd) => {
                    if let Some(sw) = list_scroller(h) {
                        scroll_list_when_laid(&sw, None, None);
                    }
                }
                Some(ListPatch::ScrollToRow(row)) => {
                    if let Some(sw) = list_scroller(h) {
                        let pinned = LIST_STATE.with(|m| {
                            m.borrow()
                                .get(&widget_key(h))
                                .and_then(|e| e.pinned.clone())
                        });
                        scroll_list_when_laid(&sw, Some(*row), pinned);
                    }
                }
                Some(ListPatch::Selected(rows)) => {
                    let selection = LIST_STATE.with(|m| {
                        m.borrow()
                            .get(&widget_key(h))
                            .and_then(|e| e.selection.clone())
                    });
                    if let Some(selection) = selection {
                        selection.requested.set(
                            rows.first()
                                .and_then(|r| u32::try_from(*r).ok())
                                .unwrap_or(gtk4::INVALID_LIST_POSITION),
                        );
                        // Native selection notifies synchronously; defer past the tree borrow
                        // and suppress the echo. Reloads reapply this same requested row.
                        gtk4::glib::idle_add_local_once(move || {
                            selection.suppress.set(true);
                            selection.apply();
                            selection.suppress.set(false);
                        });
                    }
                }
                Some(ListPatch::RowSizeInvalidated { cell, .. }) => {
                    // A self-sizing row Day re-laid at a new height: the DayCell answers it from
                    // `cell_height` in `measure`, so it only has to be measured again. Deferred
                    // past the tree borrow, which that measure would otherwise find held.
                    let entry = LIST_STATE.with(|m| {
                        m.borrow()
                            .get(&widget_key(h))
                            .map(|e| (e.bound.clone(), e.pinned.clone()))
                    });
                    if let Some((bound, pinned)) = entry {
                        let cell = *cell;
                        gtk4::glib::idle_add_local_once(move || {
                            let item = bound.borrow().get(&cell).and_then(|li| li.upgrade());
                            if let Some(cell) = item.and_then(|li| li.child()) {
                                cell.queue_resize();
                            }
                            // The pinned header copy is a bound cell too, outside the list.
                            if let Some(pinned) = pinned
                                && pinned.cell.as_ptr() as usize == cell
                            {
                                pinned.cell.queue_resize();
                                pinned.overlay.queue_allocate();
                            }
                        });
                    }
                }
                // No editing mode (Cap::ListEditing is Unsupported).
                Some(ListPatch::Editing(_)) | None => {}
            },
            kinds::TREE => match patch.downcast_ref::<TreePatch>() {
                // ALL of these defer to an idle: they arrive inside a `with_tree` borrow, and
                // model changes bind rows synchronously (the schedule_list_resize rule).
                Some(TreePatch::Reload) => {
                    if let Some(entry) = tree_entry(widget_key(h)) {
                        schedule_tree_rebuild(entry);
                    }
                }
                Some(TreePatch::Expand(token, on)) => {
                    if let Some(entry) = tree_entry(widget_key(h)) {
                        if *on {
                            entry.expanded.borrow_mut().insert(*token);
                        } else {
                            entry.expanded.borrow_mut().remove(token);
                        }
                        gtk4::glib::idle_add_local_once(move || {
                            ffi_guard::contain((), || tree_apply_expansion(&entry));
                        });
                    }
                }
                Some(TreePatch::Selected(tokens)) => {
                    if let Some(entry) = tree_entry(widget_key(h)) {
                        *entry.selected.borrow_mut() = tokens.clone();
                        gtk4::glib::idle_add_local_once(move || {
                            ffi_guard::contain((), || tree_apply_selection(&entry));
                        });
                    }
                }
                Some(TreePatch::Reveal(token)) => {
                    // The v4_10 route the list documents: position × uniform pitch through
                    // the scroller's adjustment.
                    if let (Some(entry), Some(sw)) = (
                        tree_entry(widget_key(h)),
                        h.downcast_ref::<gtk4::ScrolledWindow>(),
                    ) {
                        let (adj, token) = (sw.vadjustment(), *token);
                        gtk4::glib::idle_add_local_once(move || {
                            ffi_guard::contain((), || {
                                let Some(model) = entry.listview.model() else {
                                    return;
                                };
                                let n = model.n_items();
                                for i in 0..n {
                                    let hit = model
                                        .item(i)
                                        .and_then(|o| o.downcast::<gtk4::TreeListRow>().ok())
                                        .and_then(|r| tree_row_token(&r))
                                        == Some(token);
                                    if hit {
                                        let y = (adj.upper() / n as f64) * i as f64;
                                        adj.set_value(
                                            y.min((adj.upper() - adj.page_size()).max(0.0)),
                                        );
                                        break;
                                    }
                                }
                            });
                        });
                    }
                }
                None => {}
            },
            _ => {
                if let Some(update) = self.registry.get(kind).map(|r| r.update) {
                    update(self, h, patch);
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
    fn release(&mut self, h: Handle) {
        LIST_ITEMS.with(|m| m.borrow_mut().remove(&widget_key(&h)));
        resign_focus_before_detach(&h);
        // A released window content = that window is gone (docs/windows.md teardown):
        // drop the record, destroying a straggler window (a programmatic-close's window is
        // already closed; destroy on it is a no-op).
        self.secondary.retain(|w| {
            if w.fixed.upcast_ref::<gtk4::Widget>() == &h {
                // The window's own pointer-keyed state (the toolbar's BARS/HEADERS tables)
                // goes with it — sweep by the WINDOW pointer, before the destroy can free the
                // address for reuse.
                day_spec::sidetable::sweep(w.window.as_ptr() as usize);
                w.window.destroy();
                false
            } else {
                true
            }
        });
        let key = widget_key(&h);
        DOCUMENT_MENU_KEYS.with(|m| m.borrow_mut().remove(&key));
        // One call clears this pointer out of every SideTable registered on this thread (canvas
        // OPS, IMAGE_SOURCE, LIST_CELL_ROWS, COVER_IDS, SURFACE's css providers, the
        // picker/textarea state, the toolbar tables), running each table's teardown hook —
        // SURFACE detaches its display-global provider here. That closes the stale-pointer-key
        // class the NAV_MENUS comment below documents: a map with no release line hands its dead
        // entry to whatever widget the allocator next puts at this address. The maps removed by
        // hand below predate the mechanism; NEW per-view state should be a SideTable, not
        // another line here.
        day_spec::sidetable::sweep(key);
        GTK_ANIMS.with(|m| {
            m.borrow_mut().remove(&key);
        });
        NODE_ORIGIN.with(|m| {
            m.borrow_mut().remove(&key);
        });
        LABEL_STYLE.with(|m| {
            m.borrow_mut().remove(&key);
        });
        // A sectioned list's pinned header copy is a cell Day bound outside the list's own
        // pool, so no factory teardown releases it: the list going away does.
        let list = LIST_STATE.with(|m| m.borrow_mut().remove(&key));
        if let Some(pinned) = list.as_ref().and_then(|e| e.pinned.as_ref())
            && let Some(src) = pinned.source.borrow().as_ref()
        {
            (src.release)(pinned.cell.as_ptr() as RawHandle);
        }
        TREE_STATE.with(|m| {
            m.borrow_mut().remove(&key);
        });
        // A nav menu's row popovers are parented to its ROWS, so they have to be unparented
        // while those rows are still alive — and `release` is the last moment day holds a
        // reference to any of them. Dropping the state without doing so leaves live popovers
        // pointing at rows GTK is about to finalize, and because NAV_ROW_POPOVERS is keyed by
        // the LISTBOX POINTER, the next listbox the allocator puts at that address inherits the
        // dead entry: `unparent_nav_popovers` then walks a freed parent chain inside
        // `gtk_widget_unparent` → `gtk_accessible_update_children` → `gtk_widget_is_ancestor`.
        // That is a use-after-free whose crash lands wherever the heap happens to be — the CI
        // segfault (`addr=36`, a type-header read off a dangling pointer) reported from a
        // different page on every run.
        if let Some(state) = NAV_MENUS.with(|m| m.borrow_mut().remove(&key)) {
            unparent_nav_popovers(&state.listbox);
        }
        // The same rule for a piece's own `.context_menu()` popover, which is parented to this
        // widget: unparent it here or GTK finalizes the widget with the popover still attached,
        // and the stale map entry — keyed by widget pointer — is then inherited by whatever the
        // allocator puts at that address next, whose `set_context_menu` unparents a popover
        // whose parent is long gone.
        MENU_POPOVERS.with(|m| {
            if let Some(pop) = m.borrow_mut().remove(&key) {
                pop.unparent();
            }
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
            g.borrow_mut().retain(|(ptr, _)| *ptr != key);
        });
        // A tab page detaches from its AdwViewStack; a nav page is owned by its AdwNavigationPage
        // (already detached in `remove`); a list row's content by its DayCell; everything else
        // lives in a GtkFixed parent.
        if let Some(stack) = h.parent().and_then(|p| p.downcast::<adw::ViewStack>().ok()) {
            stack.remove(&h);
        } else if let Some(parent) = h.parent() {
            if let Some(fixed) = parent.downcast_ref::<gtk4::Fixed>() {
                fixed.remove(&h);
            } else if let Some(cell) = parent.downcast_ref::<DayCell>() {
                cell.remove_child(&h);
            }
        }
    }

    fn insert(&mut self, parent: &Handle, child: &Handle, index: usize) {
        // The first mount into a dialog-hosted panel decides the dialog: Day content gets the
        // plain one (the settings shell claimed its own before mounting anything).
        if let Some(h) = dialog_host_for(parent) {
            h.materialize_plain();
        }
        // A nav menu that has just gained ancestors: if one of them is a navigation suite, its
        // rows are that suite's switcher. Runs before the insert proper so the toggles exist by
        // the time the first page is shown.
        if let Some((node, titles, icons)) =
            NAV_MENU_ROWS.with(|m| m.borrow().get(&widget_key(child)).cloned())
        {
            let keys = DOCUMENT_MENU_KEYS.with(|m| {
                m.borrow()
                    .get(&widget_key(child))
                    .cloned()
                    .unwrap_or_default()
            });
            suite_rows_changed(parent, node, &titles, &icons, &keys);
        }
        let host_key = widget_key(parent);
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
            let nav_page = match &state.present {
                NavPresent::Documents(doc) => {
                    // Day's nav host keeps its menu as child 0, inside the sidebar page: that
                    // page goes in the hidden sidebar box and never into `pages` or the tab
                    // view, so document i is Day's child i + 1 and the view's page i.
                    if index == 0 {
                        doc.sidebar.append(child);
                        return true;
                    } else {
                        let cell = DayCell::filling();
                        cell.add_child(child);
                        doc.suppress.set(true);
                        let page = doc.view.append(&cell);
                        if let Some(key) = doc.keys.borrow().get(index - 1).cloned() {
                            doc.page_keys
                                .borrow_mut()
                                .insert(widget_key(cell.upcast_ref()), key);
                        }
                        if let Some(title) = doc.titles.borrow().get(index - 1) {
                            page.set_title(title);
                        }
                        doc.suppress.set(false);
                        cell.on_allocate(move |w, h| {
                            if w > 0 && h > 0 {
                                emit(id, Event::FrameChanged(Size::new(w as f64, h as f64)));
                            }
                        });
                    }
                    None
                }
                NavPresent::Split(sp) => {
                    // The page goes into its pane's filling cell, which hands it the whole
                    // pane; Day lays the page's content to the width `nav_report` reports.
                    // Which pane is the page's own declaration, never its insert index: a
                    // replaced detail page arrives at index 1 or 2 depending on whether the
                    // host has a list.
                    let pane = NAV_PAGE_PANES
                        .with(|m| m.borrow().get(&widget_key(child)).copied())
                        .unwrap_or_default();
                    let cell = match pane {
                        day_spec::props::Pane::Sidebar => &sp.sidebar,
                        day_spec::props::Pane::List => &sp.list,
                        day_spec::props::Pane::Detail => &sp.detail,
                    };
                    cell.add_child(child);
                    None
                }
                NavPresent::Stack(nv) => {
                    // The root page is the content as it stands. A PUSHED page gets a header
                    // bar (GNOME's navigation page shape): the view draws its back button in
                    // it, so there is a way out besides the swipe and Escape, and the title
                    // names where the user is. The page's content sits in a filling cell
                    // under the bar and reports its own size, the bar's height taken off.
                    let nav_page = if state.pages.is_empty() {
                        adw::NavigationPage::new(child, &title)
                    } else {
                        let cell = DayCell::filling();
                        cell.add_child(child);
                        cell.on_allocate(move |w, h| {
                            if w > 0 && h > 0 {
                                emit(id, Event::FrameChanged(Size::new(w as f64, h as f64)));
                            }
                        });
                        STACK_PAGE_CELLS
                            .with(|m| m.borrow_mut().insert(widget_key(child), cell.clone()));
                        let header = adw::HeaderBar::new();
                        header.set_show_start_title_buttons(false);
                        header.set_show_end_title_buttons(false);
                        let toolbar = adw::ToolbarView::new();
                        toolbar.add_top_bar(&header);
                        toolbar.set_content(Some(&cell));
                        adw::NavigationPage::new(&toolbar, &title)
                    };
                    state.suppress.set(true);
                    nv.push(&nav_page);
                    state.suppress.set(false);
                    Some(nav_page)
                }
                NavPresent::Suite { stack, rows, .. } => {
                    // Every destination is resident and the switcher shows one at a time. The
                    // page at index 0 is the SIDEBAR page, whose rows became the switcher: it
                    // stays in the stack so its nav menu has a path up to this host, but it is
                    // never shown — drawing the rows again as a list would be the same
                    // navigation twice. A filling cell holds each page (`NavPresent::Suite`
                    // explains) and reports the page's size as the stack allocates it.
                    let cell = DayCell::filling();
                    cell.add_child(child);
                    stack.add_named(&cell, Some(&format!("p{index}")));
                    let page = stack.page(&cell);
                    if index == 0 {
                        page.set_visible(false);
                        cell.set_visible(false);
                    } else {
                        let r = rows.borrow();
                        if let Some(t) = r.0.get(index - 1) {
                            page.set_title(Some(t));
                        }
                        let icon =
                            r.1.get(index - 1)
                                .and_then(|o| o.as_deref())
                                .and_then(vector_icon_name)
                                .or_else(blank_icon_name);
                        page.set_icon_name(icon.as_deref());
                        if stack.visible_child().is_none() {
                            state.suppress.set(true);
                            stack.set_visible_child(&cell);
                            state.suppress.set(false);
                        }
                    }
                    let last = std::cell::Cell::new((0, 0));
                    cell.on_allocate(move |width, height| {
                        if width > 0
                            && height > 0
                            && last.replace((width, height)) != (width, height)
                        {
                            ffi_guard::contain((), || {
                                emit(
                                    id,
                                    Event::FrameChanged(Size::new(
                                        f64::from(width),
                                        f64::from(height),
                                    )),
                                )
                            });
                        }
                    });
                    None
                }
            };
            state.pages.push((widget_key(child), id, nav_page));
            true
        });
        if handled {
            gtk4::glib::idle_add_local_once(move || {
                ffi_guard::contain((), || nav_report(host_key))
            });
            return;
        }
        // A split pane landing in its paned (docs/split.md): into its pane's filling cell.
        let split_pane = SPLIT_STATE.with(|t| t.get(host_key)).map(|state| {
            let (pane_id, second) = SPLIT_PANES
                .with(|t| t.get(widget_key(child)))
                .unwrap_or((NodeId(0), index == 1));
            let mut state = state.borrow_mut();
            if second {
                state.second.add_child(child);
            } else {
                state.first.add_child(child);
            }
            state.panes.push((pane_id, second, child.clone()));
        });
        if split_pane.is_some() {
            return;
        }
        // An inspector pane landing in its paned (docs/inspector.md): into its pane's filling
        // cell, which hands it the whole pane — so Day framing the content to the full width
        // while the panel is hidden never becomes a GTK minimum the reveal cannot push against.
        let inspected = INSPECTOR_STATE.with(|t| t.get(host_key)).map(|state| {
            let (pane_id, panel) = INSPECTOR_PANES
                .with(|t| t.get(widget_key(child)))
                .unwrap_or((NodeId(0), index == 1));
            let mut state = state.borrow_mut();
            if panel {
                state.panel.add_child(child);
            } else {
                state.content.add_child(child);
            }
            state.panes.push((pane_id, panel, child.clone()));
        });
        if inspected.is_some() {
            gtk4::glib::idle_add_local_once(move || {
                ffi_guard::contain((), || inspector_report(host_key))
            });
        } else if let Some(fixed) = content_of(parent).downcast_ref::<gtk4::Fixed>() {
            fixed.put(child, 0.0, 0.0);
        } else if let Some(cell) = content_of(parent).downcast_ref::<DayCell>() {
            // A list row's content: the cell places it from the frames `set_frame` records.
            cell.add_child(child);
        }
    }

    fn remove(&mut self, parent: &Handle, child: &Handle) {
        resign_focus_before_detach(child);
        let handled = NAV_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let Some(state) = m.get_mut(&widget_key(parent)) else {
                return false;
            };
            let key = widget_key(child);
            // The document host's sidebar page is not in `pages` (see `insert`).
            if let NavPresent::Documents(doc) = &state.present
                && child.parent().as_ref() == Some(doc.sidebar.upcast_ref::<gtk4::Widget>())
            {
                doc.sidebar.remove(child);
                return true;
            }
            if let Some(pos) = state.pages.iter().position(|(k, _, _)| *k == key) {
                let (_, _, nav_page) = state.pages.remove(pos);
                match (&state.present, nav_page) {
                    // A pane's page is being replaced (a new detail follows): its cell lets
                    // it go.
                    (NavPresent::Split(_), _) => {
                        if let Some(cell) =
                            child.parent().and_then(|p| p.downcast::<DayCell>().ok())
                        {
                            cell.remove_child(child);
                        }
                    }
                    (NavPresent::Documents(doc), _) => {
                        if let Some(cell) =
                            child.parent().and_then(|p| p.downcast::<DayCell>().ok())
                        {
                            doc.suppress.set(true);
                            let page = doc.view.page(&cell);
                            doc.view.close_page(&page);

                            doc.page_keys
                                .borrow_mut()
                                .remove(&widget_key(cell.upcast_ref()));
                            cell.remove_child(child);
                            doc.suppress.set(false);
                        }
                    }
                    (NavPresent::Suite { stack, .. }, _) => {
                        if let Some(cell) =
                            child.parent().and_then(|p| p.downcast::<DayCell>().ok())
                        {
                            stack.remove(&cell);
                            cell.remove_child(child);
                        }
                    }
                    // The stack pop already removed it (day-driven pop or native gesture);
                    // dropping our ref is enough — and a pushed page's cell lets the content go.
                    (NavPresent::Stack(_), _) => {
                        if let Some(cell) = STACK_PAGE_CELLS.with(|m| m.borrow_mut().remove(&key)) {
                            cell.remove_child(child);
                        }
                    }
                }
            }
            true
        });
        if handled {
            return;
        }
        let split_pane = SPLIT_STATE
            .with(|t| t.get(widget_key(parent)))
            .map(|state| {
                let (pane_id, second) = SPLIT_PANES
                    .with(|t| t.get(widget_key(child)))
                    .unwrap_or((NodeId(0), false));
                let mut state = state.borrow_mut();
                if second {
                    state.second.remove_child(child);
                } else {
                    state.first.remove_child(child);
                }
                state.panes.retain(|(id, ..)| *id != pane_id);
            });
        if split_pane.is_some() {
            return;
        }
        let inspected = INSPECTOR_STATE
            .with(|t| t.get(widget_key(parent)))
            .map(|state| {
                let (pane_id, panel) = INSPECTOR_PANES
                    .with(|t| t.get(widget_key(child)))
                    .unwrap_or((NodeId(0), false));
                let mut state = state.borrow_mut();
                if panel {
                    state.panel.remove_child(child);
                } else {
                    state.content.remove_child(child);
                }
                state.panes.retain(|(id, ..)| *id != pane_id);
            });
        if inspected.is_none() {
            let host = content_of(parent);
            if let Some(fixed) = host.downcast_ref::<gtk4::Fixed>() {
                fixed.remove(child);
            } else if let Some(cell) = host.downcast_ref::<DayCell>() {
                cell.remove_child(child);
            }
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
                // A non-label backing (a realize arm degraded, or a day-core regression):
                // fall back to the generic widget measure rather than aborting mid-layout.
                let Some(label) = h.downcast_ref::<gtk4::Label>() else {
                    let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
                    let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
                    return Size::new(nat_w as f64, nat_h as f64);
                };
                let layout = gtk4::pango::Layout::new(&label.pango_context());
                // A label with styled runs measures from its MARKUP: `label.text()` is the
                // markup stripped of tags and its attribute list is empty (see `base_span`), so
                // measuring those two would size bold and monospace runs at the base font.
                let key = label.clone().upcast::<gtk4::Widget>().as_ptr() as usize;
                let rich = LABEL_STYLE.with(|m| {
                    m.borrow().get(&key).and_then(|s| {
                        s.rich.as_ref().map(|(t, r)| {
                            // Without the link tags: `<a href>` is GtkLabel's own extension, not
                            // Pango markup — a bare layout fails to parse it, comes out empty,
                            // and reports a zero size, which collapses the label. Links change no
                            // metrics, so dropping the tag measures exactly the same text.
                            let unlinked: Vec<_> = r
                                .iter()
                                .cloned()
                                .map(|mut run| {
                                    run.link = None;
                                    run
                                })
                                .collect();
                            rich_markup(t, &unlinked, s)
                        })
                    })
                });
                match rich {
                    Some(markup) => layout.set_markup(&markup),
                    None => {
                        layout.set_text(&label.text());
                        layout.set_attributes(label.attributes().as_ref());
                    }
                }
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
            kinds::PICKER => picker::measure_any(self, h, p),
            kinds::TEXT_AREA => textarea::measure_any(self, h, p),
            kinds::TEXT_FIELD => {
                let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
                Size::new(p.width.unwrap_or(180.0), (nat_h as f64).max(24.0))
            }
            kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 1.0),
            // The row box's own natural size, asked with its size request lifted: GTK never
            // measures a widget smaller than its request, which Day set to the last frame, so a
            // row that lost its subtitle would otherwise keep the taller height.
            kinds::LIST_ITEM => {
                let (rw, rh) = h.size_request();
                h.set_size_request(-1, -1);
                let width = p.width.map(|w| w.round() as i32).unwrap_or(-1);
                let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
                let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, width);
                h.set_size_request(rw, rh);
                Size::new(p.width.unwrap_or(nat_w as f64), (nat_h as f64).max(44.0))
            }
            // A canvas has no intrinsic size: accept the offered axes, just as the other
            // backends do. GtkDrawingArea's natural size is zero; using it here collapses
            // `.grow_w().aspect_ratio(...)` to zero height despite the square proposal.
            kinds::CANVAS => Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0)),
            // The recycling list fills the space it is offered (its scroll owns overflow).
            kinds::LIST | kinds::TREE => Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0)),
            kinds::PROGRESS => {
                if let Some(side) = crate::controls::spinner::side(h) {
                    Size::new(side, side)
                } else {
                    let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
                    Size::new(p.width.unwrap_or(180.0), (nat_h as f64).max(6.0))
                }
            }
            _ => {
                if let Some(measure) = self.registry.get(kind).and_then(|r| r.measure) {
                    return measure(self, h, p);
                }
                // Free of request state. `set_frame` sizes GtkFixed children through
                // `set_size_request`, and `gtk_widget_measure` never reports less than the
                // current request — so measuring the widget as it stands RATCHETS to wherever
                // Day last placed it (the LABEL arm measures Pango for the same reason). A shape
                // or canvas placed at 300 once would never measure narrower again, and nothing
                // containing it could shrink: a list row re-laid at its cell's real width kept
                // its old width exactly that way (day#35). Measure the widget's own natural size
                // with the request cleared, then put the request back.
                let (req_w, req_h) = h.size_request();
                h.set_size_request(-1, -1);
                let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
                let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
                h.set_size_request(req_w, req_h);
                Size::new(nat_w as f64, nat_h as f64)
            }
        }
    }

    fn set_opacity(&mut self, h: &Handle, opacity: f64, anim: Option<&AnimSpec>) {
        let key = widget_key(h);
        match anim {
            None => {
                GTK_ANIMS.with(|m| m.borrow_mut().entry(key).or_default().opacity = None);
                h.set_opacity(opacity);
            }
            Some(a) => {
                // Apply the final value first (so it lands even if the frame clock never ticks —
                // e.g. an unmapped/headless window), then tween over it from the current value via
                // libadwaita's frame-clock animation. The callback's v=0 frame re-establishes the
                // start, so there's no visible jump when it does run.
                let from = h.opacity();
                h.set_opacity(opacity);
                let widget = h.clone();
                let target = adw::CallbackAnimationTarget::new(move |v| widget.set_opacity(v));
                let animation = gtk_animation(h, from, opacity, a, target);
                animation.play();
                GTK_ANIMS
                    .with(|m| m.borrow_mut().entry(key).or_default().opacity = Some(animation));
            }
        }
    }

    fn set_transform(&mut self, h: &Handle, t: Transform, size: Size, anim: Option<&AnimSpec>) {
        // A child's transform lives on its GtkFixed parent (GskTransform), applied about the
        // widget's center so scale/rotation match the other backends.
        let Some(parent) = h.parent() else {
            return;
        };
        let Some(fixed) = parent.downcast_ref::<gtk4::Fixed>() else {
            return;
        };
        let key = widget_key(h);
        let from = GTK_ANIMS
            .with(|m| m.borrow().get(&key).map(|s| s.cur_transform))
            .unwrap_or_default();
        match anim {
            None => {
                apply_gtk_transform(fixed, h, t, size);
                GTK_ANIMS.with(|m| {
                    let mut b = m.borrow_mut();
                    let s = b.entry(key).or_default();
                    s.transform = None;
                    s.cur_transform = t;
                });
            }
            Some(a) => {
                // Apply the final transform first (robust if the frame clock never ticks), then
                // interpolate the whole transform (progress 0→1) over it on the Adw frame clock.
                apply_gtk_transform(fixed, h, t, size);
                let fixed = fixed.clone();
                let widget = h.clone();
                let target = adw::CallbackAnimationTarget::new(move |v| {
                    apply_gtk_transform(&fixed, &widget, from.lerp(t, v), size);
                });
                let animation = gtk_animation(h, 0.0, 1.0, a, target);
                animation.play();
                GTK_ANIMS.with(|m| {
                    let mut b = m.borrow_mut();
                    let s = b.entry(key).or_default();
                    s.transform = Some(animation);
                    s.cur_transform = t;
                });
            }
        }
    }

    fn set_selectable(&mut self, h: &Handle, selectable: bool) -> Option<Handle> {
        // A plain label is a GtkLabel (docs/text.md); the downcast guards a non-label backing.
        if let Some(l) = h.downcast_ref::<gtk4::Label>() {
            l.set_selectable(selectable);
        }
        None
    }

    fn set_input_traits(&mut self, h: &Handle, traits: &InputTraits) -> Option<Handle> {
        // One GtkEntry serves every trait as a property (docs/textfield.md), so the widget, its
        // signal wiring, its focus and its caret all stay; the downcast guards a non-entry
        // backing. Each setter compares before it notifies, so a repeat call costs nothing.
        let entry = h.downcast_ref::<gtk4::Entry>()?;
        // Visibility first: the characters are hidden before anything else about the field
        // moves.
        entry.set_visibility(!traits.secure);
        let (purpose, hints) = gtk_input_purpose(traits.purpose, traits.secure);
        entry.set_input_purpose(purpose);
        entry.set_input_hints(hints);
        // Not editable is GTK's read-only: the text still selects and copies, and the entry
        // keeps its ordinary look (insensitive is what `.disabled()` uses).
        entry.set_editable(!traits.read_only);
        // GTK counts the limit in characters, as day-pieces does, and reads 0 as unbounded.
        // 65535 is the most a GtkEntryBuffer holds. A bound of zero has no native spelling, so
        // day-pieces alone holds that one.
        entry.set_max_length(traits.max_length.map_or(0, |n| n.min(65535) as i32));
        // `submit_label` has no GTK counterpart: the on-screen keyboard draws its own Enter.
        None
    }

    fn set_cursor(&mut self, h: &Handle, cursor: Cursor) {
        // GDK names cursors the way CSS does (docs/cursor.md), so the keyword goes straight
        // through; a theme missing one falls back to its default arrow rather than to nothing.
        if cursor == Cursor::Default {
            h.set_cursor(None);
            return;
        }
        let fallback = gtk4::gdk::Cursor::from_name("default", None);
        let shape = gtk4::gdk::Cursor::from_name(cursor.css_name(), fallback.as_ref());
        h.set_cursor(shape.as_ref());
    }

    /// GTK reports baselines from its own measure protocol (docs/baseline.md): the third and
    /// fourth out-params of `gtk_widget_measure` are the minimum and natural baselines, which is
    /// what `GTK_ALIGN_BASELINE` containers align on. `-1` means the widget has none.
    ///
    /// The baseline is reported for the widget's NATURAL height, which is the height day
    /// allocates it, so the two agree without any correction.
    fn first_baseline(&mut self, h: &Handle, kind: PieceKind, size: Size) -> Option<f64> {
        if !day_spec::kind_has_baseline(kind) {
            return None;
        }
        let for_width = if size.width > 0.0 {
            size.width.round() as i32
        } else {
            -1
        };
        let (_, _, _, nat_baseline) = h.measure(gtk4::Orientation::Vertical, for_width);
        (nat_baseline >= 0).then_some(nat_baseline as f64)
    }

    fn set_frame(&mut self, h: &Handle, frame: Rect, _anim: Option<&AnimSpec>) {
        let key = widget_key(h);
        // Nav pages are laid out by their native container, not by Day; skip them.
        if NAV_PAGE_IDS.with(|m| m.borrow().contains_key(&key)) {
            return;
        }
        // A list row's content: the DayCell records the frame and places the child itself when
        // GTK allocates it. No `set_size_request` — that would make Day's width the cell's
        // minimum and inflate the row, the very thing the cell exists to prevent (day#35).
        if let Some(cell) = h.parent().and_then(|p| p.downcast::<DayCell>().ok()) {
            NODE_ORIGIN.with(|m| {
                m.borrow_mut()
                    .insert(key, (frame.origin.x as f32, frame.origin.y as f32))
            });
            cell.set_child_frame(h, frame);
            return;
        }
        if let Some(parent) = h.parent()
            && let Some(fixed) = parent.downcast_ref::<gtk4::Fixed>()
        {
            NODE_ORIGIN.with(|m| {
                m.borrow_mut()
                    .insert(key, (frame.origin.x as f32, frame.origin.y as f32))
            });
            fixed.move_(h, frame.origin.x, frame.origin.y);
            // `move_` rewrote the child transform to a plain translation, dropping any active
            // animation transform. Re-apply it over the new origin so a relayout (window resize,
            // deep-link mount) doesn't reset a transformed widget's scale/rotation/offset — or
            // strand it at the corner.
            let cur = GTK_ANIMS.with(|m| m.borrow().get(&key).map(|s| s.cur_transform));
            if let Some(t) = cur
                && !t.is_identity()
            {
                apply_gtk_transform(fixed, h, t, frame.size);
            }
        }
        h.set_size_request(
            frame.size.width.round() as i32,
            frame.size.height.round() as i32,
        );
        // A stack or suite host resized (window resize): re-report page sizes for relayout.
        // GTK allocates asynchronously — defer one idle so size/position settle. A split
        // host's panes report themselves as GTK allocates them (`nav_pane_report`), and so do
        // an inspector's; a second report here would only lay the same pages out twice.
        let stacked_nav = NAV_STATE.with(|m| {
            m.borrow()
                .get(&key)
                .is_some_and(|s| !matches!(s.present, NavPresent::Split(_)))
        });
        if stacked_nav {
            gtk4::glib::idle_add_local_once(move || ffi_guard::contain((), || nav_report(key)));
        }
    }

    fn set_scroll_content(&mut self, h: &Handle, content: Size) {
        let inner = content_of(h);
        if inner.as_ptr() != h.as_ptr() {
            inner.set_size_request(content.width.round() as i32, content.height.round() as i32);
        }
    }

    /// A GNOME list row: 12px sides, as Adwaita pads its rich lists, and 44px tall.
    fn list_row_metrics(&mut self, _h: &Handle) -> day_spec::RowMetrics {
        day_spec::RowMetrics {
            insets: day_spec::Insets {
                top: 8.0,
                leading: 12.0,
                bottom: 8.0,
                trailing: 12.0,
            },
            min_height: 44.0,
        }
    }

    fn scroll_to(&mut self, h: &Handle, target: Rect, _animated: bool) {
        // Minimal scroll so `target` (content space) is visible; the adjustments clamp to
        // their own range. GTK adjustments animate only via kinetic scroll, so jumps are
        // immediate — dayscript wants that anyway.
        let Some(sw) = h.downcast_ref::<gtk4::ScrolledWindow>() else {
            return;
        };
        let reveal = |adj: gtk4::Adjustment, lo: f64, hi: f64| {
            let mut v = adj.value();
            if hi > v + adj.page_size() {
                v = hi - adj.page_size();
            }
            if lo < v {
                v = lo;
            }
            adj.set_value(v);
        };
        reveal(
            sw.vadjustment(),
            target.origin.y,
            target.origin.y + target.size.height,
        );
        reveal(
            sw.hadjustment(),
            target.origin.x,
            target.origin.x + target.size.width,
        );
    }

    fn set_focusable(&mut self, h: &Handle, node: NodeId, focusable: bool) {
        h.set_focusable(focusable);
        h.set_can_focus(focusable);
        if !focusable {
            return;
        }
        if h.is::<gtk4::DrawingArea>()
            || CONTAINER_KEYS
                .with(|t| t.get(h.as_ptr() as usize))
                .is_some()
        {
            return;
        }
        CONTAINER_KEYS.with(|t| t.insert(h.as_ptr() as usize, true));
        h.connect_has_focus_notify(move |w| {
            ffi_guard::contain((), || emit(node, Event::FocusChanged(w.has_focus())));
        });
        let keys = gtk4::EventControllerKey::new();
        keys.connect_key_pressed(move |controller, key, _, state| {
            ffi_guard::contain(gtk4::glib::Propagation::Proceed, || {
                if !controller.widget().is_some_and(|w| w.has_focus()) {
                    return gtk4::glib::Propagation::Proceed;
                }
                let Some(name) = key_name(key, state) else {
                    return gtk4::glib::Propagation::Proceed;
                };
                if !day_spec::keys::handled(node) {
                    return gtk4::glib::Propagation::Proceed;
                }
                emit(
                    node,
                    Event::Key(day_spec::KeyEvent {
                        key: name.into(),
                        modifiers: key_modifiers(state),
                    }),
                );
                gtk4::glib::Propagation::Stop
            })
        });
        h.add_controller(keys);
        let click = gtk4::GestureClick::new();
        click.connect_pressed(|g, _, _, _| {
            if let Some(w) = g.widget() {
                w.grab_focus();
            }
        });
        h.add_controller(click);
    }

    fn focus(&mut self, h: &Handle, _node: NodeId, focused: bool) {
        if focused {
            // grab_focus only lands on a mapped widget; a request racing the first map
            // (mount reconciliation, docs/focus.md rule 4) retries once at map time.
            if h.is_mapped() {
                h.grab_focus();
            } else {
                let handler = Rc::new(RefCell::new(None));
                let handler2 = handler.clone();
                *handler.borrow_mut() = Some(h.connect_map(move |w| {
                    w.grab_focus();
                    if let Some(sig) = handler2.borrow_mut().take() {
                        w.disconnect(sig);
                    }
                }));
            }
        } else if h
            .state_flags()
            .intersects(gtk4::StateFlags::FOCUSED | gtk4::StateFlags::FOCUS_WITHIN)
            && let Some(root) = h.root()
        {
            // Resign only while this widget (or its inner text) holds focus, so a stale
            // release can't blur a sibling.
            root.set_focus(None::<&gtk4::Widget>);
        }
    }

    fn set_event_sink(&mut self, sink: EventSink) {
        SINK.with(|s| *s.borrow_mut() = Some(Rc::from(sink)));
    }

    fn enable_gesture(&mut self, h: &Handle, node: NodeId, kind: day_spec::GestureKind) {
        use day_spec::{DragPhase, GestureKind, Point};
        let key = (h.as_ptr() as usize, kind);
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
                        ffi_guard::contain((), || {
                            start.set((x, y));
                            emit(
                                node,
                                Event::Drag {
                                    phase: DragPhase::Began,
                                    location: Point::new(x, y),
                                    translation: Point::ZERO,
                                },
                            );
                        });
                    }
                });
                drag.connect_drag_update({
                    let start = start.clone();
                    move |_, ox, oy| {
                        ffi_guard::contain((), || {
                            let (sx, sy) = start.get();
                            emit(
                                node,
                                Event::Drag {
                                    phase: DragPhase::Changed,
                                    location: Point::new(sx + ox, sy + oy),
                                    translation: Point::new(ox, oy),
                                },
                            );
                        });
                    }
                });
                drag.connect_drag_end({
                    let start = start.clone();
                    move |_, ox, oy| {
                        ffi_guard::contain((), || {
                            let (sx, sy) = start.get();
                            emit(
                                node,
                                Event::Drag {
                                    phase: DragPhase::Ended,
                                    location: Point::new(sx + ox, sy + oy),
                                    translation: Point::new(ox, oy),
                                },
                            );
                        });
                    }
                });
                h.add_controller(drag);
            }
            GestureKind::Pinch => {
                // GtkGestureZoom's scale-changed is CUMULATIVE since the gesture began —
                // exactly Event::Pinch's contract.
                let zoom = gtk4::GestureZoom::new();
                let center = |g: &gtk4::GestureZoom| {
                    g.bounding_box_center()
                        .map(|(x, y)| Point::new(x, y))
                        .unwrap_or(Point::ZERO)
                };
                zoom.connect_begin(move |g, _| {
                    ffi_guard::contain((), || {
                        emit(
                            node,
                            Event::Pinch {
                                phase: DragPhase::Began,
                                scale: 1.0,
                                location: center(g),
                            },
                        )
                    });
                });
                zoom.connect_scale_changed(move |g, scale| {
                    ffi_guard::contain((), || {
                        emit(
                            node,
                            Event::Pinch {
                                phase: DragPhase::Changed,
                                scale,
                                location: center(g),
                            },
                        )
                    });
                });
                zoom.connect_end(move |g, _| {
                    ffi_guard::contain((), || {
                        emit(
                            node,
                            Event::Pinch {
                                phase: DragPhase::Ended,
                                scale: g.scale_delta(),
                                location: center(g),
                            },
                        )
                    });
                });
                h.add_controller(zoom);
            }
            GestureKind::Pan => {
                // Trackpad two-finger scroll / wheel. Deltas arrive per event; wheel notches
                // are scaled to a usable pane step. Sign: Event::Pan's delta is the CONTENT
                // displacement, and a GTK scroll-down (positive dy) moves content up.
                use gtk4::EventControllerScrollFlags;
                let scroll =
                    gtk4::EventControllerScroll::new(EventControllerScrollFlags::BOTH_AXES);
                let container_pan = !h.is::<gtk4::DrawingArea>();
                if container_pan {
                    scroll.set_propagation_phase(gtk4::PropagationPhase::Capture);
                }
                scroll.connect_scroll_begin(move |_| {
                    ffi_guard::contain((), || {
                        emit(
                            node,
                            Event::Pan {
                                phase: DragPhase::Began,
                                delta: Point::ZERO,
                                location: Point::ZERO,
                            },
                        )
                    });
                });
                scroll.connect_scroll(move |c, dx, dy| {
                    if container_pan && dx.abs() <= dy.abs() {
                        return gtk4::glib::Propagation::Proceed;
                    }
                    ffi_guard::contain(gtk4::glib::Propagation::Stop, || {
                        let unit = c.unit();
                        let step = if unit == gtk4::gdk::ScrollUnit::Wheel {
                            40.0
                        } else {
                            1.0
                        };
                        emit(
                            node,
                            Event::Pan {
                                phase: DragPhase::Changed,
                                delta: Point::new(-dx * step, -dy * step),
                                location: Point::ZERO,
                            },
                        );
                        gtk4::glib::Propagation::Stop
                    })
                });
                scroll.connect_scroll_end(move |_| {
                    ffi_guard::contain((), || {
                        emit(
                            node,
                            Event::Pan {
                                phase: DragPhase::Ended,
                                delta: Point::ZERO,
                                location: Point::ZERO,
                            },
                        )
                    });
                });
                h.add_controller(scroll);
            }
            GestureKind::Hover => {
                // GtkEventControllerMotion is exactly this gesture: enter, motion, leave. It
                // reports only when a pointer is present, which is the contract.
                let motion = gtk4::EventControllerMotion::new();
                let last = std::rc::Rc::new(std::cell::Cell::new(Point::ZERO));
                let l = last.clone();
                motion.connect_enter(move |_, x, y| {
                    ffi_guard::contain((), || {
                        l.set(Point::new(x, y));
                        emit(
                            node,
                            Event::Hover {
                                phase: DragPhase::Began,
                                location: Point::new(x, y),
                            },
                        );
                    });
                });
                let l = last.clone();
                motion.connect_motion(move |_, x, y| {
                    ffi_guard::contain((), || {
                        l.set(Point::new(x, y));
                        emit(
                            node,
                            Event::Hover {
                                phase: DragPhase::Changed,
                                location: Point::new(x, y),
                            },
                        );
                    });
                });
                motion.connect_leave(move |_| {
                    ffi_guard::contain((), || {
                        // A leave carries no position, so the last point seen inside stands.
                        emit(
                            node,
                            Event::Hover {
                                phase: DragPhase::Ended,
                                location: last.get(),
                            },
                        );
                    });
                });
                h.add_controller(motion);
            }
            _ => {
                let click = gtk4::GestureClick::new();
                click.connect_released(move |_, _n, x, y| {
                    ffi_guard::contain((), || emit(node, Event::Tap(Point::new(x, y))));
                });
                h.add_controller(click);
            }
        }
    }

    fn set_drag_source(&mut self, h: &Handle, source: day_spec::transfer::Source) {
        transfer::source(h, source);
    }
    fn set_drop_target(&mut self, h: &Handle, target: day_spec::transfer::Target) {
        transfer::target(h, target);
    }
    fn set_context_menu_fn(&mut self, h: &Handle, _node: NodeId, f: day_spec::ContextMenuFn) {
        let key = widget_key(h);
        CTX_MENU_FNS.with(|m| m.borrow_mut().insert(key, f));
        let already = CTX_MENU_WIRED.with(|s| !s.borrow_mut().insert(key));
        if already {
            return;
        }
        h.set_can_target(true);
        let attach = |w: &Handle, button: Option<u32>| {
            let show = {
                let w = w.clone();
                move |x: f64, y: f64| {
                    let f = CTX_MENU_FNS.with(|m| m.borrow().get(&widget_key(&w)).cloned());
                    let Some(f) = f else { return };
                    // Guarded: the provider is app code (it usually re-selects, then builds).
                    ffi_guard::contain((), || {
                        let items = f(day_spec::Point::new(x, y));
                        if !items.is_empty() {
                            show_menu_popover(&w, &items, x, y);
                        }
                    });
                }
            };
            match button {
                Some(b) => {
                    let click = gtk4::GestureClick::new();
                    click.set_button(b);
                    click.connect_pressed(move |_, _n, x, y| show(x, y));
                    w.add_controller(click);
                }
                None => {
                    let long = gtk4::GestureLongPress::new();
                    long.connect_pressed(move |_, x, y| show(x, y));
                    w.add_controller(long);
                }
            }
        };
        attach(h, Some(3));
        attach(h, None);
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

    fn edit_toolbar(&mut self, h: &Handle, ops: &[day_spec::ToolbarOp]) -> bool {
        self.edit_toolbar(h, ops)
    }

    fn update_toolbar(&mut self, h: &Handle, patch: &day_spec::ToolbarPatch) {
        self.patch_toolbar(h, patch);
    }

    fn set_app_menu(&mut self, items: &[day_spec::MenuItem]) {
        // GNOME's bar: File, Edit, View, the app's own menus, then Help — no Window menu, as
        // the shell owns window management on Linux.
        let items = day_core::menu::standard_menu_bar_for(
            day_core::menu::MenuBarStyle::Gnome,
            items.to_vec(),
        );
        let items = &items[..];
        let Some(window) = self
            .window_fixed
            .as_ref()
            .and_then(|f| f.root())
            .and_then(|r| r.downcast::<gtk4::Window>().ok())
        else {
            return;
        };
        let Some(app) = window.application() else {
            return;
        };
        let group = gtk4::gio::SimpleActionGroup::new();
        let model = build_gio_menu(items, &group);
        // Actions live under the "daymenu" prefix on every Day window (menu activations —
        // in-window bar or macOS global bar — resolve against the FOCUSED window);
        // accelerators on the app. NOT `active_window()`: that is None before the first
        // present, which silently left every item action-less at startup.
        window.insert_action_group("daymenu", Some(&group));
        for w in &self.secondary {
            w.window.insert_action_group("daymenu", Some(&group));
        }
        self.menu_group = Some(group);
        set_menu_accels(&app, items);
        register_app_prefs_action(&app, items);
        if cfg!(target_os = "macos") {
            // macOS: the quartz backend renders the menubar model in the GLOBAL menu bar —
            // where a Mac user actually looks (an in-window widget bar reads as broken
            // there). GTK's stock app menu also carries a Preferences… item that enables
            // through the `app.preferences` action registered above.
            app.set_menubar(Some(&model));
            return;
        }
        // Linux/Windows: an in-window PopoverMenuBar under the header bar. The Adwaita
        // window is Window → AdwToolbarView[ AdwHeaderBar, ScrolledWindow[ Viewport[
        // fixed ] ] ] — the ScrolledWindow auto-wraps the (non-scrollable) GtkFixed in a
        // Viewport, so a fixed two-parent hop lands on the ScrolledWindow and the
        // ToolbarView downcast fails (the bar was never attached at all). Walk ancestors
        // until the ToolbarView instead.
        let mut cur = self.window_fixed.as_ref().and_then(|f| f.parent());
        let toolbar = loop {
            match cur {
                Some(w) => match w.clone().downcast::<adw::ToolbarView>() {
                    Ok(t) => break Some(t),
                    Err(_) => cur = w.parent(),
                },
                None => break None,
            }
        };
        let Some(toolbar) = toolbar else { return };
        // Replace any previously-installed bar (set_app_menu may be called again).
        if let Some(old) = self.menu_bar.take() {
            toolbar.remove(&old);
        }
        let bar = gtk4::PopoverMenuBar::from_model(Some(&model));
        toolbar.add_top_bar(&bar);
        self.menu_bar = Some(bar);
    }

    fn attach_tree(&mut self, host: &Handle, source: TreeSource) {
        if let Some(entry) = tree_entry(widget_key(host)) {
            entry.source.replace(Some(source));
            schedule_tree_rebuild(entry);
        }
    }

    fn attach_list(&mut self, host: &Handle, source: ListSource) {
        if let Some(report) = &source.first_visible {
            report(0);
        }
        LIST_STATE.with(|m| {
            if let Some(e) = m.borrow().get(&widget_key(host)) {
                *e.source.borrow_mut() = Some(source);
                // Deferred off this with_tree borrow: splice binds cells synchronously (see
                // schedule_list_resize), which would otherwise re-enter with_tree via bind_row.
                schedule_list_resize(
                    e.model.clone(),
                    e.source.clone(),
                    e.selection.clone(),
                    e.bound.clone(),
                );
            }
        });
    }

    fn adopt(&mut self, raw: RawHandle) -> Handle {
        // A recycling GtkListView cell (a GtkFixed) — Day fills/rebinds its row content in place.
        unsafe { gtk4::glib::translate::from_glib_none(raw as *mut gtk4::ffi::GtkWidget) }
    }

    fn set_a11y(&mut self, h: &Handle, a11y: &A11yProps) {
        use day_spec::Role;
        use gtk4::AccessibleRole;
        use gtk4::accessible::{Property, State};
        if let Some(id) = &a11y.identifier {
            h.set_widget_name(id); // GtkInspector-visible automation id (§13's honest table)
        }
        // Real GtkAccessible properties → AT-SPI (screen readers on Linux; no AT bridge on macOS,
        // §13).
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
        // An explicit role retypes the widget's accessible. GTK lets the `accessible-role`
        // property change only while the widget's AT context is unrealized, which it is until
        // the widget is rooted in a window (afterwards the setter g_criticals), and day-core
        // applies the annotations straight after `realize`, before the parent takes the
        // widget — so the first call lands the role and a rooted widget keeps the one it has.
        // A heading also carries its level, which AT-SPI reports as an attribute.
        let role = match a11y.role {
            Role::None => None,
            Role::Button => Some(AccessibleRole::Button),
            Role::Toggle => Some(AccessibleRole::Switch),
            Role::Slider => Some(AccessibleRole::Slider),
            Role::TextInput => Some(AccessibleRole::TextBox),
            Role::Heading(level) => {
                props.push(Property::Level(i32::from(level.max(1))));
                Some(AccessibleRole::Heading)
            }
            Role::Image => Some(AccessibleRole::Img),
            Role::Meter => Some(AccessibleRole::Meter),
            Role::Group => Some(AccessibleRole::Group),
            Role::Tree => Some(AccessibleRole::Tree),
            Role::TreeItem => Some(AccessibleRole::TreeItem),
            Role::Tab => Some(AccessibleRole::Tab),
            Role::TabList => Some(AccessibleRole::TabList),
        };
        if let Some(role) = role
            && h.root().is_none()
            && h.accessible_role() != role
        {
            h.set_accessible_role(role);
        }
        if !props.is_empty() {
            h.update_property(&props);
        }
        // The selected state, which AT-SPI reports for a tab or a row.
        if let Some(on) = a11y.selected {
            h.update_state(&[State::Selected(Some(on))]);
        }
        // Hidden drops the whole subtree from AT-SPI: children hang under a node the bridge
        // no longer presents.
        if a11y.hidden {
            h.update_state(&[State::Hidden(true)]);
        }
    }

    fn read_native(&self, h: &Handle) -> day_spec::NativeSnapshot {
        // The accessibility group stays unread (`found: false`): GtkAccessible offers setters
        // for the label, description and value but no getters, and the widget name answers the
        // type name when Day set none, so none of it reads back faithfully. The widget facts
        // below come from the controls' own getters.
        let mut snap = day_spec::NativeSnapshot {
            enabled: Some(h.is_sensitive()),
            visible: native_visible(h),
            frame: native_frame(h),
            ..Default::default()
        };
        if let Some(text) = picker::selected_text(h) {
            // A picker of any style: the selected option's label; `enabled` above is the
            // root's sensitivity, which every segmented/inline button inherits.
            snap.text = text;
        } else if let Some(label) = h.downcast_ref::<gtk4::Label>() {
            // `text` is the label without its Pango markup (the link runs, the attributes).
            snap.text = Some(label.text().to_string());
        } else if let Some(btn) = h.downcast_ref::<gtk4::Button>() {
            snap.text = button_text(btn);
            if let Some(toggle) = btn.downcast_ref::<gtk4::ToggleButton>() {
                snap.checked = Some(toggle.is_active());
            } else {
                // A Day button's selected state (`set_button_selected`): the `:checked` flag.
                snap.checked = Some(btn.state_flags().contains(gtk4::StateFlags::CHECKED));
            }
        } else if let Some(sw) = h.downcast_ref::<gtk4::Switch>() {
            snap.checked = Some(sw.is_active());
        } else if let Some(check) = h.downcast_ref::<gtk4::CheckButton>() {
            snap.checked = Some(check.is_active());
        } else if let Some(scale) = h.downcast_ref::<gtk4::Scale>() {
            // The adjustment holds Day's own min/max, so the value is in Day's range.
            snap.number = Some(scale.value());
        } else if let Some(bar) = h.downcast_ref::<gtk4::ProgressBar>() {
            // Day's progress value is the 0…1 fraction, which is what the bar stores.
            snap.number = Some(bar.fraction());
        } else if let Some(entry) = h.downcast_ref::<gtk4::Entry>() {
            // A secure field masks its characters, so its text is not reported.
            if gtk4::prelude::EntryExt::is_visible(entry) {
                snap.text = Some(entry.text().to_string());
            }
        } else {
            snap.text = textarea::text_of(h);
        }
        snap
    }

    fn announce(&mut self, text: &str, urgent: bool) {
        let Some(fixed) = self.window_fixed.as_ref() else {
            return;
        };
        announce_through(fixed.upcast_ref(), text, urgent);
    }

    fn request_frame(
        &mut self,
        host: &Handle,
        cb: day_spec::FrameCallback,
    ) -> day_spec::CancelFrame {
        let token = day_core::frame::native::register(cb);
        let fired = Rc::new(std::cell::Cell::new(false));
        let done = fired.clone();
        // GTK attaches its frame clock when the widget maps, and stops it while unmapped.
        let id = host.add_tick_callback(move |_, clock| {
            done.set(true);
            day_core::frame::native::deliver(
                token,
                day_spec::FrameStamp::new(clock.frame_time() as f64 / 1_000_000.0),
            );
            gtk4::glib::ControlFlow::Break
        });
        Box::new(move || {
            day_core::frame::native::cancel(token);
            if !fired.get() {
                id.remove();
            }
        })
    }

    fn replay(&mut self, h: &Handle, ops: &[DrawOp], _size: Size) {
        OPS.with(|t| t.insert(h.as_ptr() as usize, day_spec::encode_ops(ops)));
        h.queue_draw();
    }

    fn toggle_sidebar(&mut self, host: &Handle) -> bool {
        crate::toggle_sidebar(host)
    }
    /// The default PangoCairo font map — the one `create_layout` draws canvas text from — listed
    /// family by family, face by face (docs/fonts.md). Bundled fonts registered in `run` are in
    /// it; the map is created on first use, after that registration.
    fn font_families(&mut self) -> Vec<day_spec::FontFamilyInfo> {
        use gtk4::pango::prelude::*;
        let context = pangocairo::FontMap::default().create_context();
        let mut out = Vec::new();
        for family in context.list_families() {
            let faces = family
                .list_faces()
                .iter()
                .map(|face| {
                    let desc = face.describe();
                    let weight = day_weight(desc.weight());
                    let italic = desc.style() != gtk4::pango::Style::Normal;
                    let name = face.face_name().to_string();
                    day_spec::FontFace {
                        name: if name.is_empty() {
                            day_spec::FontFace::synthesized_name(weight, italic)
                        } else {
                            name
                        },
                        weight,
                        italic,
                    }
                })
                .collect();
            out.push(day_spec::FontFamilyInfo {
                family: family.name().to_string(),
                faces,
            });
        }
        out
    }

    /// A layout on the same font map `replay` draws from, at the same absolute size: its
    /// logical extents are the line box and its baseline the ascent.
    fn measure_text(
        &mut self,
        text: &str,
        size: f64,
        font: &day_spec::CanvasFont,
    ) -> Option<day_spec::TextMetrics> {
        let context = pangocairo::FontMap::default().create_context();
        let layout = gtk4::pango::Layout::new(&context);
        let pending = PendingFont {
            weight: f64::from(font.css_weight()),
            italic: font.italic,
            family: font.family_str().to_string(),
        };
        layout.set_font_description(Some(&canvas_font_desc(size, &pending)));
        layout.set_single_paragraph_mode(true);
        layout.set_text(text);
        // Pango hands back both boxes from one layout; the ink one used to be discarded.
        let (ink, logical) = layout.extents();
        let scale = f64::from(gtk4::pango::SCALE);
        let px = |v: i32| f64::from(v) / scale;
        let ascent = px(layout.baseline());
        // Pango has no cap-height metric. The ink top of a capital IS the cap height, and asking
        // for it is one more measurement of a one-character string — memoized per (size, font)
        // like every other, so a whole process pays for it once (docs/fonts.md "It is cached").
        let cap = {
            let probe = gtk4::pango::Layout::new(&context);
            probe.set_font_description(layout.font_description().as_ref());
            probe.set_single_paragraph_mode(true);
            probe.set_text("H");
            let (pink, _) = probe.extents();
            px(probe.baseline()) - px(pink.y())
        };
        Some(day_spec::TextMetrics {
            width: px(logical.width()),
            height: px(logical.height()),
            ascent,
            cap_height: cap,
            // Pango's extents are already relative to the LOGICAL box's top-leading corner, which
            // is exactly this field's origin — no baseline shift to undo.
            ink: day_spec::Rect::new(px(ink.x()), px(ink.y()), px(ink.width()), px(ink.height())),
        })
    }

    /// Decode bytes into a `GdkTexture` (docs/images.md), which reads whatever gdk-pixbuf's
    /// loaders read — PNG, JPEG, TIFF, BMP, GIF and, where the platform ships the loader, WebP.
    fn decode_image(&mut self, req: u64, id: day_spec::BitmapId, bytes: &[u8]) {
        let data = gtk4::glib::Bytes::from_owned(bytes.to_vec());
        let event = match gtk4::gdk::Texture::from_bytes(&data) {
            Ok(texture) => {
                let pixbuf = gtk4::gdk::pixbuf_get_from_texture(&texture);
                let info = day_spec::BitmapInfo {
                    pixels: Size::new(f64::from(texture.width()), f64::from(texture.height())),
                    // Decoded bytes carry no density — a PNG is simply its pixels.
                    scale: 1.0,
                    format: day_spec::ImageFormat::sniff(bytes),
                    // Unknown counts as "may have alpha": compositing an opaque image as if it
                    // had an alpha channel is harmless, the reverse loses transparency.
                    has_alpha: pixbuf.as_ref().is_none_or(|p| p.has_alpha()),
                };
                BITMAPS.with(|m| {
                    m.borrow_mut().insert(
                        id.0,
                        GtkBitmap {
                            texture,
                            pixbuf,
                            info,
                        },
                    )
                });
                Event::ImageDecoded {
                    req,
                    result: Ok(info),
                }
            }
            Err(_) => Event::ImageDecoded {
                req,
                result: Err(day_spec::ImageError::Decode),
            },
        };
        emit(day_spec::WINDOW_NODE, event);
    }

    fn image_info(&mut self, id: day_spec::BitmapId) -> Option<day_spec::BitmapInfo> {
        BITMAPS.with(|m| m.borrow().get(&id.0).map(|b| b.info))
    }

    fn encode_image(&mut self, req: u64, id: day_spec::BitmapId, spec: &day_spec::EncodeSpec) {
        let result = encode_bitmap(id, spec);
        emit(day_spec::WINDOW_NODE, Event::ImageEncoded { req, result });
    }

    /// What gdk-pixbuf WRITES. It reads more than this — GIF and often WebP — and the asymmetry
    /// is the library's own, which is why this duty exists rather than letting `Cap::ImageEncode`
    /// imply that everything decodable is also encodable.
    fn encode_formats(&mut self) -> Vec<day_spec::ImageFormat> {
        use day_spec::ImageFormat::{Bmp, Jpeg, Png, Tiff};
        vec![Png, Jpeg, Tiff, Bmp]
    }

    fn release_image(&mut self, id: day_spec::BitmapId) {
        release_bitmap(id);
    }

    /// Replay `ops` through `cairo_draw` — the canvas's own painter — into a cairo image surface,
    /// then keep the pixels as a texture under `id`, beside the decoded bitmaps
    /// (docs/canvas.md "Exporting").
    fn render_bitmap(
        &mut self,
        req: u64,
        id: day_spec::BitmapId,
        ops: &[DrawOp],
        size: Size,
        scale: f64,
    ) {
        let result = render_bitmap_to(id, ops, size, scale).map_err(day_spec::ExportError::Failed);
        emit(day_spec::WINDOW_NODE, Event::BitmapRendered { req, result });
    }

    /// One cairo PDF surface, resized before each page and painted by `cairo_draw`, so shapes
    /// stay vector and Pango text is embedded as font subsets (docs/canvas.md "Exporting").
    fn render_pdf(&mut self, req: u64, pages: &[day_spec::PdfPage], info: &day_spec::PdfInfo) {
        let result = render_pdf_bytes(pages, info).map_err(day_spec::ExportError::Failed);
        emit(day_spec::WINDOW_NODE, Event::PdfRendered { req, result });
    }

    fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
        // A PLAIN render of the widget tree — no main-loop iteration. This runs inside the
        // engine's tree borrow (`with_tree`), and pumping GLib here can dispatch a callback
        // that re-enters the tree ("RefCell already borrowed" abort seen on the macos-gtk CI).
        // The capture-after-paint gating that used to live here is `ui_idle` below: the
        // screenshot step polls it (retryable, from the socket thread) so the main loop runs
        // FREELY between polls until the pending layout/draw has actually happened.
        let fixed = self.window_fixed.as_ref().ok_or("no window")?;
        snapshot_widget(fixed.upcast_ref())
    }

    fn snapshot_window_chrome(&mut self) -> Result<Vec<u8>, String> {
        // GTK draws its own decorations (CSD), so the HeaderBar is a widget inside the window —
        // rendering the root instead of the content Fixed is the whole difference between the
        // two captures (docs/window-image.md). The compositor's drop shadow stays out either way.
        let fixed = self.window_fixed.as_ref().ok_or("no window")?;
        let root = fixed.root().ok_or("no window root")?;
        snapshot_widget(root.upcast_ref())
    }

    fn snapshot_window_of(&mut self, host: &Handle) -> Result<Vec<u8>, String> {
        if let Some(h) = dialog_host_for(host) {
            let sheet = h.capture_widget().ok_or("dialog not presented")?;
            return snapshot_widget(&sheet);
        }
        snapshot_widget(host)
    }

    fn native_document_tabs(&self) -> bool {
        true
    }

    fn open_window(
        &mut self,
        id: NodeId,
        options: &WindowOptions,
        kind: day_spec::WindowKind,
    ) -> day_spec::WindowOpenReply<Handle> {
        // Always present at call time: open_window runs after `ready`, which runs inside
        // `activate` (AdwApplicationWindow::new only asserts GTK-initialized + main thread).
        let Some(app) = self.app.clone() else {
            return day_spec::WindowOpenReply::Unsupported;
        };
        // A panel over an app window is a dialog in it (`DialogHost`); with no window to
        // present over (nothing is up yet) it is a window of its own, below.
        if kind.is_panel()
            && let Some(parent) = self.active_window()
        {
            let host = Rc::new(DialogHost {
                node: id,
                kind,
                fixed: gtk4::Fixed::new(),
                parent,
                dialog: RefCell::new(None),
                title: RefCell::new(options.title.clone()),
                size: std::cell::Cell::new(options.size),
                presented: std::cell::Cell::new(false),
                presented_at: std::cell::Cell::new(None),
                closed: std::cell::Cell::new(false),
            });
            DIALOGS.with(|d| d.borrow_mut().push(host.clone()));
            // About content is Day's own; Preferences content decides at its first mount
            // (the settings shell claims an `AdwPreferencesDialog` before that).
            if kind == day_spec::WindowKind::About {
                host.materialize_plain();
            }
            return day_spec::WindowOpenReply::Open(host.fixed.clone().upcast());
        }
        let (window, fixed, _header) = build_day_window(&app, options, Some(id));
        // An overlay title bar's height is the content's top inset, there before it builds.
        shell::report_overlay_inset(window.upcast_ref(), Some(id));
        if kind.is_panel() {
            window.set_resizable(false);
        }
        window.connect_close_request(move |_| {
            // Confirm to day-core, which tears down on a DEFERRED hop (never inside this
            // close frame); GTK proceeds with the destroy — the widgets stay alive as
            // Rust refs until day-core releases them.
            ffi_guard::contain(gtk4::glib::Propagation::Proceed, || {
                emit(id, Event::WindowClosed);
                gtk4::glib::Propagation::Proceed
            })
        });
        {
            let app = app.clone();
            window.connect_is_active_notify(move |w| {
                ffi_guard::contain((), || {
                    note_last_active(w.upcast_ref());
                    emit(id, Event::WindowFocused(w.is_active()));
                    note_activation_changed(&app);
                });
            });
        }
        // The app-menu actions resolve against the FOCUSED window (macOS global bar and
        // accelerators both) — without the group, menu items go dead while this window
        // is key.
        if let Some(group) = &self.menu_group {
            window.insert_action_group("daymenu", Some(group));
        }
        if !options.start_hidden {
            window.present();
        }
        self.secondary.push(GtkWin {
            window,
            fixed: fixed.clone(),
        });
        day_spec::WindowOpenReply::Open(fixed.upcast())
    }

    fn fit_window(&mut self, host: &Handle, size: Size) {
        if let Some(h) = dialog_host_for(host) {
            h.size.set(size);
            h.apply_size();
            return;
        }
        // A panel that fits its content (docs/windows.md `size_to_fit`): the default size is
        // the size GTK gives a window it has not been asked to resize, mapped or not, so
        // resetting it to the content plus the header bar resizes the live window. The
        // `default-height` notify then reports the new content area back to day-core.
        if let Some(w) = self.gtk_window_for(host) {
            let bar = shell::bar_height(&w);
            w.set_default_size(
                size.width.round() as i32,
                (size.height + bar).round() as i32,
            );
        }
    }

    fn close_window(&mut self, host: &Handle) {
        if let Some(h) = dialog_host_for(host) {
            h.close();
            return;
        }
        // The primary too (`gtk_window_for`): the initial window is an ordinary window
        // (docs/windows.md), and `close()` takes it through `close-request`, so a scripted close
        // ends the app exactly as the title bar's button does.
        if let Some(w) = self.gtk_window_for(host) {
            w.close();
        }
    }

    fn focus_window(&mut self, host: &Handle) {
        if let Some(h) = dialog_host_for(host) {
            // Presenting a presented dialog again brings it forward and refocuses it (the
            // open-or-focus half of the preferences singleton).
            if h.presented.get()
                && let Some(d) = h.dialog.borrow().clone()
            {
                d.present(panel_parent(h.parent.upcast_ref()));
            }
            return;
        }
        if let Some(w) = self.gtk_window_for(host) {
            w.present();
        }
    }

    fn set_window_title(&mut self, host: &Handle, title: &str) {
        if let Some(h) = dialog_host_for(host) {
            *h.title.borrow_mut() = title.to_string();
            if let Some(d) = h.dialog.borrow().clone() {
                d.set_title(title);
            }
            return;
        }
        if let Some(w) = self
            .secondary
            .iter()
            .find(|w| w.fixed.upcast_ref::<gtk4::Widget>() == host)
        {
            w.window.set_title(Some(title));
            return;
        }
        // The primary is an ordinary window (docs/windows.md): `day::window_title` in the first
        // window's shell arrives with the primary's own content, and the GNOME window list and
        // taskbars label a window by exactly this.
        if self
            .window_fixed
            .as_ref()
            .is_some_and(|f| f.upcast_ref::<gtk4::Widget>() == host)
            && let Some(win) = self
                .window_fixed
                .as_ref()
                .and_then(|f| f.root())
                .and_then(|r| r.downcast::<gtk4::Window>().ok())
        {
            win.set_title(Some(title));
        }
    }

    fn apply_window(&mut self, host: &Handle, change: &day_spec::WindowChange) {
        use day_spec::WindowChange as C;
        // A dialog has no window properties of its own: every change is ignored, which the
        // `Cap`s named on `WindowChange` already allow for.
        if dialog_host_for(host).is_some() {
            return;
        }
        let Some(window) = self.gtk_window_for(host) else {
            return;
        };
        match change {
            C::State(state) => set_gtk_window_state(&window, *state),
            // No Linux display server (X11, Wayland) lets a client keep its surface out of
            // screenshots or screen sharing, so `Cap::ContentProtection` is Unsupported and the
            // request is ignored.
            C::ContentProtected(_) => {}
            other => shell::apply_property(&window, other),
        }
    }

    fn set_drag_region(&mut self, h: &Handle, drag: bool) {
        shell::set_drag_region(h, drag);
    }

    fn set_status_items(&mut self, items: &[day_spec::StatusItemSpec]) {
        shell::set_status_items(items);
    }

    fn set_keep_running(&mut self, keep: bool) {
        shell::set_keep_running(self.app.as_ref(), keep);
    }

    fn set_app_progress(&mut self, progress: day_spec::AppProgress) {
        shell::set_progress(progress);
    }

    fn set_app_badge(&mut self, badge: &day_spec::AppBadge) {
        shell::set_badge(badge);
    }

    fn quit_app(&mut self) {
        if let Some(app) = &self.app {
            app.quit();
        }
    }

    fn monitors(&self) -> Vec<day_spec::Monitor> {
        shell::monitors()
    }

    fn prepare_snapshot(
        &mut self,
        host: Option<&Self::Handle>,
        revision: u32,
    ) -> Result<day_spec::capture::Readiness, String> {
        use gtk4::prelude::*;
        // A dialog-hosted panel paints on the window's frame clock through its dialog; its
        // content container is not what is on screen. Not before the dialog has animated in.
        let dialog_host = host.and_then(dialog_host_for);
        if let Some(h) = &dialog_host
            && !h.settled()
        {
            return Ok(day_spec::capture::Readiness::Pending);
        }
        let widget = dialog_host
            .and_then(|h| {
                h.dialog
                    .borrow()
                    .clone()
                    .map(|d| d.upcast::<gtk4::Widget>())
            })
            .or_else(|| host.cloned())
            .or_else(|| self.window_fixed.as_ref().map(|w| w.clone().upcast()))
            .ok_or("no window to capture")?;
        let Some(clock) = widget.frame_clock() else {
            return Ok(day_spec::capture::Readiness::Pending);
        };
        thread_local! {
            static FENCES: std::cell::RefCell<std::collections::HashMap<usize,
                (gtk4::glib::WeakRef<gtk4::Widget>, day_spec::capture::Fence)>> = Default::default();
        }
        let fence = FENCES.with(|f| {
            let mut f = f.borrow_mut();
            f.retain(|_, (w, _)| w.upgrade().is_some());
            f.entry(widget.as_ptr() as usize)
                .or_insert_with(|| (widget.downgrade(), Default::default()))
                .1
                .clone()
        });
        if fence.begin(revision) {
            let done = fence.clone();
            let handler = std::rc::Rc::new(std::cell::RefCell::new(None));
            let remove = handler.clone();
            *handler.borrow_mut() = Some(clock.connect_after_paint(move |clock| {
                if let Some(id) = remove.borrow_mut().take() {
                    clock.disconnect(id);
                }
                done.complete(revision, Ok(()));
            }));
            widget.queue_draw();
            clock.request_phase(gtk4::gdk::FrameClockPhase::PAINT);
        }
        fence.poll()
    }

    fn present(&mut self, req: u64, spec: &day_spec::present::PresentSpec) {
        use day_spec::present::{ButtonRole, PresentResult, PresentSpec};
        // AdwDialog presents relative to any widget inside its AdwApplicationWindow.
        // Dialogs attach to the ACTIVE Day window at present time (docs/windows.md);
        // primary is the fallback.
        let parent = self
            .secondary
            .iter()
            .find(|w| w.window.is_active())
            .map(|w| w.fixed.clone())
            .or_else(|| self.window_fixed.clone());
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
                        ffi_guard::contain((), || {
                            let result = resp
                                .parse::<i64>()
                                .map(PresentResult::Button)
                                .unwrap_or(PresentResult::Dismissed);
                            finish(result);
                        });
                    });
                }
                NAV_DIALOGS.with(|m| m.borrow_mut().insert(req, DialogHandle { finish }));
                dialog.present(parent.as_ref().and_then(|p| dialog_parent(p.upcast_ref())));
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
                        ffi_guard::contain((), || {
                            let result = if resp == "ok" {
                                PresentResult::Text(entry.text().to_string())
                            } else {
                                PresentResult::Dismissed
                            };
                            finish(result);
                        });
                    });
                }
                NAV_DIALOGS.with(|m| m.borrow_mut().insert(req, DialogHandle { finish }));
                dialog.present(parent.as_ref().and_then(|p| dialog_parent(p.upcast_ref())));
            }
            // GtkFileDialog (GTK 4.10+): async open/save with a native GTK picker. The chosen
            // GFile's local path crosses back; a Cancellable lets dismiss() cancel it.
            // Presenting a modal GtkFileDialog pumps the GTK main loop (mapping its window — a
            // synchronous round-trip under a headless/xvfb display). But day-core calls this
            // `present()` While holding the tree borrow (`with_tree(|t| t.present(..))` in
            // present.rs), so a loop-spin here re-enters Day (e.g. the on-main dayscript engine's
            // next step) and its `with_tree` panics "already borrowed" — inside a GTK C callback,
            // which aborts rather than unwinds. So DEFER the actual open/save to an idle: by then
            // `present()` has returned and the borrow is released, making any re-entry safe. (Same
            // reasoning as `schedule_list_resize`.)
            PresentSpec::OpenFile {
                title,
                filters,
                multiple,
            } => {
                let dialog = gtk4::FileDialog::builder()
                    .title(title.as_str())
                    .modal(true)
                    .build();
                apply_gtk_filters(&dialog, filters);
                let cancellable = gtk4::gio::Cancellable::new();
                FILE_DIALOGS.with(|m| m.borrow_mut().insert(req));
                let window = file_dialog_window(&parent);
                let multiple = *multiple;
                gtk4::glib::idle_add_local_once(move || {
                    if !claim_file_dialog(req) {
                        return;
                    }
                    if multiple {
                        // `open_multiple` answers a GListModel of GFiles: every chosen path
                        // crosses back, in the picker's order.
                        dialog.open_multiple(window.as_ref(), Some(&cancellable), move |res| {
                            ffi_guard::contain((), || {
                                emit_file_result(req, res.map(|files| gfile_paths(&files)))
                            })
                        });
                    } else {
                        dialog.open(window.as_ref(), Some(&cancellable), move |res| {
                            ffi_guard::contain((), || {
                                emit_file_result(req, res.map(|f| f.path().into_iter().collect()))
                            })
                        });
                    }
                });
            }
            // The folder chooser: the same GtkFileDialog, in its `select_folder` mode.
            PresentSpec::OpenFolder { title } => {
                let dialog = gtk4::FileDialog::builder()
                    .title(title.as_str())
                    .modal(true)
                    .build();
                let cancellable = gtk4::gio::Cancellable::new();
                FILE_DIALOGS.with(|m| m.borrow_mut().insert(req));
                let window = file_dialog_window(&parent);
                gtk4::glib::idle_add_local_once(move || {
                    if !claim_file_dialog(req) {
                        return;
                    }
                    dialog.select_folder(window.as_ref(), Some(&cancellable), move |res| {
                        ffi_guard::contain((), || {
                            emit_file_result(req, res.map(|f| f.path().into_iter().collect()))
                        })
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
                FILE_DIALOGS.with(|m| m.borrow_mut().insert(req));
                let window = file_dialog_window(&parent);
                // The pieces layer copies the staged bytes to the chosen local path.
                gtk4::glib::idle_add_local_once(move || {
                    if !claim_file_dialog(req) {
                        return;
                    }
                    dialog.save(window.as_ref(), Some(&cancellable), move |res| {
                        ffi_guard::contain((), || {
                            emit_file_result(req, res.map(|f| f.path().into_iter().collect()))
                        })
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
        end_file_dialog(req);
    }

    #[cfg(target_os = "macos")]
    fn share_support(&self) -> day_spec::Support {
        day_spec::Support::Native
    }
    #[cfg(target_os = "macos")]
    fn share_url(&mut self, url: &str, _title: &str) -> bool {
        use objc2::{msg_send, runtime::AnyObject};
        use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString, NSURL};
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return false;
        };
        let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) else {
            return false;
        };
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        let Some(view) = app.keyWindow().and_then(|w| w.contentView()) else {
            return false;
        };
        unsafe {
            let items = NSArray::from_retained_slice(&[url]);
            let picker: objc2::rc::Allocated<AnyObject> =
                msg_send![objc2::class!(NSSharingServicePicker), alloc];
            let picker: objc2::rc::Retained<AnyObject> = msg_send![picker, initWithItems: &*items];
            let bounds = view.bounds();
            let y = if view.isFlipped() {
                30.0
            } else {
                bounds.size.height - 30.0
            };
            let rect = NSRect::new(
                NSPoint::new(bounds.size.width - 30.0, y),
                NSSize::new(1.0, 1.0),
            );
            let _: () = msg_send![&*picker, showRelativeToRect: rect, ofView: &*view, preferredEdge: 3usize];
            self.sharing = Some(picker);
        }
        true
    }
    #[cfg(windows)]
    fn share_support(&self) -> day_spec::Support {
        day_spec::Support::Native
    }
    #[cfg(windows)]
    fn share_url(&mut self, url: &str, title: &str) -> bool {
        day_windows_runtime::share::share(url, title)
    }
    fn open_url(&mut self, url: &str) {
        // Hand the URI to the desktop's default handler (xdg-open equivalent). Fire and forget;
        // a bad URI just returns an error we ignore.
        let _ =
            gtk4::gio::AppInfo::launch_default_for_uri(url, None::<&gtk4::gio::AppLaunchContext>);
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

/// A file picker between `present()` and its result. `present()` records it as queued and defers
/// the picker itself to an idle (see the OpenFile arm); the idle then `claim`s it.
/// Take ownership of a deferred picker on behalf of GTK: `true` to open it, `false` if the
/// request was answered while the idle sat in the queue (`dismiss` removed the entry).
///
/// Answering that fast is ordinary — a dayscript `respond` does it, and so does any app that
/// resolves its own presentation — and the picker must NOT open then: Day considers the request
/// closed, so a window appearing after it is a ghost whose result belongs to nothing. GTK gives
/// a second reason to skip it. Opening with an already-cancelled GCancellable leaves the
/// operation in a state GTK never completes: it drops the callback outright on macOS, and on
/// Linux (GTK 4.14) its completion runs against portal data the cancel already freed and
/// dereferences null — the SIGSEGV that took down the linux-gtk walkthrough.
fn claim_file_dialog(req: u64) -> bool {
    FILE_DIALOGS.with(|m| m.borrow().contains(&req))
}

/// Drop a file picker on dismissal — without cancelling it, whether it is queued or already
/// shown. Removing the entry is the whole mechanism: a queued picker's idle then skips opening
/// it (`claim_file_dialog`), and a shown one's eventual result is dropped by `emit_file_result`.
///
/// Cancelling is what we must not do. `claim_file_dialog` already records that cancelling a
/// QUEUED picker crashes GTK 4.14; a SHOWN one turned out to be no safer, and that was the
/// linux-gtk CI segfault — reproduced 8 runs out of 8 at the walkthrough's `btn-save-file`
/// step, and gone in the same harness once the cancel is removed. GTK's own frames confirm it:
/// the faulting pc sits inside the file-chooser implementation, reached from the completion of
/// the operation the cancel tore down.
///
/// The cost is that a picker the APP dismisses programmatically stays on screen until the user
/// closes it; its answer is then ignored. That is the same bargain already struck for queued
/// pickers, and a lingering window beats taking the process down.
fn end_file_dialog(req: u64) {
    FILE_DIALOGS.with(|m| {
        m.borrow_mut().remove(&req);
    });
}

/// Turn a GtkFileDialog result into a `PresentResult` and enqueue it.
///
/// A result for a request that is no longer live has been answered already — `dismiss` removed
/// the entry — so it is dropped rather than delivered: without that, a picker the app dismissed
/// could still hand the app a file the user chose afterwards.
fn emit_file_result(req: u64, res: Result<Vec<std::path::PathBuf>, gtk4::glib::Error>) {
    let live = FILE_DIALOGS.with(|m| m.borrow_mut().remove(&req));
    if !live {
        return;
    }
    let result = match res {
        Ok(paths) if !paths.is_empty() => day_spec::present::PresentResult::Files(
            paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
        ),
        _ => day_spec::present::PresentResult::Dismissed,
    };
    emit(day_spec::WINDOW_NODE, Event::PresentResult { req, result });
}

/// The local paths of a `GtkFileDialog.open_multiple` answer: a GListModel of GFiles, in the
/// picker's order. A file with no local path (a remote GVFS mount) is skipped.
fn gfile_paths(files: &gtk4::gio::ListModel) -> Vec<std::path::PathBuf> {
    (0..files.n_items())
        .filter_map(|i| files.item(i))
        .filter_map(|o| o.downcast::<gtk4::gio::File>().ok())
        .filter_map(|f| f.path())
        .collect()
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
    let closed = Rc::new(std::cell::Cell::new(false));
    let did_close = closed.clone();
    dialog.connect_closed(move |_| did_close.set(true));
    // The response signal retains this finisher; keeping the dialog strongly here
    // would keep both alive after the native host removes it.
    let dialog = dialog.downgrade();
    Rc::new(move |result| {
        if answered.replace(true) {
            return;
        }
        emit(day_spec::WINDOW_NODE, Event::PresentResult { req, result });
        NAV_DIALOGS.with(|m| {
            m.borrow_mut().remove(&req);
        });
        let Some(dialog) = dialog.upgrade() else {
            return;
        };
        dialog.close();
        if !closed.get() {
            // libadwaita 1.5 opens the sheet on its first mapped frame ticks. Closing
            // before those ticks sets an already-closed sheet to closed, then the pending
            // open runs anyway, leaving the dialog host's content unfocusable. Replay the
            // close on frame ticks until the native `closed` signal confirms completion.
            // No strong widget capture: disposal also drops this callback.
            let closed = closed.clone();
            dialog.add_tick_callback(move |dialog, _| {
                ffi_guard::contain(gtk4::glib::ControlFlow::Break, || {
                    if closed.get() || dialog.root().is_none() {
                        return gtk4::glib::ControlFlow::Break;
                    }
                    dialog.close();
                    if closed.get() {
                        gtk4::glib::ControlFlow::Break
                    } else {
                        gtk4::glib::ControlFlow::Continue
                    }
                })
            });
        }
    })
}

/// Adwaita's default header-bar height, used to size Day's content area before the header is
/// first allocated (`report_content_size` reads the real height thereafter).
const HEADER_H: f64 = 47.0;

/// Report the content wrapper's actual allocation on every resize. Window default sizes
/// include client-side decoration on some GTK/X11 configurations.
/// `target`: `None` = the primary window (`WINDOW_NODE` + the cover follow-along);
/// `Some(node)` = a secondary window's root (docs/windows.md).
fn report_content_size(size: Size, target: Option<NodeId>) {
    match target {
        Some(node) => emit(node, Event::WindowResized(size)),
        None => {
            emit(day_spec::WINDOW_NODE, Event::WindowResized(size));
            // Presented covers track the content area (their frame is native-owned).
            COVERS.with(|c| {
                for (cover, node) in c.borrow().iter() {
                    let (y, size) = cover_frame(cover, size);
                    if let Some(root) = cover
                        .parent()
                        .and_then(|p| p.downcast::<gtk4::Fixed>().ok())
                    {
                        root.move_(cover, 0.0, y);
                    }
                    cover.set_size_request(size.width as i32, size.height as i32);
                    emit(*node, Event::FrameChanged(size));
                }
            });
        }
    }
}

/// A presented cover's place in a content area `area` large: the whole area, or the bottom
/// half for a half sheet (docs/cover.md). Answers the y offset and the cover's own size.
fn cover_frame(cover: &gtk4::Fixed, area: Size) -> (f64, Size) {
    let half = COVER_HALF.with(|t| t.get(widget_key(&cover.clone().upcast())).is_some());
    if half {
        let h = (area.height * 0.5).floor();
        (area.height - h, Size::new(area.width, h))
    } else {
        (0.0, area)
    }
}

/// Render a widget subtree to PNG (the dayscript screenshot seam — see `snapshot_window`
/// for why no main-loop iteration may happen here).
fn snapshot_widget(widget: &gtk4::Widget) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "macos")]
    if let Ok(png) = snapshot_macos::snapshot(widget) {
        return Ok(png);
    }
    let w = widget.width() as f64;
    let h = widget.height() as f64;
    if w <= 0.0 || h <= 0.0 {
        return Err("zero-size window".into());
    }
    use gtk4::gdk::prelude::PaintableExt;
    let paintable = gtk4::WidgetPaintable::new(Some(widget));
    let snapshot = gtk4::Snapshot::new();
    // A stated capture scale (`DAY_CAPTURE_SCALE`, Day.toml `[screenshots]`): render the node
    // tree through a scale transform, so the texture is the widget's points times the scale on
    // any display, a 1x xvfb included. The viewport is then the WHOLE widget rather than the
    // node's drawn bounds (which stop at the last thing painted and made capture widths vary),
    // over the window background the content Fixed does not paint itself. Together they make
    // the capture exactly the size the run asked for.
    //
    // The widget is rendered on its own FIRST: before its first frame it yields no node at all,
    // and that error is what makes the screenshot step retry until there is something to
    // capture. Filling the background into the same snapshot would hide it behind a node that
    // is never empty, and the run's first capture would come back a blank rectangle.
    paintable.snapshot(&snapshot, w, h);
    let content = snapshot.to_node().ok_or("empty render node")?;
    let scale = day_spec::capture_scale();
    let node = match scale {
        Some(s) => {
            let scaled = gtk4::Snapshot::new();
            scaled.scale(s as f32, s as f32);
            scaled.append_color(
                &window_background(),
                &gtk4::graphene::Rect::new(0.0, 0.0, w as f32, h as f32),
            );
            scaled.append_node(&content);
            scaled.to_node().ok_or("empty render node")?
        }
        None => content,
    };
    let native = widget.native().ok_or("no native")?;
    let renderer = native.renderer().ok_or("no renderer")?;
    let viewport =
        scale.map(|s| gtk4::graphene::Rect::new(0.0, 0.0, (w * s) as f32, (h * s) as f32));
    let texture = renderer.render_texture(&node, viewport.as_ref());
    Ok(texture.save_to_png_bytes().to_vec())
}

/// libadwaita's `window_bg_color` for the current appearance: what shows through Day's
/// transparent content area on screen, and so what a whole-widget capture is filled with.
/// The stylesheet's own values (libadwaita 1.5+), since GTK 4.10 deprecated the style-context
/// lookup that could ask for them.
fn window_background() -> gtk4::gdk::RGBA {
    let (r, g, b): (u8, u8, u8) = if adw::StyleManager::default().is_dark() {
        (0x22, 0x22, 0x26)
    } else {
        (0xfa, 0xfa, 0xfb)
    };
    gtk4::gdk::RGBA::new(
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        1.0,
    )
}

/// Build one Day window: AdwApplicationWindow + ToolbarView/HeaderBar chrome + the filling
/// `DayCell` wrapper + the GtkFixed content (docs/windows.md; see the wrapper comments below —
/// factored so `open_window` builds identical chrome).
/// Wires the resize notifies to `target` (`None` = primary).
fn build_day_window(
    app: &adw::Application,
    options: &WindowOptions,
    target: Option<NodeId>,
) -> (adw::ApplicationWindow, gtk4::Fixed, adw::HeaderBar) {
    let window = adw::ApplicationWindow::new(app);
    window.set_title(Some(&options.title));
    window.set_default_size(options.size.width as i32, options.size.height as i32);
    let fixed = gtk4::Fixed::new();
    // A GtkFixed reports its children's bounding box as its MINIMUM size, which would pin the
    // window at the content size. A filling `DayCell` around it asks for nothing, so the window
    // resizes freely and Day sizes the content on every resize — and unlike the External-policy
    // scroll window that used to do this, nothing here can ever pan the UI under a wheel.
    let wrapper = DayCell::filling();
    wrapper.add_child(fixed.upcast_ref());
    // AdwApplicationWindow carries no titlebar of its own; an AdwToolbarView supplies an
    // AdwHeaderBar (window controls, drag handle, and the window title) above Day's
    // content — the standard Adwaita window structure, and the AdwDialog host that
    // AdwAlertDialog needs.
    let header = adw::HeaderBar::new();
    let toolbar = adw::ToolbarView::new();
    toolbar.set_content(Some(&wrapper));
    // The chrome (docs/window-chrome.md): the header bar for `Standard` (a day toolbar packs
    // into it, docs/toolbars.md), window controls over the content for `Overlay`, nothing for
    // `Frameless`; plus resizability, limits, background and shadow.
    shell::dress(&window, &toolbar, &header, options);
    // Observe the same content allocation that snapshot_window captures. Inferring it from
    // default-width/height counts decoration borders, shifting even precise pixel samples.
    let last = std::cell::Cell::new((0, 0));
    wrapper.on_allocate(move |width, height| {
        if width > 0 && height > 0 && last.replace((width, height)) != (width, height) {
            ffi_guard::contain((), || {
                report_content_size(Size::new(f64::from(width), f64::from(height)), target)
            });
        }
    });
    watch_window_state(window.upcast_ref(), target.unwrap_or(day_spec::WINDOW_NODE));
    (window, fixed, header)
}

/// One window's display-state bookkeeping (docs/windows.md "Window properties").
///
/// Wayland has no minimized state: `xdg_toplevel.set_minimized` is a one-way request, and the
/// compositor never tells the client it was minimized or restored, so GDK's Wayland backend
/// never sets `GDK_TOPLEVEL_STATE_MINIMIZED`. On X11 (`_NET_WM_STATE_HIDDEN`), macOS and Windows
/// it does. So on Wayland a minimize THIS backend asked for is assumed to have landed
/// (`assumed_minimized`), and the window's next activation clears it, since a minimized window
/// cannot hold focus. A minimize the user makes through the compositor (a shortcut, the dock)
/// is invisible there and leaves the reported state where it was.
struct StateTrack {
    /// Where reports go: the window's root node, or `WINDOW_NODE` for the primary.
    target: NodeId,
    /// The `WindowState::code()` last reported.
    last: std::cell::Cell<i32>,
    /// A minimize requested on a display that never reports one (Wayland).
    assumed_minimized: std::cell::Cell<bool>,
}

thread_local! {
    /// Every Day window's [`StateTrack`], by window. Entries go with their window.
    static STATE_TRACKS: RefCell<Vec<(gtk4::glib::WeakRef<gtk4::Window>, Rc<StateTrack>)>> =
        const { RefCell::new(Vec::new()) };
}

/// `window`'s [`StateTrack`], if it is a Day window.
fn state_track(window: &gtk4::Window) -> Option<Rc<StateTrack>> {
    STATE_TRACKS.with(|t| {
        t.borrow()
            .iter()
            .find(|(w, _)| w.upgrade().as_ref() == Some(window))
            .map(|(_, s)| s.clone())
    })
}

/// Whether this window's display ever reports a minimized toplevel (every GDK backend but
/// Wayland; see [`StateTrack`]). Read by type name, which needs no backend-specific crate.
fn reports_minimized(window: &gtk4::Window) -> bool {
    WidgetExt::display(window).type_().name() != "GdkWaylandDisplay"
}

/// What `window` is showing as: minimized wins (GTK keeps `maximized` set underneath it), then
/// fullscreen (a fullscreen window may also be maximized), then maximized.
fn gtk_window_state(window: &gtk4::Window, track: &StateTrack) -> day_spec::WindowState {
    use day_spec::WindowState as S;
    let minimized = track.assumed_minimized.get()
        || window
            .surface()
            .and_then(|s| s.downcast::<gtk4::gdk::Toplevel>().ok())
            .is_some_and(|t| t.state().contains(gtk4::gdk::ToplevelState::MINIMIZED));
    if minimized {
        S::Minimized
    } else if window.is_fullscreen() {
        S::Fullscreen
    } else if window.is_maximized() {
        S::Maximized
    } else {
        S::Normal
    }
}

/// Report `window`'s display state if it changed since the last report, or regardless when
/// `force` (a settle after a request, so a declined one reads back as the state the window
/// kept; day-core drops a report that matches its signal).
fn report_window_state(window: &gtk4::Window, force: bool) {
    let Some(track) = state_track(window) else {
        return;
    };
    let state = gtk_window_state(window, &track);
    let changed = track.last.replace(state.code()) != state.code();
    if changed || force {
        let how = if changed { "now" } else { "settled at" };
        log::debug!("gtk: window state {how} {state:?}");
        emit(track.target, Event::WindowStateChanged(state));
    }
}

/// Follow `window`'s display state and report each change to `target` (docs/windows.md): the
/// `maximized` and `fullscreened` properties, the toplevel surface's state for minimized (the
/// surface exists from realize on), and activation, which ends an assumed Wayland minimize.
fn watch_window_state(window: &gtk4::Window, target: NodeId) {
    STATE_TRACKS.with(|t| {
        let mut t = t.borrow_mut();
        t.retain(|(w, _)| w.upgrade().is_some());
        t.push((
            window.downgrade(),
            Rc::new(StateTrack {
                target,
                last: std::cell::Cell::new(day_spec::WindowState::Normal.code()),
                assumed_minimized: std::cell::Cell::new(false),
            }),
        ));
    });
    window.connect_maximized_notify(|w| ffi_guard::contain((), || report_window_state(w, false)));
    window
        .connect_fullscreened_notify(|w| ffi_guard::contain((), || report_window_state(w, false)));
    window.connect_is_active_notify(|w| {
        ffi_guard::contain((), || {
            note_last_active(w);
            if w.is_active()
                && let Some(track) = state_track(w)
            {
                track.assumed_minimized.set(false);
            }
            report_window_state(w, false);
        })
    });
    window.connect_realize(|w| {
        ffi_guard::contain((), || {
            let Some(toplevel) = w
                .surface()
                .and_then(|s| s.downcast::<gtk4::gdk::Toplevel>().ok())
            else {
                return;
            };
            // Weak: the surface belongs to the window, so a strong ref here would be a cycle.
            let weak = w.downgrade();
            toplevel.connect_state_notify(move |_| {
                ffi_guard::contain((), || {
                    if let Some(w) = weak.upgrade() {
                        report_window_state(&w, false);
                    }
                })
            });
        })
    });
}

/// How long after a state request this backend re-reads the window and reports it whatever it
/// is: long enough for a window manager's maximize or fullscreen round trip, so a request it
/// declined settles day-core's optimistic signal on the state the window kept.
const STATE_SETTLE: std::time::Duration = std::time::Duration::from_millis(1000);

/// Ask GTK for `state` (docs/windows.md "Window properties"). Every call is a request to the
/// window manager; the outcome arrives through `watch_window_state`, and a settle re-read
/// covers the requests it ignores.
fn set_gtk_window_state(window: &gtk4::Window, state: day_spec::WindowState) {
    use day_spec::WindowState as S;
    let Some(track) = state_track(window) else {
        return;
    };
    let minimized = gtk_window_state(window, &track) == S::Minimized;
    match state {
        S::Normal => {
            if window.is_fullscreen() {
                window.unfullscreen();
            }
            if window.is_maximized() {
                window.unmaximize();
            }
            // GTK has no unminimize; presenting the window is how it comes back. An assumed
            // Wayland minimize stays assumed until the window actually takes focus.
            if minimized {
                window.present();
            }
        }
        S::Minimized => {
            window.minimize();
            if !reports_minimized(window) {
                track.assumed_minimized.set(true);
                report_window_state(window, false);
            }
        }
        S::Maximized => {
            if window.is_fullscreen() {
                window.unfullscreen();
            }
            if minimized {
                window.present();
            }
            window.maximize();
        }
        S::Fullscreen => {
            if minimized {
                window.present();
            }
            window.fullscreen();
        }
    }
    let weak = window.downgrade();
    gtk4::glib::timeout_add_local_once(STATE_SETTLE, move || {
        ffi_guard::contain((), || {
            if let Some(w) = weak.upgrade() {
                report_window_state(&w, true);
            }
        })
    });
}

/// Recompute "is any Day window active" on an idle (letting a focus handoff between two
/// Day windows settle) and emit the app-level lifecycle transition only on a real change.
fn note_activation_changed(app: &adw::Application) {
    let app = app.clone();
    gtk4::glib::idle_add_local_once(move || {
        ffi_guard::contain((), || {
            use gtk4::prelude::GtkWindowExt as _;
            let any = app.windows().iter().any(|w| w.is_active());
            if ANY_ACTIVE.with(|c| c.replace(any)) != any {
                let phase = if any {
                    day_spec::Lifecycle::DidBecomeActive
                } else {
                    day_spec::Lifecycle::WillResignActive
                };
                emit(day_spec::WINDOW_NODE, Event::Lifecycle(phase));
            }
        });
    });
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
        // Bundled custom fonts (§18.4) must be registered before any GTK/Pango initialization:
        // Pango's fontmaps (CoreText on macOS, fontconfig on Linux) enumerate the available
        // families when the fontmap is created and do NOT re-scan, so a font registered after
        // GTK init silently falls back to the default family. `check_bundled_fonts` (in
        // activate, below) verifies the families actually resolved and warns loudly if not.
        register_bundled_fonts();

        // AdwApplication initializes libadwaita and loads the Adwaita stylesheet, so
        // AdwNavigationSplitView / AdwNavigationView render with the GNOME treatment.
        let app = adw::Application::builder()
            .application_id(
                std::env::var("DAY_APP_ID").unwrap_or_else(|_| "dev.daybrite.day.app".into()),
            )
            .flags(gtk4::gio::ApplicationFlags::HANDLES_OPEN)
            .build();

        app.connect_open(|app, files, _| {
            ffi_guard::contain((), || {
                let uris: Vec<String> = files.iter().map(|f| f.uri().to_string()).collect();
                // A second launch's arguments arrive here through GApplication when this
                // process owns the app id (docs/deep-links.md "Single-instance forwarding").
                log::debug!("gtk: open request {uris:?}");
                day_core::request_open_files(uris);
                app.activate();
            });
        });

        // DAY_THEME=light|dark forces the Adwaita color scheme (themed CI screenshot runs and
        // local theme checks); unset ⇒ follow the system. Applied in `startup`, once libadwaita
        // is initialized (StyleManager::default() needs adw_init).
        app.connect_startup(|_| {
            // A dayscript driving the app (`DAY_SCRIPT`, set by `day launch --script`) gets
            // GTK's widget transitions off too: Adwaita fades a toggle's or a switch's state
            // over ~200 ms, and a capture taken right after a step shows the state before it.
            let scripted = std::env::var_os("DAY_SCRIPT").is_some_and(|s| !s.is_empty());
            if (day_core::testing::fast_animations() || scripted)
                && let Some(settings) = gtk4::Settings::default()
            {
                settings.set_gtk_enable_animations(false);
            }

            // Follow the SYSTEM appearance while running: libadwaita's StyleManager flips
            // `dark` on desktop theme switches — refresh day-core's reactive dark-mode
            // signal so palette closures recolor live.
            adw::StyleManager::default().connect_dark_notify(|_| {
                ffi_guard::contain((), || {
                    day_core::note_appearance_changed();
                    retint_template_images();
                });
            });
            // Likewise the reduce-motion setting: the desktop's "Animations" switch lands on
            // GtkSettings as `gtk-enable-animations`, and its notify arrives on the main
            // thread, which is Day's — so the re-read happens in place (docs/accessibility.md).
            if let Some(settings) = gtk4::Settings::default() {
                settings.connect_gtk_enable_animations_notify(|_| {
                    ffi_guard::contain((), day_core::note_motion_changed);
                });
            }
            if let Ok(theme) = std::env::var("DAY_THEME") {
                let scheme = match theme.as_str() {
                    "dark" => Some(adw::ColorScheme::ForceDark),
                    "light" => Some(adw::ColorScheme::ForceLight),
                    _ => None,
                };
                if let Some(scheme) = scheme {
                    adw::StyleManager::default().set_color_scheme(scheme);
                }
            }
            // Day's scroll wrappers hold a GtkFixed, which GtkScrolledWindow auto-wraps in a
            // GtkViewport — and the viewport's stock background stays WHITE under the dark
            // color scheme, whiting out every content pane. Retint it with Adwaita's named
            // view color, which tracks light/dark automatically (white in light, so the light
            // appearance is unchanged).
            {
                let p = gtk4::CssProvider::new();
                p.load_from_data("viewport { background-color: @view_bg_color; }");
                if let Some(display) = gtk4::gdk::Display::default() {
                    gtk4::style_context_add_provider_for_display(
                        &display,
                        &p,
                        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
                    );
                }
            }
            // RTL locales (docs/localization): flip GTK's default direction so native widget
            // internals (label alignment, sliders, the Adw split view's sidebar side, back
            // chevrons) mirror; Day's own frames mirror in the layout engine.
            if day_core::layout_direction() == day_spec::LayoutDirection::Rtl {
                gtk4::Widget::set_default_direction(gtk4::TextDirection::Rtl);
            }
        });

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
            ffi_guard::contain((), || {
                emit(
                    day_spec::WINDOW_NODE,
                    Event::Lifecycle(day_spec::Lifecycle::WillTerminate),
                );
            });
        });

        let state = RefCell::new(Some((self, ready, options)));
        // Take on first activate (FnOnce payload inside an Fn handler). Contained like every
        // other signal handler: `ready` boots day-core, and activate is a C up-call.
        app.connect_activate(move |app| {
            ffi_guard::contain((), || {
                let Some((mut backend, ready, options)) = state.borrow_mut().take() else {
                    return;
                };
                let (window, fixed, header) = build_day_window(app, &options, None);
                shell::report_overlay_inset(window.upcast_ref(), None);
                let bar = shell::bar_height(window.upcast_ref());
                // A run with a stated capture size (`DAY_CAPTURE_SCALE`, Day.toml
                // `[screenshots]`) names the CONTENT it captures, and the capture is the content
                // Fixed, below the header bar. Grow the window by the bar so the content is the
                // stated size: by the bar's measured natural height now (the estimate is a pixel
                // off under some themes, and the first capture can come before any correction),
                // and by its allocated height once it has one, should the two differ.
                let capture_run = day_spec::capture_scale().is_some();
                if capture_run && bar > 0.0 {
                    let measured = header.measure(gtk4::Orientation::Vertical, -1).1;
                    let planned = if measured > 0 {
                        measured
                    } else {
                        HEADER_H as i32
                    };
                    window.set_default_size(
                        options.size.width as i32,
                        options.size.height as i32 + planned,
                    );
                    let (window, size) = (window.clone(), options.size);
                    gtk4::glib::idle_add_local_once(move || {
                        ffi_guard::contain((), || {
                            let bar = header.height();
                            if bar > 0 && bar != planned {
                                window
                                    .set_default_size(size.width as i32, size.height as i32 + bar);
                            }
                        })
                    });
                }
                check_bundled_fonts(&window);
                apply_app_icon(&window);
                backend.window_fixed = Some(fixed.clone());
                backend.app = Some(app.clone());
                // Closing the PRIMARY window quits the app, taking secondary windows with it
                // (docs/windows.md close policy) — GApplication would otherwise stay alive
                // while a secondary exists. `quit()` routes through `shutdown` → WillTerminate
                // like every other quit path.
                // …unless day-core asked to keep running (`set_keep_running`), when the close is
                // reported and day-core decides.
                {
                    let app = app.clone();
                    window.connect_close_request(move |_| {
                        ffi_guard::contain(gtk4::glib::Propagation::Proceed, || {
                            shell::primary_close(&app);
                            gtk4::glib::Propagation::Proceed
                        })
                    });
                }
                // Day's content area is the window height minus the header bar; estimate it
                // until the header is allocated (report_content_size reads the real height
                // thereafter).
                ready(
                    backend,
                    fixed.upcast(),
                    Size::new(
                        options.size.width,
                        if capture_run || bar == 0.0 {
                            options.size.height
                        } else {
                            (options.size.height - HEADER_H).max(0.0)
                        },
                    ),
                );
                // Lifecycle activation (docs/lifecycle.md): debounced across ALL Day windows —
                // focus moving between two Day windows is not an app-level resign/become.
                {
                    let app = app.clone();
                    window.connect_is_active_notify(move |_| note_activation_changed(&app));
                }
                window.present();
            });
        });
        // Let GApplication forward document arguments to an already-running instance on
        // desktops with a session bus. Core must not deliver the same arguments again.
        let mut args = vec!["day".to_owned()];
        let files = day_core::take_launch_files();
        if !files.is_empty() {
            args.push("--".into());
            args.extend(files);
        }
        app.run_with_args(&args);
    }

    fn locale_hints(&self) -> Vec<String> {
        // No desktop-wide API here that beats the environment every session sets
        // (§12.2, docs/localization.md).
        day_spec::posix_locale_hints()
    }

    fn post(f: Box<dyn FnOnce() + Send>) {
        // `idle_add_once`, NOT `MainContext::invoke`. glib's invoke runs the closure INLINE when
        // the calling thread owns the context — and a thread that calls it while nobody owns the
        // context BECOMES the owner. Before `g_application_run()` acquires it, that window is
        // open: a dayscript step arriving during startup (day-script's engine listens from its
        // own thread) ran `exec` on the engine thread, which panicked in `with_tree` with "no
        // tree installed on this thread" and left that thread owning the default context, so the
        // main thread's `g_application_run()` then failed with "cannot acquire the default main
        // context because it is already acquired by another thread". An idle source always
        // queues, whoever posts and whenever.
        //
        // At DEFAULT priority, not the idle default: `invoke` ran at G_PRIORITY_DEFAULT, and
        // dropping posted work below GTK's own redraws would quietly reorder every reactive
        // flush and dayscript dispatch against frames. Only the thread rules change here.
        let mut once = Some(f);
        gtk4::glib::idle_add_full(gtk4::glib::Priority::DEFAULT, move || {
            if let Some(f) = once.take() {
                // The posted-closure trampoline: `f` is arbitrary day-core/app work arriving
                // from any thread, run here inside a C idle dispatch — contain it.
                ffi_guard::contain((), f);
            }
            gtk4::glib::ControlFlow::Break
        });
    }

    fn post_delayed(ms: u32, f: Box<dyn FnOnce() + Send>) {
        // The timer door behind `day::sleep` (docs/async.md): a GLib timeout source on the
        // default context, at `post`'s priority and with its thread rules (a source always
        // queues, whoever adds it). No helper thread; the closure runs contained like a post.
        let mut once = Some(f);
        gtk4::glib::timeout_add_full(
            std::time::Duration::from_millis(u64::from(ms)),
            gtk4::glib::Priority::DEFAULT,
            move || {
                if let Some(f) = once.take() {
                    ffi_guard::contain((), f);
                }
                gtk4::glib::ControlFlow::Break
            },
        );
    }
}

use day_spec::WindowOptions;

/// File-picker bookkeeping — the queued/shown distinction that keeps a dismissal from handing
/// GTK a cancelled operation. Pure map state, so it runs without a display.
#[cfg(test)]
mod tests {
    use super::*;

    /// GTK4's <Primary> is a plain <Control> alias (the GTK3 Command mapping is gone), so on
    /// macOS the conventional command modifier must spell the Command key — META to the macos
    /// GDK backend. Everywhere else <Primary> stays.
    #[test]
    fn primary_accelerator_is_command_on_macos() {
        let z = accel_string(&day_spec::Shortcut::primary("z"));
        let sz = accel_string(&day_spec::Shortcut::primary("z").shift());
        if cfg!(target_os = "macos") {
            assert_eq!(z, "<Meta>z");
            assert_eq!(sz, "<Meta><Shift>z");
        } else {
            assert_eq!(z, "<Primary>z");
            assert_eq!(sz, "<Primary><Shift>z");
        }
    }

    /// The dayscript ordering that crashed the linux-gtk walkthrough: `respond` resolves the
    /// request before the deferred idle runs. The idle then declines to open a picker nothing
    /// is waiting for.
    #[test]
    fn a_picker_answered_before_it_opens_never_opens() {
        FILE_DIALOGS.with(|m| m.borrow_mut().insert(1));

        end_file_dialog(1);

        assert!(
            !claim_file_dialog(1),
            "the idle must skip an answered request"
        );
    }

    /// The other ordering: the picker is already up. Dismissal still only DROPS the request —
    /// cancelling a shown picker is what segfaulted GTK 4.14 in CI (see `end_file_dialog`) — and
    /// the drop is what makes its late answer inert.
    #[test]
    fn a_picker_already_showing_is_dropped_so_its_late_answer_is_inert() {
        FILE_DIALOGS.with(|m| m.borrow_mut().insert(2));
        assert!(claim_file_dialog(2), "a live request opens its picker");

        end_file_dialog(2);

        assert!(FILE_DIALOGS.with(|m| m.borrow().is_empty()));
        assert!(
            !claim_file_dialog(2),
            "a result arriving after the dismissal belongs to nothing"
        );
    }
}

// Bitmap handles in reactive state can outlive the toolkit's thread-local registry at Quit.
// Keep normal releases strict about borrow conflicts, but tolerate a destroyed registry.
fn release_bitmap(id: day_spec::BitmapId) {
    let _ = BITMAPS.try_with(|m| m.borrow_mut().remove(&id.0));
}

#[cfg(test)]
mod bitmap_teardown_tests {
    use super::*;

    #[test]
    fn releasing_during_bitmap_registry_teardown_does_not_abort() {
        const CHILD: &str = "DAY_GTK_BITMAP_TEARDOWN_CHILD";
        if std::env::var_os(CHILD).is_some() {
            struct LateRelease(std::sync::mpsc::Sender<()>);
            impl Drop for LateRelease {
                fn drop(&mut self) {
                    assert!(BITMAPS.try_with(|_| ()).is_err());
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
