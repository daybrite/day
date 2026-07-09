//! day-reactive — the reactive core (DESIGN.md §3.3, §4).
//!
//! Build-once / bind-forever: signals, memos, effects, `bind`, and `watch` over a thread-local
//! generational arena. All handles are `Copy` and `!Send`; the only cross-thread door is
//! [`Setter`]. Writes batch; the drain runs to fixpoint in (priority, scope-depth, creation-seq)
//! order; layout/turn-end callbacks run once after the fixpoint (§3.3's turn state machine).
//!
//! `Signal` is `!Send`:
//! ```compile_fail
//! fn assert_send<T: Send>(_: T) {}
//! let s = day_reactive::Signal::new(1);
//! assert_send(s); // must not compile
//! ```

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;
use std::panic::Location;
use std::rc::Rc;

use slotmap::{Key, SlotMap, new_key_type};

new_key_type! {
    pub struct NodeKey;
    pub struct ScopeKey;
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum NodeState {
    Clean,
    Check,
    Dirty,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NodeKind {
    Signal,
    Memo,
    /// Effects, binds, watches — anything with a re-runnable reaction closure.
    Reaction,
}

/// Type-erased memo comparator (a monomorphized `PartialEq::eq`).
type MemoEq = fn(&dyn Any, &dyn Any) -> bool;

struct Node {
    kind: NodeKind,
    state: NodeState,
    value: Option<Box<dyn Any>>,
    /// Memo recompute (returns boxed new value) — compared with `eq`.
    memo_compute: Option<Rc<dyn Fn() -> Box<dyn Any>>>,
    memo_eq: Option<MemoEq>,
    /// Reaction closure (effect/bind/watch body).
    reaction: Option<Rc<dyn Fn()>>,
    sources: Vec<NodeKey>,
    observers: Vec<NodeKey>,
    #[allow(dead_code)] // ownership is tracked scope→nodes; kept for diagnostics
    scope: ScopeKey,
    /// Ordering: (priority, scope depth, creation seq). Priority 0 = structural binding.
    priority: u8,
    depth: u32,
    seq: u64,
    last_changed: u64,
    last_run: u64,
    queued: bool,
    created_at: &'static Location<'static>,
}

struct ScopeData {
    parent: ScopeKey,
    children: Vec<ScopeKey>,
    nodes: Vec<NodeKey>,
    cleanups: Vec<Box<dyn FnOnce()>>,
    context: HashMap<TypeId, Box<dyn Any>>,
    depth: u32,
}

struct Runtime {
    nodes: SlotMap<NodeKey, Node>,
    scopes: SlotMap<ScopeKey, ScopeData>,
    root_scope: ScopeKey,
    current_scope: ScopeKey,
    /// Observer stack; `None` = untracked frame.
    observers: Vec<Option<NodeKey>>,
    pending: Vec<NodeKey>,
    batch_depth: u32,
    draining: bool,
    tick: u64,
    next_seq: u64,
    scheduler: Option<Rc<dyn Fn()>>,
    schedule_posted: bool,
    turn_end: Vec<Rc<dyn Fn()>>,
    warned_writes: HashSet<*const Location<'static>>,
}

impl Runtime {
    fn new() -> Self {
        let mut scopes = SlotMap::with_key();
        let root_scope = scopes.insert(ScopeData {
            parent: ScopeKey::null(),
            children: Vec::new(),
            nodes: Vec::new(),
            cleanups: Vec::new(),
            context: HashMap::new(),
            depth: 0,
        });
        Runtime {
            nodes: SlotMap::with_key(),
            scopes,
            root_scope,
            current_scope: root_scope,
            observers: Vec::new(),
            pending: Vec::new(),
            batch_depth: 0,
            draining: false,
            tick: 1,
            next_seq: 0,
            scheduler: None,
            schedule_posted: false,
            turn_end: Vec::new(),
            warned_writes: HashSet::new(),
        }
    }
}

thread_local! {
    static RT: RefCell<Runtime> = RefCell::new(Runtime::new());
}

/// Per-drain re-run cap (§4.2): panic in debug, warn-and-defer in release.
const RERUN_CAP: u32 = 100;

fn with_rt<R>(f: impl FnOnce(&mut Runtime) -> R) -> R {
    RT.with(|rt| f(&mut rt.borrow_mut()))
}

// ---------------------------------------------------------------------------
// Graph internals
// ---------------------------------------------------------------------------

#[track_caller]
fn create_node(rt: &mut Runtime, kind: NodeKind, scope: ScopeKey) -> NodeKey {
    let depth = rt.scopes.get(scope).map(|s| s.depth).unwrap_or(0);
    let seq = rt.next_seq;
    rt.next_seq += 1;
    let key = rt.nodes.insert(Node {
        kind,
        state: NodeState::Clean,
        value: None,
        memo_compute: None,
        memo_eq: None,
        reaction: None,
        sources: Vec::new(),
        observers: Vec::new(),
        scope,
        priority: 1,
        depth,
        seq,
        last_changed: 0,
        last_run: 0,
        queued: false,
        created_at: Location::caller(),
    });
    if let Some(s) = rt.scopes.get_mut(scope) {
        s.nodes.push(key);
    }
    key
}

/// Register a tracked read: current observer gains `source` as a dependency.
fn track_read(rt: &mut Runtime, source: NodeKey) {
    if let Some(Some(obs)) = rt.observers.last().copied() {
        if obs == source {
            return;
        }
        let already = rt.nodes[obs].sources.contains(&source);
        if !already {
            rt.nodes[obs].sources.push(source);
            rt.nodes[source].observers.push(obs);
        }
    }
}

fn clear_sources(rt: &mut Runtime, key: NodeKey) {
    let sources = std::mem::take(&mut rt.nodes[key].sources);
    for s in sources {
        if let Some(n) = rt.nodes.get_mut(s)
            && let Some(pos) = n.observers.iter().position(|&o| o == key)
        {
            n.observers.swap_remove(pos);
        }
    }
}

/// Mark downstream after a source changed. Direct observers get `Dirty`; transitive
/// (through memos) get `Check`. Reactions are enqueued.
fn mark_observers(rt: &mut Runtime, source: NodeKey, level: NodeState) {
    // Small explicit stack to avoid recursion borrow issues.
    let mut stack: Vec<(NodeKey, NodeState)> = rt.nodes[source]
        .observers
        .iter()
        .map(|&o| (o, level))
        .collect();
    while let Some((key, level)) = stack.pop() {
        let Some(node) = rt.nodes.get_mut(key) else {
            continue;
        };
        if node.state >= level {
            continue; // already at least this dirty
        }
        node.state = level;
        match node.kind {
            NodeKind::Reaction => {
                if !node.queued {
                    node.queued = true;
                    rt.pending.push(key);
                }
            }
            NodeKind::Memo => {
                for &o in rt.nodes[key].observers.iter() {
                    stack.push((o, NodeState::Check));
                }
            }
            NodeKind::Signal => {}
        }
    }
}

/// Pull-refresh a memo: recompute if (transitively) dirty; bump `last_changed` only on real change.
fn refresh_memo(key: NodeKey) {
    let (state, kind) = match with_rt(|rt| rt.nodes.get(key).map(|n| (n.state, n.kind))) {
        Some(v) => v,
        None => return,
    };
    if kind != NodeKind::Memo || state == NodeState::Clean {
        return;
    }
    if state == NodeState::Check {
        // Refresh sources; only recompute if one actually changed since our last run.
        let (sources, last_run) =
            with_rt(|rt| (rt.nodes[key].sources.clone(), rt.nodes[key].last_run));
        let mut any_changed = false;
        for s in sources {
            refresh_memo(s);
            if with_rt(|rt| {
                rt.nodes
                    .get(s)
                    .map(|n| n.last_changed > last_run)
                    .unwrap_or(false)
            }) {
                any_changed = true;
            }
        }
        if !any_changed {
            with_rt(|rt| {
                if let Some(n) = rt.nodes.get_mut(key) {
                    n.state = NodeState::Clean;
                }
            });
            return;
        }
    }
    // Recompute.
    let compute = with_rt(|rt| rt.nodes[key].memo_compute.clone());
    let Some(compute) = compute else { return };
    with_rt(|rt| clear_sources(rt, key));
    with_rt(|rt| rt.observers.push(Some(key)));
    let new_value = compute();
    with_rt(|rt| {
        rt.observers.pop();
        let tick = rt.tick;
        let node = &mut rt.nodes[key];
        let changed = match (&node.value, node.memo_eq) {
            (Some(old), Some(eq)) => !eq(old.as_ref(), new_value.as_ref()),
            _ => true,
        };
        node.last_run = tick;
        node.state = NodeState::Clean;
        if changed {
            node.value = Some(new_value);
            node.last_changed = tick;
            rt.tick += 1;
        }
        // Downstream was already marked Check when we were invalidated; observers that pull us
        // will see last_changed. Nothing further to do here.
    });
}

/// Run one reaction if it is actually stale.
fn run_reaction(key: NodeKey) {
    let info = with_rt(|rt| {
        rt.nodes.get_mut(key).map(|n| {
            n.queued = false;
            (n.state, n.sources.clone(), n.last_run)
        })
    });
    let Some((state, sources, last_run)) = info else {
        return;
    };
    if state == NodeState::Clean {
        return;
    }
    if state == NodeState::Check {
        let mut any_changed = false;
        for s in &sources {
            refresh_memo(*s);
            if with_rt(|rt| {
                rt.nodes
                    .get(*s)
                    .map(|n| n.last_changed > last_run)
                    .unwrap_or(false)
            }) {
                any_changed = true;
                break;
            }
        }
        if !any_changed {
            with_rt(|rt| {
                if let Some(n) = rt.nodes.get_mut(key) {
                    n.state = NodeState::Clean;
                }
            });
            return;
        }
    }
    let reaction = with_rt(|rt| rt.nodes.get(key).and_then(|n| n.reaction.clone()));
    let Some(reaction) = reaction else { return };
    with_rt(|rt| {
        clear_sources(rt, key);
        if let Some(n) = rt.nodes.get_mut(key) {
            n.state = NodeState::Clean;
        }
        rt.observers.push(Some(key));
    });
    reaction();
    with_rt(|rt| {
        rt.observers.pop();
        let tick = rt.tick;
        rt.tick += 1;
        if let Some(n) = rt.nodes.get_mut(key) {
            n.last_run = tick;
        }
    });
}

/// Drain the pending queue to fixpoint, then run turn-end callbacks once (§3.3 steps 2–3).
pub fn flush_sync() {
    let already = with_rt(|rt| {
        if rt.draining {
            return true;
        }
        rt.draining = true;
        false
    });
    if already {
        return; // re-entrant flush folds into the current drain
    }
    let mut run_counts: HashMap<NodeKey, u32> = HashMap::new();
    loop {
        let mut batch = with_rt(|rt| std::mem::take(&mut rt.pending));
        if batch.is_empty() {
            break;
        }
        // (priority, scope-depth, creation-seq) — owners before descendants.
        with_rt(|rt| {
            batch.sort_by_key(|&k| {
                rt.nodes
                    .get(k)
                    .map(|n| (n.priority, n.depth, n.seq))
                    .unwrap_or((u8::MAX, u32::MAX, u64::MAX))
            })
        });
        for key in batch {
            let count = run_counts.entry(key).or_insert(0);
            *count += 1;
            if *count > RERUN_CAP {
                let loc = with_rt(|rt| rt.nodes.get(key).map(|n| n.created_at));
                if let Some(loc) = loc {
                    if cfg!(debug_assertions) {
                        panic!(
                            "day-reactive: effect created at {loc} re-ran more than {RERUN_CAP} times in one drain (reactive cycle?)"
                        );
                    } else {
                        eprintln!(
                            "day-reactive: effect created at {loc} exceeded the re-run cap; deferring"
                        );
                    }
                }
                continue;
            }
            run_reaction(key);
        }
    }
    let turn_end = with_rt(|rt| {
        rt.draining = false;
        rt.schedule_posted = false;
        rt.turn_end.clone()
    });
    for cb in turn_end {
        cb();
    }
}

/// Reset the runtime to a clean idle state after a panic unwound through a drain or batch — e.g. a
/// reactive-cycle assertion ([`RERUN_CAP`]) that tripped inside a native event callback which the
/// backend *contained* (rather than letting it abort the process across the C ABI — a GTK/Qt signal
/// trampoline can't unwind). The in-flight `pending` work and the observer stack are dropped (the next
/// interaction re-derives them); persistent registrations (effects, memos, turn-end hooks) are kept.
pub fn recover_from_panic() {
    with_rt(|rt| {
        rt.draining = false;
        rt.schedule_posted = false;
        rt.batch_depth = 0;
        rt.pending.clear();
        rt.observers.clear();
    });
}

/// After a write: schedule work. Inside a batch or drain, the fixpoint picks it up; outside,
/// post a coalesced drain through the installed scheduler (§3.3 step 3).
fn schedule_after_write(rt: &mut Runtime) -> Option<Rc<dyn Fn()>> {
    if rt.batch_depth > 0 || rt.draining {
        return None;
    }
    if rt.schedule_posted {
        return None;
    }
    rt.schedule_posted = true;
    rt.scheduler.clone()
}

fn signal_write_boxed(key: NodeKey, apply: impl FnOnce(&mut Box<dyn Any>) -> bool) {
    let poster = with_rt(|rt| {
        let node = rt.nodes.get_mut(key)?;
        let value = node.value.as_mut()?;
        let changed = apply(value);
        if !changed {
            return None;
        }
        node.last_changed = rt.tick;
        rt.tick += 1;
        mark_observers(rt, key, NodeState::Dirty);
        schedule_after_write(rt)
    });
    if let Some(post) = poster {
        post();
    }
}

// ---------------------------------------------------------------------------
// Public: batching / scheduling / turn end
// ---------------------------------------------------------------------------

/// Run `f` in a batch: writes coalesce; the synchronous fixpoint drain runs at batch close.
pub fn batch<R>(f: impl FnOnce() -> R) -> R {
    with_rt(|rt| rt.batch_depth += 1);
    let r = f();
    let should_drain = with_rt(|rt| {
        rt.batch_depth -= 1;
        rt.batch_depth == 0 && !rt.draining && !rt.pending.is_empty()
    });
    if should_drain {
        flush_sync();
    }
    r
}

/// Run `f` without tracking reads.
pub fn untrack<R>(f: impl FnOnce() -> R) -> R {
    with_rt(|rt| rt.observers.push(None));
    let r = f();
    with_rt(|rt| {
        rt.observers.pop();
    });
    r
}

/// Install "post a drain on the main loop". Backends call this once at startup.
pub fn install_scheduler(post: impl Fn() + 'static) {
    with_rt(|rt| rt.scheduler = Some(Rc::new(post)));
}

/// Register a callback run once after every fixpoint drain (day-core's layout turn).
pub fn on_turn_end(cb: impl Fn() + 'static) {
    with_rt(|rt| rt.turn_end.push(Rc::new(cb)));
}

// ---------------------------------------------------------------------------
// Cross-thread: the main poster + Setter
// ---------------------------------------------------------------------------

type MainPoster = Box<dyn Fn(Box<dyn FnOnce() + Send>) + Send + Sync>;
static MAIN_POSTER: std::sync::OnceLock<MainPoster> = std::sync::OnceLock::new();

/// Install the cross-thread → main-thread door. Backends call this once at startup.
pub fn install_main_poster(post: impl Fn(Box<dyn FnOnce() + Send>) + Send + Sync + 'static) {
    let _ = MAIN_POSTER.set(Box::new(post));
}

/// Schedule `f` on the UI thread (usable from any thread once a backend installed the poster).
pub fn on_main(f: impl FnOnce() + Send + 'static) {
    match MAIN_POSTER.get() {
        Some(post) => post(Box::new(f)),
        None => panic!("day-reactive: no main poster installed (backend not started)"),
    }
}

// ---------------------------------------------------------------------------
// Scope
// ---------------------------------------------------------------------------

/// Ownership scope for signals/effects (§4.3). `Copy` handle; not `Send`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Scope {
    key: ScopeKey,
    _not_send: PhantomData<*const ()>,
}

impl Scope {
    pub fn current() -> Scope {
        Scope {
            key: with_rt(|rt| rt.current_scope),
            _not_send: PhantomData,
        }
    }

    pub fn root() -> Scope {
        Scope {
            key: with_rt(|rt| rt.root_scope),
            _not_send: PhantomData,
        }
    }

    /// Create a child of the current scope.
    pub fn child() -> Scope {
        Scope::current().create_child()
    }

    pub fn create_child(self) -> Scope {
        let key = with_rt(|rt| {
            let depth = rt.scopes.get(self.key).map(|s| s.depth + 1).unwrap_or(1);
            let child = rt.scopes.insert(ScopeData {
                parent: self.key,
                children: Vec::new(),
                nodes: Vec::new(),
                cleanups: Vec::new(),
                context: HashMap::new(),
                depth,
            });
            if let Some(p) = rt.scopes.get_mut(self.key) {
                p.children.push(child);
            }
            child
        });
        Scope {
            key,
            _not_send: PhantomData,
        }
    }

    /// A scope owned by nobody — dispose it manually.
    pub fn detached() -> Scope {
        let key = with_rt(|rt| {
            rt.scopes.insert(ScopeData {
                parent: ScopeKey::null(),
                children: Vec::new(),
                nodes: Vec::new(),
                cleanups: Vec::new(),
                context: HashMap::new(),
                depth: 0,
            })
        });
        Scope {
            key,
            _not_send: PhantomData,
        }
    }

    /// Run `f` with `self` as the current scope.
    pub fn enter<R>(self, f: impl FnOnce() -> R) -> R {
        let prev = with_rt(|rt| std::mem::replace(&mut rt.current_scope, self.key));
        let r = f();
        with_rt(|rt| rt.current_scope = prev);
        r
    }

    pub fn on_cleanup(self, f: impl FnOnce() + 'static) {
        with_rt(|rt| {
            if let Some(s) = rt.scopes.get_mut(self.key) {
                s.cleanups.push(Box::new(f));
            }
        });
    }

    pub fn is_alive(self) -> bool {
        with_rt(|rt| rt.scopes.contains_key(self.key))
    }

    /// Dispose this scope: children first, then own nodes (unsubscribed + dropped) and cleanups.
    pub fn dispose(self) {
        let children = match with_rt(|rt| rt.scopes.get(self.key).map(|s| s.children.clone())) {
            Some(c) => c,
            None => return,
        };
        for c in children {
            (Scope {
                key: c,
                _not_send: PhantomData,
            })
            .dispose();
        }
        let (nodes, cleanups, parent) = match with_rt(|rt| {
            rt.scopes
                .remove(self.key)
                .map(|s| (s.nodes, s.cleanups, s.parent))
        }) {
            Some(v) => v,
            None => return,
        };
        with_rt(|rt| {
            if let Some(p) = rt.scopes.get_mut(parent)
                && let Some(pos) = p.children.iter().position(|&c| c == self.key)
            {
                p.children.swap_remove(pos);
            }
            for key in nodes {
                clear_sources(rt, key);
                // Detach us from downstream observers too.
                if let Some(node) = rt.nodes.get_mut(key) {
                    let observers = std::mem::take(&mut node.observers);
                    for o in observers {
                        if let Some(on) = rt.nodes.get_mut(o)
                            && let Some(pos) = on.sources.iter().position(|&s| s == key)
                        {
                            on.sources.swap_remove(pos);
                        }
                    }
                }
                rt.nodes.remove(key);
                // Pending entries for removed nodes are skipped at pop (generational key check).
            }
        });
        for c in cleanups {
            c();
        }
    }

    /// Provide a context value visible to this scope and its descendants.
    pub fn provide<T: 'static>(self, value: T) {
        with_rt(|rt| {
            if let Some(s) = rt.scopes.get_mut(self.key) {
                s.context.insert(TypeId::of::<T>(), Box::new(value));
            }
        });
    }

    /// Look up a context value here or in any ancestor (requires `T: Clone`).
    pub fn use_context<T: Clone + 'static>(self) -> Option<T> {
        with_rt(|rt| {
            let mut cur = self.key;
            while let Some(s) = rt.scopes.get(cur) {
                if let Some(v) = s.context.get(&TypeId::of::<T>()) {
                    return v.downcast_ref::<T>().cloned();
                }
                cur = s.parent;
            }
            None
        })
    }
}

