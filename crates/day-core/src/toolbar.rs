// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Toolbar contributions (docs/toolbars.md). The MODEL ([`day_spec::ToolbarItem`]) is
//! toolkit-neutral and carries only ids for its commands; the real closures live here, keyed by
//! id — the same shape as [`crate::menu`], and deliberately the same id space, so one closure can
//! back both a toolbar button and its menu-bar twin.
//!
//! Any piece can declare items. Where it sits decides which CHROME carries them — the window's
//! own, or one navigation page's — and its scope decides how long they stay: a contribution is
//! withdrawn when the piece that registered it is disposed, so a command leaves with the content
//! it acts on. Several pieces may contribute to one chrome; this module merges them in
//! registration order and hands the result to the toolkit as one model.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use day_spec::{ToolbarItem, ToolbarPatch, ToolbarValue};

use crate::tree::{RNode, with_tree};

/// A toolbar item's value callback — what a search field's text or a toggle's state runs.
type ValueAction = Rc<dyn Fn(&ToolbarValue)>;

/// Which chrome a contribution lands on.
///
/// Not a placement — [`day_spec::ToolbarPlacement`] says where on a chrome an item sits. This
/// says WHICH chrome, and it is never written by an app: it follows from the piece that declared
/// the items, which is the whole point of the design (docs/toolbars.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Chrome {
    /// The window's own chrome, shown on every page of that window.
    Window(RNode),
    /// One navigation page's chrome. Which surface draws it follows from the page's
    /// [`day_spec::props::Pane`].
    Page(RNode),
}

/// One navigation page being built: the node contributions land on, whether it is on screen, and
/// which column of the window it is.
struct PageFrame {
    page: RNode,
    active: Option<Rc<dyn Fn() -> bool>>,
    column: day_spec::ToolbarColumn,
    /// The window this page belongs to, captured when its HOST was built. A destination page is
    /// built lazily, on the first selection, long after its window's own build has returned —
    /// and `window_being_built()` then answers the PRIMARY window, so a second window's pages
    /// would contribute to the first one's bar.
    window: RNode,
}

/// One piece's declared items, alive as long as that piece is.
struct Contribution {
    chrome: Chrome,
    /// Registration order within its chrome. Stable across an update, so re-deriving a list does
    /// not move it past a neighbor that was declared later.
    seq: u64,
    items: Vec<ToolbarItem>,
    /// The window this contribution's chrome belongs to.
    window: RNode,
    /// Whether the page carrying it is ON SCREEN. One bar serves the window, so a pane that is
    /// collapsed or a destination that is not the one showing must not leave its commands on it
    /// — that is what made a sidebar row's chrome look one level out of step (docs/toolbars.md).
    /// `None` for a window's own items, which are always showing.
    active: Option<Rc<dyn Fn() -> bool>>,
}

day_reactive::tls_slots! {
    toolbar;
    /// Value callbacks (search text, toggle state) by dispatch id. Plain buttons don't appear
    /// here — they register with [`crate::menu::register_menu_action`] and arrive as
    /// `Event::MenuAction`.
    static VALUE_ACTIONS: RefCell<HashMap<u64, ValueAction>> = RefCell::new(HashMap::new());
    /// Live contributions by token.
    static CONTRIBUTIONS: RefCell<HashMap<u64, Contribution>> = RefCell::new(HashMap::new());
    /// Each chrome's merged model as last lowered — dayscript resolves an item's action here,
    /// and a re-lower diffs against it to drop the closures the old model owned.
    static MODELS: RefCell<Vec<(Chrome, Vec<ToolbarItem>)>> = const { RefCell::new(Vec::new()) };
    /// Next contribution token, and the registration counter behind `Contribution::seq`.
    static NEXT_TOKEN: Cell<u64> = const { Cell::new(1) };
    /// The window whose content is being built right now (see [`with_window`]).
    static BUILDING: Cell<Option<RNode>> = const { Cell::new(None) };
    /// The navigation pages this build is inside, innermost last, each with the predicate that
    /// says whether it is on screen. A contribution registered with a non-empty stack belongs to
    /// the innermost page; one registered with an empty stack belongs to the window.
    static PAGE_STACK: RefCell<Vec<PageFrame>> = RefCell::new(Vec::new());
}

