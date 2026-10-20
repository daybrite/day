// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The app menu builder: `menu_item`, `sub_menu`, `menu_separator`, and `menu_role`, assembled
//! with `app_menu`. Lowers to day_spec's toolkit-neutral `MenuItem` model and registers each
//! item's action closure.

use std::cell::RefCell;
use std::rc::Rc;

use day_reactive::Scope;

use crate::{IntoReactive, IntoText, Reactive, TextSource};

// ---------------------------------------------------------------------------
// Menus: the app-side builder over day_spec's toolkit-neutral MenuItem model. Lowering registers
// each item's action closure with day-core (which dispatches `Event::MenuAction`) and assigns its
// id.
// ---------------------------------------------------------------------------

/// A menu entry under construction. Build a command with [`menu_item`], a nested submenu with
/// [`sub_menu`], a standard system command with [`menu_role`], and a divider with [`menu_separator`].
/// Attach to a Piece via [`Decorate::context_menu`] or install app-wide via [`app_menu`].
#[derive(Clone)]
pub struct MenuEntry {
    id: Option<String>,
    /// The title, live: resolved (tracked) when the menu is lowered, so a localized title
    /// follows a language switch and a title built from a signal follows the signal.
    label: TextSource,
    shortcut: Option<day_spec::Shortcut>,
    enabled: Reactive<bool>,
    role: Option<day_spec::MenuRole>,
    icon: Option<day_spec::Icon>,
    checked: Option<Reactive<bool>>,
    action: Option<Rc<dyn Fn()>>,
    children: Option<Vec<MenuEntry>>,
    separator: bool,
    bar_role: Option<day_spec::MenuBarRole>,
}

impl MenuEntry {
    fn command(label: TextSource) -> MenuEntry {
        MenuEntry {
            id: None,
            label,
            shortcut: None,
            enabled: Reactive::Const(true),
            role: None,
            icon: None,
            bar_role: None,
            checked: None,
            action: None,
            children: None,
            separator: false,
        }
    }

    /// A standard [`Symbol`](day_spec::Symbol) beside the item's title, drawn with the
    /// platform's glyph, the same vocabulary toolbars take. Menus carry icons on macOS,
    /// Windows, GNOME, KDE and Android; a backend whose menus are text-only ignores it, so an
    /// icon is always an addition to a menu that already reads correctly without one.
    pub fn icon(mut self, s: day_spec::Symbol) -> MenuEntry {
        self.icon = Some(day_spec::Icon::Symbol(s));
        self
    }

    /// A bundled image from `resource/images`, for an item the standard set has no glyph for:
    /// an app's own vocabulary (a shape, a brand). Same rule as [`MenuEntry::icon`].
    pub fn image(mut self, name: impl Into<String>) -> MenuEntry {
        self.icon = Some(day_spec::Icon::Image(name.into()));
        self
    }
    /// Run `f` when the item is chosen.
    pub fn action(mut self, f: impl Fn() + 'static) -> MenuEntry {
        self.action = Some(Rc::new(f));
        self
    }
    /// Full shortcut spec, e.g. `Shortcut::primary("s").shift()`.
    pub fn shortcut(mut self, s: day_spec::Shortcut) -> MenuEntry {
        self.shortcut = Some(s);
        self
    }
    /// Convenience: the platform's primary modifier (⌘ / Ctrl) + `key`. This sets the item's
    /// shortcut; to name the item, use [`MenuEntry::id`].
    pub fn key(mut self, key: impl Into<String>) -> MenuEntry {
        self.shortcut = Some(day_spec::Shortcut::primary(key));
        self
    }

    /// Name this item, so something outside the menu can address it: a dayscript
    /// `menu: { id: "view-both" }` step, or a `tap` on the composed menu's row where the platform
    /// has no native menus.
    ///
    /// A label cannot do that job. It is localized, so it changes with the run's language, and an
    /// item that shows its own state changes it whenever the state moves, which is exactly the
    /// case [`MenuEntry::checked`] creates. Give an id to any item a script drives.
    pub fn id(mut self, id: impl Into<String>) -> MenuEntry {
        self.id = Some(id.into());
        self
    }

