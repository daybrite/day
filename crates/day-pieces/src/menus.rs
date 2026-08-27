// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The app menu builder: `menu_item`, `sub_menu`, `menu_separator`, and `menu_role`, assembled
//! with `app_menu`. Lowers to day_spec's toolkit-neutral `MenuItem` model and registers each
//! item's action closure.

use std::rc::Rc;

// ---------------------------------------------------------------------------
// Menus — the app-side builder over day_spec's toolkit-neutral MenuItem model. Lowering registers each
// item's action closure with day-core (which dispatches `Event::MenuAction`) and assigns its id.
// ---------------------------------------------------------------------------

/// A menu entry under construction. Build a command with [`menu_item`], a nested submenu with
/// [`sub_menu`], a standard system command with [`menu_role`], and a divider with [`menu_separator`].
/// Attach to a Piece via [`Decorate::context_menu`] or install app-wide via [`app_menu`].
pub struct MenuEntry {
    label: String,
    shortcut: Option<day_spec::Shortcut>,
    enabled: bool,
    role: Option<day_spec::MenuRole>,
    icon: Option<day_spec::Icon>,
    action: Option<Rc<dyn Fn()>>,
    children: Option<Vec<MenuEntry>>,
    separator: bool,
    bar_role: Option<day_spec::MenuBarRole>,
}

impl MenuEntry {
    fn command(label: impl Into<String>) -> MenuEntry {
        MenuEntry {
            label: label.into(),
            shortcut: None,
            enabled: true,
            role: None,
            icon: None,
            bar_role: None,
            action: None,
            children: None,
            separator: false,
        }
    }

    /// A standard [`Symbol`](day_spec::Symbol) beside the item's title, drawn with the
    /// platform's own glyph — the same vocabulary toolbars take. Menus carry icons on macOS,
    /// Windows, GNOME, KDE and Android; a backend whose menus are text-only ignores it, so an
    /// icon is always an addition to a menu that already reads correctly without one.
    pub fn icon(mut self, s: day_spec::Symbol) -> MenuEntry {
        self.icon = Some(day_spec::Icon::Symbol(s));
        self
    }

    /// A bundled image from `resource/images`, for an item the standard set has no glyph for —
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
    /// Full shortcut spec, e.g. `Shortcut::new("s").shift()`.
    pub fn shortcut(mut self, s: day_spec::Shortcut) -> MenuEntry {
        self.shortcut = Some(s);
        self
    }
    /// Convenience: the platform's primary modifier (⌘ / Ctrl) + `key`.
    pub fn key(mut self, key: impl Into<String>) -> MenuEntry {
        self.shortcut = Some(day_spec::Shortcut::new(key));
        self
    }
    pub fn enabled(mut self, on: bool) -> MenuEntry {
        self.enabled = on;
        self
    }
    /// Tag a custom command with a standard [`day_spec::MenuRole`] (usually you use [`menu_role`]).
    pub fn role(mut self, role: day_spec::MenuRole) -> MenuEntry {
        self.role = Some(role);
        self
    }
}

/// A clickable command: `menu_item("Save").key("s").action(|| …)`.
pub fn menu_item(label: impl Into<String>) -> MenuEntry {
    MenuEntry::command(label)
}

/// A nested submenu: `sub_menu("File", vec![menu_item("New"), …])`.
pub fn sub_menu(label: impl Into<String>, items: Vec<MenuEntry>) -> MenuEntry {
    MenuEntry {
        children: Some(items),
        ..MenuEntry::command(label)
    }
}

/// Claim one of the platform's standard menu-bar slots for this submenu: the backend places it
/// where that menu belongs and does NOT add its own stock version. Without a role, a submenu is
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
        ..MenuEntry::command("")
    }
}

