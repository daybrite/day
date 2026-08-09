// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The window-toolbar builder: `toolbar_button`, `toolbar_toggle`, `toolbar_menu`,
//! `toolbar_label`, and the spacers, assembled with [`toolbar`].
//!
//! SEARCH is not here. It is declared on the navigation surface it filters
//! (`Selector::searchable`, docs/search.md) and day-core merges the resulting field into the
//! window's bar, so the platform can move it — into the navigation list on a narrow window —
//! without the app re-declaring anything.
//!
//! A toolbar is window chrome, not a piece: it is not laid out by day and does not live in the
//! tree. It lowers to day_spec's toolkit-neutral [`day_spec::ToolbarItem`] model, which each
//! backend realizes with its platform's real toolbar — `NSToolbar`, `AdwHeaderBar`, `QToolBar`,
//! `CommandBar` (docs/toolbars.md). Where the platform has no toolbar (`Cap::Toolbar` is
//! `Unsupported` — every phone) nothing is drawn and nothing is faked.
//!
//! ```ignore
//! toolbar(vec![
//!     toolbar_button("refresh", tr("refresh")).icon(Symbol::Refresh).action(refresh_all),
//!     toolbar_flexible_space(),
//! ]);
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use day_reactive::{Scope, Signal, bind, bind_seeded};
use day_spec::{Icon, Symbol, ToolbarItem, ToolbarItemKind, ToolbarPatch, ToolbarValue};

use crate::{IntoText, MenuEntry, TextSource};

/// A toolbar item under construction. Build a command with [`toolbar_button`], a two-state
/// button with [`toolbar_toggle`], a pull-down with [`toolbar_menu`], a search field with
/// and the gaps with [`toolbar_space`] / [`toolbar_flexible_space`]. Search is declared on the
/// navigation surface instead (`Selector::searchable`, docs/search.md).
pub struct ToolbarEntry {
    id: String,
    kind: Kind,
    label: Option<TextSource>,
    tooltip: Option<TextSource>,
    icon: Option<Icon>,
    enabled: bool,
    enabled_when: Option<Rc<dyn Fn() -> bool>>,
    action: Option<Rc<dyn Fn()>>,
}

/// The app-side kinds, carrying the live signals the spec model cannot.
enum Kind {
    Button,
    Toggle(Signal<bool>),
    Menu(Vec<MenuEntry>),
    SidebarToggle,
    Label,
    Separator,
    Space,
    FlexibleSpace,
}

fn entry(id: impl Into<String>, kind: Kind) -> ToolbarEntry {
    ToolbarEntry {
        id: id.into(),
        kind,
        label: None,
        tooltip: None,
        icon: None,
        enabled: true,
        enabled_when: None,
        action: None,
    }
}

/// A push button: `toolbar_button("refresh", tr("refresh")).icon(Symbol::Refresh).action(…)`.
pub fn toolbar_button<M>(id: impl Into<String>, label: impl IntoText<M>) -> ToolbarEntry {
    ToolbarEntry {
        label: Some(label.into_text()),
        ..entry(id, Kind::Button)
    }
}

/// A two-state button bound to `on`: the user flipping it writes the signal, and writing the
/// signal restyles the button.
pub fn toolbar_toggle<M>(
    id: impl Into<String>,
    label: impl IntoText<M>,
    on: Signal<bool>,
) -> ToolbarEntry {
    ToolbarEntry {
        label: Some(label.into_text()),
        ..entry(id, Kind::Toggle(on))
    }
}

/// A button that drops a menu, built from the same entries [`crate::app_menu`] takes.
pub fn toolbar_menu<M>(
    id: impl Into<String>,
    label: impl IntoText<M>,
    items: Vec<MenuEntry>,
) -> ToolbarEntry {
    ToolbarEntry {
        label: Some(label.into_text()),
        ..entry(id, Kind::Menu(items))
    }
}

/// Static text in the bar — a status or a caption.
pub fn toolbar_label<M>(id: impl Into<String>, text: impl IntoText<M>) -> ToolbarEntry {
    ToolbarEntry {
        label: Some(text.into_text()),
        ..entry(id, Kind::Label)
    }
}

