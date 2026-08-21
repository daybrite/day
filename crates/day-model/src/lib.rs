// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Per-property observation: a store whose writes wake only the readers of the field that
//! changed.
//!
//! With one `Signal<Vec<Item>>`, every observer of the store re-runs when any field of any
//! element changes. Day's `bind` equality gate keeps the *native* side precise, so what is wasted
//! is compute — every row's closures re-run and re-clone on every keystroke, and the waste grows
//! with the list. Here each (element, field) is its own dependency node, so a write wakes exactly
//! the readers of that field, plus readers who asked for something coarser by reading something
//! coarser.
//!
//! The pieces:
//!
//! - [`Store<T>`] — a `Copy`, process-lifetime handle to one value, like `Signal::global`.
//! - [`Keyed<T>`] — a collection addressed by stable key, with its own key→index map.
//! - [`Elem<T>`] — one element of a keyed store; [`Elem::exists`] is the tracked deletion guard.
//! - [`Field`] — one field projected out of any [`Source`]; `Copy`, itself a `Source` (so fields
//!   nest to any depth), and a [`Binding`], so every control binds to it directly.
//! - `#[derive(Observable)]` (in day-macros) — generates the typed field accessors, so
//!   `store.elem(id).name()` is written once per struct, not once per call site.
//! - The **change log** — every write announces `(path components, field label, operation)`, with
//!   the slot's prior and new value captured when a consumer asks ([`record_values`]). The test
//!   seam today; the persistence layer's input later.
//!
//! ## Paths
//!
//! A path names one observable slot: a store, one element of it, one field, a field of that
//! field, to any depth. Each gets its own `Trigger` (day-reactive's data-less dependency node),
//! created lazily on first observation — a path nobody reads has no trigger and costs nothing to
//! write. Reads track the most specific path they touched; writes notify that path and its
//! ancestors. That asymmetry is the entire granularity story.
//!
//! A path is a parent (an interned id) plus one component. The leaf component is not interned, so
//! building a field handle — the thing that happens on every read of every row — costs no lookup.
//! Interning happens only where a path is used as a *parent*: once per store, once per element
//! handle, once per nested struct.
//!
//! ## Reclamation
//!
//! Both tables shrink. Triggers are refcounted by the scopes observing them: a binding's scope
//! dies — a page popped, a row recycled — and the last watcher out disposes the trigger. Interner
//! entries are refcounted by their children and their triggers: when the last trigger under an
//! element is released, the element's entry (and any empty ancestors up to its pinned store root)
//! is freed. A freed slot's id carries a generation, and every handle validates its cached id
//! before use, re-interning through its own handle chain when stale — so a `Copy` handle held
//! across a reclamation heals itself, and everyone converges on the current identity.
//!
//! A claim made from inside a reactive computation belongs to that computation's CURRENT RUN,
//! not to a scope: it is released when the computation re-tracks or dies
//! ([`day_reactive::on_run_retrack`]), exactly mirroring day-reactive's own per-run source
//! bookkeeping. This is what keeps a long-lived recycled list cell from accumulating claims for
//! every row it ever showed — rotate a binding across a million rows and the claim count stays
//! at one row's worth. Outside any computation — a build seeding an initial value, an event
//! handler — a tracked read subscribes nothing and therefore CLAIMS nothing: no trigger is
//! created at all, because nothing could ever wake through it.
//!
//! ## Threads
//!
//! `Store` is `Send + Sync` (when `T` is); the trigger tables are thread-local to the main
//! thread. A worker edits through [`Store::transact`], naming what it touched as plain path
//! components; dropping the transaction commits the data and queues the announcements, and
//! [`Store::pump`], called on the main thread, wakes exactly the paths the worker named. (The
//! persistence container will schedule the pump itself; a bare store keeps delivery explicit.)

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

use day_reactive::{Binding, Scope, Trigger};

// ---------------------------------------------------------------------------
// Paths and the interner
// ---------------------------------------------------------------------------

/// An interned interior node: a path that something hangs off. Carries a generation so a handle
/// can tell a live id from one whose slot was reclaimed and reused.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeId {
    idx: u32,
    generation: u32,
}

/// The parent of a store's own path.
const ROOT: NodeId = NodeId {
    idx: 0,
    generation: 0,
};

/// The reserved component naming a collection's SHAPE (which keys, in what order) as opposed to
/// any element's contents.
pub const STRUCTURE: u64 = u64::MAX;

/// One observable slot: a parent, and one step down from it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Path {
    parent: NodeId,
    part: u64,
}

impl Path {
    /// A store's own path.
    fn root(id: u64) -> Path {
        Path {
            parent: ROOT,
            part: id,
        }
    }

    /// One step down. Cheap: the parent is already interned by whoever handed it over.
    pub fn under(parent: NodeId, part: u64) -> Path {
        Path { parent, part }
    }

    /// The path's components, outermost first, resolved through the interner.
    pub fn components(self) -> Vec<u64> {
        let mut out: Vec<u64> = self.chain().iter().map(|p| p.part).collect();
        out.reverse();
        out
    }