    /// Draw this item with the platform's check mark, on or off, for a setting the menu toggles
    /// or one of several mutually exclusive choices. An item that never calls this is a plain
    /// command and reserves no room for a mark.
    ///
    /// Choosing the item still runs its [`action`](MenuEntry::action); no backend flips the mark
    /// by itself. The app owns the state: pass the signal (or a closure reading it) and the mark
    /// follows the state, the menu re-lowering when it changes.
    ///
    /// ```ignore
    /// menu_item(tr("view-grid")).id("view-grid").checked(grid).action(move || grid.set(!grid.get()))
    /// ```
    pub fn checked<M>(mut self, on: impl IntoReactive<bool, M>) -> MenuEntry {
        self.checked = Some(on.into_reactive());
        self
    }
    /// Whether the item can be chosen (default `true`): a constant, a `Signal<bool>`, or a
    /// closure, re-read whenever the menu is lowered.
    pub fn enabled<M>(mut self, on: impl IntoReactive<bool, M>) -> MenuEntry {
        self.enabled = on.into_reactive();
        self
    }
    /// Tag a custom command with a standard [`day_spec::MenuRole`] (usually you use [`menu_role`]).
    pub fn role(mut self, role: day_spec::MenuRole) -> MenuEntry {
        self.role = Some(role);
        self
    }
}

/// A clickable command: `menu_item(res::str::save()).key("s").action(|| …)`. The title takes
/// the same sources as a label (a literal, a localized text, a signal, a closure) and stays
/// live.
pub fn menu_item<M>(label: impl IntoText<M>) -> MenuEntry {
    MenuEntry::command(label.into_text())
}

/// A nested submenu: `sub_menu(res::str::menu_file(), vec![menu_item(…), …])`.
pub fn sub_menu<M>(label: impl IntoText<M>, items: Vec<MenuEntry>) -> MenuEntry {
    MenuEntry {
        children: Some(items),
        ..MenuEntry::command(label.into_text())
    }
}

/// Claim one of the platform's standard menu-bar slots for this submenu: the backend places it
/// where that menu belongs and does not add its stock version. Without a role, a submenu is
/// an app menu and sits between the standard ones.
///
/// ```ignore
/// sub_menu(tr("menu-file"), vec![…]).bar_role(MenuBarRole::File)
/// ```
impl MenuEntry {
    pub fn bar_role(mut self, role: day_spec::MenuBarRole) -> MenuEntry {
        self.bar_role = Some(role);
        self
    }
}

/// A visual divider between items.
pub fn menu_separator() -> MenuEntry {
    MenuEntry {
        separator: true,
        ..MenuEntry::command(TextSource::Static(String::new()))
    }
}

/// A standard/system command (`MenuRole::Copy`, `MenuRole::Quit`, …) rendered with the platform's
/// native item (correct label, default shortcut, focus-targeting, and automatic enable/disable),
/// so default menu items (Edit ▸ Cut/Copy/Paste, the app's Quit/About) work without
/// re-implementation.
pub fn menu_role(role: day_spec::MenuRole) -> MenuEntry {
    MenuEntry {
        role: Some(role),
        ..MenuEntry::command(TextSource::Static(String::new()))
    }
}

/// The core-catalog key for a standard menu command's label (docs/menus.md, docs/localization.md).
fn role_catalog_key(role: day_spec::MenuRole) -> &'static str {
    use day_spec::MenuRole as R;
    match role {
        R::Cut => "day-cut",
        R::Copy => "day-copy",
        R::Paste => "day-paste",
        R::SelectAll => "day-select-all",
        R::Undo => "day-undo",
        R::Redo => "day-redo",
        R::Delete => "day-delete",
        R::About => "day-about",
        R::Quit => "day-quit",
        R::Preferences => "day-preferences",
        R::Minimize => "day-minimize",
        R::CloseWindow => "day-close",
        R::Fullscreen => "day-fullscreen",
        R::NewWindow => "day-new-window",
        R::CheckForUpdates => "day-check-for-updates",
    }
}

/// Lower app-side entries to the spec model, registering action closures with day-core. A standard
/// `role` item with no explicit label gets its label from the localized core catalog here, so the
/// backends receive a ready, locale-correct label instead of each hardcoding English (day-l10n).
///
/// This variant registers process-lived closures, correct for the app menu and toolbars, whose
/// ids day-core manages by shape-rebinding, toolbar slots and explicit sweeps. Menus owned by a piece build
/// (a `.context_menu`, a nav row's menu) go through [`lower_menu_scoped`] instead, so their
/// closures are reclaimed when the registering scope is disposed rather than leaking per remount.
pub(crate) fn lower_menu(entries: Vec<MenuEntry>) -> Vec<day_spec::MenuItem> {
    lower_menu_with(entries, &day_core::register_menu_action)
}