// ---------------------------------------------------------------------------
// Signal
// ---------------------------------------------------------------------------

/// A `Copy`, `!Send` reactive value handle (§4.2).
pub struct Signal<T: 'static> {
    key: NodeKey,
    created_at: &'static Location<'static>,
    _m: PhantomData<*const T>,
}

impl<T: 'static> Clone for Signal<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: 'static> Copy for Signal<T> {}

impl<T: 'static> std::fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Signal({:?})", self.key)
    }
}

impl<T: 'static> Signal<T> {
    #[track_caller]
    pub fn new(value: T) -> Self {
        Self::new_in(Scope::current(), value)
    }

    #[track_caller]
    pub fn new_in(scope: Scope, value: T) -> Self {
        let created_at = Location::caller();
        let key = with_rt(|rt| {
            let k = create_node(rt, NodeKind::Signal, scope.key);
            rt.nodes[k].value = Some(Box::new(value));
            rt.nodes[k].created_at = created_at;
            k
        });
        Signal {
            key,
            created_at,
            _m: PhantomData,
        }
    }

    /// Tracked read by reference.
    #[track_caller]
    pub fn with<R>(self, f: impl FnOnce(&T) -> R) -> R {
        match self.try_with(f) {
            Some(r) => r,
            None => panic!(
                "day-reactive: read of disposed Signal created at {} (use try_with in closures that can outlive their scope)",
                self.created_at
            ),
        }
    }

    pub fn try_with<R>(self, f: impl FnOnce(&T) -> R) -> Option<R> {
        let value_ptr = with_rt(|rt| {
            track_read(rt, self.key);
            // Take the value out to run `f` without holding the RefCell borrow; put it back after.
            rt.nodes.get_mut(self.key).and_then(|n| n.value.take())
        });
        match value_ptr {
            Some(boxed) => {
                let r = boxed.downcast_ref::<T>().map(f);
                with_rt(|rt| {
                    if let Some(n) = rt.nodes.get_mut(self.key) {
                        n.value = Some(boxed);
                    }
                });
                r
            }
            None => None,
        }
    }

    pub fn with_untracked<R>(self, f: impl FnOnce(&T) -> R) -> R {
        untrack(|| self.with(f))
    }

    #[track_caller]
    pub fn get(self) -> T
    where
        T: Clone,
    {
        self.with(|v| v.clone())
    }

    pub fn try_get(self) -> Option<T>
    where
        T: Clone,
    {
        self.try_with(|v| v.clone())
    }

    pub fn get_untracked(self) -> T
    where
        T: Clone,
    {
        untrack(|| self.get())
    }

    /// Mark-only tracked read (subscribe without reading).
    pub fn track(self) {
        with_rt(|rt| track_read(rt, self.key));
    }

    #[track_caller]
    pub fn set(self, value: T) {
        self.write_check(move |slot| {
            *slot = value;
            true
        });
    }

    #[track_caller]
    pub fn set_if_changed(self, value: T)
    where
        T: PartialEq,
    {
        self.write_check(move |slot| {
            if *slot == value {
                false
            } else {
                *slot = value;
                true
            }
        });
    }

    #[track_caller]
    pub fn update(self, f: impl FnOnce(&mut T)) {
        self.write_check(move |slot| {
            f(slot);
            true
        });
    }

    #[track_caller]
    fn write_check(self, apply: impl FnOnce(&mut T) -> bool) {
        let alive = with_rt(|rt| rt.nodes.contains_key(self.key));
        if !alive {
            // Writes to disposed handles are defined no-ops (§4.3), warned once per callsite.
            let loc = Location::caller();
            let first = with_rt(|rt| rt.warned_writes.insert(loc as *const _));
            if first && cfg!(debug_assertions) {
                eprintln!(
                    "day-reactive: write at {loc} to a disposed signal (created at {}) ignored",
                    self.created_at
                );
            }
            return;
        }
        signal_write_boxed(self.key, |boxed| match boxed.downcast_mut::<T>() {
            Some(slot) => apply(slot),
            None => false,
        });
    }

    /// A `Send` write-only handle (§3.3).
    pub fn setter(self) -> Setter<T>
    where
        T: Send,
    {
        Setter {
            key: self.key,
            _m: PhantomData,
        }
    }
}