    /// This path and every resolvable ancestor, innermost first — exactly what a write wakes.
    fn chain(self) -> Vec<Path> {
        let mut out = vec![self];
        let mut at = self.parent;
        NODES.with(|n| {
            let nodes = n.borrow();
            while at != ROOT {
                match nodes.get(at) {
                    Some(p) => {
                        out.push(p);
                        at = p.parent;
                    }
                    None => break,
                }
            }
        });
        out
    }
}

/// A field's path component, derived from its NAME at compile time.
///
/// Hand-assigned indices are a hazard: two fields given the same number share a trigger, and the
/// symptom is a wakeup that fires too often — invisible until someone profiles it. A name is
/// already unique within a struct, so the compiler's own uniqueness rule becomes the path's.
pub const fn field_id(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
        i += 1;
    }
    h
}

struct Slot {
    path: Path,
    generation: u32,
    alive: bool,
    /// A store root: never reclaimed, because the leaked `Store` handle is forever.
    pinned: bool,
    /// Live child slots whose `path.parent` is this id.
    children: u32,
    /// Live triggers on paths whose parent is this id.
    triggers: u32,
}

#[derive(Default)]
struct Interner {
    /// Index 0 is the root sentinel and is never read as a path.
    slots: Vec<Slot>,
    free: Vec<u32>,
    lookup: HashMap<Path, NodeId>,
}

impl Interner {
    fn get(&self, id: NodeId) -> Option<Path> {
        match self.slots.get(id.idx as usize) {
            Some(s) if s.alive && s.generation == id.generation => Some(s.path),
            _ => None,
        }
    }

    fn is_current(&self, id: NodeId) -> bool {
        id == ROOT || self.get(id).is_some()
    }

    /// Free `id` if nothing holds it, then walk up freeing empty ancestors. Stops at pinned
    /// store roots and at anything still watched or still a parent.
    fn maybe_free(&mut self, mut id: NodeId) {
        loop {
            let Some(s) = self.slots.get(id.idx as usize) else {
                return;
            };
            if !s.alive
                || s.generation != id.generation
                || s.pinned
                || s.children > 0
                || s.triggers > 0
            {
                return;
            }
            let path = s.path;
            self.lookup.remove(&path);
            let s = &mut self.slots[id.idx as usize];
            s.alive = false;
            s.generation = s.generation.wrapping_add(1);
            self.free.push(id.idx);
            if path.parent == ROOT {
                return;
            }
            if let Some(ps) = self.slots.get_mut(path.parent.idx as usize)
                && ps.alive
                && ps.generation == path.parent.generation
                && ps.children > 0
            {
                ps.children -= 1;
                id = path.parent;
                continue;
            }
            return;
        }
    }
}

thread_local! {
    static NODES: RefCell<Interner> = RefCell::new(Interner {
        slots: vec![Slot {
            path: Path { parent: ROOT, part: 0 },
            generation: 0,
            alive: true,
            pinned: true,
            children: 0,
            triggers: 0,
        }],
        free: Vec::new(),
        lookup: HashMap::new(),
    });

    /// Path → trigger. Each trigger lives in its own child of the ROOT scope: created inside
    /// whichever page first observed it, it would be disposed with that page and every later
    /// read would panic. Its own scope is also what makes reclamation possible.
    static TRIGGERS: RefCell<HashMap<Path, Entry>> = RefCell::new(HashMap::new());

    /// Every change announced while recording is on — the test seam and, with a schema beside
    /// it, the persistence change set.
    static RECORDER: RefCell<Option<Vec<Change>>> = const { RefCell::new(None) };

    /// Whether writes should capture prior/new values into the change log.
    static WANT_VALUES: Cell<bool> = const { Cell::new(false) };
}

/// A live trigger and who is watching it.
struct Entry {
    trigger: Trigger,
    /// The trigger's own scope, a child of root, so it can be disposed on its own.
    scope: Scope,
    /// One claim per observing computation, deduped; when the last one releases (on its
    /// re-track or death), the trigger goes with it.
    watchers: Vec<day_reactive::RunId>,
}

/// Give this path an id, so something can hang off it. Called once per source handle, never on
/// the per-field read path.
fn intern(p: Path) -> NodeId {
    NODES.with(|n| {
        let mut i = n.borrow_mut();
        if let Some(id) = i.lookup.get(&p) {
            return *id;
        }
        let id = match i.free.pop() {
            Some(idx) => {
                let s = &mut i.slots[idx as usize];
                s.path = p;
                s.alive = true;
                s.pinned = false;
                s.children = 0;
                s.triggers = 0;
                NodeId {
                    idx,
                    generation: s.generation,
                }
            }
            None => {
                let idx = i.slots.len() as u32;
                i.slots.push(Slot {
                    path: p,
                    generation: 0,
                    alive: true,
                    pinned: false,
                    children: 0,
                    triggers: 0,
                });
                NodeId { idx, generation: 0 }
            }
        };
        i.lookup.insert(p, id);
        if p.parent != ROOT
            && let Some(s) = i.slots.get_mut(p.parent.idx as usize)
            && s.alive
            && s.generation == p.parent.generation
        {
            s.children += 1;
        }
        id
    })
}

/// Whether an id still names a live slot. A handle whose cached id fails this re-interns
/// through its own chain — see the module docs on reclamation.
fn is_current(id: NodeId) -> bool {
    NODES.with(|n| n.borrow().is_current(id))
}

