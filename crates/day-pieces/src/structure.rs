// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Data-driven structure pieces: `when` (conditional), `each` (keyed children from a fixed
//! sequence), `list` (a reactive, diffed collection), and the scoped `environment` context
//! (`with_environment` / `environment`).

use std::cell::RefCell;
use std::collections::HashSet;
use std::hash::Hash;
use std::rc::Rc;

use day_core::*;
use day_reactive::{Scope, Signal, watch};
use day_spec::props::*;

use crate::Decorated;
use day_spec::{Event, kinds};

// ---------------------------------------------------------------------------
// Structure: when / each (§5.3–§5.4)
// ---------------------------------------------------------------------------

/// Reactive conditional subtree. The anchor is a layout-transparent group; the active arm
/// lives in its own child scope, disposed on switch (§4.3).
///
/// A bare `when` builds nothing while the condition is false. Chain
/// [`otherwise`](When::otherwise) for the either/or case:
///
/// ```ignore
/// when(move || ok.get(), || label("Saved")).otherwise(|| label("Failed"))
/// ```
///
/// Prefer a binding to a flip when both arms are the same widget with different content —
/// `label(move || if ok.get() { "Saved" } else { "Failed" })` keeps ONE native label alive and
/// costs one setter call, where a `when` swap destroys and recreates native widgets.
pub fn when<P: Piece>(
    cond: impl Fn() -> bool + 'static,
    build_arm: impl Fn() -> P + 'static,
) -> When {
    When {
        cond: Rc::new(cond),
        then: Rc::new(move || AnyPiece::new(build_arm())),
        otherwise: None,
    }
}

/// A conditional subtree, optionally with an else arm. Built by [`when`].
pub struct When {
    cond: Rc<dyn Fn() -> bool>,
    then: Rc<dyn Fn() -> AnyPiece>,
    otherwise: Option<Rc<dyn Fn() -> AnyPiece>>,
}

impl When {
    /// The arm built while the condition is FALSE. Without it, false means "no subtree".
    ///
    /// The two arms need not return the same `Piece` type — each is erased to [`AnyPiece`] here.
    /// Exactly one arm is mounted at a time, in its own child scope, so flipping disposes the
    /// outgoing arm's signals, bindings, and handlers before the incoming arm builds (§4.3).
    pub fn otherwise<P: Piece>(mut self, build_arm: impl Fn() -> P + 'static) -> Self {
        self.otherwise = Some(Rc::new(move || AnyPiece::new(build_arm())));
        self
    }
}

impl Piece for When {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let anchor = cx.layout_only(
            Rc::new(PassThrough),
            Flex {
                is_group: true,
                ..Default::default()
            },
            Boundary::No,
        );
        let state: Rc<RefCell<Option<Scope>>> = Rc::new(RefCell::new(None));
        let When {
            cond,
            then,
            otherwise,
        } = self;

        let mount = {
            let state = state.clone();
            move |on: bool| {
                // Same lifetime hazard as `each`'s sync, and the same answer — see the note there.
                if !with_tree(|t| t.node_exists(anchor)) {
                    return;
                }
                // Unmount whatever is up before building the incoming arm: with two arms, every
                // flip is a swap, and the outgoing scope must be disposed BEFORE its replacement
                // builds so a rebuilt arm never observes the old arm's still-live bindings.
                if let Some(scope) = state.borrow_mut().take() {
                    scope.dispose();
                    // Remove everything under the anchor.
                    while with_tree(|t| t.child_count(anchor)) > 0 {
                        let child = with_tree(|t| t.first_child(anchor));
                        match child {
                            Some(c) => with_tree(|t| t.remove_subtree(c)),
                            None => break,
                        }
                    }
                }
                let arm = if on { Some(&then) } else { otherwise.as_ref() };
                if let Some(arm) = arm {
                    let scope = Scope::child();
                    scope.enter(|| {
                        let mut cx = BuildCx::new(anchor);
                        let _ = arm().build(&mut cx);
                    });
                    *state.borrow_mut() = Some(scope);
                }
            }
        };

        let initial = {
            let cond = cond.clone();
            day_reactive::untrack(move || cond())
        };
        mount(initial);
        watch(
            move || cond(),
            move |now, old| {
                if Some(now) != old {
                    mount(*now);
                }
            },
        );
        anchor
    }
}

/// A `Copy` handle to one keyed item's state — the unified `each`/`list` contract (§5.4).
pub struct ItemSlot<T: 'static, K: 'static> {
    sig: Signal<T>,
    key: Signal<K>,
}

impl<T: 'static, K: 'static> Clone for ItemSlot<T, K> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: 'static, K: 'static> Copy for ItemSlot<T, K> {}

impl<T: Clone + 'static, K: Clone + 'static> ItemSlot<T, K> {
    /// Tracked whole-item read. **Read it inside a reactive closure** — e.g.
    /// `label(move || slot.get())` — not eagerly. A recycling [`list`] rebinds one physical row to
    /// many items, and only bindings that read the slot reactively update on rebind; an eager
    /// `let name = slot.get()` freezes the row at its first item.
    pub fn get(self) -> T {
        self.sig.get()
    }
    /// Tracked read via a projection. Read it inside a reactive closure (see [`get`](Self::get)).
    pub fn with<R>(self, f: impl FnOnce(&T) -> R) -> R {
        self.sig.with(f)
    }
    /// Tracked field projection (equality-gating happens in the binding layer). Read it inside a
    /// reactive closure (see [`get`](Self::get)) so recycled rows update on rebind.
    pub fn field<V: Clone>(self, f: impl FnOnce(&T) -> V) -> V {
        self.sig.with(f)
    }
    pub fn key(self) -> K {
        self.key.get_untracked()
    }
}

// ---------------------------------------------------------------------------
// Row sources — where `each` and `list` get their rows (§5.4, docs/list.md)
// ---------------------------------------------------------------------------

/// Where a collection's rows come from. [`each`]`(source, row)` and [`list`]`(source, row)`
/// accept anything implementing this: plain data via [`items`]`(closure, key_of)`, or — with
/// the `model` feature — a day-model store directly (collection order) or
/// `store.rows(projection)` (a tracked display projection of key ids).
pub trait RowSource {
    /// What a row builder receives.
    type Slot: Copy + 'static;
    /// What selection callbacks hand the app.
    type Ref: 'static;
    type Conn: RowConn<Slot = Self::Slot, Ref = Self::Ref>;
    fn connect(self) -> Self::Conn;
}