/// [`lower_menu`] with scope-tied action registration (see there for when to use which).
pub(crate) fn lower_menu_scoped(entries: Vec<MenuEntry>) -> Vec<day_spec::MenuItem> {
    lower_menu_with(entries, &day_core::register_scoped_menu_action)
}

fn lower_menu_with(
    entries: Vec<MenuEntry>,
    register: &dyn Fn(Rc<dyn Fn()>) -> u64,
) -> Vec<day_spec::MenuItem> {
    entries
        .into_iter()
        .map(|e| {
            if e.separator {
                day_spec::MenuItem::Separator
            } else if let Some(children) = e.children {
                day_spec::MenuItem::Submenu {
                    label: e.label.resolve(),
                    items: lower_menu_with(children, register),
                    role: e.bar_role,
                }
            } else {
                let mut action = e.action.map(register).unwrap_or(0);
                // Tracked reads: a menu lowered inside `app_menu`'s effect re-lowers when
                // a title, availability or check state changes.
                let mut enabled = e.enabled.get();
                let mut shortcut = e.shortcut;
                // Window roles have no native selector on any platform: an action-less item
                // lowers to the registered day dispatcher (docs/windows.md): live when the
                // app registered a builder/preferences piece, disabled otherwise.
                if action == 0 {
                    match e.role {
                        Some(day_spec::MenuRole::NewWindow) => {
                            action = day_core::windows::new_window_action_id();
                            enabled = enabled && action != 0;
                            shortcut = shortcut.or(Some(day_spec::Shortcut::primary("n")));
                        }
                        // The undo pair and the clipboard trio lower to standing dispatchers
                        // too, for toolkits whose role items come back as plain menu actions
                        // (Android's app-bar menu, the iOS menu, web context menus). Toolkits
                        // with a native responder route ignore the action and keep their selector
                        // (see each backend's menu build). Each also takes the platform-neutral
                        // standard shortcut (primary+Z/X/C/V/A, shift for redo) unless the app
                        // set its own. AppKit's native items already carry these, so this is
                        // what gives GTK/Qt/web the same accelerators.
                        Some(day_spec::MenuRole::Undo) => {
                            action = day_core::undo_action_id(false);
                            shortcut = shortcut.or(Some(day_spec::Shortcut::primary("z")));
                        }
                        Some(day_spec::MenuRole::Redo) => {
                            action = day_core::undo_action_id(true);
                            shortcut = shortcut.or(Some(day_spec::Shortcut::primary("z").shift()));
                        }
                        Some(day_spec::MenuRole::Cut) => {
                            action = day_core::edit_action_id(day_spec::EditOp::Cut);
                            shortcut = shortcut.or(Some(day_spec::Shortcut::primary("x")));
                        }
                        Some(day_spec::MenuRole::Copy) => {
                            action = day_core::edit_action_id(day_spec::EditOp::Copy);
                            shortcut = shortcut.or(Some(day_spec::Shortcut::primary("c")));
                        }
                        Some(day_spec::MenuRole::Paste) => {
                            action = day_core::edit_action_id(day_spec::EditOp::Paste);
                            shortcut = shortcut.or(Some(day_spec::Shortcut::primary("v")));
                        }
                        Some(day_spec::MenuRole::SelectAll) => {
                            action = day_core::edit_action_id(day_spec::EditOp::SelectAll);
                            shortcut = shortcut.or(Some(day_spec::Shortcut::primary("a")));
                        }
                        Some(day_spec::MenuRole::Preferences) => {
                            action = day_core::windows::preferences_action_id();
                            shortcut = shortcut.or(Some(day_spec::Shortcut::primary(",")));
                        }
                        // About keeps the toolkit's stock panel (action 0) until an app
                        // registers its own (docs/settings.md); the updater command has no
                        // native fallback and lowers disabled when nothing registered.
                        Some(day_spec::MenuRole::About) => {
                            action = day_core::windows::about_action_id();
                        }
                        Some(day_spec::MenuRole::CheckForUpdates) => {
                            action = day_core::windows::check_for_updates_action_id();
                            enabled = enabled && action != 0;
                        }
                        _ => {}
                    }
                }
                let label = e.label.resolve();
                let label = match (label.is_empty(), e.role) {
                    (true, Some(role)) => day_l10n::t(role_catalog_key(role)),
                    _ => label,
                };
                day_spec::MenuItem::Action {
                    id: e.id,
                    action,
                    label,
                    shortcut,
                    enabled,
                    checked: e.checked.as_ref().map(Reactive::get),
                    role: e.role,
                    icon: e.icon,
                }
            }
        })
        .collect()
}