fn path_of(id: NodeId) -> Path {
    NODES.with(|n| n.borrow().get(id).unwrap_or(Path::root(0)))
}

fn pin(id: NodeId) {
    NODES.with(|n| {
        if let Some(s) = n.borrow_mut().slots.get_mut(id.idx as usize) {
            s.pinned = true;
        }
    });
}

/// How many interior nodes are currently interned (the sentinel excluded) — the cost of
/// observation's second table, assertable in a test.
pub fn interned_nodes() -> usize {
    NODES.with(|n| n.borrow().slots.iter().filter(|s| s.alive).count() - 1)
}

/// Subscribe the current reactive computation to exactly this path, and register the claim so
/// the trigger can be reclaimed on the computation's next re-track (or death).
///
/// OBSERVATION BELONGS TO COMPUTATIONS: outside any active run — a build seeding an initial
/// value, an event handler, `untrack` — a tracked read subscribes nothing in day-reactive, so
/// creating a trigger for it would be dead weight that some scope had to carry. It does
/// nothing, on purpose; the read itself proceeds unobserved.
fn track(p: Path) {
    let Some(watcher) = day_reactive::active_run() else {
        return;
    };
    let (trigger, created, fresh) = TRIGGERS.with(|t| {
        let mut map = t.borrow_mut();
        let mut created = false;
        let entry = map.entry(p).or_insert_with(|| {
            created = true;
            // Its own child of root: outlives every observer, and disposable on its own.
            let scope = Scope::root().create_child();
            Entry {
                trigger: scope.enter(Trigger::new),
                scope,
                watchers: Vec::new(),
            }
        });
        let fresh = !entry.watchers.contains(&watcher);
        if fresh {
            entry.watchers.push(watcher);
        }
        (entry.trigger, created, fresh)
    });
    if created && p.parent != ROOT {
        NODES.with(|n| {
            let mut i = n.borrow_mut();
            if let Some(s) = i.slots.get_mut(p.parent.idx as usize)
                && s.alive
                && s.generation == p.parent.generation
            {
                s.triggers += 1;
            }
        });
    }
    if fresh {
        // The claim dies with the run: released when the computation re-tracks (having stopped
        // reading this path, or about to read it afresh) or is disposed.
        day_reactive::on_run_retrack(move || release(p, watcher));
    }
    trigger.track();
}

/// Drop one watcher's claim; the last one out disposes the trigger and lets the interner free
/// the ancestors nothing else holds.
fn release(p: Path, watcher: day_reactive::RunId) {
    let removed = TRIGGERS.with(|t| {
        let mut map = t.borrow_mut();
        let Some(entry) = map.get_mut(&p) else {
            return false;
        };
        entry.watchers.retain(|w| *w != watcher);
        if entry.watchers.is_empty() {
            let scope = entry.scope;
            map.remove(&p);
            drop(map);
            // A trigger holds nothing but a counter, so disposing it loses no state: the next
            // reader creates a fresh one.
            scope.dispose();
            true
        } else {
            false
        }
    });
    if removed && p.parent != ROOT {
        NODES.with(|n| {
            let mut i = n.borrow_mut();
            if let Some(s) = i.slots.get_mut(p.parent.idx as usize)
                && s.alive
                && s.generation == p.parent.generation
                && s.triggers > 0
            {
                s.triggers -= 1;
            }
            i.maybe_free(p.parent);
        });
    }
}

/// How many paths currently have a trigger — the cost of observation, assertable in a test.
pub fn observed_paths() -> usize {
    TRIGGERS.with(|t| t.borrow().len())
}

// ---------------------------------------------------------------------------
// The change log
// ---------------------------------------------------------------------------

/// What kind of change was announced. A persistence layer needs this to choose between an
/// INSERT, an UPDATE and a DELETE; the UI does not care, which is why it rides alongside the
/// path rather than inside it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    /// One field was written.
    Set,
    /// A row entered the collection.
    Insert,
    /// A row left it.
    Delete,
    /// The collection's order changed.
    Move,
}

/// One announced change, in the form a store outside the UI can consume.
///
/// `prior` and `value` are the slot's value before and after the write, captured only while a
/// consumer that wants them is active ([`record_values`]; later, an undo manager) — a write with
/// no such consumer clones nothing. They are type-erased; downcast with [`Change::prior_as`] /
/// [`Change::value_as`].
#[derive(Clone)]
pub struct Change {
    /// The path's components, outermost first: store, key, field, …
    pub components: Vec<u64>,
    /// The field's name — which is also its column name.
    pub label: &'static str,
    pub op: Op,
    pub prior: Option<Rc<dyn Any>>,
    pub value: Option<Rc<dyn Any>>,
}

impl Change {
    /// The value before the write, if captured and of type `T`.
    pub fn prior_as<T: 'static>(&self) -> Option<&T> {
        self.prior.as_deref().and_then(|v| v.downcast_ref())
    }

    /// The value after the write, if captured and of type `T`.
    pub fn value_as<T: 'static>(&self) -> Option<&T> {
        self.value.as_deref().and_then(|v| v.downcast_ref())
    }
}