/// `Send` write-only handle to a signal; delivery hops to the UI thread via the main poster.
/// Writes after disposal are silent no-ops (§4.3).
pub struct Setter<T: Send + 'static> {
    key: NodeKey,
    _m: PhantomData<fn(T)>,
}

impl<T: Send + 'static> Clone for Setter<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: Send + 'static> Copy for Setter<T> {}

impl<T: Send + 'static> Setter<T> {
    pub fn set(self, value: T) {
        let key = self.key;
        on_main(move || {
            let poster = with_rt(|rt| {
                let node = rt.nodes.get_mut(key)?;
                let slot = node.value.as_mut().and_then(|b| b.downcast_mut::<T>())?;
                *slot = value;
                node.last_changed = rt.tick;
                rt.tick += 1;
                mark_observers(rt, key, NodeState::Dirty);
                schedule_after_write(rt)
            });
            if let Some(post) = poster {
                post();
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Memo
// ---------------------------------------------------------------------------

/// Cached, equality-diffed derived value (§4.2).
pub struct Memo<T: 'static> {
    key: NodeKey,
    created_at: &'static Location<'static>,
    _m: PhantomData<*const T>,
}

impl<T: 'static> Clone for Memo<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: 'static> Copy for Memo<T> {}

impl<T: 'static> Memo<T> {
    #[track_caller]
    pub fn new(f: impl Fn() -> T + 'static) -> Self
    where
        T: PartialEq,
    {
        Self::new_with_eq(f, |a, b| a == b)
    }

    #[track_caller]
    pub fn new_with_eq(f: impl Fn() -> T + 'static, eq: fn(&T, &T) -> bool) -> Self {
        let created_at = Location::caller();
        // The value is stored as EqBox<T> (value + comparator together), so diffing needs no
        // specialization and the arena-level eq is a plain fn pointer (eqbox_eq::<T>).
        let key = with_rt(|rt| {
            let scope = rt.current_scope;
            create_node(rt, NodeKind::Memo, scope)
        });
        let compute: Rc<dyn Fn() -> Box<dyn Any>> =
            Rc::new(move || Box::new(EqBox { value: f(), eq }) as Box<dyn Any>);
        with_rt(|rt| {
            let node = &mut rt.nodes[key];
            node.memo_compute = Some(compute);
            node.memo_eq = Some(eqbox_eq::<T>);
            node.state = NodeState::Dirty; // lazy: computed on first read
            node.created_at = created_at;
        });
        Memo {
            key,
            created_at,
            _m: PhantomData,
        }
    }

    #[track_caller]
    pub fn with<R>(self, f: impl FnOnce(&T) -> R) -> R {
        refresh_memo(self.key);
        let value = with_rt(|rt| {
            track_read(rt, self.key);
            rt.nodes.get_mut(self.key).and_then(|n| n.value.take())
        });
        match value {
            Some(boxed) => {
                let r = boxed
                    .downcast_ref::<EqBox<T>>()
                    .map(|e| f(&e.value))
                    .unwrap_or_else(|| panic!("day-reactive: memo type mismatch"));
                with_rt(|rt| {
                    if let Some(n) = rt.nodes.get_mut(self.key) {
                        n.value = Some(boxed);
                    }
                });
                r
            }
            None => panic!(
                "day-reactive: read of disposed Memo created at {}",
                self.created_at
            ),
        }
    }

    #[track_caller]
    pub fn get(self) -> T
    where
        T: Clone,
    {
        self.with(|v| v.clone())
    }
}

/// Value + comparator stored together so memo diffing needs no specialization.
struct EqBox<T> {
    value: T,
    eq: fn(&T, &T) -> bool,
}

fn eqbox_eq<T: 'static>(a: &dyn Any, b: &dyn Any) -> bool {
    match (a.downcast_ref::<EqBox<T>>(), b.downcast_ref::<EqBox<T>>()) {
        (Some(a), Some(b)) => (a.eq)(&a.value, &b.value),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Effect / bind / watch / Trigger
// ---------------------------------------------------------------------------

/// A reaction that re-runs when its tracked reads change. Runs once at creation.
pub struct Effect;

impl Effect {
    // Constructor-style registration: the reaction is owned by the current Scope (disposed
    // with it), so there is no handle value to return.
    #[allow(clippy::new_ret_no_self)]
    #[track_caller]
    pub fn new(f: impl Fn() + 'static) {
        create_reaction(Rc::new(f), 1);
    }
}

#[track_caller]
fn create_reaction(f: Rc<dyn Fn()>, priority: u8) -> NodeKey {
    let created_at = Location::caller();
    let key = with_rt(|rt| {
        let scope = rt.current_scope;
        let k = create_node(rt, NodeKind::Reaction, scope);
        rt.nodes[k].reaction = Some(f);
        rt.nodes[k].priority = priority;
        rt.nodes[k].created_at = created_at;
        k
    });
    // Initial run, tracked.
    with_rt(|rt| rt.observers.push(Some(key)));
    let reaction = with_rt(|rt| rt.nodes[key].reaction.clone());
    if let Some(r) = reaction {
        r();
    }
    with_rt(|rt| {
        rt.observers.pop();
        let tick = rt.tick;
        rt.tick += 1;
        if let Some(n) = rt.nodes.get_mut(key) {
            n.last_run = tick;
        }
    });
    key
}

/// The binding primitive (§4.2): compute (tracked) + apply (untracked), equality-gated.
/// Structural priority — bindings drain before plain effects. `apply` receives the new value
/// by reference so `V` needs only `PartialEq` (no `Clone`).
#[track_caller]
pub fn bind<V: PartialEq + 'static>(
    compute: impl Fn() -> V + 'static,
    apply: impl Fn(&V) + 'static,
) {
    let last: RefCell<Option<V>> = RefCell::new(None);
    create_reaction(
        Rc::new(move || {
            let v = compute();
            if last.borrow().as_ref() != Some(&v) {
                untrack(|| apply(&v));
                *last.borrow_mut() = Some(v);
            }
        }),
        0,
    );
}

/// `bind` pre-seeded with the value already applied at build time: the initial run does NOT
/// re-apply (pieces pass initial values through realize props; §5.2's no-duplicate-op rule).
#[track_caller]
pub fn bind_seeded<V: PartialEq + 'static>(
    seed: V,
    compute: impl Fn() -> V + 'static,
    apply: impl Fn(&V) + 'static,
) {
    let last: RefCell<Option<V>> = RefCell::new(Some(seed));
    create_reaction(
        Rc::new(move || {
            let v = compute();
            if last.borrow().as_ref() != Some(&v) {
                untrack(|| apply(&v));
                *last.borrow_mut() = Some(v);
            }
        }),
        0,
    );
}

/// `bind` for payloads without `PartialEq` — applies on every recompute.
#[track_caller]
pub fn bind_always<V: 'static>(compute: impl Fn() -> V + 'static, apply: impl Fn(V) + 'static) {
    create_reaction(
        Rc::new(move || {
            let v = compute();
            untrack(|| apply(v));
        }),
        0,
    );
}

/// Derive-state without effect-write loops (§4.2): `source` is tracked; `cb` runs untracked
/// with (new, old). Does NOT fire for the initial value.
#[track_caller]
pub fn watch<S: Clone + 'static>(
    source: impl Fn() -> S + 'static,
    cb: impl Fn(&S, Option<&S>) + 'static,
) {
    let prev: RefCell<Option<S>> = RefCell::new(None);
    create_reaction(
        Rc::new(move || {
            let new = source();
            let old = prev.borrow_mut().replace(new.clone());
            if old.is_some() {
                untrack(|| cb(&new, old.as_ref()));
            }
        }),
        1,
    );
}

