// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Toolbar items: `toolbar_button`, `toolbar_toggle`, `toolbar_menu`, `toolbar_label`,
//! `toolbar_segmented`, `toolbar_separator` — and [`Decorate::toolbar`], the one way to declare
//! them (docs/toolbars.md).
//!
//! WHERE an item appears is decided by the piece that declares it, so an app never says it twice:
//! items declared on a destination page ride the detail chrome and leave when the page does,
//! items on a `selector` ride its sidebar column, and items on a window's root piece ride every
//! page of that window. [`day_spec::ToolbarPlacement`] then says where on that chrome the item
//! sits.
//!
//! SEARCH is not here. It is declared on the navigation surface it filters
//! (`Selector::searchable`, docs/search.md), which is what lets the platform move it — into the
//! navigation list on a narrow window — without the app re-declaring anything.
//!
//! A toolbar is chrome, not a piece: it is not laid out by day and does not live in the tree. It
//! lowers to day_spec's toolkit-neutral [`day_spec::ToolbarItem`] model, which each backend
//! realizes with its platform's own bar — `NSToolbar`, `AdwHeaderBar`, `QToolBar`, `CommandBar`,
//! a `UINavigationItem`, a `MaterialToolbar` menu (docs/toolbars.md).
//!
//! ```ignore
//! reader_page(article).toolbar([
//!     toolbar_button("refresh", tr("refresh")).icon(Symbol::Refresh).action(refresh_all),
//!     toolbar_toggle("star", tr("star"), starred).icon(Symbol::Star),
//! ])
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use day_reactive::{Scope, Signal, bind, bind_seeded};
use day_spec::{Icon, Symbol, ToolbarItem, ToolbarItemKind, ToolbarPatch, ToolbarValue};

use crate::{IntoText, MenuEntry, TextSource};

/// A toolbar item under construction. Build a command with [`toolbar_button`], a two-state
/// button with [`toolbar_toggle`], a pull-down with [`toolbar_menu`], one control over a set of
/// choices with [`toolbar_segmented`], static text with [`toolbar_label`], and a divider with
/// [`toolbar_separator`]. Search is declared on the navigation surface instead
/// (`Selector::searchable`, docs/search.md).
#[derive(Clone)]
pub struct ToolbarEntry {
    id: String,
    kind: Kind,
    label: Option<TextSource>,
    tooltip: Option<TextSource>,
    icon: Option<Icon>,
    enabled: bool,
    enabled_when: Option<Rc<dyn Fn() -> bool>>,
    action: Option<Rc<dyn Fn()>>,
    placement: day_spec::ToolbarPlacement,
    label_style: day_spec::LabelStyle,
    prominent: bool,
}

/// The app-side kinds, carrying the live signals the spec model cannot.
#[derive(Clone)]
enum Kind {
    Button,
    Segmented(Vec<Segment>, Signal<usize>),
    Toggle(Signal<bool>),
    Menu(Vec<MenuEntry>),
    Label,
    Separator,
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
        placement: day_spec::ToolbarPlacement::Automatic,
        label_style: day_spec::LabelStyle::Automatic,
        prominent: false,
    }
}

/// A push button: `toolbar_button("refresh", tr("refresh")).icon(Symbol::Refresh).action(…)`.
pub fn toolbar_button<M>(id: impl Into<String>, label: impl IntoText<M>) -> ToolbarEntry {
    ToolbarEntry {
        label: Some(label.into_text()),
        ..entry(id, Kind::Button)
    }
}

/// A row of mutually exclusive choices as ONE native control (docs/toolbars.md): the platform's
/// segmented control, bound to `selected`.
///
/// Reach for this instead of N toggles whenever exactly one choice is on at a time — a theme
/// picker, a view mode. The platform then draws it as the single control it is, announces it as
/// one, and keeps the exclusivity itself; three toggles leave all of that to the app.
///
/// ```ignore
/// toolbar_segmented("theme", vec![
///     segment(tr("light")).icon(Symbol::Light),
///     segment(tr("system")).icon(Symbol::Auto),
///     segment(tr("dark")).icon(Symbol::Dark),
/// ], mode)
/// ```
pub fn toolbar_segmented(
    id: impl Into<String>,
    segments: Vec<Segment>,
    selected: Signal<usize>,
) -> ToolbarEntry {
    ToolbarEntry {
        label: None,
        ..entry(id, Kind::Segmented(segments, selected))
    }
}

/// One choice in a [`toolbar_segmented`] control.
#[derive(Clone)]
pub struct Segment {
    label: TextSource,
    icon: Option<day_spec::Icon>,
}

/// A segment showing `label`; add `.icon(…)` for the platforms that draw one.
pub fn segment<M>(label: impl IntoText<M>) -> Segment {
    Segment {
        label: label.into_text(),
        icon: None,
    }
}