impl fmt::Debug for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Change")
            .field("components", &self.components)
            .field("label", &self.label)
            .field("op", &self.op)
            .field("prior", &self.prior.as_ref().map(|_| ".."))
            .field("value", &self.value.as_ref().map(|_| ".."))
            .finish()
    }
}

fn values_wanted() -> bool {
    WANT_VALUES.with(|w| w.get())
}

/// Record every change announced inside `f`. The test seam, and the persistence layer's input.
pub fn record_changes<R>(f: impl FnOnce() -> R) -> (R, Vec<Change>) {
    RECORDER.with(|r| *r.borrow_mut() = Some(Vec::new()));
    let out = f();
    let log = RECORDER.with(|r| r.borrow_mut().take().unwrap_or_default());
    (out, log)
}

/// [`record_changes`], with prior/new values captured on every field write inside `f` — the
/// form an undo unit needs. Costs one clone per write while active, nothing when not.
pub fn record_values<R>(f: impl FnOnce() -> R) -> (R, Vec<Change>) {
    WANT_VALUES.with(|w| w.set(true));
    let out = record_changes(f);
    WANT_VALUES.with(|w| w.set(false));
    out
}

/// Just the labels, for the tests that only care which fields moved.
pub fn record<R>(f: impl FnOnce() -> R) -> (R, Vec<&'static str>) {
    let (out, changes) = record_changes(f);
    (out, changes.into_iter().map(|c| c.label).collect())
}

/// Wake the observers of this path and of every ancestor — and nobody else. `components` is
/// resolved lazily: the change log wants the full path as data, but building the `Vec` is wasted
/// work when nothing records, which is almost always.
fn notify_change(
    p: Path,
    components: impl FnOnce() -> Vec<u64>,
    label: &'static str,
    op: Op,
    prior: Option<Rc<dyn Any>>,
    value: Option<Rc<dyn Any>>,
) {
    RECORDER.with(|r| {
        if let Some(log) = r.borrow_mut().as_mut() {
            log.push(Change {
                components: components(),
                label,
                op,
                prior,
                value,
            });
        }
    });
    wake(p);
}

fn wake(p: Path) {
    let chain = p.chain();
    let live: Vec<Trigger> = TRIGGERS.with(|t| {
        let map = t.borrow();
        chain
            .iter()
            .filter_map(|k| map.get(k).map(|e| e.trigger))
            .collect()
    });
    for tr in live {
        tr.notify();
    }
}

/// Announce a change named by plain components — how a background transaction's writes reach the
/// triggers on the main thread. Wakes the deepest path whose interior steps are interned;
/// anything deeper cannot have an observer, because observing is what interns.
fn announce(parts: &[u64], label: &'static str) {
    let resolved = (|| {
        let (last, interior) = parts.split_last()?;
        let mut parent = ROOT;
        for part in interior {
            let step = Path::under(parent, *part);
            match NODES.with(|n| {
                let i = n.borrow();
                i.lookup.get(&step).copied().filter(|id| i.is_current(*id))
            }) {
                Some(id) => parent = id,
                // The step was never observed: wake it as a leaf, so coarser observers above
                // it still hear the change.
                None => return Some(step),
            }
        }
        Some(Path::under(parent, *last))
    })();
    let Some(p) = resolved else { return };
    RECORDER.with(|r| {
        if let Some(log) = r.borrow_mut().as_mut() {
            log.push(Change {
                components: parts.to_vec(),
                label,
                op: Op::Set,
                prior: None,
                value: None,
            });
        }
    });
    wake(p);
}

// ---------------------------------------------------------------------------
// Source
// ---------------------------------------------------------------------------

/// Anything a field can be projected out of: the store itself, one element of a keyed
/// collection, or another field (which is how nesting works).
///
/// `with_value_untracked` is the one every projection reads through — a field must subscribe to
/// its OWN path, not to its parent's, or the granularity is gone.
pub trait Source<T: 'static>: Copy + 'static {
    /// Whether this source's LOCATION can change over the handle's lifetime — a recycled list
    /// slot whose current row rotates. Fields projected from a dynamic source resolve their
    /// path through the source on every operation instead of trusting the one cached at
    /// projection time, and their tracked reads also run [`Source::track_extra`].
    const DYNAMIC: bool = false;

    /// Extra tracking a projected field's tracked reads perform. A dynamic source tracks its
    /// own rebind signal here, so a control bound at build follows the slot to its next row.
    fn track_extra(self) {}

    /// The path this source occupies. Implementations revalidate against the interner, so the
    /// result is always current.
    fn path(self) -> Path;
    /// This source's interned id, so a child can be built with no lookup of its own.
    ///
    /// Ids are minted per THREAD. They are the fast path for a handle built and used on the main
    /// thread, and must never cross a thread boundary — see `components`.
    fn node(self) -> NodeId;
    /// The path as plain components, resolved by walking the HANDLE chain rather than the
    /// interner. This is what crosses a thread boundary: a worker names what it changed with
    /// components, and the main thread re-establishes them on its own side when it announces.
    fn components(self, out: &mut Vec<u64>);
    fn with_value_untracked<R>(self, f: impl FnOnce(Option<&T>) -> R) -> R;
    /// Returns false when the value is gone (a deleted row) — in which case nothing is
    /// announced either.
    fn update_value(self, f: impl FnOnce(&mut T)) -> bool;
    fn bump_version(self);

