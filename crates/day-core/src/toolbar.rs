// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Toolbar contributions (docs/toolbars.md). The model ([`day_spec::ToolbarItem`]) is
//! toolkit-neutral and carries only ids for its commands; the closures live here, keyed by
//! id, the same shape as [`crate::menu`] and the same id space, so one closure can
//! back both a toolbar button and its menu-bar twin.
//!
//! Any piece can declare items. Where it sits decides which chrome carries them (the window's
//! own, or one navigation page's), and its scope decides how long they stay: a contribution is
//! withdrawn when the piece that registered it is disposed, so a command leaves with the content
//! it acts on. Several pieces may contribute to one chrome; this module merges them in
//! registration order and hands the result to the toolkit as one model.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use day_spec::{ToolbarItem, ToolbarPatch, ToolbarValue};

use crate::tree::{RNode, with_tree};

/// A toolbar item's value callback: what a search field's text or a toggle's state runs.
type ValueAction = Rc<dyn Fn(&ToolbarValue)>;

/// Which chrome a contribution lands on.
///
/// Not a placement; [`day_spec::ToolbarPlacement`] says where on a chrome an item sits. This
/// says which chrome, and it is never written by an app: it follows from the piece that declared
/// the items, which is what the design is built around (docs/toolbars.md).
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
    /// The window this page belongs to, captured when its host was built. A destination page is
    /// built lazily, on the first selection, long after its window's build has returned, and
    /// `window_being_built()` then answers the primary window, so a second window's pages
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
    document_window: Option<DocumentWindow>,
    /// Whether the page carrying it is on screen. One bar serves the window, so a pane that is
    /// collapsed or a destination that is not the one showing must not leave its commands on it;
    /// that is what made a sidebar row's chrome look one level out of step (docs/toolbars.md).
    /// `None` for a window's items, which are always showing.
    active: Option<Rc<dyn Fn() -> bool>>,
}

/// A resident document's mutable presentation window. Contributions inherit this
/// context so moving the document also moves its commands without rebuilding them.
#[derive(Clone)]
pub struct DocumentWindow(Rc<Cell<RNode>>);
impl DocumentWindow {
    pub fn new(root: RNode) -> Self {
        Self(Rc::new(Cell::new(root)))
    }
    pub fn set(&self, root: RNode) {
        if self.0.replace(root) != root {
            schedule_recompose();
        }
    }
}
impl Contribution {
    fn window(&self) -> RNode {
        self.document_window
            .as_ref()
            .map_or(self.window, |w| w.0.get())
    }
}

