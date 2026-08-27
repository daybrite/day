// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Relations (docs/persistence.md): `One<M>` foreign keys, `Many<M>` maintained inverses,
//! delete rules, and the machinery that keeps them true.
//!
//! There is ONE source of truth per to-one relation — the foreign-key column on the child —
//! and the parent's `Many` side is a VIEW over it, answered from the engine's own foreign-key
//! index and memoized per parent. Nothing is loaded at open: the first read of one parent's
//! children is one indexed `SELECT`, remembered until a membership write invalidates it.
//! Write `lodging.trip()` and the trip's `lodging()` read wakes; call
//! `trip.lodging().add(id)` and it writes the lodging's foreign key through the front door,
//! so the change announces, captures for undo, folds to one `UPDATE`, and marks any live
//! query watching either table stale.
//!
//! Reads made MID-TURN see the truth: the memo answers from the last flush, overlaid with
//! this turn's unflushed dirty rows — a child reparented a millisecond ago is under its new
//! parent before any SQL runs.
//!
//! Delete rules run where the delete announces: a parent's removal cascades (recursively —
//! the nested deletes take the same pipeline, so undoing the cascade is one unit that
//! restores the whole subtree), nullifies (`Option<One<M>>` references clear), or denies
//! (the checked door [`crate::ModelContainer::delete`] refuses while children remain). The
//! generated DDL carries the matching `REFERENCES … ON DELETE …` clause, `DEFERRABLE
//! INITIALLY DEFERRED` so within-transaction statement order never trips it — which also
//! keeps another process honest about the same rules, and is what deletes the NON-resident
//! rows a cascade reaches: the pipeline walks every child (so queries, memos and undo hear),
//! and the engine's own clause is the backstop that makes the file agree.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::{Rc, Weak};

use day_model::{ApplyField, Keyed, ModelId, Op, Store, announce, field_id};

use crate::{
    ColumnValue, ContainerInner, DbError, DbErrorKind, DirtyRow, Model, ModelContainer, Row,
    SqlType, Value, key_param, value_to_handle,
};

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

/// Predicates over a to-one reference column: the identity and presence tests, which need
/// nothing but the foreign-key column already stored here.
impl<M: Model> crate::Col<One<M>> {
    /// The reference points at exactly this row.
    pub fn is(self, id: impl Into<ModelId<M>>) -> crate::Pred {
        self.eq(One::<M>::from_handle(id.into().handle()))
    }

    /// The reference points at one of these rows — the set form, and what a query feeding
    /// another query compiles to. (Named apart from the generic [`crate::Col::is_in`], which
    /// takes `One` values; this one takes ids, which is what a caller actually holds.)
    pub fn is_one_of(self, ids: impl IntoIterator<Item = impl Into<ModelId<M>>>) -> crate::Pred {
        self.is_in(
            ids.into_iter()
                .map(|i| One::<M>::from_handle(i.into().handle()))
                .collect::<Vec<_>>(),
        )
    }

    /// The reference was never set, or a nullify rule cleared it.
    pub fn is_unset(self) -> crate::Pred {
        self.is_null()
    }

    /// The reference points somewhere.
    pub fn is_set(self) -> crate::Pred {
        self.is_not_null()
    }
}

/// Traversing a to-one reference in a predicate: "lodgings whose trip is done". A to-one is
/// a to-many of at most one, so the quantifier vocabulary is the same one the `Many` side
/// uses — `any` reads as "its target matches, and there is one".
impl<M: Model> crate::Col<One<M>> {
    pub fn any(self, inner: crate::Pred) -> crate::Pred {
        self.quantified(crate::Quant::Any, inner)
    }
    pub fn none(self, inner: crate::Pred) -> crate::Pred {
        self.quantified(crate::Quant::None, inner)
    }

    fn quantified(self, quant: crate::Quant, inner: crate::Pred) -> crate::Pred {
        crate::Pred::Related {
            owner: self.owner,
            field: self.field,
            target: <M as Model>::TABLE,
            quant,
            inner: Box::new(inner),
        }
    }
}