    /// Tracked read of the whole value — the COARSE subscription.
    fn with<R>(self, f: impl FnOnce(Option<&T>) -> R) -> R {
        self.track_extra();
        track(self.path());
        self.with_value_untracked(f)
    }
}

/// What a transaction touched, in a form that survives the trip to the main thread: the path's
/// components, outermost first, plus the label the change log records.
pub type TxPaths = Vec<(Vec<u64>, &'static str)>;

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// A `Copy`, process-lifetime handle to one observable value.
pub struct Store<T: 'static> {
    inner: &'static Inner<T>,
    node: NodeId,
    root_id: u64,
}

impl<T> Clone for Store<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Store<T> {}

struct Inner<T> {
    data: RwLock<T>,
    pending: Mutex<TxPaths>,
    version: AtomicU64,
}

/// Store root ids are process-global so two stores can never collide.
static NEXT_STORE: AtomicU64 = AtomicU64::new(1);

impl<T: 'static> Store<T> {
    /// A process-lifetime store, leaked so the handle stays `Copy` — the property that makes a
    /// `Signal` pleasant to pass, kept. Create once (a `thread_local!` + accessor fn is the
    /// idiom); the scoped owner arrives with the persistence container.
    pub fn new(value: T) -> Store<T> {
        let inner: &'static Inner<T> = Box::leak(Box::new(Inner {
            data: RwLock::new(value),
            pending: Mutex::new(Vec::new()),
            version: AtomicU64::new(0),
        }));
        let root_id = NEXT_STORE.fetch_add(1, Ordering::Relaxed);
        let node = intern(Path::root(root_id));
        pin(node);
        Store {
            inner,
            node,
            root_id,
        }
    }

    pub fn with_untracked<R>(self, f: impl FnOnce(&T) -> R) -> R {
        f(&self.inner.data.read().expect("store poisoned"))
    }

    /// Mutate the whole value; announces a store-wide change (every reader wakes).
    pub fn update(self, label: &'static str, f: impl FnOnce(&mut T)) {
        {
            let mut g = self.inner.data.write().expect("store poisoned");
            f(&mut g);
        }
        self.inner.version.fetch_add(1, Ordering::Relaxed);
        let root_id = self.root_id;
        notify_change(
            path_of(self.node),
            || vec![root_id],
            label,
            Op::Set,
            None,
            None,
        );
    }

    /// Bumped on every write; a cheap way for tests to ask "did anything change".
    pub fn version(self) -> u64 {
        self.inner.version.load(Ordering::Relaxed)
    }

    /// Announce everything background transactions committed, returning how many paths were
    /// queued. An app posts this to the main loop after a worker commits; a test calls it
    /// directly and asserts what it woke.
    pub fn pump(self) -> usize {
        let drained: TxPaths = {
            let mut q = self.inner.pending.lock().expect("queue poisoned");
            std::mem::take(&mut *q)
        };
        let n = drained.len();
        day_reactive::batch(|| {
            for (parts, label) in drained {
                announce(&parts, label);
            }
        });
        n
    }
}

impl<T: 'static> Source<T> for Store<T> {
    fn path(self) -> Path {
        path_of(self.node)
    }
    fn node(self) -> NodeId {
        // Pinned at creation, so always current.
        self.node
    }
    fn components(self, out: &mut Vec<u64>) {
        out.push(self.root_id);
    }
    fn with_value_untracked<R>(self, f: impl FnOnce(Option<&T>) -> R) -> R {
        f(Some(&self.inner.data.read().expect("store poisoned")))
    }
    fn update_value(self, f: impl FnOnce(&mut T)) -> bool {
        let mut g = self.inner.data.write().expect("store poisoned");
        f(&mut g);
        true
    }
    fn bump_version(self) {
        self.inner.version.fetch_add(1, Ordering::Relaxed);
    }
}

// SAFETY: the handle is a shared reference to data behind a lock plus Copy ids. The reactive
// triggers those ids name are touched only on the main thread — writes from other threads go
// through `transact`, whose announcements are queued and delivered by `pump`.
unsafe impl<T: Send + Sync> Send for Store<T> {}
unsafe impl<T: Send + Sync> Sync for Store<T> {}

// ---------------------------------------------------------------------------
// Transactions
// ---------------------------------------------------------------------------

impl<T: Send + Sync + 'static> Store<T> {
    /// Open a write transaction — the way a background thread edits. Holding it holds the
    /// store's write lock, so a reader never sees half of one; dropping it commits and queues
    /// the announcements for [`Store::pump`].
    pub fn transact(self) -> Tx<T> {
        Tx {
            store: self,
            guard: Some(self.inner.data.write().expect("store poisoned")),
            touched: Vec::new(),
        }
    }
}

/// An open background transaction. Name what you touch with [`Field::touch`] (or push
/// components directly); commit by dropping.
pub struct Tx<T: Send + Sync + 'static> {
    store: Store<T>,
    guard: Option<std::sync::RwLockWriteGuard<'static, T>>,
    touched: TxPaths,
}