day_reactive::tls_slots! {
    toolbar;
    /// Value callbacks (search text, toggle state) by dispatch id. Plain buttons don't appear
    /// here; they register with [`crate::menu::register_menu_action`] and arrive as
    /// `Event::MenuAction`.
    static VALUE_ACTIONS: RefCell<HashMap<u64, ValueAction>> = RefCell::new(HashMap::new());
    /// Live contributions by token.
    static CONTRIBUTIONS: RefCell<HashMap<u64, Contribution>> = RefCell::new(HashMap::new());
    /// Each chrome's merged model as last lowered; dayscript resolves an item's action here,
    /// and a recompose compares against it to skip an install that would change nothing. Items
    /// carry slot ids here, never closure ids (see [`slot_model`]).
    static MODELS: RefCell<Vec<(Chrome, Vec<ToolbarItem>)>> = const { RefCell::new(Vec::new()) };
    /// Each window's dispatch slots: (window, item key) → the id the toolkit holds for that item
    /// (see [`slot_model`]).
    static SLOT_IDS: RefCell<HashMap<(RNode, String), u64>> = RefCell::new(HashMap::new());
    /// Slot id → the closure id it dispatches to right now.
    static SLOT_TARGETS: RefCell<HashMap<u64, u64>> = RefCell::new(HashMap::new());
    /// Each window's bar as its toolkit holds it: what an edit is computed against. Separate
    /// from [`MODELS`], which is the bar the window should carry (and what dayscript reads), so
    /// an edit the toolkit never received is not mistaken for one it did: the next recompose
    /// sends it again, the way a whole-bar install used to heal on the next change.
    static DELIVERED: RefCell<HashMap<RNode, Vec<ToolbarItem>>> = RefCell::new(HashMap::new());
    /// A recompose is owed at the end of this turn ([`schedule_recompose`]).
    static RECOMPOSE_PENDING: Cell<bool> = const { Cell::new(false) };
    static RECOMPOSING: Cell<bool> = const { Cell::new(false) };
    static RECOMPOSE_AGAIN: Cell<bool> = const { Cell::new(false) };
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

/// [`with_page_gated`] naming the window explicitly. Callers that build pages lazily (every
/// navigation host) pass the window they were built in.
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

/// [`with_page`] with the predicate that says whether this page is on screen (a collapsed
/// content-list pane, a destination that is not the one showing, a page covered by a push). The
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

/// Which column the page being built is; [`day_spec::ToolbarColumn::Window`] at the window root.
pub fn current_page_column() -> day_spec::ToolbarColumn {
    PAGE_STACK.with(|s| {
        s.borrow()
            .last()
            .map(|f| f.column)
            .unwrap_or(day_spec::ToolbarColumn::Window)
    })
}

/// The window being built, else the primary root. Shared with [`crate::ambient`], which scopes
/// the same way for the same reason: an app's one `size_class()` call inside a shared
/// `build_shell` must mean "this window".
pub fn window_being_built() -> RNode {
    BUILDING
        .with(|b| b.get())
        .unwrap_or_else(|| with_tree(|t| t.root_node()))
}

/// The chrome a contribution registered right now belongs to: the innermost navigation page
/// being built, else the window being built, else the primary window.
///
/// Captured once, at registration. A derived contribution re-runs long after its build, when
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
/// `Event::ToolbarChanged`, inside a reactive batch so multiple signal writes coalesce. `action`
/// is the slot id the toolkit holds (see [`slot_model`]); a raw closure id works too.
pub fn dispatch_toolbar_value(action: u64, value: &ToolbarValue) {
    mirror_value(action, value);
    let target = resolve_slot(action);
    let f = VALUE_ACTIONS.with(|m| m.borrow().get(&target).cloned());
    if let Some(f) = f {
        day_reactive::batch(|| f(value));
    }
}

/// The closure id a toolkit-held slot id dispatches to now; any other id is returned as is.
/// The menu rail resolves through here too, since toolbar buttons ride it.
pub(crate) fn resolve_slot(id: u64) -> u64 {
    SLOT_TARGETS.with(|m| m.borrow().get(&id).copied().unwrap_or(id))
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
                document_window: day_reactive::Scope::current().use_context::<DocumentWindow>(),
                active,
            },
        )
    });
    let _ = chrome;
    schedule_recompose();
    token
}