/// Where a menu's entries come from: a fixed list, or a closure that derives the list and
/// re-runs whenever its reactive reads change. What [`app_menu`], [`dock_menu`] and
/// [`StatusItem::menu`](crate::StatusItem::menu) accept, through [`MenuContent`].
#[derive(Clone)]
pub enum MenuSource {
    Fixed(Vec<MenuEntry>),
    Derived(Rc<dyn Fn() -> Vec<MenuEntry>>),
}

impl MenuSource {
    /// The entries right now: a tracked read for a derived source.
    pub(crate) fn entries(&self) -> Vec<MenuEntry> {
        match self {
            MenuSource::Fixed(v) => v.clone(),
            MenuSource::Derived(f) => f(),
        }
    }
}

/// Anything an app menu takes: a `Vec` or array of entries, or a closure that derives the
/// entries and re-runs whenever its reactive reads change. The marker parameter is the same
/// E0119 dodge as [`ToolbarContent`](crate::ToolbarContent).
pub trait MenuContent<M> {
    fn into_source(self) -> MenuSource;
}

impl MenuContent<crate::ManyMark> for Vec<MenuEntry> {
    fn into_source(self) -> MenuSource {
        MenuSource::Fixed(self)
    }
}

impl<const N: usize> MenuContent<crate::ManyMark> for [MenuEntry; N] {
    fn into_source(self) -> MenuSource {
        MenuSource::Fixed(Vec::from(self))
    }
}

impl<C, F> MenuContent<crate::DerivedMark> for F
where
    C: IntoIterator<Item = MenuEntry>,
    F: Fn() -> C + 'static,
{
    fn into_source(self) -> MenuSource {
        MenuSource::Derived(Rc::new(move || self().into_iter().collect()))
    }
}

thread_local! {
    /// The scope owning the current app-menu and dock-menu installs, so a second call replaces
    /// the first (and reclaims its bindings) instead of leaving two installers racing.
    static APP_MENU_SCOPE: RefCell<Option<Scope>> = const { RefCell::new(None) };
    static DOCK_MENU_SCOPE: RefCell<Option<Scope>> = const { RefCell::new(None) };
}

/// Install `source` through `install`, in a root-owned scope that replaces the previous one.
fn install_menu(
    slot: &'static std::thread::LocalKey<RefCell<Option<Scope>>>,
    source: MenuSource,
    install: fn(Vec<day_spec::MenuItem>),
) {
    if let Some(old) = slot.with(|s| s.borrow_mut().take()) {
        old.dispose();
    }
    let scope = Scope::root().enter(Scope::child);
    scope.enter(|| {
        day_reactive::bind(
            move || {
                // Track the locale even when the entries carry no localized reads, so
                // role-label fallbacks still refresh; every entry's title, availability and
                // check state is a tracked read of its own inside `lower_menu`.
                let _ = day_l10n::locale().get();
                lower_menu(source.entries())
            },
            move |items: &Vec<day_spec::MenuItem>| install(items.clone()),
        );
    });
    slot.with(|s| *s.borrow_mut() = Some(scope));
}

/// Install the application menu: the native menu bar on desktop, the app-bar overflow on Android,
/// the UIMenuBuilder main menu on iPadOS/Catalyst. Top-level entries are usually `sub_menu(…)`s
/// (the menu-bar menus). Takes a list of entries or a closure that derives them.
///
/// The menu is live (docs/menus.md): every title, `enabled` and `checked` is re-read, and the
/// menu re-lowered and reinstalled, whenever one of them changes, so `menu_role` labels,
/// `res::str` titles and `day::tr` all follow a runtime language switch, and a derived closure
/// re-runs on whatever signals it reads. Calling it again replaces the previous install and
/// reclaims its action closures. Install once, at startup.
///
/// ```ignore
/// app_menu([
///     sub_menu(res::str::menu_file(), vec![menu_item(res::str::new_item()).key("n").action(add)]),
///     sub_menu(res::str::menu_view(), vec![menu_item(res::str::grid()).checked(grid).action(flip)]),
/// ]);
/// ```
pub fn app_menu<M>(menus: impl MenuContent<M>) {
    install_menu(&APP_MENU_SCOPE, menus.into_source(), day_core::set_app_menu);
}

