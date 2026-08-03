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
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::marker::PhantomData;
use std::panic::Location;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;

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

/// A node's stored value, shared so a reader can hold it while the runtime borrow is released.
///
/// `with`/`try_with` must run the caller's closure WITHOUT holding the thread-local runtime
/// borrow (the closure routinely reads other signals, which re-enters `with_rt`). This used to be
/// done by taking the value out of the node and putting it back afterwards, which had two costs: a
/// re-entrant read of the same signal found the hole and reported "disposed", and a panic inside
/// the closure lost the value permanently. Sharing the cell instead lets a reader clone it out
/// cheaply, so concurrent reads simply work and the value is never in limbo.
type NodeValue = Rc<RefCell<Box<dyn Any>>>;

struct Node {
    kind: NodeKind,
    state: NodeState,
    value: Option<NodeValue>,
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
    /// The async-spawn door ([`install_spawner`]): runs a boxed future on the app's main-loop
    /// executor, returning an abort closure.
    spawner: Option<Spawner>,
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
            spawner: None,
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
        let changed = match (node.value.as_ref(), node.memo_eq) {
            (Some(old), Some(eq)) => {
                // A reader may be holding this cell; compare against it without disturbing them.
                let old = old.borrow();
                !eq(old.as_ref(), new_value.as_ref())
            }
            _ => true,
        };
        node.last_run = tick;
        node.state = NodeState::Clean;
        if changed {
            // Install a NEW cell rather than writing through the old one: a read in flight keeps
            // reading the value it borrowed, for the duration of its closure, instead of panicking.
            node.value = Some(Rc::new(RefCell::new(new_value)));
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
        // The scope stack is unwound by `Scope::enter`'s guard, but a panic raised BETWEEN
        // scopes (or from a callsite that set `current_scope` directly) can still leave it on a
        // disposed scope, where every later `Signal::new` would be born dead. Re-root it.
        rt.current_scope = rt.root_scope;
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
        let cell = node.value.as_ref()?.clone();
        let mut value = match cell.try_borrow_mut() {
            Ok(v) => v,
            // A write to a signal from inside its own `with`/`try_with` closure. Before the value
            // was shared this silently did nothing (the read's restore clobbered the write), so
            // say it plainly instead of losing the update.
            Err(_) => panic!(
                "day-reactive: wrote to a Signal while a with()/try_with() read of it is still \
                 in flight — the write would be lost. Finish the read closure first (copy the \
                 value out with get()) and write after it returns."
            ),
        };
        let changed = apply(&mut value);
        drop(value);
        let node = rt.nodes.get_mut(key)?;
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
/// Restores a piece of runtime state when it goes out of scope — on the normal path AND on an
/// unwind. day-core deliberately CONTAINS panics at its trampoline boundaries (`pump_events`,
/// posted tasks, lifecycle) so a panicking app callback degrades the UI instead of aborting the
/// process; that guarantee is only worth anything if the reactive runtime is still coherent
/// afterwards. Restoring after `f()` returns is not enough, because that line is skipped on unwind.
struct RtGuard<F: FnMut(&mut Runtime)>(F);

impl<F: FnMut(&mut Runtime)> Drop for RtGuard<F> {
    fn drop(&mut self) {
        with_rt(|rt| (self.0)(rt));
    }
}

pub fn batch<R>(f: impl FnOnce() -> R) -> R {
    with_rt(|rt| rt.batch_depth += 1);
    let r = {
        // Runs on unwind too, so a contained panic can't strand `batch_depth` above zero — which
        // would stop every later write from scheduling a drain (a silently frozen UI).
        let _g = RtGuard(|rt: &mut Runtime| rt.batch_depth -= 1);
        f()
    };
    let should_drain = with_rt(|rt| rt.batch_depth == 0 && !rt.draining && !rt.pending.is_empty());
    if should_drain {
        flush_sync();
    }
    r
}

/// Force a synchronous fixpoint drain **now**, even inside an open [`batch`]. Event dispatch wraps
/// handlers in a batch (day-core), so a handler that needs its writes to drain immediately — namely
/// `with_animation`, whose ambient animation must be live *while* the resulting patches apply —
/// cannot rely on the batch's own close (that runs later, after the scope ends). This temporarily
/// drops the batch depth so [`flush_sync`] runs, then restores it. No-op if a drain is already in
/// progress (the writes fold into it); harmless if nothing is pending.
pub fn flush_now() {
    let saved = with_rt(|rt| std::mem::replace(&mut rt.batch_depth, 0));
    flush_sync();
    with_rt(|rt| rt.batch_depth = saved);
}

/// Run `f` without tracking reads.
pub fn untrack<R>(f: impl FnOnce() -> R) -> R {
    with_rt(|rt| rt.observers.push(None));
    // Popped on unwind too: a stranded `None` observer would silently disable dependency
    // tracking for every read that followed, so nothing would ever update again.
    let _g = RtGuard(|rt: &mut Runtime| {
        rt.observers.pop();
    });
    f()
}

/// Install "post a drain on the main loop". Backends call this once at startup.
pub fn install_scheduler(post: impl Fn() + 'static) {
    with_rt(|rt| rt.scheduler = Some(Rc::new(post)));
}

/// A boxed, `!Send` future for the [`install_spawner`] executor door.
pub type LocalBoxFuture = Pin<Box<dyn Future<Output = ()> + 'static>>;

/// The installed spawner: runs a future on the main-loop executor, returns its abort closure.
type Spawner = Rc<dyn Fn(LocalBoxFuture) -> Box<dyn FnOnce()>>;

/// Install the async-spawn door: `spawn` runs a future on the app's main-loop executor and
/// returns an ABORT closure (remove + drop the future). The abort MUST be a no-op once the
/// task has completed — [`Resource`] stores it after an eager first poll, so a synchronously
/// ready future has already finished by then. day-core's `launch_with` wires this to
/// `day::task` / `TaskHandle::abort`; call it once at startup (docs/async.md).
pub fn install_spawner(spawn: impl Fn(LocalBoxFuture) -> Box<dyn FnOnce()> + 'static) {
    with_rt(|rt| rt.spawner = Some(Rc::new(spawn)));
}

/// Spawn through the installed door ([`install_spawner`]); the [`on_main`] panic model.
fn spawn_local(fut: LocalBoxFuture) -> Box<dyn FnOnce()> {
    let spawner = with_rt(|rt| rt.spawner.clone());
    match spawner {
        Some(s) => s(fut),
        None => panic!("day-reactive: no spawner installed (backend not started)"),
    }
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

type DelayedPoster = Box<dyn Fn(u32, Box<dyn FnOnce() + Send>) + Send + Sync>;
static DELAYED_POSTER: std::sync::OnceLock<DelayedPoster> = std::sync::OnceLock::new();

/// Install the timer door (`Platform::post_delayed`). Backends call this once at startup.
pub fn install_delayed_poster(
    post: impl Fn(u32, Box<dyn FnOnce() + Send>) + Send + Sync + 'static,
) {
    let _ = DELAYED_POSTER.set(Box::new(post));
}

/// Schedule `f` on the UI thread after (at least) `ms` milliseconds — the rail behind
/// `day::sleep` (docs/async.md).
pub fn on_main_delayed(ms: u32, f: impl FnOnce() + Send + 'static) {
    match DELAYED_POSTER.get() {
        Some(post) => post(ms, Box::new(f)),
        None => panic!("day-reactive: no delayed poster installed (backend not started)"),
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
        // Restored on unwind too. Otherwise a contained panic leaves the runtime parented to
        // this (usually transient, soon-disposed) scope, so every later `Signal::new` is created
        // dead and its first read panics with a misleading "read of disposed Signal".
        let _g = RtGuard(move |rt: &mut Runtime| rt.current_scope = prev);
        f()
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

    /// A process-global signal: allocated in the ROOT scope regardless of where the call
    /// runs, so it never dies with a transient caller. **Every lazily-initialized global
    /// registry must use this, not [`Signal::new`]** — a lazy global first touched inside a
    /// presented cover / pushed page / `when` arm otherwise inherits that scope and is
    /// disposed with it, and every later read panics (the day-l10n locale signal was the
    /// observed case).
    #[track_caller]
    pub fn global(value: T) -> Self {
        Self::new_in(Scope::root(), value)
    }

    #[track_caller]
    pub fn new_in(scope: Scope, value: T) -> Self {
        let created_at = Location::caller();
        let key = with_rt(|rt| {
            let k = create_node(rt, NodeKind::Signal, scope.key);
            rt.nodes[k].value = Some(Rc::new(RefCell::new(Box::new(value) as Box<dyn Any>)));
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
        // Clone the shared cell out so `f` runs with the runtime borrow released (it routinely
        // reads other signals). The value stays in the node throughout, so a re-entrant read of
        // THIS signal inside `f` succeeds instead of reporting a disposed node, and a panic in
        // `f` cannot strand it.
        let cell = with_rt(|rt| {
            track_read(rt, self.key);
            rt.nodes.get(self.key).and_then(|n| n.value.clone())
        })?;
        let value = cell.borrow();
        value.downcast_ref::<T>().map(f)
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
                let cell = rt.nodes.get(key)?.value.clone()?;
                {
                    let mut boxed = cell.try_borrow_mut().ok()?;
                    let slot = boxed.downcast_mut::<T>()?;
                    *slot = value;
                }
                let node = rt.nodes.get_mut(key)?;
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
            rt.nodes.get(self.key).and_then(|n| n.value.clone())
        });
        match value {
            // Shared, like `Signal::try_with`: the memo stays readable while `f` runs.
            Some(cell) => {
                let boxed = cell.borrow();
                boxed
                    .downcast_ref::<EqBox<T>>()
                    .map(|e| f(&e.value))
                    .unwrap_or_else(|| panic!("day-reactive: memo type mismatch"))
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
// Async data loading: Load + Resource (§4.5, docs/async.md)
// ---------------------------------------------------------------------------

/// The lifecycle of an async-loaded value. `Failed` carries the fetcher's error as a shared
/// trait object so `Load` stays `Clone` for signal reads.
#[derive(Clone)]
pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(Arc<dyn std::error::Error + Send + Sync>),
}

impl<T> Load<T> {
    /// The loaded value, if ready.
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(v) => Some(v),
            _ => None,
        }
    }
    pub fn is_loading(&self) -> bool {
        matches!(self, Load::Loading)
    }
    pub fn is_ready(&self) -> bool {
        matches!(self, Load::Ready(_))
    }
    /// The failure, if the last fetch failed.
    pub fn error(&self) -> Option<&(dyn std::error::Error + Send + Sync + 'static)> {
        match self {
            Load::Failed(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for Load<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Load::Loading => write!(f, "Loading"),
            Load::Ready(v) => f.debug_tuple("Ready").field(v).finish(),
            Load::Failed(e) => f.debug_tuple("Failed").field(&e.to_string()).finish(),
        }
    }
}

/// Latest-wins bookkeeping shared by a Resource's reaction, its spawned futures, and disposal.
struct FetchState {
    generation: Cell<u64>,
    abort: RefCell<Option<Box<dyn FnOnce()>>>,
}

impl FetchState {
    /// Invalidate the in-flight fetch: bump the generation FIRST (so a completion that cannot
    /// be aborted still fails its check), then take-and-call the abort OUTSIDE the borrow
    /// (dropping a future can re-enter reactive state).
    fn supersede(&self) -> u64 {
        let next = self.generation.get() + 1;
        self.generation.set(next);
        let old = self.abort.borrow_mut().take();
        if let Some(abort) = old {
            abort();
        }
        next
    }
}

/// Declarative async data loading (§4.5): a TRACKED `source` whose value feeds an async
/// `fetcher`; the result lands in a `Signal<Load<T>>`. The source re-runs when its tracked
/// reads change; an unchanged source value refetches nothing, [`Resource::refetch`] always
/// does. Latest wins: a source change supersedes the in-flight fetch (aborting its task —
/// dropping any `FetchFuture` inside cancels the platform request) and a stale completion
/// writes nothing. Scope disposal aborts the in-flight fetch the same way.
///
/// ```ignore
/// let stations = Resource::new(
///     move || region.get(),                              // tracked: refetch on change
///     |region| async move { fetch_stations(region).await },
/// );
/// when(move || stations.ready(), move || station_list(stations));
/// ```
pub struct Resource<T: 'static> {
    state: Signal<Load<T>>,
    refetch_count: Signal<u64>,
    _not_send: PhantomData<*const T>,
}

impl<T> Clone for Resource<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Resource<T> {}

impl<T: 'static> Resource<T> {
    /// Start loading: `source` runs tracked (its signal reads become dependencies) and its
    /// value moves into `fetcher`'s future, which runs on the main-loop executor via the
    /// installed spawner — so no `Send` bound anywhere, and the future may read/write signals
    /// after its awaits. Infallible fetchers use `E = std::convert::Infallible`.
    #[track_caller]
    pub fn new<S, Fut, E>(
        source: impl Fn() -> S + 'static,
        fetcher: impl Fn(S) -> Fut + 'static,
    ) -> Resource<T>
    where
        S: Clone + PartialEq + 'static,
        Fut: Future<Output = Result<T, E>> + 'static,
        E: std::error::Error + Send + Sync + 'static,
    {
        let state: Signal<Load<T>> = Signal::new(Load::Loading);
        let refetch_count: Signal<u64> = Signal::new(0);
        let fs = Rc::new(FetchState {
            generation: Cell::new(0),
            abort: RefCell::new(None),
        });
        let reaction_fs = fs.clone();
        let prev: RefCell<Option<(S, u64)>> = RefCell::new(None);
        create_reaction(
            Rc::new(move || {
                // Tracked reads FIRST: the refetch counter and the source. Everything else runs
                // untracked — the fetcher may read signals without making them dependencies.
                let count = refetch_count.get();
                let s = source();
                let fs = reaction_fs.clone();
                untrack(|| {
                    // Equality gate on (source value, refetch count): a rerun caused by an
                    // unrelated tracked read refetches nothing; refetch() always changes the
                    // pair, so it always fetches.
                    let same = prev
                        .borrow()
                        .as_ref()
                        .is_some_and(|(ps, pc)| ps == &s && *pc == count);
                    if same {
                        return;
                    }
                    *prev.borrow_mut() = Some((s.clone(), count));
                    let my_generation = fs.supersede();
                    state.set(Load::Loading);
                    let fut = fetcher(s);
                    let done = fs.clone();
                    let abort = spawn_local(Box::pin(async move {
                        let result = fut.await;
                        if done.generation.get() != my_generation {
                            return; // superseded while in flight — latest wins
                        }
                        done.abort.borrow_mut().take(); // completed: the stored abort is dead
                        match result {
                            Ok(v) => state.set(Load::Ready(v)),
                            Err(e) => state.set(Load::Failed(Arc::new(e))),
                        }
                    }));
                    // Eager-poll ordering: the spawner polls once before returning, so a
                    // synchronously-ready fetcher has ALREADY completed here — storing its
                    // abort is harmless only because the spawner contract makes aborting a
                    // finished task a no-op (this line is why that contract exists).
                    *fs.abort.borrow_mut() = Some(abort);
                });
            }),
            1,
        );
        // Disposal: supersede + abort the in-flight task. (A completion that slipped through
        // would write to disposed signals — the defined no-op — but aborting frees the platform
        // request immediately.)
        let cleanup_fs = fs;
        Scope::current().on_cleanup(move || {
            cleanup_fs.supersede();
        });
        Resource {
            state,
            refetch_count,
            _not_send: PhantomData,
        }
    }

    /// The underlying state signal (tracked reads, `when(move || …)` guards, `each` sources).
    pub fn signal(self) -> Signal<Load<T>> {
        self.state
    }

    /// The current load state (tracked).
    pub fn get(self) -> Load<T>
    where
        T: Clone,
    {
        self.state.get()
    }

    /// Read the current load state by reference (tracked).
    pub fn with<R>(self, f: impl FnOnce(&Load<T>) -> R) -> R {
        self.state.with(f)
    }

    /// Whether a fetch is in flight (tracked).
    pub fn loading(self) -> bool {
        self.with(Load::is_loading)
    }

    /// Whether a value is ready (tracked) — `when(move || r.ready(), …)`.
    pub fn ready(self) -> bool {
        self.with(Load::is_ready)
    }

    /// Force a refetch with the current source value (even if unchanged).
    pub fn refetch(self) {
        self.refetch_count.update(|c| *c = c.wrapping_add(1));
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

#[cfg(test)]
mod resource_tests {
    use super::*;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    // ---- test executor: mirrors day::task's shape (eager first poll; abort = remove+drop;
    // pump() re-polls whatever is still pending). ABORTED records aborts of still-pending
    // tasks — the probe the latest-wins/disposal tests read. All thread-local: each #[test]
    // thread gets a fresh Runtime AND a fresh executor.

    thread_local! {
        static TEST_TASKS: RefCell<Vec<(u64, Option<LocalBoxFuture>)>> =
            const { RefCell::new(Vec::new()) };
        static NEXT_ID: Cell<u64> = const { Cell::new(1) };
        static ABORTED: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
        static INSTALLED: Cell<bool> = const { Cell::new(false) };
    }

    fn noop_waker() -> Waker {
        fn raw() -> RawWaker {
            RawWaker::new(std::ptr::null(), &VT)
        }
        static VT: RawWakerVTable = RawWakerVTable::new(|_| raw(), |_| {}, |_| {}, |_| {});
        // SAFETY: every vtable entry ignores the data pointer.
        unsafe { Waker::from_raw(raw()) }
    }

    fn poll_one(fut: &mut LocalBoxFuture) -> bool {
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        fut.as_mut().poll(&mut cx).is_ready()
    }

    fn install_test_spawner() {
        if INSTALLED.with(|c| c.replace(true)) {
            return;
        }
        install_spawner(|mut fut| {
            let id = NEXT_ID.with(|c| {
                let v = c.get();
                c.set(v + 1);
                v
            });
            if !poll_one(&mut fut) {
                TEST_TASKS.with(|t| t.borrow_mut().push((id, Some(fut))));
            }
            Box::new(move || {
                let removed = TEST_TASKS.with(|t| {
                    let mut t = t.borrow_mut();
                    let before = t.len();
                    t.retain(|(i, _)| *i != id);
                    before != t.len()
                });
                if removed {
                    ABORTED.with(|a| a.borrow_mut().push(id));
                }
            })
        });
    }

    fn aborted_count() -> usize {
        ABORTED.with(|a| a.borrow().len())
    }

    /// Re-poll every pending task once (completions drop out).
    fn pump() {
        let ids: Vec<u64> = TEST_TASKS.with(|t| t.borrow().iter().map(|(i, _)| *i).collect());
        for id in ids {
            let fut = TEST_TASKS.with(|t| {
                t.borrow_mut()
                    .iter_mut()
                    .find(|(i, _)| *i == id)
                    .and_then(|(_, s)| s.take())
            });
            let Some(mut fut) = fut else { continue };
            if poll_one(&mut fut) {
                TEST_TASKS.with(|t| t.borrow_mut().retain(|(i, _)| *i != id));
            } else {
                TEST_TASKS.with(|t| {
                    if let Some((_, s)) = t.borrow_mut().iter_mut().find(|(i, _)| *i == id) {
                        *s = Some(fut);
                    }
                });
            }
        }
        // Completions wrote signals; with no scheduler installed a test must drain explicitly
        // (the production main loop drains via the installed scheduler).
        flush_sync();
    }

    type Slot<T> = Rc<RefCell<Option<Result<T, TestErr>>>>;

    /// A future the test resolves by hand (fill the slot, then `pump()`); no waker wiring —
    /// pump re-polls everything.
    fn manual_future<T: 'static>(slot: Slot<T>) -> impl Future<Output = Result<T, TestErr>> {
        std::future::poll_fn(move |_| match slot.borrow_mut().take() {
            Some(r) => Poll::Ready(r),
            None => Poll::Pending,
        })
    }

    #[derive(Debug)]
    struct TestErr(&'static str);
    impl std::fmt::Display for TestErr {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }
    impl std::error::Error for TestErr {}

    #[test]
    fn resource_sync_fetcher_is_ready_immediately() {
        install_test_spawner();
        let r = Resource::new(|| (), |_| async { Ok::<_, std::convert::Infallible>(42) });
        assert!(r.ready());
        assert_eq!(r.with(|l| l.ready().copied()), Some(42));
    }

    #[test]
    fn resource_loading_then_ready() {
        install_test_spawner();
        let slot: Slot<i32> = Rc::new(RefCell::new(None));
        let s2 = slot.clone();
        let r = Resource::new(move || (), move |_| manual_future(s2.clone()));
        assert!(r.loading());
        *slot.borrow_mut() = Some(Ok(7));
        pump();
        assert_eq!(r.with(|l| l.ready().copied()), Some(7));
    }

    #[test]
    fn resource_failed_preserves_error() {
        install_test_spawner();
        let r = Resource::new(|| (), |_| async { Err::<i32, _>(TestErr("boom")) });
        assert!(!r.ready());
        assert_eq!(
            r.with(|l| l.error().map(|e| e.to_string())),
            Some("boom".into())
        );
    }

    #[test]
    fn resource_refetches_on_source_change() {
        install_test_spawner();
        let src = Signal::new(1i32);
        let fetches = Rc::new(Cell::new(0));
        let f = fetches.clone();
        let r = Resource::new(
            move || src.get(),
            move |v| {
                f.set(f.get() + 1);
                async move { Ok::<_, TestErr>(v * 10) }
            },
        );
        assert_eq!(fetches.get(), 1);
        assert_eq!(r.with(|l| l.ready().copied()), Some(10));
        batch(|| src.set(2));
        assert_eq!(fetches.get(), 2);
        assert_eq!(r.with(|l| l.ready().copied()), Some(20));
    }

    #[test]
    fn resource_source_equality_gates() {
        install_test_spawner();
        let a = Signal::new(1i32);
        let b = Signal::new(10i32);
        let fetches = Rc::new(Cell::new(0));
        let f = fetches.clone();
        let r = Resource::new(
            move || {
                let _ = b.get(); // tracked but not part of the source VALUE
                a.get()
            },
            move |v| {
                f.set(f.get() + 1);
                async move { Ok::<_, TestErr>(v) }
            },
        );
        assert_eq!(fetches.get(), 1);
        batch(|| b.set(20)); // reruns the reaction; the source value is unchanged → no refetch
        assert_eq!(fetches.get(), 1);
        assert_eq!(r.with(|l| l.ready().copied()), Some(1));
        batch(|| a.set(2)); // the value changed → refetch
        assert_eq!(fetches.get(), 2);
    }

    #[test]
    fn resource_refetch_forces() {
        install_test_spawner();
        let fetches = Rc::new(Cell::new(0));
        let f = fetches.clone();
        let r = Resource::new(
            || (),
            move |_| {
                f.set(f.get() + 1);
                async { Ok::<_, TestErr>(0) }
            },
        );
        assert_eq!(fetches.get(), 1);
        batch(|| r.refetch()); // same source value — the refetch counter forces it past the gate
        assert_eq!(fetches.get(), 2);
    }

    #[test]
    fn resource_latest_wins() {
        install_test_spawner();
        let src = Signal::new(1i32);
        let slots: Rc<RefCell<Vec<Slot<i32>>>> = Rc::new(RefCell::new(Vec::new()));
        let sl = slots.clone();
        let r = Resource::new(
            move || src.get(),
            move |_| {
                let slot: Slot<i32> = Rc::new(RefCell::new(None));
                sl.borrow_mut().push(slot.clone());
                manual_future(slot)
            },
        );
        assert!(r.loading());
        let before = aborted_count();
        batch(|| src.set(2)); // supersede generation 1 while it is in flight
        assert_eq!(aborted_count(), before + 1, "the stale task was aborted");
        // Resolving generation 1 anyway must change nothing (its future is gone).
        *slots.borrow()[0].borrow_mut() = Some(Ok(111));
        pump();
        assert!(r.loading(), "a stale result writes nothing");
        *slots.borrow()[1].borrow_mut() = Some(Ok(222));
        pump();
        assert_eq!(r.with(|l| l.ready().copied()), Some(222));
    }

    #[test]
    fn resource_scope_disposal_aborts() {
        install_test_spawner();
        let scope = Scope::child();
        let slot: Slot<i32> = Rc::new(RefCell::new(None));
        let s2 = slot.clone();
        let r = scope.enter(|| Resource::new(move || (), move |_| manual_future(s2.clone())));
        assert!(r.loading());
        let before = aborted_count();
        scope.dispose();
        assert_eq!(aborted_count(), before + 1, "disposal aborted the fetch");
        // Resolving afterwards is inert: the future is gone; nothing pends; no panic.
        *slot.borrow_mut() = Some(Ok(1));
        pump();
    }

    #[test]
    fn resource_ready_is_tracked() {
        install_test_spawner();
        let slot: Slot<i32> = Rc::new(RefCell::new(None));
        let s2 = slot.clone();
        let r = Resource::new(move || (), move |_| manual_future(s2.clone()));
        let runs = Rc::new(Cell::new(0));
        let rn = runs.clone();
        Effect::new(move || {
            let _ = r.ready();
            rn.set(rn.get() + 1);
        });
        assert_eq!(runs.get(), 1);
        *slot.borrow_mut() = Some(Ok(1));
        pump();
        assert!(runs.get() >= 2, "ready() re-ran the effect on completion");
    }

    #[test]
    #[should_panic(expected = "no spawner installed")]
    fn spawner_missing_panics() {
        // Deliberately NOT installing the test spawner: this test thread's Runtime has none.
        let _r = Resource::new(|| (), |_| async { Ok::<_, std::convert::Infallible>(1) });
    }
}

#[cfg(test)]
mod unwind_safety_tests {
    use super::*;

    /// Run `f`, swallowing a panic, with the panic hook silenced so the test output stays clean.
    fn contained(f: impl FnOnce() + std::panic::UnwindSafe) {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let r = std::panic::catch_unwind(f);
        std::panic::set_hook(prev);
        assert!(r.is_err(), "expected the closure to panic");
    }

    /// A panic inside `Scope::enter` must not leave the runtime parented to the transient scope.
    /// Before the guard, every later `Signal::new` landed in a disposed scope and its first read
    /// panicked with a misleading "read of disposed Signal".
    #[test]
    fn panic_in_scope_enter_restores_current_scope() {
        let before = with_rt(|rt| rt.current_scope);
        let scope = Scope::detached();
        contained(|| scope.enter(|| panic!("boom in enter")));
        assert_eq!(with_rt(|rt| rt.current_scope), before, "scope not restored");
        // The real symptom: a signal created afterwards must still be alive and readable.
        let s = Signal::new(7u32);
        assert_eq!(s.get(), 7);
    }

    /// A panic inside `untrack` must pop its `None` observer, or dependency tracking stays off
    /// for the rest of the process and nothing ever updates again.
    #[test]
    fn panic_in_untrack_restores_tracking() {
        let depth = with_rt(|rt| rt.observers.len());
        contained(|| untrack(|| panic!("boom in untrack")));
        assert_eq!(with_rt(|rt| rt.observers.len()), depth, "observer stranded");

        // Tracking still works: an effect re-runs when its source changes.
        let src = Signal::new(0i32);
        let runs = Rc::new(RefCell::new(0));
        let r2 = runs.clone();
        Effect::new(move || {
            src.get();
            *r2.borrow_mut() += 1;
        });
        flush_sync();
        let before = *runs.borrow();
        src.set(1);
        flush_sync();
        assert!(
            *runs.borrow() > before,
            "effect did not re-run — tracking is off"
        );
    }

    /// A panic inside `batch` must not strand `batch_depth`, which would stop every later write
    /// from scheduling a drain.
    #[test]
    fn panic_in_batch_restores_depth() {
        let depth = with_rt(|rt| rt.batch_depth);
        contained(|| batch(|| panic!("boom in batch")));
        assert_eq!(with_rt(|rt| rt.batch_depth), depth, "batch depth stranded");
    }

    /// A panic inside a `with` closure must not consume the signal's value. Before the value was
    /// shared it was taken out for the closure's duration, so an unwind lost it permanently and
    /// the signal was dead for the rest of the process.
    #[test]
    fn panic_in_with_leaves_the_value_intact() {
        let s = Signal::new(String::from("intact"));
        contained(|| s.with(|_| panic!("boom in with")));
        assert_eq!(s.with(|v| v.clone()), "intact");
    }

    /// Reading a signal from inside its own `with` closure now works. It used to find the hole
    /// left by the take and panic with "read of disposed Signal".
    #[test]
    fn reentrant_read_inside_with_succeeds() {
        let s = Signal::new(5u32);
        let doubled = s.with(|outer| *outer + s.with(|inner| *inner));
        assert_eq!(doubled, 10);
        assert_eq!(
            s.with(|v| *v),
            5,
            "value still in place after the nested read"
        );
    }

    /// Two different signals read inside one another's closures — the ordinary case, and the
    /// reason the closure must run with the runtime borrow released.
    #[test]
    fn nested_reads_of_distinct_signals_succeed() {
        let a = Signal::new(2u32);
        let b = Signal::new(3u32);
        assert_eq!(a.with(|x| *x * b.with(|y| *y)), 6);
    }

    /// Writing a signal while a read of it is in flight is reported instead of silently lost.
    #[test]
    fn write_during_read_panics_with_a_clear_message() {
        let s = Signal::new(1u32);
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            s.with(|_| s.set(2));
        }));
        std::panic::set_hook(prev);
        let payload = r.unwrap_err();
        // A `panic!` with no format arguments carries a `&'static str`, not a `String`.
        let msg = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .expect("panic payload should be a string");
        assert!(
            msg.contains("in flight") && msg.contains("Signal"),
            "message should name the cause, got: {msg}"
        );
    }
}
