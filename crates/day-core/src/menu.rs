//! Menu action dispatch (§ menus). The MODEL ([`day_spec::MenuItem`]) is toolkit-neutral and carries
//! only ids for its actions; the real closures live here, keyed by id. A backend fires
//! `Event::MenuAction(id)` when a native item is chosen; the event pump routes it to
//! [`dispatch_menu_action`], which runs the app's closure. Ids are process-unique and monotonic.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    static ACTIONS: RefCell<HashMap<u64, Rc<dyn Fn()>>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
    /// The app menu as last installed (post-injection) — the dayscript `menu:` step
    /// matches against it, and late `register_preferences` re-forwards it.
    static APP_MENU_MODEL: RefCell<Vec<day_spec::MenuItem>> = const { RefCell::new(Vec::new()) };
    /// The action ids the current app menu registered, so replacing the menu (the reactive
    /// re-lower on a locale change) can drop the stale closures WITHOUT touching context
    /// menus, which share the `ACTIONS` map.
    static APP_MENU_IDS: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

/// Register an app closure for a menu item and return its dispatch id (nonzero). The `day-pieces`
/// menu builder calls this while lowering a menu tree to the [`day_spec::MenuItem`] model.
pub fn register_menu_action(f: Rc<dyn Fn()>) -> u64 {
    let id = NEXT_ID.with(|c| {
        let id = c.get();
        c.set(id.wrapping_add(1).max(1));
        id
    });
    ACTIONS.with(|m| m.borrow_mut().insert(id, f));
    id
}

/// Run the closure registered for `id` (no-op if none). Called by the event pump on
/// `Event::MenuAction`. Runs inside a reactive batch so multiple signal writes coalesce.
pub fn dispatch_menu_action(id: u64) {
    let f = ACTIONS.with(|m| m.borrow().get(&id).cloned());
    if let Some(f) = f {
        day_reactive::batch(|| f());
    }
}

/// Set the application menu (menu bar / app-bar overflow / iPad main menu). Retains the
/// model (post-injection — [`app_menu_model`]), injects the auto Preferences item when one
/// is registered (docs/windows.md), drops the PREVIOUS app menu's action closures (context
/// menus share the map and are untouched), and forwards to the backend.
pub fn set_app_menu(items: Vec<day_spec::MenuItem>) {
    let items = inject_preferences(items);
    let new_ids = collect_action_ids(&items);
    // The prefs/new-window dispatch ids are registered by `day::register_*` (not by menu
    // lowering) and outlive any menu install — never sweep them.
    let durable = [
        crate::windows::preferences_action_id(),
        crate::windows::new_window_action_id(),
    ];
    let stale: Vec<u64> = APP_MENU_IDS.with(|ids| {
        ids.borrow()
            .iter()
            .copied()
            .filter(|id| !new_ids.contains(id) && !durable.contains(id))
            .collect()
    });
    ACTIONS.with(|m| {
        let mut m = m.borrow_mut();
        for id in stale {
            m.remove(&id);
        }
    });
    APP_MENU_IDS.with(|ids| *ids.borrow_mut() = new_ids);
    APP_MENU_MODEL.with(|m| *m.borrow_mut() = items.clone());
    crate::with_tree(|t| t.set_app_menu(items));
}

/// The app menu as last installed (post-injection). The dayscript `menu:` step walks it to
/// resolve an item's dispatch id; backends re-read it on late preferences registration.
pub fn app_menu_model() -> Vec<day_spec::MenuItem> {
    APP_MENU_MODEL.with(|m| m.borrow().clone())
}

/// Whether any app menu was installed (an empty model = the backend default menu is up).
pub fn app_menu_installed() -> bool {
    APP_MENU_MODEL.with(|m| !m.borrow().is_empty())
}

/// Re-forward the retained model through the injection pass — the self-heal for
/// `register_preferences` running AFTER `app_menu` (docs/menus.md ordering note).
pub(crate) fn reinstall_app_menu() {
    let items = APP_MENU_MODEL.with(|m| m.borrow().clone());
    if !items.is_empty() {
        set_app_menu(items);
    }
}