/// One collection's live connection to its data, held for the life of the `each`/`list` that
/// [`RowSource::connect`]ed it.
pub trait RowConn: 'static {
    type Slot: Copy + 'static;
    type Ref: 'static;

    /// Refresh the snapshot and return the display rows as identity tokens. TRACKED: the
    /// enclosing watch re-runs exactly when something this read depends on changes — for a
    /// store source that is the collection's shape (or the projection's own reads), never the
    /// fields a row merely displays.
    fn refresh(&self) -> Vec<u64>;
    /// Row count of the current snapshot (no tracking).
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Identity token of row `index` in the current snapshot.
    fn token_at(&self, index: usize) -> u64;
    /// The current tokens, no refresh and no tracking — the reorder/delete echo compares these.
    fn tokens_now(&self) -> Vec<u64>;
    /// A fresh slot for row `index` (`None` on a stale mid-animation pull). Call inside the
    /// row's scope: plain-data slots allocate their signals here.
    fn slot_at(&self, index: usize) -> Option<Self::Slot>;
    /// Point an existing slot at row `index` — the recycle write.
    fn rebind(&self, slot: &Self::Slot, index: usize);
    /// The selection currency for row `index`.
    fn select_ref(&self, index: usize) -> Option<Self::Ref>;
    /// Whether row VALUES reach existing rows only through reload→rebind. Plain-data rows say
    /// true (their slots hold copies); store rows say false (values flow through the store's
    /// own per-field notifications), which lets `list` skip the native reload entirely when
    /// the row SET is unchanged.
    fn values_flow_by_reload(&self) -> bool;
    /// What changed since the last refresh, if the source can say PRECISELY — sequential
    /// row deltas a host can animate. `None` means unknown: the list reloads, which is
    /// always honest. Only sources that maintain their set incrementally (a live query)
    /// answer `Some`.
    fn take_row_events(&self) -> Option<Vec<day_spec::props::RowDelta>> {
        None
    }
    /// Rotate the snapshot for a committed native move (display indices).
    fn commit_move(&self, from: usize, to: usize);
    /// Drop a row from the snapshot for a committed native delete.
    fn commit_delete(&self, index: usize);
}

/// Plain-data rows: an items closure plus a key function —
/// `list(items(model::ordered, |i: &Item| i.id), row_view)`.
pub fn items<T, K>(
    f: impl Fn() -> Vec<T> + 'static,
    key_of: impl Fn(&T) -> K + 'static,
) -> Items<T, K>
where
    T: Clone + 'static,
    K: Clone + Hash + 'static,
{
    Items {
        f: Rc::new(f),
        key_of: Rc::new(key_of),
    }
}

/// The plain-data [`RowSource`] built by [`items`].
pub struct Items<T: 'static, K: 'static> {
    f: Rc<dyn Fn() -> Vec<T>>,
    key_of: Rc<dyn Fn(&T) -> K>,
}

impl<T: Clone + 'static, K: Clone + Hash + 'static> RowSource for Items<T, K> {
    type Slot = ItemSlot<T, K>;
    type Ref = K;
    type Conn = ItemsConn<T, K>;
    fn connect(self) -> ItemsConn<T, K> {
        ItemsConn {
            f: self.f,
            key_of: self.key_of,
            snapshot: RefCell::new(Vec::new()),
            tokens: RefCell::new(Vec::new()),
        }
    }
}

/// [`Items`]' connection: the current data snapshot the slots read from.
pub struct ItemsConn<T: 'static, K: 'static> {
    f: Rc<dyn Fn() -> Vec<T>>,
    key_of: Rc<dyn Fn(&T) -> K>,
    snapshot: RefCell<Vec<T>>,
    tokens: RefCell<Vec<u64>>,
}

impl<T: Clone + 'static, K: Clone + Hash + 'static> RowConn for ItemsConn<T, K> {
    type Slot = ItemSlot<T, K>;
    type Ref = K;

    fn refresh(&self) -> Vec<u64> {
        let its = (self.f)();
        let toks: Vec<u64> = its.iter().map(|t| key_token(&(self.key_of)(t))).collect();
        *self.snapshot.borrow_mut() = its;
        *self.tokens.borrow_mut() = toks.clone();
        toks
    }
    fn len(&self) -> usize {
        self.snapshot.borrow().len()
    }
    fn token_at(&self, index: usize) -> u64 {
        self.tokens.borrow().get(index).copied().unwrap_or(0)
    }
    fn tokens_now(&self) -> Vec<u64> {
        self.tokens.borrow().clone()
    }
    fn slot_at(&self, index: usize) -> Option<ItemSlot<T, K>> {
        let item = self.snapshot.borrow().get(index).cloned()?;
        let key = (self.key_of)(&item);
        Some(ItemSlot {
            sig: Signal::new(item),
            key: Signal::new(key),
        })
    }
    fn rebind(&self, slot: &ItemSlot<T, K>, index: usize) {
        let Some(item) = self.snapshot.borrow().get(index).cloned() else {
            return; // stale recycle index — skip, the next bind corrects it
        };
        slot.key.set((self.key_of)(&item));
        slot.sig.set(item);
    }
    fn select_ref(&self, index: usize) -> Option<K> {
        self.snapshot.borrow().get(index).map(|t| (self.key_of)(t))
    }
    fn values_flow_by_reload(&self) -> bool {
        true
    }
    fn commit_move(&self, from: usize, to: usize) {
        let mut snap = self.snapshot.borrow_mut();
        let mut toks = self.tokens.borrow_mut();
        if from >= snap.len() || to >= snap.len() {
            return;
        }
        let item = snap.remove(from);
        snap.insert(to, item);
        let tok = toks.remove(from);
        toks.insert(to, tok);
    }
    fn commit_delete(&self, index: usize) {
        let mut snap = self.snapshot.borrow_mut();
        let mut toks = self.tokens.borrow_mut();
        if index >= snap.len() {
            return;
        }
        snap.remove(index);
        toks.remove(index);
    }
}

