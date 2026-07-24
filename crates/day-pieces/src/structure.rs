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
    scroll_to_end: Option<day_reactive::Trigger>,
    stick_to_bottom: bool,
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
        scroll_to_end: None,
        stick_to_bottom: false,
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
}

impl<T: Clone + 'static, K: Clone + Hash + 'static> Piece for List<T, K> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let props = ListProps {
            row_height: self.row_height,
            selectable: self.on_select.is_some(),
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

        // Selection → key (translate the native row index through the snapshot).
        if let Some(on_select) = self.on_select.clone() {
            let (snap, key_of) = (snapshot.clone(), self.key_of.clone());
            cx.on(node, move |ev| {
                if let Event::SelectionChanged(i) = ev
                    && let Some(item) = snap.borrow().get(*i as usize)
                {
                    on_select(key_of(item));
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
                        let item = snapshot.borrow()[index].clone();
                        let sig = Signal::new(item.clone());
                        let keysig = Signal::new(key_of(&item));
                        let slot = ItemSlot { sig, key: keysig };
                        let mut rowcx = BuildCx::new(anchor);
                        build_row(slot).build(&mut rowcx);
                        // Rebind on recycle: one slot-write of the new row's item + key.
                        let (snap, key_of) = (snapshot.clone(), key_of.clone());
                        Rc::new(move |i: usize| {
                            let it = snap.borrow()[i].clone();
                            keysig.set(key_of(&it));
                            sig.set(it);
                        }) as Rc<dyn Fn(usize)>
                    });
                    BuiltRow { scope, rebind }
                })
            },
        };
        install_list(node, driver);

        // Keep the snapshot current and tell the native host to re-query on every change.
        // `watch` (not `bind`) so `T` need not be `PartialEq` — matching `each`; run once eagerly.
        let refresh: RefreshFn<T> = {
            let (snapshot, tokens, key_of) =
                (snapshot.clone(), tokens.clone(), self.key_of.clone());
            Rc::new(move |its: &Vec<T>| {
                *tokens.borrow_mut() = its.iter().map(|t| key_token(&key_of(t))).collect();
                *snapshot.borrow_mut() = its.clone();
                list_reload(node);
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
        node
    }
}
