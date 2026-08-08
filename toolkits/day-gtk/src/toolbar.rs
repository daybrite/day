// ---------------------------------------------------------------------------
// GTK: the window's AdwHeaderBar (docs/toolbars.md). GNOME has no separate toolbar — the header
// bar IS the toolbar, holding the window title in the middle and the app's actions at either
// end, and GTK4 removed GtkToolbar outright. So a day toolbar packs into the AdwHeaderBar the
// window already has: items before the first flexible space go to the start, the rest to the
// end, which is exactly how a GNOME app is laid out.
// ---------------------------------------------------------------------------

use std::cell::RefCell;
use std::collections::HashMap;

use day_spec::{Event, Icon, Symbol, ToolbarItem, ToolbarItemKind, ToolbarPatch, ToolbarValue};
use libadwaita as adw;

// AdwHeaderBar's packing methods and every GTK widget trait this file uses come through the
// libadwaita prelude, which re-exports GTK's.
use adw::prelude::*;

use crate::{Gtk, Handle, emit};

/// The freedesktop icon names a standard symbol can draw as, best first. Symbolic variants,
/// because that is what a header bar wants: they recolour with the theme, in dark mode and on
/// selection. More than one name per symbol because icon themes vary in how complete they are —
/// a bare Adwaita install on a non-Linux host is missing a good number of these.
fn icon_candidates(s: Symbol) -> &'static [&'static str] {
    match s {
        Symbol::Add => &["list-add-symbolic"],
        Symbol::Remove => &["list-remove-symbolic"],
        Symbol::Delete => &["user-trash-symbolic", "edit-delete-symbolic"],
        Symbol::Edit => &["document-edit-symbolic", "edit-symbolic"],
        Symbol::New => &["document-new-symbolic", "list-add-symbolic"],
        Symbol::Open => &["document-open-symbolic", "folder-open-symbolic"],
        Symbol::Save => &["document-save-symbolic"],
        Symbol::Print => &["document-print-symbolic", "printer-symbolic"],
        Symbol::Refresh => &["view-refresh-symbolic"],
        Symbol::Search => &["system-search-symbolic", "edit-find-symbolic"],
        Symbol::Share => &["emblem-shared-symbolic", "send-to-symbolic"],
        Symbol::Settings => &["preferences-system-symbolic", "emblem-system-symbolic"],
        Symbol::Info => &["dialog-information-symbolic", "help-about-symbolic"],
        Symbol::Star => &["starred-symbolic", "non-starred-symbolic", "star-symbolic"],
        Symbol::Bookmark => &["user-bookmarks-symbolic", "bookmark-new-symbolic"],
        Symbol::Back => &["go-previous-symbolic"],
        Symbol::Forward => &["go-next-symbolic"],
        Symbol::Up => &["go-up-symbolic", "pan-up-symbolic"],
        Symbol::Down => &["go-down-symbolic", "pan-down-symbolic"],
        Symbol::Home => &["go-home-symbolic", "user-home-symbolic"],
        Symbol::Sidebar => &["sidebar-show-symbolic", "view-dual-symbolic"],
        Symbol::Filter => &["view-filter-symbolic", "funnel-symbolic"],
        Symbol::Sort => &["view-sort-ascending-symbolic"],
        Symbol::More => &["view-more-symbolic", "open-menu-symbolic"],
        Symbol::Play => &["media-playback-start-symbolic"],
        Symbol::Pause => &["media-playback-pause-symbolic"],
        Symbol::Stop => &["media-playback-stop-symbolic"],
        Symbol::ZoomIn => &["zoom-in-symbolic"],
        Symbol::ZoomOut => &["zoom-out-symbolic"],
        Symbol::Undo => &["edit-undo-symbolic"],
        Symbol::Redo => &["edit-redo-symbolic"],
        Symbol::Copy => &["edit-copy-symbolic"],
        Symbol::Cut => &["edit-cut-symbolic"],
        Symbol::Paste => &["edit-paste-symbolic"],
        Symbol::Mail => &["mail-send-symbolic", "mail-unread-symbolic"],
        Symbol::Folder => &["folder-symbolic"],
        Symbol::Document => &["text-x-generic-symbolic", "document-symbolic"],
        Symbol::Check => &["object-select-symbolic", "emblem-ok-symbolic"],
        Symbol::Close => &["window-close-symbolic"],
        Symbol::Warning => &["dialog-warning-symbolic"],
        // The vocabulary is `#[non_exhaustive]`: an unmapped symbol falls back to the item's
        // label rather than to GTK's broken-image icon.
        _ => &[],
    }
}

/// The first candidate the running icon theme actually has. Setting a name the theme lacks
/// paints GTK's broken-image glyph, which is worse than the item's own text.
fn icon_name(s: Symbol) -> Option<&'static str> {
    let theme = gtk4::gdk::Display::default().map(|d| gtk4::IconTheme::for_display(&d));
    icon_candidates(s)
        .iter()
        .copied()
        .find(|name| theme.as_ref().is_some_and(|t| t.has_icon(name)))
}

