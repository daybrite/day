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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

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
