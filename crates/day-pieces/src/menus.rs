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
    action: Option<Rc<dyn Fn()>>,
    children: Option<Vec<MenuEntry>>,
    separator: bool,
}

impl MenuEntry {
    fn command(label: impl Into<String>) -> MenuEntry {
        MenuEntry {
            label: label.into(),
            shortcut: None,
            enabled: true,
            role: None,
            action: None,
            children: None,
            separator: false,
        }
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
    }
}

/// Lower app-side entries to the spec model, registering action closures with day-core. A standard
/// `role` item with no explicit label gets its label from the localized core catalog here — so the
/// backends receive a ready, locale-correct label instead of each hardcoding English (day-l10n).
pub(crate) fn lower_menu(entries: Vec<MenuEntry>) -> Vec<day_spec::MenuItem> {
    entries
        .into_iter()
        .map(|e| {
            if e.separator {
                day_spec::MenuItem::Separator
            } else if let Some(children) = e.children {
                day_spec::MenuItem::Submenu {
                    label: e.label,
                    items: lower_menu(children),
                }
            } else {
                let id = e.action.map(day_core::register_menu_action).unwrap_or(0);
                let label = match (e.label.is_empty(), e.role) {
                    (true, Some(role)) => day_l10n::t(role_catalog_key(role)),
                    _ => e.label,
                };
                day_spec::MenuItem::Action {
                    id,
                    label,
                    shortcut: e.shortcut,
                    enabled: e.enabled,
                    role: e.role,
                }
            }
        })
        .collect()
}

/// Install the application menu — the native menu bar on desktop, the app-bar overflow on Android, the
/// UIMenuBuilder main menu on iPadOS/Catalyst. Top-level entries are usually `sub_menu(...)`s (the
/// menu-bar menus). Call at startup or whenever the menu changes; it replaces any previous app menu.
pub fn app_menu(menus: Vec<MenuEntry>) {
    day_core::set_app_menu(lower_menu(menus));
}
