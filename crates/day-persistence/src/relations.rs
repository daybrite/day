// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Relations (docs/persistence.md): `One<M>` foreign keys, `Many<M>` maintained inverses,
//! delete rules, and the machinery that keeps them true.
//!
//! There is ONE source of truth per to-one relation — the foreign-key column on the child —
//! and the parent's `Many` side is an INDEX over it, maintained from the container's change
//! sink. That is what makes the inverses SwiftData promises come out of the existing
//! pipeline instead of parallel bookkeeping: write `lodging.trip()` and the trip's
//! `lodging()` read wakes; call `trip.lodging().add(id)` and it writes the lodging's foreign
//! key through the front door, so the change announces, captures for undo, folds to one
//! `UPDATE`, and animates any live query watching either table.
//!
//! Delete rules run where the delete announces: a parent's removal cascades (recursively —
//! the nested deletes take the same pipeline, so undoing the cascade is one unit that
//! restores the whole subtree), nullifies (`Option<One<M>>` references clear), or denies
//! (the checked door [`crate::ModelContainer::delete`] refuses while children remain). The
//! generated DDL carries the matching `REFERENCES … ON DELETE …` clause, `DEFERRABLE
//! INITIALLY DEFERRED` so within-transaction statement order never trips it — which also
//! keeps another process honest about the same rules.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::{Rc, Weak};

use day_model::{ApplyField, Keyed, ModelId, Op, Store, announce, field_id};

use crate::{
    ColumnValue, ContainerInner, DbError, DbErrorKind, Model, ModelContainer, Row, SqlType, Value,
    key_param, value_to_handle,
};

/// Read one row's order value by handle; write it back. The pair is what an ordered relation
/// needs, whichever shape holds the order (a child's field, or a membership's position).
type ReadOrder = Box<dyn Fn(u64) -> f64>;
type WriteOrder = Box<dyn Fn(u64, f64) -> bool>;

/// What placing one row inside an ordered relation needs, resolved from whichever shape holds
/// the order.
struct Placement {
    ordered: bool,
    read_ord: ReadOrder,
    write_ord: WriteOrder,
    is_member: bool,
}

// ---------------------------------------------------------------------------
// The reference types
// ---------------------------------------------------------------------------

/// A to-one reference — the foreign-key column on the child row. `One<M>` alone is a required
/// reference (`NOT NULL`); wrap it `Option<One<M>>` for a nullable one. `Copy`, 16 bytes, and
/// an ordinary [`ColumnValue`], so the derive treats the field as a plain column whose stored
/// form is the target's own key shape.
pub struct One<M: ?Sized> {
    id: Option<u64>,
    _m: PhantomData<fn() -> M>,
}

impl<M: ?Sized> Clone for One<M> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<M: ?Sized> Copy for One<M> {}
impl<M: ?Sized> Default for One<M> {
    /// Unset. A required (`NOT NULL`) reference left at its default surfaces at flush time as
    /// the constraint violation it is; set it before the turn ends.
    fn default() -> Self {
        One {
            id: None,
            _m: PhantomData,
        }
    }
}
impl<M: ?Sized> PartialEq for One<M> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl<M: ?Sized> Eq for One<M> {}
impl<M: ?Sized> std::hash::Hash for One<M> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}
impl<M: ?Sized> std::fmt::Debug for One<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.id {
            Some(h) => write!(f, "One({:?})", ModelId::<M>::from_handle(h)),
            None => f.write_str("One(unset)"),
        }
    }
}

impl<M: ?Sized> One<M> {
    /// A reference to `id`.
    pub fn to(id: impl Into<ModelId<M>>) -> Self {
        One {
            id: Some(id.into().handle()),
            _m: PhantomData,
        }
    }
    /// The unset reference (what [`Default`] gives).
    pub const fn none() -> Self {
        One {
            id: None,
            _m: PhantomData,
        }
    }
    /// The referenced id — `None` when unset.
    pub fn id(self) -> Option<ModelId<M>> {
        self.id.map(ModelId::from_handle)
    }
    pub(crate) fn from_handle(h: u64) -> Self {
        One {
            id: Some(h),
            _m: PhantomData,
        }
    }
    pub(crate) fn from_handle_opt(h: Option<u64>) -> Self {
        One {
            id: h,
            _m: PhantomData,
        }
    }
    pub(crate) fn handle_opt(self) -> Option<u64> {
        self.id
    }
}

impl<M: Model> ColumnValue for One<M> {
    /// The target's own key shape — `INTEGER`, a 16-byte `BLOB`, or `TEXT`.
    const SQL_TYPE: SqlType = M::KEY_SQL;
    fn to_sqlite_value(&self) -> Value {
        match self.id {
            Some(h) => key_param(h),
            None => Value::Null,
        }
    }
    fn from_sqlite_value(v: Value) -> Result<Self, DbError> {
        match value_to_handle(&v) {
            Some(h) => Ok(One::from_handle(h)),
            None => Err(DbError::new(
                DbErrorKind::Decode,
                format!("a stored reference would not read as a key: {v:?}"),
            )),
        }
    }
}

/// The to-many side: a marker field. It stores NOTHING — membership lives in the children's
/// foreign keys (or the join table), and the field exists so the relation has an observable
/// path of its own and a declared home for `#[model(relation(…))]`. Reads go through the
/// generated accessor's [`RelationRef`].
pub struct Many<M: ?Sized> {
    _m: PhantomData<fn() -> M>,
}

impl<M: ?Sized> Clone for Many<M> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<M: ?Sized> Copy for Many<M> {}
impl<M: ?Sized> Default for Many<M> {
    fn default() -> Self {
        Many { _m: PhantomData }
    }
}
impl<M: ?Sized> PartialEq for Many<M> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl<M: ?Sized> Eq for Many<M> {}
impl<M: ?Sized> std::fmt::Debug for Many<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Many")
    }
}

/// What deleting a referenced parent does. Declared per relation
/// (`#[model(relation(delete = "…"))]`); the default is `Nullify`, SwiftData's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeleteRule {
    /// Children survive; their references clear. Needs `Option<One<M>>` on the child —
    /// a required reference cannot hold "nothing", and wiring refuses the combination.
    Nullify,
    /// Children delete with the parent, recursively, through the normal pipeline — undoable
    /// as one unit, animated by live queries.
    Cascade,
    /// The delete is refused while children remain — through
    /// [`crate::ModelContainer::delete`], the checked door; a raw `restructure` bypasses the
    /// in-memory check and the deferred SQL `RESTRICT` refuses at flush instead.
    Deny,
}