/// Data-less invalidation source.
#[derive(Clone, Copy)]
pub struct Trigger {
    signal: Signal<u64>,
}

impl Trigger {
    #[track_caller]
    pub fn new() -> Self {
        Trigger {
            signal: Signal::new(0),
        }
    }
    pub fn track(self) {
        self.signal.track();
    }
    pub fn notify(self) {
        self.signal.update(|v| *v = v.wrapping_add(1));
    }
}

impl Default for Trigger {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell as StdCell;
    use std::rc::Rc;

    fn counter() -> (Rc<StdCell<u32>>, impl Fn()) {
        let c = Rc::new(StdCell::new(0));
        let c2 = c.clone();
        (c, move || c2.set(c2.get() + 1))
    }

    #[test]
    fn signal_get_set() {
        let s = Signal::new(1);
        assert_eq!(s.get(), 1);
        s.set(5);
        assert_eq!(s.get(), 5);
        s.update(|v| *v += 1);
        assert_eq!(s.get(), 6);
    }

    #[test]
    fn effect_reruns_on_change() {
        let s = Signal::new(1);
        let (count, bump) = counter();
        Effect::new(move || {
            s.track();
            bump();
        });
        assert_eq!(count.get(), 1); // initial run
        batch(|| s.set(2));
        assert_eq!(count.get(), 2);
        batch(|| s.set(3));
        assert_eq!(count.get(), 3);
    }