impl<T: Send + Sync + 'static> Tx<T> {
    pub fn data(&mut self) -> &mut T {
        // A Tx is only reachable while uncommitted: the guard is taken in Drop alone.
        self.guard.as_mut().expect("transaction already committed")
    }
    pub fn touched(&mut self, parts: Vec<u64>, label: &'static str) {
        self.touched.push((parts, label));
    }
    pub fn paths(&mut self) -> &mut TxPaths {
        &mut self.touched
    }
}

impl<T: Send + Sync + 'static> Drop for Tx<T> {
    fn drop(&mut self) {
        let touched = std::mem::take(&mut self.touched);
        let store = self.store;
        // Release the data lock FIRST: the commit IS the unlock.
        drop(self.guard.take());
        store.inner.version.fetch_add(1, Ordering::Relaxed);
        store
            .inner
            .pending
            .lock()
            .expect("queue poisoned")
            .extend(touched);
        // Announcing is the main thread's job — see `Store::pump`. (Scheduling the pump
        // automatically is the persistence container's job later; a bare store keeps delivery
        // explicit so a headless test owns its own timing.)
    }
}

// ---------------------------------------------------------------------------
// Keyed collections
// ---------------------------------------------------------------------------

/// Gives an element the stable key its paths are addressed by. Implemented by
/// `#[derive(Observable)]` from the `#[obs(key)]` field.
pub trait Identified {
    fn obs_key(&self) -> u64;
}

/// A keyed collection that keeps its own key→index map, so an element read is O(1) rather than
/// a scan — and no caller can forget to maintain the map, because the collection does it.
pub struct Keyed<T> {
    items: Vec<T>,
    index: HashMap<u64, usize>,
}

impl<T: Identified> Default for Keyed<T> {
    fn default() -> Self {
        Keyed {
            items: Vec::new(),
            index: HashMap::new(),
        }
    }
}

impl<T: Identified> Keyed<T> {
    pub fn new(items: Vec<T>) -> Self {
        let mut k = Keyed {
            items,
            index: HashMap::new(),
        };
        k.reindex();
        k
    }

    /// Rebuild the map. Called after any structural change; O(n) once, not per read.
    pub fn reindex(&mut self) {
        self.index.clear();
        for (i, item) in self.items.iter().enumerate() {
            self.index.insert(item.obs_key(), i);
        }
    }

    pub fn get(&self, key: u64) -> Option<&T> {
        self.index.get(&key).and_then(|i| self.items.get(*i))
    }

    pub fn get_mut(&mut self, key: u64) -> Option<&mut T> {
        match self.index.get(&key) {
            Some(i) => self.items.get_mut(*i),
            None => None,
        }
    }

    pub fn keys(&self) -> Vec<u64> {
        self.items.iter().map(|t| t.obs_key()).collect()
    }

    pub fn items(&self) -> &[T] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    // --- structural operations: each leaves the index correct ---------------------------------

    pub fn push(&mut self, item: T) {
        self.index.insert(item.obs_key(), self.items.len());
        self.items.push(item);
    }

    pub fn remove(&mut self, key: u64) -> Option<T> {
        let i = self.index.remove(&key)?;
        let item = self.items.remove(i);
        self.reindex();
        Some(item)
    }

    pub fn move_item(&mut self, from: usize, to: usize) {
        if from >= self.items.len() || to >= self.items.len() {
            return;
        }
        let item = self.items.remove(from);
        self.items.insert(to, item);
        self.reindex();
    }

    /// The raw list, for a structural edit inside [`Store::restructure`] that the helpers above
    /// do not cover. The caller's closure runs before the store reindexes.
    pub fn items_mut(&mut self) -> &mut Vec<T> {
        &mut self.items
    }
}

// ---------------------------------------------------------------------------
// Elements
// ---------------------------------------------------------------------------

/// One element of a keyed store, addressed by a key that survives reordering.
pub struct Elem<T: 'static> {
    store: Store<Keyed<T>>,
    key: u64,
    /// Interned once per handle, not once per field read; revalidated on use.
    node: NodeId,
}

impl<T> Clone for Elem<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Elem<T> {}

impl<T: Identified + 'static> Store<Keyed<T>> {
    /// One element, addressed by key. O(1) — the collection keeps a key→index map.
    pub fn elem(self, key: u64) -> Elem<T> {
        Elem {
            store: self,
            key,
            node: intern(Path::under(self.node, key)),
        }
    }

    /// Tracked read of the collection's SHAPE (which keys, in what order) — what a list widget
    /// wants. A field write does not wake it; an insert, a removal or a reorder does.
    pub fn keys(self) -> Vec<u64> {
        track(Path::under(self.node, STRUCTURE));
        self.with_untracked(|k| k.keys().to_vec())
    }

    /// Structural change: insert, remove, reorder.
    ///
    /// The affected key and the operation are announced alongside the shape path, because a
    /// persistence layer has to choose between an INSERT and a DELETE and cannot infer it from
    /// "the shape changed" — while the UI, which only re-reads `keys()`, ignores both.
    pub fn restructure(self, label: &'static str, op: Op, key: u64, f: impl FnOnce(&mut Keyed<T>)) {
        {
            let mut g = self.inner.data.write().expect("store poisoned");
            f(&mut g);
            g.reindex();
        }
        self.inner.version.fetch_add(1, Ordering::Relaxed);
        let root_id = self.root_id;
        notify_change(
            Path::under(self.node, key),
            || vec![root_id, key],
            label,
            op,
            None,
            None,
        );
        notify_change(
            Path::under(self.node, STRUCTURE),
            || vec![root_id, STRUCTURE],
            label,
            op,
            None,
            None,
        );
    }
}

