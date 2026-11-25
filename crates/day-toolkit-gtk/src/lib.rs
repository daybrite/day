// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-toolkit-gtk — the GTK 4 backend (linux-gtk / macos-gtk; DESIGN.md §9). gtk4-rs, pure Rust.
//!
//! `Handle = gtk4::Widget` (GObject-refcounted, `!Send`). Containers are `GtkFixed`; Day's
//! layout positions children via `fixed.move_()` + `set_size_request` (hop's proven pattern).
//! The one exception is a recycling list's cell, a [`DayCell`]: a custom container that takes
//! part in GTK's size negotiation instead — asks for no width, places its children itself from
//! the frames Day recorded — because inside a `GtkListView` a `set_size_request` floor would
//! drag the list wider than its frame (day#35). Measurement of a native leaf is likewise done
//! with its request cleared, or the last placement becomes a floor on the next measure.
//! Native signals connect once at realize, capturing the `NodeId` and emitting into the Day sink.

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
    A11yProps, AnimSpec, Builtin, Cap, Cursor, DrawOp, Event, EventSink, Font, InputPurpose,
    InputTraits, ListSource, NodeId, PieceKind, Platform, Proposal, RawHandle, Rect, Registry,
    Renderer, Size, Support, Toolkit, Transform, TreeSource, ffi_guard, kinds, props_of,
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
        #[expect(
            clippy::match_same_arms,
            reason = "each purpose keeps its own arm and the comment explaining its mapping"
        )]
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
mod input;
mod motion;
mod picker;
mod scroll;
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
    /// Each button's style and the button, for the re-application an appearance change asks
    /// of a tint or a glyph color (`restyle_dynamic_buttons`).
    static BUTTON_STYLE: day_spec::sidetable::SideTable<(gtk4::glib::WeakRef<gtk4::Button>, day_spec::props::ButtonStyleSpec)> = day_spec::sidetable::SideTable::new();
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

    // Day's laid-out top-left origin per GtkFixed child. GTK positions a Fixed child *via* its
    // child transform, which is the same slot the animation transform uses — so the two must be
    // composed (see apply_gtk_transform) rather than overwrite each other. While a frame
    // animation runs (motion.rs) it is the origin shown, not Day's target.
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

    /// Per-widget CSS provider and style for surfaces (docs/surfaces.md), keyed by widget ptr, so
    /// a reactive background repaints by reloading the same provider (no provider accumulation)
    /// and keeps the rounding, border and shadow the surface was realized with.
    /// Each provider is registered on the DISPLAY (`apply_surface`), so the teardown — run by
    /// `release`'s sweep — must take it back off, or display-global providers pile up for the
    /// life of the process, one per decorated widget that ever existed.
    /// The surfaces with a shadow (`true` from the start) and the plain Day containers (no
    /// clip, rounding or role asked for), which turn `true` once they hold such a surface and
    /// let their content draw past their bounds (`let_shadow_spill`). A `GtkFixed` clips its children unless told otherwise, which would
    /// cut a shadow off at the edge of the stack the surface sits in.
    static SPILLS: SideTable<bool> = SideTable::new();
    static SURFACE: SideTable<SurfaceEntry> = SideTable::with_teardown(|e: SurfaceEntry| {
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::style_context_remove_provider_for_display(&display, &e.provider);
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

    /// LIST host key → its model, source holder and scroll state.
    static LIST_STATE: RefCell<HashMap<usize, Rc<ListEntry>>> = RefCell::new(HashMap::new());
    /// TREE host scrolled-window key → its live state (docs/tree.md).
    static TREE_STATE: RefCell<HashMap<usize, Rc<TreeEntry>>> = RefCell::new(HashMap::new());
    /// Widget key → its summon-time context-menu provider (docs/menus.md).
    static CTX_MENU_FNS: RefCell<HashMap<usize, day_spec::ContextMenuFn>> =
        RefCell::new(HashMap::new());
    /// Menu button widget key → the provider its press opens (`set_button_menu`, docs/menus.md
    /// "Menu buttons"). Swept on release.
    static BUTTON_MENUS: SideTable<day_spec::ContextMenuFn> = SideTable::new();
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
    /// Covers built as modal sheets (`CoverProps::sheet` or `large`, not `half`): Escape closes
    /// the topmost one (docs/cover.md "Modal sheet").
    static COVER_SHEET: SideTable<()> = SideTable::new();
    /// Labels that report their selection (`LabelProps::reports_selection`): the node and the
    /// range reported last (`None` after a clear), by label widget key.
    static LABEL_SELECTION: SideTable<(NodeId, Rc<std::cell::Cell<Option<(usize, usize)>>>)> =
        SideTable::new();

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
    let Some((_, runs)) = &style.rich else {
        // Plain text through the plain setter: no markup parse, no escaping to get wrong. The
        // attribute list comes back, since it no longer has runs to override.
        if label.text() != text {
            label.set_text(text);
        }
        restyle_label(label, &style);
        return;
    };
    restyle_label(label, &style);
    // A link run is hit-tested by the label itself, so a label with one takes the pointer.
    if runs.iter().any(|r| r.link.is_some()) {
        label.set_can_target(true);
    }
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

/// The title a button shows, read off its `GtkLabel`: the child itself for a text button, the
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
/// `build_day_window` sets as the `AdwToolbarView`'s content, below the header bar). `None` while
/// the widget is unmapped (its allocation may be stale) or outside a Day window.
fn native_frame(h: &Handle) -> Option<Rect> {
    if !h.is_mapped() {
        return None;
    }
    let content = content_root(h)?;
    let b = h.compute_bounds(&content)?;
    Some(Rect::new(
        f64::from(b.x()),
        f64::from(b.y()),
        f64::from(b.width()),
        f64::from(b.height()),
    ))
}

/// The content area of the Day window `h` is in (the `AdwToolbarView`'s content): the space
/// `NativeSnapshot::frame` and `Displayed::frame` are measured in.
fn content_root(h: &Handle) -> Option<gtk4::Widget> {
    let mut cur = h.parent();
    loop {
        let w = cur?;
        if let Some(view) = w.downcast_ref::<adw::ToolbarView>() {
            return view.content();
        }
        cur = w.parent();
    }
}

thread_local! {
    /// The user's `gtk-enable-animations` while a scripted or fast run holds the setting off
    /// for GTK's own widget transitions (`mute_widget_transitions`); `None` while the setting
    /// is the user's. `reduce_motion` answers this one.
    static USER_ANIMATIONS: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    /// Set while Day itself writes `gtk-enable-animations`, so the notify it raises is not read
    /// as the user's change.
    static MUTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Set while Day moves a slider (`SliderPatch::Value`), so the `value-changed` GTK raises
    /// from inside `set_value` is not reported as the reader's.
    static SLIDER_WRITING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Turn GTK's and libadwaita's own transitions off (a switch's slide, a toggle's fade) through
/// `gtk-enable-animations`, remembering the user's value first. Day's animated changes are
/// stepped by motion.rs, which does not read the setting, so a script still sees them play.
fn mute_widget_transitions(settings: &gtk4::Settings) {
    let user = settings.is_gtk_enable_animations();
    USER_ANIMATIONS.with(|u| {
        if u.get().is_none() {
            u.set(Some(user));
        }
    });
    if user {
        MUTING.with(|m| m.set(true));
        settings.set_gtk_enable_animations(false);
        MUTING.with(|m| m.set(false));
    }
}

/// Put a [`day_spec::props::ButtonStyleSpec`] on a `GtkButton`, keeping it a `GtkButton`.
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
            let key = hex(c.resolved());
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
                    hex(S::on_tint(c.resolved()))
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
        S::Tinted(c) => S::on_tint(c.resolved()),
        S::Prominent => day_spec::Color::WHITE,
        _ => {
            if adw::StyleManager::default().is_dark() {
                day_spec::Color::WHITE
            } else {
                day_spec::Color::hex(0x001a_1a1a)
            }
        }
    };
    BUTTON_INK.with(|m| {
        m.insert(key, ink);
    });
    BUTTON_STYLE.with(|m| {
        m.insert(key, (btn.downgrade(), style));
    });
    if let Some(c) = BUTTON_CONTENT.with(|m| m.get(key)) {
        apply_button_content(btn, &c.title, c.icon.as_ref(), c.icon_only);
    }
}

/// Re-apply the style of every button whose look depends on the appearance: a tint given as a
/// pair or a role (its fill and readable title color are resolved into a CSS class), and a
/// button with an icon (its glyph color follows the theme foreground).
fn restyle_dynamic_buttons() {
    use day_spec::props::ButtonStyleSpec as S;
    let mut live = Vec::new();
    BUTTON_STYLE.with(|t| {
        t.for_each(|key, (button, style)| {
            let follows = match style {
                S::Tinted(ink) => ink.is_dynamic(),
                _ => BUTTON_CONTENT
                    .with(|c| c.get(key))
                    .is_some_and(|c| c.icon.is_some()),
            };
            if follows && let Some(b) = button.upgrade() {
                live.push((b, *style));
            }
        });
    });
    for (button, style) in live {
        apply_button_style(&button, style);
    }
}

fn cairo_set_color(cr: &gtk4::cairo::Context, bits: f64) {
    let v = bits as u32;
    cr.set_source_rgba(
        f64::from((v >> 16) & 0xff) / 255.0,
        f64::from((v >> 8) & 0xff) / 255.0,
        f64::from(v & 0xff) / 255.0,
        f64::from((v >> 24) & 0xff) / 255.0,
    );
}

/// A wire color (`0xAARRGGBB`) as cairo's four unit channels, red first.
fn cairo_rgba(bits: u32) -> (f64, f64, f64, f64) {
    (
        f64::from((bits >> 16) & 0xff) / 255.0,
        f64::from((bits >> 8) & 0xff) / 255.0,
        f64::from(bits & 0xff) / 255.0,
        f64::from((bits >> 24) & 0xff) / 255.0,
    )
}

/// A decoded kind-14 record (set-gradient), waiting for its shape record. Unit geometry
/// resolves against the shape's bounding box; stops are (offset, 0xAARRGGBB).
struct PendingGradient {
    /// Encoded gradient type: 0 = linear (a,b→c,d unit points), 1 = radial (a,b center, c
    /// radius), 2 = conic (a,b center, c start angle in canvas degrees).
    kind: u8,
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    stops: Vec<(f64, u32)>,
}

impl PendingGradient {
    /// Install the gradient as the cairo source, resolved against the shape's bbox. `reach` is
    /// how far past the bbox the drawing can extend (a stroke's half width times its miter
    /// limit; `0` for a fill): a conic gradient is a finite fan and has to cover that too.
    fn set_source(&self, cr: &gtk4::cairo::Context, bbox: (f64, f64, f64, f64), reach: f64) {
        let (x, y, w, h) = bbox;
        let stops = |g: &gtk4::cairo::Gradient| {
            for (o, bits) in &self.stops {
                let (r, gr, b, a) = cairo_rgba(*bits);
                g.add_color_stop_rgba(*o, r, gr, b, a);
            }
        };
        match self.kind {
            1 => {
                // Radial, elliptical-to-bounds: a circular gradient in the unit space of the
                // bounds, mapped by the PATTERN matrix (user → pattern space, so the inverse
                // of the unit→bounds map).
                let rg =
                    gtk4::cairo::RadialGradient::new(self.a, self.b, 0.0, self.a, self.b, self.c);
                stops(rg.as_ref());
                let m = gtk4::cairo::Matrix::new(1.0 / w, 0.0, 0.0, 1.0 / h, -x / w, -y / h);
                rg.set_matrix(m);
                let _ = cr.set_source(&rg);
            }
            2 => {
                let (cx, cy) = (x + self.a * w, y + self.b * h);
                // Far enough to reach the farthest corner of the bounds from the center.
                let far = [(x, y), (x + w, y), (x, y + h), (x + w, y + h)]
                    .iter()
                    .map(|(px, py)| (px - cx).hypot(py - cy))
                    .fold(0.0, f64::max);
                let radius = far + reach.max(0.0) + 2.0;
                set_conic_source(cr, cx, cy, radius, self.c, &self.stops);
            }
            _ => {
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
}

/// One wedge of a conic gradient: from `from` to `to` (canvas degrees, clockwise), running from
/// one `0xAARRGGBB` color to the other.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ConicWedge {
    from: f64,
    to: f64,
    start: u32,
    end: u32,
}

/// The widest wedge a changing color is drawn with, in degrees. Each wedge is one Coons patch
/// whose colors run linearly along its arc; at this width the patch's own interpolation and
/// [`day_spec::ConicGradient::color_at`]'s cannot be told apart.
const CONIC_STEP: f64 = 2.0;
/// The widest wedge of one flat color: only the arc's shape limits it (a cubic Bézier is a
/// faithful arc well past this).
const CONIC_FLAT_STEP: f64 = 30.0;

/// A conic gradient as wedges covering exactly one turn from `start_deg`, in order
/// ([`day_spec::ConicGradient`]): flat before the first stop and after the last, linear per
/// channel between two stops, and a hard edge where two stops share an offset.
fn conic_wedges(start_deg: f64, stops: &[(f64, u32)]) -> Vec<ConicWedge> {
    let lerp = |a: u32, b: u32, k: f64| -> u32 {
        let ch = |shift: u32| {
            let (ca, cb) = (
                f64::from((a >> shift) & 0xff),
                f64::from((b >> shift) & 0xff),
            );
            ((ca + (cb - ca) * k).round().clamp(0.0, 255.0) as u32) << shift
        };
        ch(24) | ch(16) | ch(8) | ch(0)
    };
    // (offset from, offset to, color from, color to), offsets clamped and kept ascending.
    let mut spans: Vec<(f64, f64, u32, u32)> = Vec::new();
    let mut at = 0.0f64;
    let mut prev: Option<u32> = None;
    for (offset, color) in stops {
        let offset = offset.clamp(at, 1.0);
        if offset > at {
            spans.push((at, offset, prev.unwrap_or(*color), *color));
        }
        at = offset;
        prev = Some(*color);
    }
    if let Some(last) = prev
        && at < 1.0
    {
        spans.push((at, 1.0, last, last));
    }
    let mut out = Vec::new();
    for (from, to, a, b) in spans {
        let sweep = (to - from) * 360.0;
        let step = if a == b { CONIC_FLAT_STEP } else { CONIC_STEP };
        let n = (sweep / step).ceil().max(1.0) as u32;
        // A span's last edge is its `to` exactly, the next span's first edge, so two wedges
        // that meet share their edge to the bit ([`set_conic_fan`] tiles on that).
        let edge = |i: u32| {
            let at = if i == n {
                to
            } else {
                from + (to - from) * f64::from(i) / f64::from(n)
            };
            start_deg + at * 360.0
        };
        for i in 0..n {
            let (k0, k1) = (f64::from(i) / f64::from(n), f64::from(i + 1) / f64::from(n));
            out.push(ConicWedge {
                from: edge(i),
                to: edge(i + 1),
                start: lerp(a, b, k0),
                end: lerp(a, b, k1),
            });
        }
    }
    out
}

/// A conic gradient as a cairo mesh pattern: cairo has no sweep gradient, so each
/// [`ConicWedge`] is one Coons patch from the center out to `radius`, colored along its arc.
/// Patches share their edges exactly, so the fan has no seams, and the mesh is a pattern like
/// any other: it fills, strokes and prints (PDF keeps it as a shading, not as pixels).
fn conic_mesh(
    cx: f64,
    cy: f64,
    radius: f64,
    start_deg: f64,
    stops: &[(f64, u32)],
) -> gtk4::cairo::Mesh {
    use gtk4::cairo::MeshCorner;
    let mesh = gtk4::cairo::Mesh::new();
    for wedge in conic_wedges(start_deg, stops) {
        let (a0, a1) = (wedge.from.to_radians(), wedge.to.to_radians());
        // The cubic that follows a circular arc: handles of length k·r along the tangents.
        let k = 4.0 / 3.0 * ((a1 - a0) / 4.0).tan() * radius;
        let (p0x, p0y) = (cx + radius * a0.cos(), cy + radius * a0.sin());
        let (p1x, p1y) = (cx + radius * a1.cos(), cy + radius * a1.sin());
        mesh.begin_patch();
        mesh.move_to(cx, cy);
        mesh.line_to(p0x, p0y);
        mesh.curve_to(
            p0x - k * a0.sin(),
            p0y + k * a0.cos(),
            p1x + k * a1.sin(),
            p1y - k * a1.cos(),
            p1x,
            p1y,
        );
        mesh.line_to(cx, cy);
        // The fourth side is the center itself: both of its corners sit there, each carrying
        // the color of the radius it ends, so the color depends on the angle alone.
        mesh.line_to(cx, cy);
        let (r, g, b, a) = cairo_rgba(wedge.start);
        mesh.set_corner_color_rgba(MeshCorner::MeshCorner0, r, g, b, a);
        mesh.set_corner_color_rgba(MeshCorner::MeshCorner1, r, g, b, a);
        let (r, g, b, a) = cairo_rgba(wedge.end);
        mesh.set_corner_color_rgba(MeshCorner::MeshCorner2, r, g, b, a);
        mesh.set_corner_color_rgba(MeshCorner::MeshCorner3, r, g, b, a);
        mesh.end_patch();
    }
    mesh
}

/// Install a conic gradient about (`cx`, `cy`), reaching `radius`, as the cairo source: the
/// mesh of [`conic_mesh`] wherever cairo paints one, else the fan of [`set_conic_fan`]. A PDF,
/// PostScript or SVG target always takes the mesh, which it writes out as a shading without
/// rasterizing it. Any other target (an image, or the recording GSK replays onto one) takes
/// the fan only where this process's cairo cannot rasterize a mesh ([`mesh_rasterizes`]).
/// The current path, the shape about to be painted, is kept.
fn set_conic_source(
    cr: &gtk4::cairo::Context,
    cx: f64,
    cy: f64,
    radius: f64,
    start_deg: f64,
    stops: &[(f64, u32)],
) {
    use gtk4::cairo::SurfaceType;
    let vector = matches!(
        cr.target().type_(),
        SurfaceType::Pdf | SurfaceType::Ps | SurfaceType::Svg
    );
    if vector || mesh_rasterizes() {
        let _ = cr.set_source(conic_mesh(cx, cy, radius, start_deg, stops));
    } else {
        set_conic_fan(cr, cx, cy, radius, start_deg, stops);
    }
}

/// Whether this process's cairo paints a mesh pattern onto an image. Every build should.
/// Ubuntu 26.04's x86-64-v3 build of cairo 1.18.4 (`libcairo2` 1.18.4-3 `amd64v3`, the one
/// GitHub's `ubuntu-latest` runners install) does not: it paints a patch's first side and
/// its diagonal and leaves the rest clear, while the baseline `amd64` build of the same
/// package paints the whole patch. That breaks [`conic_mesh`] on every GSK renderer, since a
/// canvas is drawn by cairo whatever renders the window, and GSK's own cairo renderer, which
/// draws a CSS `conic-gradient()` as a mesh too ([`SurfaceStyle::draws_conic`]).
///
/// Asked once per process: one opaque square patch is painted and two of its pixels, one on
/// each side of that diagonal and clear of every side, must come back opaque. A probe that
/// cannot run answers `true`, which keeps the mesh.
fn mesh_rasterizes() -> bool {
    static PAINTS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PAINTS.get_or_init(|| {
        use gtk4::cairo::{self, MeshCorner};
        const SIDE: i32 = 16;
        let Ok(mut surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, SIDE, SIDE) else {
            return true;
        };
        {
            let Ok(cr) = cairo::Context::new(&surface) else {
                return true;
            };
            let side = f64::from(SIDE);
            let mesh = cairo::Mesh::new();
            mesh.begin_patch();
            mesh.move_to(0.0, 0.0);
            mesh.line_to(side, 0.0);
            mesh.line_to(side, side);
            mesh.line_to(0.0, side);
            for corner in [
                MeshCorner::MeshCorner0,
                MeshCorner::MeshCorner1,
                MeshCorner::MeshCorner2,
                MeshCorner::MeshCorner3,
            ] {
                mesh.set_corner_color_rgba(corner, 1.0, 0.0, 0.0, 1.0);
            }
            mesh.end_patch();
            if cr.set_source(&mesh).is_err() || cr.paint().is_err() {
                return true;
            }
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let Ok(data) = surface.data() else {
            return true;
        };
        // ARGB32 pixels are native-endian words with alpha in the top byte.
        [(12usize, 3usize), (3, 12)].iter().all(|&(x, y)| {
            let at = y * stride + x * 4;
            data.get(at..at + 4)
                .and_then(|px| <[u8; 4]>::try_from(px).ok())
                .is_some_and(|px| u32::from_ne_bytes(px) >> 24 == 0xff)
        })
    })
}

/// A conic gradient as a fan of [`ConicWedge`]s painted into a group that becomes the cairo
/// source: the stand-in for [`conic_mesh`] where cairo cannot rasterize a mesh
/// ([`mesh_rasterizes`]). Each wedge is a sector from the center out to `radius`, filled with
/// its two colors as a linear gradient across its rim: exact on the rim, and nowhere further
/// from [`day_spec::ConicGradient::color_at`] than half the change across one wedge. The
/// wedges are drawn without antialiasing and with the SOURCE operator, so neighbors tile on
/// their shared edges ([`conic_wedges`] makes them bit-identical) with no seam of partial
/// coverage and no translucent wedge laid over another; the shape painted with the fan keeps
/// its own antialiased outline. The current path, the shape about to be painted, is kept.
fn set_conic_fan(
    cr: &gtk4::cairo::Context,
    cx: f64,
    cy: f64,
    radius: f64,
    start_deg: f64,
    stops: &[(f64, u32)],
) {
    use gtk4::cairo;
    let shape = cr.copy_path();
    cr.new_path();
    // The group needs no more surface than the fan's square.
    let _ = cr.save();
    cr.rectangle(cx - radius, cy - radius, 2.0 * radius, 2.0 * radius);
    cr.clip();
    cr.push_group();
    cr.set_operator(cairo::Operator::Source);
    cr.set_antialias(cairo::Antialias::None);
    for wedge in conic_wedges(start_deg, stops) {
        let (a0, a1) = (wedge.from.to_radians(), wedge.to.to_radians());
        let (p0x, p0y) = (cx + radius * a0.cos(), cy + radius * a0.sin());
        let (p1x, p1y) = (cx + radius * a1.cos(), cy + radius * a1.sin());
        let (r, g, b, a) = cairo_rgba(wedge.start);
        if wedge.start == wedge.end {
            cr.set_source_rgba(r, g, b, a);
        } else {
            let rim = cairo::LinearGradient::new(p0x, p0y, p1x, p1y);
            rim.add_color_stop_rgba(0.0, r, g, b, a);
            let (r, g, b, a) = cairo_rgba(wedge.end);
            rim.add_color_stop_rgba(1.0, r, g, b, a);
            let _ = cr.set_source(&rim);
        }
        // Both straight sides run between the center and points computed here, never
        // through the arc's own end, so a wedge's side is exactly its neighbor's.
        cr.move_to(cx, cy);
        cr.line_to(p0x, p0y);
        cr.arc(cx, cy, radius, a0, a1);
        cr.line_to(p1x, p1y);
        cr.close_path();
        let _ = cr.fill();
    }
    let fan = cr.pop_group();
    let _ = cr.restore();
    if let Ok(fan) = fan {
        let _ = cr.set_source(&fan);
    }
    if let Ok(shape) = shape {
        cr.append_path(&shape);
    }
}

/// A color as the wire's `0xAARRGGBB`.
fn color_bits(c: day_spec::Color) -> u32 {
    let ch = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    (ch(c.a) << 24) | (ch(c.r) << 16) | (ch(c.g) << 8) | ch(c.b)
}

/// Fill a `w` × `h` box with a surface's conic gradient in the current appearance, the way a
/// canvas paints one ([`set_conic_source`]): what a Day container draws itself where GSK's
/// cairo renderer would not draw the CSS gradient ([`SurfaceStyle::draws_conic`]).
fn paint_conic_fill(cr: &gtk4::cairo::Context, w: f64, h: f64, g: &day_spec::ConicGradient) {
    let stops: Vec<(f64, u32)> = g
        .stops
        .iter()
        .map(|(offset, ink)| (*offset, color_bits(ink.resolved())))
        .collect();
    let (cx, cy) = (g.center.x * w, g.center.y * h);
    let far = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)]
        .iter()
        .map(|(px, py)| (px - cx).hypot(py - cy))
        .fold(0.0, f64::max);
    cr.new_path();
    set_conic_source(cr, cx, cy, far + 2.0, g.start.degrees(), &stops);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();
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

/// The shadow a kind-24 record sets (docs/canvas.md "Shadows"): graphics state, cast by every
/// drawing record until it is replaced or the save it was set under is restored. cairo has no
/// shadow state of its own, so `cairo_draw` keeps this beside cairo's save stack.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CanvasShadow {
    /// The offset, in points of the canvas as a whole: the current transform does not move it.
    dx: f64,
    dy: f64,
    /// The CSS blur radius in points, twice the Gaussian's standard deviation.
    blur: f64,
    /// `0xAARRGGBB`.
    color: u32,
}

/// The prefix records waiting for the drawing record they apply to: the gradient (kind 14),
/// the stroke style (18), the font (19) and the image source region (23).
#[derive(Default)]
struct CanvasPrefix {
    gradient: Option<PendingGradient>,
    stroke: Option<PendingStroke>,
    font: Option<PendingFont>,
    source: Option<day_spec::Rect>,
}

/// The cairo operator for a kind-25 record's mode number ([`day_spec::BlendMode::wire`]). The
/// separable modes are cairo's own; `Add` is its Porter-Duff "plus" and `Erase` its
/// destination-out. A number this decoder does not know draws normally.
fn blend_operator(mode: i32) -> gtk4::cairo::Operator {
    use gtk4::cairo::Operator;
    match mode {
        1 => Operator::Multiply,
        2 => Operator::Screen,
        3 => Operator::Overlay,
        4 => Operator::Darken,
        5 => Operator::Lighten,
        6 => Operator::Add,
        7 => Operator::DestOut,
        _ => Operator::Over,
    }
}

/// The three box blurs that stand in for a Gaussian of standard deviation `sigma` pixels, as
/// each pass's reach to the left and to the right of the pixel it averages into (the
/// construction CSS filters specify: three boxes of width `d`, the first two shifted half a
/// pixel either way when `d` is even). All zero when the blur is too small for a box.
fn box_blur_passes(sigma: f64) -> [(usize, usize); 3] {
    let d = (sigma * 3.0 * std::f64::consts::TAU.sqrt() / 4.0 + 0.5).floor();
    if d.is_nan() || d < 2.0 {
        return [(0, 0); 3];
    }
    let d = d as usize;
    let half = d / 2;
    if d % 2 == 1 {
        [(half, half); 3]
    } else {
        [(half, half - 1), (half - 1, half), (half, half)]
    }
}

/// A normalized Gaussian kernel of standard deviation `sigma` pixels, three deviations to
/// each side: what a blur too small for the box construction is convolved with.
fn gaussian_kernel(sigma: f64) -> Vec<f32> {
    let reach = (sigma * 3.0).ceil().max(1.0) as i32;
    let weights: Vec<f64> = (-reach..=reach)
        .map(|i| (-f64::from(i * i) / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f64 = weights.iter().sum();
    weights.iter().map(|w| (w / total) as f32).collect()
}

/// Below this many pixels of standard deviation the blur is a true Gaussian convolution; from
/// here up three box blurs are indistinguishable from one and cost the same at any size.
const BOX_BLUR_FROM: f64 = 2.0;

/// How many pixels [`blur_alpha`] spreads coverage past a silhouette's edge on each side.
fn blur_reach(sigma: f64) -> usize {
    if sigma <= 0.0 {
        0
    } else if sigma < BOX_BLUR_FROM {
        (sigma * 3.0).ceil().max(1.0) as usize
    } else {
        box_blur_passes(sigma).iter().map(|(l, r)| *l.max(r)).sum()
    }
}

/// Blur an 8-bit coverage mask in place by a Gaussian of standard deviation `sigma` pixels.
/// `data` is `height` rows of `stride` bytes, the first `width` of each being pixels; what
/// lies outside the mask counts as empty.
fn blur_alpha(data: &mut [u8], width: usize, height: usize, stride: usize, sigma: f64) {
    if sigma <= 0.0 || width == 0 || height == 0 || data.len() < (height - 1) * stride + width {
        return;
    }
    let mut scratch = vec![0u8; width.max(height)];
    if sigma < BOX_BLUR_FROM {
        let kernel = gaussian_kernel(sigma);
        for y in 0..height {
            convolve_line(data, y * stride, 1, width, &kernel, &mut scratch);
        }
        for x in 0..width {
            convolve_line(data, x, stride, height, &kernel, &mut scratch);
        }
        return;
    }
    for (left, right) in box_blur_passes(sigma) {
        if left + right == 0 {
            continue;
        }
        for y in 0..height {
            box_blur_line(data, y * stride, 1, width, (left, right), &mut scratch);
        }
        for x in 0..width {
            box_blur_line(data, x, stride, height, (left, right), &mut scratch);
        }
    }
}

/// One box-blur pass over the `n` bytes at `start`, `start + step`, …: each becomes the mean
/// of the window reaching `reach.0` before it and `reach.1` after it. A running sum, so the
/// cost does not grow with the window.
fn box_blur_line(
    data: &mut [u8],
    start: usize,
    step: usize,
    n: usize,
    reach: (usize, usize),
    scratch: &mut [u8],
) {
    let (left, right) = reach;
    for (i, s) in scratch.iter_mut().enumerate().take(n) {
        *s = data[start + i * step];
    }
    let window = (left + right + 1) as u32;
    let mut sum: u32 = scratch[..n.min(right + 1)]
        .iter()
        .map(|v| u32::from(*v))
        .sum();
    for i in 0..n {
        data[start + i * step] = ((sum + window / 2) / window).min(255) as u8;
        if i + right + 1 < n {
            sum += u32::from(scratch[i + right + 1]);
        }
        if i >= left {
            sum -= u32::from(scratch[i - left]);
        }
    }
}

/// Convolve the `n` bytes at `start`, `start + step`, … with an odd-length `kernel`.
fn convolve_line(
    data: &mut [u8],
    start: usize,
    step: usize,
    n: usize,
    kernel: &[f32],
    scratch: &mut [u8],
) {
    for (i, s) in scratch.iter_mut().enumerate().take(n) {
        *s = data[start + i * step];
    }
    let reach = kernel.len() / 2;
    for i in 0..n {
        let mut acc = 0.0f32;
        for (j, weight) in kernel.iter().enumerate() {
            if let Some(at) = (i + j).checked_sub(reach)
                && at < n
            {
                acc += weight * f32::from(scratch[at]);
            }
        }
        data[start + i * step] = acc.round().clamp(0.0, 255.0) as u8;
    }
}

/// The most pixels one shadow's mask may hold: a full 16,384-pixel export side squared would
/// be a quarter of a gigabyte per shadow, so past this the shadow is skipped, not the drawing.
const SHADOW_MASK_LIMIT: i64 = 64 << 20;

/// Draw the shadow of what `draw` paints (docs/canvas.md "Shadows"). cairo has neither a
/// shadow nor a blur, so this is the emulation: `draw` runs once into a recording surface at
/// device resolution, the recording's coverage is copied into an 8-bit mask around its ink
/// extents, the mask is blurred, and the shadow color is painted through it at the offset.
///
/// `base` is the transform the canvas started with and `density` the device pixels per unit
/// of cairo's device space: the offset and the blur are in points of the canvas as a whole,
/// so they take `base` (an export's scale) and never the current transform. The mask is
/// painted with the context's operator and under its clip, as the shape itself is.
fn cast_shadow(
    cr: &gtk4::cairo::Context,
    shadow: &CanvasShadow,
    base: &gtk4::cairo::Matrix,
    density: f64,
    draw: &dyn Fn(&gtk4::cairo::Context),
) -> Option<()> {
    use gtk4::cairo::{Content, Context, Format, ImageSurface, Matrix, RecordingSurface};
    // User space → the mask's pixels: the current transform, then the device's density.
    let to_pixels = Matrix::multiply(
        &cr.matrix(),
        &Matrix::new(density, 0.0, 0.0, density, 0.0, 0.0),
    );
    let recording = RecordingSurface::create(Content::ColorAlpha, None).ok()?;
    {
        let rc = Context::new(&recording).ok()?;
        rc.set_matrix(to_pixels);
        draw(&rc);
    }
    let (ink_x, ink_y, ink_w, ink_h) = recording.ink_extents();
    if ink_w <= 0.0 || ink_h <= 0.0 {
        return None;
    }
    let points_to_pixels = (base.xx() * base.yy() - base.xy() * base.yx()).abs().sqrt() * density;
    let sigma = shadow.blur / 2.0 * points_to_pixels;
    let reach = blur_reach(sigma) as f64 + 1.0;
    let (off_x, off_y) = base.transform_distance(shadow.dx, shadow.dy);
    let (off_x, off_y) = (off_x * density, off_y * density);
    // Only the silhouette that can land inside the clip matters: the clip's box in pixels,
    // moved back by the offset and widened by the blur, bounds the mask on a large drawing.
    let (mut x0, mut y0, mut x1, mut y1) = (ink_x, ink_y, ink_x + ink_w, ink_y + ink_h);
    if let Ok((cx0, cy0, cx1, cy1)) = cr.clip_extents() {
        let corners = [(cx0, cy0), (cx1, cy0), (cx0, cy1), (cx1, cy1)]
            .map(|(x, y)| to_pixels.transform_point(x, y));
        let min = |f: fn(&(f64, f64)) -> f64| corners.iter().map(f).fold(f64::MAX, f64::min);
        let max = |f: fn(&(f64, f64)) -> f64| corners.iter().map(f).fold(f64::MIN, f64::max);
        x0 = x0.max(min(|p| p.0) - off_x - reach);
        y0 = y0.max(min(|p| p.1) - off_y - reach);
        x1 = x1.min(max(|p| p.0) - off_x + reach);
        y1 = y1.min(max(|p| p.1) - off_y + reach);
    }
    let (x0, y0) = ((x0 - reach).floor(), (y0 - reach).floor());
    let (width, height) = ((x1 + reach).ceil() - x0, (y1 + reach).ceil() - y0);
    if !(width >= 1.0 && height >= 1.0) || width * height > SHADOW_MASK_LIMIT as f64 {
        return None;
    }
    let (width, height) = (width as i32, height as i32);
    let mut mask = ImageSurface::create(Format::A8, width, height).ok()?;
    {
        let mc = Context::new(&mask).ok()?;
        mc.set_source_surface(&recording, -x0, -y0).ok()?;
        mc.paint().ok()?;
    }
    mask.flush();
    let stride = usize::try_from(mask.stride()).ok()?;
    {
        let mut data = mask.data().ok()?;
        blur_alpha(&mut data, width as usize, height as usize, stride, sigma);
    }
    cr.save().ok()?;
    // Pixel space for the composite: one mask pixel per device pixel, whatever the transform.
    cr.set_matrix(Matrix::new(
        1.0 / density,
        0.0,
        0.0,
        1.0 / density,
        0.0,
        0.0,
    ));
    let (r, g, b, a) = cairo_rgba(shadow.color);
    cr.set_source_rgba(r, g, b, a);
    let painted = cr.mask_surface(&mask, x0 + off_x, y0 + off_y);
    cr.restore().ok()?;
    painted.ok()
}

/// Trace a rounded rectangle (radius clamped to half the short side): cairo has no primitive,
/// so the four corner arcs are joined into one closed sub-path.
fn cairo_rounded_rect(cr: &gtk4::cairo::Context, x: f64, y: f64, w: f64, h: f64, radius: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.close_path();
}

/// Trace a polygon's "x,y x,y …" payload as one closed sub-path, answering its bounding box,
/// or `None` when no point parsed.
fn cairo_trace_polygon(cr: &gtk4::cairo::Context, pts: &str) -> Option<(f64, f64, f64, f64)> {
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
    if first {
        return None;
    }
    cr.close_path();
    Some((x0, y0, x1 - x0, y1 - y0))
}

/// Whether a record kind draws (a fill, a stroke, text or an image) rather than setting state.
fn is_drawing_kind(k: i32) -> bool {
    matches!(k, 0..=7 | 11..=13 | 15 | 16 | 22)
}

/// Paint one drawing record (`[kind, a..g, color]`) on `cr`, with the texts-channel `payload`
/// it carries, if any, and the prefix records that apply to it.
///
/// Everything the record needs is set here and nothing is read from the context but its
/// transform, clip and operator, so the same call paints the shape on the canvas and, into a
/// recording surface, the silhouette its shadow is made from ([`cast_shadow`]).
fn cairo_draw_record(
    cr: &gtk4::cairo::Context,
    rec: &[f64; 9],
    payload: &str,
    prefix: &CanvasPrefix,
) {
    let &[k, a, b, c, d, e, f, g, col] = rec;
    let k = k as i32;
    cairo_set_color(cr, col);
    // Stroke kinds, in encode_ops order: rect, ellipse, arc, line, polygon, rrect, path.
    let is_stroke = matches!(k, 1 | 4 | 5 | 6 | 12 | 13 | 16);
    if is_stroke {
        match &prefix.stroke {
            Some(style) => style.apply(cr),
            None => reset_stroke(cr),
        }
        cr.set_line_width(g);
    }
    // Fill or stroke the traced path, in the gradient when one is pending. A gradient
    // resolves against the bounding box of the shape, stroked or filled.
    let paint = |bbox: (f64, f64, f64, f64)| {
        if let Some(gradient) = &prefix.gradient {
            let reach = if is_stroke {
                g / 2.0 * cr.miter_limit().max(1.0)
            } else {
                0.0
            };
            gradient.set_source(cr, bbox, reach);
        }
        let _ = if is_stroke { cr.stroke() } else { cr.fill() };
    };
    match k {
        0 | 1 => {
            cr.rectangle(a, b, c, d);
            paint((a, b, c, d));
        }
        // Rounded rect (2 fill / 13 stroke).
        2 | 13 => {
            cairo_rounded_rect(cr, a, b, c, d, e);
            paint((a, b, c, d));
        }
        3 | 4 => {
            cr.save().ok();
            cr.translate(a + c / 2.0, b + d / 2.0);
            cr.scale(c / 2.0, d / 2.0);
            cr.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
            cr.restore().ok();
            paint((a, b, c, d));
        }
        5 => {
            cr.set_line_cap(gtk4::cairo::LineCap::Round);
            cr.arc(
                a + c / 2.0,
                b + d / 2.0,
                c.min(d) / 2.0,
                e.to_radians(),
                (e + f).to_radians(),
            );
            paint((a, b, c, d));
        }
        6 => {
            cr.move_to(a, b);
            cr.line_to(c, d);
            paint((a.min(c), b.min(d), (c - a).abs(), (d - b).abs()));
        }
        // Text (7): a Pango layout, drawn from its top-left — which IS the `LEADING`
        // anchor — offset by whatever the packed anchor asks for, using the layout's own
        // logical extents and baseline. The layout picks up the cairo CTM, so it rotates and
        // scales with the drawing.
        7 => {
            let default_font = PendingFont::default();
            let font = prefix.font.as_ref().unwrap_or(&default_font);
            let layout = pangocairo::functions::create_layout(cr);
            layout.set_font_description(Some(&canvas_font_desc(e, font)));
            layout.set_single_paragraph_mode(true);
            layout.set_text(payload);
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
        // Polygon (11 fill / 12 stroke): points ride the texts channel as "x,y x,y …".
        11 | 12 => {
            if let Some(bbox) = cairo_trace_polygon(cr, payload) {
                paint(bbox);
            }
        }
        // Path (15 fill / 16 stroke): segments ride the texts channel; slot f is the fill
        // rule (0 non-zero, 1 even-odd) and slot g the stroke width.
        15 | 16 => {
            let bbox = cairo_trace_path(cr, payload);
            if k == 15 && f > 0.5 {
                cr.set_fill_rule(gtk4::cairo::FillRule::EvenOdd);
            }
            paint(bbox);
            cr.set_fill_rule(gtk4::cairo::FillRule::Winding);
        }
        // Image (22): a,b origin · c,d size · e the BitmapId · f opacity (docs/images.md).
        // A released bitmap draws nothing rather than a placeholder: a canvas re-records
        // on every tracked read, so a handle can be dropped between record and replay.
        22 => {
            let pixbuf =
                BITMAPS.with(|m| m.borrow().get(&(e as u64)).and_then(|b| b.pixbuf.clone()));
            let Some(pixbuf) = pixbuf else {
                return;
            };
            let (pw, ph) = (f64::from(pixbuf.width()), f64::from(pixbuf.height()));
            // A pending region (kind 23, in the pixbuf's pixels) is drawn by clipping to the
            // rect and scaling the whole pixbuf so the region lands in it: cairo has no
            // sub-rect paint, and the clip is exact. The shared clamp decides only whether
            // anything is inside the bitmap at all.
            let region = match prefix.source {
                None => Some((day_spec::Rect::new(0.0, 0.0, pw, ph), false)),
                Some(src) => day_spec::clamp_image_source(
                    src,
                    day_spec::Rect::new(a, b, c, d),
                    Size::new(pw, ph),
                )
                .map(|_| (src, true)),
            };
            if let Some((src, clip)) = region
                && pw > 0.0
                && ph > 0.0
                && c > 0.0
                && d > 0.0
            {
                use gtk4::gdk::prelude::GdkCairoContextExt as _;
                cr.save().ok();
                if clip {
                    cr.rectangle(a, b, c, d);
                    cr.clip();
                }
                cr.translate(a, b);
                cr.scale(c / src.size.width, d / src.size.height);
                cr.set_source_pixbuf(&pixbuf, -src.origin.x, -src.origin.y);
                let _ = cr.paint_with_alpha(f.clamp(0.0, 1.0));
                cr.restore().ok();
            }
        }
        _ => {}
    }
}

/// The device pixels per point an on-screen canvas is rasterized at, for [`cairo_draw`]: GTK
/// records the drawing and rasterizes it later, at the surface's scale factor, or at a
/// capture's stated scale when that is finer (`snapshot_widget`).
fn canvas_density(scale_factor: i32) -> f64 {
    static CAPTURE: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    let capture = *CAPTURE.get_or_init(|| day_spec::capture_scale().unwrap_or(1.0));
    f64::from(scale_factor.max(1)).max(capture)
}

/// Replay an encoded display list (`day_spec::encode_ops`) on `cr`: the canvas's painter, on
/// screen and for the bitmap and PDF exports alike.
///
/// `density` is how many device pixels one unit of the context's device space covers (the
/// widget's scale factor on screen, where GTK hands over a recording surface that it
/// rasterizes later; `1` for an image surface). Only the shadow emulation reads it, to size
/// its mask ([`cast_shadow`]).
fn cairo_draw(cr: &gtk4::cairo::Context, nums: &[f64], texts: &[String], density: f64) {
    let base = cr.matrix();
    // Blend modes combine with what THIS canvas has drawn, and `Erase` must leave it clear
    // (docs/canvas.md "Blend modes"). A list that blends is therefore painted into a group
    // of its own and composited once, whatever surface the context turns out to draw on.
    let (records, _) = nums.as_chunks::<9>();
    let isolate = records
        .iter()
        .any(|rec| rec[0] as i32 == 25 && rec[1] as i32 != 0);
    if isolate {
        cr.push_group();
    }
    let mut ti = 0;
    let mut prefix = CanvasPrefix::default();
    // A decoded kind-20 record (stamp): the positions the next shape record is drawn at, once
    // each. Empty means the ordinary one-shape-one-record case (docs/canvas.md "Stamping").
    let mut stamp_at: Vec<(f64, f64)> = Vec::new();
    let mut stamp_n = 0usize;
    // The shadow in force, and the one to return to at each restore: cairo's own save stack
    // carries the transform, the clip and the operator, but knows no shadow.
    let mut shadow: Option<CanvasShadow> = None;
    let mut saved_shadows: Vec<Option<CanvasShadow>> = Vec::new();
    // The texts channel is positional: each payload-bearing record takes the next entry.
    let mut next_text = || -> &str {
        ti += 1;
        texts.get(ti - 1).map_or("", String::as_str)
    };
    for rec in records {
        let &[k, a, b, c, d, e, f, g, col] = rec;
        let k = k as i32;
        if k != 19 && k != 7 {
            prefix.font = None;
        }
        if k != 23 && k != 22 {
            prefix.source = None;
        }
        match k {
            // Stamp prefix and its coordinate records: collected, never drawn on their own.
            20 => {
                stamp_at.clear();
                stamp_n = a as usize;
                stamp_at.reserve(stamp_n);
                continue;
            }
            21 => {
                // Four points per record; the last record of a run is padded with zeros, so
                // the header's count is what says where the real ones stop.
                for pair in [(a, b), (c, d), (e, f), (g, col)] {
                    if stamp_at.len() < stamp_n {
                        stamp_at.push(pair);
                    }
                }
                continue;
            }
            _ if is_drawing_kind(k) => {
                let payload = if matches!(k, 7 | 11 | 12 | 15 | 16) {
                    next_text()
                } else {
                    ""
                };
                let draw = |cx: &gtk4::cairo::Context| cairo_draw_record(cx, rec, payload, &prefix);
                let paint = || {
                    if let Some(shadow) = &shadow {
                        let _ = cast_shadow(cr, shadow, &base, density, &draw);
                    }
                    draw(cr);
                };
                if stamp_at.is_empty() {
                    paint();
                } else {
                    // The template is painted once per position under a translated CTM.
                    for (dx, dy) in &stamp_at {
                        cr.save().ok();
                        cr.translate(*dx, *dy);
                        paint();
                        cr.restore().ok();
                    }
                }
                prefix.gradient = None;
                if matches!(k, 1 | 4 | 5 | 6 | 12 | 13 | 16) {
                    prefix.stroke = None;
                }
            }
            8 => {
                cr.save().ok();
                saved_shadows.push(shadow);
            }
            // A restore with no save to match is dropped: it would otherwise unwind the
            // caller's own state (and the blend group above).
            9 => {
                if let Some(saved) = saved_shadows.pop() {
                    cr.restore().ok();
                    shadow = saved;
                }
            }
            10 => {
                // Packed affine (a,b,c,d,tx,ty); cairo Matrix is (xx,yx,xy,yy,x0,y0) with the
                // same row-vector meaning as day_geometry::Affine.
                cr.transform(gtk4::cairo::Matrix::new(a, b, c, d, e, f));
            }
            // Set-gradient (f = type: 0 linear a,b→c,d; 1 radial a,b center + c radius; 2
            // conic a,b center + c start degrees); "offset,aarrggbb …" stops ride the texts
            // channel. Applies to the next shape record (encode_ops contract).
            14 => {
                let stops = next_text()
                    .split(' ')
                    .filter_map(|s| {
                        let (o, c) = s.split_once(',')?;
                        Some((o.parse::<f64>().ok()?, u32::from_str_radix(c, 16).ok()?))
                    })
                    .collect();
                prefix.gradient = Some(PendingGradient {
                    kind: f as u8,
                    a,
                    b,
                    c,
                    d,
                    stops,
                });
            }
            // Clip (17): slot f names the shape (0 rect, 1 rounded rect, 2 ellipse, 3 path,
            // 4 polygon); geometry in a..d, corner radius or fill rule in e.
            17 => {
                match f as i32 {
                    1 => cairo_rounded_rect(cr, a, b, c, d, e),
                    2 => {
                        cr.save().ok();
                        cr.translate(a + c / 2.0, b + d / 2.0);
                        cr.scale(c / 2.0, d / 2.0);
                        cr.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
                        cr.restore().ok();
                    }
                    3 => {
                        cairo_trace_path(cr, next_text());
                        cr.set_fill_rule(if e > 0.5 {
                            gtk4::cairo::FillRule::EvenOdd
                        } else {
                            gtk4::cairo::FillRule::Winding
                        });
                    }
                    4 => {
                        let _ = cairo_trace_polygon(cr, next_text());
                    }
                    _ => cr.rectangle(a, b, c, d),
                }
                cr.clip();
                cr.set_fill_rule(gtk4::cairo::FillRule::Winding);
            }
            // Stroke style (18): applies to the next stroke record only.
            18 => {
                prefix.stroke = Some(PendingStroke {
                    cap: a,
                    join: b,
                    miter: c,
                    phase: d,
                    dashes: next_text()
                        .split(' ')
                        .filter_map(|s| s.parse::<f64>().ok())
                        .collect(),
                });
            }
            // Font (19): applies to the next text record only.
            19 => {
                prefix.font = Some(PendingFont {
                    weight: a,
                    italic: b > 0.5,
                    family: next_text().to_owned(),
                });
            }
            // Image source (23): the region (a,b origin · c,d size, in the bitmap's pixels)
            // the next image record draws.
            23 => prefix.source = Some(day_spec::Rect::new(a, b, c, d)),
            // Shadow (24): a,b offset · c blur · the color slot its color; a clear color is
            // no shadow. State until the next one or the matching restore.
            24 => {
                let color = col as u32;
                shadow = (color >> 24 != 0).then_some(CanvasShadow {
                    dx: a,
                    dy: b,
                    blur: c.max(0.0),
                    color,
                });
            }
            // Blend (25): a = the mode number. cairo keeps the operator in the state its own
            // save and restore carry.
            25 => cr.set_operator(blend_operator(a as i32)),
            _ => {}
        }
        stamp_at.clear();
    }
    if isolate {
        // Unwind what the list left saved, so the group pops from the level it was pushed at.
        for _ in saved_shadows.drain(..) {
            cr.restore().ok();
        }
        if cr.pop_group_to_source().is_ok() {
            let _ = cr.paint();
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
/// Escape closes the topmost presented sheet cover, as it cancels a sheet on a desktop
/// (docs/cover.md "Modal sheet"): the cover hears it as its back request. In the capture
/// phase, so a web view or field focused inside the sheet never keeps it.
fn sheet_escape() -> gtk4::EventControllerKey {
    let keys = gtk4::EventControllerKey::new();
    keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
    keys.connect_key_pressed(|_, key, _, _| {
        ffi_guard::contain(gtk4::glib::Propagation::Proceed, || {
            if key != gtk4::gdk::Key::Escape {
                return gtk4::glib::Propagation::Proceed;
            }
            let top = COVERS.with(|c| {
                c.borrow()
                    .last()
                    .map(|(w, node)| (widget_key(&w.clone().upcast()), *node))
            });
            match top {
                Some((key, node)) if COVER_SHEET.with(|t| t.get(key).is_some()) => {
                    emit(
                        node,
                        Event::NavBack {
                            already_popped: false,
                        },
                    );
                    gtk4::glib::Propagation::Stop
                }
                _ => gtk4::glib::Propagation::Proceed,
            }
        })
    });
    keys
}

thread_local! {
    /// A text field's text being written by Day (`TextFieldPatch::Text`): its `changed`
    /// handler stays quiet, so the write is not reported back as typing (§4.4).
    static FIELD_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

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

/// The web name for a key that types nothing (docs/menus.md "The full route"), or `None` for
/// a key that types.
fn named_key(key: gtk4::gdk::Key) -> Option<&'static str> {
    use gtk4::gdk::Key;
    Some(match key {
        Key::Left | Key::KP_Left => "ArrowLeft",
        Key::Right | Key::KP_Right => "ArrowRight",
        Key::Up | Key::KP_Up => "ArrowUp",
        Key::Down | Key::KP_Down => "ArrowDown",
        Key::Return | Key::KP_Enter | Key::ISO_Enter => "Enter",
        Key::Tab | Key::KP_Tab | Key::ISO_Left_Tab => "Tab",
        Key::Escape => "Escape",
        Key::BackSpace => "Backspace",
        Key::Delete | Key::KP_Delete => "Delete",
        Key::Home | Key::KP_Home => "Home",
        Key::End | Key::KP_End => "End",
        Key::Page_Up | Key::KP_Page_Up => "PageUp",
        Key::Page_Down | Key::KP_Page_Down => "PageDown",
        Key::Insert | Key::KP_Insert => "Insert",
        Key::F1 => "F1",
        Key::F2 => "F2",
        Key::F3 => "F3",
        Key::F4 => "F4",
        Key::F5 => "F5",
        Key::F6 => "F6",
        Key::F7 => "F7",
        Key::F8 => "F8",
        Key::F9 => "F9",
        Key::F10 => "F10",
        Key::F11 => "F11",
        Key::F12 => "F12",
        _ => return None,
    })
}

/// A key press on the full route (`Decorate::keyboard`, docs/menus.md "The full route") that
/// the canvas's input method context did not take: the event for the app, or `None` for a
/// press that is not the app's. The order is the policy: a Meta/Super chord (a Mac's ⌘) is
/// the menu's; a key that types nothing is named; a Control or Alt chord names its key and
/// types nothing (the keyval already carries Shift and ignores those two, so Ctrl+C names
/// "c"); what is left typed a character the input method let through, and is text. A dead
/// key never gets here: the input method context composes it and commits the result.
fn full_route_key(
    key: gtk4::gdk::Key,
    state: gtk4::gdk::ModifierType,
) -> Option<day_spec::KeyEvent> {
    use gtk4::gdk::ModifierType as M;
    let mods = key_modifiers(state);
    if state.intersects(M::META_MASK | M::SUPER_MASK) {
        return None;
    }
    if let Some(named) = named_key(key) {
        return Some(day_spec::KeyEvent::named(named, mods));
    }
    let c = key.to_unicode().filter(|c| !c.is_control())?;
    if state.intersects(M::CONTROL_MASK | M::ALT_MASK) {
        return Some(day_spec::KeyEvent::named(c.to_string(), mods));
    }
    Some(day_spec::KeyEvent::typed_text(c.to_string(), mods))
}

/// GDK's modifier state as day's mask.
fn key_modifiers(state: gtk4::gdk::ModifierType) -> u8 {
    let mut m = 0u8;
    if state.contains(gtk4::gdk::ModifierType::SHIFT_MASK) {
        m |= day_spec::Modifiers::SHIFT.bits();
    }
    // Ctrl is both the primary key and the Control key, except on a Mac, where GTK's ⌘ is
    // Meta and the primary key (`accel_string` spells `<Primary>` as `<Meta>` there) and ⌃
    // is the Control key alone.
    if state.contains(gtk4::gdk::ModifierType::CONTROL_MASK) {
        m |= if cfg!(target_os = "macos") {
            day_spec::Modifiers::CONTROL.bits()
        } else {
            day_spec::Modifiers::PRIMARY.bits() | day_spec::Modifiers::CONTROL.bits()
        };
    }
    if cfg!(target_os = "macos") && state.contains(gtk4::gdk::ModifierType::META_MASK) {
        m |= day_spec::Modifiers::PRIMARY.bits();
    }
    if state.contains(gtk4::gdk::ModifierType::ALT_MASK) {
        m |= day_spec::Modifiers::ALT.bits();
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
                        .map_or_else(|| k.to_lowercase(), |n| n.to_string())
                }
                _ => k.to_lowercase(),
            }
        }
        k => k.to_string(),
    };
    format!("{acc}{key}")
}

/// A menu item's glyph, as the `GMenuModel` "icon" attribute `GtkPopoverMenu` renders
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

/// Register app-menu accelerators on the `GtkApplication` so shortcuts fire even without opening the bar.
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

/// Register the conventional `app.preferences` `GAction` when the lowered model carries a
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

/// nav host(Sidebar) → `NavSplit`; stack → `AdwNavigationView` (push/pop).
enum NavPresent {
    Documents(Rc<DocumentHost>),
    /// Sidebar, content list, and detail in nested `GtkPaneds` with draggable dividers.
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
        /// destination pages (`p1`, `p2`, …). They arrive from the `NAV_MENU`, which is built
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
                );
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
                        Event::SelectionChanged(i64::from(view.page_position(&page))),
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
                            num: f64::from(index),
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
    /// (page `GtkFixed` key, node id, its `AdwNavigationPage`) in order (index 0 = sidebar/root).
    /// A split pane's page has no `AdwNavigationPage`: its cell holds it directly.
    pages: Vec<(usize, NodeId, Option<adw::NavigationPage>)>,
    /// A programmatic pop is in flight: the `popped` handler must not re-emit `NavBack`.
    suppress: Rc<std::cell::Cell<bool>>,
}

struct NavMenuState {
    listbox: gtk4::ListBox,
    rows: usize,
    /// The rows' words and decorations as last given, for re-tinting rows whose tint depends
    /// on the appearance (`retint_nav_menus`).
    decor: NavDecor,
    /// Programmatic selection in flight: don't re-emit `SelectionChanged`.
    suppress: Rc<std::cell::Cell<bool>>,
    /// The row index last selected, by Day or by the user: a `row-selected` naming it again is
    /// an echo and is not reported.
    current: Rc<std::cell::Cell<Option<i32>>>,
}

/// A nav menu's per-row words and decorations (`NavMenuProps`, index-aligned).
#[derive(Default)]
struct NavDecor {
    items: Vec<String>,
    icons: Vec<Option<String>>,
    badges: Vec<Option<String>>,
    badge_icons: Vec<Option<String>>,
    tints: Vec<Option<day_spec::Ink>>,
    badge_tints: Vec<Option<day_spec::Ink>>,
}

impl NavDecor {
    /// The rows whose leading or trailing glyph is tinted with a pair or a role.
    fn dynamic_rows(&self) -> Vec<usize> {
        let follows = |t: &[Option<day_spec::Ink>], i: usize| {
            t.get(i)
                .copied()
                .flatten()
                .is_some_and(day_spec::Ink::is_dynamic)
        };
        (0..self.items.len())
            .filter(|&i| follows(&self.tints, i) || follows(&self.badge_tints, i))
            .collect()
    }
}

/// Re-tint every nav row whose glyph tint depends on the appearance (the tint is baked into
/// the glyph's texture), in place.
fn retint_nav_menus() {
    NAV_MENUS.with(|m| {
        for state in m.borrow().values() {
            let d = &state.decor;
            for i in d.dynamic_rows() {
                refresh_nav_row(
                    &state.listbox,
                    i,
                    &d.items[i],
                    d.icons.get(i).and_then(|o| o.as_deref()),
                    d.tints.get(i).copied().flatten(),
                    d.badges.get(i).and_then(|o| o.as_deref()),
                    d.badge_icons.get(i).and_then(|o| o.as_deref()),
                    d.badge_tints.get(i).copied().flatten(),
                );
            }
        }
    });
}

/// GTK 4.12's `gtk_list_view_scroll_to(view, position, flags, scroll_info)`.
type ListScrollToFn =
    unsafe extern "C" fn(*mut gtk4::ffi::GtkListView, u32, u32, *mut std::ffi::c_void);
/// GTK 4.12's `gtk_scroll_info_new()`.
type ScrollInfoNewFn = unsafe extern "C" fn() -> *mut std::ffi::c_void;
/// GTK 4.12's `gtk_scroll_info_set_enable_vertical` / `_horizontal(info, enable)`.
type ScrollInfoEnableFn = unsafe extern "C" fn(*mut std::ffi::c_void, std::ffi::c_int);

/// Bring row `row` of `view` into view with GTK's own `gtk_list_view_scroll_to` (4.12; the
/// binding is pinned at `v4_10`, so the symbols are looked up rather than linked). GTK lays the
/// row out, measured, at the edge it scrolls to, which is what a glide to a row of unknown
/// height needs before it can align it exactly. `false` where the running GTK lacks the call.
fn list_view_scroll_to(view: &gtk4::ListView, row: usize) -> bool {
    #[cfg(unix)]
    {
        let Ok(row) = u32::try_from(row) else {
            return false;
        };
        // SAFETY: `RTLD_DEFAULT` searches the images already loaded, the names are
        // NUL-terminated literals, and each pointer is used only after the null check below.
        let symbols = unsafe {
            [
                libc::dlsym(libc::RTLD_DEFAULT, c"gtk_list_view_scroll_to".as_ptr()),
                libc::dlsym(libc::RTLD_DEFAULT, c"gtk_scroll_info_new".as_ptr()),
                libc::dlsym(
                    libc::RTLD_DEFAULT,
                    c"gtk_scroll_info_set_enable_vertical".as_ptr(),
                ),
                libc::dlsym(
                    libc::RTLD_DEFAULT,
                    c"gtk_scroll_info_set_enable_horizontal".as_ptr(),
                ),
            ]
        };
        if symbols.iter().any(|p| p.is_null()) {
            return false;
        }
        let [scroll_to, info_new, vertical, horizontal] = symbols;
        // SAFETY: each non-null `dlsym` result for these names is the function GTK exports
        // under it, with the signature its header declares. The scroll info is fresh, and
        // `gtk_list_view_scroll_to` takes it over (transfer full), so it is not freed here.
        unsafe {
            let scroll_to = std::mem::transmute::<*mut std::ffi::c_void, ListScrollToFn>(scroll_to);
            let info_new = std::mem::transmute::<*mut std::ffi::c_void, ScrollInfoNewFn>(info_new);
            let vertical =
                std::mem::transmute::<*mut std::ffi::c_void, ScrollInfoEnableFn>(vertical);
            let horizontal =
                std::mem::transmute::<*mut std::ffi::c_void, ScrollInfoEnableFn>(horizontal);
            let info = info_new();
            if info.is_null() {
                return false;
            }
            vertical(info, 1);
            horizontal(info, 0);
            // GTK_LIST_SCROLL_NONE: scroll only, with no focus or selection change.
            scroll_to(view.as_ptr(), row, 0, info);
        }
        true
    }
    #[cfg(not(unix))]
    {
        let _ = (view, row);
        false
    }
}

fn widget_key(w: &Handle) -> usize {
    w.as_ptr() as usize
}

/// Day's laid-out origin for a `GtkFixed` child (`NODE_ORIGIN`), `(0, 0)` before it has one.
fn node_origin(widget: &gtk4::Widget) -> (f32, f32) {
    NODE_ORIGIN
        .with(|m| m.borrow().get(&widget_key(widget)).copied())
        .unwrap_or((0.0, 0.0))
}

/// Put a `GtkFixed` child at `frame` (in the fixed's coordinates): its origin, re-composed
/// with any transform it shows, and its size. `Toolkit::set_frame` and each frame of a frame
/// animation (motion.rs) go through here.
fn place_in_fixed(fixed: &gtk4::Fixed, h: &gtk4::Widget, frame: Rect) {
    let key = widget_key(h);
    NODE_ORIGIN.with(|m| {
        m.borrow_mut()
            .insert(key, (frame.origin.x as f32, frame.origin.y as f32))
    });
    fixed.move_(h, frame.origin.x, frame.origin.y);
    // `move_` rewrote the child transform to a plain translation, dropping any transform the
    // widget shows. Re-apply it over the new origin so a relayout (window resize, deep-link
    // mount) doesn't reset a transformed widget's scale/rotation/offset — or strand it at the
    // corner.
    if let Some(t) = motion::shown_transform(h)
        && !t.is_identity()
    {
        apply_gtk_transform(fixed, h, t, frame.size);
    }
    h.set_size_request(
        frame.size.width.max(0.0).round() as i32,
        frame.size.height.max(0.0).round() as i32,
    );
    surface_resized(key, (frame.size.width, frame.size.height));
}

/// Apply Day's layout origin AND the animation transform `t` to `widget` as its `GtkFixed` child
/// transform, about the widget's center. GTK positions a Fixed child *through* its child transform,
/// so the laid-out origin and the animation transform share one slot and must be composed here —
/// otherwise setting the transform would strand the widget at the fixed's (0,0) corner.
fn apply_gtk_transform(fixed: &gtk4::Fixed, widget: &gtk4::Widget, t: Transform, size: Size) {
    let (ox, oy) = node_origin(widget);
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
        .rotate(t.rotation.degrees() as f32)
        .scale(t.sx as f32, t.sy as f32)
        .translate(&gtk4::graphene::Point::new(-cx, -cy));
    fixed.set_child_transform(widget, Some(&transform));
}

/// A nav row's title: fills the row, ellipsized. Ellipsize, or a long feed title makes the row
/// wider than the sidebar: the `ListBox`'s natural width then wins over its allocation and GTK
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
    tint: Option<day_spec::Ink>,
    badge: Option<&str>,
    badge_icon: Option<&str>,
    badge_tint: Option<day_spec::Ink>,
) {
    // A tint is baked into the glyph's texture: resolved now, re-applied on an appearance
    // change (`retint_nav_menus`).
    let (tint, badge_tint) = (
        tint.map(day_spec::Ink::resolved),
        badge_tint.map(day_spec::Ink::resolved),
    );
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
    tint: Option<day_spec::Ink>,
    badge: Option<&str>,
    badge_icon: Option<&str>,
    badge_tint: Option<day_spec::Ink>,
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

/// Build a nav-menu `ListBox`'s rows (an optional template icon left of the label). Shared by the
/// `NAV_MENU` realize and the data-driven `NavMenuPatch::Items` rebuild.
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
    badge_tints: &[Option<day_spec::Ink>],
    sections: &[Option<String>],
    tints: &[Option<day_spec::Ink>],
    menus: &[Vec<day_spec::MenuItem>],
) {
    // Idempotent: a caller that clears the ListBox itself must call this before doing so (the
    // popovers are parented to the rows it is about to destroy), and the map entry is taken, so
    // running it twice is a no-op.
    unparent_nav_popovers(listbox);
    for (i, item) in items.iter().enumerate() {
        let decorated = icons.get(i).is_some_and(std::option::Option::is_some)
            || badges.get(i).is_some_and(std::option::Option::is_some)
            || badge_icons.get(i).is_some_and(std::option::Option::is_some);
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
/// heading above it, or nothing. Installed once per `ListBox` at realize.
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
        // An image surface: one device unit is one pixel, and the export's scale is in the
        // transform, where the painter reads it.
        cairo_draw(&cr, &nums, &texts, 1.0);
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

/// Pixels per point a shadow's blurred mask is rendered at in a PDF (288 per inch). A PDF has no
/// blur, so a canvas shadow is the one part of a page that is an image, not vector art.
const PDF_SHADOW_DENSITY: f64 = 4.0;

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
        cairo_draw(&cr, &nums, &texts, PDF_SHADOW_DENSITY);
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

/// Recolor premultiplied ARGB32 pixels (native-endian words, cairo's and
/// `gdk_texture_download`'s layout) to `tint`, keeping each pixel's alpha: the image becomes
/// a mask the color shows through (docs/images.md "Tinting").
fn tint_argb32(data: &mut [u8], tint: day_spec::Color) {
    let channel = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    let (r, g, b) = (channel(tint.r), channel(tint.g), channel(tint.b));
    for px in data.as_chunks_mut::<4>().0 {
        let alpha = u32::from_ne_bytes(*px) >> 24;
        let pre = |c: u32| (c * alpha + 127) / 255;
        let word = (alpha << 24) | (pre(r) << 16) | (pre(g) << 8) | pre(b);
        *px = word.to_ne_bytes();
    }
}

/// A copy of `texture` recolored to `tint` with its alpha kept: how a raster with no file
/// behind it (encoded bytes, a shared decode, a `GResource` entry) is tinted.
fn tint_texture(texture: &gtk4::gdk::Texture, tint: day_spec::Color) -> Option<gtk4::gdk::Texture> {
    let (w, h) = (texture.width(), texture.height());
    let stride = usize::try_from(w).ok()?.checked_mul(4)?;
    let mut data = vec![0u8; stride.checked_mul(usize::try_from(h).ok()?)?];
    if data.is_empty() {
        return None;
    }
    texture.download(&mut data, stride);
    tint_argb32(&mut data, tint);
    // Premultiplied ARGB in native-endian words, spelled out per byte order.
    let format = if cfg!(target_endian = "little") {
        gtk4::gdk::MemoryFormat::B8g8r8a8Premultiplied
    } else {
        gtk4::gdk::MemoryFormat::A8r8g8b8Premultiplied
    };
    Some(
        gtk4::gdk::MemoryTexture::new(w, h, format, &gtk4::glib::Bytes::from_owned(data), stride)
            .upcast(),
    )
}

/// The texture a tinted raster `source` draws: every pixel `tint`, the image's alpha kept.
/// `None` when the source has no pixels to recolor (an empty name, undecodable bytes, a
/// released decode), and the caller then draws the source as it is.
fn tinted_source_texture(
    source: &day_spec::ImageSource,
    tint: day_spec::Color,
) -> Option<gtk4::gdk::Texture> {
    let texture = match source {
        day_spec::ImageSource::Named(name) if name.is_empty() => return None,
        day_spec::ImageSource::Named(name) => {
            if let Some(texture) = tinted_image_texture(name, tint) {
                return Some(texture);
            }
            // Staged as a GResource entry only (§18.3), with no loose file to re-read.
            let res_path = format!("/day/images/{name}");
            gtk4::gio::resources_lookup_data(&res_path, gtk4::gio::ResourceLookupFlags::NONE)
                .ok()?;
            gtk4::gdk::Texture::from_resource(&res_path)
        }
        day_spec::ImageSource::Bytes(bytes) => {
            let data = gtk4::glib::Bytes::from_owned(bytes.as_ref().clone());
            gtk4::gdk::Texture::from_bytes(&data).ok()?
        }
        day_spec::ImageSource::Decoded(id) => {
            BITMAPS.with(|m| m.borrow().get(&id.0).map(|b| b.texture.clone()))?
        }
    };
    tint_texture(&texture, tint)
}

/// A realized image's source and paint (`IMAGE_SOURCE`): what it draws (a staged name,
/// encoded bytes or a shared decode), the tint on it now, whether it is a template glyph
/// (docs/vectors.md "Template"), and the picture itself — weakly, so a theme switch can
/// recolor every untinted template to the new foreground without keeping any alive.
#[derive(Clone)]
struct ImageEntry {
    source: day_spec::ImageSource,
    tint: Option<day_spec::Ink>,
    template: bool,
    picture: gtk4::glib::WeakRef<gtk4::Picture>,
}

/// The theme foreground a template glyph with no tint of its own draws in (docs/vectors.md
/// "Template"): the same near-white / near-black the sidebar's template icons use.
fn template_foreground() -> day_spec::Color {
    if adw::StyleManager::default().is_dark() {
        day_spec::Color::hex(0x00FF_FFFF)
    } else {
        day_spec::Color::hex(0x001A_1A1A)
    }
}

/// Paint a picture from its recorded source with `tint`, or — for a template — with the theme
/// foreground; a plain image with no tint loads the source as authored. A raster is recolored
/// pixel by pixel into a new texture (GTK has no template rendering for a `GtkPicture`), so
/// the same call serves realize, a tint patch and a source swap.
fn repaint_image(
    pic: &gtk4::Picture,
    source: &day_spec::ImageSource,
    tint: Option<day_spec::Ink>,
    template: bool,
) {
    let tint = tint
        .map(day_spec::Ink::resolved)
        .or_else(|| template.then(template_foreground));
    if let day_spec::ImageSource::Named(name) = source
        && let Some(path) = day_spec::resource::resolve_vector_svg(name)
    {
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
    match tint.and_then(|t| tinted_source_texture(source, t)) {
        Some(texture) => pic.set_paintable(Some(&texture)),
        None => set_picture_source(pic, source),
    }
}

/// A theme switch: every untinted template glyph takes the new foreground, and every image
/// tinted with an appearance-dependent ink takes its new color. The recolor is baked into
/// the texture, so nothing else would repaint it.
fn retint_images() {
    let mut live = Vec::new();
    IMAGE_SOURCE.with(|t| {
        t.for_each(|_, entry| {
            let follows = match entry.tint {
                Some(ink) => ink.is_dynamic(),
                None => entry.template,
            };
            if follows && let Some(pic) = entry.picture.upgrade() {
                live.push((pic, entry.source.clone(), entry.tint, entry.template));
            }
        });
    });
    for (pic, source, tint, template) in live {
        repaint_image(&pic, &source, tint, template);
    }
}

thread_local! {
    /// An appearance refresh is queued for the next idle (`queue_appearance_refresh`).
    static REFRESH_DUE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run [`refresh_appearance`] once on the next idle, however many notifies ask for it.
fn queue_appearance_refresh() {
    if REFRESH_DUE.with(|d| d.replace(true)) {
        return;
    }
    gtk4::glib::idle_add_local_once(|| {
        REFRESH_DUE.with(|d| d.set(false));
        ffi_guard::contain((), refresh_appearance);
    });
}

/// The appearance changed (light/dark, high contrast, the accent): Day reads the new palette,
/// then every ink GTK cannot re-resolve by itself is applied again, resolved against it
/// (docs/color.md "How a toolkit follows the appearance"). A role written into CSS as a named
/// color has already followed; a light/dark pair, an alpha-overridden role, a Pango color,
/// a recoloured texture and a border drawn over content have not.
fn refresh_appearance() {
    day_core::note_appearance_changed();
    restyle_dynamic_surfaces();
    restyle_dynamic_labels();
    retint_images();
    restyle_dynamic_buttons();
    redress_dynamic_list_items();
    retint_nav_menus();
}

/// Release this listbox's row popovers, and do it before the rows themselves go.
///
/// A `PopoverMenu` is parented to its row's label, and GTK4 requires a popover to be unparented
/// while that parent is still alive. Destroying the rows first leaves each popover pointing at
/// freed memory — GTK says so at the time ("Finalizing `GtkLabel`, but it still has children left:
/// `GtkPopoverMenu`") — and the later `unparent()` then walks a dead parent chain inside
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

/// Attach a context menu to one nav row (docs/menus.md): the same gio-menu + `PopoverMenu` +
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
    /// The row box of each entry in `LIST_ITEMS`, for the re-dress an appearance change asks
    /// of an icon tinted with a pair or a role (`redress_dynamic_list_items`).
    static LIST_ITEM_VIEWS: std::cell::RefCell<std::collections::HashMap<usize, gtk4::glib::WeakRef<gtk4::Widget>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Re-dress every list item whose icon tint depends on the appearance: the tint is baked into
/// the icon's texture.
fn redress_dynamic_list_items() {
    let items: Vec<(usize, day_spec::props::ListItemProps)> = LIST_ITEMS.with(|m| {
        m.borrow()
            .iter()
            .filter(|(_, p)| p.icon_tint.is_some_and(day_spec::Ink::is_dynamic))
            .map(|(k, p)| (*k, p.clone()))
            .collect()
    });
    for (key, props) in items {
        if let Some(w) =
            LIST_ITEM_VIEWS.with(|m| m.borrow().get(&key).and_then(gtk4::glib::WeakRef::upgrade))
        {
            dress_list_item(&w, &props);
        }
    }
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
    LIST_ITEM_VIEWS.with(|m| m.borrow_mut().insert(widget_key(&w), w.downgrade()));
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
        Some(day_spec::Icon::Image(name)) => {
            tinted_template_icon(name, p.icon_tint.map(day_spec::Ink::resolved))
        }
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
/// Whether `w` shows a context menu (`set_context_menu` or `set_context_menu_fn`), whose own
/// long press keeps priority over a `LongPress` recognizer (docs/gestures.md "Long press").
fn has_context_menu(w: &gtk4::Widget) -> bool {
    let key = widget_key(w);
    CTX_MENU_FNS.with(|m| m.borrow().contains_key(&key))
        || MENU_POPOVERS.with(|m| m.borrow().contains_key(&key))
}

/// The keyboard focus ring on a Day view that takes focus (docs/focus.md): Adwaita rings its
/// own controls, not a canvas or a container. `:focus-visible` holds only when focus came from
/// the keyboard. One display-wide provider.
fn focus_ring_css() {
    thread_local! {
        static INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if INSTALLED.with(|c| c.replace(true)) {
        return;
    }
    let p = gtk4::CssProvider::new();
    p.load_from_data(
        ".day-focusable:focus-visible { outline: 2px solid alpha(@accent_color, 0.5); \
         outline-offset: -2px; }",
    );
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &p,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

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

/// What a surface draws (docs/surfaces.md): the `ContainerProps` fields that become CSS, and
/// the border a Day container draws over its content.
#[derive(Clone, Debug, Default, PartialEq)]
struct SurfaceStyle {
    fill: Option<day_spec::Paint>,
    corner_radius: f64,
    corners: day_spec::Corners,
    border: Option<day_spec::Border>,
    shadow: Option<day_spec::Shadow>,
    clips: bool,
}

impl SurfaceStyle {
    /// A plain fill with the same radius on every corner: what `background` and
    /// `corner_radius` have always drawn.
    fn plain(background: Option<day_spec::Ink>, corner_radius: f64, clips: bool) -> Self {
        SurfaceStyle {
            fill: background.map(day_spec::Paint::Solid),
            corner_radius,
            clips,
            ..Default::default()
        }
    }

    fn of(p: &ContainerProps) -> Self {
        SurfaceStyle {
            fill: p.fill.clone(),
            corner_radius: p.corner_radius,
            corners: p.corners,
            border: p.border,
            shadow: p.shadow,
            clips: p.clips,
        }
    }

    /// The conic fill a Day container draws itself rather than as a CSS `conic-gradient()`:
    /// GSK's cairo renderer (the one `day launch` runs GTK apps on) draws that gradient as a
    /// cairo mesh, which some cairo builds paint as a few lines ([`mesh_rasterizes`]). So
    /// there, and only there, the container paints the fill the way a canvas does
    /// ([`input::set_fill_under`]). `None` for any other fill, and wherever meshes paint.
    fn draws_conic(&self) -> Option<&day_spec::ConicGradient> {
        match &self.fill {
            Some(day_spec::Paint::Conic(g)) if !mesh_rasterizes() => Some(g),
            _ => None,
        }
    }

    /// Whether there is anything for a CSS provider to say.
    fn is_styled(&self) -> bool {
        self.fill.is_some()
            || self.corner_radius > 0.0
            || self.border.is_some()
            || self.shadow.is_some()
            || self.clips
    }

    /// Whether any ink depends on the appearance: such a surface is written again when the
    /// appearance changes (docs/color.md "How a toolkit follows the appearance").
    fn is_dynamic(&self) -> bool {
        let stops = |stops: &[(f64, day_spec::Ink)]| stops.iter().any(|(_, i)| i.is_dynamic());
        let fill = match &self.fill {
            Some(day_spec::Paint::Solid(ink)) => ink.is_dynamic(),
            Some(day_spec::Paint::Linear(g)) => stops(&g.stops),
            Some(day_spec::Paint::Radial(g)) => stops(&g.stops),
            Some(day_spec::Paint::Conic(g)) => stops(&g.stops),
            None => false,
        };
        fill || self.border.is_some_and(|b| b.color.is_dynamic())
            || self.shadow.is_some_and(|s| s.color.is_dynamic())
    }

    /// The corner radii, per corner.
    fn radii(&self) -> day_spec::CornerRadii {
        if self.corner_radius > 0.0 {
            self.corners.radii(self.corner_radius)
        } else {
            day_spec::CornerRadii::default()
        }
    }

    /// The declarations for a widget of `size` points. Only a slanted linear gradient depends
    /// on the size ([`css_linear_gradient`]); everything else is the same at any frame.
    /// `inset_border` writes the border as an inset box shadow, under the content: the
    /// fallback for a widget that cannot draw it over its children (anything but a Day
    /// container, [`input::set_border_over`]).
    fn css(&self, size: (f64, f64), inset_border: bool) -> String {
        use std::fmt::Write as _;
        let mut body = String::new();
        match &self.fill {
            Some(day_spec::Paint::Solid(ink)) => {
                let _ = write!(body, "background-color: {};", css_ink(*ink));
            }
            Some(day_spec::Paint::Linear(g)) => {
                let _ = write!(body, "background-image: {};", css_linear_gradient(g, size));
            }
            Some(day_spec::Paint::Radial(g)) => {
                let _ = write!(body, "background-image: {};", css_radial_gradient(g));
            }
            Some(day_spec::Paint::Conic(g)) => {
                let _ = write!(body, "background-image: {};", css_conic_gradient(g));
            }
            None => {}
        }
        if self.corner_radius > 0.0 {
            // Top-left, top-right, bottom-right, bottom-left: CSS's order and `CornerRadii`'s.
            let r = self.radii();
            let _ = write!(
                body,
                "border-radius: {}px {}px {}px {}px;",
                r.top_left, r.top_right, r.bottom_right, r.bottom_left
            );
        }
        // The shadow is GTK's own blurred box shadow of the rounded box, outside it. The
        // border of a Day container is drawn by its snapshot after its children (over them,
        // docs/surfaces.md); elsewhere it is an INSET shadow with no blur and a spread of its
        // width: a band inside the frame that follows the rounded corners, under the content.
        // A CSS `border` would instead take room from the widget's content box and move every
        // child Day has placed.
        let mut shadows = Vec::new();
        if let Some(s) = self.shadow {
            shadows.push(format!(
                "{}px {}px {}px {}",
                s.dx,
                s.dy,
                s.blur.max(0.0),
                css_ink(s.color)
            ));
        }
        if inset_border
            && let Some(b) = self.border
            && b.width > 0.0
        {
            shadows.push(format!("inset 0 0 0 {}px {}", b.width, css_ink(b.color)));
        }
        if !shadows.is_empty() {
            let _ = write!(body, "box-shadow: {};", shadows.join(", "));
        }
        body
    }
}

/// A Day color as a CSS `rgba()`.
fn css_color(c: day_spec::Color) -> String {
    let ch = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    format!(
        "rgba({},{},{},{})",
        ch(c.r),
        ch(c.g),
        ch(c.b),
        c.a.clamp(0.0, 1.0)
    )
}

/// The libadwaita color each [`day_spec::SystemColor`] is, as a CSS color expression
/// (color brief §1.4). GTK resolves it against the stylesheet in force, so CSS that names one
/// follows a light/dark, high-contrast or accent change by itself; the same expression answers
/// `Toolkit::system_color` ([`role_color`]), so what Day resolves a role to is what GTK draws.
/// libadwaita 1.6 and later define the colors as CSS variables (the old `@named` colors stay
/// as aliases, but the accent alias does not follow the dark variant); 1.5 has the named
/// colors only. GTK's `alpha()` multiplies: the roles defined as a fraction of a foreground
/// are written that way.
fn role_css(role: day_spec::SystemColor) -> &'static str {
    use day_spec::SystemColor as R;
    // SAFETY: both return constants compiled into the linked libadwaita, readable before
    // GTK is initialized (the safe wrappers assert initialization first, which the unit tests
    // and an early style query do not have).
    let linked = unsafe {
        (
            adw::ffi::adw_get_major_version(),
            adw::ffi::adw_get_minor_version(),
        )
    };
    let variables = linked >= (1, 6);
    let pick = |var: &'static str, named: &'static str| if variables { var } else { named };
    match role {
        R::Accent => pick("var(--accent-bg-color)", "@accent_bg_color"),
        R::Text => pick("var(--window-fg-color)", "@window_fg_color"),
        R::SecondaryText => pick(
            "alpha(var(--window-fg-color), 0.55)",
            "alpha(@window_fg_color, 0.55)",
        ),
        R::TertiaryText => pick(
            "alpha(var(--window-fg-color), 0.35)",
            "alpha(@window_fg_color, 0.35)",
        ),
        R::Placeholder => pick(
            "alpha(var(--view-fg-color), 0.5)",
            "alpha(@view_fg_color, 0.5)",
        ),
        R::Link => pick("var(--accent-color)", "@accent_color"),
        R::Separator => pick(
            "alpha(var(--window-fg-color), 0.15)",
            "alpha(@window_fg_color, 0.15)",
        ),
        R::ControlFill => pick(
            "alpha(var(--window-fg-color), 0.1)",
            "alpha(@window_fg_color, 0.1)",
        ),
        R::Background | R::GroupedBackground => pick("var(--window-bg-color)", "@window_bg_color"),
        R::Surface => pick("var(--view-bg-color)", "@view_bg_color"),
        R::ElevatedSurface => pick("var(--popover-bg-color)", "@popover_bg_color"),
        R::Selection => pick(
            "alpha(var(--accent-bg-color), 0.3)",
            "alpha(@accent_bg_color, 0.3)",
        ),
        R::Destructive => pick("var(--destructive-color)", "@destructive_color"),
        R::Success => pick("var(--success-color)", "@success_color"),
        R::Warning => pick("var(--warning-color)", "@warning_color"),
    }
}

/// An ink as a CSS color: a role with no alpha of its own as its named color (GTK follows
/// the appearance itself), anything else resolved for the current appearance (re-written by
/// [`refresh_appearance`] when it changes). A role with an alpha override is resolved because
/// the override replaces the role's alpha, which GTK's `alpha()` cannot say.
fn css_ink(ink: day_spec::Ink) -> String {
    match ink {
        day_spec::Ink::System { role, alpha: None } => role_css(role).to_string(),
        _ => css_color(ink.resolved()),
    }
}

thread_local! {
    /// The display provider holding one `label.day-role-N { color: … }` rule per role, which
    /// [`role_color`]'s probes read; installed on first use.
    static ROLE_PROBES: std::cell::OnceCell<gtk4::CssProvider> = const { std::cell::OnceCell::new() };
}

/// `role`'s color as GTK draws it now: the computed `color` of a fresh, unparented label
/// styled with [`role_css`]. A fresh probe each time, since an unparented widget is not
/// restyled when the stylesheet changes. `None` before GTK has a display.
fn role_color(role: day_spec::SystemColor) -> Option<day_spec::Color> {
    use std::fmt::Write as _;
    let display = gtk4::gdk::Display::default()?;
    ROLE_PROBES.with(|probes| {
        probes.get_or_init(|| {
            let provider = gtk4::CssProvider::new();
            let mut rules = String::new();
            for r in day_spec::SystemColor::ALL {
                let _ = writeln!(
                    rules,
                    "label.day-role-{} {{ color: {}; }}",
                    r.wire(),
                    role_css(r)
                );
            }
            provider.load_from_data(&rules);
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            provider
        });
    });
    let probe = gtk4::Label::new(None);
    probe.add_css_class(&format!("day-role-{}", role.wire()));
    let c = probe.color();
    // libadwaita mixes its colors in Oklab, so a channel can land just outside sRGB.
    let ch = |v: f32| f64::from(v).clamp(0.0, 1.0);
    Some(day_spec::Color::rgba(
        ch(c.red()),
        ch(c.green()),
        ch(c.blue()),
        ch(c.alpha()),
    ))
}

/// Gradient stops as CSS `<color> <offset>%` items, the offset along the gradient's own line.
fn css_stops(stops: &[(f64, day_spec::Ink)]) -> String {
    stops
        .iter()
        .map(|(offset, ink)| format!("{} {:.4}%", css_ink(*ink), offset * 100.0))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A Day linear gradient as a CSS `linear-gradient()` for a box of `size` points.
///
/// Day's gradient runs between two unit points of the frame; CSS's runs along a line through
/// the box's center at an angle, as long as the box's projection onto it. So the angle is
/// the direction between the resolved points, and each stop's position is its point on Day's
/// segment projected onto the CSS line, which may fall outside 0–100% (CSS pads with the end
/// colors, as the canvas's gradients do). A vertical or horizontal gradient comes out the
/// same for any size; a slanted one depends on the aspect ratio and is rewritten when the
/// frame changes (`surface_resized`). A size not yet known counts as square.
fn css_linear_gradient(g: &day_spec::LinearGradient, size: (f64, f64)) -> String {
    let (w, h) = if size.0 > 0.0 && size.1 > 0.0 {
        size
    } else {
        (1.0, 1.0)
    };
    let (sx, sy) = (g.start.x * w, g.start.y * h);
    let (dx, dy) = (g.end.x * w - sx, g.end.y * h - sy);
    let len = dx.hypot(dy);
    if len <= f64::EPSILON || g.stops.is_empty() {
        // No direction to run in: the last stop everywhere, as a zero-length gradient pads.
        let c = g
            .stops
            .last()
            .map_or_else(|| css_color(day_spec::Color::CLEAR), |s| css_ink(s.1));
        return format!("linear-gradient({c}, {c})");
    }
    let (ux, uy) = (dx / len, dy / len);
    // CSS: 0deg points up and angles turn clockwise, so the direction is (sin, -cos).
    let angle = ux.atan2(-uy).to_degrees();
    let line = (w * ux).abs() + (h * uy).abs();
    let from = (sx - w / 2.0) * ux + (sy - h / 2.0) * uy;
    let stops: Vec<String> = g
        .stops
        .iter()
        .map(|(offset, ink)| {
            let at = ((from + offset * len) / line + 0.5) * 100.0;
            format!("{} {at:.4}%", css_ink(*ink))
        })
        .collect();
    format!("linear-gradient({angle:.4}deg, {})", stops.join(", "))
}

/// A Day radial gradient as a CSS `radial-gradient()`: an ellipse whose radii are `radius` of
/// the frame's width and height (Day's unit space, elliptical in a non-square frame), so the
/// same declaration serves every size.
fn css_radial_gradient(g: &day_spec::RadialGradient) -> String {
    if g.stops.is_empty() {
        return "none".to_string();
    }
    let r = g.radius.max(0.0) * 100.0;
    format!(
        "radial-gradient(ellipse {r:.4}% {r:.4}% at {:.4}% {:.4}%, {})",
        g.center.x * 100.0,
        g.center.y * 100.0,
        css_stops(&g.stops)
    )
}

/// A Day conic gradient as a CSS `conic-gradient()`. Day's angles run clockwise from +x on
/// screen and CSS's clockwise from up, so CSS starts a quarter turn later.
fn css_conic_gradient(g: &day_spec::ConicGradient) -> String {
    if g.stops.is_empty() {
        return "none".to_string();
    }
    format!(
        "conic-gradient(from {:.4}deg at {:.4}% {:.4}%, {})",
        g.start.degrees() + 90.0,
        g.center.x * 100.0,
        g.center.y * 100.0,
        css_stops(&g.stops)
    )
}

/// A surface's provider, what it was last told to draw, the frame it was written for, and the
/// widget (for the re-write an appearance change asks for).
struct SurfaceEntry {
    provider: gtk4::CssProvider,
    style: SurfaceStyle,
    size: (f64, f64),
    /// Whether the widget draws the border over its children ([`input::set_border_over`]);
    /// otherwise the CSS carries it as an inset shadow.
    over: bool,
    widget: gtk4::glib::WeakRef<gtk4::Widget>,
}

/// Apply a surface to a container widget via a scoped CSS provider (a unique
/// `.day-surface-N` class added to just this widget): fill or gradient, rounding per corner
/// and shadow; the border over the content (docs/surfaces.md). `overflow: hidden` rounds the
/// child clip, except under a shadow, where the surface is built not to clip. Idempotent:
/// a later call reloads the same provider with the new style.
fn apply_surface_style(w: &Handle, style: &SurfaceStyle) {
    let key = widget_key(w);
    let class = format!("day-surface-{key}");
    let border = style
        .border
        .filter(|b| b.width > 0.0)
        .map(|b| (b, style.radii()));
    let over = input::set_border_over(w, border);
    let drawn = input::set_fill_under(w, style.draws_conic().map(|g| (g.clone(), style.radii())))
        && style.draws_conic().is_some();
    let known = SURFACE.with(|t| {
        t.with(key, |e| {
            e.style = style.clone();
            e.over = over;
            (e.provider.clone(), e.size)
        })
    });
    let (provider, size) = if let Some(known) = known {
        known
    } else {
        let p = gtk4::CssProvider::new();
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &p,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        w.add_css_class(&class);
        let size = (f64::from(w.width()), f64::from(w.height()));
        SURFACE.with(|t| {
            t.insert(
                key,
                SurfaceEntry {
                    provider: p.clone(),
                    style: style.clone(),
                    size,
                    over,
                    widget: w.downgrade(),
                },
            );
        });
        (p, size)
    };
    // A fill the container draws itself is left out of the CSS.
    let css = if drawn {
        SurfaceStyle {
            fill: None,
            ..style.clone()
        }
        .css(size, !over)
    } else {
        style.css(size, !over)
    };
    provider.load_from_data(&format!(".{class} {{ {css} }}"));
    if style.clips || (style.corner_radius > 0.0 && style.shadow.is_none()) {
        w.set_overflow(gtk4::Overflow::Hidden);
    } else if style.shadow.is_some() {
        // A shadowed surface does not clip its content (docs/surfaces.md); a `GtkFixed`
        // does unless told otherwise. The shadow itself is outside any clip of this widget.
        w.set_overflow(gtk4::Overflow::Visible);
    }
}

/// `child` has just been put in its parent. If it is a surface with a shadow, or holds one,
/// the plain Day containers above it stop clipping their content, so the shadow can fall
/// outside the stack the surface sits in (docs/surfaces.md). The walk ends at the first
/// ancestor that is anything else: a container asked to clip or round, a scroll view, a list
/// cell, a page. Each container it passes is marked, so a subtree mounted later carries on
/// from where it is attached.
fn let_shadow_spill(child: &Handle) {
    if SPILLS.with(|t| t.get(widget_key(child))) != Some(true) {
        return;
    }
    let mut above = child.parent();
    while let Some(w) = above {
        let key = widget_key(&w);
        // Stop at anything that is not a plain container, and at one already open: the
        // containers above that one were opened when it was.
        if SPILLS.with(|t| t.with(key, |spills| std::mem::replace(spills, true))) != Some(false) {
            break;
        }
        w.set_overflow(gtk4::Overflow::Visible);
        above = w.parent();
    }
}

/// [`apply_surface_style`] for a plain fill: the cover's backdrop, and what `background` and
/// `corner_radius` ask for.
fn apply_surface(w: &Handle, bg: Option<day_spec::Ink>, corner_radius: f64, clips: bool) {
    apply_surface_style(w, &SurfaceStyle::plain(bg, corner_radius, clips));
}

/// A container's whole surface changed (`ContainerPatch::Surface`: a reactive `.surface`,
/// `.corner_radius`, `.border` or `.shadow`, or a frame-relative radius Day re-resolved).
/// Everything but the fill applies at once (GTK animates none of it); the fill goes through
/// [`motion::set_background`], so a solid fill animates as `ContainerPatch::Fill` does.
fn restyle_surface(h: &Handle, p: &ContainerProps, anim: Option<&day_spec::AnimSpec>) {
    let key = widget_key(h);
    let shown = SURFACE
        .with(|t| t.with(key, |e| e.style.fill.clone()))
        .flatten();
    let style = SurfaceStyle::of(p);
    if style.shadow.is_some() && SPILLS.with(|t| t.get(key)) != Some(true) {
        SPILLS.with(|t| t.insert(key, true));
        let_shadow_spill(h);
    }
    apply_surface_style(
        h,
        &SurfaceStyle {
            fill: shown,
            ..style.clone()
        },
    );
    motion::set_background(h, style.fill, anim);
}

/// The solid fill a surface shows now, in flight included (motion.rs writes each frame's
/// color here), resolved for the current appearance; `None` for a widget with no solid fill.
fn surface_background(w: &Handle) -> Option<day_spec::Color> {
    SURFACE
        .with(|t| {
            t.with(widget_key(w), |e| match &e.style.fill {
                Some(day_spec::Paint::Solid(ink)) => Some(ink.resolved()),
                _ => None,
            })
        })
        .flatten()
}

/// The fill a surface was last given (`None` without one or without a surface).
fn surface_fill(w: &Handle) -> Option<day_spec::Paint> {
    SURFACE
        .with(|t| t.with(widget_key(w), |e| e.style.fill.clone()))
        .flatten()
}

/// Change only a surface's fill (`ContainerPatch::Fill`, and each frame of a fill
/// animation): the rounding, border and shadow stay as realized. Writing the fill it already
/// has reloads nothing.
fn paint_fill(w: &Handle, fill: Option<day_spec::Paint>) {
    let style = SURFACE
        .with(|t| t.with(widget_key(w), |e| e.style.clone()))
        .unwrap_or_default();
    if SURFACE.with(|t| t.contains(widget_key(w))) && style.fill == fill {
        return;
    }
    apply_surface_style(w, &SurfaceStyle { fill, ..style });
}

/// A surface's frame changed: a gradient's unit points resolve against the frame, so a
/// slanted one is rewritten for the new aspect ratio. Nothing happens for any other surface,
/// or when the CSS comes out the same.
fn surface_resized(key: usize, size: (f64, f64)) {
    let reload = SURFACE.with(|t| {
        t.with(key, |e| {
            if !matches!(e.style.fill, Some(day_spec::Paint::Linear(_))) || e.size == size {
                return None;
            }
            let (old, new) = (e.style.css(e.size, !e.over), e.style.css(size, !e.over));
            e.size = size;
            (old != new).then(|| (e.provider.clone(), new))
        })
        .flatten()
    });
    if let Some((provider, css)) = reload {
        provider.load_from_data(&format!(".day-surface-{key} {{ {css} }}"));
    }
}

/// Write every surface with an appearance-dependent ink again: a pair or an alpha-overridden
/// role in the CSS takes the new color, and a border drawn over the content redraws.
fn restyle_dynamic_surfaces() {
    let mut live = Vec::new();
    SURFACE.with(|t| {
        t.for_each(|_, e| {
            if e.style.is_dynamic()
                && let Some(w) = e.widget.upgrade()
            {
                live.push((w, e.style.clone()));
            }
        });
    });
    for (w, style) in live {
        apply_surface_style(&w, &style);
    }
}

/// Emit each page's content size so `NavLayout` re-lays it (enqueue-only, §8.3). Split: each
/// pane's page gets its pane's width — the dividers are user-draggable, so this runs on every
/// drag and every allocation; stack: every page fills the host.
fn nav_report(host_key: usize) {
    let reports: Vec<(NodeId, Size)> = NAV_STATE.with(|m| {
        let m = m.borrow();
        let Some(state) = m.get(&host_key) else {
            return Vec::new();
        };
        let (hw, hh) = match &state.present {
            NavPresent::Split(sp) => (f64::from(sp.outer.width()), f64::from(sp.outer.height())),
            NavPresent::Stack(nv) => (f64::from(nv.width()), f64::from(nv.height())),
            NavPresent::Documents(doc) => {
                (f64::from(doc.view.width()), f64::from(doc.view.height()))
            }
            NavPresent::Suite { stack, .. } => {
                (f64::from(stack.width()), f64::from(stack.height()))
            }
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
                f64::from(cell.width())
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
                        .map_or(Size::new(hw, hh), |cell| {
                            Size::new(f64::from(cell.width()), f64::from(cell.height()))
                        }),
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
    /// zero, so the first placement takes the host's length instead. `GtkPaned` keeps a
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

/// Emit each inspector pane's content size so `InspectorLayout` re-lays it (the `nav_report`
/// counterpart). The panel reports its width even while hidden, so revealing it never re-lays
/// the panel's content from zero.
fn inspector_report(host_key: usize) {
    let reports: Vec<(NodeId, Size)> =
        INSPECTOR_STATE
            .with(|t| t.get(host_key))
            .map_or_default(|state| {
                let state = state.borrow();
                let (hw, hh) = (
                    f64::from(state.paned.width()),
                    f64::from(state.paned.height()),
                );
                if hw <= 0.0 || hh <= 0.0 {
                    return Vec::new();
                }
                // The panel's allocation once it has one (the divider may have moved it), its
                // remembered width until then and while hidden; the content gets the rest.
                let panel_w = if state.shown && state.panel.width() > 0 {
                    f64::from(state.panel.width())
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
            });
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
    /// The list's scrolled window (the host, or under a sectioned list's overlay) and the view
    /// in it.
    sw: gtk4::ScrolledWindow,
    listview: gtk4::ListView,
    /// `RowHeight::Uniform` without sections: every item row is one pitch tall, so a row's
    /// place is known before GTK lays it out (unless supplementary rows are in the list).
    uniform: bool,
    /// Row changes waiting for the next main-loop turn (`ListPatch::Reload` / `Splice`).
    pending: RefCell<PendingRows>,
    /// Bumped each time the model changes; `laid` catches up at the frame that lays the change
    /// out (`after_paint`). A row scroll waits for the two to agree.
    generation: std::cell::Cell<u64>,
    laid: std::cell::Cell<u64>,
    /// The viewport may have changed: report it after the next frame (docs/list.md "Visible
    /// rows").
    viewport_due: std::cell::Cell<bool>,
    /// Frames in a row at which a due report found nothing to read: past
    /// [`ListEntry::MISSED_FRAMES`] the report stops counting as owed (`viewport_owed`).
    missed: std::cell::Cell<u32>,
    /// The frame clock's `after-paint` handler, while the view is realized.
    frame_hook: RefCell<Option<(gtk4::gdk::FrameClock, gtk4::glib::SignalHandlerId)>>,
}

/// A list's row changes that have arrived since its model was last brought up to date.
#[derive(Default)]
struct PendingRows {
    /// Sequential deltas (`ListPatch::Splice`), in arrival order.
    deltas: Vec<RowDelta>,
    /// A reload arrived: the model is resized to the source and every shown row rebound, and
    /// any deltas are moot.
    reload: bool,
    /// The flush is posted.
    posted: bool,
}

impl ListEntry {
    /// Queue a row change (`None` = a reload) for the next main-loop turn. Not at once: a model
    /// change makes `GtkListView` bind rows synchronously, and the patch arrives inside a
    /// day-core borrow that `bind_row` would re-enter (see `schedule_list_resize`). Changes in
    /// one turn are applied together, in order.
    fn queue_rows(self: &Rc<Self>, deltas: Option<Vec<RowDelta>>) {
        {
            let mut pending = self.pending.borrow_mut();
            match deltas {
                None => {
                    pending.reload = true;
                    pending.deltas.clear();
                }
                Some(deltas) if !pending.reload => pending.deltas.extend(deltas),
                Some(_) => {}
            }
            if std::mem::replace(&mut pending.posted, true) {
                return;
            }
        }
        let weak = Rc::downgrade(self);
        gtk4::glib::idle_add_local_once(move || {
            if let Some(entry) = weak.upgrade() {
                // Contained: `len` is app code and the bind re-enters day-core.
                ffi_guard::contain((), || entry.flush_rows());
            }
        });
    }

    /// Bring the model up to date with the queued changes: the deltas as
    /// `g_list_model_items_changed` runs on the model when they fit it, and otherwise (a reload,
    /// or deltas against a row count the model does not have) a resize to the source's row
    /// count that rebinds the shown rows in place and keeps the scroll position.
    fn flush_rows(&self) {
        let PendingRows { deltas, reload, .. } = std::mem::take(&mut *self.pending.borrow_mut());
        let n = self.source.borrow().as_ref().map_or(0, |s| (s.len)());
        if let Some(selection) = &self.selection {
            selection.suppress.set(true);
        }
        let plan = (!reload)
            .then(|| scroll::plan_splice(self.model.n_items() as usize, &deltas, n))
            .flatten();
        let spliced = plan.is_some();
        if let Some(runs) = plan {
            let several = runs.len() > 1;
            for run in runs {
                let blanks = vec![""; run.added as usize];
                self.model.splice(run.at, run.removed, &blanks);
            }
            let mut items: Vec<gtk4::ListItem> = self
                .bound
                .borrow()
                .values()
                .filter_map(gtk4::glib::WeakRef::upgrade)
                .collect();
            // GtkListView binds the rows a run inserts as the run lands, at the index the row
            // has then; the source already answers in the final state, so a row inserted by
            // any run but the last was bound to whatever row ends at that index (in a sectioned
            // list, a header's content in an item's place). Bind the shown rows again where they
            // ended.
            if several {
                items.sort_by_key(gtk4::prelude::ListItemExt::position);
                rebind_list_items(items, n, &self.source);
            } else {
                // The rows that stayed moved with the run; a cell's drag reads its row from
                // here.
                for li in items {
                    if let Some(cell) = li.child()
                        && li.position() != gtk4::INVALID_LIST_POSITION
                    {
                        LIST_CELL_ROWS
                            .with(|t| t.insert(cell.as_ptr() as usize, li.position() as usize));
                    }
                }
            }
        } else {
            if !reload {
                log::warn!(
                    "list splice of {} deltas does not fit the {} rows shown (source has {n}): \
                     reloading",
                    deltas.len(),
                    self.model.n_items()
                );
            }
            let at = self.sw.vadjustment().value();
            list_resize(&self.model, n, &self.source, &self.bound);
            // GTK keeps its top row through a resize at the end of the model; a scroll on its
            // way (`follow_end`) stays in charge.
            if !scroll::gliding(&self.sw) {
                self.sw.vadjustment().set_value(at);
            }
        }
        if let Some(selection) = &self.selection {
            if spliced {
                // The selection followed its row through the runs.
                selection.requested.set(selection.model.selected());
            } else {
                selection.apply();
            }
            selection.suppress.set(false);
        }
        self.generation.set(self.generation.get() + 1);
        self.viewport_due.set(true);
    }

    /// Whether the model is waiting for queued changes or for the layout of the last ones.
    fn busy(&self) -> bool {
        self.pending.borrow().posted || self.laid.get() != self.generation.get()
    }

    /// Whether the list has supplementary rows (a header, footer, empty state or padding row),
    /// which are not one pitch tall.
    fn has_supplements(&self) -> bool {
        self.source
            .borrow()
            .as_ref()
            .is_some_and(|s| s.is_supplement.is_some())
    }

    /// Every row GTK has laid out in view or in its overscan: `(row, top, height)` in the
    /// viewport's coordinates. A bound row GTK keeps out of view is not child-visible.
    fn laid_rows(&self) -> Vec<(usize, f64, f64)> {
        self.bound
            .borrow()
            .values()
            .filter_map(|li| {
                let li = li.upgrade()?;
                let pos = li.position();
                if pos == gtk4::INVALID_LIST_POSITION {
                    return None;
                }
                let row = li.child()?.parent()?;
                if !row.is_visible() || !row.is_child_visible() {
                    return None;
                }
                let b = row.compute_bounds(&self.listview)?;
                Some((pos as usize, f64::from(b.y()), f64::from(b.height())))
            })
            .collect()
    }

    /// The frame clock painted: the model's last change is laid out, and a due viewport report
    /// goes out now, from this frame's row positions.
    fn after_paint(&self) {
        if self.pending.borrow().posted {
            return;
        }
        self.laid.set(self.generation.get());
        if self.viewport_due.get() {
            if self.report_viewport() {
                self.viewport_due.set(false);
                self.missed.set(0);
            } else {
                self.missed.set(self.missed.get().saturating_add(1));
            }
        }
    }

    /// How many frames a due report may find nothing to read before it stops holding up
    /// `ui_idle`: about half a second.
    const MISSED_FRAMES: u32 = 30;

    /// Whether this list still owes the app its viewport report (`Toolkit::ui_idle`, docs/list.md
    /// "Visible rows"): it is shown, and row changes wait for their flush or a move waits for the
    /// frame that lays it out. Until then the app reads the rows the list showed before, and a
    /// list that never reported counts as at its end (`follow_end`). A frame is asked for, so
    /// the report goes out at the next one.
    fn viewport_owed(&self) -> bool {
        let reports = self
            .source
            .borrow()
            .as_ref()
            .is_some_and(|s| s.viewport.is_some());
        if !reports || !self.listview.is_mapped() {
            return false;
        }
        if self.pending.borrow().posted {
            return true;
        }
        let owed = self.viewport_due.get()
            && self.missed.get() < Self::MISSED_FRAMES
            && self.model.n_items() > 0
            && self.sw.vadjustment().page_size() > 0.0;
        if owed {
            self.listview.queue_draw();
        }
        owed
    }

    /// Report the viewport through `ListSource::viewport` (docs/list.md "Visible rows"): the
    /// first and last rows with any part in view, from the laid rows (the uniform pitch before
    /// GTK laid any), and the distance from the viewport's bottom to the content's. `false`
    /// when there is nothing to read yet.
    fn report_viewport(&self) -> bool {
        let Some(report) = self
            .source
            .borrow()
            .as_ref()
            .and_then(|s| s.viewport.clone())
        else {
            return true;
        };
        let n = self.model.n_items() as usize;
        if n == 0 {
            return true;
        }
        let adj = self.sw.vadjustment();
        let (value, page, upper) = (adj.value(), adj.page_size(), adj.upper());
        if page <= 0.0 {
            return false;
        }
        let range = scroll::visible_range(&self.laid_rows(), page).or_else(|| {
            (self.uniform && !self.has_supplements())
                .then(|| scroll::pitch_range(value, page, upper / n as f64, n))
                .flatten()
        });
        let Some((first, last)) = range else {
            return false;
        };
        report(day_spec::ListViewport {
            first,
            last,
            end_distance: (upper - value - page).max(0.0),
        });
        true
    }

    /// Scroll to row `row` (the end of the content for `None`) as `align` says, animated or
    /// not (`Toolkit::list_scroll`, docs/list.md "Programmatic scrolling"). The target is
    /// worked out on each frame of the glide, after the rows' layout: a row GTK laid out by its
    /// real place; otherwise by the uniform pitch, or for rows of mixed heights by an estimate
    /// that the glide refines once GTK has laid the row out (on GTK 4.12 and later an instant
    /// scroll asks `gtk_list_view_scroll_to` to lay it out first). A sectioned list stops a row
    /// below its pinned header copy.
    fn scroll_rows(self: &Rc<Self>, row: Option<usize>, align: day_spec::RowAlign, animated: bool) {
        let weak = Rc::downgrade(self);
        let asked = std::cell::Cell::new(false);
        // The model generation the glide aims in, and its last target.
        let aimed: std::cell::Cell<Option<(u64, f64)>> = std::cell::Cell::new(None);
        scroll::glide(
            &self.sw,
            animated,
            move |sw| {
                let Some(entry) = weak.upgrade() else {
                    return scroll::Aim::Stop;
                };
                if entry.busy() {
                    return scroll::Aim::Wait;
                }
                let generation = entry.generation.get();
                if let Some((at, v)) = aimed.get()
                    && at != generation
                {
                    // The rows changed under the scroll: it ends where it was going rather than
                    // chasing the new end (an `on_end_reached` that appends would never stop). A
                    // list that follows its end asks again (`follow_end`).
                    return scroll::Aim::At {
                        v: Some(v),
                        h: None,
                        exact: true,
                    };
                }
                let n = entry.model.n_items() as usize;
                if n == 0 {
                    return scroll::Aim::Stop;
                }
                let adj = sw.vadjustment();
                let (value, page, upper) = (adj.value(), adj.page_size(), adj.upper());
                let end = (upper - page).max(0.0);
                let Some(row) = row.map(|r| r.min(n - 1)) else {
                    aimed.set(Some((generation, end)));
                    return scroll::Aim::At {
                        v: Some(end),
                        h: None,
                        exact: true,
                    };
                };
                let pitch = upper / n as f64;
                let laid = entry
                    .laid_rows()
                    .into_iter()
                    .find(|(r, ..)| *r == row)
                    .map(|(_, top, height)| (value + top, height));
                let (top, height, exact) = match laid {
                    Some((top, height)) => (top, height, true),
                    None if entry.uniform && !entry.has_supplements() => {
                        (pitch * row as f64, pitch, true)
                    }
                    None => {
                        if !animated
                            && !asked.replace(true)
                            && list_view_scroll_to(&entry.listview, row)
                        {
                            return scroll::Aim::Wait;
                        }
                        (pitch * row as f64, pitch, false)
                    }
                };
                let inset = entry.pinned.as_ref().map_or(0.0, |p| p.inset_for(row));
                let v = scroll::aligned_offset(value, page, end, top, height, align, inset);
                aimed.set(Some((generation, v)));
                scroll::Aim::At {
                    v: Some(v),
                    h: None,
                    exact,
                }
            },
            {
                let weak = Rc::downgrade(self);
                move |_| {
                    if let Some(entry) = weak.upgrade() {
                        entry.anchor_top();
                    }
                }
            },
        );
    }

    /// Anchor `GtkListView` at the topmost fully visible row, at the viewport's top edge, once
    /// a programmatic scroll rests. GTK re-anchors on every adjustment change, but keeps the
    /// anchor's previous alignment to pick the new one: after a glide that alignment can end up
    /// a fifth of the viewport down, and GTK then keeps up to 200 x that fraction rows ABOVE
    /// the anchor bound (its widget window is `GTK_LIST_VIEW_MAX_LIST_ITEMS` = 200 rows placed
    /// by the alignment), so rows a few dozen above the view stayed bound and answering element
    /// lookups. `gtk_list_view_scroll_to` on a fully visible row moves nothing and sets the
    /// anchor there with alignment 0 (`GtkListBase`'s `compute_scroll_align`), so the window
    /// is the view and the rows below it, as after a plain jump. GTK before 4.12: nothing to do.
    fn anchor_top(&self) {
        let top = self
            .laid_rows()
            .into_iter()
            .filter(|&(_, y, h)| h > 0.0 && y >= -0.5)
            .min_by_key(|&(row, ..)| row)
            .map(|(row, ..)| row);
        if let Some(row) = top {
            list_view_scroll_to(&self.listview, row);
        }
    }
}

/// The live state of list host `h`.
fn list_entry(h: &Handle) -> Option<Rc<ListEntry>> {
    LIST_STATE.with(|m| m.borrow().get(&widget_key(h)).cloned())
}

/// The pinned copy of a sectioned list's current header (docs/list.md "Sections";
/// `Cap::ListSections` is Emulated here).
///
/// `GtkListView`'s own section headers (`GtkSectionModel` + header factory, GTK 4.12) scroll away with
/// their rows, so a sectioned list's headers are ordinary rows of the flat model, and the list's
/// host is a `GtkOverlay` over its scrolled window. One extra `DayCell` above it holds the header of
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
            forward_scroll(w, sw, list_reader(&source));
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
        // A list that reports its whole viewport (`ListEntry::report_viewport`) does not report
        // its first row alone as well: the one-row range would overwrite the full one.
        if src.viewport.is_none()
            && let Some(report) = &src.first_visible
        {
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
/// GTK's own wheel step is the page size to the 2/3 power; a touchpad reports pixels. A step
/// that moved the list is the reader's own scroll (`reader`).
fn forward_scroll(w: &gtk4::Widget, sw: &gtk4::ScrolledWindow, reader: Rc<dyn Fn()>) {
    let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
    let adj = sw.vadjustment();
    scroll.connect_scroll(move |ctrl, _dx, dy| {
        let step = if ctrl.unit() == gtk4::gdk::ScrollUnit::Wheel {
            adj.page_size().powf(2.0 / 3.0)
        } else {
            1.0
        };
        let before = adj.value();
        adj.set_value(before + dy * step);
        if (adj.value() - before).abs() > f64::EPSILON {
            reader();
        }
        gtk4::glib::Propagation::Stop
    });
    w.add_controller(scroll);
}

/// The list's report of a scroll the reader started (`ListSource::reader_scroll`, docs/list.md
/// "Following the end"), read from `source` when the scroll happens: `attach_list` brings the
/// source after the host is made, and a list that does not follow its end has none.
fn list_reader(source: &Rc<RefCell<Option<ListSource>>>) -> Rc<dyn Fn()> {
    let source = source.clone();
    Rc::new(move || {
        let report = source
            .try_borrow()
            .ok()
            .and_then(|s| s.as_ref()?.reader_scroll.clone());
        if let Some(report) = report {
            report();
        }
    })
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

/// Whether display row `pos` is a plain row: a section header, or a supplementary row (the
/// list's header, footer, empty state or a content padding row, docs/list.md "Header, footer
/// and empty state"). Neither is ever selected, activated or focused.
fn plain_row(src: &ListSource, pos: usize) -> bool {
    src.is_header.as_ref().is_some_and(|f| f(pos))
        || src.is_supplement.as_ref().is_some_and(|f| f(pos))
}

/// Mark row `pos`'s list item for what it holds (`mark_list_header`). A supplementary row is
/// marked like a header: GTK takes the `.activatable` class off an unactivatable row, so it
/// draws no hover, press or selection, and the separators (`row.activatable` only) skip it.
/// Its height comes from `cell_height` like every row's (`DayCell`).
fn mark_list_row(li: &gtk4::ListItem, src: &ListSource, pos: usize) {
    if src.is_header.is_some() || src.is_supplement.is_some() {
        mark_list_header(li, plain_row(src, pos));
    }
}

/// A list's bound cells: physical cell (`DayCell` ptr) → the `GtkListItem` it fills. Written by the
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
        /// allocation. That is what the `External`-policy `ScrolledWindows` did around a nav pane's
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
                .map_or(0.0, |h| h(self.obj().as_ptr() as RawHandle));
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
                    .map_or_default(|f| (f.origin.x, f.origin.y, f.size.width, f.size.height));
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
            ffi_guard::contain((), move || layout(cell, f64::from(width)));
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

/// Day's row height is the row's height. `GtkListView`'s themed `row` node pads each row
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

/// The children of `parent` as a `StringList` of tokens, from the source's snapshot.
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
/// Open a menu button's menu (docs/menus.md "Menu buttons"): asks the provider now, so the
/// entries are the current ones, and shows the same `PopoverMenu` a context menu gets, pointing
/// at the whole button from below, the way `GtkMenuButton` places its popover.
fn pop_button_menu(w: &Handle, f: &day_spec::ContextMenuFn) {
    // The provider may remove this button; then no menu opens (docs/menus.md).
    let key = widget_key(w);
    let Some(items) =
        day_spec::resolve_button_menu(f, || BUTTON_MENUS.with(|t| t.get(key)).is_some())
    else {
        return;
    };
    let popover = menu_popover(w, &items);
    popover.set_position(gtk4::PositionType::Bottom);
    popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(
        0,
        0,
        w.width(),
        w.height(),
    )));
    popover.popup();
}

/// A one-shot `PopoverMenu` for `items`, parented to `w` and unparented once it closes.
fn menu_popover(w: &Handle, items: &[day_spec::MenuItem]) -> gtk4::PopoverMenu {
    let group = gtk4::gio::SimpleActionGroup::new();
    let model = build_gio_menu(items, &group);
    w.insert_action_group("daymenu", Some(&group));
    let popover = gtk4::PopoverMenu::from_model(Some(&model));
    popover.set_parent(w);
    popover.set_has_arrow(false);
    popover.connect_closed(|pop| {
        // Unparent outside the close handler — GTK is still walking the popover's state.
        let pop = pop.clone();
        gtk4::glib::idle_add_local_once(move || pop.unparent());
    });
    popover
}

fn show_menu_popover(w: &Handle, items: &[day_spec::MenuItem], x: f64, y: f64) {
    let popover = menu_popover(w, items);
    popover.set_pointing_to(Some(&gtk4::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.popup();
}

/// Bring the backing model to `n` rows (content is irrelevant — `bind_row` provides it), then
/// re-bind the rows GTK already shows, in place.
///
/// Only the count difference is spliced. Replacing every item would make `GtkListView` destroy
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
        .filter_map(gtk4::glib::WeakRef::upgrade)
        .collect();
    kept.sort_by_key(gtk4::prelude::ListItemExt::position);
    let (cur32, n32) = (cur as u32, n as u32);
    if n > cur {
        let blanks: Vec<&str> = vec![""; n - cur];
        model.splice(cur32, 0, &blanks);
    } else if n < cur {
        model.splice(n32, cur32 - n32, &[]);
    }
    rebind_list_items(kept, n, source);
}

/// Re-bind `items` to the rows at their positions now, top to bottom: their content, their
/// header or supplementary marking and their height. An item past row `n` is skipped (the
/// model's last change unbound it).
fn rebind_list_items(
    items: Vec<gtk4::ListItem>,
    n: usize,
    source: &Rc<RefCell<Option<ListSource>>>,
) {
    let Some(src) = source.borrow().clone() else {
        return;
    };
    let n32 = u32::try_from(n).unwrap_or(u32::MAX);
    for li in items {
        let pos = li.position();
        // A row past the new end was unbound by the splice above.
        if pos == gtk4::INVALID_LIST_POSITION || pos >= n32 {
            continue;
        }
        let Some(cell) = li.child() else { continue };
        LIST_CELL_ROWS.with(|t| t.insert(cell.as_ptr() as usize, pos as usize));
        mark_list_row(&li, &src, pos as usize);
        (src.bind_row)(pos as usize, cell.as_ptr() as RawHandle);
        if let Ok(cell) = cell.downcast::<DayCell>() {
            cell.bind_layout(src.layout_cell.clone(), src.cell_height.clone());
            // New content may need a new height (an Automatic row); GTK measured the old one.
            cell.queue_resize();
        }
    }
}

/// Resize the model on the next main-loop turn. CRUCIAL: `splice` makes `GtkListView` bind visible
/// cells SYNCHRONOUSLY (unlike `NSTableView`'s deferred reloadData), and a reload is driven from
/// inside a `with_tree` borrow — so resizing inline would re-enter `with_tree` (`bind_row`) and
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
            let n = source.borrow().as_ref().map_or(0, |s| (s.len)());
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
        static DIR: std::cell::OnceCell<Option<std::path::PathBuf>> =
            const { std::cell::OnceCell::new() };
    }
    DIR.with(|d| {
        d.get_or_init(|| {
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
        })
        .clone()
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
#[must_use]
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
#[must_use]
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
        log::info!(
            "day-toolkit-gtk: presenting the dialog as a window: its window hosts a native surface"
        );
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
                );
            });
        });
        let overlay = adw::ToastOverlay::new();
        overlay.set_child(Some(&cell));
        toolbar.set_content(Some(&overlay));
        dialog.set_child(Some(&toolbar));
        self.install(&dialog);
    }

    /// The `AdwPreferencesDialog` the settings shell fills: made on the first claim, answered
    /// again on later ones. `None` once a plain dialog holds the content.
    fn claim_preferences(self: &Rc<Self>) -> Option<adw::PreferencesDialog> {
        if let Some(d) = self.dialog.borrow().clone() {
            return d.downcast::<adw::PreferencesDialog>().ok();
        }
        let dialog = adw::PreferencesDialog::new();
        dialog.set_title(&self.title.borrow());
        self.install(dialog.upcast_ref());
        Some(dialog)
    }

    /// Keep `dialog`, wire its close to day-core's confirm path, and present it on the next
    /// main-loop turn.
    fn install(self: &Rc<Self>, dialog: &adw::Dialog) {
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
                        .retain(|h| h.dialog.borrow().as_ref() != Some(d));
                });
                // The window underneath is key again: an in-window dialog never changed the
                // toplevel's active state, so nothing else reports it.
                emit(node, Event::WindowFocused(false));
                if let Some(track) = state_track(&parent) {
                    emit(track.target, Event::WindowFocused(true));
                }
                // day-core tears down on a deferred hop, never inside this signal.
                emit(node, Event::WindowClosed);
            });
        });
        let host = Rc::downgrade(self);
        gtk4::glib::idle_add_local_once(move || {
            ffi_guard::contain((), || {
                if let Some(h) = host.upgrade() {
                    h.present();
                }
            });
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
        let bar = find_header_bar(dialog.upcast_ref()).map_or(0.0, |h| {
            let h = h.measure(gtk4::Orientation::Vertical, -1).1;
            if h > 0 {
                f64::from(h)
            } else {
                HEADER_BAR_FALLBACK_H
            }
        });
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
                    ffi_guard::contain((), || emit(node, Event::WindowClosed));
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
#[must_use]
pub fn preferences_dialog_of(root: NodeId) -> Option<adw::PreferencesDialog> {
    dialog_host_of_node(root)
        .filter(|h| h.kind == day_spec::WindowKind::Preferences)
        .and_then(|h| h.claim_preferences())
}

/// GTK 4.14's `gtk_accessible_announce(GtkAccessible *, const char *, GtkAccessibleAnnouncementPriority)`.
type AnnounceFn =
    unsafe extern "C" fn(*mut gtk4::ffi::GtkAccessible, *const std::ffi::c_char, std::ffi::c_int);

/// `gtk_accessible_announce`, if the GTK this process loaded has it (4.14; the binding is
/// pinned at `v4_10`, so the symbol is looked up rather than linked).
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
        );
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
    #[must_use]
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
            .and_then(gtk4::prelude::WidgetExt::root)
            .and_then(|r| r.downcast::<gtk4::Window>().ok());
        self.secondary
            .iter()
            .find(|w| w.window.is_active())
            .map(|w| w.window.clone().upcast())
            .or_else(|| {
                primary
                    .clone()
                    .filter(gtk4::prelude::GtkWindowExt::is_active)
            })
            .or_else(|| {
                LAST_ACTIVE_WINDOW
                    .with(|w| w.borrow().as_ref().and_then(gtk4::glib::WeakRef::upgrade))
                    .filter(gtk4::prelude::WidgetExt::is_visible)
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

    /// The `GtkWindow` whose content container is `host`: a secondary window's, or the
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
            .and_then(gtk4::prelude::WidgetExt::root)
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
                // SAFETY: on the main thread (`mtm`); `img` is a live NSImage the app retains.
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

/// Register the app's native `GResource` blob (§18.3) — `day build` compiles images + data into it and
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

/// `resource("name")` → the `/day/assets/<name>` `GResource` entry, borrowed zero-copy (the `GBytes`
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
#[must_use]
#[expect(
    clippy::match_same_arms,
    reason = "one row per text style, in the order of the type scale it approximates"
)]
pub fn gtk_style(f: Font) -> (f64, day_spec::FontWeight) {
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

/// A Day weight as a Pango one. Public for standalone pieces that build their own Pango
/// attributes or text tags (docs/extending.md).
#[must_use]
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

/// A label's remembered style: the base font, its color, the paragraph attributes and — for a
/// label with styled runs — the text and runs the markup is built from.
#[derive(Default, Clone)]
struct LabelStyle {
    font: day_spec::FontSpec,
    color: Option<day_spec::Ink>,
    /// `Some` once `.runs()` has put styled runs on this label; the pair rebuilds the markup
    /// whenever the base font or color is patched.
    rich: Option<(String, Vec<day_spec::TextRun>)>,
    /// `LabelProps::wraps`: `false` holds the label to one line whatever the limit says.
    wraps: bool,
    /// The line limit, truncation, letter spacing and line height (`LabelPatch::Paragraph`).
    /// Spacing and height are Pango attributes in device units, so they are rebuilt with the
    /// font and again when the font resolution (`gtk-xft-dpi`) changes.
    paragraph: day_spec::props::LabelParagraph,
    /// The label itself, for that re-application from the settings notify.
    label: gtk4::glib::WeakRef<gtk4::Label>,
}

/// The resolution Pango renders point sizes at: `gtk-xft-dpi` (in 1024ths of a dot per inch,
/// which GNOME's text-scaling-factor feeds), or Pango's own 96 when the setting is unset.
fn font_dpi() -> f64 {
    let dpi = gtk4::Settings::default().map_or(-1, |s| s.gtk_xft_dpi());
    if dpi > 0 {
        f64::from(dpi) / 1024.0
    } else {
        96.0
    }
}

/// The `gtk-xft-dpi` a desktop sets at its default text size: GDK's macOS backend reports 72
/// (one point is one pixel there); X11 and Wayland desktops start at 96 and grow it with the
/// text-scaling factor.
const DEFAULT_FONT_DPI: f64 = if cfg!(target_os = "macos") {
    72.0
} else {
    96.0
};

/// The size a label's font is drawn at, in pixels (Day's units): the style's points with
/// `FontSpec::scale` applied, through the font resolution.
fn resolved_font_px(spec: day_spec::FontSpec) -> f64 {
    spec.resolved_points(gtk_style(spec.style).0) * font_dpi() / 72.0
}

/// Letter spacing and an absolute line height in Pango units (1024ths of a pixel) for a font
/// drawn at `font_px`, or `None` where the paragraph keeps the font's own.
///
/// Both are multiples of the font size (docs/text.md "Letter spacing and line height"). A line
/// height has a floor of one pixel, since Pango markup reads a `line_height` below 1024 as a
/// factor rather than an absolute height.
fn paragraph_units(
    font_px: f64,
    p: &day_spec::props::LabelParagraph,
) -> (Option<i32>, Option<i32>) {
    let scale = f64::from(gtk4::pango::SCALE);
    let spacing = (p.letter_spacing.is_finite() && p.letter_spacing != 0.0)
        .then(|| (p.letter_spacing * font_px * scale).round() as i32)
        .filter(|&u| u != 0);
    let height = (p.line_height.is_finite() && p.line_height > 0.0)
        .then(|| ((p.line_height * font_px * scale).round() as i32).max(gtk4::pango::SCALE));
    (spacing, height)
}

/// A truncation mode as the Pango ellipsis that draws it.
fn ellipsize_mode(t: day_spec::props::Truncation) -> gtk4::pango::EllipsizeMode {
    use day_spec::props::Truncation;
    use gtk4::pango::EllipsizeMode;
    match t {
        Truncation::Tail => EllipsizeMode::End,
        Truncation::Head => EllipsizeMode::Start,
        Truncation::Middle => EllipsizeMode::Middle,
    }
}

/// How a label's lines end: one line ellipsized as `truncation` says when it does not wrap;
/// at most `max_lines` lines, the last one ellipsized, when it wraps with a limit; as many lines
/// as the text needs otherwise. `GtkLabel`'s `lines` applies only to an ellipsizing label, and
/// Pango counts it per paragraph, so a label with a limit also clips to its frame: Day measures
/// it at the first `max_lines` lines of the whole text, `\n` breaks included.
fn apply_line_limit(label: &gtk4::Label, wraps: bool, p: &day_spec::props::LabelParagraph) {
    use gtk4::prelude::*;
    let limited = wraps && p.max_lines > 0;
    label.set_wrap(wraps);
    label.set_lines(if limited {
        i32::try_from(p.max_lines).unwrap_or(i32::MAX)
    } else {
        -1
    });
    label.set_ellipsize(if wraps && !limited {
        gtk4::pango::EllipsizeMode::None
    } else {
        ellipsize_mode(p.truncation)
    });
    label.set_overflow(if limited {
        gtk4::Overflow::Hidden
    } else {
        gtk4::Overflow::Visible
    });
}

/// A label's size from a Pango layout holding its text and attributes, laid out the way the
/// `GtkLabel` draws it ([`apply_line_limit`]): at its natural width or the proposed one if
/// narrower, wrapped unless it is held to one line, and no taller than its first `max_lines`
/// lines.
///
/// `spacing_px` is the label's letter spacing. Pango trims it after a line's last character
/// but reserves it while breaking, so a wrapping layout exactly as wide as its natural width
/// breaks before the last character: the natural width carries one positive spacing as slack,
/// which is also the space other platforms leave after the last character.
fn measure_label_layout(
    layout: &gtk4::pango::Layout,
    wraps: bool,
    p: &day_spec::props::LabelParagraph,
    spacing_px: f64,
    width: Option<f64>,
) -> Size {
    let scale = f64::from(gtk4::pango::SCALE);
    layout.set_wrap(gtk4::pango::WrapMode::WordChar);
    let (nat_w, _) = layout.pixel_size();
    let nat_w = f64::from(nat_w) + spacing_px.max(0.0).ceil();
    let w = width.map_or(nat_w, |pw| nat_w.min(pw));
    let max_lines = if wraps {
        i32::try_from(p.max_lines).unwrap_or(i32::MAX)
    } else {
        1
    };
    if max_lines > 0 {
        // With an ellipsis Pango no longer wraps unless a negative height lets it.
        layout.set_ellipsize(ellipsize_mode(p.truncation));
        if wraps {
            layout.set_height(-max_lines);
        }
    }
    layout.set_width((w * scale).round() as i32);
    let (_, mut height) = layout.pixel_size();
    // Pango's limit is per paragraph; GtkLabel clips to its frame, so explicit breaks count.
    if wraps && max_lines > 0 && layout.line_count() > max_lines {
        let mut iter = layout.iter();
        for _ in 1..max_lines {
            if !iter.next_line() {
                break;
            }
        }
        let (_, logical) = iter.line_extents();
        height = (f64::from(logical.y() + logical.height()) / scale).ceil() as i32;
    }
    Size::new(w.ceil(), f64::from(height))
}

/// Give a widget and the internal widgets it is built from a reading direction
/// (`Toolkit::set_layout_direction`). GTK 4 widgets do not inherit a parent's direction, so a
/// composite control (an entry's text, a text area's view and placeholder, a slider's trough)
/// needs it on each part. The walk never enters a `GtkFixed`: that is a Day container whose
/// children are Day's own nodes, told their direction one by one.
fn set_subtree_direction(w: &gtk4::Widget, dir: gtk4::TextDirection) {
    w.set_direction(dir);
    if w.is::<gtk4::Fixed>() {
        return;
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        set_subtree_direction(&c, dir);
        child = c.next_sibling();
    }
}

/// Byte offset of the `chars`-th character of `text` (its end past the last one): `GtkLabel`
/// counts its selection in characters, `Event::TextSelected` in bytes.
fn char_to_byte(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(b, _)| b)
}

/// Report a reporting label's selection (`LabelProps::reports_selection`) once per change:
/// `TextSelected` with byte offsets into the text it displays, and one empty range when the
/// selection goes away (`0..0`). `last` is the range reported last, `None` after a clear.
fn report_label_selection(
    label: &gtk4::Label,
    id: NodeId,
    last: &std::cell::Cell<Option<(usize, usize)>>,
) {
    let text = label.text();
    let now = label.selection_bounds().and_then(|(a, b)| {
        let (a, b) = (a.min(b), a.max(b));
        let start = char_to_byte(&text, usize::try_from(a).ok()?);
        let end = char_to_byte(&text, usize::try_from(b).ok()?);
        (start < end).then_some((start, end))
    });
    if now == last.get() {
        return;
    }
    last.set(now);
    // GTK 4's label keeps no readable caret, so a clear is the empty range at the start.
    let (start, end) = now.unwrap_or_default();
    emit(id, Event::TextSelected { start, end });
}

/// Read a reporting label's selection back once the current event has been handled.
///
/// GTK 4's `GtkLabel` does not notify `selection-bound` or `cursor-position` (its selection
/// lives outside its properties' notify path), so the label is asked after everything that can
/// move the selection: pointer and key events on it, focus changes (`gtk-label-select-on-focus`
/// selects the whole text on focus in), its own cursor keybindings and new text. One idle per
/// main-loop turn coalesces a drag's motions; the compare in [`report_label_selection`] keeps
/// it to one report per change.
fn queue_selection_report(label: &gtk4::Label) {
    let key = label.clone().upcast::<gtk4::Widget>().as_ptr() as usize;
    if !LABEL_SELECTION.with(|t| t.contains(key)) {
        return;
    }
    let weak = label.downgrade();
    gtk4::glib::idle_add_local_once(move || {
        ffi_guard::contain((), || {
            let Some(label) = weak.upgrade() else {
                return;
            };
            if let Some((id, last)) = LABEL_SELECTION.with(|t| t.get(key)) {
                report_label_selection(&label, id, &last);
            }
        });
    });
}

/// Watch a reporting label for what can change its selection ([`queue_selection_report`]).
/// The legacy controller observes in the capture phase and never claims, so the label's own
/// selection gestures, links and Day's recognizers see every event as before.
fn watch_label_selection(label: &gtk4::Label, id: NodeId) {
    let key = label.clone().upcast::<gtk4::Widget>().as_ptr() as usize;
    LABEL_SELECTION.with(|t| t.insert(key, (id, Rc::new(std::cell::Cell::new(None)))));
    let events = gtk4::EventControllerLegacy::new();
    events.set_propagation_phase(gtk4::PropagationPhase::Capture);
    events.connect_event(|c, event| {
        use gtk4::gdk::EventType as T;
        if matches!(
            event.event_type(),
            T::ButtonPress
                | T::ButtonRelease
                | T::MotionNotify
                | T::KeyPress
                | T::KeyRelease
                | T::TouchBegin
                | T::TouchUpdate
                | T::TouchEnd
        ) && let Some(label) = c.widget().and_downcast::<gtk4::Label>()
        {
            queue_selection_report(&label);
        }
        gtk4::glib::Propagation::Proceed
    });
    label.add_controller(events);
    label.connect_has_focus_notify(queue_selection_report);
    label.connect_move_cursor(|l, _, _, _| queue_selection_report(l));
    label.connect_label_notify(queue_selection_report);
}

/// The base font, color and paragraph spacing as a Pango markup span that WRAPS a label's run
/// markup.
///
/// A `GtkLabel`'s attribute list OVERRIDES the attributes its markup parsed, so a base weight
/// attribute spanning the whole label silently defeats a `<b>` run — bold text rendered at the
/// body weight while italic, color and the monospace family (which set no base attribute) came
/// through. A rich label therefore carries NO attribute list, and its base font arrives as this
/// wrapping span. Inside one markup parse a nested tag wins over an enclosing span, which is the
/// ordering the run tags need.
fn base_span(style: &LabelStyle) -> String {
    use gtk4::pango;
    use std::fmt::Write as _;
    let spec = style.font;
    let (_, inherent) = gtk_style(spec.style);
    let weight = spec.weight.unwrap_or(inherent);
    let mut s = String::from("<span");
    // Monospace wins over a bundled family, as it does on Qt: the label asked for code type.
    if spec.monospace {
        s.push_str(" font_family=\"monospace\"");
    } else if let Font::Custom(family, _) = spec.style {
        let _ = write!(s, " font_family=\"{family}\"");
    }
    // Pango markup sizes are in 1024ths of a point, the same unit as `AttrSize`.
    let _ = write!(
        s,
        " size=\"{}\"",
        (spec.resolved_points(gtk_style(spec.style).0) * f64::from(pango::SCALE)) as i32
    );
    let _ = write!(
        s,
        " weight=\"{}\"",
        gtk4::glib::translate::IntoGlib::into_glib(pango_weight(weight))
    );
    if spec.italic {
        s.push_str(" style=\"italic\"");
    }
    let features = font_feature_string(spec);
    if !features.is_empty() {
        let _ = write!(s, " font_features=\"{features}\"");
    }
    // Pango units of the device, the same values the plain label's attribute list carries; a
    // `line_height` of 1024 or more is absolute (Pango 1.50, which GTK 4.6 requires).
    let (spacing, height) = paragraph_units(resolved_font_px(spec), &style.paragraph);
    if let Some(units) = spacing {
        let _ = write!(s, " letter_spacing=\"{units}\"");
    }
    if let Some(units) = height {
        let _ = write!(s, " line_height=\"{units}\"");
    }
    if let Some(c) = style.color.map(day_spec::Ink::resolved) {
        let ch = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        let _ = write!(
            s,
            " foreground=\"#{:02x}{:02x}{:02x}\"",
            ch(c.r),
            ch(c.g),
            ch(c.b)
        );
        if c.a < 1.0 {
            let _ = write!(s, " alpha=\"{}\"", (c.a.clamp(0.0, 1.0) * 65535.0) as u16);
        }
    }
    s.push('>');
    s
}

/// The OpenType features a font descriptor asks for, in Pango's `font_features` syntax:
/// `tnum 1, smcp 1, onum 1` (`FontSpec::feature_tags`). Empty when it asks for none. Features
/// change glyphs, never the face, and a font without one draws its ordinary glyphs.
fn font_feature_string(spec: day_spec::FontSpec) -> String {
    spec.feature_tags()
        .iter()
        .map(|tag| format!("{tag} 1"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The full markup for a rich label: its base font as a wrapping span, run tags inside.
fn rich_markup(text: &str, runs: &[day_spec::TextRun], style: &LabelStyle) -> String {
    let mut m = base_span(style);
    m.push_str(&day_spec::runs_to_markup(
        text,
        runs,
        day_spec::MarkupDialect::Pango,
        style.font.resolved_points(gtk_style(style.font.style).0),
    ));
    m.push_str("</span>");
    m
}

/// Rebuild a plain label's full Pango attribute list: font family/size/weight/style, font
/// features, letter spacing, line height and the foreground color.
fn apply_text_attrs(label: &gtk4::Label, style: &LabelStyle) {
    use gtk4::pango;
    let spec = style.font;
    let (style_pt, inherent) = gtk_style(spec.style);
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
    // `FontSpec::scale` (`.font_scale`, and the `max_text_scale` cap) multiplies the style's
    // size, which Pango then draws through the text-scaling resolution.
    let mut size =
        pango::AttrSize::new((spec.resolved_points(style_pt) * f64::from(pango::SCALE)) as i32);
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
    // Tabular figures, small capitals and oldstyle figures come from OpenType features rather
    // than a different family, so the face is untouched and only those glyphs change. A font
    // without a feature simply ignores it; nothing is synthesized.
    let features = font_feature_string(spec);
    if !features.is_empty() {
        let mut f = pango::AttrFontFeatures::new(&features);
        f.set_start_index(0);
        attrs.insert(f);
    }
    let (spacing, height) = paragraph_units(resolved_font_px(spec), &style.paragraph);
    if let Some(units) = spacing {
        let mut a = pango::AttrInt::new_letter_spacing(units);
        a.set_start_index(0);
        attrs.insert(a);
    }
    // Absolute, not the markup factor: the line box is the multiple of the font size, and
    // Pango splits the difference from the natural height above and below each line.
    if let Some(units) = height {
        let mut a = pango::AttrInt::new_line_height_absolute(units);
        a.set_start_index(0);
        attrs.insert(a);
    }
    if let Some(c) = style.color.map(day_spec::Ink::resolved) {
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

/// Render a label from its remembered style: markup for a label with runs (an attribute list
/// would override them, see [`base_span`]), the attribute list otherwise, then the line limit.
fn restyle_label(label: &gtk4::Label, style: &LabelStyle) {
    match &style.rich {
        Some((text, runs)) => {
            label.set_attributes(None);
            label.set_markup(&rich_markup(text, runs, style));
        }
        None => apply_text_attrs(label, style),
    }
    apply_line_limit(label, style.wraps, &style.paragraph);
}

/// Update part of a label's remembered style and re-apply the whole of it.
fn update_label_style(label: &gtk4::Label, change: impl FnOnce(&mut LabelStyle)) {
    use gtk4::prelude::*;
    let key = label.clone().upcast::<gtk4::Widget>().as_ptr() as usize;
    let style = LABEL_STYLE.with(|m| {
        let mut m = m.borrow_mut();
        let entry = m.entry(key).or_default();
        change(entry);
        entry.clone()
    });
    restyle_label(label, &style);
}

/// Re-apply every live label's style after the font resolution changed: letter spacing and
/// line height are device units computed from it.
/// Re-apply every label whose color, or one of whose runs' colors, depends on the
/// appearance: Pango attributes and markup carry concrete colors.
fn restyle_dynamic_labels() {
    fn run_follows(r: &day_spec::TextRun) -> bool {
        let deco = |d: &Option<day_spec::Decoration>| {
            d.as_ref()
                .and_then(|d| d.color)
                .is_some_and(day_spec::Ink::is_dynamic)
        };
        r.color.is_some_and(day_spec::Ink::is_dynamic)
            || r.background.is_some_and(day_spec::Ink::is_dynamic)
            || deco(&r.underline)
            || deco(&r.strikethrough)
    }
    let labels: Vec<(gtk4::Label, LabelStyle)> = LABEL_STYLE.with(|m| {
        m.borrow()
            .values()
            .filter(|s| {
                s.color.is_some_and(day_spec::Ink::is_dynamic)
                    || s.rich
                        .as_ref()
                        .is_some_and(|(_, runs)| runs.iter().any(run_follows))
            })
            .filter_map(|s| s.label.upgrade().map(|l| (l, s.clone())))
            .collect()
    });
    for (label, style) in &labels {
        restyle_label(label, style);
    }
}

fn restyle_all_labels() {
    let labels: Vec<(gtk4::Label, LabelStyle)> = LABEL_STYLE.with(|m| {
        m.borrow()
            .values()
            .filter_map(|s| s.label.upgrade().map(|l| (l, s.clone())))
            .collect()
    });
    for (label, style) in &labels {
        restyle_label(label, style);
    }
}

/// Register bundled fonts (§18.4) with whatever font system Pango draws from on this OS, so
/// `Font::Custom` family names resolve: fontconfig on Linux; on macOS both CoreText and
/// fontconfig (Homebrew Pango may use either fontmap depending on how it was built). Failures log and move on — the family
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
            // SAFETY: `c` is a NUL-terminated path that outlives the call, which fontconfig
            // copies; a null config is documented to mean the current default configuration.
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
            // SAFETY: `bytes` is `bytes.len()` readable bytes for the call; the URL follows the
            // Create rule, is used only when non-null and released once; a null error is allowed.
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
/// non-scrollable children in a `GtkViewport`, so `sw.child()` is the viewport, not our Fixed.
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

    fn system_color(&mut self, role: day_spec::SystemColor) -> day_spec::Color {
        // The libadwaita named color GTK draws the role in (`role_css`), read from the
        // stylesheet in force: the same expression a surface's CSS names, so what Day resolves
        // is what GTK shows.
        role_color(role).unwrap_or_else(|| role.fallback(self.dark_mode()))
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
        // which is not a request for less motion. A scripted run turns the setting off for
        // GTK's own widget transitions (`mute_widget_transitions`); that is not the user's
        // choice, so the user's value is answered instead.
        if let Some(enabled) = USER_ANIMATIONS.with(std::cell::Cell::get) {
            return !enabled;
        }
        gtk4::Settings::default().is_some_and(|s| !s.is_gtk_enable_animations())
    }

    fn text_scale(&mut self, _style: Font) -> f64 {
        // One factor for every style: Pango draws every point size through `gtk-xft-dpi`,
        // which GNOME's text-scaling-factor sets (docs/text.md "Text scale").
        font_dpi() / DEFAULT_FONT_DPI
    }

    fn set_layout_direction(
        &mut self,
        h: &Self::Handle,
        kind: PieceKind,
        direction: day_spec::LayoutDirection,
    ) {
        // Hosts whose GTK widget places its children by direction (a GtkPaned swaps its
        // panes, a GtkScrolledWindow moves its scrollbar) would move what Day placed: they
        // keep the app's direction. A Day container is a GtkFixed, which places children at
        // Day's coordinates in either direction, so it takes the direction for its hit slop.
        if matches!(
            kind,
            kinds::SCROLL
                | kinds::NAV
                | kinds::NAV_PAGE
                | kinds::LIST
                | kinds::LIST_CELL
                | kinds::TREE
                | kinds::COVER
                | kinds::INSPECTOR
                | kinds::INSPECTOR_PANE
                | kinds::SPLIT
                | kinds::SPLIT_PANE
        ) {
            return;
        }
        let dir = match direction {
            day_spec::LayoutDirection::Rtl => gtk4::TextDirection::Rtl,
            day_spec::LayoutDirection::Ltr => gtk4::TextDirection::Ltr,
        };
        set_subtree_direction(h, dir);
    }

    fn attach_refresh(&mut self, handle: &Self::Handle, host: NodeId) {
        controls::refresh_gesture::attach(handle, host);
    }

    fn set_presented(&mut self, handle: &Self::Handle, visible: bool) {
        handle.set_visible(visible);
    }

    /// Settled once no programmatic scroll is still gliding (`scroll::settled`) and every shown
    /// list has reported its viewport (`ListEntry::viewport_owed`): a scripted step after
    /// "scroll to row" or a row change then sees the rows the list shows now.
    fn ui_idle(&mut self) -> bool {
        scroll::settled()
            && LIST_STATE.with(|m| {
                let lists: Vec<Rc<ListEntry>> = m.borrow().values().cloned().collect();
                !lists.iter().any(|e| e.viewport_owed())
            })
    }

    #[expect(
        clippy::match_same_arms,
        reason = "one arm per capability, each documented with the GTK mechanism behind it"
    )]
    fn capability(&self, cap: Cap) -> Support {
        match cap {
            // A reordering sidebar is Day's own list standing in for the native one: the rows
            // drag natively, the sidebar does not (docs/navigation.md).
            Cap::NavReorder => Support::Emulated,
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
            // Opacity, transform, a `GtkFixed` child's frame and a surface's fill, stepped
            // from a tick callback on the window's frame clock through `AnimSpec::sample`
            // (motion.rs): the whole contract of docs/tween.md, read back mid-flight.
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
            // libcairo itself; Homebrew and Debian/Ubuntu both build that backend, so every GTK
            // target links and answers it.
            Cap::CanvasBitmap | Cap::CanvasPdf => Support::Native,
            // Focus (docs/focus.md): Tab order, `grab_focus`, the press that focuses before the
            // recognizers, `has-focus` reports both ways, the short and full key routes, and the
            // `:focus-visible` ring (input.rs).
            Cap::Focusable => Support::Native,
            // GTK repeats a held key as more `key-pressed` emissions, told apart by the keys a
            // controller holds down; `key-released` is the release (input.rs `HeldKeys`).
            Cap::KeyPhases => Support::Native,
            // `contains` on Day's containers, canvases and shapes (input.rs); a label, an image
            // or a native control keeps its own bounds (docs/gestures.md "Toolkit support").
            Cap::HitSlop => Support::Native,
            // `gtk_label_get_selection_bounds`, read back after every pointer, key and focus
            // event on a reporting label (GTK 4's label notifies no selection property).
            Cap::LabelSelection => Support::Native,
            // Lists and scroll views (docs/list.md, docs/scroll.md): the viewport from the laid
            // rows after each frame, row and target scrolls as glides over the frame clock, the
            // offset from the adjustments, and the phases from the user's own input on the
            // scrolled window (`scroll.rs`).
            Cap::ListViewport | Cap::ListScrollAlign | Cap::ScrollOffset | Cap::ScrollPhases => {
                Support::Native
            }
            // The scrolls the reader starts, from the same input on the list's scrolled window:
            // a wheel or touchpad step, the adjustment moving under a finger or a held scroll
            // bar, or just after a key that scrolls (`scroll::track_reader`).
            Cap::ListReaderScroll => Support::Native,
            // No GTK scroll view snaps or zooms, a desktop has no software keyboard, and the
            // pager is the composed one (`Cap::Pager` Emulated), whose progress steps with the
            // selection.
            Cap::ScrollSnap | Cap::ScrollZoom | Cap::ScrollKeyboardDismiss | Cap::PagerProgress => {
                Support::Unsupported
            }
            // GtkTextView has no selection switch: with selection off, Day collapses every
            // selection the buffer's `mark-set` reports, stops copy/cut/select-all and sets the
            // context menu aside (textarea.rs, docs/textarea.md).
            Cap::TextSelectable => Support::Emulated,
            // GtkTextView is editable-toggleable and ships no spell-check, so TextSpellCheck
            // stays Unsupported (the default arm).
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
            | Cap::DragDeferredReceipt | Cap::DragDrop | Cap::DragExternalImport | Cap::DragExternalExport | Cap::DragMultipleItems | Cap::DragFileReferences | Cap::ListReorder
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
                // Pass-through (docs/gestures.md): a container takes pointer input only once a
                // recognizer, menu, drop target, cursor or focus asks for it (input.rs).
                let w: Handle = input::DayFixed::new().upcast();
                if let Some(p) = props.downcast_ref::<ContainerProps>() {
                    if p.role == Some(day_spec::SurfaceRole::SectionCard) {
                        // libadwaita's own grouped-card treatment — rounded, elevated, and
                        // theme-adaptive (follows the Adwaita light/dark stylesheet).
                        w.add_css_class("card");
                    } else if p.role == Some(day_spec::SurfaceRole::Window) {
                        // Adwaita's `background` class, the theme window background an opaque
                        // cover paints; the rounding and clip ride the surface provider. An
                        // opaque window surface also stops the pointer, as a window does.
                        w.add_css_class("background");
                        input::take_input(&w);
                        if p.corner_radius > 0.0 || p.clips {
                            apply_surface(&w, None, p.corner_radius, p.clips);
                        }
                    } else {
                        let style = SurfaceStyle::of(p);
                        if style.shadow.is_some() {
                            SPILLS.with(|t| t.insert(widget_key(&w), true));
                        } else if !(style.clips || style.corner_radius > 0.0) {
                            SPILLS.with(|t| t.insert(widget_key(&w), false));
                        }
                        if style.is_styled() {
                            apply_surface_style(&w, &style);
                        }
                    }
                }
                w
            }
            Some(Builtin::Inspector) => {
                let (visible, width, edge) = props
                    .downcast_ref::<InspectorProps>()
                    .map_or((false, 280.0, PaneEdge::Trailing), |p| {
                        (p.visible, p.width, p.edge)
                    });
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
                    .map_or((day_spec::props::SplitAxis::Horizontal, 0.5, 80.0), |p| {
                        (p.axis, p.fraction, p.min_pane)
                    });
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
                    .is_some_and(|p| p.second);
                SPLIT_PANES.with(|t| t.insert(widget_key(&w), (id, second)));
                w
            }
            Some(Builtin::InspectorPane) => {
                let w: Handle = gtk4::Fixed::new().upcast();
                let panel = props
                    .downcast_ref::<InspectorPaneProps>()
                    .is_some_and(|p| p.panel);
                INSPECTOR_PANES.with(|t| t.insert(widget_key(&w), (id, panel)));
                w
            }
            Some(Builtin::Nav) => {
                let presentation = props
                    .downcast_ref::<NavProps>()
                    .map_or(day_spec::props::NavPresentation::Split, |p| p.presentation);
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
                                    nav_pane_report(key, pane, width, height);
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
                    .map_or_default(|p| (p.title.clone(), p.pane));
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
                let p = props_of::<CoverProps>(kind, "gtk", props);
                if p.is_some_and(|p| p.half) {
                    COVER_HALF.with(|t| t.insert(widget_key(&cover.clone().upcast()), ()));
                }
                if p.is_some_and(|p| (p.sheet || p.large) && !p.half) {
                    COVER_SHEET.with(|t| t.insert(widget_key(&cover.clone().upcast()), ()));
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
                                emit(id, Event::SelectionChanged(i64::from(row.index())));
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
                        .insert(widget_key(&handle), p.document_keys.clone());
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
                            decor: NavDecor {
                                items: p.items.clone(),
                                icons: p.icons.clone(),
                                badges: p.badges.clone(),
                                badge_icons: p.badge_icons.clone(),
                                tints: p.tints.clone(),
                                badge_tints: p.badge_tints.clone(),
                            },
                            suppress,
                            current,
                        },
                    )
                });
                handle
            }
            Some(Builtin::Scroll) => {
                let p = props
                    .downcast_ref::<day_spec::props::ScrollProps>()
                    .cloned()
                    .unwrap_or_default();
                let sw = gtk4::ScrolledWindow::new();
                // `.hide_indicators()` (docs/scroll.md): GTK_POLICY_EXTERNAL scrolls the axis
                // with no scroll bar. `bounce` has no switch of its own here (GTK's overshoot
                // comes with kinetic scrolling, which touch scrolling needs), `snap` and `zoom`
                // have no GTK counterpart, and a desktop has no software keyboard to dismiss:
                // those props are ignored and their caps say so.
                let bar = if p.indicators == ScrollIndicators::Hidden {
                    gtk4::PolicyType::External
                } else {
                    gtk4::PolicyType::Automatic
                };
                if p.horizontal {
                    sw.set_policy(bar, gtk4::PolicyType::Never);
                } else {
                    sw.set_policy(gtk4::PolicyType::Never, bar);
                }
                if p.track_offset {
                    scroll::track_offset(&sw, id);
                }
                if p.track_phase {
                    scroll::track_phase(&sw, id);
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
                // Pass-through (docs/gestures.md): a plain label lets presses reach what is
                // behind it. A link run, selection or a recognizer turns hit-testing back on.
                label.set_can_target(false);
                // Both halves of alignment. `xalign` places the text block inside the label's
                // allocation; `justify` aligns the WRAPPED LINES against each other. A centered
                // paragraph needs the second — with xalign alone the block sits centered while
                // its lines stay ragged-right, which is not what `TextAlign::Center` means.
                // GtkLabel reads `xalign` and the Left/Right justifications against the widget's
                // direction, so `Leading` follows `set_layout_direction`. Justified is Pango's
                // justify (every line but the last stretched), its last line at the leading edge.
                let (xalign, justify) = match p.align {
                    day_spec::props::TextAlign::Center => (0.5, gtk4::Justification::Center),
                    day_spec::props::TextAlign::Trailing => (1.0, gtk4::Justification::Right),
                    day_spec::props::TextAlign::Leading => (0.0, gtk4::Justification::Left),
                    day_spec::props::TextAlign::Justified => (0.0, gtk4::Justification::Fill),
                };
                label.set_xalign(xalign);
                label.set_justify(justify);
                label.set_yalign(0.0);
                label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
                update_label_style(&label, |s| {
                    s.font = p.font;
                    s.color = p.color;
                    s.wraps = p.wraps;
                    s.paragraph = p.paragraph();
                    s.label.set(Some(&label));
                });
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
                // Selection reports (`Label::on_selection`, Cap::LabelSelection), read back from
                // `gtk_label_get_selection_bounds` after each event that can move them.
                if p.reports_selection {
                    watch_label_selection(&label, id);
                }
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
                btn.connect_clicked(move |b| {
                    ffi_guard::contain((), || {
                        // A menu button's press opens its menu rather than reporting a press.
                        let key = widget_key(b.upcast_ref());
                        match BUTTON_MENUS.with(|t| t.get(key)) {
                            Some(f) => pop_button_menu(b.upcast_ref(), &f),
                            None => emit(id, Event::Pressed),
                        }
                    });
                });
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
                    ffi_guard::contain((), || emit(id, Event::ToggleChanged(s.is_active())));
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
                    // A value the app set (`SliderPatch::Value`) is not the reader's: GTK
                    // raises `value-changed` from inside `set_value` too (DESIGN.md §4.4).
                    if SLIDER_WRITING.get() {
                        return;
                    }
                    ffi_guard::contain((), || emit(id, Event::ValueChanged(s.value())));
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
                        ffi_guard::contain((), || emit(id, Event::ValueCommitted(scale.value())));
                    });
                }
                scale.add_controller(released);
                let keys = gtk4::EventControllerKey::new();
                keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
                {
                    let scale = scale.clone();
                    keys.connect_key_released(move |_, _k, _c, _m| {
                        ffi_guard::contain((), || emit(id, Event::ValueCommitted(scale.value())));
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
                    if FIELD_WRITE.with(std::cell::Cell::get) {
                        return;
                    }
                    ffi_guard::contain((), || emit(id, Event::TextChanged(e.text().to_string())));
                });
                entry.connect_activate(move |_| {
                    ffi_guard::contain((), || emit(id, Event::Submitted));
                });
                // The keys a field with a key handler claims (docs/textfield.md "Keys"), heard
                // in the capture phase so the inner GtkText never acts on them: Up and Down
                // would otherwise move focus to the neighboring widget.
                // With their phases (docs/menus.md "Keys"): a held arrow repeats, and the
                // release of a claimed key reaches the same handler.
                let held = input::HeldKeys::default();
                let keys = gtk4::EventControllerKey::new();
                keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
                keys.connect_key_pressed({
                    let held = held.clone();
                    move |_, key, code, state| {
                        ffi_guard::contain(gtk4::glib::Propagation::Proceed, || {
                            use gtk4::gdk::Key;
                            let name = match key {
                                Key::Up | Key::KP_Up => "ArrowUp",
                                Key::Down | Key::KP_Down => "ArrowDown",
                                Key::Escape => "Escape",
                                _ => return gtk4::glib::Propagation::Proceed,
                            };
                            // A repeat of a key whose down was claimed stays claimed.
                            let repeat = held.is_held(code);
                            if !repeat
                                && !day_spec::keys::field_claims(id, name, key_modifiers(state))
                            {
                                return gtk4::glib::Propagation::Proceed;
                            }
                            held.deliver(id, code, day_spec::KeyEvent::named(name, 0), repeat);
                            gtk4::glib::Propagation::Stop
                        })
                    }
                });
                {
                    let held = held.clone();
                    keys.connect_key_released(move |_, _, code, state| {
                        ffi_guard::contain((), || held.deliver_release(id, code, state));
                    });
                }
                entry.add_controller(keys);
                input::clear_on_blur(&entry, &held);
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
                let line = gtk4::Separator::new(orientation);
                // Pass-through (docs/gestures.md): a rule never stops a press.
                line.set_can_target(false);
                line.upcast()
            }
            Some(Builtin::Progress) => {
                let Some(p) = props_of::<ProgressProps>(kind, "gtk", props) else {
                    return placeholder_label(kind);
                };
                if p.value.is_none() {
                    return crate::controls::spinner::make(self, p, id);
                }
                if let Some(v) = p.value {
                    let bar = gtk4::ProgressBar::new();
                    bar.set_fraction(v);
                    bar.upcast()
                } else {
                    let spin = gtk4::Spinner::new();
                    spin.start();
                    spin.upcast()
                }
            }
            Some(Builtin::Canvas) => {
                // A Day canvas: pointer input only once something asks for it (input.rs).
                let area = input::DayCanvas::new(id);
                area.set_draw_func(|area, cr, _w, _h| {
                    // Contained: the ops decode runs app-supplied data inside GTK's C draw
                    // dispatch, where an unwound panic is an abort.
                    ffi_guard::contain((), || {
                        let ptr = area.as_ptr() as usize;
                        let (nums, texts) = OPS.with(|t| t.get(ptr)).unwrap_or_default();
                        let density = canvas_density(area.scale_factor());
                        cairo_draw(cr, &nums, &texts, density);
                    });
                });
                // Focus, and with it the keyboard (docs/menus.md). A DrawingArea is not
                // focusable by default — nothing would ever make it the focus widget — so
                // arrow keys aimed at what an app DRAWS need this and the click below.
                area.set_focusable(true);
                area.set_can_focus(true);
                wire_focus(&area, id);
                let handle: Handle = area.upcast();
                // A press focuses the canvas before its recognizers hear it. The input method
                // context for the full route (docs/menus.md "The full route") lets the keyboard
                // layout's dead keys and the user's input method (ibus, fcitx) compose and
                // commit once, as text; only a canvas that took the whole keyboard feeds it a
                // key. Keys carry their phases (input.rs).
                input::focus_on_press(&handle);
                input::wire_keys(&handle, id);
                focus_ring_css();
                handle.add_css_class("day-focusable");
                handle
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
                    ffi_guard::contain((), || emit(id, Event::ListActivated(row as usize)));
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
                                        // A section header or a supplementary row never
                                        // lifts (docs/list.md "Sections"); its cell may hold
                                        // an item row later.
                                        let plain = LIST_STATE.with(|m| {
                                            let m = m.borrow();
                                            let source = m.get(&host_key.get())?.source.borrow();
                                            Some(plain_row(source.as_ref()?, row))
                                        });
                                        if plain == Some(true) {
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
                                // A sectioned list's header rows (docs/list.md "Sections") and
                                // the supplementary rows. GtkListView pools one item type, so
                                // a recycled item can turn from a header into a row and back:
                                // mark it on every bind.
                                mark_list_row(li, src, pos);
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
                                let _ = t.remove(key);
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
                            // A header or a supplementary row is never selected (docs/list.md
                            // "Sections"). Its item is unselectable, but keyboard navigation
                            // on GTK before 4.12 (no item `focusable`) can still land on it:
                            // put the row the selection held back, past this notify.
                            let header = i != gtk4::INVALID_LIST_POSITION
                                && source
                                    .borrow()
                                    .as_ref()
                                    .is_some_and(|src| plain_row(src, i as usize));
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
                                    i64::from(i)
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
                    ffi_guard::contain((), || emit(id, Event::ListActivated(row as usize)));
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
                                // `.activatable` only: a supplementary row (and a section
                                // header) is unactivatable and gets no separator
                                // (docs/list.md "Header, footer and empty state").
                                "listview.day-separators > row.activatable { \
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
                    // reports the first visible row from the laid rows instead. A list that
                    // reports its whole viewport (`ListEntry::report_viewport`) does not
                    // report its first row alone.
                    if !p.sections {
                        vadj.connect_value_changed(move |adjustment| {
                            let source = source.borrow().clone();
                            if let Some(source) = source
                                && source.viewport.is_none()
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
                // The scrolls the reader starts (docs/list.md "Following the end"), told from
                // the list's own (`scroll.rs`).
                scroll::track_reader(&sw, list_reader(&source));
                // A sectioned list pins its current header over the rows (docs/list.md
                // "Sections"): the host is the overlay that holds the copy above the scroller.
                let pinned = p
                    .sections
                    .then(|| PinnedHeader::new(&sw, &listview, source.clone(), bound.clone()));
                let host: Handle = match &pinned {
                    Some(pinned) => pinned.overlay.clone().upcast(),
                    None => sw.clone().upcast(),
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
                            let n = source.borrow().as_ref().map_or(0, |s| (s.len)());
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
                            let (from_key, from) = DRAG_FROM.with(std::cell::Cell::get)?;
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
                                let Some((_, from)) = DRAG_FROM.with(std::cell::Cell::get) else {
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
                let entry = Rc::new(ListEntry {
                    model,
                    source,
                    selection,
                    bound,
                    pinned,
                    sw: sw.clone(),
                    listview: listview.clone(),
                    uniform: matches!(p.row_height, RowHeight::Uniform(_)) && !p.sections,
                    pending: RefCell::default(),
                    generation: std::cell::Cell::new(0),
                    laid: std::cell::Cell::new(0),
                    viewport_due: std::cell::Cell::new(false),
                    missed: std::cell::Cell::new(0),
                    frame_hook: RefCell::new(None),
                });
                // The viewport report (docs/list.md "Visible rows") is due whenever the offset,
                // the content's height or the viewport's moved; it goes out after the frame
                // that lays the move out, from the rows' positions in that frame.
                let vadj = sw.vadjustment();
                for signal in ["value-changed", "changed"] {
                    let weak = Rc::downgrade(&entry);
                    vadj.connect_local(signal, false, move |_| {
                        if let Some(entry) = weak.upgrade() {
                            entry.viewport_due.set(true);
                        }
                        None
                    });
                }
                listview.connect_realize({
                    let weak = Rc::downgrade(&entry);
                    move |view| {
                        let (Some(entry), Some(clock)) = (weak.upgrade(), view.frame_clock())
                        else {
                            return;
                        };
                        let hook = {
                            let weak = weak.clone();
                            clock.connect_after_paint(move |_| {
                                if let Some(entry) = weak.upgrade() {
                                    ffi_guard::contain((), || entry.after_paint());
                                }
                            })
                        };
                        if let Some((old, id)) = entry.frame_hook.replace(Some((clock, hook))) {
                            old.disconnect(id);
                        }
                    }
                });
                listview.connect_unrealize({
                    let weak = Rc::downgrade(&entry);
                    move |_| {
                        if let Some((clock, id)) =
                            weak.upgrade().and_then(|entry| entry.frame_hook.take())
                        {
                            clock.disconnect(id);
                        }
                    }
                });
                LIST_STATE.with(|m| m.borrow_mut().insert(widget_key(&host), entry));
                host
            }
            Some(Builtin::Image) => {
                let Some(p) = props_of::<ImageProps>(kind, "gtk", props) else {
                    return placeholder_label(kind);
                };
                let pic = gtk4::Picture::new();
                // Pass-through (docs/gestures.md): hit-testable once a recognizer asks.
                pic.set_can_target(false);
                // Scaling (§18.3): GtkPicture content-fit — Contain (fit) / Cover (fill) / Fill.
                pic.set_content_fit(match p.content_mode {
                    ContentMode::Fit => gtk4::ContentFit::Contain,
                    ContentMode::Fill => gtk4::ContentFit::Cover,
                    ContentMode::Stretch => gtk4::ContentFit::Fill,
                });
                // Tint (docs/vectors.md, docs/images.md "Tinting"): recolor every pixel to the
                // tint, keeping alpha as the mask — the same recolor the sidebar template
                // icons use. The source is recorded so a later tint patch can paint it again,
                // and a source swap keeps the tint.
                IMAGE_SOURCE.with(|t| {
                    t.insert(
                        widget_key(pic.upcast_ref()),
                        ImageEntry {
                            source: p.source.clone(),
                            tint: p.tint,
                            template: p.template,
                            picture: pic.downgrade(),
                        },
                    )
                });
                // A template with no tint (docs/vectors.md "Template") recolors to the theme
                // foreground, as the sidebar's own icons do, and is re-painted on a switch.
                repaint_image(&pic, &p.source, p.tint, p.template);
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
        anim: Option<&AnimSpec>,
    ) {
        match kind {
            kinds::LIST_ITEM => {
                let Some(p) = patch.downcast_ref::<day_spec::props::ListItemPatch>() else {
                    return;
                };
                let props = LIST_ITEMS.with(|m| {
                    use day_spec::props::ListItemPatch as P;
                    let mut m = m.borrow_mut();
                    let props = m.get_mut(&widget_key(h))?;
                    match p {
                        P::Title(t) => props.title.clone_from(t),
                        P::Subtitle(t) => props.subtitle.clone_from(t),
                        P::Detail(t) => props.detail.clone_from(t),
                        P::Icon(i) => props.icon.clone_from(i),
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
                        ffi_guard::contain((), || inspector_report(key));
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
                            if background.is_some() {
                                cover.remove_css_class("background");
                                apply_surface(h, *background, 0.0, false);
                            } else {
                                apply_surface(h, None, 0.0, false);
                                cover.add_css_class("background");
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
                                let size =
                                    Size::new(f64::from(area.width()), f64::from(area.height()));
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
                                repaint_image(pic, &entry.source, *c, entry.template);
                            }
                        }
                        // A source swap repaints the same widget (docs/images.md), so an `image()`
                        // bound to a signal shows new pixels without rebuilding its subtree. A
                        // tinted or template image is re-rendered from the new source in the same
                        // paint, so the tint survives the swap and a recycled sidebar row shows
                        // the icon it was rebound to.
                        day_spec::props::ImagePatch::Source(source) => {
                            let entry = IMAGE_SOURCE.with(|t| {
                                t.with(widget_key(h), |e| {
                                    e.source = source.clone();
                                    e.clone()
                                })
                            });
                            match entry {
                                Some(e) => repaint_image(pic, source, e.tint, e.template),
                                None => set_picture_source(pic, source),
                            }
                        }
                    }
                }
            }
            kinds::CONTAINER => {
                match patch.downcast_ref::<ContainerPatch>() {
                    // Only the fill changes, blended from the color shown when animated
                    // (docs/tween.md rule 8); the rounding, border and shadow stay as realized.
                    Some(ContainerPatch::Fill(fill)) => {
                        motion::set_background(h, fill.clone(), anim);
                    }
                    // The realize-time surface again on the same widget (color brief §3).
                    Some(ContainerPatch::Surface(p)) => restyle_surface(h, p, anim),
                    None => {}
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
                    // `movable` is for toolkits whose own sidebar drags (Cap::NavReorder).
                    ..
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
                        state.decor = NavDecor {
                            items: items.clone(),
                            icons: icons.clone(),
                            badges: badges.clone(),
                            badge_icons: badge_icons.clone(),
                            tints: tints.clone(),
                            badge_tints: badge_tints.clone(),
                        };
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
                        entry.1.clone_from(items);
                        entry.2.clone_from(icons);
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
                        // Idempotent: the same paragraph re-applies the same properties, which
                        // GtkLabel's setters compare before they notify.
                        LabelPatch::Paragraph(p) => update_label_style(label, |s| s.paragraph = *p),
                        LabelPatch::Text(t) => {
                            if label.text() != t.as_str() {
                                label.set_text(t);
                            }
                        }
                        LabelPatch::Font(f) => update_label_style(label, |s| s.font = *f),
                        LabelPatch::Color(c) => update_label_style(label, |s| s.color = *c),
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
                            apply_button_content(btn, &c.title, c.icon.as_ref(), c.icon_only);
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
                                SLIDER_WRITING.set(true);
                                scale.set_value(*v);
                                SLIDER_WRITING.set(false);
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
                                // Day's own write, not typing: `set_text` emits `changed` for
                                // the deletion and again for the insertion, and reporting those
                                // would hand the app back an empty field and then its own text.
                                FIELD_WRITE.with(|w| w.set(true));
                                entry.set_text(text);
                                FIELD_WRITE.with(|w| w.set(false));
                            }
                        }
                        TextFieldPatch::Placeholder(t) => entry.set_placeholder_text(Some(t)),
                        TextFieldPatch::Enabled(e) => entry.set_sensitive(*e),
                        // GtkEditable counts characters, Day's unit. An unfocused entry keeps
                        // the selection, though a keyboard focus-in selects everything.
                        TextFieldPatch::Select(sel) => {
                            let sel = sel.clamped(&entry.text());
                            entry.select_region(sel.start as i32, sel.end as i32);
                        }
                    }
                }
            }
            kinds::LIST => match patch.downcast_ref::<ListPatch>() {
                Some(patch @ (ListPatch::Reload | ListPatch::Splice(_))) => {
                    if let Some(e) = list_entry(h) {
                        // Deferred: this runs inside a with_tree borrow (`ListEntry::queue_rows`).
                        e.queue_rows(match patch {
                            ListPatch::Splice(deltas) => Some(deltas.clone()),
                            _ => None,
                        });
                        if let Some(pinned) = &e.pinned {
                            pinned.reset_later();
                        }
                    }
                }
                // Superseded by `list_scroll`, which this backend handles: day-core sends these
                // two only to a backend that does not.
                Some(ListPatch::ScrollToEnd) => {
                    if let Some(e) = list_entry(h) {
                        e.scroll_rows(None, day_spec::RowAlign::Nearest, false);
                    }
                }
                Some(ListPatch::ScrollToRow(row)) => {
                    if let Some(e) = list_entry(h) {
                        e.scroll_rows(Some(*row), day_spec::RowAlign::Nearest, false);
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
                            let item = bound
                                .borrow()
                                .get(&cell)
                                .and_then(gtk4::glib::WeakRef::upgrade);
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
                        (*entry.selected.borrow_mut()).clone_from(tokens);
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
                                        let y = (adj.upper() / f64::from(n)) * f64::from(i);
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
        LIST_ITEM_VIEWS.with(|m| m.borrow_mut().remove(&widget_key(&h)));
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
        if let Some(list) = &list {
            // The frame clock outlives the list: its viewport hook goes now.
            if let Some((clock, id)) = list.frame_hook.take() {
                clock.disconnect(id);
            }
            // A sectioned list's scrolled window is not the host, so no sweep reaches it.
            day_spec::sidetable::sweep(list.sw.as_ptr() as usize);
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
                    }
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
                            emit(
                                id,
                                Event::FrameChanged(Size::new(f64::from(w), f64::from(h))),
                            );
                        }
                    });
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
                                emit(
                                    id,
                                    Event::FrameChanged(Size::new(f64::from(w), f64::from(h))),
                                );
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
                                );
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
                ffi_guard::contain((), || nav_report(host_key));
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
                ffi_guard::contain((), || inspector_report(host_key));
            });
        } else if let Some(fixed) = content_of(parent).downcast_ref::<gtk4::Fixed>() {
            fixed.put(child, 0.0, 0.0);
            let_shadow_spill(child);
        } else if let Some(cell) = content_of(parent).downcast_ref::<DayCell>() {
            // A list row's content: the cell places it from the frames `set_frame` records.
            cell.add_child(child);
        }
    }

    fn remove(&mut self, parent: &Handle, child: &Handle) {
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
                let rows = NAV_MENUS.with(|m| m.borrow().get(&widget_key(h)).map_or(0, |s| s.rows));
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
                    return Size::new(f64::from(nat_w), f64::from(nat_h));
                };
                let layout = gtk4::pango::Layout::new(&label.pango_context());
                // A label with styled runs measures from its MARKUP: `label.text()` is the
                // markup stripped of tags and its attribute list is empty (see `base_span`), so
                // measuring those two would size bold and monospace runs at the base font.
                let key = label.clone().upcast::<gtk4::Widget>().as_ptr() as usize;
                let (wraps, paragraph, spacing_px) = LABEL_STYLE.with(|m| {
                    m.borrow()
                        .get(&key)
                        .map_or((true, LabelParagraph::default(), 0.0), |s| {
                            let units = paragraph_units(resolved_font_px(s.font), &s.paragraph).0;
                            let px = f64::from(units.unwrap_or(0)) / f64::from(gtk4::pango::SCALE);
                            (s.wraps, s.paragraph, px)
                        })
                });
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
                if let Some(markup) = rich {
                    layout.set_markup(&markup);
                } else {
                    layout.set_text(&label.text());
                    layout.set_attributes(label.attributes().as_ref());
                }
                measure_label_layout(&layout, wraps, &paragraph, spacing_px, p.width)
            }
            kinds::PICKER => picker::measure_any(self, h, p),
            kinds::TEXT_AREA => textarea::measure_any(self, h, p),
            kinds::SLIDER | kinds::TEXT_FIELD => {
                let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
                Size::new(p.width.unwrap_or(180.0), f64::from(nat_h).max(24.0))
            }
            kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 1.0),
            // The row box's own natural size, asked with its size request lifted: GTK never
            // measures a widget smaller than its request, which Day set to the last frame, so a
            // row that lost its subtitle would otherwise keep the taller height.
            kinds::LIST_ITEM => {
                let (rw, rh) = h.size_request();
                h.set_size_request(-1, -1);
                let width = p.width.map_or(-1, |w| w.round() as i32);
                let (_, nat_w, _, _) = h.measure(gtk4::Orientation::Horizontal, -1);
                let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, width);
                h.set_size_request(rw, rh);
                Size::new(
                    p.width.unwrap_or(f64::from(nat_w)),
                    f64::from(nat_h).max(44.0),
                )
            }
            // A canvas has no intrinsic size: accept the offered axes, just as the other
            // backends do. GtkDrawingArea's natural size is zero; using it here collapses
            // `.grow(Dimension::Width).aspect_ratio(...)` to zero height despite the square proposal.
            #[expect(
                clippy::match_same_arms,
                reason = "the canvas and the lists fill the offer for different, separately documented reasons"
            )]
            kinds::CANVAS => Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0)),
            // The recycling list fills the space it is offered (its scroll owns overflow).
            kinds::LIST | kinds::TREE => Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0)),
            kinds::PROGRESS => {
                if let Some(side) = crate::controls::spinner::side(h) {
                    Size::new(side, side)
                } else {
                    let (_, nat_h, _, _) = h.measure(gtk4::Orientation::Vertical, -1);
                    Size::new(p.width.unwrap_or(180.0), f64::from(nat_h).max(6.0))
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
                Size::new(f64::from(nat_w), f64::from(nat_h))
            }
        }
    }

    fn set_opacity(&mut self, h: &Handle, opacity: f64, anim: Option<&AnimSpec>) {
        // Stepped on the frame clock from the opacity shown (motion.rs, docs/tween.md).
        motion::set_opacity(h, opacity, anim);
    }

    fn set_transform(&mut self, h: &Handle, t: Transform, size: Size, anim: Option<&AnimSpec>) {
        // A child's transform lives on its GtkFixed parent (GskTransform), applied about the
        // widget's center so scale/rotation match the other backends; animated, it is stepped
        // on the frame clock from the transform shown (motion.rs).
        let Some(parent) = h.parent() else {
            return;
        };
        let Some(fixed) = parent.downcast_ref::<gtk4::Fixed>() else {
            return;
        };
        motion::set_transform(fixed, h, t, size, anim);
    }

    fn read_displayed(&self, h: &Handle) -> day_spec::Displayed {
        // The values motion.rs last wrote (opacity, transform, the frame of a `GtkFixed` child,
        // a surface's fill): what GTK draws, mid-animation included.
        motion::displayed(h)
    }

    fn set_selectable(&mut self, h: &Handle, selectable: bool) -> Option<Handle> {
        // A plain label is a GtkLabel (docs/text.md); the downcast guards a non-label backing.
        if let Some(l) = h.downcast_ref::<gtk4::Label>() {
            l.set_selectable(selectable);
            if selectable {
                l.set_can_target(true);
            }
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
        // GTK shows the cursor of the widget its hit test finds, so a shaped cursor needs a
        // hit-testable view (a limitation: on GTK such a view also stops presses).
        input::take_input(h);
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
        (nat_baseline >= 0).then_some(f64::from(nat_baseline))
    }

    fn set_frame(&mut self, h: &Handle, frame: Rect, anim: Option<&AnimSpec>) {
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
        // A stack or suite host resized (window resize): re-report page sizes for relayout.
        // GTK allocates asynchronously — defer one idle so size/position settle. A split
        // host's panes report themselves as GTK allocates them (`nav_pane_report`), and so do
        // an inspector's; a second report here would only lay the same pages out twice.
        let stacked_nav = NAV_STATE.with(|m| {
            m.borrow()
                .get(&key)
                .is_some_and(|s| !matches!(s.present, NavPresent::Split(_)))
        });
        if let Some(parent) = h.parent()
            && let Some(fixed) = parent.downcast_ref::<gtk4::Fixed>()
        {
            // Placed through the fixed (origin, transform, size), stepped on the frame clock
            // when animated (motion.rs). A stacked nav host reports its pages' size once it
            // has it, so it takes its frame at once.
            motion::set_frame(fixed, h, frame, anim.filter(|_| !stacked_nav));
        } else {
            h.set_size_request(
                frame.size.width.round() as i32,
                frame.size.height.round() as i32,
            );
            surface_resized(key, (frame.size.width, frame.size.height));
        }
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

    /// Minimal scroll so `target` (content space) is visible, clamped to the range (docs/scroll.md
    /// "How a target becomes a scroll"). Animated: an ease-out over the frame clock
    /// (`scroll::glide`); a new scroll or the user's own scrolling stops it. Instant: at once,
    /// and again on the next frame once GTK has laid out a content size Day just set.
    fn scroll_to(&mut self, h: &Handle, target: Rect, animated: bool) {
        let Some(sw) = h.downcast_ref::<gtk4::ScrolledWindow>() else {
            return;
        };
        let (lo, hi) = (
            (target.origin.x, target.origin.y),
            (target.max_x(), target.max_y()),
        );
        let goal = move |sw: &gtk4::ScrolledWindow| {
            let (vadj, hadj) = (sw.vadjustment(), sw.hadjustment());
            (
                scroll::reveal(vadj.value(), vadj.page_size(), lo.1, hi.1),
                scroll::reveal(hadj.value(), hadj.page_size(), lo.0, hi.0),
            )
        };
        if !animated {
            let (v, x) = goal(sw);
            sw.vadjustment().set_value(v);
            sw.hadjustment().set_value(x);
        }
        // The content size Day last set (`set_scroll_content`): until GTK's range covers it,
        // an edge target would land on the old extent. A few frames at most.
        let content = content_of(h);
        let fixed: std::cell::Cell<Option<(f64, f64)>> = std::cell::Cell::new(None);
        let waited = std::cell::Cell::new(0_u32);
        scroll::glide(
            sw,
            animated,
            move |sw| {
                let covers = |adj: &gtk4::Adjustment, want: i32| {
                    want <= 0 || adj.upper() + 0.5 >= f64::from(want)
                };
                if !(covers(&sw.vadjustment(), content.height_request())
                    && covers(&sw.hadjustment(), content.width_request()))
                    && waited.replace(waited.get() + 1) < 10
                {
                    return scroll::Aim::Wait;
                }
                // The minimal scroll from where the view was when the glide began.
                let (v, x) = fixed.get().unwrap_or_else(|| {
                    let at = goal(sw);
                    fixed.set(Some(at));
                    at
                });
                scroll::Aim::At {
                    v: Some(v),
                    h: Some(x),
                    exact: true,
                }
            },
            |_| {},
        );
    }

    fn scroll_offset(&mut self, h: &Handle) -> day_spec::Point {
        list_scroller(h).map_or(day_spec::Point::ZERO, |sw| scroll::offset_of(&sw))
    }

    fn list_scroll(&mut self, h: &Handle, to: day_spec::ListScroll) -> bool {
        let Some(e) = list_entry(h) else {
            return false;
        };
        e.scroll_rows(to.row, to.align, to.animated);
        true
    }

    fn set_focusable(&mut self, h: &Handle, node: NodeId, focusable: bool) {
        h.set_focusable(focusable);
        h.set_can_focus(focusable);
        if !focusable {
            return;
        }
        // The focus contract (docs/focus.md): a focusable view takes pointer input, and a
        // canvas already carries the rest from its realize.
        input::take_input(h);
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
        // Keys with their phases, the full route for `.keyboard()`, and the press that
        // focuses it before its recognizers hear that press.
        input::wire_keys(h, node);
        input::focus_on_press(h);
        focus_ring_css();
        h.add_css_class("day-focusable");
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
        // Idempotent per (widget, kind); every recognizer makes the widget hit-testable.
        if !input::note(h, node, kind) {
            return;
        }
        input::take_input(h);
        match kind {
            // Day's own recognizers on GTK's gestures, in the widget's one gesture group
            // (input.rs): tap and double tap, long press, press, touches and drag.
            GestureKind::Tap
            | GestureKind::DoubleTap
            | GestureKind::LongPress
            | GestureKind::Press
            | GestureKind::Touches
            | GestureKind::Drag => input::attach(h, kind),
            GestureKind::Pinch => {
                // GtkGestureZoom's scale-changed is CUMULATIVE since the gesture began —
                // exactly Event::Pinch's contract.
                let zoom = gtk4::GestureZoom::new();
                let center = |g: &gtk4::GestureZoom| {
                    g.bounding_box_center()
                        .map_or(Point::ZERO, |(x, y)| Point::new(x, y))
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
                        );
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
                        );
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
                        );
                    });
                });
                input::add_gesture(h, &zoom);
            }
            GestureKind::Rotate => {
                // GtkGestureRotate (two touches, or a touchpad's pinch-rotate) measures in
                // y-down widget space, so its angle grows clockwise as Event::Rotate's does.
                // Its `angle_delta` is taken modulo a full turn, though, so a small turn the
                // other way reads as nearly 2π: the total is unwrapped here step by step,
                // which also keeps counting past half a circle.
                let rotate = gtk4::GestureRotate::new();
                // (last raw delta, total since Began)
                let turn = Rc::new(std::cell::Cell::new((0.0f64, 0.0f64)));
                let center = |g: &gtk4::GestureRotate| {
                    g.bounding_box_center()
                        .map_or(Point::ZERO, |(x, y)| Point::new(x, y))
                };
                rotate.connect_begin({
                    let turn = turn.clone();
                    move |g, _| {
                        turn.set((0.0, 0.0));
                        ffi_guard::contain((), || {
                            emit(
                                node,
                                Event::Rotate {
                                    phase: DragPhase::Began,
                                    rotation: 0.0,
                                    location: center(g),
                                },
                            );
                        });
                    }
                });
                rotate.connect_angle_changed({
                    let turn = turn.clone();
                    move |g, _angle, delta| {
                        let (last, total) = turn.get();
                        let tau = std::f64::consts::TAU;
                        let mut step = delta - last;
                        step -= (step / tau).round() * tau;
                        let total = total + step;
                        turn.set((delta, total));
                        ffi_guard::contain((), || {
                            emit(
                                node,
                                Event::Rotate {
                                    phase: DragPhase::Changed,
                                    rotation: total,
                                    location: center(g),
                                },
                            );
                        });
                    }
                });
                rotate.connect_end(move |g, _| {
                    let (_, total) = turn.get();
                    ffi_guard::contain((), || {
                        emit(
                            node,
                            Event::Rotate {
                                phase: DragPhase::Ended,
                                rotation: total,
                                location: center(g),
                            },
                        );
                    });
                });
                input::add_gesture(h, &rotate);
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
                        );
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
                        );
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
        }
    }

    fn gesture_support(&self, kind: day_spec::GestureKind) -> Support {
        use day_spec::GestureKind as K;
        match kind {
            // GDK reports touch sequences where the windowing system has a touch screen
            // (Wayland, X11); macOS has none, and its trackpad is a pointer, so the single
            // contact day-core derives from the drag is the honest answer there.
            K::Touches => {
                if cfg!(target_os = "macos") {
                    Support::Unsupported
                } else {
                    Support::Native
                }
            }
            // GtkGestureClick n_press, GtkGestureLongPress, a never-claiming GtkGestureDrag
            // observer (input.rs), and GTK's own recognizers for the rest.
            K::Tap
            | K::DoubleTap
            | K::LongPress
            | K::Press
            | K::Drag
            | K::Pan
            | K::Pinch
            | K::Rotate
            | K::Hover => Support::Native,
        }
    }

    fn set_hit_slop(&mut self, h: &Handle, _node: NodeId, slop: day_spec::Insets) {
        input::set_slop(h, slop);
    }

    fn set_drag_axis(&mut self, h: &Handle, _node: NodeId, axis: Option<day_spec::DragAxis>) {
        input::set_axis(h, axis);
    }

    fn set_subtree_enabled(&mut self, h: &Handle, _node: NodeId, enabled: bool) {
        // Sensitivity cascades: every control under the view dims, takes no pointer input,
        // leaves the focus order (GTK moves focus out of an insensitive subtree) and reads as
        // unavailable to assistive technology. A view under two disabled groups stays
        // insensitive until both are enabled, since an insensitive ancestor wins.
        h.set_sensitive(enabled);
    }

    fn set_drag_source(&mut self, h: &Handle, source: day_spec::transfer::Source) {
        input::take_input(h);
        transfer::source(h, source);
    }
    fn set_drop_target(&mut self, h: &Handle, target: day_spec::transfer::Target) {
        input::take_input(h);
        transfer::target(h, target);
    }
    fn set_context_menu_fn(&mut self, h: &Handle, _node: NodeId, f: day_spec::ContextMenuFn) {
        let key = widget_key(h);
        CTX_MENU_FNS.with(|m| m.borrow_mut().insert(key, f));
        let already = CTX_MENU_WIRED.with(|s| !s.borrow_mut().insert(key));
        if already {
            return;
        }
        input::take_input(h);
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
            if let Some(b) = button {
                let click = gtk4::GestureClick::new();
                click.set_button(b);
                click.connect_pressed(move |_, _n, x, y| show(x, y));
                w.add_controller(click);
            } else {
                let long = gtk4::GestureLongPress::new();
                long.connect_pressed(move |_, x, y| show(x, y));
                w.add_controller(long);
            }
        };
        attach(h, Some(3));
        attach(h, None);
    }

    fn set_button_menu(&mut self, h: &Handle, _node: NodeId, f: day_spec::ContextMenuFn) {
        // Consulted by the button's `clicked` handler (realize), which covers the pointer,
        // Space/Enter and the accessibility activate alike.
        if h.downcast_ref::<gtk4::Button>().is_some() {
            BUTTON_MENUS.with(|t| t.insert(widget_key(h), f));
            // Announce it as a menu button: assistive technology says it opens a menu.
            h.update_property(&[gtk4::accessible::Property::HasPopup(true)]);
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
        input::take_input(h);
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
        Gtk::edit_toolbar(h, ops)
    }

    fn update_toolbar(&mut self, h: &Handle, patch: &day_spec::ToolbarPatch) {
        Gtk::patch_toolbar(h, patch);
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
            .and_then(gtk4::prelude::WidgetExt::root)
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
        let mut cur = self
            .window_fixed
            .as_ref()
            .and_then(gtk4::prelude::WidgetExt::parent);
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
        // A list that reports its whole viewport does so once its first rows are laid out
        // (`ListEntry::after_paint`).
        if source.viewport.is_none()
            && let Some(report) = &source.first_visible
        {
            report(0);
        }
        if let Some(e) = list_entry(host) {
            *e.source.borrow_mut() = Some(source);
            e.viewport_due.set(true);
            // Deferred off this with_tree borrow: a model change binds cells synchronously
            // (`ListEntry::queue_rows`), which would otherwise re-enter with_tree via bind_row.
            e.queue_rows(None);
        }
    }

    fn adopt(&mut self, raw: RawHandle) -> Handle {
        // A recycling GtkListView cell (a GtkFixed) — Day fills/rebinds its row content in place.
        // SAFETY: `raw` is a live GtkWidget (the list's cell, which the list keeps referenced);
        // `from_glib_none` takes a reference of its own for the returned handle.
        unsafe { gtk4::glib::translate::from_glib_none(raw.cast::<gtk4::ffi::GtkWidget>()) }
    }

    fn set_tooltip(&mut self, h: &Handle, text: &str) {
        h.set_tooltip_text((!text.is_empty()).then_some(text));
    }

    fn open_popover(&mut self, anchor: &Handle, content: &Handle, size: Size, id: NodeId) -> bool {
        let popup = gtk4::Popover::new();
        popup.set_autohide(true);
        popup.set_position(gtk4::PositionType::Bottom);
        content.set_size_request(size.width.ceil() as i32, size.height.ceil() as i32);
        popup.set_child(Some(content));
        popup.set_parent(anchor);
        let handler = popup.connect_closed(move |_| emit(id, Event::PopoverDismissed));
        // SAFETY: this private key is written and taken only here and in close_popover.
        unsafe {
            popup.set_data("day-popover-closed", handler);
        }
        popup.popup();
        true
    }

    fn resize_popover(&mut self, content: &Handle, size: Size) {
        content.set_size_request(size.width.ceil() as i32, size.height.ceil() as i32);
    }
    fn close_popover(&mut self, content: &Handle) {
        if let Some(popup) = content
            .ancestor(gtk4::Popover::static_type())
            .and_downcast::<gtk4::Popover>()
        {
            if let Some(handler) =
                // SAFETY: open_popover stores exactly one closed signal handler under this key,
                // as a `SignalHandlerId`, the type taken here.
                unsafe {
                    popup.steal_data::<gtk4::glib::SignalHandlerId>("day-popover-closed")
                }
            {
                popup.disconnect(handler);
            }
            popup.popdown();
            popup.set_child(gtk4::Widget::NONE);
            popup.unparent();
        }
    }

    fn show_toast(&mut self, host: &Handle, text: &str) {
        let toast = adw::Toast::new(&gtk4::glib::markup_escape_text(text));
        if let Some(dialog) = dialog_host_for(host)
            .and_then(|h| h.dialog.borrow().clone())
            .and_downcast::<adw::PreferencesDialog>()
        {
            dialog.add_toast(toast);
        } else if let Some(overlay) = host
            .ancestor(adw::ToastOverlay::static_type())
            .and_downcast::<adw::ToastOverlay>()
        {
            overlay.add_toast(toast);
        }
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
    /// The default `PangoCairo` font map — the one `create_layout` draws canvas text from — listed
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
                    has_alpha: pixbuf
                        .as_ref()
                        .is_none_or(gtk4::gdk_pixbuf::Pixbuf::has_alpha),
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
                .and_then(gtk4::prelude::WidgetExt::root)
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
                    .map(gtk4::glib::object::Cast::upcast::<gtk4::Widget>)
            })
            .or_else(|| host.cloned())
            .or_else(|| self.window_fixed.as_ref().map(|w| w.clone().upcast()))
            .ok_or("no window to capture")?;
        let Some(clock) = widget.frame_clock() else {
            return Ok(day_spec::capture::Readiness::Pending);
        };
        thread_local! {
            static FENCES: std::cell::RefCell<std::collections::HashMap<usize,
                (gtk4::glib::WeakRef<gtk4::Widget>, day_spec::capture::Fence)>> = RefCell::default();
        }
        let fence = FENCES.with(|f| {
            let mut f = f.borrow_mut();
            f.retain(|_, (w, _)| w.upgrade().is_some());
            f.entry(widget.as_ptr() as usize)
                .or_insert_with(|| (widget.downgrade(), day_spec::capture::Fence::default()))
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
                let finish = dialog_finisher(req, &dialog);
                {
                    let finish = finish.clone();
                    dialog.connect_response(None, move |_, resp| {
                        ffi_guard::contain((), || {
                            let result = resp
                                .parse::<i64>()
                                .map_or(PresentResult::Dismissed, PresentResult::Button);
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
                let finish = dialog_finisher(req, &dialog);
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
                let window = file_dialog_window(parent.as_ref());
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
                                emit_file_result(req, res.map(|files| gfile_paths(&files)));
                            });
                        });
                    } else {
                        dialog.open(window.as_ref(), Some(&cancellable), move |res| {
                            ffi_guard::contain((), || {
                                emit_file_result(req, res.map(|f| f.path().into_iter().collect()));
                            });
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
                let window = file_dialog_window(parent.as_ref());
                gtk4::glib::idle_add_local_once(move || {
                    if !claim_file_dialog(req) {
                        return;
                    }
                    dialog.select_folder(window.as_ref(), Some(&cancellable), move |res| {
                        ffi_guard::contain((), || {
                            emit_file_result(req, res.map(|f| f.path().into_iter().collect()));
                        });
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
                let window = file_dialog_window(parent.as_ref());
                // The pieces layer copies the staged bytes to the chosen local path.
                gtk4::glib::idle_add_local_once(move || {
                    if !claim_file_dialog(req) {
                        return;
                    }
                    dialog.save(window.as_ref(), Some(&cancellable), move |res| {
                        ffi_guard::contain((), || {
                            emit_file_result(req, res.map(|f| f.path().into_iter().collect()));
                        });
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
    fn share(&mut self, spec: &day_spec::share::ShareSpec, _anchor: Option<&Self::Handle>) -> bool {
        use objc2::{msg_send, runtime::AnyObject};
        use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString, NSURL};
        let url = spec.url.as_str();
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
        // SAFETY: main thread (`mtm`). `alloc`/`initWithItems:` (an NSArray) and
        // `showRelativeToRect:ofView:preferredEdge:` (NSRect, NSView, NSRectEdge as NSUInteger,
        // returning void) match NSSharingServicePicker's declarations; `self.sharing` keeps the
        // picker alive while it shows.
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
    fn open_url(&mut self, url: &str) {
        // Hand the URI to the desktop's default handler (xdg-open equivalent). Fire and forget;
        // a bad URI just returns an error we ignore.
        let _ =
            gtk4::gio::AppInfo::launch_default_for_uri(url, None::<&gtk4::gio::AppLaunchContext>);
    }
}

/// The `GtkWindow` to anchor a file picker on: the fixed content's toplevel.
fn file_dialog_window(parent: Option<&gtk4::Fixed>) -> Option<gtk4::Window> {
    parent
        .and_then(gtk4::prelude::WidgetExt::root)
        .and_then(|r| r.downcast::<gtk4::Window>().ok())
}

/// Apply a file dialog's extension filters as `GtkFileFilters` (`*.ext` glob patterns).
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
/// the picker itself to an idle (see the `OpenFile` arm); the idle then `claim`s it.
/// Take ownership of a deferred picker on behalf of GTK: `true` to open it, `false` if the
/// request was answered while the idle sat in the queue (`dismiss` removed the entry).
///
/// Answering that fast is ordinary — a dayscript `respond` does it, and so does any app that
/// resolves its own presentation — and the picker must NOT open then: Day considers the request
/// closed, so a window appearing after it is a ghost whose result belongs to nothing. GTK gives
/// a second reason to skip it. Opening with an already-cancelled `GCancellable` leaves the
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

/// Turn a `GtkFileDialog` result into a `PresentResult` and enqueue it.
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

/// The local paths of a `GtkFileDialog.open_multiple` answer: a `GListModel` of `GFiles`, in the
/// picker's order. A file with no local path (a remote GVFS mount) is skipped.
fn gfile_paths(files: &gtk4::gio::ListModel) -> Vec<std::path::PathBuf> {
    (0..files.n_items())
        .filter_map(|i| files.item(i))
        .filter_map(|o| o.downcast::<gtk4::gio::File>().ok())
        .filter_map(|f| f.path())
        .collect()
}

/// A live modal's resolver: emits the first result only, then closes the `AdwDialog` (whose
/// close-response re-enters the finisher guarded, and no-ops).
struct DialogHandle {
    finish: Rc<dyn Fn(day_spec::present::PresentResult)>,
}

fn dialog_finisher(
    req: u64,
    dialog: &adw::AlertDialog,
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
    if let Some(node) = target {
        emit(node, Event::WindowResized(size));
    } else {
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
    use gtk4::gdk::prelude::PaintableExt;
    #[cfg(target_os = "macos")]
    if let Ok(png) = snapshot_macos::snapshot(widget) {
        return Ok(png);
    }
    let w = f64::from(widget.width());
    let h = f64::from(widget.height());
    if w <= 0.0 || h <= 0.0 {
        return Err("zero-size window".into());
    }
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

/// Build one Day window: `AdwApplicationWindow` + ToolbarView/HeaderBar chrome + the filling
/// `DayCell` wrapper + the `GtkFixed` content (docs/windows.md; see the wrapper comments below —
/// factored so `open_window` builds identical chrome).
/// Wires the resize notifies to `target` (`None` = primary).
fn build_day_window(
    app: &adw::Application,
    options: &WindowOptions,
    target: Option<NodeId>,
) -> (adw::ApplicationWindow, gtk4::Fixed, adw::HeaderBar) {
    let window = adw::ApplicationWindow::new(app);
    window.set_title(Some(&options.title));
    // The input kind behind the pointer (docs/gestures.md "Input kind").
    input::watch_pointer_kind(&window);
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
    let overlay = adw::ToastOverlay::new();
    overlay.set_child(Some(&wrapper));
    toolbar.set_content(Some(&overlay));
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
                report_content_size(Size::new(f64::from(width), f64::from(height)), target);
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
        });
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
                });
            });
        });
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
        });
    });
}

/// Recompute "is any Day window active" on an idle (letting a focus handoff between two
/// Day windows settle) and emit the app-level lifecycle transition only on a real change.
fn note_activation_changed(app: &adw::Application) {
    let app = app.clone();
    gtk4::glib::idle_add_local_once(move || {
        ffi_guard::contain((), || {
            let any = app
                .windows()
                .iter()
                .any(gtk4::prelude::GtkWindowExt::is_active);
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
#[must_use]
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
            // Day's own animated changes (motion.rs) do not read the setting, and
            // `reduce_motion` keeps answering the user's.
            let scripted = std::env::var_os("DAY_SCRIPT").is_some_and(|s| !s.is_empty());
            if (day_core::testing::fast_animations() || scripted)
                && let Some(settings) = gtk4::Settings::default()
            {
                mute_widget_transitions(&settings);
            }

            // Follow the SYSTEM appearance while running: libadwaita's StyleManager flips
            // `dark` on desktop theme switches — refresh day-core's reactive dark-mode
            // signal so palette closures recolor live.
            // libadwaita's stylesheet for the new appearance is loaded after this notify, so
            // the refresh reads the palette and re-applies inks on the next idle
            // (`refresh_appearance`). The high-contrast stylesheet and the system accent
            // (libadwaita 1.6+; the notify never fires on older ones) change the named
            // colors too.
            let manager = adw::StyleManager::default();
            manager.connect_dark_notify(|_| queue_appearance_refresh());
            for property in ["high-contrast", "accent-color"] {
                manager.connect_notify_local(Some(property), |_, _| queue_appearance_refresh());
            }
            // Likewise the reduce-motion setting: the desktop's "Animations" switch lands on
            // GtkSettings as `gtk-enable-animations`, and its notify arrives on the main
            // thread, which is Day's — so the re-read happens in place (docs/accessibility.md).
            if let Some(settings) = gtk4::Settings::default() {
                settings.connect_gtk_enable_animations_notify(|s| {
                    ffi_guard::contain((), || {
                        if MUTING.with(std::cell::Cell::get) {
                            return;
                        }
                        // The desktop changed the setting under a muted run: that value is
                        // the user's, and GTK's transitions stay off.
                        if USER_ANIMATIONS.with(std::cell::Cell::get).is_some() {
                            USER_ANIMATIONS.with(|u| u.set(Some(s.is_gtk_enable_animations())));
                            mute_widget_transitions(s);
                        }
                        day_core::note_motion_changed();
                    });
                });
                // And the text size (docs/text.md "Text scale"): GNOME's text-scaling-factor
                // arrives as `gtk-xft-dpi`. GTK gives its widgets the new resolution as their
                // styles revalidate, so Day's labels re-derive their spacing and line height
                // and everything re-measures once that has happened (an idle runs after the
                // frame clock's style and layout phases).
                settings.connect_gtk_xft_dpi_notify(|_| {
                    gtk4::glib::idle_add_local_once(|| {
                        ffi_guard::contain((), || {
                            restyle_all_labels();
                            day_core::note_text_size_changed();
                        });
                    });
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
                        });
                    });
                }
                check_bundled_fonts(&window);
                apply_app_icon(&window);
                window.add_controller(sheet_escape());
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

    #[test]
    fn spacing_and_line_height_are_multiples_of_the_drawn_font_size() {
        use day_spec::props::LabelParagraph;
        let p = |letter_spacing, line_height| LabelParagraph {
            letter_spacing,
            line_height,
            ..LabelParagraph::default()
        };
        // 20 px font: half an em is 10 px, twice the size is 40 px, in 1024ths.
        assert_eq!(
            paragraph_units(20.0, &p(0.5, 2.0)),
            (Some(10 * 1024), Some(40 * 1024))
        );
        // A 20-point font at 96 dpi is drawn 26.67 px tall: the units follow the drawn size.
        assert_eq!(
            paragraph_units(20.0 * 96.0 / 72.0, &p(-0.05, 1.5)),
            (Some(-1365), Some(40 * 1024))
        );
        // Zero, nonpositive and nonfinite values keep the font's own spacing and height.
        assert_eq!(paragraph_units(20.0, &p(0.0, 0.0)), (None, None));
        assert_eq!(paragraph_units(20.0, &p(f64::NAN, -1.0)), (None, None));
        assert_eq!(
            paragraph_units(20.0, &p(f64::INFINITY, f64::INFINITY)),
            (None, None)
        );
        // Markup reads a `line_height` below 1024 as a factor: the absolute height floors at
        // one pixel.
        assert_eq!(paragraph_units(1.0, &p(0.0, 0.1)).1, Some(1024));
    }

    #[test]
    fn selection_offsets_convert_from_characters_to_bytes() {
        let text = "Grüße, 東京 ok";
        assert_eq!(char_to_byte(text, 0), 0);
        assert_eq!(char_to_byte(text, 3), 4); // after "Grü": ü is two bytes
        assert_eq!(&text[char_to_byte(text, 2)..char_to_byte(text, 5)], "üße");
        assert_eq!(&text[char_to_byte(text, 7)..char_to_byte(text, 9)], "東京");
        assert_eq!(char_to_byte(text, 12), text.len());
        assert_eq!(char_to_byte(text, 99), text.len());
    }

    #[test]
    fn font_features_spell_every_tag_the_descriptor_asks_for() {
        let mut spec = day_spec::FontSpec::default();
        assert_eq!(font_feature_string(spec), "");
        spec.small_caps = true;
        assert_eq!(font_feature_string(spec), "smcp 1");
        spec.tabular = true;
        spec.oldstyle_figures = true;
        assert_eq!(font_feature_string(spec), "tnum 1, smcp 1, onum 1");
    }

    #[test]
    fn truncation_modes_map_to_their_pango_ellipsis() {
        use day_spec::props::Truncation;
        use gtk4::pango::EllipsizeMode;
        assert_eq!(ellipsize_mode(Truncation::Tail), EllipsizeMode::End);
        assert_eq!(ellipsize_mode(Truncation::Head), EllipsizeMode::Start);
        assert_eq!(ellipsize_mode(Truncation::Middle), EllipsizeMode::Middle);
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

#[cfg(test)]
mod surface_canvas_tests {
    use super::*;
    use day_spec::{BlendMode, Color, Paint, Rect, Shadow, Shape};

    /// Paint `ops` through the canvas painter onto a clear image surface of `w` x `h` points
    /// at `scale`, answering its premultiplied `0xAARRGGBB` pixels and the pixel width.
    fn painted(ops: &[DrawOp], w: i32, h: i32, scale: i32) -> (Vec<u32>, usize) {
        use gtk4::cairo;
        let mut surface =
            cairo::ImageSurface::create(cairo::Format::ARgb32, w * scale, h * scale).unwrap();
        {
            let cr = cairo::Context::new(&surface).unwrap();
            cr.scale(f64::from(scale), f64::from(scale));
            let (nums, texts) = day_spec::encode_ops(ops);
            cairo_draw(&cr, &nums, &texts, 1.0);
            cr.status().unwrap();
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let width = (w * scale) as usize;
        let data = surface.data().unwrap();
        let mut out = Vec::with_capacity(width * (h * scale) as usize);
        for row in data.chunks_exact(stride) {
            for px in row[..width * 4].as_chunks::<4>().0 {
                out.push(u32::from_ne_bytes(*px));
            }
        }
        (out, width)
    }

    fn fill(rect: Rect, color: u32) -> DrawOp {
        DrawOp::Fill(Shape::Rect(rect), Paint::Solid(Color::hex(color).into()))
    }

    #[test]
    fn conic_wedges_cover_one_turn_in_order() {
        let stops = [(0.25, 0xffff_0000), (0.75, 0xff00_00ff)];
        let wedges = conic_wedges(90.0, &stops);
        assert!((wedges[0].from - 90.0).abs() < 1e-9);
        assert!((wedges[wedges.len() - 1].to - 450.0).abs() < 1e-9);
        for pair in wedges.windows(2) {
            assert!((pair[0].to - pair[1].from).abs() < 1e-9);
        }
        // Flat before the first stop and after the last, in wide wedges.
        assert_eq!((wedges[0].start, wedges[0].end), (0xffff_0000, 0xffff_0000));
        assert!((wedges[0].to - wedges[0].from - 30.0).abs() < 1e-9);
        let last = wedges[wedges.len() - 1];
        assert_eq!((last.start, last.end), (0xff00_00ff, 0xff00_00ff));
        // Between them the color changes in steps no wider than the limit, without a jump.
        for w in wedges.iter().filter(|w| w.start != w.end) {
            assert!(w.to - w.from <= CONIC_STEP + 1e-9);
        }
        for pair in wedges.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
    }

    #[test]
    fn conic_wedges_keep_a_hard_edge_and_the_shared_arithmetic() {
        let stops = [
            (0.0, 0xffff_0000),
            (0.5, 0xffff_0000),
            (0.5, 0xff00_ff00),
            (1.0, 0xff00_00ff),
        ];
        let wedges = conic_wedges(0.0, &stops);
        let edge = wedges
            .windows(2)
            .find(|p| p[0].end != p[1].start)
            .expect("one hard edge");
        assert!((edge[0].to - 180.0).abs() < 1e-9);
        assert_eq!((edge[0].end, edge[1].start), (0xffff_0000, 0xff00_ff00));
        // A wedge's ends are the colors `ConicGradient::color_at` gives for its angles.
        let spec = day_spec::ConicGradient::centered(
            day_spec::Angle::deg(0.0),
            stops
                .iter()
                .map(|(o, c)| (*o, Color::hex(*c & 0x00ff_ffff)))
                .collect::<Vec<_>>(),
        );
        let mid = wedges
            .iter()
            .find(|w| (w.from - 270.0).abs() < 1e-9)
            .unwrap();
        let want = spec.color_at(day_spec::Angle::deg(270.0));
        let (r, g, b, _) = cairo_rgba(mid.start);
        assert!(
            (r - want.r).abs() < 0.01 && (g - want.g).abs() < 0.01 && (b - want.b).abs() < 0.01
        );
        assert_eq!(conic_wedges(0.0, &[]), []);
    }

    #[test]
    fn conic_gradient_paints_its_quarters_clockwise_from_plus_x() {
        let quarter = |at: f64, c: u32| [(at, Color::hex(c)), (at + 0.25, Color::hex(c))];
        let stops = [
            quarter(0.0, 0x00ff_0000),
            quarter(0.25, 0x0000_ff00),
            quarter(0.5, 0x0000_00ff),
            quarter(0.75, 0x00ff_ff00),
        ]
        .concat();
        let ops = [DrawOp::Fill(
            Shape::Rect(Rect::new(0.0, 0.0, 100.0, 100.0)),
            Paint::Conic(day_spec::ConicGradient::centered(
                day_spec::Angle::deg(0.0),
                stops,
            )),
        )];
        let (px, w) = painted(&ops, 100, 100, 1);
        assert_eq!(px[75 * w + 75], 0xffff_0000);
        assert_eq!(px[75 * w + 25], 0xff00_ff00);
        assert_eq!(px[25 * w + 25], 0xff00_00ff);
        assert_eq!(px[25 * w + 75], 0xffff_ff00);
        // The fan reaches the corners and leaves no clear seam along its spokes.
        assert!(px.iter().all(|p| p >> 24 == 0xff));
    }

    /// `stops` painted over a `w` × `h` rect as the fan that stands in for the mesh where
    /// cairo cannot rasterize one ([`set_conic_fan`]), at `scale` device pixels per point.
    fn fan_painted(w: i32, h: i32, scale: i32, stops: &[(f64, u32)]) -> (Vec<u32>, usize) {
        use gtk4::cairo;
        let mut surface =
            cairo::ImageSurface::create(cairo::Format::ARgb32, w * scale, h * scale).unwrap();
        {
            let cr = cairo::Context::new(&surface).unwrap();
            cr.scale(f64::from(scale), f64::from(scale));
            let (w, h) = (f64::from(w), f64::from(h));
            // The shape is traced first, as the canvas does: the fan must keep it.
            cr.rectangle(0.0, 0.0, w, h);
            set_conic_fan(&cr, w / 2.0, h / 2.0, w.hypot(h) / 2.0 + 2.0, 0.0, stops);
            cr.fill().unwrap();
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let width = (w * scale) as usize;
        let data = surface.data().unwrap();
        let mut out = Vec::with_capacity(width * (h * scale) as usize);
        for row in data.chunks_exact(stride) {
            for px in row[..width * 4].as_chunks::<4>().0 {
                out.push(u32::from_ne_bytes(*px));
            }
        }
        (out, width)
    }

    #[test]
    fn conic_fan_paints_its_quarters_clockwise_without_seams() {
        let quarter = |at: f64, c: u32| [(at, c), (at + 0.25, c)];
        let stops = [
            quarter(0.0, 0xffff_0000),
            quarter(0.25, 0xff00_ff00),
            quarter(0.5, 0xff00_00ff),
            quarter(0.75, 0xffff_ff00),
        ]
        .concat();
        for scale in [1, 2] {
            let (px, w) = fan_painted(100, 100, scale, &stops);
            let at = |x: usize, y: usize| px[y * scale as usize * w + x * scale as usize];
            assert_eq!(at(75, 75), 0xffff_0000, "scale {scale}");
            assert_eq!(at(25, 75), 0xff00_ff00, "scale {scale}");
            assert_eq!(at(25, 25), 0xff00_00ff, "scale {scale}");
            assert_eq!(at(75, 25), 0xffff_ff00, "scale {scale}");
            // Every pixel is one wedge's, opaque: no clear seam along a spoke, none at the
            // center, and the fan reaches the corners.
            assert!(px.iter().all(|p| p >> 24 == 0xff), "scale {scale}");
        }
    }

    #[test]
    fn conic_fan_follows_color_at_around_the_turn() {
        let stops = [(0.0, 0xffff_0000), (1.0, 0xff00_00ff)];
        let (px, w) = fan_painted(100, 100, 1, &stops);
        let spec = day_spec::ConicGradient::centered(
            day_spec::Angle::deg(0.0),
            [(0.0, Color::hex(0xff_0000)), (1.0, Color::hex(0x00_00ff))],
        );
        // A ring of pixels away from the center and from the seam where the turn ends.
        for step in 1..36 {
            let a = f64::from(step) * 10.0;
            let (x, y) = (
                (50.0 + 40.0 * a.to_radians().cos()) as usize,
                (50.0 + 40.0 * a.to_radians().sin()) as usize,
            );
            // The angle of the pixel's center, which is what the fan's color stands for.
            let at = (y as f64 + 0.5 - 50.0)
                .atan2(x as f64 + 0.5 - 50.0)
                .to_degrees();
            let want = spec.color_at(day_spec::Angle::deg(at));
            let p = px[y * w + x];
            let (r, b) = (f64::from((p >> 16) & 0xff), f64::from(p & 0xff));
            assert!(
                (r - want.r * 255.0).abs() <= 3.0 && (b - want.b * 255.0).abs() <= 3.0,
                "{a}°: {p:08x} vs {want:?}"
            );
        }
    }

    #[test]
    fn box_blur_passes_follow_the_css_construction() {
        assert_eq!(box_blur_passes(0.2), [(0, 0); 3]);
        // d = floor(sigma * 1.88 + 0.5): 3 at sigma 1.5 (odd), 4 at sigma 2 (even).
        assert_eq!(box_blur_passes(1.5), [(1, 1); 3]);
        assert_eq!(box_blur_passes(2.0), [(2, 1), (1, 2), (2, 2)]);
        assert_eq!(blur_reach(2.0), 6);
        assert_eq!(blur_reach(0.0), 0);
        assert_eq!(blur_reach(1.0), 3);
    }

    #[test]
    fn blur_spreads_coverage_evenly_and_keeps_its_weight() {
        for sigma in [1.0, 4.0] {
            let (w, h, stride) = (96usize, 96usize, 100usize);
            let mut data = vec![0u8; stride * h];
            for y in 32..64 {
                for x in 32..64 {
                    data[y * stride + x] = 255;
                }
            }
            blur_alpha(&mut data, w, h, stride, sigma);
            let at = |x: usize, y: usize| i32::from(data[y * stride + x]);
            // The middle stays solid at these sizes, the edge is half covered, far is clear.
            assert!(at(48, 48) > 250, "sigma {sigma}");
            assert!((at(32, 48) + at(31, 48) - 255).abs() <= 12, "sigma {sigma}");
            assert_eq!(at(2, 2), 0);
            // Symmetric left to right and top to bottom.
            assert!((at(30, 48) - at(65, 48)).abs() <= 2, "sigma {sigma}");
            assert!((at(48, 30) - at(48, 65)).abs() <= 2, "sigma {sigma}");
            let total: u32 = (0..h)
                .flat_map(|y| (0..w).map(move |x| (x, y)))
                .map(|(x, y)| u32::from(data[y * stride + x]))
                .sum();
            let want = 255 * 32 * 32;
            assert!(total.abs_diff(want) < want / 50, "sigma {sigma}: {total}");
            // The padding bytes past each row's width are never touched.
            assert!((0..h).all(|y| data[y * stride + w..(y + 1) * stride] == [0, 0, 0, 0]));
        }
    }

    #[test]
    fn shadow_keeps_its_offset_under_a_transform_and_ends_with_its_scope() {
        let ops = [
            fill(Rect::new(0.0, 0.0, 200.0, 100.0), 0x00ff_ffff),
            DrawOp::Save,
            DrawOp::Concat(day_spec::Affine::scale(2.0, 2.0)),
            DrawOp::Shadow(Some(
                Shadow::new(Color::hex(0x0000_0000), 2.0).offset(0.0, 20.0),
            )),
            fill(Rect::new(15.0, 10.0, 20.0, 15.0), 0x0000_00ff),
            DrawOp::Restore,
            fill(Rect::new(130.0, 20.0, 40.0, 30.0), 0x0000_00ff),
        ];
        for scale in [1, 2] {
            let (px, w) = painted(&ops, 200, 100, scale);
            let s = scale as usize;
            let at = |x: usize, y: usize| px[y * s * w + x * s];
            assert_eq!(at(50, 35), 0xff00_00ff, "the box, at {scale}x");
            assert_eq!(at(50, 60), 0xff00_0000, "its shadow 20 points down");
            assert_eq!(at(50, 85), 0xffff_ffff, "not 40 points down");
            assert_eq!(at(150, 35), 0xff00_00ff, "the second box");
            assert_eq!(at(150, 60), 0xffff_ffff, "casts nothing");
            // The edge is soft: one point of sigma leaves a ramp around y = 70.
            let edge = at(50, 70) & 0xff;
            assert!(edge > 0x20 && edge < 0xe0, "edge {edge:#x} at {scale}x");
        }
    }

    #[test]
    fn a_clear_shadow_color_turns_the_shadow_off() {
        let ops = [
            fill(Rect::new(0.0, 0.0, 60.0, 60.0), 0x00ff_ffff),
            DrawOp::Shadow(Some(
                Shadow::new(Color::hex(0x0000_0000), 0.0).offset(0.0, 20.0),
            )),
            DrawOp::Shadow(None),
            fill(Rect::new(20.0, 10.0, 20.0, 10.0), 0x0000_00ff),
        ];
        let (px, w) = painted(&ops, 60, 60, 1);
        assert_eq!(px[35 * w + 30], 0xffff_ffff);
    }

    #[test]
    fn blend_modes_combine_with_the_canvas_and_end_with_their_scope() {
        let band = |i: f64| Rect::new(20.0 * i, 0.0, 20.0, 20.0);
        let ops = [
            fill(Rect::new(0.0, 0.0, 60.0, 20.0), 0x00ff_ffff),
            fill(band(0.0), 0x00ff_ff00),
            DrawOp::Save,
            DrawOp::Blend(BlendMode::Multiply),
            fill(band(0.0), 0x0000_ffff),
            DrawOp::Restore,
            fill(band(1.0), 0x00ff_0000),
            DrawOp::Save,
            DrawOp::Blend(BlendMode::Add),
            fill(band(1.0), 0x0000_00ff),
            DrawOp::Restore,
            fill(band(2.0), 0x00ff_ff00),
            fill(band(2.0), 0x0000_00ff),
        ];
        let (px, w) = painted(&ops, 60, 20, 1);
        assert_eq!(px[10 * w + 10], 0xff00_ff00);
        assert_eq!(px[10 * w + 30], 0xffff_00ff);
        assert_eq!(px[10 * w + 50], 0xff00_00ff);
    }

    #[test]
    fn erase_leaves_the_canvas_clear() {
        let ops = [
            fill(Rect::new(0.0, 0.0, 40.0, 40.0), 0x00ff_ffff),
            DrawOp::Save,
            DrawOp::Blend(BlendMode::Erase),
            fill(Rect::new(10.0, 10.0, 20.0, 20.0), 0x0000_ff00),
            DrawOp::Restore,
            // An unmatched restore must not unwind the painter's own group.
            DrawOp::Restore,
        ];
        let (px, w) = painted(&ops, 40, 40, 1);
        assert_eq!(px[20 * w + 20], 0x0000_0000);
        assert_eq!(px[5 * w + 5], 0xffff_ffff);
        assert_eq!(blend_operator(99), gtk4::cairo::Operator::Over);
    }

    #[test]
    fn pdf_export_takes_shadows_blends_and_conic_gradients() {
        let size = Size::new(120.0, 80.0);
        let ops = vec![
            DrawOp::Fill(
                Shape::Ellipse(Rect::new(10.0, 10.0, 60.0, 60.0)),
                Paint::Conic(day_spec::ConicGradient::centered(
                    day_spec::Angle::deg(0.0),
                    vec![
                        (0.0, Color::hex(0x00ff_0000)),
                        (1.0, Color::hex(0x0000_00ff)),
                    ],
                )),
            ),
            DrawOp::Save,
            DrawOp::Shadow(Some(Shadow::new(Color::BLACK, 6.0).offset(2.0, 3.0))),
            DrawOp::Blend(BlendMode::Multiply),
            fill(Rect::new(50.0, 20.0, 40.0, 30.0), 0x0000_ff00),
            DrawOp::Restore,
        ];
        let pdf = render_pdf_bytes(
            &[day_spec::PdfPage { size, ops }],
            &day_spec::PdfInfo::default(),
        )
        .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn surface_css_names_each_corner_and_draws_border_and_shadow_as_box_shadows() {
        let style = SurfaceStyle {
            fill: Some(Color::rgb(0.0, 0.0, 1.0).into()),
            corner_radius: 30.0,
            corners: day_spec::Corners::TOP,
            border: Some(day_spec::Border::new(Color::rgb(1.0, 0.0, 0.0), 8.0)),
            shadow: Some(Shadow::new(Color::rgba(0.0, 0.0, 0.0, 0.5), 4.0).offset(0.0, 16.0)),
            ..Default::default()
        };
        // Under the content (a widget that cannot draw over its children): an inset shadow.
        assert_eq!(
            style.css((80.0, 80.0), true),
            "background-color: rgba(0,0,255,1);border-radius: 30px 30px 0px 0px;\
             box-shadow: 0px 16px 4px rgba(0,0,0,0.5), inset 0 0 0 8px rgba(255,0,0,1);"
        );
        // Over the content (a Day container's snapshot draws it): not in the CSS.
        assert_eq!(
            style.css((80.0, 80.0), false),
            "background-color: rgba(0,0,255,1);border-radius: 30px 30px 0px 0px;\
             box-shadow: 0px 16px 4px rgba(0,0,0,0.5);"
        );
        assert!(!SurfaceStyle::default().is_styled());
        assert!(!style.is_dynamic());
        // The plain form is what `background` and `corner_radius` always wrote.
        assert_eq!(
            SurfaceStyle::plain(Some(Color::rgb(1.0, 0.0, 0.0).into()), 12.0, true)
                .css((0.0, 0.0), false),
            "background-color: rgba(255,0,0,1);border-radius: 12px 12px 12px 12px;"
        );
    }

    #[test]
    fn a_role_is_a_named_color_and_a_pair_is_resolved_and_dynamic() {
        use day_spec::{Ink, SystemColor};
        // A role without an alpha of its own is libadwaita's color, by name.
        assert_eq!(
            css_ink(Ink::system(SystemColor::Accent)),
            role_css(SystemColor::Accent)
        );
        assert!(
            ["var(--accent-bg-color)", "@accent_bg_color"].contains(&role_css(SystemColor::Accent))
        );
        assert!(css_ink(Ink::system(SystemColor::SecondaryText)).starts_with("alpha("));
        // An alpha override replaces the role's alpha, which `alpha()` cannot say: resolved.
        assert!(css_ink(SystemColor::Accent.with_alpha(0.2)).starts_with("rgba("));
        let pair = Ink::adaptive(Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0));
        let style = SurfaceStyle::plain(Some(pair), 0.0, false);
        assert!(style.is_dynamic());
        let bordered = SurfaceStyle {
            border: Some(day_spec::Border::new(SystemColor::Separator, 1.0)),
            ..Default::default()
        };
        assert!(bordered.is_dynamic());
    }

    #[test]
    fn radial_and_conic_fills_are_css_gradients_in_unit_space() {
        use day_spec::{Angle, ConicGradient, RadialGradient, UnitPoint};
        let radial = RadialGradient::new(
            UnitPoint::CENTER,
            0.5,
            [
                (0.0, Color::rgb(1.0, 0.0, 0.0)),
                (1.0, Color::rgb(0.0, 0.0, 1.0)),
            ],
        );
        assert_eq!(
            css_radial_gradient(&radial),
            "radial-gradient(ellipse 50.0000% 50.0000% at 50.0000% 50.0000%, \
             rgba(255,0,0,1) 0.0000%, rgba(0,0,255,1) 100.0000%)"
        );
        // Day's zero is +x and CSS's is up: a quarter turn later.
        let conic = ConicGradient::centered(
            Angle::deg(30.0),
            vec![
                (0.0, Color::rgb(1.0, 0.0, 0.0)),
                (1.0, Color::rgb(0.0, 1.0, 0.0)),
            ],
        );
        assert_eq!(
            css_conic_gradient(&conic),
            "conic-gradient(from 120.0000deg at 50.0000% 50.0000%, \
             rgba(255,0,0,1) 0.0000%, rgba(0,255,0,1) 100.0000%)"
        );
    }

    #[test]
    fn css_gradient_lands_its_stops_on_the_unit_points() {
        use day_spec::UnitPoint as U;
        let red_blue = |start, end| {
            day_spec::LinearGradient::new(
                start,
                end,
                vec![
                    (0.0, Color::rgb(1.0, 0.0, 0.0)),
                    (1.0, Color::rgb(0.0, 0.0, 1.0)),
                ],
            )
        };
        // Axis-aligned: the same at any size.
        for size in [(80.0, 80.0), (300.0, 20.0), (0.0, 0.0)] {
            assert_eq!(
                css_linear_gradient(&red_blue(U::TOP, U::BOTTOM), size),
                "linear-gradient(180.0000deg, rgba(255,0,0,1) 0.0000%, rgba(0,0,255,1) 100.0000%)"
            );
            assert_eq!(
                css_linear_gradient(&red_blue(U::LEADING, U::TRAILING), size),
                "linear-gradient(90.0000deg, rgba(255,0,0,1) 0.0000%, rgba(0,0,255,1) 100.0000%)"
            );
        }
        // Corner to corner of a 2:1 box: the line slants with the box, and since it runs
        // exactly corner to corner the stops still sit at the ends of the CSS line.
        assert_eq!(
            css_linear_gradient(
                &red_blue(U::TOP_LEADING, U::BOTTOM_TRAILING),
                (200.0, 100.0)
            ),
            "linear-gradient(116.5651deg, rgba(255,0,0,1) 0.0000%, rgba(0,0,255,1) 100.0000%)"
        );
        // A gradient over the middle half of the height: stops at 25% and 75%.
        assert_eq!(
            css_linear_gradient(
                &red_blue(U::new(0.5, 0.25), U::new(0.5, 0.75)),
                (50.0, 200.0)
            ),
            "linear-gradient(180.0000deg, rgba(255,0,0,1) 25.0000%, rgba(0,0,255,1) 75.0000%)"
        );
    }

    #[test]
    fn tint_recolors_and_keeps_alpha() {
        let words = [0xffff_0000u32, 0x8000_0040, 0x0000_0000];
        let mut data: Vec<u8> = words.iter().flat_map(|w| w.to_ne_bytes()).collect();
        tint_argb32(&mut data, Color::rgb(0.0, 1.0, 0.0));
        let out: Vec<u32> = data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| u32::from_ne_bytes(*p))
            .collect();
        // Premultiplied: half-covered green is half green.
        assert_eq!(out, [0xff00_ff00, 0x8000_8000, 0x0000_0000]);
    }
}