/// The same, over a nullable reference.
impl<M: Model> crate::Col<Option<One<M>>> {
    pub fn any(self, inner: crate::Pred) -> crate::Pred {
        self.quantified(crate::Quant::Any, inner)
    }
    pub fn none(self, inner: crate::Pred) -> crate::Pred {
        self.quantified(crate::Quant::None, inner)
    }

    fn quantified(self, quant: crate::Quant, inner: crate::Pred) -> crate::Pred {
        crate::Pred::Related {
            owner: self.owner,
            field: self.field,
            target: <M as Model>::TABLE,
            quant,
            inner: Box::new(inner),
        }
    }
}

/// The same tests over a NULLABLE reference (`Option<One<M>>`), which is what a relation with
/// a nullify delete rule requires the child to hold.
impl<M: Model> crate::Col<Option<One<M>>> {
    pub fn is(self, id: impl Into<ModelId<M>>) -> crate::Pred {
        self.eq(Some(One::<M>::from_handle(id.into().handle())))
    }

    pub fn is_one_of(self, ids: impl IntoIterator<Item = impl Into<ModelId<M>>>) -> crate::Pred {
        self.is_in(
            ids.into_iter()
                .map(|i| Some(One::<M>::from_handle(i.into().handle())))
                .collect::<Vec<_>>(),
        )
    }

    /// The reference was never set, or a nullify rule cleared it.
    pub fn is_unset(self) -> crate::Pred {
        self.is_null()
    }