/// One window's whole bar for a toolkit that draws only one: its items, then every page
/// chrome on screen under it, in registration order (docs/toolbars.md).
fn merged_window(root: RNode) -> Vec<ToolbarItem> {
    let mut live: Vec<(u64, Vec<ToolbarItem>)> = CONTRIBUTIONS.with(|m| {
        m.borrow()
            .values()
            .filter(|c| c.window() == root)
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
    let old = CONTRIBUTIONS.with(|m| {
        let mut m = m.borrow_mut();
        let c = m.get_mut(&token)?;
        Some(std::mem::replace(&mut c.items, items))
    });
    if let Some(old) = old {
        forget_closures(&old);
        schedule_recompose();
    }
}

/// Withdraw a contribution. Its items leave the chrome, and the closures only it owned go with
/// them.
pub fn unregister_contribution(token: u64) {
    let old = CONTRIBUTIONS.with(|m| m.borrow_mut().remove(&token).map(|c| c.items));
    if let Some(old) = old {
        forget_closures(&old);
        schedule_recompose();
    }
}

/// Drop the closures `items` registered that no live contribution still carries, so a page
/// visited a hundred times does not hold a hundred generations of its commands.
///
/// Only the item-level ids: a button's or a toggle's own closure, registered fresh by each
/// lowering. A pull-down's entries can name Day's standing dispatchers (a new-window item), so
/// they stay registered, as they did before.
fn forget_closures(items: &[ToolbarItem]) {
    let live: std::collections::HashSet<u64> = CONTRIBUTIONS.with(|m| {
        m.borrow()
            .values()
            .flat_map(|c| c.items.iter().map(|i| i.action))
            .collect()
    });
    for item in items
        .iter()
        .filter(|i| i.action != 0 && !live.contains(&i.action))
    {
        VALUE_ACTIONS.with(|m| m.borrow_mut().remove(&item.action));
        if matches!(
            item.kind,
            day_spec::ToolbarItemKind::Button | day_spec::ToolbarItemKind::Menu { .. }
        ) {
            crate::menu::forget_action(item.action);
        }
    }
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

/// Record a value the user put into a live item, without touching the widget that reported it.
///
/// The widget already shows it; what is stale is Day's copy. Both copies (the owning
/// contribution's, which a re-compose merges from, and the window's model, which the toolkit
/// drew and dayscript reads) must learn it, or the next rebuild seeds the item with whatever Day
/// last pushed. That is how a search field typed into came back empty after a page change, with
/// the list still filtered by the text it had lost, and how clearing it afterwards reported
/// nothing new (the field held the stale seed already).
fn mirror_value(action: u64, value: &ToolbarValue) {
    let Some((root, id)) = MODELS.with(|m| {
        m.borrow().iter().find_map(|(c, items)| match c {
            Chrome::Window(root) => items
                .iter()
                .find(|i| i.action == action)
                .map(|i| (*root, i.id.clone())),
            Chrome::Page(_) => None,
        })
    }) else {
        return;
    };
    let patch = match value {
        ToolbarValue::Text(text) => ToolbarPatch::Text {
            item: id,
            text: text.clone(),
        },
        ToolbarValue::On(on) => ToolbarPatch::On { item: id, on: *on },
        ToolbarValue::Selected(index) => ToolbarPatch::Selected {
            item: id,
            index: *index,
        },
    };
    CONTRIBUTIONS.with(|m| {
        for c in m.borrow_mut().values_mut().filter(|c| c.window() == root) {
            apply_to_model(&mut c.items, &patch);
        }
    });
    MODELS.with(|m| {
        if let Some((_, items)) = m
            .borrow_mut()
            .iter_mut()
            .find(|(c, _)| *c == Chrome::Window(root))
        {
            apply_to_model(items, &patch);
        }
    });
    deliver_value(root, &patch);
}

/// Owe the windows a recompose, paid once when the current turn settles.
///
/// Every change to what a bar carries lands here: a contribution registered, re-derived or
/// withdrawn, a page gate flipping. A single user action makes several of them. Swapping the
/// page withdraws the old page's items and contributes the new page's, and handing each step to
/// the toolkit rebuilt the bar twice, through an intermediate bar nobody should see, when the
/// bar before and after was the same. Deferring to the end of the turn composes only the settled
/// result, and [`recompose_windows`] then finds nothing to install. Outside a turn (a window's
/// first build) it runs right away.
fn schedule_recompose() {
    if RECOMPOSE_PENDING.with(|p| p.replace(true)) {
        return;
    }
    day_reactive::at_turn_end(recompose_windows);
}

/// Hand every window whose composed bar changed its new model, and nothing to the rest.
///
/// A bar is re-installed only when something the toolkit draws changed: an item added,
/// removed or reordered, a label, an icon, a kind. What changes on every derivation, the
/// closures, never reaches the model: each item is handed to the toolkit under a stable slot
/// id (see [`slot_model`]), so a page re-built with the same commands composes a model equal to
/// the installed one. Values the user or the app change (search text, a toggle) reach the model
/// through the same patches that update the widget, so they never differ either.
fn recompose_windows() {
    recompose_serially(recompose_windows_inner);
}

// Native toolbar edits can enqueue layout/focus events. `with_tree` drains those events
// before returning, and their handlers can request another toolbar composition. Finish
// recording DELIVERED first: nested diffs against the old baseline insert an item twice,
// which raises an Objective-C exception in NSToolbar.
fn recompose_serially(mut apply: impl FnMut()) {
    if RECOMPOSING.with(|busy| busy.replace(true)) {
        RECOMPOSE_AGAIN.with(|again| again.set(true));
        return;
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            RECOMPOSING.with(|busy| busy.set(false));
            RECOMPOSE_AGAIN.with(|again| again.set(false));
        }
    }
    let _reset = Reset;
    loop {
        RECOMPOSE_AGAIN.with(|again| again.set(false));
        apply();
        if !RECOMPOSE_AGAIN.with(|again| again.get()) {
            break;
        }
    }
}

fn recompose_windows_inner() {
    RECOMPOSE_PENDING.with(|p| p.set(false));
    let roots: Vec<RNode> = {
        let mut v: Vec<RNode> =
            CONTRIBUTIONS.with(|m| m.borrow().values().map(|c| c.window()).collect());
        // A window whose last contribution left still has a bar to take down.
        v.extend(MODELS.with(|m| {
            m.borrow()
                .iter()
                .filter_map(|(c, _)| match c {
                    Chrome::Window(r) => Some(*r),
                    Chrome::Page(_) => None,
                })
                .collect::<Vec<_>>()
        }));
        v.sort();
        v.dedup();
        v
    };
    for root in roots {
        let items = slot_model(root, unique_ids(merged_window(root)));
        MODELS.with(|m| {
            let mut m = m.borrow_mut();
            match m.iter_mut().find(|(c, _)| *c == Chrome::Window(root)) {
                Some(entry) => entry.1 = items.clone(),
                None => m.push((Chrome::Window(root), items.clone())),
            }
        });
        let prev = DELIVERED.with(|d| d.borrow().get(&root).cloned().unwrap_or_default());
        let (ops, patches) = diff_bar(&prev, &items);
        if ops.is_empty() && patches.is_empty() {
            continue;
        }
        debug_assert!(edit_is_ordered(&ops), "toolbar edit out of order: {ops:?}");
        // Recorded as the toolkit's only once it has the edit. A window whose root has no native
        // handle yet takes none; its bar goes out whole on the next recompose instead.
        let delivered = ops.is_empty() || with_tree(|t| t.edit_window_toolbar(root, ops));
        if !delivered {
            continue;
        }
        for patch in patches {
            with_tree(|t| t.patch_window_toolbar(root, patch));
        }
        DELIVERED.with(|d| d.borrow_mut().insert(root, items));
    }
}

/// The order backends rely on (`Toolkit::edit_toolbar`): every removal, then the insertions in
/// ascending position.
fn edit_is_ordered(ops: &[day_spec::ToolbarOp]) -> bool {
    use day_spec::ToolbarOp as O;
    let first_insert = ops
        .iter()
        .position(|o| matches!(o, O::Insert { .. }))
        .unwrap_or(ops.len());
    let inserts: Vec<usize> = ops[first_insert..]
        .iter()
        .filter_map(|o| match o {
            O::Insert { index, .. } => Some(*index),
            O::Remove { .. } => None,
        })
        .collect();
    inserts.len() == ops.len() - first_insert && inserts.windows(2).all(|w| w[0] < w[1])
}

/// Record a value the toolkit already shows (a patch it took, or input it reported) in its copy
/// of the bar, so the next edit is computed against what is on screen.
fn deliver_value(root: RNode, patch: &ToolbarPatch) {
    DELIVERED.with(|d| {
        if let Some(items) = d.borrow_mut().get_mut(&root) {
            patch.apply_to(items);
        }
    });
}

/// The bar without the items whose id an earlier one already has.
///
/// An id is an item's identity on the bar (the native identifier, the key an edit and a patch
/// address), so it must be unique, and two showing pages declaring the same one is an app bug.
/// Keeping the first, and saying so once, leaves the bar coherent rather than editing two
/// natives under one name.
fn unique_ids(items: Vec<ToolbarItem>) -> Vec<ToolbarItem> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|i| {
            let fresh = seen.insert(i.id.clone());
            if !fresh {
                log::warn!(
                    "toolbar item id {:?} appears twice in one window's bar; only the first is \
                     shown (docs/toolbars.md: ids are unique within a bar)",
                    i.id
                );
            }
            fresh
        })
        .collect()
}