struct EachRow<S: 'static> {
    token: u64,
    slot: S,
    scope: Scope,
    root: RNode,
}

/// Reactive keyed collection (§5.4): keyed diff, per-key child scopes, slot rebinds for
/// surviving rows, debug key-uniqueness assertion. Takes any [`RowSource`].
pub fn each<S, P>(source: S, build_row: impl Fn(S::Slot) -> P + 'static) -> impl Piece
where
    S: RowSource + 'static,
    P: Piece,
{
    piece_fn(move |cx| {
        let anchor = cx.layout_only(
            Rc::new(PassThrough),
            Flex {
                is_group: true,
                ..Default::default()
            },
            Boundary::No,
        );
        let conn = Rc::new(source.connect());
        let rows: Rc<RefCell<Vec<EachRow<S::Slot>>>> = Rc::new(RefCell::new(Vec::new()));
        let build_row = Rc::new(build_row);

        let sync = {
            let rows = rows.clone();
            let conn = conn.clone();
            let build_row = build_row.clone();
            move |tokens: &Vec<u64>| {
                // The reaction driving this closure can outlive the subtree it builds into: a
                // page swapped out of a nav, a `when` arm closed above us, and the anchor is gone
                // while the reaction is still subscribed. Building into a removed parent panics in
                // `Tree::attach`, and that panic is contained at the native event boundary — which
                // ABANDONS THE REST OF THE DRAIN, so every reaction queued behind it is skipped
                // and the UI stops updating until something rebuilds it. An anchor that is no
                // longer in the tree has nothing to sync, so leave quietly.
                if !with_tree(|t| t.node_exists(anchor)) {
                    return;
                }
                if cfg!(debug_assertions) {
                    let mut seen = HashSet::new();
                    for k in tokens {
                        assert!(seen.insert(*k), "day: duplicate key in `each` diff");
                    }
                }
                let mut old = std::mem::take(&mut *rows.borrow_mut());
                let mut next: Vec<EachRow<S::Slot>> = Vec::with_capacity(tokens.len());
                for (index, tok) in tokens.iter().enumerate() {
                    if let Some(pos) = old.iter().position(|r| r.token == *tok) {
                        let row = old.remove(pos);
                        // Surviving row: one rebind (plain data refreshes the slot's copy;
                        // a store slot is a no-op when the key is unchanged).
                        conn.rebind(&row.slot, index);
                        next.push(row);
                    } else {
                        let scope = Scope::child();
                        let built = scope.enter(|| {
                            let slot = conn.slot_at(index)?;
                            let mut cx = BuildCx::new(anchor);
                            Some((slot, build_row(slot).build(&mut cx)))
                        });
                        match built {
                            Some((slot, root)) => next.push(EachRow {
                                token: *tok,
                                slot,
                                scope,
                                root,
                            }),
                            None => scope.dispose(),
                        }
                    }
                }
                // Removals.
                for row in old {
                    row.scope.dispose();
                    with_tree(|t| t.remove_subtree(row.root));
                }
                // Order: reattach in the new sequence.
                let order: Vec<RNode> = next.iter().map(|r| r.root).collect();
                with_tree(|t| t.reorder_children(anchor, order));
                *rows.borrow_mut() = next;
            }
        };

        let initial = {
            let conn = conn.clone();
            day_reactive::untrack(move || conn.refresh())
        };
        sync(&initial);
        let conn2 = conn.clone();
        watch(move || conn2.refresh(), move |new, _| sync(new));
        anchor
    })
}

// ---------------------------------------------------------------------------
// @Environment — ambient values over day-reactive's scope context (§4.3). No backend work.
// ---------------------------------------------------------------------------

/// Provide an ambient value `T` to `content` and its ENTIRE descendant subtree (the SwiftUI
/// `@Environment`/`.environment(_)` analog, layered over day-reactive's scope context). `content`
/// — and any piece built within it — reads it back with [`environment`]. A thin, non-reactive
/// wrapper: `T` is a snapshot captured here; for a value that must react, provide a `Signal<T>`
/// (or a `Memo<T>`) and read it reactively inside the subtree.
///
/// ```ignore
/// #[derive(Clone)] struct Theme { accent: Color }
/// with_environment(Theme { accent: BLUE }, || my_screen())
/// // deep inside my_screen():  let accent = environment::<Theme>().unwrap().accent;
/// ```
pub fn with_environment<T: Clone + 'static, P: Piece>(
    value: T,
    content: impl FnOnce() -> P + 'static,
) -> impl Piece {
    piece_fn(move |cx| {
        // A child scope carrying `T`, entered for the whole of `content`'s construction AND build,
        // so both `content`'s own body and every descendant piece's build resolve it via
        // `use_context` (which walks scope → ancestors). Owned by the current build scope, so it is
        // disposed with the enclosing subtree (e.g. a `when` arm) exactly like `when`/`each` scopes.
        let scope = Scope::child();
        scope.provide(value);
        scope.enter(|| content().build(cx))
    })
}

/// Read the nearest ambient `T` provided by an enclosing [`with_environment`], or `None` if none is
/// in scope. Call it while constructing or building a piece within that subtree.
pub fn environment<T: Clone + 'static>() -> Option<T> {
    Scope::current().use_context::<T>()
}

// ---------------------------------------------------------------------------
// `list` — native recycling list (docs/list.md, §10)
// ---------------------------------------------------------------------------

/// Stable u64 identity token for a key, for the native list's diffing.
fn key_token<K: Hash>(k: &K) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    k.hash(&mut h);
    h.finish()
}

/// A native recycling list: the platform widget owns scrolling + cell reuse; Day builds each
/// visible row once and *rebinds* it as cells recycle. Takes any [`RowSource`]; shares the row
/// contract with [`each`], so migrating a collection between them is a one-word change.
pub struct List<S: RowSource> {
    source: Option<S>,
    build_row: Rc<dyn Fn(S::Slot) -> AnyPiece>,
    row_height: RowHeight,
    on_select: Option<Rc<dyn Fn(S::Ref)>>,
    on_selection: Option<SelectionFn<S::Ref>>,
    multi_select: bool,
    selected_rows: Option<Rc<dyn Fn() -> Vec<usize>>>,
    scroll_to_end: Option<day_reactive::Trigger>,
    scroll_to_row: Option<Signal<Option<usize>>>,
    stick_to_bottom: bool,
    reorderable: bool,
    on_reorder: Option<Rc<dyn Fn(usize, usize)>>,
    reorder_guard: Option<Rc<dyn Fn(usize, usize) -> Reorder>>,
    deletable: bool,
    delete_label: String,
    on_delete: Option<Rc<dyn Fn(usize)>>,
    delete_guard: Option<Rc<dyn Fn(usize) -> bool>>,
}