/// Install the app's Dock menu (docs/menus.md "Dock menu"): the entries macOS lists when the
/// user right-clicks the app's Dock icon, above the app's `[[shortcuts]]` and the system's own
/// Show / Hide / Quit items. Probe `Cap::DockMenu`; elsewhere the call does nothing. Live, like
/// [`app_menu`]: "Resume <last document>", a checked "Do Not Disturb", the locale. Install
/// once, at startup.
///
/// The menu works while the app runs, so its actions are ordinary closures, unlike shortcuts,
/// which are routes because they also launch the app.
pub fn dock_menu<M>(entries: impl MenuContent<M>) {
    install_menu(
        &DOCK_MENU_SCOPE,
        entries.into_source(),
        day_core::set_dock_menu,
    );
}

// ---------------------------------------------------------------------------
// The composed context menu (docs/menus.md "Dynamic context menus"): presentation for a
// toolkit that REPORTS the summon (`Event::ContextMenu`) instead of showing a native menu
// of its own (web-dom). Works on the LOWERED model, so role resolution, localization, and
// per-summon action scoping are exactly what a native backend would have received.
// ---------------------------------------------------------------------------

/// The composed menu's stand-in for a native check mark, and the blank that holds its column.
const COMPOSED_CHECK_ON: &str = "\u{2713}\u{2002}";
const COMPOSED_CHECK_OFF: &str = "\u{2002}\u{2002}";
/// Panel width of the composed menu, and the extra indent inlined submenu items take.
const COMPOSED_MENU_W: f64 = 220.0;
const COMPOSED_SUBMENU_INDENT: f64 = 12.0;

/// Mount the composed presentation beside a `.context_menu` node: an
/// `Event::ContextMenu` handler plus a lazily-armed, unrouted [`crate::cover`] that shows the
/// provider's items at the summon point. Nothing builds until the first summon arrives, so
/// backends that serve the menu natively (and therefore never emit the event) never pay.
pub(crate) fn mount_composed_menu(
    cx: &mut day_core::BuildCx,
    node: day_core::RNode,
    provider: day_spec::ContextMenuFn,
) {
    use day_reactive::Signal;
    use std::cell::RefCell;

    struct Pending {
        items: Vec<day_spec::MenuItem>,
        at: day_spec::Point,
    }
    let armed = Signal::new(false);
    let open: Signal<Option<String>> = Signal::new(None);
    let pending: Rc<RefCell<Pending>> = Rc::new(RefCell::new(Pending {
        items: Vec::new(),
        at: day_spec::Point::ZERO,
    }));

    {
        let pending = pending.clone();
        cx.on(node, move |ev| {
            if let day_spec::Event::ContextMenu { local, window } = ev {
                let items = provider(*local);
                if items.is_empty() {
                    return;
                }
                *pending.borrow_mut() = Pending { items, at: *window };
                armed.set(true);
                open.set(Some("menu".into()));
            }
        });
    }

    let host = crate::when(
        move || armed.get(),
        move || {
            let pending = pending.clone();
            crate::cover(open, move |_: &String| {
                let (items, at) = {
                    let p = pending.borrow();
                    (p.items.clone(), p.at)
                };
                composed_menu_host(items, at, open)
            })
            .unrouted()
            .background(|_| day_spec::Color::rgba(0.0, 0.0, 0.0, 0.0))
        },
    );
    let _ = day_core::Piece::build(host, cx);
}

/// The presented surface: a fullscreen dismiss catcher plus the item panel, placed at the
/// summon point (clamped into the window by [`MenuPlace`]).
fn composed_menu_host(
    items: Vec<day_spec::MenuItem>,
    at: day_spec::Point,
    open: day_reactive::Signal<Option<String>>,
) -> impl day_core::Piece {
    use crate::Decorate;

    let mut rows: Vec<day_core::AnyPiece> = Vec::new();
    let mut next = 0usize;
    composed_menu_rows(&items, 0.0, &mut next, open, &mut rows);

    let panel = crate::column(rows)
        .padding(day_spec::Insets::symmetric(0.0, 5.0))
        .background(|| {
            if day_core::dark_mode() {
                day_spec::Color::rgb(0.22, 0.22, 0.24)
            } else {
                day_spec::Color::rgb(0.97, 0.97, 0.97)
            }
        })
        .corner_radius(8.0);
    let catcher = crate::spacer()
        .background(day_spec::Color::rgba(0.0, 0.0, 0.0, 0.0))
        .on_tap(move || open.set(None));
    ComposedMenuHost {
        at,
        catcher: day_core::AnyPiece::new(catcher),
        panel: day_core::AnyPiece::new(panel),
    }
}