impl Segment {
    /// A standard [`Symbol`](day_spec::Symbol), drawn with the platform's own glyph.
    pub fn icon(mut self, s: day_spec::Symbol) -> Segment {
        self.icon = Some(day_spec::Icon::Symbol(s));
        self
    }
    /// A bundled image from `resource/images`, for a segment the standard set has no glyph for.
    pub fn image(mut self, name: impl Into<String>) -> Segment {
        self.icon = Some(day_spec::Icon::Image(name.into()));
        self
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

/// A divider, where the platform draws one (macOS toolbars have none — AppKit renders it as a
/// fixed gap; docs/toolbars.md).
pub fn toolbar_separator() -> ToolbarEntry {
    entry("", Kind::Separator)
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

    /// This item's role on the chrome carrying it (docs/toolbars.md) — leading, centered,
    /// trailing, or first to fold away. It never names a surface: which chrome an item rides
    /// follows from the piece that declared it.
    pub fn placement(mut self, placement: day_spec::ToolbarPlacement) -> ToolbarEntry {
        self.placement = placement;
        self
    }

    /// Draw the title, the icon, or both, where the platform can do more than one. An item
    /// folded into an overflow menu shows its title whatever this asks for.
    pub fn label_style(mut self, style: day_spec::LabelStyle) -> ToolbarEntry {
        self.label_style = style;
        self
    }

    /// Draw this item in the platform's emphasized style — for the one action a chrome is
    /// really offering.
    pub fn prominent(mut self) -> ToolbarEntry {
        self.prominent = true;
        self
    }
}

/// Where a contribution's items come from: a fixed list, or a closure re-run on every change.
#[derive(Clone)]
pub enum ToolbarSource {
    Fixed(Vec<ToolbarEntry>),
    Derived(Rc<dyn Fn() -> Vec<ToolbarEntry>>),
}

/// Anything [`crate::Decorate::toolbar`] accepts: one entry, a list of them, or a closure that
/// derives the list and re-runs whenever its reactive reads change.
///
/// The marker parameter `M` is what lets one method name take all three. A blanket impl over
/// `Fn() -> Vec<ToolbarEntry>` and a concrete impl for `Vec<ToolbarEntry>` overlap as far as
/// coherence can tell, since it will not assume `Vec` never gains an `Fn` impl — the same E0119
/// dodge as [`IntoText`](crate::IntoText), and resolved the same way.
pub trait ToolbarContent<M> {
    fn into_source(self) -> ToolbarSource;
}

/// Marker for a single [`ToolbarEntry`].
pub struct OneMark;
/// Marker for a list of entries — a `Vec` or an array.
pub struct ManyMark;
/// Marker for a closure that derives the list.
pub struct DerivedMark;

impl ToolbarContent<OneMark> for ToolbarEntry {
    fn into_source(self) -> ToolbarSource {
        ToolbarSource::Fixed(vec![self])
    }
}

impl ToolbarContent<ManyMark> for Vec<ToolbarEntry> {
    fn into_source(self) -> ToolbarSource {
        ToolbarSource::Fixed(self)
    }
}

impl<const N: usize> ToolbarContent<ManyMark> for [ToolbarEntry; N] {
    fn into_source(self) -> ToolbarSource {
        ToolbarSource::Fixed(Vec::from(self))
    }
}

impl<F: Fn() -> Vec<ToolbarEntry> + 'static> ToolbarContent<DerivedMark> for F {
    fn into_source(self) -> ToolbarSource {
        ToolbarSource::Derived(Rc::new(self))
    }
}

/// The item a sidebar host draws for itself: leading, the platform's own glyph, and no app
/// action — the click drives that window's split directly.
pub fn sidebar_toggle_item() -> ToolbarEntry {
    toolbar_button(
        day_spec::SIDEBAR_TOGGLE_ID,
        day_l10n::t("day-toggle-sidebar"),
    )
    .icon(Symbol::Sidebar)
    .placement(day_spec::ToolbarPlacement::Navigation)
    .action(|| {
        day_core::toggle_sidebar();
    })
}

/// Register `content` against `chrome` for as long as the CURRENT reactive scope lives.
///
/// The one path every declaration takes: [`crate::Decorate::toolbar`] resolves the chrome from
/// where the piece sits, and a `selector` names its own sidebar page explicitly. A fixed list
/// lowers once; a derived one lowers inside an `Effect`, and each pass owns its bindings through
/// a child scope the next pass disposes, so a re-derived bar leaves no binding writing patches at
/// items that no longer exist.
pub fn contribute(chrome: day_core::Chrome, content: ToolbarSource) {
    // Whether the page carrying these is ON SCREEN. The toolkits that draw one bar per window
    // are handed the showing pages' items already merged, and this is how day-core knows which
    // those are — the same gate stack `register_nav` uses, so "showing" means one thing across
    // the navigation layer (docs/toolbars.md).
    // Whether the page carrying these is ON SCREEN, from the page itself (docs/toolbars.md).
    let active = day_core::current_page_gate();
    let gate = active.clone();
    // Captured HERE, at the declaration site: a derived list re-runs long after this build, when
    // no page is being built and the column would answer `Window` (docs/toolbars.md).
    let column = day_core::current_page_column();
    let token = match content {
        ToolbarSource::Fixed(entries) => {
            day_core::register_contribution_gated(chrome, lower(entries, chrome, column), active)
        }
        ToolbarSource::Derived(builder) => {
            let token = day_core::register_contribution_gated(chrome, Vec::new(), active);
            let pass: Rc<RefCell<Option<Scope>>> = Rc::new(RefCell::new(None));
            let outer = Scope::child();
            outer.enter(|| {
                day_reactive::Effect::new(move || {
                    // Track the locale even when the builder has no localized reads of its own.
                    let _ = day_l10n::locale().get();
                    let entries = builder();
                    let next = Scope::root().enter(Scope::child);
                    let items = next.enter(|| lower(entries, chrome, column));
                    if let Some(old) = pass.borrow_mut().replace(next) {
                        old.dispose();
                    }
                    day_core::update_contribution(token, items);
                });
            });
            token
        }
    };
    // Recompose when what is ON SCREEN changes. Reading the gate inside an effect subscribes to
    // whatever it reads — the selection, the pushed path — so a tab switch or a push re-merges
    // the window's bar without the navigation layer having to announce it.
    if let Some(gate) = gate {
        day_reactive::Effect::new(move || {
            let _ = gate();
            day_core::chrome_changed();
        });
    }
    Scope::current().on_cleanup(move || day_core::unregister_contribution(token));
}

/// Lower app-side entries to the spec model, registering each item's closures with day-core and
/// wiring the live bindings (toggle state, search text, `enabled_when`).
fn lower(
    entries: Vec<ToolbarEntry>,
    chrome: day_core::Chrome,
    column: day_spec::ToolbarColumn,
) -> Vec<ToolbarItem> {
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
                placement,
                label_style,
                prominent,
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
                            day_core::patch_chrome(
                                chrome,
                                ToolbarPatch::On {
                                    item: item.clone(),
                                    on: *v,
                                },
                            );
                        },
                    );
                    (ToolbarItemKind::Toggle { on: seed }, act)
                }
                Kind::Segmented(segments, sel) => {
                    let seed = sel.get_untracked().min(segments.len().saturating_sub(1));
                    let item = id.clone();
                    let extra = extra.clone();
                    let act = day_core::register_toolbar_value(Rc::new(move |v: &ToolbarValue| {
                        if let ToolbarValue::Selected(next) = v {
                            sel.set(*next);
                            if let Some(f) = &extra {
                                f();
                            }
                        }
                    }));
                    // The app's own writes patch the one item back, exactly as a toggle's do.
                    bind_seeded(
                        seed,
                        move || sel.get(),
                        move |v: &usize| {
                            day_core::patch_chrome(
                                chrome,
                                ToolbarPatch::Selected {
                                    item: item.clone(),
                                    index: *v,
                                },
                            );
                        },
                    );
                    (
                        ToolbarItemKind::Segmented {
                            segments: segments
                                .into_iter()
                                .map(|s| day_spec::ToolbarSegment {
                                    title: s.label.initial(),
                                    icon: s.icon,
                                })
                                .collect(),
                            selected: seed,
                        },
                        act,
                    )
                }
                Kind::Label => (ToolbarItemKind::Label, 0),
                Kind::Separator => (ToolbarItemKind::Separator, 0),
            };

            // SEED the item from the predicate, rather than leaving the declared default and
            // letting the binding correct it: the binding's first run happens HERE, inside
            // `lower`, and the model it patches is only stored by the `set_window_toolbar` this
            // list is on its way to — so the correction landed on the previous bar (or nothing)
            // and the new one installed enabled. A command that starts out unavailable then
            // lowered live and answered dayscript's `toolbar:` step.
            let enabled = match &enabled_when {
                Some(f) => day_reactive::untrack(|| f()),
                None => enabled,
            };
            if let Some(f) = enabled_when {
                let item = id.clone();
                bind(
                    move || f(),
                    move |on: &bool| {
                        day_core::patch_chrome(
                            chrome,
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
                placement,
                label_style,
                prominent,
                column,
            }
        })
        .collect()
}