/// The edit that turns bar `prev` into bar `next`, and the value patches for the items it keeps
/// (docs/toolbars.md).
///
/// Items are matched by id. One the toolkit already draws is kept when it is drawn the same way
/// (everything but its values: search text and completions, toggle state, selection,
/// enablement) and when keeping it does not reorder it past another kept item; the kept set is
/// the longest run of matched items already in order, so a reshuffle costs the fewest items.
/// Everything else is removed and inserted. A kept item whose values moved gets the patches that
/// bring it current, the same patches a bound signal would have sent.
fn diff_bar(
    prev: &[ToolbarItem],
    next: &[ToolbarItem],
) -> (Vec<day_spec::ToolbarOp>, Vec<ToolbarPatch>) {
    use day_spec::ToolbarOp;
    // For each item of `next`, the position of its unchanged counterpart in `prev`.
    let matched: Vec<Option<usize>> = next
        .iter()
        .map(|n| {
            prev.iter()
                .position(|p| p.id == n.id)
                .filter(|&at| shape_of(&prev[at]) == shape_of(n))
        })
        .collect();
    let kept = longest_increasing(&matched);

    let mut ops: Vec<ToolbarOp> = prev
        .iter()
        .enumerate()
        .filter(|(at, _)| !kept.iter().any(|&k| matched[k] == Some(*at)))
        .map(|(_, p)| ToolbarOp::Remove { id: p.id.clone() })
        .collect();
    let mut patches = Vec::new();
    for (index, item) in next.iter().enumerate() {
        if kept.contains(&index) {
            let before = &prev[matched[index].unwrap_or_default()];
            patches.extend(value_patches(before, item));
        } else {
            ops.push(ToolbarOp::Insert {
                index,
                item: item.clone(),
            });
        }
    }
    (ops, patches)
}

