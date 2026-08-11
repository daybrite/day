// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Window toolbars (docs/toolbars.md). The MODEL ([`day_spec::ToolbarItem`]) is toolkit-neutral
//! and carries only ids for its commands; the real closures live here, keyed by id — the same
//! shape as [`crate::menu`], and deliberately the same id space, so one closure can back both a
//! toolbar button and its menu-bar twin.
//!
//! A toolbar belongs to a WINDOW, not to the app: each window root keeps its own model, and the
//! primary window is just the root every install falls back to. During a secondary window's
//! content build [`with_window`] names that window, so an app's one `toolbar(...)` call inside a
//! shared `build_shell` gives every window its own bar without the app tracking any of it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use day_spec::{ToolbarItem, ToolbarPatch, ToolbarValue};

use crate::tree::{RNode, with_tree};

/// A toolbar item's value callback — what a search field's text or a toggle's state runs.
type ValueAction = Rc<dyn Fn(&ToolbarValue)>;

thread_local! {
    /// Value callbacks (search text, toggle state) by dispatch id. Plain buttons don't appear
    /// here — they register with [`crate::menu::register_menu_action`] and arrive as
    /// `Event::MenuAction`.
    static VALUE_ACTIONS: RefCell<HashMap<u64, ValueAction>> = RefCell::new(HashMap::new());
    /// Each window's toolbar as last installed — dayscript resolves an item's action here, and
    /// a replace diffs against it to drop the closures the old model owned.
    static MODELS: RefCell<Vec<(RNode, Vec<ToolbarItem>)>> = const { RefCell::new(Vec::new()) };
    /// The window whose content is being built right now (see [`with_window`]).
    static BUILDING: Cell<Option<RNode>> = const { Cell::new(None) };
    /// Per-window search field from a `.searchable()` surface, merged into every toolbar install
    /// (see [`set_window_search`]).
    static SEARCH_ITEMS: RefCell<Vec<(RNode, ToolbarItem)>> = const { RefCell::new(Vec::new()) };
}

/// Register a value callback for a search or toggle item and return its dispatch id (nonzero).
/// The id comes from the menu action counter, so toolbar and menu ids never collide.
pub fn register_toolbar_value(f: Rc<dyn Fn(&ToolbarValue)>) -> u64 {
    let id = crate::menu::next_action_id();
    VALUE_ACTIONS.with(|m| m.borrow_mut().insert(id, f));
    id
}

/// Run the value callback registered for `action` (no-op if none). Called by the event pump on
/// `Event::ToolbarChanged`, inside a reactive batch so multiple signal writes coalesce.
pub fn dispatch_toolbar_value(action: u64, value: &ToolbarValue) {
    let f = VALUE_ACTIONS.with(|m| m.borrow().get(&action).cloned());
    if let Some(f) = f {
        day_reactive::batch(|| f(value));
    }
}

/// Run `f` with `root` as the window every [`set_toolbar`] call inside it targets. day-core wraps
/// each window's content build in this; nesting restores the previous window on the way out.
pub(crate) fn with_window<R>(root: RNode, f: impl FnOnce() -> R) -> R {
    let prev = BUILDING.replace(Some(root));
    let out = f();
    BUILDING.set(prev);
    out
}

/// The window a toolbar call targets: the one being built, else the primary root.
///
/// Public because the reactive installer must CAPTURE it (day-pieces `toolbar_reactive`). Its
/// effect re-runs long after the build that created it, when `BUILDING` is unset again — and the
/// fallback here is the primary window, so a second window's rebuilt toolbar would replace the
/// PRIMARY's. Reading it once, at install time, is what keeps a toolbar with its own window.
pub fn current_window() -> RNode {
    target_window()
}

fn target_window() -> RNode {
    BUILDING
        .get()
        .unwrap_or_else(|| with_tree(|t| t.root_node()))
}

/// Install the toolbar on the window currently being built (the primary window outside a window
/// build). Replaces any previous toolbar on that window; an empty `items` removes it.
pub fn set_toolbar(items: Vec<ToolbarItem>) {
    set_window_toolbar(target_window(), items);
}