/// Apply an item's icon to a button, falling back to its label when there is no icon (a header
/// bar button with neither would be an invisible click target).
fn dress_button(button: &impl IsA<gtk4::Button>, item: &ToolbarItem) {
    let button = button.as_ref();
    let mut dressed = false;
    match &item.icon {
        Some(Icon::Symbol(s)) => {
            if let Some(name) = icon_name(*s) {
                button.set_icon_name(name);
                dressed = true;
            }
        }
        // A bundled image: tint it to the theme foreground like the sidebar icons, so a black
        // template glyph stays visible on a dark-mode toolbar instead of rendering dark-on-dark.
        Some(Icon::Image(name)) => {
            if let Some(img) = crate::tinted_template_icon(name, None) {
                button.set_child(Some(&img));
                dressed = true;
            }
        }
        _ => {}
    }
    // A header-bar button with neither an icon nor a label is an invisible click target.
    if !dressed {
        button.set_label(&item.label);
    }
    // GNOME HIG: header-bar buttons are flat until hovered.
    button.add_css_class("flat");
    button.set_tooltip_text(Some(item.tooltip.as_deref().unwrap_or(&item.label)));
    button.set_sensitive(item.enabled);
}

/// One window's day-installed header-bar widgets, so a re-install can take the old ones out
/// without disturbing anything Adwaita put there (the window controls, the title).
struct WinToolbar {
    header: adw::HeaderBar,
    widgets: HashMap<String, gtk4::Widget>,
    /// Packed order, for removal — a HashMap alone would leak the spacers, which share the
    /// empty id.
    packed: Vec<gtk4::Widget>,
    /// Guards the programmatic search sync: GtkSearchEntry re-emits `search-changed` on
    /// `set_text`, which would echo straight back into the bound signal.
    suppress: std::rc::Rc<std::cell::Cell<bool>>,
}

thread_local! {
    static BARS: RefCell<HashMap<usize, WinToolbar>> = RefCell::new(HashMap::new());
    /// Every Day window's header bar, registered as the window is built — the only way back
    /// from a content handle to the chrome, since AdwToolbarView does not enumerate its bars.
    static HEADERS: RefCell<Vec<(gtk4::Window, adw::HeaderBar)>> = const { RefCell::new(Vec::new()) };
}

/// Remember a window's header bar (called for every window `build_day_window` makes).
pub(crate) fn register_header(window: &impl IsA<gtk4::Window>, header: &adw::HeaderBar) {
    let window = window.as_ref().clone();
    HEADERS.with(|h| h.borrow_mut().push((window, header.clone())));
}

/// The header bar of the window `h` lives in.
fn header_of(h: &Handle) -> Option<(gtk4::Window, adw::HeaderBar)> {
    let root = h.root()?;
    let window = root.downcast::<gtk4::Window>().ok()?;
    HEADERS.with(|hs| {
        hs.borrow()
            .iter()
            .find(|(w, _)| w == &window)
            .map(|(w, hb)| (w.clone(), hb.clone()))
    })
}