/// Show/hide the window's sidebar — the leading item of a desktop toolbar in an app built
/// around a `selector(Sidebar)` (Mail, Finder, Files, Explorer).
///
/// Takes no `.action`: the toolkit binds it to the sidebar host in this window and drives that
/// host's own collapse, so the app declares the affordance and each platform supplies its
/// native behaviour and glyph. Place it first, before any [`toolbar_flexible_space`]. In a
/// window with no sidebar it renders disabled rather than vanishing, so the bar keeps its shape
/// as the route changes. docs/toolbars.md.
pub fn toolbar_sidebar_toggle<M>(id: impl Into<String>, label: impl IntoText<M>) -> ToolbarEntry {
    ToolbarEntry {
        label: Some(label.into_text()),
        ..entry(id, Kind::SidebarToggle)
    }
}

/// A divider, where the platform draws one (macOS toolbars have none — AppKit renders it as a
/// fixed gap; docs/toolbars.md).
pub fn toolbar_separator() -> ToolbarEntry {
    entry("", Kind::Separator)
}

/// A fixed gap.
pub fn toolbar_space() -> ToolbarEntry {
    entry("", Kind::Space)
}

/// A gap that absorbs the leftover width. Everything before the first one is packed to the
/// leading edge and everything after it to the trailing edge, which is how each toolkit's own
/// packing (GTK's start/end, XAML's content/commands) is expressed in one ordered list.
pub fn toolbar_flexible_space() -> ToolbarEntry {
    entry("", Kind::FlexibleSpace)
}

impl ToolbarEntry {
    /// Run `f` when the item is chosen. On a toggle or a search field the value binding carries
    /// the change; an action here runs in addition to it.
    pub fn action(mut self, f: impl Fn() + 'static) -> ToolbarEntry {
        self.action = Some(Rc::new(f));
        self
    }

    /// Draw a standard [`Symbol`], using the platform's own icon set — an SF Symbol on macOS, a
    /// freedesktop icon name on GTK and Qt, a Fluent glyph on Windows.
    pub fn icon(mut self, symbol: Symbol) -> ToolbarEntry {
        self.icon = Some(Icon::Symbol(symbol));
        self
    }

    /// Draw a bundled image from `resource/images` — for an icon only this app has. Prefer
    /// [`ToolbarEntry::icon`] for anything standard: one PNG cannot look native on four desktops.
    pub fn image(mut self, name: impl Into<day_spec::ImageName>) -> ToolbarEntry {
        self.icon = Some(Icon::Image(name.into().as_str().to_string()));
        self
    }

    /// Hover help. Defaults to the item's label.
    pub fn tooltip<M>(mut self, t: impl IntoText<M>) -> ToolbarEntry {
        self.tooltip = Some(t.into_text());
        self
    }

    /// Enable or disable the item once, at build.
    pub fn enabled(mut self, on: bool) -> ToolbarEntry {
        self.enabled = on;
        self
    }

    /// Enable the item while `f` reads true, re-evaluated whenever its reactive reads change.
    /// This is the live path: it patches the one item rather than rebuilding the bar, so a
    /// command greying out never disturbs a search field mid-word.
    pub fn enabled_when(mut self, f: impl Fn() -> bool + 'static) -> ToolbarEntry {
        self.enabled_when = Some(Rc::new(f));
        self
    }
}