/// Inject the auto Settings…/Preferences item (docs/windows.md): when a preferences piece
/// is registered and the model carries an INERT `role(Preferences)` item (id 0), rewrite
/// its id to the registered action; when the model has no Preferences item at all, append
/// `separator + item` to the FIRST top-level submenu (the File menu by convention). An
/// app-supplied `.action` (nonzero id) always wins. Backends give the item its platform
/// placement (macOS hoists it into the App menu) and label fallback.
fn inject_preferences(mut items: Vec<day_spec::MenuItem>) -> Vec<day_spec::MenuItem> {
    let prefs_id = crate::windows::preferences_action_id();
    if prefs_id == 0 {
        return items;
    }
    fn rewrite(items: &mut [day_spec::MenuItem], prefs_id: u64) -> bool {
        use day_spec::MenuItem as M;
        for it in items.iter_mut() {
            match it {
                M::Action { id, role, .. } if *role == Some(day_spec::MenuRole::Preferences) => {
                    if *id == 0 {
                        *id = prefs_id;
                    }
                    return true;
                }
                M::Submenu { items, .. } => {
                    if rewrite(items, prefs_id) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
    if rewrite(&mut items, prefs_id) {
        return items;
    }
    let item = day_spec::MenuItem::Action {
        id: prefs_id,
        // Empty label: each backend falls back to its localized role label
        // ("Settings…" on macOS, "Preferences" elsewhere).
        label: String::new(),
        shortcut: Some(day_spec::Shortcut::new(",")),
        enabled: true,
        role: Some(day_spec::MenuRole::Preferences),
    };
    if let Some(day_spec::MenuItem::Submenu { items: first, .. }) = items
        .iter_mut()
        .find(|it| matches!(it, day_spec::MenuItem::Submenu { .. }))
    {
        first.push(day_spec::MenuItem::Separator);
        first.push(item);
    } else {
        items.push(item);
    }
    items
}

fn collect_action_ids(items: &[day_spec::MenuItem]) -> Vec<u64> {
    let mut ids = Vec::new();
    fn walk(items: &[day_spec::MenuItem], ids: &mut Vec<u64>) {
        for it in items {
            match it {
                day_spec::MenuItem::Action { id, .. } if *id != 0 => ids.push(*id),
                day_spec::MenuItem::Submenu { items, .. } => walk(items, ids),
                _ => {}
            }
        }
    }
    walk(items, &mut ids);
    ids
}

/// Arrange an app's top-level menus into the platform's standard bar, filling every standard
/// slot the app did not claim with the backend's stock menu.
///
/// Which desktop's menu-bar conventions a backend follows. A toolkit picks the style of the
/// platform it is native to, not the one it happens to be compiled on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MenuBarStyle {
    /// macOS: an app menu, and a Window menu the toolkit installs natively.
    Macos,
    /// GNOME / GTK on Linux.
    Gnome,
    /// KDE Plasma / Qt on Linux.
    Kde,
    /// Windows.
    Windows,
}

impl MenuBarStyle {
    /// The standard slots before and after the app's own menus on this desktop.
    pub fn bar_order(
        self,
    ) -> (
        &'static [day_spec::MenuBarRole],
        &'static [day_spec::MenuBarRole],
    ) {
        use day_spec::MenuBarRole as B;
        const LEADING: &[day_spec::MenuBarRole] = &[B::File, B::Edit, B::View];
        match self {
            // Only macOS has a Window menu; the Linux and Windows shells own window management.
            MenuBarStyle::Macos => (LEADING, &[B::Window, B::Help]),
            MenuBarStyle::Gnome | MenuBarStyle::Windows => (LEADING, &[B::Help]),
            MenuBarStyle::Kde => (LEADING, &[B::Settings, B::Help]),
        }
    }

    /// This desktop's stock menu for a slot, as a pure model: every entry is a `MenuRole`, so
    /// each backend renders it with its own native command and localized label and nothing here
    /// is toolkit- or app-specific.
    ///
    /// `File` is always the app's own. `Window` is `None` even on macOS because the toolkit
    /// installs it natively (only AppKit can append the live window list). `Settings` is `None`
    /// for now everywhere: Preferences is placed before a backend sees the model
    /// (`inject_preferences`), so a stock Settings menu would be a second copy of it.
    pub fn stock(self, role: day_spec::MenuBarRole) -> Option<day_spec::MenuItem> {
        use day_spec::{MenuBarRole as B, MenuItem as MI, MenuRole as R};
        let act = |r: R| MI::Action {
            id: 0,
            label: String::new(),
            shortcut: None,
            enabled: true,
            role: Some(r),
        };
        let sub = |key: &str, items: Vec<MI>| {
            Some(MI::Submenu {
                label: day_l10n::t(key),
                items,
                role: None,
            })
        };
        match role {
            B::Edit => sub(
                "day-edit",
                vec![
                    act(R::Undo),
                    act(R::Redo),
                    MI::Separator,
                    act(R::Cut),
                    act(R::Copy),
                    act(R::Paste),
                    act(R::Delete),
                    act(R::SelectAll),
                ],
            ),
            B::View => sub("day-view", vec![act(R::Fullscreen)]),
            // macOS keeps About in the app menu; the other desktops keep it in Help. An empty
            // Help menu still earns its place on macOS — AppKit fills it with the help search.
            B::Help => match self {
                MenuBarStyle::Macos => sub("day-help", Vec::new()),
                _ => sub("day-help", vec![act(R::About)]),
            },
            _ => None,
        }
    }
}

/// Assemble the bar for a desktop's conventions — [`standard_menu_bar`] with that style's slot
/// order and stock menus.
pub fn standard_menu_bar_for(
    style: MenuBarStyle,
    app_menus: Vec<day_spec::MenuItem>,
) -> Vec<day_spec::MenuItem> {
    let (leading, trailing) = style.bar_order();
    standard_menu_bar(app_menus, leading, trailing, |r| style.stock(r))
}

/// `leading` are the slots before the app's own menus and `trailing` the ones after — each
/// backend passes its platform's bar (macOS trails Window and Help; KDE trails Settings and
/// Help; GNOME and Windows trail Help alone). `stock` returns the backend's house version of a
/// slot, or `None` where that platform has no such menu. Menus the app tagged with
/// [`MenuBarRole`] replace the stock one IN PLACE, so an app customizes a standard menu by
/// claiming it rather than by rebuilding the whole bar.
pub fn standard_menu_bar(
    app_menus: Vec<day_spec::MenuItem>,
    leading: &[day_spec::MenuBarRole],
    trailing: &[day_spec::MenuBarRole],
    stock: impl Fn(day_spec::MenuBarRole) -> Option<day_spec::MenuItem>,
) -> Vec<day_spec::MenuItem> {
    use day_spec::MenuBarRole as R;
    let _ = R::File;

    let mut claimed: Vec<(R, day_spec::MenuItem)> = Vec::new();
    let mut own: Vec<day_spec::MenuItem> = Vec::new();
    for item in app_menus {
        match &item {
            day_spec::MenuItem::Submenu { role: Some(r), .. } => claimed.push((*r, item)),
            _ => own.push(item),
        }
    }
    let mut take = |r: R| -> Option<day_spec::MenuItem> {
        claimed
            .iter()
            .position(|(cr, _)| *cr == r)
            .map(|i| claimed.remove(i).1)
            .or_else(|| stock(r))
    };

    let mut out: Vec<day_spec::MenuItem> = leading.iter().filter_map(|r| take(*r)).collect();
    out.append(&mut own);
    out.extend(trailing.iter().filter_map(|r| take(*r)));
    // A role the platform does not know about still belongs on the bar rather than vanishing.
    out.extend(claimed.into_iter().map(|(_, m)| m));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    use day_spec::MenuBarRole as BR;
    const MAC_LEADING: &[BR] = &[BR::File, BR::Edit, BR::View];
    const MAC_TRAILING: &[BR] = &[BR::Window, BR::Help];

    fn sub(label: &str, role: Option<day_spec::MenuBarRole>) -> day_spec::MenuItem {
        day_spec::MenuItem::Submenu {
            label: label.into(),
            items: Vec::new(),
            role,
        }
    }

    fn labels(items: &[day_spec::MenuItem]) -> Vec<String> {
        items
            .iter()
            .map(|i| match i {
                day_spec::MenuItem::Submenu { label, .. } => label.clone(),
                _ => String::new(),
            })
            .collect()
    }

    /// Every stock slot appears, in bar order, with the app's own menus between View and Window.
    #[test]
    fn stock_menus_fill_the_slots_an_app_left_open() {
        use day_spec::MenuBarRole as R;
        let out = standard_menu_bar(
            vec![sub("Go", None), sub("Article", None)],
            MAC_LEADING,
            MAC_TRAILING,
            |r| {
                Some(sub(
                    match r {
                        R::File => "File",
                        R::Edit => "Edit",
                        R::View => "View",
                        R::Window => "Window",
                        R::Help => "Help",
                        _ => "?",
                    },
                    Some(r),
                ))
            },
        );
        assert_eq!(
            labels(&out),
            ["File", "Edit", "View", "Go", "Article", "Window", "Help"]
        );
    }

    /// A claimed slot replaces the stock menu in place — it does NOT also get the stock one,
    /// and it does not slide into the app-menu run.
    #[test]
    fn a_claimed_slot_replaces_the_stock_menu_in_place() {
        use day_spec::MenuBarRole as R;
        let out = standard_menu_bar(
            vec![sub("My File", Some(R::File)), sub("Go", None)],
            MAC_LEADING,
            MAC_TRAILING,
            |r| Some(sub("stock", Some(r))),
        );
        let l = labels(&out);
        assert_eq!(l[0], "My File", "the app's File sits in the File slot");
        assert_eq!(l.iter().filter(|s| *s == "My File").count(), 1);
        assert!(!l.contains(&"File".to_string()));
        assert_eq!(l[3], "Go", "app menus still follow the leading slots");
    }

    /// The bar order is the BACKEND's, not one hardcoded shape: KDE trails Settings and Help,
    /// GNOME and Windows trail Help alone, and neither grows a macOS Window menu.
    #[test]
    fn each_platform_gets_its_own_bar_order() {
        use day_spec::MenuBarRole as R;
        // File is the app's own everywhere — no backend ships a stock one.
        let stock = |r: R| {
            let label = match r {
                R::Edit => "Edit",
                R::View => "View",
                R::Settings => "Settings",
                R::Help => "Help",
                R::Window => "Window",
                _ => return None,
            };
            Some(sub(label, Some(r)))
        };
        let kde = standard_menu_bar(
            vec![sub("Go", None)],
            &[R::File, R::Edit, R::View],
            &[R::Settings, R::Help],
            stock,
        );
        assert_eq!(labels(&kde), ["Edit", "View", "Go", "Settings", "Help"]);

        let gnome = standard_menu_bar(
            vec![sub("Go", None)],
            &[R::File, R::Edit, R::View],
            &[R::Help],
            stock,
        );
        assert_eq!(labels(&gnome), ["Edit", "View", "Go", "Help"]);
        assert!(!labels(&gnome).contains(&"Window".to_string()));
    }

    /// A backend with no stock menu for a slot simply has no such menu.
    #[test]
    fn a_slot_with_no_stock_menu_is_omitted() {
        use day_spec::MenuBarRole as R;
        let out = standard_menu_bar(vec![sub("Go", None)], MAC_LEADING, MAC_TRAILING, |r| {
            (r == R::Edit).then(|| sub("Edit", Some(r)))
        });
        assert_eq!(labels(&out), ["Edit", "Go"]);
    }

    #[test]
    fn dispatch_runs_the_registered_action_by_id() {
        thread_local! { static FIRED: Cell<u32> = const { Cell::new(0) }; }
        let id = register_menu_action(Rc::new(|| FIRED.with(|c| c.set(c.get() + 1))));
        assert_ne!(
            id, 0,
            "ids are nonzero so role-only items (id 0) never dispatch"
        );
        assert_eq!(FIRED.with(Cell::get), 0);

        dispatch_menu_action(id);
        assert_eq!(FIRED.with(Cell::get), 1, "the closure ran exactly once");

        // A second, distinct action gets a distinct id and doesn't fire the first.
        let id2 = register_menu_action(Rc::new(|| {}));
        assert_ne!(id, id2);

        // Unknown / zero ids are silent no-ops (role items, stale ids).
        dispatch_menu_action(0);
        dispatch_menu_action(u64::MAX);
        assert_eq!(FIRED.with(Cell::get), 1);
    }
}