/// A standard/system command (`MenuRole::Copy`, `MenuRole::Quit`, …) rendered with the platform's
/// NATIVE item — correct label, default shortcut, focus-targeting, and automatic enable/disable — so
/// default menu items (Edit ▸ Cut/Copy/Paste, the app's Quit/About) work without re-implementation.
pub fn menu_role(role: day_spec::MenuRole) -> MenuEntry {
    MenuEntry {
        role: Some(role),
        ..MenuEntry::command("")
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
    }
}

/// Lower app-side entries to the spec model, registering action closures with day-core. A standard
/// `role` item with no explicit label gets its label from the localized core catalog here — so the
/// backends receive a ready, locale-correct label instead of each hardcoding English (day-l10n).
///
/// This variant registers PROCESS-lived closures — correct for the app menu and toolbars, whose
/// ids day-core manages by shape-rebinding and explicit sweeps. Menus owned by a piece build
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
                    label: e.label,
                    items: lower_menu_with(children, register),
                    role: e.bar_role,
                }
            } else {
                let mut id = e.action.map(register).unwrap_or(0);
                let mut enabled = e.enabled;
                let mut shortcut = e.shortcut;
                // Window roles have no native selector on any platform: an action-less item
                // lowers to the registered day dispatcher (docs/windows.md) — live when the
                // app registered a builder/preferences piece, disabled otherwise.
                if id == 0 {
                    match e.role {
                        Some(day_spec::MenuRole::NewWindow) => {
                            id = day_core::windows::new_window_action_id();
                            enabled = enabled && id != 0;
                            shortcut = shortcut.or(Some(day_spec::Shortcut::new("n")));
                        }
                        // The undo pair and the clipboard trio lower to standing dispatchers
                        // too, for toolkits whose role items come back as plain menu actions
                        // (Android's app-bar menu, the iOS menu, web context menus). Toolkits
                        // with a native responder route ignore the id and keep their selector
                        // — see each backend's menu build. Each also takes the platform-neutral
                        // STANDARD shortcut (primary+Z/X/C/V/A, shift for redo) unless the app
                        // set its own — AppKit's native items already carry these, so this is
                        // what gives GTK/Qt/web the same accelerators.
                        Some(day_spec::MenuRole::Undo) => {
                            id = day_core::undo_action_id(false);
                            shortcut = shortcut.or(Some(day_spec::Shortcut::new("z")));
                        }
                        Some(day_spec::MenuRole::Redo) => {
                            id = day_core::undo_action_id(true);
                            shortcut = shortcut.or(Some(day_spec::Shortcut::new("z").shift()));
                        }
                        Some(day_spec::MenuRole::Cut) => {
                            id = day_core::edit_action_id(day_spec::EditOp::Cut);
                            shortcut = shortcut.or(Some(day_spec::Shortcut::new("x")));
                        }
                        Some(day_spec::MenuRole::Copy) => {
                            id = day_core::edit_action_id(day_spec::EditOp::Copy);
                            shortcut = shortcut.or(Some(day_spec::Shortcut::new("c")));
                        }
                        Some(day_spec::MenuRole::Paste) => {
                            id = day_core::edit_action_id(day_spec::EditOp::Paste);
                            shortcut = shortcut.or(Some(day_spec::Shortcut::new("v")));
                        }
                        Some(day_spec::MenuRole::SelectAll) => {
                            id = day_core::edit_action_id(day_spec::EditOp::SelectAll);
                            shortcut = shortcut.or(Some(day_spec::Shortcut::new("a")));
                        }
                        Some(day_spec::MenuRole::Preferences) => {
                            id = day_core::windows::preferences_action_id();
                            shortcut = shortcut.or(Some(day_spec::Shortcut::new(",")));
                        }
                        _ => {}
                    }
                }
                let label = match (e.label.is_empty(), e.role) {
                    (true, Some(role)) => day_l10n::t(role_catalog_key(role)),
                    _ => e.label,
                };
                day_spec::MenuItem::Action {
                    id,
                    label,
                    shortcut,
                    enabled,
                    role: e.role,
                    icon: e.icon,
                }
            }
        })
        .collect()
}

