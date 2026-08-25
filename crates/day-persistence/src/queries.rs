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
/// Everything evaluating a predicate can reach: the query's own rows, and — for a predicate
/// that crosses a relation — the related ids and the rows on the other side.
///
/// The two relation methods default to "no relations", so a caller that only has a table
/// (a unit test, a sort comparator) implements one method and behaves exactly as before.
pub trait EvalCtx {
    /// One row of the query's OWN table.
    fn local(&self, key: u64) -> Option<Box<dyn RowView + '_>>;

    /// The ids related to `key` through `field` — the children of a to-many, the referent of
    /// a to-one, the members of a join.
    fn related(&self, owner: &str, field: &str, key: u64) -> Vec<u64> {
        let _ = (owner, field, key);
        Vec::new()
    }

    /// One row of another table, reached across a relation.
    fn target(&self, table: &str, id: u64) -> Option<Box<dyn RowView + '_>> {
        let _ = (table, id);
        None
    }
}

/// A single row as a context — what a unit test evaluating one predicate against one row
/// wants. Answers that row for every key, and knows no relations.
pub struct OneRow<'a>(pub &'a dyn RowView);

impl EvalCtx for OneRow<'_> {
    fn local(&self, _key: u64) -> Option<Box<dyn RowView + '_>> {
        Some(Box::new(Borrowed(self.0)))
    }
}

struct Borrowed<'a>(&'a dyn RowView);

impl RowView for Borrowed<'_> {
    fn col(&self, column: &str) -> Option<Value> {
        self.0.col(column)
    }
}