impl Gtk {
    /// Install `items` into this window's header bar (docs/toolbars.md).
    pub(crate) fn install_toolbar(&mut self, h: &Handle, items: &[ToolbarItem]) {
        let Some((window, header)) = header_of(h) else {
            return;
        };
        let key = window.as_ptr() as usize;

        // Take out whatever the previous install packed. Anything else in the header bar is
        // Adwaita's own and must be left alone.
        BARS.with(|b| {
            if let Some(old) = b.borrow_mut().remove(&key) {
                for w in old.packed {
                    header.remove(&w);
                }
            }
        });
        if items.is_empty() {
            return;
        }

        let suppress = std::rc::Rc::new(std::cell::Cell::new(false));
        let mut widgets: HashMap<String, gtk4::Widget> = HashMap::new();
        let mut packed: Vec<gtk4::Widget> = Vec::new();

        // Everything up to the first flexible space packs to the start, the rest to the end.
        let split = items
            .iter()
            .position(|i| matches!(i.kind, ToolbarItemKind::FlexibleSpace))
            .unwrap_or(items.len());

        let mut build = |item: &ToolbarItem| -> Option<gtk4::Widget> {
            let w: gtk4::Widget = match &item.kind {
                ToolbarItemKind::Button => {
                    let b = gtk4::Button::new();
                    dress_button(&b, item);
                    let action = item.action;
                    if action != 0 {
                        b.connect_clicked(move |_| {
                            emit(day_spec::WINDOW_NODE, Event::MenuAction(action));
                        });
                    }
                    b.upcast()
                }
                ToolbarItemKind::Toggle { on } => {
                    let b = gtk4::ToggleButton::new();
                    dress_button(&b, item);
                    b.set_active(*on);
                    let action = item.action;
                    if action != 0 {
                        b.connect_toggled(move |t| {
                            emit(
                                day_spec::WINDOW_NODE,
                                Event::ToolbarChanged {
                                    action,
                                    value: ToolbarValue::On(t.is_active()),
                                },
                            );
                        });
                    }
                    b.upcast()
                }
                ToolbarItemKind::SidebarToggle => {
                    // GNOME's own sidebar affordance: the `sidebar-show-symbolic` button that
                    // opens Files' and Text Editor's side pane. The app supplies no action —
                    // the click drives the split host directly (docs/toolbars.md).
                    let b = gtk4::Button::new();
                    dress_button(&b, item);
                    if item.icon.is_none() {
                        b.set_icon_name("sidebar-show-symbolic");
                    }
                    b.set_sensitive(item.enabled);
                    b.connect_clicked(|b| {
                        if !crate::toggle_sidebar() {
                            b.set_sensitive(false); // no sidebar in this window
                        }
                    });
                    b.upcast()
                }
                ToolbarItemKind::Menu { items } => {
                    let b = gtk4::MenuButton::new();
                    let group = gtk4::gio::SimpleActionGroup::new();
                    let model = crate::build_gio_menu(items, &group);
                    // The menu model names its actions `daymenu.aN`; inserting the group on the
                    // button itself shadows the window's menu-bar group rather than replacing it.
                    b.insert_action_group("daymenu", Some(&group));
                    b.set_menu_model(Some(&model));
                    match item.icon.as_ref().and_then(|i| match i {
                        Icon::Symbol(s) => icon_name(*s),
                        Icon::Image(_) => None,
                    }) {
                        Some(name) => b.set_icon_name(name),
                        None => b.set_label(&item.label),
                    }
                    b.add_css_class("flat");
                    b.set_tooltip_text(Some(item.tooltip.as_deref().unwrap_or(&item.label)));
                    b.set_sensitive(item.enabled);
                    b.upcast()
                }
                ToolbarItemKind::Search { text, placeholder } => {
                    let e = gtk4::SearchEntry::new();
                    e.set_text(text);
                    if !placeholder.is_empty() {
                        e.set_placeholder_text(Some(placeholder));
                    }
                    // A header-bar search entry is sized, not stretched: GNOME keeps the title
                    // visible beside it.
                    e.set_max_width_chars(24);
                    e.set_sensitive(item.enabled);
                    let action = item.action;
                    if action != 0 {
                        let suppress = suppress.clone();
                        e.connect_search_changed(move |entry| {
                            if suppress.get() {
                                return;
                            }
                            emit(
                                day_spec::WINDOW_NODE,
                                Event::ToolbarChanged {
                                    action,
                                    value: ToolbarValue::Text(entry.text().to_string()),
                                },
                            );
                        });
                    }
                    e.upcast()
                }
                ToolbarItemKind::Label => {
                    let l = gtk4::Label::new(Some(&item.label));
                    l.add_css_class("dim-label");
                    l.upcast()
                }
                ToolbarItemKind::Separator => {
                    gtk4::Separator::new(gtk4::Orientation::Vertical).upcast()
                }
                ToolbarItemKind::Space => {
                    let b = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
                    b.set_size_request(12, -1);
                    b.upcast()
                }
                // The pack split IS the flexible space; there is no widget for it.
                ToolbarItemKind::FlexibleSpace => return None,
            };
            if !item.id.is_empty() {
                widgets.insert(item.id.clone(), w.clone());
            }
            Some(w)
        };

        for item in &items[..split] {
            if let Some(w) = build(item) {
                header.pack_start(&w);
                packed.push(w);
            }
        }
        // `pack_end` grows right-to-left, so packing the trailing group in reverse is what puts
        // it on screen in the order the app wrote it.
        for item in items[split..].iter().rev() {
            if let Some(w) = build(item) {
                header.pack_end(&w);
                packed.push(w);
            }
        }

        BARS.with(|b| {
            b.borrow_mut().insert(
                key,
                WinToolbar {
                    header,
                    widgets,
                    packed,
                    suppress,
                },
            )
        });
    }

    /// Apply a targeted change to one live item.
    pub(crate) fn patch_toolbar(&mut self, h: &Handle, patch: &ToolbarPatch) {
        let Some((window, _)) = header_of(h) else {
            return;
        };
        let key = window.as_ptr() as usize;
        BARS.with(|b| {
            let b = b.borrow();
            let Some(bar) = b.get(&key) else { return };
            let _ = &bar.header;
            match patch {
                ToolbarPatch::Text { item, text } => {
                    if let Some(w) = bar.widgets.get(item)
                        && let Some(e) = w.downcast_ref::<gtk4::SearchEntry>()
                        && e.text() != text.as_str()
                    {
                        // GtkSearchEntry re-emits `search-changed` on a programmatic set, which
                        // would write the value straight back into the signal.
                        bar.suppress.set(true);
                        e.set_text(text);
                        bar.suppress.set(false);
                    }
                }
                ToolbarPatch::On { item, on } => {
                    if let Some(w) = bar.widgets.get(item)
                        && let Some(t) = w.downcast_ref::<gtk4::ToggleButton>()
                        && t.is_active() != *on
                    {
                        t.set_active(*on);
                    }
                }
                ToolbarPatch::Enabled { item, on } => {
                    if let Some(w) = bar.widgets.get(item) {
                        w.set_sensitive(*on);
                    }
                }
            }
        });
    }
}
