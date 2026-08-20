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
use day_spec::{Event, kinds};

// ---------------------------------------------------------------------------
// Structure: when / each (§5.3–§5.4)
// ---------------------------------------------------------------------------

/// Reactive conditional subtree. The anchor is a layout-transparent group; the active arm
/// lives in its own child scope, disposed on switch (§4.3).
pub fn when<P: Piece>(
    cond: impl Fn() -> bool + 'static,
    build_arm: impl Fn() -> P + 'static,
) -> AnyPiece {
    piece_fn(move |cx| {
        let anchor = cx.layout_only(
            Rc::new(PassThrough),
            Flex {
                is_group: true,
                ..Default::default()
            },
            Boundary::No,
        );
        let state: Rc<RefCell<Option<Scope>>> = Rc::new(RefCell::new(None));
        let build_arm = Rc::new(build_arm);

        let mount = {
            let state = state.clone();
            let build_arm = build_arm.clone();
            move |on: bool| {
                // Same lifetime hazard as `each`'s sync, and the same answer — see the note there.
                if !with_tree(|t| t.node_exists(anchor)) {
                    return;
                }
                if on {
                    let scope = Scope::child();
                    scope.enter(|| {
                        let mut cx = BuildCx::new(anchor);
                        let _ = build_arm().build(&mut cx);
                    });
                    *state.borrow_mut() = Some(scope);
                } else if let Some(scope) = state.borrow_mut().take() {
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
            }
        };

        let initial = day_reactive::untrack(&cond);
        mount(initial);
        watch(cond, move |now, old| {
            if Some(now) != old {
                mount(*now);
            }
        });
        anchor
    })
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

/// Type-erased slot writer: feeds a surviving row's `ItemSlot` signal a new `&T` (§5.4).
type SlotWriter = Box<dyn Fn(&dyn std::any::Any)>;

struct EachRow<K> {
    key: K,
    scope: Scope,
    root: RNode,
    sig_set: SlotWriter,
}

/// Reactive keyed collection (§5.4): keyed diff, per-key child scopes, slot writes for
/// surviving keys, debug key-uniqueness assertion.
pub fn each<T, K, P>(
    items: impl Fn() -> Vec<T> + 'static,
    key_of: impl Fn(&T) -> K + 'static,
    build_row: impl Fn(ItemSlot<T, K>) -> P + 'static,
) -> AnyPiece
where
    T: Clone + 'static,
    K: Eq + Hash + Clone + 'static,
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
        let rows: Rc<RefCell<Vec<EachRow<K>>>> = Rc::new(RefCell::new(Vec::new()));
        let key_of = Rc::new(key_of);
        let build_row = Rc::new(build_row);

        let sync = {
            let rows = rows.clone();
            let key_of = key_of.clone();
            let build_row = build_row.clone();
            move |new_items: &Vec<T>| {
                // The reaction driving this closure can outlive the subtree it builds into: a
                // page swapped out of a nav, a `when` arm closed above us, and the anchor is gone
                // while the reaction is still subscribed. Building into a removed parent panics in
                // `Tree::attach` ("attach to missing parent"), and that panic is contained at the
                // native event boundary — which ABANDONS THE REST OF THE DRAIN, so every reaction
                // queued behind it (including the live subtree's own) is skipped and the UI stops
                // updating until something rebuilds it. An anchor that is no longer in the tree has
                // nothing to sync, so leave quietly and let the disposal that removed it finish.
                if !with_tree(|t| t.node_exists(anchor)) {
                    return;
                }
                let new_keys: Vec<K> = new_items.iter().map(|t| key_of(t)).collect();
                if cfg!(debug_assertions) {
                    let mut seen = HashSet::new();
                    for k in &new_keys {
                        assert!(seen.insert(k.clone()), "day: duplicate key in `each` diff");
                    }
                }
                let mut old = std::mem::take(&mut *rows.borrow_mut());
                let mut next: Vec<EachRow<K>> = Vec::with_capacity(new_keys.len());
                for (item, k) in new_items.iter().zip(new_keys.iter()) {
                    if let Some(pos) = old.iter().position(|r| &r.key == k) {
                        let row = old.remove(pos);
                        // Surviving key: one unconditional slot write (§5.4).
                        (row.sig_set)(item as &dyn std::any::Any);
                        next.push(row);
                    } else {
                        let scope = Scope::child();
                        let (root, sig) = scope.enter(|| {
                            let sig = Signal::new(item.clone());
                            let keysig = Signal::new(k.clone());
                            let slot = ItemSlot { sig, key: keysig };
                            let mut cx = BuildCx::new(anchor);
                            (build_row(slot).build(&mut cx), sig)
                        });
                        next.push(EachRow {
                            key: k.clone(),
                            scope,
                            root,
                            sig_set: Box::new(move |any| {
                                if let Some(v) = any.downcast_ref::<T>() {
                                    sig.set(v.clone());
                                }
                            }),
                        });
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

        let initial = day_reactive::untrack(&items);
        sync(&initial);
        watch(items, move |new, _| sync(new));
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
pub fn with_environment<T: Clone + 'static>(
    value: T,
    content: impl FnOnce() -> AnyPiece + 'static,
) -> AnyPiece {
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

/// Applies a fresh items snapshot (refresh the data-source view + tell the native host to reload).
type RefreshFn<T> = Rc<dyn Fn(&Vec<T>)>;

/// A native recycling list: the platform widget owns scrolling + cell reuse; Day builds each
/// visible row once and *rebinds* it (a slot-write into its `ItemSlot`) as cells recycle.
/// Shares the `ItemSlot` row contract with [`each`]; migrating is a one-word change.
pub struct List<T: 'static, K: 'static> {
    items: Rc<dyn Fn() -> Vec<T>>,
    key_of: Rc<dyn Fn(&T) -> K>,
    build_row: Rc<dyn Fn(ItemSlot<T, K>) -> AnyPiece>,
    row_height: RowHeight,
    on_select: Option<Rc<dyn Fn(K)>>,
    on_selection: Option<Rc<dyn Fn(Vec<K>)>>,
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

/// Build a recycling list from a reactive items closure, a key function, and a row builder.
pub fn list<T, K, P>(
    items: impl Fn() -> Vec<T> + 'static,
    key_of: impl Fn(&T) -> K + 'static,
    build_row: impl Fn(ItemSlot<T, K>) -> P + 'static,
) -> List<T, K>
where
    T: Clone + 'static,
    K: Clone + Hash + 'static,
    P: Piece,
{
    List {
        items: Rc::new(items),
        key_of: Rc::new(key_of),
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

impl<T: Clone + 'static, K: Clone + Hash + 'static> List<T, K> {
    /// Row sizing: `Uniform(h)` (fastest) or `Automatic` (self-sizing).
    pub fn row_height(mut self, h: RowHeight) -> Self {
        self.row_height = h;
        self
    }
    /// Called with the selected row's key when the native list reports a selection.
    pub fn on_select(mut self, f: impl Fn(K) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }
    /// Allow selecting several rows at once, where the toolkit supports it (docs/list.md has
    /// the matrix; single-selection backends fall back to one row at a time). Every selection
    /// change calls [`Self::on_selection`] with the FULL set of selected keys.
    pub fn multi_select(mut self, on: bool) -> Self {
        self.multi_select = on;
        self
    }
    /// Called with the full set of selected keys (row order, empty = cleared) whenever the
    /// selection changes — the multi-select analogue of [`Self::on_select`]. Also fired by
    /// single-selection backends with a one-element (or empty) set.
    pub fn on_selection(mut self, f: impl Fn(Vec<K>) + 'static) -> Self {
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

impl<T: Clone + 'static, K: Clone + Hash + 'static> Piece for List<T, K> {
    fn build(self, cx: &mut BuildCx) -> RNode {
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

        // The data-source's view of the world: the current items + their tokens, refreshed by a
        // bind on the items closure. The native host queries these synchronously; the driver's
        // build/rebind closures read the same snapshot.
        let snapshot: Rc<RefCell<Vec<T>>> = Rc::new(RefCell::new(Vec::new()));
        let tokens: Rc<RefCell<Vec<u64>>> = Rc::new(RefCell::new(Vec::new()));
        // After a committed native move, the token order the app's own data update is expected to
        // echo back. When the refresh below sees exactly this order, the native rows already sit
        // in it (the drop animated them there) — update the snapshot and skip the redundant
        // `Reload` instead of visibly re-binding every visible row.
        let pending_echo: Rc<RefCell<Option<Vec<u64>>>> = Rc::new(RefCell::new(None));

        // Selection → keys (translate the native row indices through the snapshot). A
        // single-selection report also feeds `on_selection` as a one-element set, so an app
        // tracking the full selection works unchanged on single-selection backends.
        if self.on_select.is_some() || self.on_selection.is_some() {
            let (on_select, on_selection) = (self.on_select.clone(), self.on_selection.clone());
            let (snap, key_of) = (snapshot.clone(), self.key_of.clone());
            cx.on(node, move |ev| match ev {
                Event::SelectionChanged(i) => {
                    let snap = snap.borrow();
                    if let Some(item) = snap.get(*i as usize) {
                        if let Some(f) = &on_select {
                            f(key_of(item));
                        }
                        if let Some(f) = &on_selection {
                            f(vec![key_of(item)]);
                        }
                    }
                }
                Event::SelectionSet(rows) => {
                    if let Some(f) = &on_selection {
                        let snap = snap.borrow();
                        f(rows
                            .iter()
                            .filter_map(|i| snap.get(*i as usize).map(&*key_of))
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
                let s = snapshot.clone();
                Box::new(move || s.borrow().len())
            },
            token_at: {
                let t = tokens.clone();
                Box::new(move |i| t.borrow().get(i).copied().unwrap_or(0))
            },
            build: {
                let (snapshot, key_of, build_row) = (
                    snapshot.clone(),
                    self.key_of.clone(),
                    self.build_row.clone(),
                );
                Box::new(move |index, anchor| {
                    let scope = Scope::child();
                    let rebind = scope.enter(|| {
                        // Native callbacks can deliver a stale index mid-animation (the
                        // `deleted`/`moved` arms below guard for the same reason); an
                        // out-of-range pull yields an empty row instead of panicking.
                        let Some(item) = snapshot.borrow().get(index).cloned() else {
                            return Rc::new(|_: usize| {}) as Rc<dyn Fn(usize)>;
                        };
                        let sig = Signal::new(item.clone());
                        let keysig = Signal::new(key_of(&item));
                        let slot = ItemSlot { sig, key: keysig };
                        let mut rowcx = BuildCx::new(anchor);
                        build_row(slot).build(&mut rowcx);
                        // Rebind on recycle: one slot-write of the new row's item + key.
                        let (snap, key_of) = (snapshot.clone(), key_of.clone());
                        Rc::new(move |i: usize| {
                            let Some(it) = snap.borrow().get(i).cloned() else {
                                return; // stale recycle index — skip, next bind corrects it
                            };
                            keysig.set(key_of(&it));
                            sig.set(it);
                        }) as Rc<dyn Fn(usize)>
                    });
                    BuiltRow { scope, rebind }
                })
            },
            delete: self.deletable.then(|| ListDeleteDriver {
                can_delete: {
                    let guard = self.delete_guard.clone();
                    Box::new(move |i| guard.as_ref().is_none_or(|g| g(i)))
                },
                // Commit: drop the row from the snapshot + tokens NOW (subsequent
                // len/token_at/bind_row serve the shorter list while the native removal
                // animates), arm the echo skip, and defer the app's callback.
                deleted: {
                    let (snapshot, tokens, echo) =
                        (snapshot.clone(), tokens.clone(), pending_echo.clone());
                    Box::new(move |index| {
                        {
                            let mut snap = snapshot.borrow_mut();
                            let mut toks = tokens.borrow_mut();
                            if index >= snap.len() {
                                return;
                            }
                            snap.remove(index);
                            toks.remove(index);
                            *echo.borrow_mut() = Some(toks.clone());
                        }
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
                // Commit: rotate the snapshot + tokens NOW (subsequent len/token_at/bind_row
                // serve the new order while the native move animates), arm the echo skip, and
                // defer the app's callback through the event queue.
                moved: {
                    let (snapshot, tokens, echo) =
                        (snapshot.clone(), tokens.clone(), pending_echo.clone());
                    Box::new(move |from, to| {
                        {
                            let mut snap = snapshot.borrow_mut();
                            let mut toks = tokens.borrow_mut();
                            if from >= snap.len() || to >= snap.len() {
                                return;
                            }
                            let item = snap.remove(from);
                            snap.insert(to, item);
                            let tok = toks.remove(from);
                            toks.insert(to, tok);
                            *echo.borrow_mut() = Some(toks.clone());
                        }
                        enqueue_event(rnode_to_id(node), Event::ListReorder { from, to });
                    })
                },
            }),
        };
        install_list(node, driver);

        // Keep the snapshot current and tell the native host to re-query on every change.
        // `watch` (not `bind`) so `T` need not be `PartialEq` — matching `each`; run once eagerly.
        let refresh: RefreshFn<T> = {
            let (snapshot, tokens, key_of, echo) = (
                snapshot.clone(),
                tokens.clone(),
                self.key_of.clone(),
                pending_echo.clone(),
            );
            Rc::new(move |its: &Vec<T>| {
                let new_tokens: Vec<u64> = its.iter().map(|t| key_token(&key_of(t))).collect();
                // The echo of a committed native move: the app rotated its data to exactly the
                // order the drop animated the rows into. The natives are already right — take the
                // data, skip the reload. Any OTHER change (even one arriving while an echo is
                // armed) reloads normally.
                let is_echo = echo.borrow_mut().take().is_some_and(|e| e == new_tokens);
                *tokens.borrow_mut() = new_tokens;
                *snapshot.borrow_mut() = its.clone();
                if !is_echo {
                    list_reload(node);
                }
            })
        };
        let items = self.items.clone();
        let initial = day_reactive::untrack(|| items());
        refresh(&initial);
        {
            // On subsequent data changes: reload, then (if sticking) follow the newest row. The
            // initial eager `refresh` above deliberately does NOT auto-scroll.
            let (refresh, items, stick) = (refresh.clone(), items.clone(), self.stick_to_bottom);
            watch(
                move || items(),
                move |new: &Vec<T>, _| {
                    refresh(new);
                    if stick {
                        list_scroll_to_end(node);
                    }
                },
            );
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