impl DeleteRule {
    pub(crate) fn sql(self) -> &'static str {
        match self {
            DeleteRule::Nullify => "SET NULL",
            DeleteRule::Cascade => "CASCADE",
            DeleteRule::Deny => "RESTRICT",
        }
    }
}

/// One declared relation, as the derive records it on the PARENT (`Many`) side.
#[derive(Clone, Copy, Debug)]
pub struct RelationDef {
    /// The `Many` field's name on the declaring model.
    pub field: &'static str,
    /// The target model's table.
    pub target_table: &'static str,
    /// To-one/to-many: the target's foreign-key FIELD. Join relations leave it empty.
    pub inverse: &'static str,
    pub delete: DeleteRule,
    /// The target's order FIELD (`REAL`) for an ordered to-many.
    pub ordered: Option<&'static str>,
    /// A join table name — the many-to-many form.
    pub join: Option<&'static str>,
}

// ---------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------

/// What `Model::wire` receives — the container mid-open. Wiring failures collect here and
/// fail the open with the first one, named.
pub struct Registrar<'c> {
    pub(crate) container: &'c ModelContainer,
    pub(crate) error: Option<DbError>,
}

impl Registrar<'_> {
    fn fail(&mut self, message: String) {
        if self.error.is_none() {
            self.error = Some(DbError::new(DbErrorKind::Schema, message));
        }
    }
}

/// One wired to-one/to-many relation: the closures are monomorphized over both model types by
/// [`wire_to_many`], so every write goes through the front door with full type knowledge.
pub(crate) struct ToOneRel {
    pub(crate) parent_store: u64,
    pub(crate) parent_field: &'static str,
    pub(crate) child_store: u64,
    pub(crate) fk_field: &'static str,
    pub(crate) delete: DeleteRule,
    pub(crate) ordered: Option<&'static str>,
    read_fk: Box<dyn Fn(u64) -> Option<u64>>,
    write_fk: Box<dyn Fn(u64, Option<u64>) -> bool>,
    delete_child: Box<dyn Fn(u64)>,
    read_ord: ReadOrder,
    pub(crate) write_ord: WriteOrder,
    index: RefCell<RelIndex>,
}

#[derive(Default)]
struct RelIndex {
    children: HashMap<u64, Vec<u64>>,
    parent_of: HashMap<u64, u64>,
}