/// The indices into `seq` of a longest strictly increasing subsequence of its `Some` values.
fn longest_increasing(seq: &[Option<usize>]) -> Vec<usize> {
    // Patience sorting: `tails[k]` is the index of the smallest tail of an increasing run of
    // length k + 1, and `back` links each element to its predecessor in the run.
    let mut tails: Vec<usize> = Vec::new();
    let mut back: Vec<Option<usize>> = vec![None; seq.len()];
    for (i, v) in seq.iter().enumerate() {
        let Some(v) = *v else { continue };
        let k = tails.partition_point(|&t| seq[t].unwrap_or_default() < v);
        back[i] = k.checked_sub(1).map(|p| tails[p]);
        if k == tails.len() {
            tails.push(i);
        } else {
            tails[k] = i;
        }
    }
    let mut out = Vec::new();
    let mut at = tails.last().copied();
    while let Some(i) = at {
        out.push(i);
        at = back[i];
    }
    out.reverse();
    out
}

/// The patches that bring a kept item's values from `was` to `now`.
fn value_patches(was: &ToolbarItem, now: &ToolbarItem) -> Vec<ToolbarPatch> {
    use day_spec::ToolbarItemKind as K;
    let item = now.id.clone();
    let mut out = Vec::new();
    match (&was.kind, &now.kind) {
        (
            K::Search {
                text: t0,
                suggestions: s0,
                ..
            },
            K::Search {
                text: t1,
                suggestions: s1,
                ..
            },
        ) => {
            if t0 != t1 {
                out.push(ToolbarPatch::Text {
                    item: item.clone(),
                    text: t1.clone(),
                });
            }
            if s0 != s1 {
                out.push(ToolbarPatch::Suggestions {
                    item: item.clone(),
                    list: s1.clone(),
                });
            }
        }
        (K::Toggle { on: a }, K::Toggle { on: b }) if a != b => out.push(ToolbarPatch::On {
            item: item.clone(),
            on: *b,
        }),
        (K::Segmented { selected: a, .. }, K::Segmented { selected: b, .. }) if a != b => {
            out.push(ToolbarPatch::Selected {
                item: item.clone(),
                index: *b,
            })
        }
        _ => {}
    }
    if was.enabled != now.enabled {
        out.push(ToolbarPatch::Enabled {
            item,
            on: now.enabled,
        });
    }
    out
}