/// The full-set selection callback, aliased for the field above.
type SelectionFn<R> = Rc<dyn Fn(Vec<R>)>;

/// A reorder guard's verdict on a proposed row move (docs/list.md): consulted synchronously from
/// the native drag's validate hook, so the affordance (gap, insertion mark, forbidden cursor)
/// reflects the answer while the user is still dragging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reorder {
    /// Accept the drop where proposed.
    Allow,
    /// Refuse the drop (the row springs back; most backends show a forbidden cursor live).
    Deny,
    /// Accept the drop, but at this row index instead of the proposed one.
    Retarget(usize),
}

/// Build a recycling list from a [`RowSource`] and a row builder:
/// `list(items(rows_fn, key_of), row_view)` for plain data, `list(store, row_view)` (or
/// `list(store.rows(projection), …)`) for a day-model store.
pub fn list<S, P>(source: S, build_row: impl Fn(S::Slot) -> P + 'static) -> List<S>
where
    S: RowSource + 'static,
    P: Piece,
{
    List {
        source: Some(source),
        build_row: Rc::new(move |slot| AnyPiece::new(build_row(slot))),
        row_height: RowHeight::Automatic,
        on_select: None,
        on_selection: None,
        multi_select: false,
        selected_rows: None,
        scroll_to_end: None,
        scroll_to_row: None,
        stick_to_bottom: false,
        reorderable: false,
        on_reorder: None,
        reorder_guard: None,
        deletable: false,
        delete_label: String::new(),
        on_delete: None,
        delete_guard: None,
    }
}

impl<S: RowSource + 'static> List<S> {
    /// Row sizing: `Uniform(h)` (fastest) or `Automatic` (self-sizing).
    pub fn row_height(mut self, h: RowHeight) -> Self {
        self.row_height = h;
        self
    }
    /// Called with the selected row when the native list reports a selection — the key for a
    /// plain-data source, the row's `Elem` handle for a store source.
    pub fn on_select(mut self, f: impl Fn(S::Ref) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }
    /// Allow selecting several rows at once, where the toolkit supports it (docs/list.md has
    /// the matrix; single-selection backends fall back to one row at a time). Every selection
    /// change calls [`Self::on_selection`] with the FULL set of selected rows.
    pub fn multi_select(mut self, on: bool) -> Self {
        self.multi_select = on;
        self
    }
    /// Called with the full set of selected rows (row order, empty = cleared) whenever the
    /// selection changes — the multi-select analogue of [`Self::on_select`]. Also fired by
    /// single-selection backends with a one-element (or empty) set.
    pub fn on_selection(mut self, f: impl Fn(Vec<S::Ref>) + 'static) -> Self {
        self.on_selection = Some(Rc::new(f));
        self
    }
    /// Reactively sync the native selection to `rows` (row indices; empty clears). Re-runs
    /// whenever the closure's tracked reads change; the toolkit applies the sync without
    /// re-emitting a selection event — drive it from app state to build "Clear selection".
    pub fn selected_rows(mut self, rows: impl Fn() -> Vec<usize> + 'static) -> Self {
        self.selected_rows = Some(Rc::new(rows));
        self
    }
    /// Scroll the list so its LAST row is fully visible whenever `trigger` fires — e.g. a chat
    /// timeline sticking to the newest message. Fire it with [`day_reactive::Trigger::notify`]
    /// after appending. No-op while the list is empty. The scroll targets the native list
    /// (`NSTableView`/`UITableView`/`GtkListView`/`QListView`/`RecyclerView`), so it respects the
    /// platform's own scroll physics.
    pub fn scroll_to_end(mut self, trigger: day_reactive::Trigger) -> Self {
        self.scroll_to_end = Some(trigger);
        self
    }
    /// Best-effort auto-stick: after a data reload, scroll to the end so freshly appended rows stay
    /// visible. Convenience over [`Self::scroll_to_end`] for feeds that always follow the newest
    /// row; for finer control (only stick when the user is already near the bottom) drive
    /// `scroll_to_end` from your own logic instead. Off by default.
    pub fn stick_to_bottom(mut self, on: bool) -> Self {
        self.stick_to_bottom = on;
        self
    }

    /// Programmatic scroll-to-row (docs/list.md): set the signal to `Some(row)` and the native
    /// list scrolls that row into view, realizing it if it was virtualized away — the row rail's
    /// counterpart to `scroll(...).scroll_target(...)`. The signal is left as written; setting
    /// the same row again re-fires.
    pub fn scroll_to_row(mut self, sig: Signal<Option<usize>>) -> Self {
        self.scroll_to_row = Some(sig);
        self
    }

    /// Let the user drag rows into a new order with the platform's native mechanism — the
    /// macOS drop gap, the iOS long-press lift, Android's `ItemTouchHelper` — where the backend
    /// supports it (probe `Cap::ListReorder`; docs/list.md has the matrix). Pair with
    /// [`on_reorder`](Self::on_reorder) so the app's data follows the move, and optionally
    /// [`reorder_guard`](Self::reorder_guard) to veto or retarget drops.
    pub fn reorderable(mut self, on: bool) -> Self {
        self.reorderable = on;
        self
    }

    /// A committed move: row `from` now sits at row `to`. Apply the same rotation to the backing
    /// data (`let it = v.remove(from); v.insert(to, it);`) — and persist it if the order should
    /// survive a relaunch. Runs on the main thread at the next event drain, never inside the
    /// native drop callback.
    pub fn on_reorder(mut self, f: impl Fn(usize, usize) + 'static) -> Self {
        self.on_reorder = Some(Rc::new(f));
        self
    }

    /// Veto or override drops while the drag is live: called synchronously from the native
    /// validate hook with `(from, proposed_to)`. Return [`Reorder::Deny`] to refuse,
    /// [`Reorder::Retarget`] to accept at a different index (a "pinned rows" pattern), or
    /// [`Reorder::Allow`] to accept as proposed (the default when no guard is set). Keep it
    /// pure — read state, decide, return; it runs inside the platform's drag callback.
    pub fn reorder_guard(mut self, g: impl Fn(usize, usize) -> Reorder + 'static) -> Self {
        self.reorder_guard = Some(Rc::new(g));
        self
    }

    /// Let the user delete rows with the platform's own delete gesture — the iOS trailing swipe
    /// action, Android's `ItemTouchHelper` swipe, ArkUI's `ListItem` swipe action — where the
    /// backend supports it (probe `Cap::ListDelete`; docs/list.md has the matrix). Pair with
    /// [`on_delete`](Self::on_delete) so the app's data follows, and optionally
    /// [`delete_guard`](Self::delete_guard) to protect individual rows.
    ///
    /// The DESKTOP toolkits have no swipe idiom and answer `Unsupported`, so a list that must be
    /// editable everywhere pairs this with an explicit control — a menu item or a button — rather
    /// than leaving desktop users with no way to delete.
    pub fn deletable(mut self, on: bool) -> Self {
        self.deletable = on;
        self
    }

    /// The label the platform puts on its delete affordance, localized by the app (`res::str::…`
    /// formatted to a `String`). Left unset, each backend falls back to a trash glyph rather than
    /// shipping an English word into every locale.
    pub fn delete_label(mut self, text: impl Into<String>) -> Self {
        self.delete_label = text.into();
        self
    }

    /// A committed delete: row `index` is gone. Apply the same removal to the backing data
    /// (`v.remove(index)`) — and persist it if the change should survive a relaunch. Runs on the
    /// main thread at the next event drain, never inside the native swipe callback.
    pub fn on_delete(mut self, f: impl Fn(usize) + 'static) -> Self {
        self.on_delete = Some(Rc::new(f));
        self
    }

    /// Protect individual rows: called synchronously before the affordance is offered, so a row
    /// that answers `false` shows no delete action rather than one that fails on use. Keep it
    /// pure — it runs inside the platform's swipe callback.
    pub fn delete_guard(mut self, g: impl Fn(usize) -> bool + 'static) -> Self {
        self.delete_guard = Some(Rc::new(g));
        self
    }
}