impl<T: Identified + 'static> Elem<T> {
    /// The key this handle addresses.
    pub fn key(self) -> u64 {
        self.key
    }

    /// Whether the row is (still) present — TRACKED, so a guard re-runs when the row is deleted
    /// or comes back. This is the deletion story: reads of a gone row return the field's
    /// `Default`, and this is the one signal a page needs to degrade instead.
    pub fn exists(self) -> bool {
        track(self.path());
        self.with_value_untracked(|v| v.is_some())
    }

    /// The cached id when still live, a fresh interning otherwise — the self-healing described
    /// in the module docs.
    fn live_node(self) -> NodeId {
        if is_current(self.node) {
            self.node
        } else {
            intern(Path::under(self.store.node, self.key))
        }
    }
}

impl<T: Identified + 'static> Source<T> for Elem<T> {
    fn path(self) -> Path {
        // The store root is pinned, so this Path is stable across reclamation.
        Path::under(self.store.node, self.key)
    }
    fn node(self) -> NodeId {
        self.live_node()
    }
    fn components(self, out: &mut Vec<u64>) {
        self.store.components(out);
        out.push(self.key);
    }
    fn with_value_untracked<R>(self, f: impl FnOnce(Option<&T>) -> R) -> R {
        let g = self.store.inner.data.read().expect("store poisoned");
        f(g.get(self.key))
    }
    fn update_value(self, f: impl FnOnce(&mut T)) -> bool {
        let mut g = self.store.inner.data.write().expect("store poisoned");
        match g.get_mut(self.key) {
            Some(t) => {
                f(t);
                true
            }
            None => false,
        }
    }
    fn bump_version(self) {
        self.store.inner.version.fetch_add(1, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Fields
// ---------------------------------------------------------------------------

/// One field, projected out of any [`Source`]. `Copy`, itself a `Source` (so fields nest), and a
/// [`Binding`] (so every control binds to it).
pub struct Field<S, T: 'static, V: 'static> {
    src: S,
    path: Path,
    part: u64,
    label: &'static str,
    get: fn(&T) -> &V,
    get_mut: fn(&mut T) -> &mut V,
}

impl<S: Copy, T, V> Clone for Field<S, T, V> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<S: Copy, T, V> Copy for Field<S, T, V> {}

/// What the derive calls. Named `project` rather than `field` so a struct may have a field
/// called `field` without a collision.
pub fn project<S: Source<T>, T: 'static, V: 'static>(
    src: S,
    label: &'static str,
    get: fn(&T) -> &V,
    get_mut: fn(&mut T) -> &mut V,
) -> Field<S, T, V> {
    let part = field_id(label);
    Field {
        src,
        // No interning here: this is the per-read path, and the parent is already an id.
        path: Path::under(src.node(), part),
        part,
        label,
        get,
        get_mut,
    }
}

impl<S: Source<T>, T: 'static, V: 'static> Field<S, T, V> {
    /// The cached path when its parent is still live, a rebuilt one otherwise. A DYNAMIC
    /// source (a recycled slot) never trusts the cache: its location is wherever the source
    /// says it is right now.
    fn live_path(self) -> Path {
        if !S::DYNAMIC && is_current(self.path.parent) {
            self.path
        } else {
            Path::under(self.src.node(), self.part)
        }
    }

    /// Tracked read of THIS field only.
    pub fn with<R>(self, f: impl FnOnce(Option<&V>) -> R) -> R {
        self.src.track_extra();
        track(self.live_path());
        self.src.with_value_untracked(|t| f(t.map(self.get)))
    }

    pub fn with_untracked<R>(self, f: impl FnOnce(Option<&V>) -> R) -> R {
        self.src.with_value_untracked(|t| f(t.map(self.get)))
    }

    /// Write, waking readers of this field and of anything coarser. When a change-log consumer
    /// asked for values ([`record_values`]), the slot's prior and new value ride along.
    pub fn update(self, f: impl FnOnce(&mut V))
    where
        V: Clone,
    {
        let capture = values_wanted();
        let mut prior: Option<Rc<dyn Any>> = None;
        let mut after: Option<Rc<dyn Any>> = None;
        let get_mut = self.get_mut;
        let ok = self.src.update_value(|t| {
            let slot = get_mut(t);
            if capture {
                prior = Some(Rc::new(slot.clone()));
            }
            f(slot);
            if capture {
                after = Some(Rc::new(slot.clone()));
            }
        });
        if ok {
            self.src.bump_version();
            notify_change(
                self.live_path(),
                || {
                    let mut parts = Vec::new();
                    Source::<V>::components(self, &mut parts);
                    parts
                },
                self.label,
                Op::Set,
                prior,
                after,
            );
        }
    }

    /// Name this field inside a background transaction. Portable: what is queued is the path's
    /// components, which the main thread re-establishes on its own side when it announces.
    pub fn touch(self, tx: &mut TxPaths) {
        let mut parts = Vec::new();
        Source::<V>::components(self, &mut parts);
        tx.push((parts, self.label));
    }

    pub fn path(self) -> Path {
        self.live_path()
    }

    pub fn label(self) -> &'static str {
        self.label
    }
}

/// A field is itself a source, so `item.address().city()` works.
impl<S: Source<T>, T: 'static, V: 'static> Source<V> for Field<S, T, V> {
    // Nesting inherits the source's dynamism: a field of a slot's struct field still rides the
    // slot's current row.
    const DYNAMIC: bool = S::DYNAMIC;
    fn track_extra(self) {
        self.src.track_extra();
    }
    fn path(self) -> Path {
        self.live_path()
    }
    /// Interning happens HERE and only here: when a field is used as a parent, which is what
    /// nesting one struct inside another means.
    fn node(self) -> NodeId {
        intern(self.live_path())
    }
    fn components(self, out: &mut Vec<u64>) {
        self.src.components(out);
        out.push(self.part);
    }
    fn with_value_untracked<R>(self, f: impl FnOnce(Option<&V>) -> R) -> R {
        self.with_untracked(f)
    }
    fn update_value(self, f: impl FnOnce(&mut V)) -> bool {
        let get_mut = self.get_mut;
        self.src.update_value(|t| f(get_mut(t)))
    }
    fn bump_version(self) {
        self.src.bump_version();
    }
}

/// A field IS a two-way binding: every control takes one, unchanged.
impl<S: Source<T>, T: 'static, V: Clone + Default + 'static> Binding<V> for Field<S, T, V> {
    fn read(&self) -> V {
        self.with(|v| v.cloned().unwrap_or_default())
    }
    fn peek(&self) -> V {
        self.with_untracked(|v| v.cloned().unwrap_or_default())
    }
    fn write(&self, v: V) {
        self.update(move |slot| *slot = v);
    }
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

/// A converted view of a field: an ISO string as a date, `#RRGGBB` as a color, an index into a
/// table as the value it names. `Copy`, and still a binding, so it goes straight into a control.
pub struct Mapped<F, V: 'static, U: 'static> {
    inner: F,
    to: fn(&V) -> U,
    from: fn(&U) -> V,
}

impl<F: Copy, V, U> Clone for Mapped<F, V, U> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<F: Copy, V, U> Copy for Mapped<F, V, U> {}

impl<S: Source<T>, T: 'static, V: 'static> Field<S, T, V> {
    /// Read this field through `to`, write it back through `from`. Both are fn pointers, so the
    /// result stays `Copy`.
    pub fn map<U: 'static>(self, to: fn(&V) -> U, from: fn(&U) -> V) -> Mapped<Self, V, U> {
        Mapped {
            inner: self,
            to,
            from,
        }
    }
}

/// The MISSING case converts the stored type's default rather than requiring the UI type to have
/// one — a `Color` has no `Default`, and asking every converted type for one would be a tax paid
/// by every caller for a case only a deleted row reaches.
impl<F: Source<V>, V: Clone + Default + 'static, U: Clone + 'static> Binding<U>
    for Mapped<F, V, U>
{
    fn read(&self) -> U {
        self.inner.track_extra();
        track(self.inner.path());
        self.peek()
    }
    fn peek(&self) -> U {
        self.inner.with_value_untracked(|v| match v {
            Some(v) => (self.to)(v),
            None => (self.to)(&V::default()),
        })
    }
    fn write(&self, u: U) {
        let v = (self.from)(&u);
        let inner = self.inner;
        let capture = values_wanted();
        let mut prior: Option<Rc<dyn Any>> = None;
        let mut after: Option<Rc<dyn Any>> = None;
        let ok = inner.update_value(|slot| {
            if capture {
                prior = Some(Rc::new(slot.clone()));
            }
            *slot = v;
            if capture {
                after = Some(Rc::new(slot.clone()));
            }
        });
        if ok {
            inner.bump_version();
            notify_change(
                inner.path(),
                || {
                    let mut parts = Vec::new();
                    inner.components(&mut parts);
                    parts
                },
                "mapped",
                Op::Set,
                prior,
                after,
            );
        }
    }
}

impl<F: Source<V> + Copy, V: 'static> Mapped<F, V, usize> {
    /// `usize` as the `f64` a slider speaks — the adapter every numeric control needs somewhere.
    pub fn map_to_f64(self) -> Numeric<Self> {
        Numeric { inner: self }
    }
}

/// `usize` ⇄ `f64`.
pub struct Numeric<M> {
    inner: M,
}

impl<M: Copy> Clone for Numeric<M> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<M: Copy> Copy for Numeric<M> {}

impl<M: Binding<usize> + Copy + 'static> Binding<f64> for Numeric<M> {
    fn read(&self) -> f64 {
        self.inner.read() as f64
    }
    fn peek(&self) -> f64 {
        self.inner.peek() as f64
    }
    fn write(&self, v: f64) {
        self.inner.write(v.round().max(0.0) as usize);
    }
}