    #[test]
    fn batch_coalesces() {
        let s = Signal::new(0);
        let (count, bump) = counter();
        Effect::new(move || {
            s.track();
            bump();
        });
        batch(|| {
            s.set(1);
            s.set(2);
            s.set(3);
        });
        assert_eq!(count.get(), 2); // initial + one drain
        assert_eq!(s.get(), 3);
    }

    #[test]
    fn set_if_changed_no_op() {
        let s = Signal::new(7);
        let (count, bump) = counter();
        Effect::new(move || {
            s.track();
            bump();
        });
        batch(|| s.set_if_changed(7));
        assert_eq!(count.get(), 1); // no re-run
    }

    #[test]
    fn memo_caches_and_diffs() {
        let s = Signal::new(1);
        let computes = Rc::new(StdCell::new(0));
        let c2 = computes.clone();
        let doubled = Memo::new(move || {
            c2.set(c2.get() + 1);
            s.get() * 2
        });
        assert_eq!(doubled.get(), 2);
        assert_eq!(doubled.get(), 2);
        assert_eq!(computes.get(), 1); // cached
        let (effect_runs, bump) = counter();
        Effect::new(move || {
            let _ = doubled.get();
            bump();
        });
        assert_eq!(effect_runs.get(), 1);
        batch(|| s.set(1)); // same value → memo recomputes? signal changed, memo recomputes, same output
        // memo output unchanged (1*2 == 2)? s was already 1 → set(1) marks dirty (set is not diffed)
        assert_eq!(effect_runs.get(), 1); // memo diffing gates the effect
        batch(|| s.set(5));
        assert_eq!(doubled.get(), 10);
        assert_eq!(effect_runs.get(), 2);
    }