/// Run `f` with `root` as the window contributions inside it belong to. day-core wraps each
/// window's content build in this; nesting restores the previous window on the way out.
pub(crate) fn with_window<R>(root: RNode, f: impl FnOnce() -> R) -> R {
    // Restored on unwind too (the anim.rs `Restore` rule): a contained panic during a
    // secondary window's build would otherwise leave `BUILDING` pointing at a dead window,
    // silently redirecting every later contribution.
    struct Restore(Option<RNode>);
    impl Drop for Restore {
        fn drop(&mut self) {
            BUILDING.with(|b| b.set(self.0));
        }
    }
    let _restore = Restore(BUILDING.with(|b| b.replace(Some(root))));
    f()
}

/// Run `f` with `page` as the navigation page contributions inside it belong to. The pieces layer
/// wraps each destination, sidebar and content-list build in this.
pub fn with_page<R>(page: RNode, f: impl FnOnce() -> R) -> R {
    with_page_in(
        page,
        None,
        day_spec::ToolbarColumn::Detail,
        window_being_built(),
        f,
    )
}

/// [`with_page_gated`] naming the window explicitly. Callers that build pages lazily — every
/// navigation host — pass the window they were built in.
pub fn with_page_in<R>(
    page: RNode,
    active: Option<Rc<dyn Fn() -> bool>>,
    column: day_spec::ToolbarColumn,
    window: RNode,
    f: impl FnOnce() -> R,
) -> R {
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            PAGE_STACK.with(|s| {
                s.borrow_mut().pop();
            });
        }
    }
    PAGE_STACK.with(|s| {
        s.borrow_mut().push(PageFrame {
            page,
            active,
            column,
            window,
        })
    });
    let _restore = Restore;
    f()
}

/// [`with_page`] with the predicate that says whether this page is ON SCREEN — a collapsed
/// content-list pane, a destination that is not the one showing, a page covered by a push. The
/// window's one bar carries only the chromes that predicate admits (docs/toolbars.md).
pub fn with_page_gated<R>(
    page: RNode,
    active: Option<Rc<dyn Fn() -> bool>>,
    column: day_spec::ToolbarColumn,
    f: impl FnOnce() -> R,
) -> R {
    with_page_in(page, active, column, window_being_built(), f)
}

/// The window the page being built belongs to, else the one being built.
pub fn current_page_window() -> RNode {
    PAGE_STACK
        .with(|s| s.borrow().last().map(|f| f.window))
        .unwrap_or_else(window_being_built)
}

/// The predicate for the page being built, if it has one.
pub fn current_page_gate() -> Option<Rc<dyn Fn() -> bool>> {
    PAGE_STACK.with(|s| s.borrow().last().and_then(|f| f.active.clone()))
}

/// Which column the page being built is — [`day_spec::ToolbarColumn::Window`] at the window root.
pub fn current_page_column() -> day_spec::ToolbarColumn {
    PAGE_STACK.with(|s| {
        s.borrow()
            .last()
            .map(|f| f.column)
            .unwrap_or(day_spec::ToolbarColumn::Window)
    })
}

/// The window being built, else the primary root. Shared with [`crate::ambient`], which scopes
/// the same way for the same reason — an app's one `size_class()` call inside a shared
/// `build_shell` must mean "this window".
pub fn window_being_built() -> RNode {
    BUILDING
        .with(|b| b.get())
        .unwrap_or_else(|| with_tree(|t| t.root_node()))
}