impl<S: RowSource + 'static> Piece for List<S> {
    fn build(mut self, cx: &mut BuildCx) -> RNode {
        let props = ListProps {
            row_height: self.row_height,
            selectable: self.on_select.is_some() || self.on_selection.is_some(),
            multi_select: self.multi_select,
            reorderable: self.reorderable,
            deletable: self.deletable,
            delete_label: self.delete_label.clone(),
        };
        let node = cx.leaf(
            kinds::LIST,
            &props,
            Flex {
                grow_w: true,
                grow_h: true,
                ..Default::default()
            },
        );

        // The source's live connection: the data-source's view of the world, refreshed by the
        // watch below. The native host queries it synchronously; the driver's build/rebind
        // closures read the same snapshot.
        let conn = Rc::new(self.source.take().expect("List built once").connect());
        // After a committed native move/delete, the token order the app's own data update is
        // expected to echo back. When the refresh below sees exactly this order, the native rows
        // already sit in it (the gesture animated them there) — take the data, skip the reload.
        let pending_echo: Rc<RefCell<Option<Vec<u64>>>> = Rc::new(RefCell::new(None));

        // Selection → refs (translate the native row indices through the snapshot). A
        // single-selection report also feeds `on_selection` as a one-element set, so an app
        // tracking the full selection works unchanged on single-selection backends.
        if self.on_select.is_some() || self.on_selection.is_some() {
            let (on_select, on_selection) = (self.on_select.clone(), self.on_selection.clone());
            let conn = conn.clone();
            cx.on(node, move |ev| match ev {
                Event::SelectionChanged(i) => {
                    if let Some(f) = &on_select
                        && let Some(r) = conn.select_ref(*i as usize)
                    {
                        f(r);
                    }
                    if let Some(f) = &on_selection
                        && let Some(r) = conn.select_ref(*i as usize)
                    {
                        f(vec![r]);
                    }
                }
                Event::SelectionSet(rows) => {
                    if let Some(f) = &on_selection {
                        f(rows
                            .iter()
                            .filter_map(|i| conn.select_ref(*i as usize))
                            .collect());
                    }
                }
                _ => {}
            });
        }

        // A committed native move, deferred to the event drain (never run inside the platform's
        // drop callback): hand the app the same (from, to) the native side animated.
        if let Some(on_reorder) = self.on_reorder.clone() {
            cx.on(node, move |ev| {
                if let Event::ListReorder { from, to } = ev {
                    on_reorder(*from, *to);
                }
            });
        }

        // A committed native delete, deferred to the event drain (never run inside the
        // platform's swipe callback): hand the app the row the native side animated away.
        if let Some(on_delete) = self.on_delete.clone() {
            cx.on(node, move |ev| {
                if let Event::ListDelete(index) = ev {
                    on_delete(*index);
                }
            });
        }

        // The type-erased driver day-core drives on cell pulls.
        let driver = ListDriver {
            row_height: self.row_height,
            len: {
                let conn = conn.clone();
                Box::new(move || conn.len())
            },
            token_at: {
                let conn = conn.clone();
                Box::new(move |i| conn.token_at(i))
            },
            build: {
                let (conn, build_row) = (conn.clone(), self.build_row.clone());
                Box::new(move |index, anchor| {
                    let scope = Scope::child();
                    let conn = conn.clone();
                    let build_row = build_row.clone();
                    let rebind = scope.enter(move || {
                        // Native callbacks can deliver a stale index mid-animation; an
                        // out-of-range pull yields an empty row instead of panicking.
                        let Some(slot) = conn.slot_at(index) else {
                            return Rc::new(|_: usize| {}) as Rc<dyn Fn(usize)>;
                        };
                        let mut rowcx = BuildCx::new(anchor);
                        build_row(slot).build(&mut rowcx);
                        // Rebind on recycle: point the slot at the cell's new row.
                        Rc::new(move |i: usize| conn.rebind(&slot, i)) as Rc<dyn Fn(usize)>
                    });
                    BuiltRow { scope, rebind }
                })
            },
            delete: self.deletable.then(|| ListDeleteDriver {
                can_delete: {
                    let guard = self.delete_guard.clone();
                    Box::new(move |i| guard.as_ref().is_none_or(|g| g(i)))
                },
                // Commit: drop the row from the snapshot NOW (subsequent len/token_at/bind_row
                // serve the shorter list while the native removal animates), arm the echo skip,
                // and defer the app's callback.
                deleted: {
                    let (conn, echo) = (conn.clone(), pending_echo.clone());
                    Box::new(move |index| {
                        if index >= conn.len() {
                            return;
                        }
                        conn.commit_delete(index);
                        *echo.borrow_mut() = Some(conn.tokens_now());
                        enqueue_event(rnode_to_id(node), Event::ListDelete(index));
                    })
                },
            }),
            reorder: self.reorderable.then(|| ListReorderDriver {
                // The guard's verdict, encoded for the sync seam: accepted index or -1.
                can_move: {
                    let guard = self.reorder_guard.clone();
                    Box::new(move |from, to| match guard.as_ref().map(|g| g(from, to)) {
                        None | Some(Reorder::Allow) => to as i64,
                        Some(Reorder::Deny) => -1,
                        Some(Reorder::Retarget(i)) => i as i64,
                    })
                },
                // Commit: rotate the snapshot NOW (subsequent len/token_at/bind_row serve the
                // new order while the native move animates), arm the echo skip, and defer the
                // app's callback through the event queue.
                moved: {
                    let (conn, echo) = (conn.clone(), pending_echo.clone());
                    Box::new(move |from, to| {
                        if from >= conn.len() || to >= conn.len() {
                            return;
                        }
                        conn.commit_move(from, to);
                        *echo.borrow_mut() = Some(conn.tokens_now());
                        enqueue_event(rnode_to_id(node), Event::ListReorder { from, to });
                    })
                },
            }),
        };
        install_list(node, driver);

        // Keep the snapshot current and tell the native host to re-query on changes. The compute
        // is the TRACKED refresh; for a store source whose values flow through per-field
        // notifications, an unchanged row set skips the native reload entirely — a field edit
        // costs the one control it patched, not a visible-rows rebind.
        {
            let (conn, echo, stick) = (conn.clone(), pending_echo.clone(), self.stick_to_bottom);
            let initial = {
                let conn = conn.clone();
                day_reactive::untrack(move || conn.refresh())
            };
            // The native host attached against an EMPTY snapshot (install_list ran before this
            // prime), so tell it the real row set now — reload, deliberately without the
            // auto-scroll a later sticky change gets. Skipping this rendered every list blank
            // until its first data CHANGE, while the synthetic rail kept passing: the
            // walkthrough drove the driver directly and never noticed.
            list_reload(node);
            let last: RefCell<Vec<u64>> = RefCell::new(initial);
            let conn2 = conn.clone();
            watch(
                move || conn2.refresh(),
                move |toks: &Vec<u64>, _| {
                    let is_echo = pending_echo.borrow_mut().take().is_some_and(|e| e == *toks);
                    let unchanged = !conn.values_flow_by_reload() && *last.borrow() == *toks;
                    *last.borrow_mut() = toks.clone();
                    if !is_echo && !unchanged {
                        match conn.take_row_events() {
                            Some(deltas) if !deltas.is_empty() => list_splice(node, deltas),
                            Some(_) => {}
                            None => list_reload(node),
                        }
                        if stick {
                            list_scroll_to_end(node);
                        }
                    } else {
                        // Consume events the host must not see twice (its own echo, or a
                        // change whose set landed identical).
                        let _ = conn.take_row_events();
                    }
                },
            );
            let _ = echo;
        }

        // Imperative scroll-to-end: each `trigger.notify()` re-runs this watch (the trigger's
        // signal is the only tracked dep), whose callback scrolls the native list to its last row.
        // `watch` never fires for the initial run, so building the list does not force a scroll.
        if let Some(trigger) = self.scroll_to_end {
            watch(
                move || trigger.track(),
                move |_: &(), _| list_scroll_to_end(node),
            );
        }

        // Programmatic scroll-to-row: every `Some(row)` write scrolls that row into view
        // (`watch` fires per write — repeats of the same row re-fire; the initial build never
        // scrolls).
        if let Some(sig) = self.scroll_to_row {
            watch(
                move || sig.get(),
                move |row: &Option<usize>, _| {
                    if let Some(row) = row {
                        list_scroll_to_row(node, *row);
                    }
                },
            );
        }

        // Programmatic selection sync: re-runs whenever the closure's tracked reads change
        // (`watch`, so the initial build doesn't clobber a toolkit-default selection).
        if let Some(rows) = self.selected_rows {
            watch(
                move || rows(),
                move |rows: &Vec<usize>, _| day_core::list_set_selected(node, rows.clone()),
            );
        }
        node
    }
}