/// One flattened item row per entry. Submenus inline as a dimmed header plus indented
/// children, since the composed panel has no flyout (docs/menus.md).
fn composed_menu_rows(
    items: &[day_spec::MenuItem],
    indent: f64,
    next: &mut usize,
    open: day_reactive::Signal<Option<String>>,
    out: &mut Vec<day_core::AnyPiece>,
) {
    use crate::Decorate;
    for item in items {
        match item {
            day_spec::MenuItem::Separator => {
                out.push(day_core::AnyPiece::new(
                    crate::divider().padding(day_spec::Insets::symmetric(0.0, 4.0)),
                ));
            }
            day_spec::MenuItem::Submenu { label, items, .. } => {
                out.push(day_core::AnyPiece::new(
                    crate::label(label.clone())
                        .padding(day_spec::Insets::symmetric(12.0 + indent, 5.0))
                        .width(COMPOSED_MENU_W)
                        .opacity(0.55),
                ));
                composed_menu_rows(items, indent + COMPOSED_SUBMENU_INDENT, next, open, out);
            }
            day_spec::MenuItem::Action {
                id,
                action,
                label,
                enabled,
                checked,
                ..
            } => {
                let n = *next;
                *next += 1;
                // A checkable item reserves the mark's column whichever way it sits, so a run of
                // choices lines up on its titles instead of stepping in and out by a tick's width.
                let text = match checked {
                    Some(true) => format!("{COMPOSED_CHECK_ON}{label}"),
                    Some(false) => format!("{COMPOSED_CHECK_OFF}{label}"),
                    None => label.clone(),
                };
                // The app's own name where it gave one, so a dayscript `tap` addresses the row it
                // means; the positional fallback keeps every other row reachable.
                let element_id = id.clone().unwrap_or_else(|| format!("day-menu-item-{n}"));
                let base = crate::label(text)
                    .padding(day_spec::Insets::symmetric(12.0 + indent, 5.0))
                    .width(COMPOSED_MENU_W);
                let row = if *enabled && *action != 0 {
                    let action = *action;
                    base.background(day_spec::Color::rgba(0.0, 0.0, 0.0, 0.0))
                        .on_tap(move || {
                            open.set(None);
                            day_core::dispatch_menu_action(action);
                        })
                        .id(element_id)
                } else {
                    base.opacity(0.45).id(element_id)
                };
                out.push(day_core::AnyPiece::new(row));
            }
        }
    }
}

/// Places the catcher over the full window and the panel at the summon point, pulled back
/// inside the bounds when the summon point sits too close to an edge.
struct MenuPlace {
    at: day_spec::Point,
}

impl day_core::Layout for MenuPlace {
    fn measure(
        &self,
        _cx: &mut dyn day_core::LayoutOps,
        _children: &[day_core::RNode],
        p: day_spec::Proposal,
    ) -> day_spec::Size {
        day_spec::Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0))
    }
    fn place(
        &self,
        cx: &mut dyn day_core::LayoutOps,
        children: &[day_core::RNode],
        bounds: day_spec::Rect,
    ) {
        if let Some(&c) = children.first() {
            let _ = cx.measure_child(c, day_spec::Proposal::exact(bounds.size));
            cx.place_child(c, day_spec::Rect::from_size(bounds.size));
        }
        if let Some(&c) = children.get(1) {
            let s = cx.measure_child(c, day_spec::Proposal::new(Some(COMPOSED_MENU_W), None));
            // Pull the panel back inside the window when the summon point sits near an edge,
            // unless bounds are degenerate (a pass before the backend reported the size).
            let (x, y) = if bounds.size.width > 1.0 && bounds.size.height > 1.0 {
                (
                    self.at.x.min(bounds.size.width - s.width).max(0.0),
                    self.at.y.min(bounds.size.height - s.height).max(0.0),
                )
            } else {
                (self.at.x, self.at.y)
            };
            cx.place_child(c, day_spec::Rect::new(x, y, s.width, s.height));
        }
    }
}