impl ToOneRel {
    /// The parent's children, in order (the order field's, or insertion). O(1) plus the copy.
    pub(crate) fn children_of(&self, parent: u64) -> Vec<u64> {
        self.index
            .borrow()
            .children
            .get(&parent)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn read_order(&self, child: u64) -> f64 {
        (self.read_ord)(child)
    }

    pub(crate) fn set_fk(&self, child: u64, parent: Option<u64>) -> bool {
        // The write announces; the sink routes it back through `reparent`, which is where the
        // index and both parents' announcements happen — one path for every door.
        (self.write_fk)(child, parent)
    }

    fn announce_parent(&self, parent: u64) {
        announce(
            &[self.parent_store, parent, field_id(self.parent_field)],
            self.parent_field,
        );
    }

    fn attach_child(&self, parent: u64, child: u64) {
        let mut idx = self.index.borrow_mut();
        idx.parent_of.insert(child, parent);
        let vec = idx.children.entry(parent).or_default();
        if self.ordered.is_some() {
            let ord = (self.read_ord)(child);
            let at = vec
                .iter()
                .position(|c| (self.read_ord)(*c) > ord)
                .unwrap_or(vec.len());
            vec.insert(at, child);
        } else {
            // Unordered relations still answer DETERMINISTICALLY — ascending by handle — so
            // the order survives reloads, undo rebuilds, and external merges identically.
            let at = vec.partition_point(|c| *c < child);
            vec.insert(at, child);
        }
    }

    fn detach_child(&self, child: u64) -> Option<u64> {
        let mut idx = self.index.borrow_mut();
        let parent = idx.parent_of.remove(&child)?;
        if let Some(vec) = idx.children.get_mut(&parent) {
            vec.retain(|c| *c != child);
        }
        Some(parent)
    }

    pub(crate) fn child_added(&self, child: u64) {
        if let Some(p) = (self.read_fk)(child) {
            self.attach_child(p, child);
            self.announce_parent(p);
        }
    }

    pub(crate) fn child_removed(&self, child: u64) {
        if let Some(p) = self.detach_child(child) {
            self.announce_parent(p);
        }
    }

    pub(crate) fn reparent(&self, child: u64) {
        let old = self.index.borrow().parent_of.get(&child).copied();
        let new = (self.read_fk)(child);
        if old == new {
            return;
        }
        if old.is_some() {
            self.detach_child(child);
        }
        if let Some(p) = new {
            self.attach_child(p, child);
        }
        for p in [old, new].into_iter().flatten() {
            self.announce_parent(p);
        }
    }

    /// The child's order field moved: reposition it and wake the parent's readers.
    pub(crate) fn order_changed(&self, child: u64) {
        let Some(parent) = self.index.borrow().parent_of.get(&child).copied() else {
            return;
        };
        self.detach_child(child);
        self.attach_child(parent, child);
        self.announce_parent(parent);
    }

    pub(crate) fn parent_deleted(&self, container: &ModelContainer, parent: u64) {
        let children = self.children_of(parent);
        if children.is_empty() {
            self.index.borrow_mut().children.remove(&parent);
            return;
        }
        match self.delete {
            DeleteRule::Cascade => {
                // Each nested delete takes the whole pipeline — announce, undo capture,
                // dirty, queries — and, for a self-referential tree, recurses through this
                // same method one level down.
                for child in children {
                    (self.delete_child)(child);
                }
            }
            DeleteRule::Nullify => {
                for child in children {
                    (self.write_fk)(child, None);
                }
            }
            DeleteRule::Deny => {
                // Too late to refuse — the row is gone from the store. The checked door
                // (`ModelContainer::delete`) refuses BEFORE; a bypass surfaces here and at
                // the deferred SQL RESTRICT.
                container.inner.error.set(Some(format!(
                    "deny: `{}` was deleted while `{}` rows still reference it — use \
                     ModelContainer::delete for deny relations",
                    self.parent_field, self.fk_field
                )));
            }
        }
        self.index.borrow_mut().children.remove(&parent);
    }
}

/// Read a child's foreign key, whichever nullability the field declared.
fn fk_of<P: Model, C: Model>(c: &C, inverse: &str) -> Option<u64> {
    let any = ApplyField::read_field(c, inverse)?;
    if let Some(one) = any.downcast_ref::<One<P>>() {
        return one.handle_opt();
    }
    if let Some(opt) = any.downcast_ref::<Option<One<P>>>() {
        return opt.and_then(|o| o.handle_opt());
    }
    None
}

/// Wire one `Many` declaration — called from the derive-generated `Model::wire`.
pub fn wire_to_many<P: Model, C: Model>(
    reg: &mut Registrar<'_>,
    field: &'static str,
    inverse: &'static str,
    delete: DeleteRule,
    ordered: Option<&'static str>,
) {
    let container = reg.container.clone();
    let Some(parent_store) = container.try_store::<P>() else {
        reg.fail(format!(
            "relation `{}.{field}` wired before `{}` attached",
            P::TABLE,
            P::TABLE
        ));
        return;
    };
    let Some(child_store) = container.try_store::<C>() else {
        reg.fail(format!(
            "relation `{}.{field}` targets `{}`, which is not in this container's schema!",
            P::TABLE,
            C::TABLE
        ));
        return;
    };
    let Some(col) = C::COLUMNS.iter().find(|c| c.field == inverse) else {
        reg.fail(format!(
            "relation `{}.{field}` names inverse `{inverse}`, which is not a persisted \
             field of `{}`",
            P::TABLE,
            C::TABLE
        ));
        return;
    };
    if delete == DeleteRule::Nullify && col.not_null {
        reg.fail(format!(
            "relation `{}.{field}` is delete = \"nullify\", but `{}.{inverse}` is a required \
             `One<…>` — make it `Option<One<…>>`, or use cascade/deny",
            P::TABLE,
            C::TABLE
        ));
        return;
    }
    if let Some(ord) = ordered {
        let ok = C::COLUMNS
            .iter()
            .any(|c| c.field == ord && c.sql == SqlType::Real);
        if !ok {
            reg.fail(format!(
                "relation `{}.{field}` is ordered by `{ord}`, which is not a REAL (`f64`) \
                 field of `{}` — the order column is a real, visible field of the child",
                P::TABLE,
                C::TABLE
            ));
            return;
        }
    }

    let read_fk = Box::new(move |h: u64| {
        child_store.with_untracked(|k| k.get(h).and_then(|c| fk_of::<P, C>(c, inverse)))
    });
    let write_fk = Box::new(move |h: u64, parent: Option<u64>| {
        if child_store.write_field(h, inverse, One::<P>::from_handle_opt(parent)) {
            return true;
        }
        let opt: Option<One<P>> = parent.map(One::<P>::from_handle);
        child_store.write_field(h, inverse, opt)
    });
    let delete_child = Box::new(move |h: u64| {
        child_store.restructure("cascade", Op::Delete, h, |k| {
            k.remove(h);
        });
    });
    let (read_ord, write_ord): (ReadOrder, WriteOrder) = match ordered {
        Some(ord) => (
            Box::new(move |h| {
                child_store.with_untracked(|k| {
                    k.get(h)
                        .and_then(|c| ApplyField::read_field(c, ord))
                        .and_then(|v| v.downcast_ref::<f64>().copied())
                        .unwrap_or(0.0)
                })
            }),
            Box::new(move |h, v| child_store.write_field(h, ord, v)),
        ),
        None => (Box::new(|_| 0.0), Box::new(|_, _| false)),
    };

    let rel = Rc::new(ToOneRel {
        parent_store: parent_store.store_id(),
        parent_field: field,
        child_store: child_store.store_id(),
        fk_field: inverse,
        delete,
        ordered,
        read_fk,
        write_fk,
        delete_child,
        read_ord,
        write_ord,
        index: RefCell::new(RelIndex::default()),
    });

    // Seed the index from the loaded rows — O(n) once, at open.
    child_store.with_untracked(|k| {
        for item in k.items() {
            let child = item.handle();
            if let Some(parent) = fk_of::<P, C>(item, inverse) {
                rel.attach_child(parent, child);
            }
        }
    });

    container.inner.relations.borrow_mut().push(rel);
}

// ---------------------------------------------------------------------------
// Many-to-many: the join table
// ---------------------------------------------------------------------------

/// One membership row of a generated join table. Its key is the PAIR — that is what makes a
/// membership addressable, undoable and mergeable by the same machinery every other row uses,
/// with no second vocabulary.
#[derive(Clone, Default, PartialEq, Debug)]
pub(crate) struct JoinRow {
    pub(crate) parent: u64,
    pub(crate) child: u64,
    /// The ordered form's position; 0.0 when the relation is unordered.
    pub(crate) position: f64,
}

impl day_model::Identified for JoinRow {
    fn key(&self) -> day_model::Key {
        day_model::Key::Pair(self.parent, self.child)
    }
}

impl ApplyField for JoinRow {
    fn apply_field(&mut self, label: &str, value: &dyn std::any::Any) -> bool {
        match (label, value.downcast_ref::<f64>()) {
            ("position", Some(v)) => {
                self.position = *v;
                true
            }
            _ => false,
        }
    }
    fn read_field(&self, label: &str) -> Option<Rc<dyn std::any::Any>> {
        match label {
            "position" => Some(Rc::new(self.position)),
            _ => None,
        }
    }
}

/// A wired many-to-many — ONE per join table, whichever side (or both) declared it.
///
/// Membership lives in the join store, one row per pair, and the index mirrors it in both
/// directions, so `tag.notes()` and `note.tags()` both answer O(1) from the same rows. When
/// both models declare the relation over the same `join = "…"` table, the second declaration
/// attaches as this relation's B side rather than opening a second store — two stores over
/// one table would double every write and disagree about column order.
pub(crate) struct JoinRel {
    pub(crate) join_table: &'static str,
    /// The side that wired first: its rows are the join row's `parent`.
    pub(crate) a_store: u64,
    pub(crate) a_field: &'static str,
    pub(crate) a_delete: DeleteRule,
    /// The other side; `b_field` is set only if that model declared the relation too —
    /// through a `Cell`, because the second declaration fills it in during wiring while the
    /// first declaration's `Rc` is already held.
    pub(crate) b_store: u64,
    pub(crate) b_field: Cell<Option<&'static str>>,
    pub(crate) b_delete: Cell<DeleteRule>,
    pub(crate) join_store: u64,
    /// Order is the A side's: a membership's position places a B row within one A row.
    pub(crate) ordered: bool,
    store: Store<Keyed<JoinRow>>,
    delete_a: Box<dyn Fn(u64)>,
    delete_b: Box<dyn Fn(u64)>,
    index: RefCell<JoinIndex>,
}

#[derive(Default)]
struct JoinIndex {
    by_parent: HashMap<u64, Vec<u64>>,
    by_child: HashMap<u64, Vec<u64>>,
}

impl JoinRel {
    /// The members `key` holds, read from whichever side `forward` names: A→B forward, B→A
    /// reverse.
    pub(crate) fn members_of(&self, key: u64, forward: bool) -> Vec<u64> {
        let idx = self.index.borrow();
        let map = if forward {
            &idx.by_parent
        } else {
            &idx.by_child
        };
        map.get(&key).cloned().unwrap_or_default()
    }