/// Lower app-side entries to the spec model, registering each item's closures with day-core and
/// wiring the live bindings (toggle state, search text, `enabled_when`).
fn lower(entries: Vec<ToolbarEntry>, window: day_core::RNode) -> Vec<ToolbarItem> {
    entries
        .into_iter()
        .map(|e| {
            let ToolbarEntry {
                id,
                kind,
                label,
                tooltip,
                icon,
                enabled,
                enabled_when,
                action,
            } = e;
            let label = label.map(|t| t.initial()).unwrap_or_default();
            let extra = action;

            // Buttons and menus dispatch through the menu registry; toggles and search fields
            // register a value callback instead (day-core keeps both in one id space).
            let (kind, action_id) = match kind {
                Kind::Button => (
                    ToolbarItemKind::Button,
                    extra
                        .clone()
                        .map(day_core::register_menu_action)
                        .unwrap_or(0),
                ),
                Kind::Menu(items) => (
                    ToolbarItemKind::Menu {
                        items: crate::lower_menu(items),
                    },
                    extra
                        .clone()
                        .map(day_core::register_menu_action)
                        .unwrap_or(0),
                ),
                Kind::Toggle(on) => {
                    let seed = on.get_untracked();
                    let item = id.clone();
                    let extra = extra.clone();
                    let act = day_core::register_toolbar_value(Rc::new(move |v: &ToolbarValue| {
                        if let ToolbarValue::On(next) = v {
                            on.set(*next);
                            if let Some(f) = &extra {
                                f();
                            }
                        }
                    }));
                    // The app's own writes patch the one item back.
                    bind_seeded(
                        seed,
                        move || on.get(),
                        move |v: &bool| {
                            day_core::patch_window_toolbar(
                                window,
                                ToolbarPatch::On {
                                    item: item.clone(),
                                    on: *v,
                                },
                            );
                        },
                    );
                    (ToolbarItemKind::Toggle { on: seed }, act)
                }
                Kind::SidebarToggle => (ToolbarItemKind::SidebarToggle, 0),
                Kind::Label => (ToolbarItemKind::Label, 0),
                Kind::Separator => (ToolbarItemKind::Separator, 0),
                Kind::Space => (ToolbarItemKind::Space, 0),
                Kind::FlexibleSpace => (ToolbarItemKind::FlexibleSpace, 0),
            };

            if let Some(f) = enabled_when {
                let item = id.clone();
                bind(
                    move || f(),
                    move |on: &bool| {
                        day_core::patch_window_toolbar(
                            window,
                            ToolbarPatch::Enabled {
                                item: item.clone(),
                                on: *on,
                            },
                        );
                    },
                );
            }

            ToolbarItem {
                id,
                kind,
                label,
                tooltip: tooltip.map(|t| t.initial()),
                icon,
                enabled,
                action: action_id,
            }
        })
        .collect()
}

/// Install the toolbar on the window being built — the primary window at startup, and the new
/// window inside a `register_new_window` builder. Replaces any previous toolbar on that window;
/// an empty `items` removes it. Add or remove an item by calling this again with a different
/// list, or use [`toolbar_reactive`] to keep the list derived from state.
///
/// Labels resolve once, in the install-time locale; an app whose language can change at runtime
/// should use [`toolbar_reactive`].
pub fn toolbar(items: Vec<ToolbarEntry>) {
    // Resolved ONCE and passed down: the bindings `lower` creates outlive this call and fire when
    // no window is being built, where the target would otherwise fall back to the primary.
    let window = day_core::current_window();
    day_core::set_window_toolbar(window, lower(items, window));
}

/// [`toolbar`] that re-lowers and re-installs whenever a reactive read inside `builder` changes —
/// a locale switch, or a command list that depends on what is selected.
///
/// Each pass replaces the previous one's bindings and closures. Because a replace rebuilds the
/// whole bar, keep per-value changes off this path: bind a toggle's signal, bind a search field's
/// signal, and use [`ToolbarEntry::enabled_when`], all of which patch a single item instead.
pub fn toolbar_reactive(builder: impl Fn() -> Vec<ToolbarEntry> + 'static) {
    // Each lowering pass owns its bindings: they hang off a child scope that the NEXT pass
    // disposes, so a rebuilt bar does not leave the old one's bindings writing patches at items
    // that no longer exist.
    let pass: Rc<RefCell<Option<Scope>>> = Rc::new(RefCell::new(None));
    // Captured HERE, while the window that owns this toolbar is still the one being built. The
    // effect below re-runs on a locale switch or a state change, long after that build has
    // finished, and would then resolve to the primary window — so a second window's rebuild
    // replaced the PRIMARY window's toolbar rather than its own.
    let window = day_core::current_window();
    let outer = Scope::root().enter(Scope::child);
    outer.enter(|| {
        day_reactive::Effect::new(move || {
            // Track the locale even when the builder has no localized reads of its own.
            let _ = day_l10n::locale().get();
            let entries = builder();
            let next = Scope::root().enter(Scope::child);
            let items = next.enter(|| lower(entries, window));
            if let Some(old) = pass.borrow_mut().replace(next) {
                old.dispose();
            }
            day_core::set_window_toolbar(window, items);
        });
    });
}