/// [`set_toolbar`] against an explicit window root.
pub fn set_window_toolbar(root: RNode, items: Vec<ToolbarItem>) {
    let items = merge_search(root, items);
    sweep_values(root, &items);
    MODELS.with(|m| {
        let mut m = m.borrow_mut();
        match m.iter_mut().find(|(r, _)| *r == root) {
            Some(entry) => entry.1 = items.clone(),
            None => m.push((root, items.clone())),
        }
    });
    with_tree(|t| t.set_window_toolbar(root, items));
}

/// Apply a targeted item update to the current window's toolbar — the path a bound signal writes
/// through, so a search field keeps its focus and its insertion point.
pub fn patch_toolbar(patch: ToolbarPatch) {
    patch_window_toolbar(target_window(), patch);
}

/// [`patch_toolbar`] against an explicit window root. Also updates the retained model, so a later
/// full replace does not resurrect the stale value.
pub fn patch_window_toolbar(root: RNode, patch: ToolbarPatch) {
    MODELS.with(|m| {
        let mut m = m.borrow_mut();
        if let Some((_, items)) = m.iter_mut().find(|(r, _)| *r == root) {
            apply_to_model(items, &patch);
        }
    });
    with_tree(|t| t.patch_window_toolbar(root, patch));
}

/// Mirror a patch into the retained model.
fn apply_to_model(items: &mut [ToolbarItem], patch: &ToolbarPatch) {
    use day_spec::ToolbarItemKind as K;
    match patch {
        ToolbarPatch::Text { item, text } => {
            if let Some(it) = items.iter_mut().find(|i| i.id == *item)
                && let K::Search { text: t, .. } = &mut it.kind
            {
                *t = text.clone();
            }
        }
        ToolbarPatch::On { item, on } => {
            if let Some(it) = items.iter_mut().find(|i| i.id == *item)
                && let K::Toggle { on: o } = &mut it.kind
            {
                *o = *on;
            }
        }
        ToolbarPatch::Enabled { item, on } => {
            if let Some(it) = items.iter_mut().find(|i| i.id == *item) {
                it.enabled = *on;
            }
        }
        ToolbarPatch::Suggestions { item, list } => {
            if let Some(it) = items.iter_mut().find(|i| i.id == *item)
                && let K::Search { suggestions, .. } = &mut it.kind
            {
                *suggestions = list.clone();
            }
        }
    }
}

/// The window's toolbar as last installed — dayscript's `toolbar:` step walks it to resolve an
/// item's dispatch id.
pub fn toolbar_model(root: RNode) -> Vec<ToolbarItem> {
    MODELS.with(|m| {
        m.borrow()
            .iter()
            .find(|(r, _)| *r == root)
            .map(|(_, items)| items.clone())
            .unwrap_or_default()
    })
}

/// The primary window's toolbar model.
pub fn primary_toolbar_model() -> Vec<ToolbarItem> {
    toolbar_model(with_tree(|t| t.root_node()))
}

/// A `.searchable()` surface's field, when its placement resolves to the window toolbar
/// (docs/search.md). Kept OUTSIDE the app's model and merged into every install, because the two
/// are written at different times by different owners: the app installs its bar before the tree
/// builds, and `toolbar_reactive` re-installs the whole model on any reactive change — an item
/// injected once would be dropped by the next rebuild.
///
/// Trailing, after the app's own items, which is where every desktop puts search.
pub fn set_window_search(root: RNode, item: Option<ToolbarItem>) {
    SEARCH_ITEMS.with(|m| {
        let mut m = m.borrow_mut();
        m.retain(|(r, _)| *r != root);
        if let Some(item) = item {
            m.push((root, item));
        }
    });
    // Re-install so the change lands now: the surface registers during the tree build, after the
    // app's `toolbar(…)` call has already gone through.
    let current = MODELS.with(|m| {
        m.borrow()
            .iter()
            .find(|(r, _)| *r == root)
            .map(|(_, items)| items.clone())
    });
    if let Some(items) = current {
        set_window_toolbar(root, items);
    }
}