    fn position_of(&self, parent: u64, child: u64) -> f64 {
        self.store
            .with_untracked(|k| {
                k.get(day_model::Key::Pair(parent, child).handle())
                    .map(|r| r.position)
            })
            .unwrap_or(0.0)
    }

    /// Wake BOTH sides' readers: a membership change moves `a.field()` and `b.field()` alike.
    fn announce_pair(&self, parent: u64, child: u64) {
        announce(
            &[self.a_store, parent, field_id(self.a_field)],
            self.a_field,
        );
        if let Some(bf) = self.b_field.get() {
            announce(&[self.b_store, child, field_id(bf)], bf);
        }
    }

    fn insert_index(&self, parent: u64, child: u64) {
        let mut idx = self.index.borrow_mut();
        let vec = idx.by_parent.entry(parent).or_default();
        if !vec.contains(&child) {
            if self.ordered {
                let pos = self.position_of(parent, child);
                let at = vec
                    .iter()
                    .position(|c| self.position_of(parent, *c) > pos)
                    .unwrap_or(vec.len());
                vec.insert(at, child);
            } else {
                let at = vec.partition_point(|c| *c < child);
                vec.insert(at, child);
            }
        }
        let back = idx.by_child.entry(child).or_default();
        if !back.contains(&parent) {
            let at = back.partition_point(|p| *p < parent);
            back.insert(at, parent);
        }
    }

    fn remove_index(&self, parent: u64, child: u64) {
        let mut idx = self.index.borrow_mut();
        if let Some(v) = idx.by_parent.get_mut(&parent) {
            v.retain(|c| *c != child);
        }
        if let Some(v) = idx.by_child.get_mut(&child) {
            v.retain(|p| *p != parent);
        }
    }

    /// A join row arrived (an `add`, an undo's re-insert, an external merge).
    pub(crate) fn row_added(&self, handle: u64) {
        if let Some(day_model::Key::Pair(p, c)) = day_model::Key::of_handle(handle) {
            self.insert_index(p, c);
            self.announce_pair(p, c);
        }
    }

    pub(crate) fn row_removed(&self, handle: u64) {
        if let Some(day_model::Key::Pair(p, c)) = day_model::Key::of_handle(handle) {
            self.remove_index(p, c);
            self.announce_pair(p, c);
        }
    }

    /// A membership's position moved: reposition and wake both sides.
    pub(crate) fn row_moved(&self, handle: u64) {
        if let Some(day_model::Key::Pair(p, c)) = day_model::Key::of_handle(handle) {
            self.remove_index(p, c);
            self.insert_index(p, c);
            self.announce_pair(p, c);
        }
    }

    /// Either side's row was deleted: its memberships go with it, and under that side's own
    /// cascade rule so do the rows across the join that nobody else still holds.
    pub(crate) fn side_deleted(&self, is_a: bool, key: u64) {
        let pairs: Vec<(u64, u64)> = {
            let idx = self.index.borrow();
            if is_a {
                idx.by_parent
                    .get(&key)
                    .map(|v| v.iter().map(|c| (key, *c)).collect())
                    .unwrap_or_default()
            } else {
                idx.by_child
                    .get(&key)
                    .map(|v| v.iter().map(|p| (*p, key)).collect())
                    .unwrap_or_default()
            }
        };
        for (p, c) in &pairs {
            let handle = day_model::Key::Pair(*p, *c).handle();
            self.store.restructure("unlink", Op::Delete, handle, |k| {
                k.remove(handle);
            });
        }
        let rule = if is_a {
            self.a_delete
        } else {
            self.b_delete.get()
        };
        if rule == DeleteRule::Cascade {
            // Cascade across a join takes the rows this one held — but only those no OTHER
            // row still holds, or deleting one album would take a shared photo with it.
            for (p, c) in pairs {
                let (other, still_held) = {
                    let idx = self.index.borrow();
                    if is_a {
                        (c, idx.by_child.get(&c).is_some_and(|v| !v.is_empty()))
                    } else {
                        (p, idx.by_parent.get(&p).is_some_and(|v| !v.is_empty()))
                    }
                };
                if !still_held {
                    if is_a {
                        (self.delete_b)(other);
                    } else {
                        (self.delete_a)(other);
                    }
                }
            }
        }
    }

    fn link(&self, parent: u64, child: u64, position: f64) -> bool {
        let handle = day_model::Key::Pair(parent, child).handle();
        if self.store.with_untracked(|k| k.get(handle).is_some()) {
            return false; // already a member — membership is a set
        }
        self.store.restructure("link", Op::Insert, handle, |k| {
            k.push(JoinRow {
                parent,
                child,
                position,
            })
        });
        true
    }