/// Swap each item's closure id for the window's stable slot id for that item, and point the
/// slot at the closure.
///
/// An item's identity is its id, so the same command on the next page, or the same page built
/// again, gets the slot it had. The toolkit keeps dispatching the id it was given, and the id keeps reaching the
/// current closure: no install to repoint it, and nothing for a toolkit to diff. A pull-down's
/// entries get slots the same way, by their path under the item.
fn slot_model(root: RNode, mut items: Vec<ToolbarItem>) -> Vec<ToolbarItem> {
    let mut live: Vec<u64> = Vec::new();
    for item in &mut items {
        let key = item.id.clone();
        item.action = slot_for(root, &key, item.action, &mut live);
        if let day_spec::ToolbarItemKind::Menu { items: entries } = &mut item.kind {
            slot_menu(root, &key, entries, &mut live);
        }
    }
    // Slots this window no longer shows dispatch nothing; the ids stay reserved for the item's
    // return.
    let keep: std::collections::HashSet<u64> = live.into_iter().collect();
    SLOT_IDS.with(|ids| {
        SLOT_TARGETS.with(|t| {
            let mut t = t.borrow_mut();
            for ((r, _), slot) in ids.borrow().iter() {
                if *r == root && !keep.contains(slot) {
                    t.remove(slot);
                }
            }
        })
    });
    items
}

fn slot_menu(root: RNode, parent: &str, entries: &mut [day_spec::MenuItem], live: &mut Vec<u64>) {
    for (i, entry) in entries.iter_mut().enumerate() {
        let key = format!("{parent}/{i}");
        match entry {
            day_spec::MenuItem::Action { action, .. } => {
                *action = slot_for(root, &key, *action, live);
            }
            day_spec::MenuItem::Submenu { items, .. } => slot_menu(root, &key, items, live),
            day_spec::MenuItem::Separator => {}
        }
    }
}

/// The slot for `key` in `root`'s bar, now dispatching to `target`. `0` stays `0`: an item with
/// nothing to run has nothing to address.
fn slot_for(root: RNode, key: &str, target: u64, live: &mut Vec<u64>) -> u64 {
    if target == 0 {
        return 0;
    }
    let slot = SLOT_IDS.with(|ids| {
        *ids.borrow_mut()
            .entry((root, key.to_string()))
            .or_insert_with(crate::menu::next_action_id)
    });
    SLOT_TARGETS.with(|t| t.borrow_mut().insert(slot, target));
    live.push(slot);
    slot
}

/// The pieces layer, after a change to what is on screen (a push, a pop, a tab switch), so a
/// one-bar-per-window toolkit is handed the showing pages' items. No-op where every page has a
/// bar of its own.
pub fn chrome_changed() {
    schedule_recompose();
}

/// Apply a targeted item update wherever the item lives: the path a bound signal writes through,
/// so a search field keeps its focus and its insertion point.
pub fn patch_toolbar(patch: ToolbarPatch) {
    let owner = MODELS.with(|m| {
        m.borrow()
            .iter()
            .find(|(_, items)| items.iter().any(|i| i.id == patch.item()))
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
                window = Some(c.window());
            }
        }
        window
    });
    // …and so does the window's model, which is the one the toolkit draws and dayscript reads.
    //
    // A page's chrome has no model of its own: everything is composed into the window's before
    // it crosses (see `lower`). Patching a per-chrome copy left the drawn bar carrying the value
    // the item was built with, which is how a page command declared while nothing was selected
    // stayed disabled on a window that never re-composed afterwards, and did nothing when tapped.
    let Some(root) = window else { return };
    MODELS.with(|m| {
        let mut m = m.borrow_mut();
        if let Some((_, items)) = m.iter_mut().find(|(c, _)| *c == Chrome::Window(root)) {
            apply_to_model(items, &patch);
        }
    });
    if with_tree(|t| t.patch_window_toolbar(root, patch.clone())) {
        deliver_value(root, &patch);
    }
}

/// Mirror a patch into the retained model.
fn apply_to_model(items: &mut [ToolbarItem], patch: &ToolbarPatch) {
    patch.apply_to(items);
}

/// An item as the toolkit draws it: everything but its values, which travel as patches, and its
/// action, whose slot id never changes while the item is on the bar.
fn shape_of(item: &ToolbarItem) -> ToolbarItem {
    use day_spec::ToolbarItemKind as K;
    let mut i = item.clone();
    i.enabled = true;
    match &mut i.kind {
        K::Search {
            text, suggestions, ..
        } => {
            text.clear();
            suggestions.clear();
        }
        K::Toggle { on } => *on = false,
        K::Segmented { selected, .. } => *selected = 0,
        _ => {}
    }
    i
}