// ---------------------------------------------------------------------------
// Model-driven rows (feature "model", docs/model.md): a day-model store as a RowSource
// ---------------------------------------------------------------------------

#[cfg(feature = "model")]
mod model_rows {
    use super::*;
    use day_model::{Elem, Identified, Keyed, Source, Store};

    /// One recycled row's connection to its store: the slot a model list's row builder
    /// receives. `Copy`, and a day-model [`Source`], so `#[derive(Observable)]` accessors hang
    /// off it — `text_field(slot.name())` binds two-way and FOLLOWS the slot to its next row
    /// when the cell recycles, because the slot resolves its row on every operation.
    pub struct ModelSlot<T: 'static> {
        store: Store<Keyed<T>>,
        cur: Signal<u64>,
    }

    impl<T> Clone for ModelSlot<T> {
        fn clone(&self) -> Self {
            *self
        }
    }
    impl<T> Copy for ModelSlot<T> {}

    impl<T: Identified + Clone + 'static> ModelSlot<T> {
        /// A slot pointed at `key` — for ROW SOURCES implemented outside this crate (a live
        /// query). Application code receives slots from `list`/`each` instead.
        pub fn for_key(store: Store<Keyed<T>>, key: u64) -> ModelSlot<T> {
            ModelSlot {
                store,
                cur: Signal::new(key),
            }
        }
        /// The recycle write, for external row sources: point this slot at another key.
        pub fn rebind_key(&self, key: u64) {
            self.cur.set_if_changed(key);
        }
    }

    impl<T: Identified + 'static> ModelSlot<T> {
        /// The row's key right now — TRACKED, so a closure reading it follows recycling
        /// (a context-menu action captures the slot and acts on whichever row the cell shows
        /// when it runs).
        pub fn key(self) -> u64 {
            self.cur.get()
        }
        /// The current row as a plain element handle — for `exists()` guards or handing to
        /// code outside the row. TRACKED via the slot, so it follows recycling too.
        pub fn item(self) -> Elem<T> {
            self.store.elem(self.cur.get())
        }
        fn elem_now(self) -> Elem<T> {
            self.store.elem(self.cur.get_untracked())
        }
    }

    impl<T: Identified + 'static> Source<T> for ModelSlot<T> {
        // The whole point: the slot's location is wherever the cell currently sits.
        const DYNAMIC: bool = true;
        fn track_extra(self) {
            self.cur.track();
        }
        fn path(self) -> day_model::Path {
            self.elem_now().path()
        }
        fn node(self) -> day_model::NodeId {
            self.elem_now().node()
        }
        fn components(self, out: &mut Vec<u64>) {
            self.elem_now().components(out);
        }
        fn with_value_untracked<R>(self, f: impl FnOnce(Option<&T>) -> R) -> R {
            self.elem_now().with_value_untracked(f)
        }
        fn update_value(self, f: impl FnOnce(&mut T)) -> bool {
            self.elem_now().update_value(f)
        }
        fn bump_version(self) {
            self.elem_now().bump_version();
        }
    }

    /// A store IS a row source: collection order, rows keyed by the store's own keys.
    impl<T: Identified + Clone + 'static> RowSource for Store<Keyed<T>> {
        type Slot = ModelSlot<T>;
        type Ref = Elem<T>;
        type Conn = StoreConn<T>;
        fn connect(self) -> StoreConn<T> {
            StoreConn {
                store: self,
                rows: None,
                keys: RefCell::new(Vec::new()),
            }
        }
    }

    /// A display projection over a store: `store.rows(move || ordered_keys())`. The projection
    /// is a TRACKED read of key ids — read only the fields the ORDER depends on, and a write to
    /// any other field cannot re-run it.
    pub struct Rows<T: 'static> {
        store: Store<Keyed<T>>,
        f: Rc<dyn Fn() -> Vec<u64>>,
    }

    impl<T: Identified + Clone + 'static> RowSource for Rows<T> {
        type Slot = ModelSlot<T>;
        type Ref = Elem<T>;
        type Conn = StoreConn<T>;
        fn connect(self) -> StoreConn<T> {
            StoreConn {
                store: self.store,
                rows: Some(self.f),
                keys: RefCell::new(Vec::new()),
            }
        }
    }

    /// `store.rows(projection)` — the display-ordered row source for [`list`]/[`each`].
    pub trait StoreRows<T: 'static> {
        fn rows(self, f: impl Fn() -> Vec<u64> + 'static) -> Rows<T>;
    }

    impl<T: Identified + Clone + 'static> StoreRows<T> for Store<Keyed<T>> {
        fn rows(self, f: impl Fn() -> Vec<u64> + 'static) -> Rows<T> {
            Rows {
                store: self,
                f: Rc::new(f),
            }
        }
    }

    /// The store connection: its snapshot is the display KEYS — no item is ever cloned.
    pub struct StoreConn<T: 'static> {
        store: Store<Keyed<T>>,
        rows: Option<Rc<dyn Fn() -> Vec<u64>>>,
        keys: RefCell<Vec<u64>>,
    }

    impl<T: Identified + Clone + 'static> RowConn for StoreConn<T> {
        type Slot = ModelSlot<T>;
        type Ref = Elem<T>;

        fn refresh(&self) -> Vec<u64> {
            let keys = match &self.rows {
                Some(f) => f(),
                None => self.store.keys(),
            };
            *self.keys.borrow_mut() = keys.clone();
            keys
        }
        fn len(&self) -> usize {
            self.keys.borrow().len()
        }
        fn token_at(&self, index: usize) -> u64 {
            self.keys.borrow().get(index).copied().unwrap_or(0)
        }
        fn tokens_now(&self) -> Vec<u64> {
            self.keys.borrow().clone()
        }
        fn slot_at(&self, index: usize) -> Option<ModelSlot<T>> {
            let key = self.keys.borrow().get(index).copied()?;
            Some(ModelSlot {
                store: self.store,
                cur: Signal::new(key),
            })
        }
        fn rebind(&self, slot: &ModelSlot<T>, index: usize) {
            if let Some(key) = self.keys.borrow().get(index).copied() {
                // Values flow through the store's own notifications; the rebind is only the
                // key, and an unchanged key is a no-op.
                slot.cur.set_if_changed(key);
            }
        }
        fn select_ref(&self, index: usize) -> Option<Elem<T>> {
            let key = self.keys.borrow().get(index).copied()?;
            Some(self.store.elem(key))
        }
        fn values_flow_by_reload(&self) -> bool {
            false
        }
        fn commit_move(&self, from: usize, to: usize) {
            let mut keys = self.keys.borrow_mut();
            if from >= keys.len() || to >= keys.len() {
                return;
            }
            let k = keys.remove(from);
            keys.insert(to, k);
        }
        fn commit_delete(&self, index: usize) {
            let mut keys = self.keys.borrow_mut();
            if index >= keys.len() {
                return;
            }
            keys.remove(index);
        }
    }
}