    fn unlink(&self, parent: u64, child: u64) -> bool {
        let handle = day_model::Key::Pair(parent, child).handle();
        if self.store.with_untracked(|k| k.get(handle).is_none()) {
            return false;
        }
        self.store.restructure("unlink", Op::Delete, handle, |k| {
            k.remove(handle);
        });
        true
    }
}

/// Wire one many-to-many — called from the derive-generated `Model::wire`.
pub fn wire_join<P: Model, C: Model>(
    reg: &mut Registrar<'_>,
    field: &'static str,
    join_table: &'static str,
    ordered: Option<&'static str>,
    delete: DeleteRule,
) {
    let container = reg.container.clone();
    let (Some(parent_store), Some(child_store)) =
        (container.try_store::<P>(), container.try_store::<C>())
    else {
        reg.fail(format!(
            "relation `{}.{field}` targets `{}`, which is not in this container's schema!",
            P::TABLE,
            C::TABLE
        ));
        return;
    };

    // The other side already wired this table: attach as its B side — one relation, one
    // store, both directions — rather than opening a second store over the same rows.
    {
        let joins = container.inner.joins.borrow();
        if let Some(existing) = joins.iter().find(|j| j.join_table == join_table) {
            let ok = existing.a_store == child_store.store_id()
                && existing.b_store == parent_store.store_id();
            if !ok {
                let msg = format!(
                    "join table `{join_table}` is declared by `{}.{field}` and by another \
                     relation over different models — a join table belongs to exactly one pair",
                    P::TABLE
                );
                drop(joins);
                reg.fail(msg);
                return;
            }
            if existing.b_field.get().is_some() {
                let msg = format!(
                    "join table `{join_table}` is declared twice on `{}` — one field per side",
                    P::TABLE
                );
                drop(joins);
                reg.fail(msg);
                return;
            }
            if ordered.is_some() && !existing.ordered {
                let msg = format!(
                    "relation `{}.{field}` asks for order, but `{join_table}`'s other side \
                     declared it unordered — a membership has ONE position, so order is \
                     declared on one side only",
                    P::TABLE
                );
                drop(joins);
                reg.fail(msg);
                return;
            }
            existing.b_field.set(Some(field));
            existing.b_delete.set(delete);
            return;
        }
    }

    if ordered.is_some_and(|o| o != "position") {
        reg.fail(format!(
            "relation `{}.{field}` is a join relation: its order lives on the join row, so \
             `ordered` takes no field name (a child can sit at different positions under \
             different parents)",
            P::TABLE
        ));
        return;
    }
    let ordered = ordered.is_some();

    // Column names are derived and documented: the singular of each table, `_id`-suffixed.
    let parent_col: &'static str = Box::leak(format!("{}_id", singular(P::TABLE)).into_boxed_str());
    let child_col: &'static str = Box::leak(format!("{}_id", singular(C::TABLE)).into_boxed_str());
    if parent_col == child_col {
        reg.fail(format!(
            "relation `{}.{field}`: a self-join needs distinct column names, which the \
             derived `{parent_col}` pair cannot give",
            P::TABLE
        ));
        return;
    }

    if let Err(e) =
        container.ensure_join_table::<P, C>(join_table, parent_col, child_col, ordered, delete)
    {
        reg.error.get_or_insert(e);
        return;
    }

    // Load the memberships this file already holds.
    let mut rows: Vec<JoinRow> = Vec::new();
    let mut load_err = None;
    let sql = if ordered {
        format!("SELECT {parent_col}, {child_col}, position FROM {join_table}")
    } else {
        format!("SELECT {parent_col}, {child_col} FROM {join_table}")
    };
    if let Err(e) = container.conn().query(&sql, &[], &mut |row| match (
        value_to_handle(&row.get(0)),
        value_to_handle(&row.get(1)),
    ) {
        (Some(p), Some(c)) => rows.push(JoinRow {
            parent: p,
            child: c,
            position: if ordered {
                row.get(2).as_real().unwrap_or(0.0)
            } else {
                0.0
            },
        }),
        _ => {
            load_err = Some(DbError::new(
                DbErrorKind::Decode,
                format!("`{join_table}` holds a membership whose ids will not read as keys"),
            ))
        }
    }) {
        reg.error.get_or_insert(e);
        return;
    }
    if let Some(e) = load_err {
        reg.error.get_or_insert(e);
        return;
    }

    let store = Store::new(Keyed::new(rows));
    let delete_a = Box::new(move |h: u64| {
        parent_store.restructure("cascade", Op::Delete, h, |k| {
            k.remove(h);
        });
    });
    let delete_b = Box::new(move |h: u64| {
        child_store.restructure("cascade", Op::Delete, h, |k| {
            k.remove(h);
        });
    });
    let rel = Rc::new(JoinRel {
        join_table,
        a_store: parent_store.store_id(),
        a_field: field,
        a_delete: delete,
        b_store: child_store.store_id(),
        b_field: Cell::new(None),
        b_delete: Cell::new(DeleteRule::Nullify),
        join_store: store.store_id(),
        ordered,
        store,
        delete_a,
        delete_b,
        index: RefCell::new(JoinIndex::default()),
    });
    store.with_untracked(|k| {
        for row in k.items() {
            rel.insert_index(row.parent, row.child);
        }
    });

    container.register_join_hooks(join_table, parent_col, child_col, ordered, store);
    container.inner.joins.borrow_mut().push(rel);
}

/// `trips` → `trip`, `boxes` → `box`, `categories` → `category` — the join column's stem.
fn singular(table: &str) -> String {
    if let Some(stem) = table.strip_suffix("ies") {
        return format!("{stem}y");
    }
    for suffix in ["ses", "xes", "zes", "ches", "shes"] {
        if let Some(stem) = table.strip_suffix(suffix) {
            return format!("{stem}{}", &suffix[..suffix.len() - 2]);
        }
    }
    table.strip_suffix('s').unwrap_or(table).to_string()
}

// ---------------------------------------------------------------------------
// The accessor surface
// ---------------------------------------------------------------------------

thread_local! {
    /// Every open container, so a `RelationRef` (which holds only a Field) can find the
    /// relation its parent store belongs to. Weak: dropping the container is the removal.
    static CONTAINERS: RefCell<Vec<Weak<ContainerInner>>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn register_container(inner: &Rc<ContainerInner>) {
    CONTAINERS.with(|c| {
        let mut v = c.borrow_mut();
        v.retain(|w| w.strong_count() > 0);
        v.push(Rc::downgrade(inner));
    });
}

/// A resolved relation, either shape — and for a join, which end asked (`forward` is the A
/// side, the one whose declaration created the table).
enum Wired {
    ToOne(Rc<ToOneRel>),
    Join(Rc<JoinRel>, bool),
}

fn find_relation(parent_store: u64, field: &str) -> Option<Wired> {
    CONTAINERS.with(|c| {
        c.borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .find_map(|inner| {
                if let Some(r) = inner
                    .relations
                    .borrow()
                    .iter()
                    .find(|r| r.parent_store == parent_store && r.parent_field == field)
                {
                    return Some(Wired::ToOne(r.clone()));
                }
                inner.joins.borrow().iter().find_map(|r| {
                    if r.a_store == parent_store && r.a_field == field {
                        Some(Wired::Join(r.clone(), true))
                    } else if r.b_store == parent_store && r.b_field.get() == Some(field) {
                        Some(Wired::Join(r.clone(), false))
                    } else {
                        None
                    }
                })
            })
    })
}

/// What the generated accessor for a `Many` field returns: the relation, addressed from one
/// parent. Reads are TRACKED through the parent's own field path — membership changes wake
/// exactly the readers of this parent's relation — and writes go through the children's
/// foreign keys, the single source of truth.
pub struct RelationRef<S: Copy + 'static, P: 'static, T: 'static> {
    field: day_model::Field<S, P, Many<T>>,
}

impl<S: Copy + 'static, P: 'static, T: 'static> Clone for RelationRef<S, P, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<S: Copy + 'static, P: 'static, T: 'static> Copy for RelationRef<S, P, T> {}

impl<S: day_model::Source<P>, P: 'static, T: 'static> RelationRef<S, P, T> {
    pub fn new(field: day_model::Field<S, P, Many<T>>) -> Self {
        RelationRef { field }
    }