/// Every live item, across every chrome. dayscript's `toolbar:` step walks it to resolve an
/// item's dispatch id, and an app has one bar per window at a time, so a duplicate id across two
/// chromes would be an app bug rather than an ambiguity to resolve here.
pub fn toolbar_model() -> Vec<ToolbarItem> {
    MODELS.with(|m| m.borrow().iter().flat_map(|(_, i)| i.clone()).collect())
}

/// Show/hide the sidebar pane of the navigation host `host`: the behavior behind the sidebar
/// affordance a nav host contributes for itself (`day_spec::SIDEBAR_TOGGLE_ID`). `false` when
/// the toolkit has no pane to toggle there. The item's action makes this call, and dayscript's
/// `toolbar:` step presses the item like any other, so a walkthrough drives the same path a
/// click does (docs/toolbars.md).
pub fn toggle_sidebar(host: RNode) -> bool {
    with_tree(|t| t.toggle_sidebar(host))
}

/// Drop a closed window's contributions, chrome model and the value closures only they owned.
///
/// Called before the window's scope is disposed. Disposal runs every contribution's cleanup,
/// and each one re-composes the window it belonged to, a merge that asks the other
/// contributions' gates whether their page is showing, through signals the same disposal has
/// already dropped. Withdrawing the whole window here first leaves those cleanups nothing to
/// re-compose (a token that is already gone is a no-op), so a closed window never merges its
/// own dying bar.
pub(crate) fn forget_window(root: RNode) {
    let gone: Vec<ToolbarItem> = CONTRIBUTIONS.with(|m| {
        let mut m = m.borrow_mut();
        let mut gone = Vec::new();
        m.retain(|_, c| {
            let mine = c.window() == root;
            if mine {
                gone.extend(c.items.clone());
            }
            !mine
        });
        gone
    });
    forget_closures(&gone);
    MODELS.with(|m| {
        m.borrow_mut()
            .retain(|(c, _)| !matches!(c, Chrome::Window(r) if *r == root))
    });
    DELIVERED.with(|d| {
        d.borrow_mut().remove(&root);
    });
    SLOT_IDS.with(|ids| {
        SLOT_TARGETS.with(|t| {
            let mut t = t.borrow_mut();
            ids.borrow_mut().retain(|(r, _), slot| {
                if *r == root {
                    t.remove(slot);
                }
                *r != root
            });
        })
    });
}

/// Reset every chrome's toolbar state (tests; pairs with `uninstall_tree`).
pub fn reset_toolbars() {
    MODELS.with(|m| m.borrow_mut().clear());
    CONTRIBUTIONS.with(|m| m.borrow_mut().clear());
    VALUE_ACTIONS.with(|m| m.borrow_mut().clear());
    SLOT_IDS.with(|m| m.borrow_mut().clear());
    SLOT_TARGETS.with(|m| m.borrow_mut().clear());
    DELIVERED.with(|m| m.borrow_mut().clear());
    RECOMPOSE_PENDING.with(|p| p.set(false));
    PAGE_STACK.with(|s| s.borrow_mut().clear());
    BUILDING.with(|b| b.set(None));
}

/// Focus the active window's search control, revealing a collapsed desktop search item.
/// Returns false when the active window has no toolbar search contribution.
pub fn focus_toolbar_search() -> bool {
    let root = crate::windows::focused_root();
    let has = MODELS.with(|models| {
        models.borrow().iter().any(|(chrome, items)| {
            *chrome == Chrome::Window(root)
                && items.iter().any(|item| item.id == day_spec::SEARCH_ITEM_ID)
        })
    });
    if has {
        with_tree(|t| {
            t.patch_window_toolbar(
                root,
                ToolbarPatch::Focus {
                    item: day_spec::SEARCH_ITEM_ID.to_owned(),
                },
            )
        });
    }
    has
}

#[cfg(test)]
mod tests {