/// Install the application menu — the native menu bar on desktop, the app-bar overflow on Android, the
/// UIMenuBuilder main menu on iPadOS/Catalyst. Top-level entries are usually `sub_menu(...)`s (the
/// menu-bar menus). Call at startup or whenever the menu changes; it replaces any previous app menu.
///
/// Labels resolve ONCE, in the install-time locale; an app whose language can change at
/// runtime (a preferences language picker) should use [`app_menu_reactive`] instead.
pub fn app_menu(menus: Vec<MenuEntry>) {
    day_core::set_app_menu(lower_menu(menus));
}

/// [`app_menu`] that re-lowers and re-installs whenever a locale-tracked read inside the
/// builder changes — `menu_role` labels, `res::str` titles, and `day::tr` all read the
/// locale signal, so a runtime language switch rebuilds the menu in the new language
/// (docs/menus.md). Replacement drops the previous install's action closures (context
/// menus are unaffected). The binding lives in a root-owned scope: install once, at startup.
pub fn app_menu_reactive(builder: impl Fn() -> Vec<MenuEntry> + 'static) {
    let scope = day_reactive::Scope::root().enter(day_reactive::Scope::child);
    scope.enter(|| {
        day_reactive::bind(
            move || {
                // Track the locale even when the builder itself has no localized reads,
                // so role-label fallbacks still refresh.
                let _ = day_l10n::locale().get();
                lower_menu(builder())
            },
            |items: &Vec<day_spec::MenuItem>| {
                day_core::set_app_menu(items.clone());
            },
        );
    });
}

// ---------------------------------------------------------------------------
// The composed context menu (docs/menus.md "Dynamic context menus"): presentation for a
// toolkit that REPORTS the summon (`Event::ContextMenu`) instead of showing a native menu
// of its own (web-dom). Works on the LOWERED model, so role resolution, localization, and
// per-summon action scoping are exactly what a native backend would have received.
// ---------------------------------------------------------------------------

/// Panel width of the composed menu, and the extra indent inlined submenu items take.
const COMPOSED_MENU_W: f64 = 220.0;
const COMPOSED_SUBMENU_INDENT: f64 = 12.0;

/// Mount the composed presentation beside a `.context_menu`/`.context_menu_fn` node: an
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

    let panel = crate::column(day_core::PieceVec(rows))
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
/// children — the composed panel has no flyout (docs/menus.md).
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
                id, label, enabled, ..
            } => {
                let n = *next;
                *next += 1;
                let base = crate::label(label.clone())
                    .padding(day_spec::Insets::symmetric(12.0 + indent, 5.0))
                    .width(COMPOSED_MENU_W);
                let row = if *enabled && *id != 0 {
                    let id = *id;
                    base.background(day_spec::Color::rgba(0.0, 0.0, 0.0, 0.0))
                        .on_tap(move || {
                            open.set(None);
                            day_core::dispatch_menu_action(id);
                        })
                        .id(format!("day-menu-item-{n}"))
                } else {
                    base.opacity(0.45).id(format!("day-menu-item-{n}"))
                };
                out.push(day_core::AnyPiece::new(row));
            }
        }
    }
}

/// Places the catcher over the full window and the panel at the summon point, pulled back
/// inside the bounds when the point sits too close to an edge.
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
            // Pull the panel back inside the window when the summon point sits near an edge
            // — unless bounds are degenerate (a pass before the backend reported the size).
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The role items every Edit menu carries take the STANDARD accelerators when the app
    /// sets none — what gives GTK/Qt/web the shortcuts AppKit's native items always had.
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
            let MenuItem::Action { shortcut, id, .. } = item else {
                panic!("role item lowered to a non-action");
            };
            let sc = shortcut.as_ref().expect("role item has a default shortcut");
            assert_eq!(sc.key, key);
            assert!(sc.primary);
            assert_eq!(sc.shift, shift);
            assert_ne!(*id, 0, "role item carries its standing dispatcher");
        }
    }
}