    fn resolve(&self) -> Option<(Wired, u64)> {
        let mut parts = Vec::new();
        day_model::Source::components(self.field, &mut parts);
        if parts.len() < 2 {
            return None;
        }
        find_relation(parts[0], self.field.label()).map(|r| (r, parts[1]))
    }

    /// The children this parent holds, in relation order.
    fn members(&self) -> Vec<u64> {
        match self.resolve() {
            Some((Wired::ToOne(r), p)) => r.children_of(p),
            Some((Wired::Join(r, forward), p)) => r.members_of(p, forward),
            None => Vec::new(),
        }
    }

    /// The children's ids, in relation order — TRACKED: the caller re-runs when membership
    /// (or order) changes, and not when a child's other fields do.
    pub fn ids(&self) -> Vec<ModelId<T>> {
        self.field.with(|_| ());
        self.members()
            .into_iter()
            .map(ModelId::from_handle)
            .collect()
    }

    /// The count, tracked like [`RelationRef::ids`].
    pub fn count(&self) -> usize {
        self.field.with(|_| ());
        self.members().len()
    }

    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }

    /// Tracked membership test.
    pub fn contains(&self, id: impl Into<ModelId<T>>) -> bool {
        let h = id.into().handle();
        self.field.with(|_| ());
        self.members().contains(&h)
    }

    /// Take `child` into this relation — writing its foreign key (reparenting it away from
    /// any previous parent, whose readers wake too). On an ordered relation the child lands
    /// LAST, its order field written past the current end. Returns false when the relation
    /// is unwired or the child is gone.
    pub fn add(&self, child: impl Into<ModelId<T>>) -> bool {
        let h = child.into().handle();
        match self.resolve() {
            Some((Wired::ToOne(r), parent)) => {
                if !r.set_fk(h, Some(parent)) {
                    return false;
                }
                if r.ordered.is_some() {
                    let last = r
                        .children_of(parent)
                        .iter()
                        .filter(|c| **c != h)
                        .map(|c| r.read_order(*c))
                        .fold(f64::NEG_INFINITY, f64::max);
                    let next = if last.is_finite() { last + 1.0 } else { 1.0 };
                    (r.write_ord)(h, next);
                }
                true
            }
            Some((Wired::Join(r, forward), key)) => {
                // Whichever end asked, the stored pair is always (A, B).
                let (a, b) = if forward { (key, h) } else { (h, key) };
                let last = r
                    .members_of(a, true)
                    .iter()
                    .map(|c| r.position_of(a, *c))
                    .fold(f64::NEG_INFINITY, f64::max);
                let next = if last.is_finite() { last + 1.0 } else { 1.0 };
                r.link(a, b, if r.ordered { next } else { 0.0 })
            }
            None => false,
        }
    }

    /// Ordered relations: place `child` at `index` among the current children — normally ONE
    /// write of the child's order field (fractional keying), plus the adopt when it was not
    /// yet this parent's. When the gap between neighbors has bisected away, the siblings
    /// rebalance to whole numbers first — O(n), rare, and every write still takes the front
    /// door. Returns false on an unordered or unwired relation.
    pub fn insert_at(&self, child: impl Into<ModelId<T>>, index: usize) -> bool {
        let h = child.into().handle();
        let Some((wired, parent)) = self.resolve() else {
            return false;
        };
        // Read the neighbors' order values, place between them, and write ONE row — whichever
        // shape holds the order (the child's field, or the membership's position).
        let Placement {
            ordered,
            read_ord,
            write_ord,
            is_member,
        } = match &wired {
            Wired::ToOne(r) => {
                let (r1, r2) = (r.clone(), r.clone());
                Placement {
                    ordered: r.ordered.is_some(),
                    read_ord: Box::new(move |c| r1.read_order(c)),
                    write_ord: Box::new(move |c, v| (r2.write_ord)(c, v)),
                    is_member: r.index.borrow().parent_of.get(&h) == Some(&parent),
                }
            }
            Wired::Join(r, forward) => {
                let (r1, r2) = (r.clone(), r.clone());
                Placement {
                    // Order belongs to the A side: a membership's position places a B row
                    // within one A row, so the reverse view has no order to write.
                    ordered: r.ordered && *forward,
                    read_ord: Box::new(move |c| r1.position_of(parent, c)),
                    write_ord: Box::new(move |c, v| {
                        r2.store.write_field(
                            day_model::Key::Pair(parent, c).handle(),
                            "position",
                            v,
                        )
                    }),
                    is_member: r.members_of(parent, *forward).contains(&h),
                }
            }
        };
        if !ordered {
            return false;
        }
        if !is_member && !self.add(ModelId::<T>::from_handle(h)) {
            return false;
        }
        let siblings: Vec<u64> = match &wired {
            Wired::ToOne(r) => r.children_of(parent),
            Wired::Join(r, forward) => r.members_of(parent, *forward),
        }
        .into_iter()
        .filter(|c| *c != h)
        .collect();
        let index = index.min(siblings.len());
        let prev = index.checked_sub(1).and_then(|i| siblings.get(i).copied());
        let next = siblings.get(index).copied();
        let ord = match (prev.map(&read_ord), next.map(&read_ord)) {
            (None, None) => 1.0,
            (Some(a), None) => a + 1.0,
            (None, Some(b)) => b - 1.0,
            (Some(a), Some(b)) => {
                let mid = a + (b - a) / 2.0;
                if mid > a && mid < b {
                    mid
                } else {
                    // The gap is spent: rebalance the siblings to whole numbers, then the
                    // target slot's midpoint is roomy again.
                    for (i, c) in siblings.iter().enumerate() {
                        write_ord(*c, (i + 1) as f64);
                    }
                    index as f64 + 0.5
                }
            }
        };
        write_ord(h, ord)
    }