#[cfg(feature = "model")]
pub use model_rows::{ModelSlot, Rows, StoreRows};

// --- Typed builders, forwarded through `Decorated` (docs/api-style.md) ---

/// [`When`]'s own builders, reachable THROUGH a decoration (§5.2): `Decorated` forwards them
/// to the piece it wraps, so generic modifiers and typed ones chain in any order.
pub trait WhenBuilder: Sized {
    fn otherwise<P: Piece>(self, build_arm: impl Fn() -> P + 'static) -> Self;
}

impl WhenBuilder for When {
    fn otherwise<P: Piece>(self, build_arm: impl Fn() -> P + 'static) -> Self {
        When::otherwise(self, build_arm)
    }
}

impl<Inner: WhenBuilder + Piece> WhenBuilder for Decorated<Inner> {
    fn otherwise<P: Piece>(self, build_arm: impl Fn() -> P + 'static) -> Self {
        self.map_inner(|inner_piece| inner_piece.otherwise(build_arm))
    }
}

/// [`List`]'s own builders, reachable THROUGH a decoration (§5.2): `Decorated` forwards them
/// to the piece it wraps, so generic modifiers and typed ones chain in any order.
pub trait ListBuilder<S: RowSource + 'static>: Sized {
    fn row_height(self, h: RowHeight) -> Self;
    fn on_select(self, f: impl Fn(S::Ref) + 'static) -> Self;
    fn multi_select(self, on: bool) -> Self;
    fn on_selection(self, f: impl Fn(Vec<S::Ref>) + 'static) -> Self;
    fn selected_rows(self, rows: impl Fn() -> Vec<usize> + 'static) -> Self;
    fn scroll_to_end(self, trigger: day_reactive::Trigger) -> Self;
    fn stick_to_bottom(self, on: bool) -> Self;
    fn scroll_to_row(self, sig: Signal<Option<usize>>) -> Self;
    fn reorderable(self, on: bool) -> Self;
    fn on_reorder(self, f: impl Fn(usize, usize) + 'static) -> Self;
    fn reorder_guard(self, g: impl Fn(usize, usize) -> Reorder + 'static) -> Self;
    fn deletable(self, on: bool) -> Self;
    fn delete_label(self, text: impl Into<String>) -> Self;
    fn on_delete(self, f: impl Fn(usize) + 'static) -> Self;
    fn delete_guard(self, g: impl Fn(usize) -> bool + 'static) -> Self;
}

impl<S: RowSource + 'static> ListBuilder<S> for List<S> {
    fn row_height(self, h: RowHeight) -> Self {
        List::row_height(self, h)
    }
    fn on_select(self, f: impl Fn(S::Ref) + 'static) -> Self {
        List::on_select(self, f)
    }
    fn multi_select(self, on: bool) -> Self {
        List::multi_select(self, on)
    }
    fn on_selection(self, f: impl Fn(Vec<S::Ref>) + 'static) -> Self {
        List::on_selection(self, f)
    }
    fn selected_rows(self, rows: impl Fn() -> Vec<usize> + 'static) -> Self {
        List::selected_rows(self, rows)
    }
    fn scroll_to_end(self, trigger: day_reactive::Trigger) -> Self {
        List::scroll_to_end(self, trigger)
    }
    fn stick_to_bottom(self, on: bool) -> Self {
        List::stick_to_bottom(self, on)
    }
    fn scroll_to_row(self, sig: Signal<Option<usize>>) -> Self {
        List::scroll_to_row(self, sig)
    }
    fn reorderable(self, on: bool) -> Self {
        List::reorderable(self, on)
    }
    fn on_reorder(self, f: impl Fn(usize, usize) + 'static) -> Self {
        List::on_reorder(self, f)
    }
    fn reorder_guard(self, g: impl Fn(usize, usize) -> Reorder + 'static) -> Self {
        List::reorder_guard(self, g)
    }
    fn deletable(self, on: bool) -> Self {
        List::deletable(self, on)
    }
    fn delete_label(self, text: impl Into<String>) -> Self {
        List::delete_label(self, text)
    }
    fn on_delete(self, f: impl Fn(usize) + 'static) -> Self {
        List::on_delete(self, f)
    }
    fn delete_guard(self, g: impl Fn(usize) -> bool + 'static) -> Self {
        List::delete_guard(self, g)
    }
}

impl<S: RowSource + 'static, Inner: ListBuilder<S> + Piece> ListBuilder<S> for Decorated<Inner> {
    fn row_height(self, h: RowHeight) -> Self {
        self.map_inner(|inner_piece| inner_piece.row_height(h))
    }
    fn on_select(self, f: impl Fn(S::Ref) + 'static) -> Self {
        self.map_inner(|inner_piece| inner_piece.on_select(f))
    }
    fn multi_select(self, on: bool) -> Self {
        self.map_inner(|inner_piece| inner_piece.multi_select(on))
    }
    fn on_selection(self, f: impl Fn(Vec<S::Ref>) + 'static) -> Self {
        self.map_inner(|inner_piece| inner_piece.on_selection(f))
    }
    fn selected_rows(self, rows: impl Fn() -> Vec<usize> + 'static) -> Self {
        self.map_inner(|inner_piece| inner_piece.selected_rows(rows))
    }
    fn scroll_to_end(self, trigger: day_reactive::Trigger) -> Self {
        self.map_inner(|inner_piece| inner_piece.scroll_to_end(trigger))
    }
    fn stick_to_bottom(self, on: bool) -> Self {
        self.map_inner(|inner_piece| inner_piece.stick_to_bottom(on))
    }
    fn scroll_to_row(self, sig: Signal<Option<usize>>) -> Self {
        self.map_inner(|inner_piece| inner_piece.scroll_to_row(sig))
    }
    fn reorderable(self, on: bool) -> Self {
        self.map_inner(|inner_piece| inner_piece.reorderable(on))
    }
    fn on_reorder(self, f: impl Fn(usize, usize) + 'static) -> Self {
        self.map_inner(|inner_piece| inner_piece.on_reorder(f))
    }
    fn reorder_guard(self, g: impl Fn(usize, usize) -> Reorder + 'static) -> Self {
        self.map_inner(|inner_piece| inner_piece.reorder_guard(g))
    }
    fn deletable(self, on: bool) -> Self {
        self.map_inner(|inner_piece| inner_piece.deletable(on))
    }
    fn delete_label(self, text: impl Into<String>) -> Self {
        self.map_inner(|inner_piece| inner_piece.delete_label(text))
    }
    fn on_delete(self, f: impl Fn(usize) + 'static) -> Self {
        self.map_inner(|inner_piece| inner_piece.on_delete(f))
    }
    fn delete_guard(self, g: impl Fn(usize) -> bool + 'static) -> Self {
        self.map_inner(|inner_piece| inner_piece.delete_guard(g))
    }
}