    /// The reference points somewhere.
    pub fn is_set(self) -> crate::Pred {
        self.is_not_null()
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
/// (`#[model(relation(delete = "…"))]`); the default is `Nullify` (also SwiftData's default
/// for `@Relationship`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeleteRule {
    /// Children survive; their references clear. Needs `Option<One<M>>` on the child —
    /// a required reference cannot hold "nothing", and wiring refuses the combination.
    Nullify,
    /// Children delete with the parent, recursively, through the normal pipeline — undoable
    /// as one unit, heard by live queries.
    Cascade,
    /// The delete is refused while children remain — through
    /// [`crate::ModelContainer::delete`], the checked door; a raw `restructure` bypasses the
    /// check and the deferred SQL `RESTRICT` refuses at flush instead.
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

/// One wired to-one/to-many relation. The typed closures are monomorphized over both model
/// types by [`wire_to_many`], so every write goes through the front door with full type
/// knowledge; the SQL half (table and column names) is what lazy reads and the query
/// compiler drive.
pub(crate) struct ToOneRel {
    pub(crate) container: Weak<ContainerInner>,
    pub(crate) parent_store: u64,
    pub(crate) parent_table: &'static str,
    pub(crate) parent_field: &'static str,
    pub(crate) parent_key_col: &'static str,
    pub(crate) child_store: u64,
    pub(crate) child_table: &'static str,
    pub(crate) child_key_col: &'static str,
    /// The foreign key as the child's FIELD (the change log's language) and COLUMN (SQL's).
    pub(crate) fk_field: &'static str,
    pub(crate) fk_col: &'static str,
    pub(crate) delete: DeleteRule,
    pub(crate) ordered: Option<&'static str>,
    pub(crate) ord_col: Option<&'static str>,
    read_fk: Box<dyn Fn(u64) -> Option<u64>>,
    write_fk: Box<dyn Fn(u64, Option<u64>) -> bool>,
    delete_child: Box<dyn Fn(u64)>,
    /// Bring children into the cache (before their fields are written, or for undo capture).
    materialize: Materialize,
    read_ord_cached: Box<dyn Fn(u64) -> Option<f64>>,
    pub(crate) write_ord: Box<dyn Fn(u64, f64) -> bool>,
    /// parent → children (relation order), filled per parent on first read.
    memo: RefCell<HashMap<u64, Vec<u64>>>,
    /// child → memoized parent, maintained only for children of memoized parents — the O(1)
    /// walk-back a foreign-key rewrite needs.
    memo_parent: RefCell<HashMap<u64, u64>>,
}

impl ToOneRel {
    fn with_container<R>(&self, f: impl FnOnce(&ModelContainer) -> R) -> Option<R> {
        self.container.upgrade().map(|inner| {
            let c = ModelContainer { inner };
            f(&c)
        })
    }

    /// The parent's children, in relation order — the memo, or one indexed `SELECT`, overlaid
    /// with this turn's unflushed dirty rows so mid-turn reads see the truth.
    pub(crate) fn children_of(&self, parent: u64) -> Vec<u64> {
        if let Some(hit) = self.memo.borrow().get(&parent) {
            return hit.clone();
        }
        let Some(mut ids) = self.with_container(|c| {
            let order = self.ord_col.unwrap_or(self.child_key_col);
            c.select_id_column(
                &format!(
                    "SELECT {k} FROM {t} WHERE {fk} = ? ORDER BY {order}, {k}",
                    k = self.child_key_col,
                    t = self.child_table,
                    fk = self.fk_col,
                ),
                &[key_param(parent)],
            )
        }) else {
            return Vec::new();
        };
        self.overlay_children(parent, &mut ids);
        let mut memo = self.memo.borrow_mut();
        let mut back = self.memo_parent.borrow_mut();
        for id in &ids {
            back.insert(*id, parent);
        }
        memo.insert(parent, ids.clone());
        ids
    }

    /// Correct a freshly-selected child list against this turn's unflushed edits: dirty
    /// resident children join or leave by their CURRENT foreign key, and dirty deletes leave.
    fn overlay_children(&self, parent: u64, ids: &mut Vec<u64>) {
        let Some(dirty) = self.with_container(|c| c.dirty_rows_of(self.child_store)) else {
            return;
        };
        for (child, state) in dirty {
            let is_mine = state != DirtyRow::Delete && (self.read_fk)(child) == Some(parent);
            let at = ids.iter().position(|c| *c == child);
            match (is_mine, at) {
                (false, Some(i)) => {
                    ids.remove(i);
                }
                (true, None) => {
                    // Place by the relation's order (the child is resident: it is dirty).
                    let ord = (self.read_ord_cached)(child).unwrap_or(f64::INFINITY);
                    let at = if self.ordered.is_some() {
                        ids.iter()
                            .position(|c| (self.read_ord_cached)(*c).unwrap_or(0.0) > ord)
                            .unwrap_or(ids.len())
                    } else {
                        ids.partition_point(|c| *c < child)
                    };
                    ids.insert(at, child);
                }
                _ => {}
            }
        }
    }

    /// The parent this child belongs to — the memo when it knows, the child's own cached
    /// foreign key, or one point `SELECT`.
    pub(crate) fn parent_of(&self, child: u64) -> Option<u64> {
        if let Some(p) = self.memo_parent.borrow().get(&child) {
            return Some(*p);
        }
        if let Some(p) = (self.read_fk)(child) {
            return Some(p);
        }
        self.with_container(|c| {
            c.select_id_column(
                &format!(
                    "SELECT {fk} FROM {t} WHERE {k} = ?",
                    fk = self.fk_col,
                    t = self.child_table,
                    k = self.child_key_col,
                ),
                &[key_param(child)],
            )
            .into_iter()
            .next()
        })
        .flatten()
    }

    /// One child's order value — cache first (which covers every unflushed write), the file
    /// otherwise.
    pub(crate) fn read_order(&self, child: u64) -> f64 {
        if let Some(v) = (self.read_ord_cached)(child) {
            return v;
        }
        let Some(col) = self.ord_col else { return 0.0 };
        self.with_container(|c| {
            c.select_real(
                &format!(
                    "SELECT {col} FROM {t} WHERE {k} = ?",
                    t = self.child_table,
                    k = self.child_key_col,
                ),
                &[key_param(child)],
            )
        })
        .flatten()
        .unwrap_or(0.0)
    }

    pub(crate) fn set_fk(&self, child: u64, parent: Option<u64>) -> bool {
        // Writes need the row resident; the announcement then routes back through
        // `reparent`, which is where the memos and both parents' announcements happen —
        // one path for every door.
        (self.materialize)(&[child]);
        (self.write_fk)(child, parent)
    }

    fn announce_parent(&self, parent: u64) {
        announce(
            &[self.parent_store, parent, field_id(self.parent_field)],
            self.parent_field,
        );
    }

    fn forget(&self, parent: u64) {
        if let Some(children) = self.memo.borrow_mut().remove(&parent) {
            let mut back = self.memo_parent.borrow_mut();
            for c in children {
                back.remove(&c);
            }
        }
    }

    pub(crate) fn invalidate_all(&self) {
        self.memo.borrow_mut().clear();
        self.memo_parent.borrow_mut().clear();
    }

    /// A child arrived (an insert, an undo re-insert, an external merge).
    pub(crate) fn child_added(&self, child: u64) {
        if let Some(p) = (self.read_fk)(child) {
            self.forget(p);
            self.announce_parent(p);
        }
    }

    /// A child left. The memo knows its parent when the parent was ever read; otherwise the
    /// file still holds the row until the flush, so one point `SELECT` answers.
    pub(crate) fn child_removed(&self, child: u64) {
        let parent = self
            .memo_parent
            .borrow()
            .get(&child)
            .copied()
            .or_else(|| self.parent_of(child));
        if let Some(p) = parent {
            self.forget(p);
            self.announce_parent(p);
        }
    }

    /// The child's foreign key was rewritten: both parents' views move. The NEW parent is the
    /// child's own field; the OLD one is the memo when a reader ever established it, the
    /// FILE otherwise (the un-flushed value is exactly the pre-write one). A reader whose
    /// memo was current re-establishes it on its own re-read, so repeated reparents in one
    /// turn stay coherent for anything actually watching.
    pub(crate) fn reparent(&self, child: u64) {
        let new = (self.read_fk)(child);
        let old = self
            .memo_parent
            .borrow()
            .get(&child)
            .copied()
            .or_else(|| self.former_parent_file(child));
        if let Some(p) = old {
            self.forget(p);
        }
        if let Some(p) = new
            && Some(p) != old
        {
            self.forget(p);
        }
        self.announce_parent_set(old, new);
    }

    /// Announce each DISTINCT parent in the pair.
    fn announce_parent_set(&self, a: Option<u64>, b: Option<u64>) {
        if let Some(p) = a {
            self.announce_parent(p);
        }
        if let Some(p) = b
            && Some(p) != a
        {
            self.announce_parent(p);
        }
    }

    /// The parent the FILE records for this child — the pre-flush truth, bypassing both the
    /// memo and the (already rewritten) cached row.
    fn former_parent_file(&self, child: u64) -> Option<u64> {
        self.with_container(|c| {
            c.select_id_column(
                &format!(
                    "SELECT {fk} FROM {t} WHERE {k} = ?",
                    fk = self.fk_col,
                    t = self.child_table,
                    k = self.child_key_col,
                ),
                &[key_param(child)],
            )
            .into_iter()
            .next()
        })
        .flatten()
    }

    /// The child's order field moved: its parent's view reorders.
    pub(crate) fn order_changed(&self, child: u64) {
        if let Some(p) = self.parent_of(child) {
            self.forget(p);
            self.announce_parent(p);
        }
    }

    pub(crate) fn parent_deleted(&self, container: &ModelContainer, parent: u64) {
        let children = self.children_of(parent);
        self.forget(parent);
        if children.is_empty() {
            return;
        }
        match self.delete {
            DeleteRule::Cascade => {
                // Each nested delete takes the whole pipeline — announce, undo capture,
                // dirty, query staleness — and, for a self-referential tree, recurses
                // through this same method one level down. When an undo stack stands by,
                // the rows materialize first so their deletes carry what inversion needs;
                // without one the deletes are value-free and the fold batches them.
                if day_model::record_capture_active() {
                    (self.materialize)(&children);
                }
                for child in children {
                    (self.delete_child)(child);
                }
            }
            DeleteRule::Nullify => {
                (self.materialize)(&children);
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
    let Some(parent_store) = container.try_cache::<P>() else {
        reg.fail(format!(
            "relation `{}.{field}` wired before `{}` attached",
            P::TABLE,
            P::TABLE
        ));
        return;
    };
    let Some(child_store) = container.try_cache::<C>() else {
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
    let mut ord_col = None;
    if let Some(ord) = ordered {
        match C::COLUMNS
            .iter()
            .find(|c| c.field == ord && c.sql == SqlType::Real)
        {
            Some(c) => ord_col = Some(c.name),
            None => {
                reg.fail(format!(
                    "relation `{}.{field}` is ordered by `{ord}`, which is not a REAL (`f64`) \
                     field of `{}` — the order column is a real, visible field of the child",
                    P::TABLE,
                    C::TABLE
                ));
                return;
            }
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
    let mat_container = container.clone();
    let materialize = Box::new(move |keys: &[u64]| {
        let _ = mat_container.ensure_resident::<C>(keys);
    });
    let read_ord_cached: Box<dyn Fn(u64) -> Option<f64>> = match ordered {
        Some(ord) => Box::new(move |h| {
            child_store.with_untracked(|k| {
                k.get(h)
                    .and_then(|c| ApplyField::read_field(c, ord))
                    .and_then(|v| v.downcast_ref::<f64>().copied())
            })
        }),
        None => Box::new(|_| None),
    };
    let write_ord: Box<dyn Fn(u64, f64) -> bool> = match ordered {
        Some(ord) => Box::new(move |h, v| child_store.write_field(h, ord, v)),
        None => Box::new(|_, _| false),
    };

    let rel = Rc::new(ToOneRel {
        container: Rc::downgrade(&container.inner),
        parent_store: parent_store.store_id(),
        parent_table: P::TABLE,
        parent_field: field,
        parent_key_col: P::KEY,
        child_store: child_store.store_id(),
        child_table: C::TABLE,
        child_key_col: C::KEY,
        fk_field: inverse,
        fk_col: col.name,
        delete,
        ordered,
        ord_col,
        read_fk,
        write_fk,
        delete_child,
        materialize,
        read_ord_cached,
        write_ord,
        memo: RefCell::new(HashMap::new()),
        memo_parent: RefCell::new(HashMap::new()),
    });

    container.inner.relations.borrow_mut().push(rel);
}

// ---------------------------------------------------------------------------
// Many-to-many: the join table
// ---------------------------------------------------------------------------

/// One membership row of a generated join table. Its key is the PAIR — that is what makes a
/// membership addressable, undoable and mergeable by the same machinery every other row uses,
/// with no second vocabulary. The store over these rows is a CACHE like every other: only
/// memberships this process touched are resident.
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
/// Membership lives in the join table; both directions answer from its indexes, memoized per
/// row. When both models declare the relation over the same `join = "…"` table, the second
/// declaration attaches as this relation's B side rather than opening a second store — two
/// stores over one table would double every write and disagree about column order.
pub(crate) struct JoinRel {
    pub(crate) container: Weak<ContainerInner>,
    pub(crate) join_table: &'static str,
    pub(crate) parent_col: &'static str,
    pub(crate) child_col: &'static str,
    /// The side that wired first: its rows are the join row's `parent`.
    pub(crate) a_store: u64,
    pub(crate) a_table: &'static str,
    pub(crate) a_field: &'static str,
    pub(crate) a_key_col: &'static str,
    pub(crate) a_delete: DeleteRule,
    /// The other side; `b_field` is set only if that model declared the relation too —
    /// through a `Cell`, because the second declaration fills it in during wiring while the
    /// first declaration's `Rc` is already held.
    pub(crate) b_store: u64,
    pub(crate) b_table: &'static str,
    pub(crate) b_key_col: &'static str,
    pub(crate) b_field: Cell<Option<&'static str>>,
    pub(crate) b_delete: Cell<DeleteRule>,
    pub(crate) join_store: u64,
    /// Order is the A side's: a membership's position places a B row within one A row.
    pub(crate) ordered: bool,
    store: Store<Keyed<JoinRow>>,
    delete_a: Box<dyn Fn(u64)>,
    delete_b: Box<dyn Fn(u64)>,
    /// A-side row → its members, and B-side row → its holders, filled on first read.
    memo_fwd: RefCell<HashMap<u64, Vec<u64>>>,
    memo_rev: RefCell<HashMap<u64, Vec<u64>>>,
}

impl JoinRel {
    fn with_container<R>(&self, f: impl FnOnce(&ModelContainer) -> R) -> Option<R> {
        self.container.upgrade().map(|inner| {
            let c = ModelContainer { inner };
            f(&c)
        })
    }

    /// The members `key` holds, read from whichever side `forward` names: A→B forward, B→A
    /// reverse. One indexed `SELECT` on first read, memoized, overlaid with this turn's
    /// unflushed links and unlinks.
    pub(crate) fn members_of(&self, key: u64, forward: bool) -> Vec<u64> {
        {
            let memo = if forward {
                &self.memo_fwd
            } else {
                &self.memo_rev
            };
            if let Some(hit) = memo.borrow().get(&key) {
                return hit.clone();
            }
        }
        let (own_col, other_col) = if forward {
            (self.parent_col, self.child_col)
        } else {
            (self.child_col, self.parent_col)
        };
        let order = if self.ordered && forward {
            format!("position, {other_col}")
        } else {
            other_col.to_string()
        };
        let Some(mut ids) = self.with_container(|c| {
            c.select_id_column(
                &format!(
                    "SELECT {other_col} FROM {t} WHERE {own_col} = ? ORDER BY {order}",
                    t = self.join_table,
                ),
                &[key_param(key)],
            )
        }) else {
            return Vec::new();
        };
        self.overlay_members(key, forward, &mut ids);
        let memo = if forward {
            &self.memo_fwd
        } else {
            &self.memo_rev
        };
        memo.borrow_mut().insert(key, ids.clone());
        ids
    }

    /// Correct a freshly-selected member list against this turn's unflushed link/unlink rows.
    fn overlay_members(&self, key: u64, forward: bool, ids: &mut Vec<u64>) {
        let Some(dirty) = self.with_container(|c| c.dirty_rows_of(self.join_store)) else {
            return;
        };
        for (handle, state) in dirty {
            let Some(day_model::Key::Pair(p, c)) = day_model::Key::of_handle(handle) else {
                continue;
            };
            let (own, other) = if forward { (p, c) } else { (c, p) };
            if own != key {
                continue;
            }
            let at = ids.iter().position(|m| *m == other);
            match (state != DirtyRow::Delete, at) {
                (false, Some(i)) => {
                    ids.remove(i);
                }
                (true, None) => {
                    if self.ordered && forward {
                        let pos = self.position_of(p, c);
                        let at = ids
                            .iter()
                            .position(|m| self.position_of(key, *m) > pos)
                            .unwrap_or(ids.len());
                        ids.insert(at, other);
                    } else {
                        let at = ids.partition_point(|m| *m < other);
                        ids.insert(at, other);
                    }
                }
                _ => {}
            }
        }
    }

    /// The rows on the OTHER side that hold `key` — the join's back-resolution.
    pub(crate) fn holders_of(&self, key: u64, forward: bool) -> Vec<u64> {
        // Looking back from a B row means asking the reverse direction, and vice versa.
        self.members_of(key, !forward)
    }

    pub(crate) fn position_of(&self, parent: u64, child: u64) -> f64 {
        let cached = self.store.with_untracked(|k| {
            k.get(day_model::Key::Pair(parent, child).handle())
                .map(|r| r.position)
        });
        if let Some(p) = cached {
            return p;
        }
        if !self.ordered {
            return 0.0;
        }
        self.with_container(|c| {
            c.select_real(
                &format!(
                    "SELECT position FROM {t} WHERE {pc} = ? AND {cc} = ?",
                    t = self.join_table,
                    pc = self.parent_col,
                    cc = self.child_col,
                ),
                &[key_param(parent), key_param(child)],
            )
        })
        .flatten()
        .unwrap_or(0.0)
    }

    /// Whether the pair is currently a membership — the cache, the dirty set, or the file.
    fn is_member(&self, parent: u64, child: u64) -> bool {
        let handle = day_model::Key::Pair(parent, child).handle();
        if let Some(state) = self.with_container(|c| c.dirty_state_of(self.join_store, handle))
            && let Some(state) = state
        {
            return state != DirtyRow::Delete;
        }
        if self.store.with_untracked(|k| k.get(handle).is_some()) {
            return true;
        }
        self.with_container(|c| {
            !c.select_id_column(
                &format!(
                    "SELECT 1 FROM {t} WHERE {pc} = ? AND {cc} = ?",
                    t = self.join_table,
                    pc = self.parent_col,
                    cc = self.child_col,
                ),
                &[key_param(parent), key_param(child)],
            )
            .is_empty()
        })
        .unwrap_or(false)
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

    fn forget_pair(&self, parent: u64, child: u64) {
        self.memo_fwd.borrow_mut().remove(&parent);
        self.memo_rev.borrow_mut().remove(&child);
    }

    pub(crate) fn invalidate_all(&self) {
        self.memo_fwd.borrow_mut().clear();
        self.memo_rev.borrow_mut().clear();
    }

    /// A join row arrived (an `add`, an undo's re-insert, an external merge).
    pub(crate) fn row_added(&self, handle: u64) {
        if let Some(day_model::Key::Pair(p, c)) = day_model::Key::of_handle(handle) {
            self.forget_pair(p, c);
            self.announce_pair(p, c);
        }
    }

    pub(crate) fn row_removed(&self, handle: u64) {
        if let Some(day_model::Key::Pair(p, c)) = day_model::Key::of_handle(handle) {
            self.forget_pair(p, c);
            self.announce_pair(p, c);
        }
    }

    /// A membership's position moved: reposition and wake both sides.
    pub(crate) fn row_moved(&self, handle: u64) {
        if let Some(day_model::Key::Pair(p, c)) = day_model::Key::of_handle(handle) {
            self.forget_pair(p, c);
            self.announce_pair(p, c);
        }
    }

    /// Either side's row was deleted: its memberships go with it, and under that side's own
    /// cascade rule so do the rows across the join that nobody else still holds.
    pub(crate) fn side_deleted(&self, is_a: bool, key: u64) {
        let others = self.members_of(key, is_a);
        if others.is_empty() {
            return;
        }
        let capture = day_model::record_capture_active();
        for other in &others {
            let (p, c) = if is_a { (key, *other) } else { (*other, key) };
            let handle = day_model::Key::Pair(p, c).handle();
            if capture && self.store.with_untracked(|k| k.get(handle).is_none()) {
                // Materialize the membership so its delete carries what undo inversion
                // needs — the pair is known; only an ordered position needs the file.
                let position = self.position_of(p, c);
                self.store.populate(vec![JoinRow {
                    parent: p,
                    child: c,
                    position,
                }]);
            }
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
            // (`forward` here is the DELETED side's perspective: the counterpart's holders
            // are read from the opposite index.)
            for other in others {
                let holders = self.holders_of(other, is_a);
                let still_held = holders.iter().any(|h| *h != key);
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
        if self.is_member(parent, child) {
            return false; // already a member — membership is a set
        }
        let handle = day_model::Key::Pair(parent, child).handle();
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
        if !self.is_member(parent, child) {
            return false;
        }
        let handle = day_model::Key::Pair(parent, child).handle();
        if day_model::record_capture_active()
            && self.store.with_untracked(|k| k.get(handle).is_none())
        {
            let position = self.position_of(parent, child);
            self.store.populate(vec![JoinRow {
                parent,
                child,
                position,
            }]);
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
        (container.try_cache::<P>(), container.try_cache::<C>())
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

    // The membership cache starts EMPTY — memberships fault through the memos on read and
    // enter through link/unlink on write; the file is never read whole.
    let store = Store::new(Keyed::new(Vec::new()));
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
        container: Rc::downgrade(&container.inner),
        join_table,
        parent_col,
        child_col,
        a_store: parent_store.store_id(),
        a_table: P::TABLE,
        a_field: field,
        a_key_col: P::KEY,
        a_delete: delete,
        b_store: child_store.store_id(),
        b_table: C::TABLE,
        b_key_col: C::KEY,
        b_field: Cell::new(None),
        b_delete: Cell::new(DeleteRule::Nullify),
        join_store: store.store_id(),
        ordered,
        store,
        delete_a,
        delete_b,
        memo_fwd: RefCell::new(HashMap::new()),
        memo_rev: RefCell::new(HashMap::new()),
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

day_reactive::tls_group! {
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

/// Bring a set of rows into the cache — what a relation write does before it writes.
type Materialize = Box<dyn Fn(&[u64])>;

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
                let next = if r.ordered {
                    let last = r
                        .members_of(a, true)
                        .iter()
                        .map(|c| r.position_of(a, *c))
                        .fold(f64::NEG_INFINITY, f64::max);
                    if last.is_finite() { last + 1.0 } else { 1.0 }
                } else {
                    0.0
                };
                r.link(a, b, next)
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
                    write_ord: Box::new(move |c, v| {
                        (r2.materialize)(&[c]);
                        (r2.write_ord)(c, v)
                    }),
                    is_member: r.parent_of(h) == Some(parent) && {
                        // `parent_of` answers from the file too; membership here means the
                        // CURRENT truth, which children_of's overlay establishes.
                        r.children_of(parent).contains(&h)
                    },
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
                        let handle = day_model::Key::Pair(parent, c).handle();
                        if r2.store.with_untracked(|k| k.get(handle).is_none()) {
                            let position = r2.position_of(parent, c);
                            r2.store.populate(vec![JoinRow {
                                parent,
                                child: c,
                                position,
                            }]);
                        }
                        r2.store.write_field(handle, "position", v)
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
                if !r.children_of(p).contains(&h) {
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
                let children = r.children_of(p);
                (r.materialize)(&children);
                for child in children {
                    (r.write_fk)(child, None);
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
    /// AFTER dirty-marking and query staleness; the writes it makes (cascades, nullifies)
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

    /// Drop every relation memo — the external-change and rescan reset.
    pub(crate) fn invalidate_relation_memos(&self) {
        for r in self.inner.relations.borrow().iter() {
            r.invalidate_all();
        }
        for j in self.inner.joins.borrow().iter() {
            j.invalidate_all();
        }
    }

    /// The cache for `M` when it is in this container's schema — the non-panicking
    /// [`ModelContainer::cache`].
    pub fn try_cache<M: Model>(&self) -> Option<Store<Keyed<M>>> {
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
        let store = self.cache::<M>();
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
        // The delete works resident or not — a value-free Delete folds to one statement —
        // but an installed undo stack needs the row to invert it.
        if day_model::record_capture_active() {
            let _ = self.ensure_resident::<M>(&[h]);
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
        let decode = move |r: &Vec<Value>| -> Result<JoinRow, DbError> {
            match (
                value_to_handle(&Row::get(r, 0)),
                value_to_handle(&Row::get(r, 1)),
            ) {
                (Some(p), Some(c)) => Ok(JoinRow {
                    parent: p,
                    child: c,
                    position: if ordered {
                        Row::get(r, 2).as_real().unwrap_or(0.0)
                    } else {
                        0.0
                    },
                }),
                _ => Err(DbError::new(
                    DbErrorKind::Decode,
                    format!("`{table}` holds a membership whose ids will not read as keys"),
                )),
            }
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
            columns,
            fields,
            fts: None,
            spatial: None,
            row_for: Rc::new(move |h| store.with_untracked(|k| k.get(h).map(&to_row))),
            all_rows: Rc::new(move || {
                store.with_untracked(|k| {
                    k.items()
                        .iter()
                        .map(|r| (day_model::Identified::handle(r), to_row(r)))
                        .collect()
                })
            }),
            resident_keys: Rc::new(move || store.with_untracked(|k| k.keys())),
            resident_len: Rc::new(move || store.with_untracked(|k| k.len())),
            is_resident: Rc::new(move |h| store.with_untracked(|k| k.get(h).is_some())),
            absorb: Rc::new(move |raw| {
                let mut rows = Vec::with_capacity(raw.len());
                for r in &raw {
                    rows.push(decode(r)?);
                }
                let keys = rows
                    .iter()
                    .map(day_model::Identified::handle)
                    .collect::<Vec<_>>();
                store.populate(rows);
                Ok(keys)
            }),
            refresh: Rc::new(move |raw| {
                // Memberships are a set: the diff against the RESIDENT rows is which pairs
                // arrived, moved, or left.
                let mut fresh: Vec<JoinRow> = Vec::with_capacity(raw.len());
                for r in &raw {
                    fresh.push(decode(r)?);
                }
                let existing: Vec<u64> = store.with_untracked(|k| k.keys());
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
            evict: Rc::new(move |candidates, want| {
                let doomed: Vec<u64> = candidates
                    .iter()
                    .copied()
                    .filter(|h| !store.is_observed(*h))
                    .take(want)
                    .collect();
                store.depopulate_many(&doomed)
            }),
            watch_undo: Rc::new(move |stack: &day_model::UndoStack| stack.watch(store)),
        };
        self.inner
            .tables
            .borrow_mut()
            .insert(store.store_id(), hooks);
    }
}