    /// Move an existing child to `index` — [`RelationRef::insert_at`], reading as intent.
    pub fn move_to(&self, child: impl Into<ModelId<T>>, index: usize) -> bool {
        self.insert_at(child, index)
    }

    /// Clear `child`'s reference — it leaves this relation and belongs to no parent. On a
    /// required (`One<…>`, non-`Option`) inverse the cleared reference is a constraint
    /// violation at flush: reparent or delete the child instead.
    pub fn remove(&self, child: impl Into<ModelId<T>>) -> bool {
        let h = child.into().handle();
        match self.resolve() {
            Some((Wired::ToOne(r), p)) => {
                if r.index.borrow().parent_of.get(&h) != Some(&p) {
                    return false; // not this parent's child
                }
                r.set_fk(h, None)
            }
            Some((Wired::Join(r, forward), key)) => {
                let (a, b) = if forward { (key, h) } else { (h, key) };
                r.unlink(a, b)
            }
            None => false,
        }
    }

    /// [`RelationRef::remove`] for every current child.
    pub fn clear(&self) {
        match self.resolve() {
            Some((Wired::ToOne(r), p)) => {
                for child in r.children_of(p) {
                    r.set_fk(child, None);
                }
            }
            Some((Wired::Join(r, forward), key)) => {
                for other in r.members_of(key, forward) {
                    let (a, b) = if forward { (key, other) } else { (other, key) };
                    r.unlink(a, b);
                }
            }
            None => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Container-side dispatch
// ---------------------------------------------------------------------------

impl ModelContainer {
    /// Route one announced change into relation maintenance. Called from the change sink,
    /// AFTER dirty-marking and query dispatch; the writes it makes (cascades, nullifies)
    /// nest through the same pipeline.
    pub(crate) fn relations_on_change(&self, change: &day_model::Change) {
        let Some(&store) = change.components.first() else {
            return;
        };
        if self.inner.relations.borrow().is_empty() && self.inner.joins.borrow().is_empty() {
            return;
        }
        let rels: Vec<Rc<ToOneRel>> = self.inner.relations.borrow().iter().cloned().collect();
        for rel in rels {
            if rel.child_store == store {
                match (change.components.get(1).copied(), change.op) {
                    (Some(k), _) if k == day_model::STRUCTURE => {}
                    (Some(child), Op::Set) if change.components.len() >= 3 => {
                        if change.label == rel.fk_field {
                            rel.reparent(child);
                        } else if Some(change.label) == rel.ordered {
                            rel.order_changed(child);
                        }
                    }
                    (Some(child), Op::Insert) if change.components.len() == 2 => {
                        rel.child_added(child);
                    }
                    (Some(child), Op::Delete) if change.components.len() == 2 => {
                        rel.child_removed(child);
                    }
                    _ => {}
                }
            }
            if rel.parent_store == store
                && change.components.len() == 2
                && change.op == Op::Delete
                && change.components[1] != day_model::STRUCTURE
            {
                rel.parent_deleted(self, change.components[1]);
            }
        }

        let joins: Vec<Rc<JoinRel>> = self.inner.joins.borrow().iter().cloned().collect();
        for rel in joins {
            let Some(&key) = change.components.get(1) else {
                continue;
            };
            if key == day_model::STRUCTURE {
                continue;
            }
            if rel.join_store == store {
                match change.op {
                    Op::Insert if change.components.len() == 2 => rel.row_added(key),
                    Op::Delete if change.components.len() == 2 => rel.row_removed(key),
                    Op::Set if change.components.len() >= 3 && change.label == "position" => {
                        rel.row_moved(key)
                    }
                    _ => {}
                }
            }
            // Either side's deletion drops its memberships (and, under cascade from the
            // parent side, the children no other parent still holds).
            if change.op == Op::Delete && change.components.len() == 2 {
                if rel.a_store == store {
                    rel.side_deleted(true, key);
                } else if rel.b_store == store {
                    rel.side_deleted(false, key);
                }
            }
        }
    }

    /// The store for `M` when it is in this container's schema — the non-panicking
    /// [`ModelContainer::store`].
    pub fn try_store<M: Model>(&self) -> Option<Store<Keyed<M>>> {
        self.inner
            .stores
            .borrow()
            .get(&std::any::TypeId::of::<M>())
            .and_then(|a| a.downcast_ref::<Store<Keyed<M>>>())
            .copied()
    }

    /// Delete one row, honoring deny rules: a parent still referenced through a
    /// `DeleteRule::Deny` relation is refused with the relation named. Cascade and nullify
    /// need no checked door — a plain `restructure` delete triggers them — but deny cannot
    /// refuse after the row is gone, so this is deny's contract.
    pub fn delete<M: Model>(&self, id: impl Into<ModelId<M>>) -> Result<(), DbError> {
        let h = id.into().handle();
        let store = self.store::<M>();
        let rels: Vec<Rc<ToOneRel>> = self.inner.relations.borrow().iter().cloned().collect();
        for r in rels
            .iter()
            .filter(|r| r.parent_store == store.store_id() && r.delete == DeleteRule::Deny)
        {
            let n = r.children_of(h).len();
            if n > 0 {
                return Err(DbError::new(
                    DbErrorKind::Deny,
                    format!(
                        "`{}` row still referenced by {n} `{}` row(s) through `{}` — \
                         delete or reparent them first",
                        M::TABLE,
                        r.fk_field,
                        r.parent_field
                    ),
                ));
            }
        }
        let joins: Vec<Rc<JoinRel>> = self.inner.joins.borrow().iter().cloned().collect();
        for r in joins.iter().filter(|r| {
            (r.a_store == store.store_id() && r.a_delete == DeleteRule::Deny)
                || (r.b_store == store.store_id() && r.b_delete.get() == DeleteRule::Deny)
        }) {
            let forward = r.a_store == store.store_id();
            let n = r.members_of(h, forward).len();
            if n > 0 {
                return Err(DbError::new(
                    DbErrorKind::Deny,
                    format!(
                        "`{}` row still holds {n} membership(s) through `{}` — unlink them \
                         first",
                        M::TABLE,
                        if forward {
                            r.a_field
                        } else {
                            r.b_field.get().unwrap_or(r.a_field)
                        }
                    ),
                ));
            }
        }
        store.restructure("delete", Op::Delete, h, |k| {
            k.remove(h);
        });
        Ok(())
    }

    /// Create (once) a generated join table: the pair as the primary key, a foreign key per
    /// side with its own delete rule, and an index on the reverse side so the child→parents
    /// direction is as cheap as the forward one.
    pub(crate) fn ensure_join_table<P: Model, C: Model>(
        &self,
        table: &str,
        parent_col: &str,
        child_col: &str,
        ordered: bool,
        delete: DeleteRule,
    ) -> Result<(), DbError> {
        // The engine's own rule for the PARENT side matches the declaration; the child side
        // always clears its memberships, because a membership row without its child is
        // garbage in any reading of the relation.
        let position = if ordered {
            ", position REAL NOT NULL"
        } else {
            ""
        };
        let body = format!(
            "CREATE TABLE IF NOT EXISTS {table} (\
             {parent_col} {} NOT NULL REFERENCES {}({}) ON DELETE {} DEFERRABLE INITIALLY DEFERRED, \
             {child_col} {} NOT NULL REFERENCES {}({}) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED\
             {position}, PRIMARY KEY ({parent_col}, {child_col}))",
            P::KEY_SQL.ddl(),
            P::TABLE,
            P::KEY,
            if delete == DeleteRule::Deny {
                "RESTRICT"
            } else {
                "CASCADE"
            },
            C::KEY_SQL.ddl(),
            C::TABLE,
            C::KEY,
        );
        if self.conn().execute(&format!("{body} STRICT"), &[]).is_err() {
            self.conn().execute(&body, &[])?;
        }
        self.conn().execute(
            &format!(
                "CREATE INDEX IF NOT EXISTS day_idx_{table}_{child_col} ON {table}({child_col})"
            ),
            &[],
        )?;
        Ok(())
    }

    /// Give a join store the same table hooks every model store has — so one fold, one
    /// external merge and one undo history cover memberships too.
    pub(crate) fn register_join_hooks(
        &self,
        table: &'static str,
        parent_col: &'static str,
        child_col: &'static str,
        ordered: bool,
        store: Store<Keyed<JoinRow>>,
    ) {
        let mut columns = vec![parent_col.to_string(), child_col.to_string()];
        let mut fields = vec![parent_col.to_string(), child_col.to_string()];
        if ordered {
            columns.push("position".into());
            fields.push("position".into());
        }
        let to_row = move |r: &JoinRow| {
            let mut v = vec![key_param(r.parent), key_param(r.child)];
            if ordered {
                v.push(Value::Real(r.position));
            }
            v
        };
        let hooks = crate::TableHooks {
            table,
            key_cols: vec![parent_col.to_string(), child_col.to_string()],
            key_where: Rc::new(move |h| match day_model::Key::of_handle(h) {
                Some(day_model::Key::Pair(p, c)) => (
                    format!("{parent_col} = ? AND {child_col} = ?"),
                    vec![key_param(p), key_param(c)],
                ),
                _ => (format!("{parent_col} IS NULL"), Vec::new()),
            }),
            key_from_row: Rc::new(|row| {
                match (value_to_handle(&row.get(0)), value_to_handle(&row.get(1))) {
                    (Some(p), Some(c)) => Some(day_model::Key::Pair(p, c).handle()),
                    _ => None,
                }
            }),
            columns,
            fields,
            row_for: Rc::new(move |h| store.with_untracked(|k| k.get(h).map(&to_row))),
            all_rows: Rc::new(move || {
                store.with_untracked(|k| {
                    k.items()
                        .iter()
                        .map(|r| (day_model::Identified::handle(r), to_row(r)))
                        .collect()
                })
            }),
            reload: Rc::new(move |raw| {
                let mut rows = Vec::with_capacity(raw.len());
                for r in raw {
                    match (
                        value_to_handle(&Row::get(&r, 0)),
                        value_to_handle(&Row::get(&r, 1)),
                    ) {
                        (Some(p), Some(c)) => rows.push(JoinRow {
                            parent: p,
                            child: c,
                            position: if ordered {
                                Row::get(&r, 2).as_real().unwrap_or(0.0)
                            } else {
                                0.0
                            },
                        }),
                        _ => {
                            return Err(DbError::new(
                                DbErrorKind::Decode,
                                format!("`{table}` holds a membership whose ids will not read"),
                            ));
                        }
                    }
                }
                let fresh = Keyed::new(rows);
                store.update("rescan", move |k| *k = fresh);
                Ok(())
            }),
            merge: Rc::new(move |raw| {
                // Memberships are a set: the diff is which pairs arrived and which left.
                let mut fresh: Vec<JoinRow> = Vec::with_capacity(raw.len());
                for r in raw {
                    match (
                        value_to_handle(&Row::get(&r, 0)),
                        value_to_handle(&Row::get(&r, 1)),
                    ) {
                        (Some(p), Some(c)) => fresh.push(JoinRow {
                            parent: p,
                            child: c,
                            position: if ordered {
                                Row::get(&r, 2).as_real().unwrap_or(0.0)
                            } else {
                                0.0
                            },
                        }),
                        _ => {
                            return Err(DbError::new(
                                DbErrorKind::Decode,
                                format!("`{table}` holds a membership whose ids will not read"),
                            ));
                        }
                    }
                }
                let existing: Vec<u64> = store.with_untracked(|k| k.keys().to_vec());
                let arriving: std::collections::HashSet<u64> =
                    fresh.iter().map(day_model::Identified::handle).collect();
                let mut changed = false;
                day_model::with_author(ModelContainer::EXTERNAL_AUTHOR, || {
                    for row in fresh {
                        let h = day_model::Identified::handle(&row);
                        match store.with_untracked(|k| k.get(h).cloned()) {
                            None => {
                                changed = true;
                                store.restructure("external", Op::Insert, h, |k| k.push(row));
                            }
                            Some(old) if ordered && old.position != row.position => {
                                changed = true;
                                store.write_field(h, "position", row.position);
                            }
                            Some(_) => {}
                        }
                    }
                    for gone in existing.into_iter().filter(|h| !arriving.contains(h)) {
                        changed = true;
                        store.restructure("external", Op::Delete, gone, |k| {
                            k.remove(gone);
                        });
                    }
                });
                Ok(changed)
            }),
            watch_undo: Rc::new(move |stack: &day_model::UndoStack| stack.watch(store)),
        };
        self.inner
            .tables
            .borrow_mut()
            .insert(store.store_id(), hooks);
    }
}
