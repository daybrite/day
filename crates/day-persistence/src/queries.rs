// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Typed queries and the live result-set maintainer (docs/persistence.md; the plan's §15).
//!
//! A predicate here is DATA, not a string: the same value compiles to SQL for a fetch and
//! evaluates in memory for every change after it — which is what lets a result set stay
//! current WITHOUT re-running the query. The change log names the column, so a write to a
//! column the query never mentions is discarded before any predicate is evaluated at all;
//! a predicate or sort column evaluates exactly the row that changed. This is Core Data's
//! `NSFetchedResultsController` algorithm with one tier it never had.

use std::cell::Cell;
use std::cmp::Ordering;

use crate::Value;

/// A row, as a predicate can read it: column name → stored value. Implementations answer in
/// the column's STORED language (through the field's codec), which is the language predicates
/// encode their arguments into — comparisons never mix representations.
pub trait RowView {
    fn col(&self, column: &str) -> Option<Value>;
}

/// The rows a [`LiveSet`] reads while maintaining itself.
pub trait RowsView {
    fn row_view(&self, key: u64) -> Option<Box<dyn RowView + '_>>;
}

/// A predicate as a value. Compiles to SQL; evaluates in memory.
#[derive(Clone, Debug, PartialEq)]
pub enum Pred {
    Always,
    Eq(&'static str, Value),
    Ne(&'static str, Value),
    Lt(&'static str, Value),
    Le(&'static str, Value),
    Gt(&'static str, Value),
    Ge(&'static str, Value),
    /// Case-sensitive substring on a TEXT column (SQL: `instr(col, ?) > 0`).
    Contains(&'static str, String),
    /// Case-insensitive substring — what a search field wants (SQL over `lower()`).
    ContainsCi(&'static str, String),
    Between(&'static str, Value, Value),
    And(Box<Pred>, Box<Pred>),
    Or(Box<Pred>, Box<Pred>),
    Not(Box<Pred>),
    /// Raw SQL the layer cannot read. Opts the query OUT of incremental maintenance, which is
    /// the honest cost of taking it.
    Raw(String, Vec<Value>),
    /// Full-text match (feature `fts` on the model): SQLite answers it, this layer cannot —
    /// the columns are the FTS-indexed set, and any change to one of them re-queries.
    Matches {
        columns: &'static [&'static str],
        query: String,
    },
    /// A bounding-box test over two REAL columns — range comparisons, so it evaluates in
    /// memory like any other predicate.
    Within {
        lat: &'static str,
        lon: &'static str,
        min_lat: f64,
        max_lat: f64,
        min_lon: f64,
        max_lon: f64,
    },
}

impl Pred {
    /// The columns whose change can move a result through this predicate.
    pub fn columns(&self, out: &mut Vec<&'static str>) {
        let push = |c: &'static str, out: &mut Vec<&'static str>| {
            if !out.contains(&c) {
                out.push(c);
            }
        };
        match self {
            Pred::Always | Pred::Raw(..) => {}
            Pred::Eq(c, _)
            | Pred::Ne(c, _)
            | Pred::Lt(c, _)
            | Pred::Le(c, _)
            | Pred::Gt(c, _)
            | Pred::Ge(c, _)
            | Pred::Contains(c, _)
            | Pred::ContainsCi(c, _)
            | Pred::Between(c, _, _) => push(c, out),
            Pred::And(a, b) | Pred::Or(a, b) => {
                a.columns(out);
                b.columns(out);
            }
            Pred::Not(a) => a.columns(out),
            Pred::Matches { columns, .. } => {
                for c in *columns {
                    push(c, out);
                }
            }
            Pred::Within { lat, lon, .. } => {
                push(lat, out);
                push(lon, out);
            }
        }
    }

    /// Whether any part is RAW SQL — unreadable, so every change re-queries.
    pub fn contains_raw(&self) -> bool {
        match self {
            Pred::Raw(..) => true,
            Pred::And(a, b) | Pred::Or(a, b) => a.contains_raw() || b.contains_raw(),
            Pred::Not(a) => a.contains_raw(),
            _ => false,
        }
    }

    /// Whether this layer can answer "does the changed row match?" without the database.
    pub fn evaluable(&self) -> bool {
        match self {
            Pred::Raw(..) | Pred::Matches { .. } => false,
            Pred::And(a, b) | Pred::Or(a, b) => a.evaluable() && b.evaluable(),
            Pred::Not(a) => a.evaluable(),
            _ => true,
        }
    }

    pub fn eval(&self, row: &dyn RowView) -> bool {
        match self {
            Pred::Always => true,
            // Never reached on the incremental path: an unevaluable predicate re-queries.
            Pred::Raw(..) | Pred::Matches { .. } => true,
            Pred::Eq(c, v) => row.col(c).as_ref() == Some(v),
            Pred::Ne(c, v) => row.col(c).as_ref() != Some(v),
            Pred::Lt(c, v) => cmp_col(row, c, v) == Some(Ordering::Less),
            Pred::Le(c, v) => matches!(cmp_col(row, c, v), Some(Ordering::Less | Ordering::Equal)),
            Pred::Gt(c, v) => cmp_col(row, c, v) == Some(Ordering::Greater),
            Pred::Ge(c, v) => matches!(
                cmp_col(row, c, v),
                Some(Ordering::Greater | Ordering::Equal)
            ),
            Pred::Contains(c, needle) => match row.col(c) {
                Some(Value::Text(t)) => t.contains(needle.as_str()),
                _ => false,
            },
            Pred::ContainsCi(c, needle) => match row.col(c) {
                Some(Value::Text(t)) => t.to_lowercase().contains(needle.to_lowercase().as_str()),
                _ => false,
            },
            Pred::Between(c, lo, hi) => {
                matches!(
                    cmp_col(row, c, lo),
                    Some(Ordering::Greater | Ordering::Equal)
                ) && matches!(cmp_col(row, c, hi), Some(Ordering::Less | Ordering::Equal))
            }
            Pred::And(a, b) => a.eval(row) && b.eval(row),
            Pred::Or(a, b) => a.eval(row) || b.eval(row),
            Pred::Not(a) => !a.eval(row),
            Pred::Within {
                lat,
                lon,
                min_lat,
                max_lat,
                min_lon,
                max_lon,
            } => {
                let in_range = |c: &str, lo: f64, hi: f64| match row.col(c) {
                    Some(Value::Real(v)) => v >= lo && v <= hi,
                    Some(Value::Int(v)) => (v as f64) >= lo && (v as f64) <= hi,
                    _ => false,
                };
                in_range(lat, *min_lat, *max_lat) && in_range(lon, *min_lon, *max_lon)
            }
        }
    }

    /// The SQL form, appending its bound parameters to `params`.
    pub fn to_sql(&self, params: &mut Vec<Value>) -> String {
        match self {
            Pred::Always => "1".into(),
            Pred::Eq(c, Value::Null) => format!("{c} IS NULL"),
            Pred::Ne(c, Value::Null) => format!("{c} IS NOT NULL"),
            Pred::Eq(c, v) => {
                params.push(v.clone());
                format!("{c} = ?")
            }
            Pred::Ne(c, v) => {
                params.push(v.clone());
                format!("{c} <> ?")
            }
            Pred::Lt(c, v) => {
                params.push(v.clone());
                format!("{c} < ?")
            }
            Pred::Le(c, v) => {
                params.push(v.clone());
                format!("{c} <= ?")
            }
            Pred::Gt(c, v) => {
                params.push(v.clone());
                format!("{c} > ?")
            }
            Pred::Ge(c, v) => {
                params.push(v.clone());
                format!("{c} >= ?")
            }
            Pred::Contains(c, s) => {
                params.push(Value::Text(s.clone()));
                format!("instr({c}, ?) > 0")
            }
            Pred::ContainsCi(c, s) => {
                params.push(Value::Text(s.to_lowercase()));
                format!("instr(lower({c}), ?) > 0")
            }
            Pred::Between(c, lo, hi) => {
                params.push(lo.clone());
                params.push(hi.clone());
                format!("{c} BETWEEN ? AND ?")
            }
            Pred::And(a, b) => format!("({} AND {})", a.to_sql(params), b.to_sql(params)),
            Pred::Or(a, b) => format!("({} OR {})", a.to_sql(params), b.to_sql(params)),
            Pred::Not(a) => format!("NOT ({})", a.to_sql(params)),
            Pred::Raw(sql, args) => {
                params.extend(args.iter().cloned());
                format!("({sql})")
            }
            Pred::Matches { query, .. } => {
                // The caller (the container) substitutes its table's FTS shadow name for
                // `{fts}`; the predicate itself cannot know which table it is applied to.
                params.push(Value::Text(query.clone()));
                "id IN (SELECT rowid FROM {fts} WHERE {fts} MATCH ?)".into()
            }
            Pred::Within {
                lat,
                lon,
                min_lat,
                max_lat,
                min_lon,
                max_lon,
            } => {
                params.push(Value::Real(*min_lat));
                params.push(Value::Real(*max_lat));
                params.push(Value::Real(*min_lon));
                params.push(Value::Real(*max_lon));
                format!("({lat} BETWEEN ? AND ? AND {lon} BETWEEN ? AND ?)")
            }
        }
    }
}

impl std::ops::BitAnd for Pred {
    type Output = Pred;
    fn bitand(self, rhs: Pred) -> Pred {
        Pred::And(Box::new(self), Box::new(rhs))
    }
}

impl std::ops::BitOr for Pred {
    type Output = Pred;
    fn bitor(self, rhs: Pred) -> Pred {
        Pred::Or(Box::new(self), Box::new(rhs))
    }
}

impl std::ops::Not for Pred {
    type Output = Pred;
    fn not(self) -> Pred {
        Pred::Not(Box::new(self))
    }
}

fn cmp_col(row: &dyn RowView, c: &str, v: &Value) -> Option<Ordering> {
    row.col(c).map(|a| compare_values(&a, v))
}

/// SQLite's cross-class ordering (NULL < numbers < text < blob), with one deliberate
/// difference: `Real` compares by `total_cmp`, so a NaN that reaches a sort key still orders
/// deterministically instead of poisoning the sort.
pub fn compare_values(a: &Value, b: &Value) -> Ordering {
    fn class(v: &Value) -> u8 {
        match v {
            Value::Null => 0,
            Value::Int(_) | Value::Real(_) => 1,
            Value::Text(_) => 2,
            Value::Blob(_) => 3,
        }
    }
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Real(x), Value::Real(y)) => x.total_cmp(y),
        (Value::Int(x), Value::Real(y)) => (*x as f64).total_cmp(y),
        (Value::Real(x), Value::Int(y)) => x.total_cmp(&(*y as f64)),
        (Value::Text(x), Value::Text(y)) => x.cmp(y),
        (Value::Blob(x), Value::Blob(y)) => x.cmp(y),
        _ => class(a).cmp(&class(b)),
    }
}

/// A typed column reference — what `Trip::name()` returns (the derive emits one inherent fn
/// per persisted field). It knows the column's NAME and its stored ENCODING, so a predicate
/// built from it compares in the column's stored language whatever codec the field uses.
pub struct Col<V: 'static> {
    pub column: &'static str,
    encode: fn(&V) -> Value,
}

impl<V> Clone for Col<V> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<V> Copy for Col<V> {}

/// The encoder a plain (codec-less) field's [`Col`] carries.
pub fn encode_column<T: crate::ColumnValue>(v: &T) -> Value {
    v.to_sqlite_value()
}

impl<V> Col<V> {
    pub const fn new(column: &'static str, encode: fn(&V) -> Value) -> Col<V> {
        Col { column, encode }
    }
    fn enc(&self, v: &V) -> Value {
        (self.encode)(v)
    }
    pub fn eq(self, v: impl std::borrow::Borrow<V>) -> Pred {
        Pred::Eq(self.column, self.enc(v.borrow()))
    }
    pub fn ne(self, v: impl std::borrow::Borrow<V>) -> Pred {
        Pred::Ne(self.column, self.enc(v.borrow()))
    }
    pub fn lt(self, v: impl std::borrow::Borrow<V>) -> Pred {
        Pred::Lt(self.column, self.enc(v.borrow()))
    }
    pub fn le(self, v: impl std::borrow::Borrow<V>) -> Pred {
        Pred::Le(self.column, self.enc(v.borrow()))
    }
    pub fn gt(self, v: impl std::borrow::Borrow<V>) -> Pred {
        Pred::Gt(self.column, self.enc(v.borrow()))
    }
    pub fn ge(self, v: impl std::borrow::Borrow<V>) -> Pred {
        Pred::Ge(self.column, self.enc(v.borrow()))
    }
    pub fn between(self, lo: impl std::borrow::Borrow<V>, hi: impl std::borrow::Borrow<V>) -> Pred {
        Pred::Between(self.column, self.enc(lo.borrow()), self.enc(hi.borrow()))
    }
    pub fn asc(self) -> Sort {
        Sort::asc(self.column)
    }
    pub fn desc(self) -> Sort {
        Sort::desc(self.column)
    }
}

/// What `Trip::fts()` returns when the model declares `#[model(fts(…))]`.
#[derive(Clone, Copy)]
pub struct FtsRef {
    pub columns: &'static [&'static str],
}

impl FtsRef {
    /// FTS5 MATCH — SQLite answers it; the query re-runs when an INDEXED column changes and
    /// ignores every other column, so the zero-cost tier survives full-text search.
    pub fn matches(self, query: impl Into<String>) -> Pred {
        Pred::Matches {
            columns: self.columns,
            query: query.into(),
        }
    }
}

/// What `Trip::geo()` returns when the model declares `#[model(spatial(…))]`.
#[derive(Clone, Copy)]
pub struct GeoRef {
    pub lat: &'static str,
    pub lon: &'static str,
}

/// A bounding box for [`GeoRef::within`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeoRect {
    pub min_lat: f64,
    pub max_lat: f64,
    pub min_lon: f64,
    pub max_lon: f64,
}

impl GeoRef {
    /// Range comparisons over the two columns — evaluable in memory, so a moved pin is one
    /// evaluation and one delta, never a re-query.
    pub fn within(self, r: GeoRect) -> Pred {
        Pred::Within {
            lat: self.lat,
            lon: self.lon,
            min_lat: r.min_lat,
            max_lat: r.max_lat,
            min_lon: r.min_lon,
            max_lon: r.max_lon,
        }
    }
}

/// Order by FTS relevance (bm25, best first) — pair with a `matches` predicate.
pub fn rank() -> Sort {
    Sort {
        column: "",
        ascending: true,
        by_rank: true,
    }
}

impl Col<String> {
    /// Case-sensitive substring.
    pub fn contains(self, needle: impl Into<String>) -> Pred {
        Pred::Contains(self.column, needle.into())
    }
    /// Case-insensitive substring — what a search field wants.
    pub fn contains_ci(self, needle: impl Into<String>) -> Pred {
        Pred::ContainsCi(self.column, needle.into())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sort {
    pub column: &'static str,
    pub ascending: bool,
    /// Order by FTS relevance (bm25) instead of a column — `column` is empty; unevaluable in
    /// memory, so a rank sort re-queries like a `matches` predicate does.
    pub by_rank: bool,
}

impl Sort {
    pub fn asc(column: &'static str) -> Sort {
        Sort {
            column,
            ascending: true,
            by_rank: false,
        }
    }
    pub fn desc(column: &'static str) -> Sort {
        Sort {
            column,
            ascending: false,
            by_rank: false,
        }
    }
}

/// A declared fetch: predicate, sort, window.
#[derive(Clone, Debug, PartialEq)]
pub struct Fetch {
    pub pred: Pred,
    pub sort: Vec<Sort>,
    pub limit: Option<usize>,
}

impl Default for Fetch {
    fn default() -> Fetch {
        Fetch {
            pred: Pred::Always,
            sort: Vec::new(),
            limit: None,
        }
    }
}

impl Fetch {
    pub fn new() -> Fetch {
        Fetch::default()
    }
    pub fn filter(mut self, p: Pred) -> Fetch {
        self.pred = match self.pred {
            Pred::Always => p,
            prior => prior & p,
        };
        self
    }
    pub fn sort(mut self, s: Sort) -> Fetch {
        self.sort.push(s);
        self
    }
    pub fn limit(mut self, n: usize) -> Fetch {
        self.limit = Some(n);
        self
    }

    /// The columns a change must touch for this query's RESULT to be able to move. Everything
    /// else is a row-level change the query ignores entirely.
    pub fn dependencies(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        self.pred.columns(&mut out);
        for s in &self.sort {
            if !s.by_rank && !out.contains(&s.column) {
                out.push(s.column);
            }
        }
        out
    }

    /// Whether the incremental path can maintain this fetch at all.
    pub fn evaluable(&self) -> bool {
        self.pred.evaluable() && self.sort.iter().all(|s| !s.by_rank)
    }
}

/// What a change did to the result set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Delta {
    Insert(usize, u64),
    Remove(usize, u64),
    Move(usize, usize, u64),
}

#[derive(Clone, PartialEq, Debug)]
pub enum Outcome {
    /// The set is unchanged, and nothing downstream needs waking.
    Unaffected,
    /// The set changed exactly this much — enough to animate a list rather than reload it.
    Changed(Vec<Delta>),
    /// The set cannot be maintained in memory; ask again.
    Requery,
}

/// A query's result set, maintained in place against announced changes.
pub struct LiveSet {
    ids: Vec<u64>,
    fetch: Fetch,
    deps: Vec<&'static str>,
    /// How many times a predicate or sort key has been evaluated — the cost this exists to
    /// avoid, counted so tests can assert the zero rows of §15's table.
    evaluations: Cell<usize>,
}

impl LiveSet {
    pub fn new(fetch: Fetch) -> LiveSet {
        LiveSet {
            ids: Vec::new(),
            deps: fetch.dependencies(),
            fetch,
            evaluations: Cell::new(0),
        }
    }

    pub fn ids(&self) -> &[u64] {
        &self.ids
    }

    pub fn fetch(&self) -> &Fetch {
        &self.fetch
    }

    pub fn evaluations(&self) -> usize {
        self.evaluations.get()
    }

    /// Replace the whole set (the seed fetch, or a requery's answer).
    pub fn reset(&mut self, ids: Vec<u64>) {
        self.ids = ids;
    }

    /// Seed by evaluating over every key — the in-memory fetch a document-pattern store uses.
    pub fn seed(&mut self, keys: &[u64], rows: &dyn RowsView) {
        self.ids = keys
            .iter()
            .copied()
            .filter(|k| {
                rows.row_view(*k)
                    .map(|r| self.fetch.pred.eval(r.as_ref()))
                    .unwrap_or(false)
            })
            .collect();
        self.sort_ids(rows);
        if let Some(n) = self.fetch.limit {
            self.ids.truncate(n);
        }
    }

    fn sort_ids(&mut self, rows: &dyn RowsView) {
        let fetch = &self.fetch;
        self.ids.sort_by(|a, b| {
            for s in &fetch.sort {
                let va = rows.row_view(*a).and_then(|r| r.col(s.column));
                let vb = rows.row_view(*b).and_then(|r| r.col(s.column));
                let ord = match (&va, &vb) {
                    (Some(x), Some(y)) => compare_values(x, y),
                    (None, None) => Ordering::Equal,
                    (None, _) => Ordering::Less,
                    (_, None) => Ordering::Greater,
                };
                let ord = if s.ascending { ord } else { ord.reverse() };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            a.cmp(b) // a stable tie-break, so the order is deterministic
        });
    }

    /// Apply one announced change: `key` is the row, `column` the changed field's column name
    /// (empty for structural ops), `op` what happened.
    pub fn apply(
        &mut self,
        key: u64,
        column: &str,
        op: day_model::Op,
        rows: &dyn RowsView,
    ) -> Outcome {
        if !self.fetch.evaluable() {
            // Raw SQL re-queries for everything — unreadable is unreadable. An FTS/rank fetch
            // still has a DECLARED dependency set (the indexed columns), so a column outside
            // it keeps the zero-cost tier even here.
            return match op {
                _ if self.fetch.pred.contains_raw() => Outcome::Requery,
                day_model::Op::Set if !column.is_empty() && !self.deps.contains(&column) => {
                    Outcome::Unaffected
                }
                day_model::Op::Move => Outcome::Unaffected,
                _ => Outcome::Requery,
            };
        }
        match op {
            // THE TIER THAT MATTERS: a column no part of this query mentions cannot move the
            // set, so nothing is evaluated at all.
            day_model::Op::Set if !column.is_empty() && !self.deps.contains(&column) => {
                Outcome::Unaffected
            }
            day_model::Op::Set | day_model::Op::Insert => self.reposition(key, rows),
            day_model::Op::Delete => match self.ids.iter().position(|k| *k == key) {
                Some(i) => {
                    self.ids.remove(i);
                    // A window that just lost a row may have another waiting behind it.
                    if self.fetch.limit.is_some() {
                        return Outcome::Requery;
                    }
                    Outcome::Changed(vec![Delta::Remove(i, key)])
                }
                None => Outcome::Unaffected,
            },
            day_model::Op::Move => Outcome::Unaffected, // user order is not a sorted query's business
        }
    }

    fn reposition(&mut self, key: u64, rows: &dyn RowsView) -> Outcome {
        self.evaluations.set(self.evaluations.get() + 1);
        let belongs = rows
            .row_view(key)
            .map(|r| self.fetch.pred.eval(r.as_ref()))
            .unwrap_or(false);
        let at = self.ids.iter().position(|k| *k == key);

        match (belongs, at) {
            (false, None) => Outcome::Unaffected,
            (false, Some(i)) => {
                self.ids.remove(i);
                if self.fetch.limit.is_some() {
                    return Outcome::Requery;
                }
                Outcome::Changed(vec![Delta::Remove(i, key)])
            }
            (true, None) => {
                if self.fetch.limit.is_some() {
                    return Outcome::Requery; // an entrant can push the window's tail out
                }
                self.ids.push(key);
                self.sort_ids(rows);
                let i = self.ids.iter().position(|k| *k == key).unwrap_or(0);
                Outcome::Changed(vec![Delta::Insert(i, key)])
            }
            (true, Some(from)) => {
                self.sort_ids(rows);
                let to = self.ids.iter().position(|k| *k == key).unwrap_or(from);
                if to == from {
                    Outcome::Unaffected // in the set, in place: the row repaints itself
                } else {
                    Outcome::Changed(vec![Delta::Move(from, to, key)])
                }
            }
        }
    }
}