/// The [`MenuPlace`] wrapper piece: catcher first, panel second.
struct ComposedMenuHost {
    at: day_spec::Point,
    catcher: day_core::AnyPiece,
    panel: day_core::AnyPiece,
}

impl day_core::Piece for ComposedMenuHost {
    fn build(self, cx: &mut day_core::BuildCx) -> day_core::RNode {
        let n = cx.layout_only(
            Rc::new(MenuPlace { at: self.at }),
            day_core::Flex {
                grow_w: true,
                grow_h: true,
                ..Default::default()
            },
            day_core::Boundary::No,
        );
        cx.under(n, |cx| {
            let _ = self.catcher.build(cx);
            let _ = self.panel.build(cx);
        });
        n
    }
}

#[cfg(feature = "conformance")]
pub(crate) mod conformance {
    use day_core::conformance::{Case, Drive};
    use day_reactive::Signal;

    use crate::*;

    /// Choosing a context-menu item runs that item's action, and only that one.
    #[day_macros::test(day_core)]
    fn context_menu_runs_action() -> Case {
        Case::new()
            .proves_modifier("context_menu")
            .proves_duty("set_context_menu")
            .page(|| {
                let last = Signal::new(String::from("none"));
                column((
                    label(move || format!("ran {}", last.get())).id("ran"),
                    label("Target")
                        .padding(8.0)
                        .context_menu(vec![
                            menu_item("Copy it").action(move || last.set("copy".into())),
                            menu_item("Paste it").action(move || last.set("paste".into())),
                        ])
                        .id("target"),
                ))
            })
            .drive(|d: Drive| async move {
                d.context_menu("target", "Paste it").await?;
                d.assert_text("ran", "ran paste").await?;
                d.context_menu("target", "Copy it").await?;
                d.assert_text("ran", "ran copy").await
            })
    }

    /// A menu built on summon reflects the state at that moment.
    #[day_macros::test(day_core)]
    fn context_menu_fn_builds_on_summon() -> Case {
        Case::new()
            .proves_modifier("context_menu")
            .proves_duty("set_context_menu_fn")
            .page(|| {
                let count = Signal::new(0i64);
                let chose = Signal::new(0i64);
                column((
                    label(move || format!("chose {}", chose.get())).id("chose"),
                    button("Add")
                        .action(move || count.update(|n| *n += 1))
                        .id("add"),
                    label("Target")
                        .padding(8.0)
                        .context_menu(move || {
                            let n = count.get_untracked();
                            vec![menu_item(format!("Take {n}")).action(move || chose.set(n))]
                        })
                        .id("target"),
                ))
            })
            .drive(|d: Drive| async move {
                d.tap("add").await?;
                d.tap("add").await?;
                d.context_menu("target", "Take 2").await?;
                d.assert_text("chose", "chose 2").await
            })
    }

    day_core::tests! {
        context_menu_runs_action,
        context_menu_fn_builds_on_summon,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The role items every Edit menu carries take the standard accelerators when the app
    /// sets none, which is what gives GTK/Qt/web the shortcuts AppKit's native items always had.
    #[test]
    fn role_items_take_standard_shortcuts() {
        use day_spec::{MenuItem, MenuRole};
        let lowered = lower_menu(vec![
            menu_role(MenuRole::Undo),
            menu_role(MenuRole::Redo),
            menu_role(MenuRole::Cut),
            menu_role(MenuRole::Copy),
            menu_role(MenuRole::Paste),
            menu_role(MenuRole::SelectAll),
        ]);
        let expect = [
            ("z", false),
            ("z", true),
            ("x", false),
            ("c", false),
            ("v", false),
            ("a", false),
        ];
        for (item, (key, shift)) in lowered.iter().zip(expect) {
            let MenuItem::Action {
                shortcut, action, ..
            } = item
            else {
                panic!("role item lowered to a non-action");
            };
            let sc = shortcut.as_ref().expect("role item has a default shortcut");
            assert_eq!(sc.key, key);
            assert!(sc.primary);
            assert_eq!(sc.shift, shift);
            assert_ne!(*action, 0, "role item carries its standing dispatcher");
        }
    }
}