/// How many related rows have to satisfy the inner predicate.
///
/// `All` over an empty relation is TRUE — the vacuous reading, and the one SQL gives for
/// `NOT EXISTS (… AND NOT p)`. It is the choice that surprises people, which is why `None`
/// sits beside it: "no unconfirmed lodging" and "every lodging confirmed" differ exactly for
/// the rows with nothing related, and an app usually means the former.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quant {
    Any,
    All,
    None,
    /// No related rows at all — answered from the relation index, no row read.
    Empty,
    /// At least `n` related rows — likewise O(1).
    CountGe(usize),
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
    /// Case-insensitive substring — what a search field wants. NOT `sql_exact`: see
    /// [`Pred::sql_exact`].
    ContainsCi(&'static str, String),
    /// Case-sensitive prefix. Deliberately NOT `LIKE`, whose SQLite default is
    /// case-INsensitive for ASCII and would quietly answer the wrong question.
    StartsWith(&'static str, String),
    /// Case-insensitive prefix. NOT `sql_exact`, for the same reason as [`Pred::ContainsCi`].
    StartsWithCi(&'static str, String),
    Between(&'static str, Value, Value),
    /// `column ∈ set`. The set is SORTED and deduped at construction, so evaluation is a
    /// binary search rather than a scan — relation traversal hands this thousands of ids.
    In(&'static str, Vec<Value>),
    /// `column ∉ set`, with SQL's own NULL rule: a NULL column is UNKNOWN, not a match.
    NotIn(&'static str, Vec<Value>),
    /// The ROW'S OWN KEY ∈ set — no column read, no decode, no codec, because the maintainer
    /// already holds the key. Sorted like [`Pred::In`].
    IdIn(Vec<u64>),
    /// A question about a row's RELATIVES: "some lodging of this trip is in Kyoto".
    /// `inner` evaluates against rows of `target`, not of the query's own table.
    Related {
        /// The table declaring the relation — what tells two like-named fields apart.
        owner: &'static str,
        /// The `Many` field, or the `One` column's FIELD name, that was crossed.
        field: &'static str,
        target: &'static str,
        quant: Quant,
        inner: Box<Pred>,
    },
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
            | Pred::StartsWith(c, _)
            | Pred::StartsWithCi(c, _)
            | Pred::In(c, _)
            | Pred::NotIn(c, _)
            | Pred::Between(c, _, _) => push(c, out),
            // A row's key never changes: it can only enter or leave an id set by being
            // inserted or deleted, which is a structural op, not a column write.
            Pred::IdIn(_) => {}
            // The inner predicate reads the TARGET's columns, which are a different table's
            // dependency — `Fetch::dependencies` collects them into `Deps::related`, and a
            // local column write can never move a row through them.
            Pred::Related { .. } => {}
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

    /// Collect what this predicate reads ACROSS relations. `deep` records that a relation was
    /// crossed inside another one: evaluation handles any depth, but resolving a related
    /// change back to the local rows it can move only walks one hop, so a deeper fetch
    /// re-queries instead of pretending.
    fn related_deps(&self, out: &mut Vec<RelatedDep>, deep: &mut bool, depth: usize) {
        match self {
            Pred::Related {
                owner,
                field,
                target,
                inner,
                ..
            } => {
                if depth > 0 {
                    *deep = true;
                    return;
                }
                let mut columns = Vec::new();
                inner.columns(&mut columns);
                out.push(RelatedDep {
                    owner,
                    field,
                    target_table: target,
                    columns,
                });
                inner.related_deps(out, deep, depth + 1);
            }
            Pred::And(a, b) | Pred::Or(a, b) => {
                a.related_deps(out, deep, depth);
                b.related_deps(out, deep, depth);
            }
            Pred::Not(a) => a.related_deps(out, deep, depth),
            _ => {}
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
            Pred::Related { inner, .. } => inner.evaluable(),
            _ => true,
        }
    }

    /// Whether [`Pred::to_sql`] selects the SAME rows this predicate's in-memory evaluation
    /// does — the contract that keeps a two-path query layer honest. `to_sql` may only be
    /// used when this answers `true`.
    ///
    /// Case-insensitive predicates answer `false`: in memory they fold with Rust's
    /// `to_lowercase`, which is full Unicode, while SQLite's `lower()` folds ASCII only, so
    /// `ÉCOLE` matches one way and not the other. SQL's form is not even a safe pre-filter
    /// there, because it *under*-matches — it would drop rows that belong. Such a predicate
    /// evaluates in memory; the exact fix, when a SQL-filtering path needs one, is a
    /// `day_lower` function registered through the driver's `with_init` hook.
    pub fn sql_exact(&self) -> bool {
        match self {
            Pred::ContainsCi(..) | Pred::StartsWithCi(..) => false,
            // The faithful SQL is a correlated `EXISTS`, which needs the wiring's column
            // names — not something the predicate carries. It belongs with the phase that
            // makes SQL filtering run at all; until then this says so rather than guessing.
            Pred::Related { .. } => false,
            Pred::And(a, b) | Pred::Or(a, b) => a.sql_exact() && b.sql_exact(),
            Pred::Not(a) => a.sql_exact(),
            _ => true,
        }
    }

    /// Does this row match? The WHERE-clause reading: UNKNOWN is not a match.
    pub fn eval(&self, key: u64, ctx: &dyn EvalCtx) -> bool {
        self.eval3(key, ctx) == Some(true)
    }

    /// [`Pred::eval`], three-valued. Resolves the row ONCE and recurses over it, so a
    /// compound predicate does not re-materialize the row per branch.
    pub fn eval3(&self, key: u64, ctx: &dyn EvalCtx) -> Option<bool> {
        match ctx.local(key) {
            Some(row) => self.eval_in(key, row.as_ref(), ctx),
            // A row that is not there matches nothing — definitely, not unknowably.
            None => Some(false),
        }
    }

    /// Three-valued evaluation — SQL's own logic, which is the only way the in-memory path
    /// and the SQL path can agree about NULL.
    ///
    /// A comparison against a NULL column is UNKNOWN (`None`), not false: SQL's `notes <> 'x'`
    /// does not select rows whose `notes` is NULL, and neither does this. Note that
    /// [`compare_values`] deliberately keeps ordering NULL below numbers — that is ORDER BY's
    /// rule and it stays correct for sorting; only comparison *predicates* follow the
    /// three-valued rule. `Eq`/`Ne` against a `Null` literal keep their `IS NULL` /
    /// `IS NOT NULL` meaning and are always definite.
    fn eval_in(&self, key: u64, row: &dyn RowView, ctx: &dyn EvalCtx) -> Option<bool> {
        match self {
            Pred::Always => Some(true),
            // Never reached on the incremental path: an unevaluable predicate re-queries.
            Pred::Raw(..) | Pred::Matches { .. } => Some(true),

            // IS NULL / IS NOT NULL: definite, even about NULL.
            Pred::Eq(c, Value::Null) => Some(matches!(row.col(c), Some(Value::Null) | None)),
            Pred::Ne(c, Value::Null) => Some(!matches!(row.col(c), Some(Value::Null) | None)),

            Pred::Eq(c, v) => defined(row.col(c)).map(|a| a == *v),
            Pred::Ne(c, v) => defined(row.col(c)).map(|a| a != *v),
            Pred::Lt(c, v) => defined(row.col(c)).map(|a| compare_values(&a, v) == Ordering::Less),
            Pred::Le(c, v) => {
                defined(row.col(c)).map(|a| compare_values(&a, v) != Ordering::Greater)
            }
            Pred::Gt(c, v) => {
                defined(row.col(c)).map(|a| compare_values(&a, v) == Ordering::Greater)
            }
            Pred::Ge(c, v) => defined(row.col(c)).map(|a| compare_values(&a, v) != Ordering::Less),
            Pred::Between(c, lo, hi) => defined(row.col(c)).map(|a| {
                compare_values(&a, lo) != Ordering::Less
                    && compare_values(&a, hi) != Ordering::Greater
            }),

            Pred::Contains(c, needle) => text(row.col(c)).map(|t| t.contains(needle.as_str())),
            Pred::ContainsCi(c, needle) => {
                text(row.col(c)).map(|t| t.to_lowercase().contains(&needle.to_lowercase()))
            }
            Pred::StartsWith(c, prefix) => text(row.col(c)).map(|t| t.starts_with(prefix.as_str())),
            Pred::StartsWithCi(c, prefix) => {
                text(row.col(c)).map(|t| t.to_lowercase().starts_with(&prefix.to_lowercase()))
            }

            Pred::In(c, set) => defined(row.col(c)).map(|a| in_set(set, &a)),
            Pred::NotIn(c, set) => defined(row.col(c)).map(|a| !in_set(set, &a)),
            // The key is always present and never NULL, so membership is definite.
            Pred::IdIn(ids) => Some(ids.binary_search(&key).is_ok()),

            Pred::Related {
                owner,
                field,
                target,
                quant,
                inner,
            } => {
                let ids = ctx.related(owner, field, key);
                match quant {
                    // Membership only: the index knows its own length, so no row is read.
                    Quant::Empty => Some(ids.is_empty()),
                    Quant::CountGe(n) => Some(ids.len() >= *n),
                    Quant::Any | Quant::None => {
                        let mut found = false;
                        for id in ids {
                            if let Some(r) = ctx.target(target, id)
                                && inner.eval_in(id, r.as_ref(), ctx) == Some(true)
                            {
                                found = true;
                                break; // short-circuits: one match settles it
                            }
                        }
                        Some(if *quant == Quant::Any { found } else { !found })
                    }
                    Quant::All => {
                        for id in ids {
                            let matched = ctx
                                .target(target, id)
                                .map(|r| inner.eval_in(id, r.as_ref(), ctx));
                            // A related row that is missing, or that the predicate cannot
                            // decide, is not a row that satisfies it.
                            if matched != Some(Some(true)) {
                                return Some(false);
                            }
                        }
                        // Vacuously true over an empty relation — see [`Quant`].
                        Some(true)
                    }
                }
            }

            // Kleene logic, so UNKNOWN propagates exactly as SQL propagates it.
            Pred::And(a, b) => match (a.eval_in(key, row, ctx), b.eval_in(key, row, ctx)) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            },
            Pred::Or(a, b) => match (a.eval_in(key, row, ctx), b.eval_in(key, row, ctx)) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            },
            Pred::Not(a) => a.eval_in(key, row, ctx).map(|v| !v),

            Pred::Within {
                lat,
                lon,
                min_lat,
                max_lat,
                min_lon,
                max_lon,
            } => {
                let in_range = |c: &str, lo: f64, hi: f64| match defined(row.col(c)) {
                    Some(Value::Real(v)) => Some(v >= lo && v <= hi),
                    Some(Value::Int(v)) => Some((v as f64) >= lo && (v as f64) <= hi),
                    Some(_) => Some(false),
                    None => None,
                };
                match (
                    in_range(lat, *min_lat, *max_lat),
                    in_range(lon, *min_lon, *max_lon),
                ) {
                    (Some(false), _) | (_, Some(false)) => Some(false),
                    (Some(true), Some(true)) => Some(true),
                    _ => None,
                }
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
            Pred::StartsWith(c, prefix) => {
                // NOT `LIKE`: SQLite's LIKE is case-INsensitive for ASCII by default, which
                // would quietly answer a different question. `substr` counts characters on
                // TEXT, which agrees with Rust's `starts_with` on any valid UTF-8.
                params.push(Value::Int(prefix.chars().count() as i64));
                params.push(Value::Text(prefix.clone()));
                format!("substr({c}, 1, ?) = ?")
            }
            // Not `sql_exact` — `to_sql` must not be reached for these. The form is written
            // for the day a SQL-filtering path registers an exact folding function.
            Pred::StartsWithCi(c, prefix) => {
                params.push(Value::Int(prefix.chars().count() as i64));
                params.push(Value::Text(prefix.to_lowercase()));
                format!("substr(lower({c}), 1, ?) = ?")
            }
            Pred::In(c, set) | Pred::NotIn(c, set) => {
                // `IN ()` is a syntax error in SQLite rather than an empty set, so the empty
                // case compiles to a constant — false for IN, true for NOT IN.
                if set.is_empty() {
                    return if matches!(self, Pred::In(..)) {
                        "0"
                    } else {
                        "1"
                    }
                    .into();
                }
                let negate = if matches!(self, Pred::NotIn(..)) {
                    "NOT "
                } else {
                    ""
                };
                let marks = vec!["?"; set.len()].join(", ");
                params.extend(set.iter().cloned());
                format!("{c} {negate}IN ({marks})")
            }
            Pred::IdIn(ids) => {
                // The container substitutes its table's key column for `{key}`, the same way
                // it substitutes the FTS shadow name for `{fts}`.
                if ids.is_empty() {
                    return "0".into();
                }
                for id in ids {
                    params.push(crate::key_param(*id));
                }
                let marks = vec!["?"; ids.len()].join(", ");
                format!("{{key}} IN ({marks})")
            }
            // Never reached: `sql_exact()` is false for a relation predicate, and `to_sql`
            // may only be used when that holds. A constant keeps the match total without
            // inventing SQL that would select the wrong rows.
            Pred::Related { .. } => "1".into(),
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

/// The fetch's total order over two rows: each sort key in turn, then the id as a stable
/// tie-break — so the order is deterministic and a binary search over it is well defined.
fn cmp_by(fetch: &Fetch, a: u64, b: u64, rows: &dyn EvalCtx) -> Ordering {
    for s in &fetch.sort {
        let va = rows.local(a).and_then(|r| r.col(s.column));
        let vb = rows.local(b).and_then(|r| r.col(s.column));
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
    a.cmp(&b)
}

/// A column's value when it is present and not NULL — otherwise UNKNOWN. A column the row
/// does not carry at all (a transient field's label) reads as UNKNOWN too, rather than as a
/// silent non-match.
fn defined(v: Option<Value>) -> Option<Value> {
    match v {
        Some(Value::Null) | None => None,
        Some(v) => Some(v),
    }
}

/// The TEXT of a column, or UNKNOWN. A text predicate can only be built on a `Col<String>`,
/// so a non-text value here is a schema mismatch rather than a real answer — UNKNOWN keeps it
/// out of the result instead of guessing at a coercion the two paths might disagree about.
fn text(v: Option<Value>) -> Option<String> {
    match defined(v) {
        Some(Value::Text(t)) => Some(t),
        _ => None,
    }
}

/// Membership in a sorted set, with the SAME equality `eq` uses: find the run that compares
/// equal under [`compare_values`], then confirm exactly. A mixed `Int`/`Real` set therefore
/// cannot make `is_in` and `eq` disagree with one another.
fn in_set(set: &[Value], v: &Value) -> bool {
    let start = set.partition_point(|x| compare_values(x, v) == Ordering::Less);
    set[start..]
        .iter()
        .take_while(|x| compare_values(x, v) == Ordering::Equal)
        .any(|x| x == v)
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
    /// The struct FIELD this column stores. The change log speaks field names and the SQL
    /// speaks column names; they differ under `#[model(column = "…")]`, and a relation is
    /// wired by field, so a predicate that crosses one needs both.
    pub field: &'static str,
    /// The table this column belongs to — what disambiguates a relation when two models
    /// happen to name a field alike.
    pub owner: &'static str,
    encode: fn(&V) -> Value,
}

impl<V> Clone for Col<V> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<V> Copy for Col<V> {}

/// A relation, as a predicate builder — what `Trip::lodging()` returns. The instance
/// accessor of the same name (`trip.lodging()`) reads and writes the relation; this one asks
/// questions about it in a query. They cannot collide: one takes `self`, this one does not.
pub struct RelationCol<P: 'static, T: 'static> {
    field: &'static str,
    owner: &'static str,
    target: &'static str,
    _p: std::marker::PhantomData<fn() -> (P, T)>,
}

impl<P, T> Clone for RelationCol<P, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<P, T> Copy for RelationCol<P, T> {}

impl<P, T> RelationCol<P, T> {
    pub const fn new(field: &'static str, owner: &'static str, target: &'static str) -> Self {
        RelationCol {
            field,
            owner,
            target,
            _p: std::marker::PhantomData,
        }
    }

    fn build(self, quant: Quant, inner: Pred) -> Pred {
        Pred::Related {
            owner: self.owner,
            field: self.field,
            target: self.target,
            quant,
            inner: Box::new(inner),
        }
    }

    /// Some related row matches. False when there are none.
    pub fn any(self, inner: Pred) -> Pred {
        self.build(Quant::Any, inner)
    }

    /// No related row matches. True when there are none.
    pub fn none(self, inner: Pred) -> Pred {
        self.build(Quant::None, inner)
    }

    /// Every related row matches — VACUOUSLY TRUE when there are none, as in SQL. Reach for
    /// [`RelationCol::none`] when that is not what you meant.
    pub fn all(self, inner: Pred) -> Pred {
        self.build(Quant::All, inner)
    }

    /// No related rows at all — answered from the index, without reading one.
    pub fn is_empty(self) -> Pred {
        self.build(Quant::Empty, Pred::Always)
    }

    /// At least `n` related rows — likewise O(1).
    pub fn count_ge(self, n: usize) -> Pred {
        self.build(Quant::CountGe(n), Pred::Always)
    }
}

/// The encoder a plain (codec-less) field's [`Col`] carries.
pub fn encode_column<T: crate::ColumnValue>(v: &T) -> Value {
    v.to_sqlite_value()
}

impl<V> Col<V> {
    pub const fn new(
        column: &'static str,
        field: &'static str,
        owner: &'static str,
        encode: fn(&V) -> Value,
    ) -> Col<V> {
        Col {
            column,
            field,
            owner,
            encode,
        }
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
    /// `column ∈ values`, each encoded through this column's own codec exactly as `eq` does.
    /// An EMPTY set matches nothing, in both evaluation paths.
    pub fn is_in(self, values: impl IntoIterator<Item = impl std::borrow::Borrow<V>>) -> Pred {
        Pred::In(self.column, self.encode_set(values))
    }

    /// The complement of [`Col::is_in`], with SQL's NULL rule: a NULL column is UNKNOWN, so
    /// it is not selected — `not_in` is therefore NOT the same as `!is_in` over nullable
    /// columns, exactly as in SQL.
    pub fn not_in(self, values: impl IntoIterator<Item = impl std::borrow::Borrow<V>>) -> Pred {
        Pred::NotIn(self.column, self.encode_set(values))
    }

    /// Sorted and deduped once, here, so evaluation can binary-search it.
    fn encode_set(
        self,
        values: impl IntoIterator<Item = impl std::borrow::Borrow<V>>,
    ) -> Vec<Value> {
        let mut set: Vec<Value> = values.into_iter().map(|v| self.enc(v.borrow())).collect();
        set.sort_by(compare_values);
        set.dedup();
        set
    }

    /// `column IS NULL`.
    pub fn is_null(self) -> Pred {
        Pred::Eq(self.column, Value::Null)
    }

    /// `column IS NOT NULL`.
    pub fn is_not_null(self) -> Pred {
        Pred::Ne(self.column, Value::Null)
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
    /// Case-SENSITIVE prefix match.
    pub fn starts_with(self, prefix: impl Into<String>) -> Pred {
        Pred::StartsWith(self.column, prefix.into())
    }

    /// Case-insensitive prefix match — evaluated in memory ([`Pred::sql_exact`]).
    pub fn starts_with_ci(self, prefix: impl Into<String>) -> Pred {
        Pred::StartsWithCi(self.column, prefix.into())
    }

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
    pub fn dependencies(&self) -> Deps {
        let mut local = Vec::new();
        self.pred.columns(&mut local);
        for s in &self.sort {
            if !s.by_rank && !local.contains(&s.column) {
                local.push(s.column);
            }
        }
        let mut related = Vec::new();
        let mut deep = false;
        self.pred.related_deps(&mut related, &mut deep, 0);
        Deps {
            local,
            related,
            deep,
        }
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

/// What a fetch reads, and therefore what can move a row through it.
///
/// Split by table on purpose: a query's own columns are one question, and the columns it reads
/// across a relation are another — a change to a related row that the predicate never mentions
/// must stay as free as a change to a local column it never mentions. Relation-traversing
/// predicates fill `related`; today it is always empty, and a fetch with no relation in it
/// takes exactly the path it always did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Deps {
    /// Columns of the query's own table — its predicate's and its sort's.
    pub local: Vec<&'static str>,
    /// One entry per relation the predicate crosses, at the top level.
    pub related: Vec<RelatedDep>,
    /// A relation was crossed INSIDE another one. Evaluation handles it; incremental
    /// back-resolution does not, so a related change re-queries rather than guess.
    pub deep: bool,
}

/// One relation's contribution to a fetch's dependencies.
#[derive(Clone, Debug, PartialEq)]
pub struct RelatedDep {
    /// The table declaring the relation.
    pub owner: &'static str,
    /// The `Many` field, or the `One` column, the predicate crossed.
    pub field: &'static str,
    pub target_table: &'static str,
    /// Columns of the TARGET table the inner predicate reads. Empty when the predicate asks
    /// only about membership (`is_empty`, `count_ge`), which no column write can change.
    pub columns: Vec<&'static str>,
}

impl Deps {
    /// Whether a change to this column of the query's OWN table can move a row.
    pub fn touches_local(&self, column: &str) -> bool {
        self.local.contains(&column)
    }

    /// Whether a change to this column of `table`, reached across a relation, can move a row.
    pub fn touches_related(&self, table: &str, column: &str) -> bool {
        self.related
            .iter()
            .any(|r| r.target_table == table && r.columns.contains(&column))
    }

    /// The tables this fetch reads across a relation — what a query subscribes to beyond its
    /// own store.
    pub fn related_tables(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = Vec::new();
        for r in &self.related {
            if !out.contains(&r.target_table) {
                out.push(r.target_table);
            }
        }
        out
    }
}

/// A query's result set, maintained in place against announced changes.
pub struct LiveSet {
    ids: Vec<u64>,
    fetch: Fetch,
    deps: Deps,
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

    /// What this set reads — its own columns, and anything it reaches across a relation.
    pub fn deps(&self) -> &Deps {
        &self.deps
    }

    pub fn evaluations(&self) -> usize {
        self.evaluations.get()
    }

    /// Replace the whole set (the seed fetch, or a requery's answer).
    pub fn reset(&mut self, ids: Vec<u64>) {
        self.ids = ids;
    }

    /// Seed by evaluating over every key — the in-memory fetch a document-pattern store uses.
    pub fn seed(&mut self, keys: &[u64], rows: &dyn EvalCtx) {
        self.ids = keys
            .iter()
            .copied()
            .filter(|k| self.fetch.pred.eval(*k, rows))
            .collect();
        self.sort_ids(rows);
        if let Some(n) = self.fetch.limit {
            self.ids.truncate(n);
        }
    }

    fn sort_ids(&mut self, rows: &dyn EvalCtx) {
        let fetch = &self.fetch;
        self.ids.sort_by(|a, b| cmp_by(fetch, *a, *b, rows));
    }

    /// Where `key` belongs in the ALREADY-SORTED `ids` — O(log n) comparisons, against the
    /// O(n log n) a re-sort costs. This is what keeps one edit to a sort column one edit's
    /// worth of work no matter how large the result set is; `ids` is a sorted run at every
    /// point the maintainer observes it, which is the invariant that makes the search valid.
    fn sorted_position(&self, key: u64, rows: &dyn EvalCtx) -> usize {
        let fetch = &self.fetch;
        self.ids
            .partition_point(|other| cmp_by(fetch, *other, key, rows) == Ordering::Less)
    }

    /// Apply one announced change: `key` is the row, `column` the changed field's column name
    /// (empty for structural ops), `op` what happened.
    pub fn apply(
        &mut self,
        key: u64,
        column: &str,
        op: day_model::Op,
        rows: &dyn EvalCtx,
    ) -> Outcome {
        if !self.fetch.evaluable() {
            // Raw SQL re-queries for everything — unreadable is unreadable. An FTS/rank fetch
            // still has a DECLARED dependency set (the indexed columns), so a column outside
            // it keeps the zero-cost tier even here.
            return match op {
                _ if self.fetch.pred.contains_raw() => Outcome::Requery,
                day_model::Op::Set if !column.is_empty() && !self.deps.touches_local(column) => {
                    Outcome::Unaffected
                }
                day_model::Op::Move => Outcome::Unaffected,
                _ => Outcome::Requery,
            };
        }
        match op {
            // THE TIER THAT MATTERS: a column no part of this query mentions cannot move the
            // set, so nothing is evaluated at all.
            day_model::Op::Set if !column.is_empty() && !self.deps.touches_local(column) => {
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

    fn reposition(&mut self, key: u64, rows: &dyn EvalCtx) -> Outcome {
        self.evaluations.set(self.evaluations.get() + 1);
        let belongs = self.fetch.pred.eval(key, rows);
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
                let i = self.sorted_position(key, rows);
                self.ids.insert(i, key);
                Outcome::Changed(vec![Delta::Insert(i, key)])
            }
            (true, Some(from)) => {
                // Lift the row out, then binary-search where it now belongs among the rest —
                // one row moved, so the remainder is still sorted.
                self.ids.remove(from);
                let to = self.sorted_position(key, rows);
                self.ids.insert(to, key);
                if to == from {
                    Outcome::Unaffected // in the set, in place: the row repaints itself
                } else {
                    Outcome::Changed(vec![Delta::Move(from, to, key)])
                }
            }
        }
    }
}