    #[test]
    fn bind_applies_on_change_only() {
        let s = Signal::new(1);
        let applied: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let a2 = applied.clone();
        bind(
            move || s.get().to_string(),
            move |v| a2.borrow_mut().push(v.clone()),
        );
        assert_eq!(*applied.borrow(), vec!["1"]);
        batch(|| s.set(2));
        assert_eq!(*applied.borrow(), vec!["1", "2"]);
        batch(|| s.update(|v| *v = 2)); // update always marks; bind's eq gate stops the apply
        assert_eq!(*applied.borrow(), vec!["1", "2"]);
    }

    #[test]
    fn watch_skips_initial_and_passes_old() {
        let s = Signal::new(10);
        type Log = Rc<RefCell<Vec<(i32, Option<i32>)>>>;
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let l2 = log.clone();
        watch(
            move || s.get(),
            move |new, old| l2.borrow_mut().push((*new, old.copied())),
        );
        assert!(log.borrow().is_empty());
        batch(|| s.set(11));
        assert_eq!(*log.borrow(), vec![(11, Some(10))]);
    }

    #[test]
    fn scope_dispose_stops_effects_and_write_is_noop() {
        let s = Signal::new(0);
        let (count, bump) = counter();
        let scope = Scope::child();
        let inner = scope.enter(|| {
            let inner = Signal::new(1);
            Effect::new(move || {
                s.track();
                bump();
            });
            inner
        });
        assert_eq!(count.get(), 1);
        batch(|| s.set(1));
        assert_eq!(count.get(), 2);
        scope.dispose();
        batch(|| s.set(2));
        assert_eq!(count.get(), 2); // effect gone
        inner.set(9); // silent no-op
        assert_eq!(inner.try_get(), None);
    }