/// The chrome a contribution registered right now belongs to: the innermost navigation page
/// being built, else the window being built, else the primary window.
///
/// Captured ONCE, at registration. A derived contribution re-runs long after its build, when
/// neither stack says anything, so reading this later would send its items to the primary
/// window's chrome.
pub fn current_chrome() -> Chrome {
    match PAGE_STACK.with(|s| s.borrow().last().map(|f| f.page)) {
        Some(page) => Chrome::Page(page),
        None => Chrome::Window(window_being_built()),
    }
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

/// Add one piece's items to `chrome` and return the token that owns them. The caller withdraws
/// them with [`unregister_contribution`], normally from its own scope cleanup.
pub fn register_contribution(chrome: Chrome, items: Vec<ToolbarItem>) -> u64 {
    register_contribution_gated(chrome, items, None)
}

/// [`register_contribution`] for a page, with the predicate that says whether that page is on
/// screen. The caller also re-reads it inside an effect, which is what re-composes the bar when
/// a pane collapses or the destination changes.
pub fn register_contribution_gated(
    chrome: Chrome,
    items: Vec<ToolbarItem>,
    active: Option<Rc<dyn Fn() -> bool>>,
) -> u64 {
    let token = NEXT_TOKEN.with(|n| {
        let t = n.get();
        n.set(t + 1);
        t
    });
    let window = current_page_window();
    CONTRIBUTIONS.with(|m| {
        m.borrow_mut().insert(
            token,
            Contribution {
                chrome,
                seq: token,
                items,
                window,
                active,
            },
        )
    });
    lower(chrome);
    token
}

/// One window's whole bar for a toolkit that draws only one: its own items, then every page
/// chrome ON SCREEN under it, in registration order (docs/toolbars.md).
fn merged_window(root: RNode) -> Vec<ToolbarItem> {
    let mut live: Vec<(u64, Vec<ToolbarItem>)> = CONTRIBUTIONS.with(|m| {
        m.borrow()
            .values()
            .filter(|c| c.window == root)
            .filter(|c| {
                c.active
                    .as_ref()
                    .is_none_or(|f| day_reactive::untrack(|| f()))
            })
            .map(|c| (c.seq, c.items.clone()))
            .collect()
    });
    live.sort_by_key(|(seq, _)| *seq);
    let mut out: Vec<ToolbarItem> = live.into_iter().flat_map(|(_, items)| items).collect();
    out.sort_by_key(|i| placement_rank(i.placement));
    out
}

/// Replace one contribution's items in place, keeping its position among its neighbors.
pub fn update_contribution(token: u64, items: Vec<ToolbarItem>) {
    let chrome = CONTRIBUTIONS.with(|m| {
        let mut m = m.borrow_mut();
        let c = m.get_mut(&token)?;
        c.items = items;
        Some(c.chrome)
    });
    if let Some(chrome) = chrome {
        lower(chrome);
    }
}

/// Withdraw a contribution. Its items leave the chrome, and the closures only it owned are
/// dropped by the re-lower that follows.
pub fn unregister_contribution(token: u64) {
    let chrome = CONTRIBUTIONS.with(|m| m.borrow_mut().remove(&token).map(|c| c.chrome));
    if let Some(chrome) = chrome {
        lower(chrome);
    }
}

/// The merged items for one chrome, in registration order.
fn merged(chrome: Chrome) -> Vec<ToolbarItem> {
    let mut live: Vec<(u64, Vec<ToolbarItem>)> = CONTRIBUTIONS.with(|m| {
        m.borrow()
            .values()
            .filter(|c| c.chrome == chrome)
            .map(|c| (c.seq, c.items.clone()))
            .collect()
    });
    live.sort_by_key(|(seq, _)| *seq);
    let mut out: Vec<ToolbarItem> = live.into_iter().flat_map(|(_, items)| items).collect();
    // Placement is the visual order; registration order breaks ties within a bucket. A stable
    // sort is what keeps two items of the same placement in the order they were declared.
    out.sort_by_key(|i| placement_rank(i.placement));
    out
}

/// The order the buckets draw in, leading to trailing. `Bottom` sorts with `Secondary`: a chrome
/// with no bottom bar draws it there, and one with a bottom bar takes it out of this list before
/// ordering matters.
fn placement_rank(p: day_spec::ToolbarPlacement) -> u8 {
    use day_spec::ToolbarPlacement as P;
    match p {
        P::Navigation => 0,
        P::Principal => 1,
        P::Automatic => 2,
        P::Primary => 3,
        P::Secondary | P::Bottom => 4,
    }
}

/// Push one chrome's merged model to the toolkit.
///
/// An install that changes nothing the user can see REBINDS rather than rebuilds.
///
/// A derived contribution re-runs whenever anything it reads changes, and a rebuild destroys and
/// recreates the native widgets. That is invisible for a button, and destructive for the search
/// field: it takes the keyboard focus and the caret with it. Typing a letter that moves the nav
/// selection re-ran the page build, which re-lowered the bar, which threw away the field being
/// typed into — on every backend, because they all rebuild what they are handed.
///
/// The remedy is to notice that only the CLOSURES are new. Same items, same order, same labels,
/// icons, kinds and enablement means the native bar is already correct; moving the new closures
/// onto the action ids it already carries makes it current without touching a widget.
fn lower(chrome: Chrome) {
    let items = merged(chrome);
    let prev = MODELS.with(|m| {
        m.borrow()
            .iter()
            .find(|(c, _)| *c == chrome)
            .map(|(_, items)| items.clone())
    });
    if let Some(prev) = prev
        && same_shape(&prev, &items)
    {
        let rebound = rebind(&prev, items);
        MODELS.with(|m| {
            if let Some(entry) = m.borrow_mut().iter_mut().find(|(c, _)| *c == chrome) {
                entry.1 = rebound;
            }
        });
        return;
    }

    sweep_values(chrome, &items);
    // ONE bar per window, whichever chrome changed: the toolkit is handed the window's items
    // plus the pages showing, already merged, so it draws what it has always drawn and never has
    // to know that a page contributed any of it (docs/toolbars.md). There is deliberately no
    // per-chrome model — one authority, so a live patch and a re-compose cannot disagree.
    let _ = items;
    recompose_windows();
}

/// Re-lower every window whose composed bar could have changed. Called after any page chrome
/// moves on a toolkit that draws one bar per window; a no-op on the rest.
fn recompose_windows() {
    let roots: Vec<RNode> = CONTRIBUTIONS.with(|m| {
        let mut v: Vec<RNode> = m.borrow().values().map(|c| c.window).collect();
        v.sort();
        v.dedup();
        v
    });
    for root in roots {
        let items = merged_window(root);
        let prev = MODELS.with(|m| {
            m.borrow()
                .iter()
                .find(|(c, _)| *c == Chrome::Window(root))
                .map(|(_, i)| i.clone())
        });
        if prev.as_deref() == Some(items.as_slice()) {
            continue;
        }
        MODELS.with(|m| {
            let mut m = m.borrow_mut();
            match m.iter_mut().find(|(c, _)| *c == Chrome::Window(root)) {
                Some(entry) => entry.1 = items.clone(),
                None => m.push((Chrome::Window(root), items.clone())),
            }
        });
        with_tree(|t| t.set_window_toolbar(root, items));
    }
}

/// The pieces layer, after a change to what is ON SCREEN (a push, a pop, a tab switch), so a
/// one-bar-per-window toolkit is handed the showing pages' items. No-op where every page has a
/// bar of its own.
pub fn chrome_changed() {
    recompose_windows();
}

/// Apply a targeted item update wherever the item lives — the path a bound signal writes through,
/// so a search field keeps its focus and its insertion point.
pub fn patch_toolbar(patch: ToolbarPatch) {
    let owner = MODELS.with(|m| {
        m.borrow()
            .iter()
            .find(|(_, items)| items.iter().any(|i| i.id == *patch_item(&patch)))
            .map(|(c, _)| *c)
    });
    if let Some(chrome) = owner {
        patch_chrome(chrome, patch);
    }
}

/// [`patch_toolbar`] against an explicit chrome. Also updates the retained model, so a later
/// full replace does not resurrect the stale value.
pub fn patch_chrome(chrome: Chrome, patch: ToolbarPatch) {
    // The contribution that owns the item keeps its own copy, so a later re-compose rebuilds
    // from the current value rather than the one the item was declared with.
    let window = CONTRIBUTIONS.with(|m| {
        let mut m = m.borrow_mut();
        let mut window = None;
        for c in m.values_mut() {
            if c.chrome == chrome {
                apply_to_model(&mut c.items, &patch);
                window = Some(c.window);
            }
        }
        window
    });
    // …and so does the WINDOW's model, which is the one the toolkit draws and dayscript reads.
    //
    // A page's chrome has no model of its own: everything is composed into the window's before
    // it crosses (see `lower`). Patching a per-chrome copy left the drawn bar carrying the value
    // the item was BUILT with — which is how a page command declared while nothing was selected
    // stayed disabled on a window that never re-composed afterwards, and did nothing when tapped.
    let Some(root) = window else { return };
    MODELS.with(|m| {
        let mut m = m.borrow_mut();
        if let Some((_, items)) = m.iter_mut().find(|(c, _)| *c == Chrome::Window(root)) {
            apply_to_model(items, &patch);
        }
    });
    with_tree(|t| t.patch_window_toolbar(root, patch));
}

fn patch_item(patch: &ToolbarPatch) -> &String {
    match patch {
        ToolbarPatch::Text { item, .. }
        | ToolbarPatch::On { item, .. }
        | ToolbarPatch::Selected { item, .. }
        | ToolbarPatch::Enabled { item, .. }
        | ToolbarPatch::Suggestions { item, .. } => item,
    }
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
        ToolbarPatch::Selected { item, index } => {
            if let Some(it) = items.iter_mut().find(|i| i.id == *item)
                && let K::Segmented { segments, selected } = &mut it.kind
                && *index < segments.len()
            {
                *selected = *index;
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

/// Every live item, across every chrome — dayscript's `toolbar:` step walks it to resolve an
/// item's dispatch id, and an app has one bar per window at a time, so a duplicate id across two
/// chromes would be an app bug rather than an ambiguity to resolve here.
pub fn toolbar_model() -> Vec<ToolbarItem> {
    MODELS.with(|m| m.borrow().iter().flat_map(|(_, i)| i.clone()).collect())
}

/// Show/hide the sidebar pane of the navigation host `host` — the behavior behind the sidebar
/// affordance a nav host contributes for itself (`day_spec::SIDEBAR_TOGGLE_ID`). `false` when
/// the toolkit has no pane to toggle there. The item's action makes this call, and dayscript's
/// `toolbar:` step presses the item like any other, so a walkthrough drives the same path a
/// click does (docs/toolbars.md).
pub fn toggle_sidebar(host: RNode) -> bool {
    with_tree(|t| t.toggle_sidebar(host))
}

/// Drop a closed window's contributions, chrome model and the value closures only they owned.
///
/// Called BEFORE the window's scope is disposed. Disposal runs every contribution's cleanup,
/// and each one re-composes the window it belonged to — a merge that asks the OTHER
/// contributions' gates whether their page is showing, through signals the same disposal has
/// already dropped. Withdrawing the whole window here first leaves those cleanups nothing to
/// re-compose (a token that is already gone is a no-op), so a closed window never merges its
/// own dying bar.
pub(crate) fn forget_window(root: RNode) {
    CONTRIBUTIONS.with(|m| m.borrow_mut().retain(|_, c| c.window != root));
    let gone: Vec<ToolbarItem> = MODELS.with(|m| {
        let mut m = m.borrow_mut();
        let mut gone = Vec::new();
        m.retain(|(c, items)| {
            let mine = matches!(c, Chrome::Window(r) if *r == root);
            if mine {
                gone.extend(items.clone());
            }
            !mine
        });
        gone
    });
    drop_values(&gone, &[]);
}

/// Forget the value closures the previous model owned and the new one does not — the same
/// discipline `set_app_menu` applies to menu actions, so a toolbar rebuilt on every locale change
/// does not leak a closure per install.
fn sweep_values(chrome: Chrome, next: &[ToolbarItem]) {
    // Against the WINDOW's model — the only one there is. `next` is this chrome's share of it,
    // so the comparison keeps every id the window still carries and drops only the ones this
    // chrome stopped declaring.
    let root = CONTRIBUTIONS.with(|m| {
        m.borrow()
            .values()
            .find(|c| c.chrome == chrome)
            .map(|c| c.window)
    });
    let Some(root) = root else { return };
    let prev = MODELS.with(|m| {
        m.borrow()
            .iter()
            .find(|(c, _)| *c == Chrome::Window(root))
            .map(|(_, items)| items.clone())
            .unwrap_or_default()
    });
    let keep = merged_window(root);
    let mut live = next.to_vec();
    live.extend(keep);
    drop_values(&prev, &live);
}

/// Whether two models describe the same BAR — everything the toolkit renders or dispatches by
/// position, ignoring the action ids (new closures every build) and the search field's live text
/// and completions (kept current through [`ToolbarPatch`], never through a rebuild).
fn same_shape(a: &[ToolbarItem], b: &[ToolbarItem]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| shape_of(x) == shape_of(y))
}

fn shape_of(item: &ToolbarItem) -> ToolbarItem {
    let mut i = item.clone();
    i.action = 0;
    match &mut i.kind {
        day_spec::ToolbarItemKind::Search {
            text, suggestions, ..
        } => {
            text.clear();
            suggestions.clear();
        }
        day_spec::ToolbarItemKind::Menu { items } => blank_menu_ids(items),
        _ => {}
    }
    i
}

fn blank_menu_ids(items: &mut [day_spec::MenuItem]) {
    for item in items {
        match item {
            day_spec::MenuItem::Action { action, .. } => *action = 0,
            day_spec::MenuItem::Submenu { items, .. } => blank_menu_ids(items),
            day_spec::MenuItem::Separator => {}
        }
    }
}

/// Move `next`'s closures onto `prev`'s action ids, so the ids the native bar already holds keep
/// dispatching. Returns the model to store: `next`'s content under `prev`'s ids.
fn rebind(prev: &[ToolbarItem], next: Vec<ToolbarItem>) -> Vec<ToolbarItem> {
    next.into_iter()
        .zip(prev)
        .map(|(mut new, old)| {
            if new.action != old.action {
                VALUE_ACTIONS.with(|m| {
                    let mut m = m.borrow_mut();
                    if let Some(f) = m.remove(&new.action) {
                        m.insert(old.action, f);
                    }
                });
                crate::menu::rebind_action(new.action, old.action);
                new.action = old.action;
            }
            if let (
                day_spec::ToolbarItemKind::Menu { items: new_items },
                day_spec::ToolbarItemKind::Menu { items: old_items },
            ) = (&mut new.kind, &old.kind)
            {
                rebind_menu(new_items, old_items);
            }
            new
        })
        .collect()
}

fn rebind_menu(next: &mut [day_spec::MenuItem], prev: &[day_spec::MenuItem]) {
    for (new, old) in next.iter_mut().zip(prev) {
        match (new, old) {
            (
                day_spec::MenuItem::Action { action: new_id, .. },
                day_spec::MenuItem::Action { action: old_id, .. },
            ) => {
                if new_id != old_id {
                    crate::menu::rebind_action(*new_id, *old_id);
                    *new_id = *old_id;
                }
            }
            (
                day_spec::MenuItem::Submenu { items: n, .. },
                day_spec::MenuItem::Submenu { items: o, .. },
            ) => rebind_menu(n, o),
            _ => {}
        }
    }
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

/// Reset every chrome's toolbar state (tests — pairs with `uninstall_tree`).
pub fn reset_toolbars() {
    MODELS.with(|m| m.borrow_mut().clear());
    CONTRIBUTIONS.with(|m| m.borrow_mut().clear());
    VALUE_ACTIONS.with(|m| m.borrow_mut().clear());
    PAGE_STACK.with(|s| s.borrow_mut().clear());
    BUILDING.with(|b| b.set(None));
}