    #[test]
    fn native_events_recompose_after_the_delivered_baseline_is_recorded() {
        let passes = std::cell::Cell::new(0);
        let delivered = std::cell::Cell::new(false);
        super::recompose_serially(|| {
            passes.set(passes.get() + 1);
            if passes.get() == 1 {
                // Simulate a native edit draining an event before the caller records delivery.
                super::recompose_serially(|| panic!("must not diff the stale baseline"));
                super::recompose_serially(|| panic!("nested requests must coalesce"));
                delivered.set(true);
            } else {
                assert!(delivered.get());
            }
        });
        assert_eq!(passes.get(), 2);
        super::recompose_serially(|| passes.set(passes.get() + 1));
        assert_eq!(passes.get(), 3, "the guard releases after a settled update");
    }
    use super::*;
    use day_spec::{ToolbarItemKind as K, ToolbarMirror, ToolbarOp};

    fn item(id: &str, label: &str) -> ToolbarItem {
        ToolbarItem {
            id: id.into(),
            kind: K::Button,
            label: label.into(),
            tooltip: None,
            icon: None,
            enabled: true,
            action: 0,
            placement: day_spec::ToolbarPlacement::Automatic,
            label_style: day_spec::LabelStyle::Automatic,
            prominent: false,
            column: day_spec::ToolbarColumn::Window,
        }
    }

    /// Apply the diff the way a backend does and compare with the target.
    fn check(prev: &[ToolbarItem], next: &[ToolbarItem]) -> Vec<ToolbarOp> {
        let (ops, patches) = diff_bar(prev, next);
        let mut mirror = ToolbarMirror::default();
        for (i, p) in prev.iter().enumerate() {
            mirror.insert(i, p.clone());
        }
        // Removals first, then insertions in ascending position: the contract backends rely on.
        let first_insert = ops
            .iter()
            .position(|o| matches!(o, ToolbarOp::Insert { .. }))
            .unwrap_or(ops.len());
        assert!(
            ops[first_insert..]
                .iter()
                .all(|o| matches!(o, ToolbarOp::Insert { .. }))
        );
        for op in &ops {
            mirror.apply(op);
        }
        for p in &patches {
            mirror.patch(p);
        }
        assert_eq!(mirror.items(), next);
        ops
    }

    #[test]
    fn identical_bars_need_no_edit() {
        let bar = vec![item("a", "A"), item("b", "B")];
        assert!(check(&bar, &bar).is_empty());
    }

    #[test]
    fn inserts_and_removes_touch_only_their_items() {
        let prev = vec![item("a", "A"), item("b", "B"), item("search", "S")];
        let next = vec![item("a", "A"), item("c", "C"), item("search", "S")];
        let ops = check(&prev, &next);
        assert_eq!(ops.len(), 2);
        assert!(ops.iter().all(|o| match o {
            ToolbarOp::Remove { id } => id == "b",
            ToolbarOp::Insert { item, index } => item.id == "c" && *index == 1,
        }));
    }

    #[test]
    fn a_reorder_moves_the_fewest_items() {
        let prev = vec![
            item("a", "A"),
            item("b", "B"),
            item("c", "C"),
            item("d", "D"),
        ];
        let next = vec![
            item("d", "D"),
            item("a", "A"),
            item("b", "B"),
            item("c", "C"),
        ];
        let ops = check(&prev, &next);
        // `d` moves; `a`, `b` and `c` stay put.
        assert_eq!(ops.len(), 2, "{ops:?}");
    }

    #[test]
    fn a_value_change_is_a_patch_and_a_label_change_an_edit() {
        let mut toggled = item("t", "T");
        toggled.kind = K::Toggle { on: false };
        let mut on = toggled.clone();
        on.kind = K::Toggle { on: true };
        on.enabled = false;
        let (ops, patches) = diff_bar(std::slice::from_ref(&toggled), std::slice::from_ref(&on));
        assert!(ops.is_empty());
        assert_eq!(patches.len(), 2);
        check(&[item("x", "Old")], &[item("x", "New")]);
    }

    #[test]
    fn longest_increasing_skips_unmatched() {
        assert_eq!(
            longest_increasing(&[Some(2), None, Some(0), Some(1), Some(3)]),
            vec![2, 3, 4]
        );
        assert!(longest_increasing(&[None, None]).is_empty());
    }
}