    #[test]
    fn dispose_during_drain_skips_pending() {
        let s = Signal::new(0);
        let scope = Scope::child();
        let (count, bump) = counter();
        // Owner effect (created first in outer scope) disposes the child scope when s becomes 1.
        let scope_cell = Rc::new(StdCell::new(Some(scope)));
        let sc = scope_cell.clone();
        Effect::new(move || {
            if s.get() == 1
                && let Some(scope) = sc.take()
            {
                scope.dispose();
            }
        });
        scope.enter(|| {
            Effect::new(move || {
                s.track();
                bump();
            });
        });
        assert_eq!(count.get(), 1);
        batch(|| s.set(1));
        // Owner ran first (created earlier, same depth? owner depth 0 < child depth 1) and
        // disposed the child → child effect must NOT run again.
        assert_eq!(count.get(), 1);
    }

    #[test]
    fn fixpoint_extends_drain() {
        let a = Signal::new(0);
        let b = Signal::new(0);
        let (count, bump) = counter();
        Effect::new(move || {
            if a.get() == 1 {
                b.set(1); // write during drain extends the drain
            }
        });
        Effect::new(move || {
            b.track();
            bump();
        });
        assert_eq!(count.get(), 1);
        batch(|| a.set(1));
        assert_eq!(count.get(), 2);
        assert_eq!(b.get(), 1);
    }