/// Keep the STORED search item's live state current, without re-installing the bar.
///
/// [`merge_search`] clones this item into every toolbar install, so the stored snapshot is what a
/// REBUILD re-seeds the field from — and the bar rebuilds for reasons that have nothing to do with
/// search (any other item re-lowering, a route change, a language change). Left stale, the field
/// silently reverted to whatever it held when `.searchable()` was installed while the query signal
/// kept the real value: an empty box that is still filtering, with no way to clear it.
///
/// Deliberately does NOT re-install: this runs on every keystroke, and rebuilding the bar under
/// the caret is the very thing the targeted patch path exists to avoid.
pub fn set_window_search_state(root: RNode, text: Option<&str>, suggestions: Option<&[String]>) {
    SEARCH_ITEMS.with(|m| {
        if let Some((_, item)) = m.borrow_mut().iter_mut().find(|(r, _)| *r == root)
            && let day_spec::ToolbarItemKind::Search {
                text: t,
                suggestions: s,
                ..
            } = &mut item.kind
        {
            if let Some(v) = text {
                *t = v.to_string();
            }
            if let Some(v) = suggestions {
                *s = v.to_vec();
            }
        }
    });
}

/// The app's items plus this window's search field, if it has one.
///
/// The merged model IS what gets stored, so dayscript's `toolbar:` step can resolve the field the
/// same way it resolves any other item. Idempotent through the `retain`: merging an
/// already-merged model replaces the item rather than appending a second one.
fn merge_search(root: RNode, mut items: Vec<ToolbarItem>) -> Vec<ToolbarItem> {
    if let Some(item) = SEARCH_ITEMS.with(|m| {
        m.borrow()
            .iter()
            .find(|(r, _)| *r == root)
            .map(|(_, i)| i.clone())
    }) {
        items.retain(|i| i.id != item.id);
        items.push(item);
    }
    items
}

/// Show/hide the window's `selector(Sidebar)` pane — the behaviour behind a
/// [`day_spec::ToolbarItemKind::SidebarToggle`] item. `false` when this toolkit has no split
/// host to toggle. The native toolbar button and dayscript's `toolbar:` step share this call,
/// so a walkthrough drives the same path a click does (docs/toolbars.md).
pub fn toggle_sidebar() -> bool {
    with_tree(|t| t.toggle_sidebar())
}

/// Drop a closed window's toolbar and the value closures only it owned.
pub(crate) fn forget_window(root: RNode) {
    SEARCH_ITEMS.with(|m| m.borrow_mut().retain(|(r, _)| *r != root));
    let gone = MODELS.with(|m| {
        let mut m = m.borrow_mut();
        m.iter()
            .position(|(r, _)| *r == root)
            .map(|i| m.remove(i).1)
            .unwrap_or_default()
    });
    drop_values(&gone, &[]);
}

/// Forget the value closures the previous model owned and the new one does not — the same
/// discipline `set_app_menu` applies to menu actions, so a toolbar rebuilt on every locale change
/// does not leak a closure per install.
fn sweep_values(root: RNode, next: &[ToolbarItem]) {
    let prev = MODELS.with(|m| {
        m.borrow()
            .iter()
            .find(|(r, _)| *r == root)
            .map(|(_, items)| items.clone())
            .unwrap_or_default()
    });
    drop_values(&prev, next);
}

fn drop_values(prev: &[ToolbarItem], next: &[ToolbarItem]) {
    let keep: Vec<u64> = next.iter().map(|i| i.action).collect();
    let stale: Vec<u64> = prev
        .iter()
        .map(|i| i.action)
        .filter(|a| *a != 0 && !keep.contains(a))
        .collect();
    if stale.is_empty() {
        return;
    }
    VALUE_ACTIONS.with(|m| {
        let mut m = m.borrow_mut();
        for a in stale {
            m.remove(&a);
        }
    });
}

/// Reset every window's toolbar state (tests — pairs with `uninstall_tree`).
pub fn reset_toolbars() {
    MODELS.with(|m| m.borrow_mut().clear());
    VALUE_ACTIONS.with(|m| m.borrow_mut().clear());
    BUILDING.set(None);
}
