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

/// Set the application menu (menu bar / app-bar overflow / iPad main menu). Forwards to the backend.
pub fn set_app_menu(items: Vec<day_spec::MenuItem>) {
    crate::with_tree(|t| t.set_app_menu(items));
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