    #[test]
    #[should_panic(expected = "re-ran more than")]
    fn rerun_cap_panics_in_debug() {
        let s = Signal::new(0);
        Effect::new(move || {
            let v = s.get();
            s.set(v + 1); // classic cycle
        });
        batch(|| s.set(1));
    }

    #[test]
    fn context_provides_down() {
        #[derive(Clone, PartialEq, Debug)]
        struct Theme(u32);
        let scope = Scope::child();
        scope.provide(Theme(7));
        let child = scope.create_child();
        assert_eq!(child.use_context::<Theme>(), Some(Theme(7)));
        scope.dispose();
    }

    #[test]
    fn trigger_notifies() {
        let t = Trigger::new();
        let (count, bump) = counter();
        Effect::new(move || {
            t.track();
            bump();
        });
        batch(|| t.notify());
        assert_eq!(count.get(), 2);
    }

    #[test]
    fn turn_end_runs_after_fixpoint() {
        let s = Signal::new(0);
        let order: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
        let o1 = order.clone();
        Effect::new(move || {
            s.track();
            o1.borrow_mut().push("effect");
        });
        let o2 = order.clone();
        on_turn_end(move || o2.borrow_mut().push("turn-end"));
        order.borrow_mut().clear();
        batch(|| s.set(1));
        assert_eq!(*order.borrow(), vec!["effect", "turn-end"]);
    }

    #[test]
    fn bindings_run_before_effects() {
        let s = Signal::new(0);
        let order: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
        let o1 = order.clone();
        // Plain effect created FIRST (lower seq) — priority must still put the bind first.
        Effect::new(move || {
            s.track();
            o1.borrow_mut().push("effect");
        });
        let o2 = order.clone();
        bind(move || s.get(), move |_| o2.borrow_mut().push("bind"));
        order.borrow_mut().clear();
        batch(|| s.set(1));
        assert_eq!(*order.borrow(), vec!["bind", "effect"]);
    }
}
