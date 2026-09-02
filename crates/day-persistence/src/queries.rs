// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Typed queries and their SQL compiler (docs/persistence.md).
//!
//! A predicate here is DATA, not a string: the same value compiles to a WHERE clause the
//! engine can drive from its indexes, and names its column DEPENDENCIES so a live query knows
//! which changes can move its result at all. The engine answers every fetch — filter, sort,
//! window, relation traversal, full-text match, spatial candidates — which is what lets a
//! container serve a million-row table without ever holding it in memory. The change log
//! names the column a write touched, so a write to a column the query never mentions is
//! discarded before any SQL runs; a write the query does depend on marks it stale, and one
//! requery after the turn's flush re-derives the id set, diffed against the old one so a list
//! can animate the difference instead of reloading.

use std::cmp::Ordering;

use crate::Value;

/// A row, as a predicate can read it: column name → stored value. Implementations answer in
/// the column's STORED language (through the field's codec), which is the language predicates
/// encode their arguments into — comparisons never mix representations.
pub trait RowView {
    fn col(&self, column: &str) -> Option<Value>;
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
    /// No related rows at all — one `NOT EXISTS` over the foreign-key index.
    Empty,
    /// At least `n` related rows — a correlated `COUNT` over the same index.
    CountGe(usize),
}

/// A predicate as a value. Compiles to SQL; the fallback evaluator ([`Pred::eval`]) exists
/// for drivers that cannot fold case exactly, and for tests.
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
    /// Case-insensitive substring — what a search field wants. Folds with full Unicode
    /// lowercasing on BOTH paths: the driver registers `day_fold` (Rust's `to_lowercase` as a
    /// SQL function), so the SQL form selects exactly the rows the in-memory form would.
    ContainsCi(&'static str, String),
    /// Case-sensitive prefix. Deliberately NOT `LIKE`, whose SQLite default is
    /// case-INsensitive for ASCII and would quietly answer the wrong question.
    StartsWith(&'static str, String),
    /// Case-insensitive prefix — `day_fold`, like [`Pred::ContainsCi`].
    StartsWithCi(&'static str, String),
    Between(&'static str, Value, Value),
    /// `column ∈ set`. The set is SORTED and deduped at construction, so the fallback
    /// evaluator can binary-search it.
    In(&'static str, Vec<Value>),
    /// `column ∉ set`, with SQL's own NULL rule: a NULL column is UNKNOWN, not a match.
    NotIn(&'static str, Vec<Value>),
    /// The ROW'S OWN KEY ∈ set — compiles against the key column directly.
    IdIn(Vec<u64>),
    /// A question about a row's RELATIVES: "some lodging of this trip is in Kyoto".
    /// Compiles to a correlated `EXISTS` over the relation's foreign key (or join table);
    /// `inner` evaluates against rows of `target`, not of the query's own table. Nesting is
    /// unlimited — each level is another subquery the engine plans.
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
    /// Raw SQL the layer cannot read. The query cannot know its dependencies, so every flush
    /// that touches the table re-runs it — the honest cost of the escape hatch.
    Raw(String, Vec<Value>),
    /// Full-text match (`#[model(fts(…))]`): compiles to a subquery over the FTS5 shadow.
    /// The dependency set is the indexed columns, so the zero-cost tier survives search.
    Matches {
        columns: &'static [&'static str],
        query: String,
    },
    /// A bounding-box test over two REAL columns. When the columns are the model's declared
    /// `spatial(…)` pair, the compiler narrows through the R*Tree shadow first and re-checks
    /// exactly (the shadow stores 32-bit floats, outward-rounded — a candidate superset).
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
    /// The columns whose change can move a result through this predicate — LOCAL columns
    /// only; what a predicate reads across a relation is collected by [`Pred::related_deps`].
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
            // local column write can never move a row through them. The membership itself
            // (a foreign key rewrite, a link) is routed by the relation's own machinery.
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

    /// Collect what this predicate reads ACROSS relations, at EVERY depth — the tables whose
    /// changes must mark a query stale. A nested crossing's `owner` is the enclosing target,
    /// recorded at construction, so each entry stands on its own.
    fn related_deps(&self, out: &mut Vec<RelatedDep>) {
        match self {
            Pred::Related {
                owner,
                field,
                target,
                inner,
                ..
            } => {
                let mut columns = Vec::new();
                inner.columns(&mut columns);
                out.push(RelatedDep {
                    owner,
                    field,
                    target_table: target,
                    columns,
                });
                inner.related_deps(out);
            }
            Pred::And(a, b) | Pred::Or(a, b) => {
                a.related_deps(out);
                b.related_deps(out);
            }
            Pred::Not(a) => a.related_deps(out),
            _ => {}
        }
    }

    /// Whether any part is RAW SQL — unreadable, so every flush of the table re-queries.
    pub fn contains_raw(&self) -> bool {
        match self {
            Pred::Raw(..) => true,
            Pred::And(a, b) | Pred::Or(a, b) => a.contains_raw() || b.contains_raw(),
            Pred::Not(a) => a.contains_raw(),
            _ => false,
        }
    }

    /// Whether any part folds case (`ContainsCi`/`StartsWithCi`) — those need the driver's
    /// `day_fold` function for exact SQL, or the fallback path.
    pub(crate) fn contains_fold(&self) -> bool {
        match self {
            Pred::ContainsCi(..) | Pred::StartsWithCi(..) => true,
            Pred::And(a, b) | Pred::Or(a, b) => a.contains_fold() || b.contains_fold(),
            Pred::Not(a) => a.contains_fold(),
            Pred::Related { inner, .. } => inner.contains_fold(),
            _ => false,
        }
    }

    /// Whether any part crosses a relation.
    pub(crate) fn contains_related(&self) -> bool {
        match self {
            Pred::Related { .. } => true,
            Pred::And(a, b) | Pred::Or(a, b) => a.contains_related() || b.contains_related(),
            Pred::Not(a) => a.contains_related(),
            _ => false,
        }
    }

    /// Does this row match? The WHERE-clause reading: UNKNOWN is not a match. The FALLBACK
    /// evaluator — the SQL compiler is the primary path; this answers for drivers without
    /// `day_fold` (over the row's own columns; relation crossings are refused upstream) and
    /// for unit tests. `Raw` and `Matches` answer true here: on the fallback path they have
    /// already filtered in SQL.
    pub fn eval(&self, key: u64, row: &dyn RowView) -> bool {
        self.eval3(key, row) == Some(true)
    }

    /// [`Pred::eval`], three-valued — SQL's own logic, which is the only way this path and
    /// the SQL path can agree about NULL.
    ///
    /// A comparison against a NULL column is UNKNOWN (`None`), not false: SQL's `notes <> 'x'`
    /// does not select rows whose `notes` is NULL, and neither does this. `Eq`/`Ne` against a
    /// `Null` literal keep their `IS NULL` / `IS NOT NULL` meaning and are always definite.
    pub fn eval3(&self, key: u64, row: &dyn RowView) -> Option<bool> {
        match self {
            Pred::Always => Some(true),
            // Already answered in SQL on every path that reaches an evaluator.
            Pred::Raw(..) | Pred::Matches { .. } => Some(true),
            // Refused before evaluation (the compiler either answers it in SQL or errors);
            // a constant keeps the match total without inventing an answer.
            Pred::Related { .. } => Some(true),

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

            // Kleene logic, so UNKNOWN propagates exactly as SQL propagates it.
            Pred::And(a, b) => match (a.eval3(key, row), b.eval3(key, row)) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            },
            Pred::Or(a, b) => match (a.eval3(key, row), b.eval3(key, row)) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            },
            Pred::Not(a) => a.eval3(key, row).map(|v| !v),

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

// ---------------------------------------------------------------------------
// The SQL compiler
// ---------------------------------------------------------------------------

/// One relation crossing, as SQL needs it — resolved by the container from its wired
/// relations when a fetch compiles.
#[derive(Clone, Debug)]
pub(crate) enum RelSql {
    /// The owner is the PARENT (a `Many` field): its children are the target rows whose
    /// foreign-key column names the owner's key.
    Children {
        target_key: String,
        fk_col: String,
        owner_key: String,
    },
    /// The owner is the CHILD (a `One` column): the target is the one row its foreign key
    /// names.
    Referent { target_key: String, fk_col: String },
    /// A many-to-many: membership lives in the join table, one column per side.
    Join {
        join_table: String,
        owner_col: String,
        target_col: String,
        owner_key: String,
        target_key: String,
    },
    /// A link by value (`#[model(link(…))]`): target rows whose `remote_col` equals the
    /// owner's `local_col` — no key on either side is involved, which is what lets it cross
    /// into an attached database.
    Linked {
        local_col: String,
        remote_col: String,
        target_key: String,
    },
}

/// What the compiler asks the container: how names resolve to SQL. Implemented over the wired
/// relations and attached tables.
pub(crate) trait SqlIndex {
    /// Resolve `owner.field` to its wired SQL shape; `None` fails the compile, loudly.
    fn relation(&self, owner: &str, field: &str) -> Option<RelSql>;
    /// The key column of `table`.
    fn key_of(&self, table: &str) -> Option<String>;
    /// The FTS5 shadow table of `table`, when the model declares `fts(…)`.
    fn fts_of(&self, table: &str) -> Option<String>;
    /// The R*Tree shadow of `table`, when `lat`/`lon` are its declared `spatial(…)` pair.
    fn geo_of(&self, table: &str, lat: &str, lon: &str) -> Option<String>;
    /// Whether the connection has `day_fold` registered (exact Unicode case folding in SQL).
    fn unicode_fold(&self) -> bool;
}

/// Why a fetch would not compile. `NeedsFold` routes to the fallback evaluator; everything
/// else is a wiring error the query surfaces.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CompileErr {
    /// A case-insensitive text predicate on a driver without `day_fold`.
    NeedsFold,
    /// A `ContainsCi`/`StartsWithCi` INSIDE a relation predicate on a driver without
    /// `day_fold` — the fallback evaluator cannot traverse relations, so this combination
    /// needs the function.
    FoldInsideRelation,
    /// `owner.field` is not a wired relation of this container.
    Unwired(&'static str, &'static str),
    /// A `matches(…)`/`rank()` fetch on a model with no `fts(…)` declaration.
    NoFts,
}

impl CompileErr {
    pub(crate) fn message(&self) -> String {
        match self {
            CompileErr::NeedsFold | CompileErr::FoldInsideRelation => {
                "case-insensitive predicate needs the driver's day_fold function".into()
            }
            CompileErr::Unwired(owner, field) => {
                format!("`{owner}.{field}` is not a wired relation of this container")
            }
            CompileErr::NoFts => "matches()/rank() needs a #[model(fts(…))] declaration".into(),
        }
    }
}

/// A compiled statement and its bound parameters.
#[derive(Clone, Debug)]
pub(crate) struct SqlQuery {
    pub sql: String,
    pub params: Vec<Value>,
}

/// Compile a whole fetch over `table` into the id `SELECT` that answers it: WHERE from the
/// predicate, ORDER BY from the sorts (with the key as the deterministic tie-break), LIMIT
/// from the window. `Err(NeedsFold)` sends the caller to the fallback path.
pub(crate) fn compile_fetch(
    table: &str,
    fetch: &Fetch,
    idx: &dyn SqlIndex,
) -> Result<SqlQuery, CompileErr> {
    let key = idx.key_of(table).ok_or(CompileErr::Unwired("", ""))?;
    let mut params = Vec::new();
    let mut aliases = 0usize;
    let by_rank = fetch.sort.iter().any(|s| s.by_rank);

    // A rank sort orders by the FTS index's own bm25 — the match query moves into a join so
    // `rank` is in scope, and the predicate's own `Matches` compiles to `1` (the join already
    // constrains to matching rows).
    let (from, rank_pred);
    if by_rank {
        let fts = idx.fts_of(table).ok_or(CompileErr::NoFts)?;
        // The hidden MATCH column carries the index's BARE name, schema-qualified or not.
        let fts_bare = fts.rsplit('.').next().unwrap_or(&fts).to_string();
        let Some(q) = find_match_query(&fetch.pred) else {
            return Err(CompileErr::NoFts);
        };
        params.push(Value::Text(q));
        from = format!(
            "{table} JOIN {fts} AS day_rank ON day_rank.rowid = {table}.{key} AND day_rank.{fts_bare} MATCH ?"
        );
        rank_pred = true;
    } else {
        from = table.to_string();
        rank_pred = false;
    }

    let ctx = SqlCtx {
        idx,
        skip_matches: rank_pred,
    };
    let where_clause = pred_sql(
        &fetch.pred,
        table,
        table,
        &key,
        &ctx,
        &mut params,
        &mut aliases,
    )?;

    let mut sql = format!("SELECT {table}.{key} FROM {from}");
    if where_clause != "1" {
        sql.push_str(&format!(" WHERE {where_clause}"));
    }
    let mut orders: Vec<String> = Vec::new();
    for s in &fetch.sort {
        if s.by_rank {
            orders.push("day_rank.rank".into());
        } else {
            orders.push(format!(
                "{table}.{} {}",
                s.column,
                if s.ascending { "ASC" } else { "DESC" }
            ));
        }
    }
    // The key tie-break makes the order total and deterministic, so the same fetch always
    // answers in the same order and a diff against the previous answer is meaningful.
    orders.push(format!("{table}.{key} ASC"));
    sql.push_str(&format!(" ORDER BY {}", orders.join(", ")));
    if let Some(n) = fetch.limit {
        sql.push_str(&format!(" LIMIT {n}"));
    }
    Ok(SqlQuery { sql, params })
}

/// Compile the COUNT form of a fetch: the same WHERE, no ORDER BY, no id vector — the
/// badge-shaped query. A `limit` caps the answer after the fact (`min(count, limit)` is what
/// a limited set's length would be), which the caller applies.
pub(crate) fn compile_count(
    table: &str,
    fetch: &Fetch,
    idx: &dyn SqlIndex,
) -> Result<SqlQuery, CompileErr> {
    let key = idx.key_of(table).ok_or(CompileErr::Unwired("", ""))?;
    let mut params = Vec::new();
    let mut aliases = 0usize;
    let ctx = SqlCtx {
        idx,
        skip_matches: false,
    };
    let where_clause = pred_sql(
        &fetch.pred,
        table,
        table,
        &key,
        &ctx,
        &mut params,
        &mut aliases,
    )?;
    let mut sql = format!("SELECT COUNT(*) FROM {table}");
    if where_clause != "1" {
        sql.push_str(&format!(" WHERE {where_clause}"));
    }
    Ok(SqlQuery { sql, params })
}

/// Compile the fallback form for a driver without `day_fold`: select the key AND every column
/// the predicate/sort reads, SQL-filter by the top-level AND conjuncts that compile exactly,
/// keep the ORDER BY (so the fallback preserves the query's order), and leave the LIMIT to
/// the caller — it applies after the in-memory re-check.
pub(crate) fn compile_fallback(
    table: &str,
    fetch: &Fetch,
    idx: &dyn SqlIndex,
) -> Result<(SqlQuery, Vec<&'static str>), CompileErr> {
    if fetch.pred.contains_related() && fetch.pred.contains_fold() {
        // The evaluator cannot traverse relations, and the SQL cannot fold — no path is
        // exact, so refuse rather than under- or over-answer.
        return Err(CompileErr::FoldInsideRelation);
    }
    let key = idx.key_of(table).ok_or(CompileErr::Unwired("", ""))?;
    let mut params = Vec::new();
    let mut aliases = 0usize;
    let ctx = SqlCtx {
        idx,
        skip_matches: false,
    };

    // The exact conjuncts filter in SQL (a candidate superset — dropping a conjunct can only
    // widen); the full predicate re-checks in memory over the selected columns.
    let mut conjuncts = Vec::new();
    split_and(&fetch.pred, &mut conjuncts);
    let mut clauses = Vec::new();
    for c in conjuncts {
        if !c.contains_fold() {
            clauses.push(pred_sql(
                c,
                table,
                table,
                &key,
                &ctx,
                &mut params,
                &mut aliases,
            )?);
        }
    }

    let mut deps = Vec::new();
    fetch.pred.columns(&mut deps);
    for s in &fetch.sort {
        if !s.by_rank && !deps.contains(&s.column) {
            deps.push(s.column);
        }
    }
    let cols = if deps.is_empty() {
        format!("{table}.{key}")
    } else {
        format!(
            "{table}.{key}, {}",
            deps.iter()
                .map(|c| format!("{table}.{c}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let mut sql = format!("SELECT {cols} FROM {table}");
    let clauses: Vec<String> = clauses.into_iter().filter(|c| c != "1").collect();
    if !clauses.is_empty() {
        sql.push_str(&format!(" WHERE {}", clauses.join(" AND ")));
    }
    let mut orders: Vec<String> = fetch
        .sort
        .iter()
        .filter(|s| !s.by_rank)
        .map(|s| {
            format!(
                "{table}.{} {}",
                s.column,
                if s.ascending { "ASC" } else { "DESC" }
            )
        })
        .collect();
    orders.push(format!("{table}.{key} ASC"));
    sql.push_str(&format!(" ORDER BY {}", orders.join(", ")));
    Ok((SqlQuery { sql, params }, deps))
}

/// Split a predicate into its top-level AND conjuncts.
fn split_and<'p>(pred: &'p Pred, out: &mut Vec<&'p Pred>) {
    match pred {
        Pred::And(a, b) => {
            split_and(a, out);
            split_and(b, out);
        }
        p => out.push(p),
    }
}

struct SqlCtx<'a> {
    idx: &'a dyn SqlIndex,
    /// A rank fetch already constrained to matching rows through its join; the `Matches`
    /// predicate inside compiles to `1` instead of a second subquery.
    skip_matches: bool,
}

/// The WHERE form of one predicate, columns qualified by `alias` (the current table or an
/// EXISTS alias), appending bound parameters to `params`. `table` is the alias's TRUE table
/// name — what FTS and R*Tree shadow lookups resolve against, since an EXISTS alias like
/// `day_r0` names no shadow.
fn pred_sql(
    pred: &Pred,
    table: &str,
    alias: &str,
    key: &str,
    ctx: &SqlCtx<'_>,
    params: &mut Vec<Value>,
    aliases: &mut usize,
) -> Result<String, CompileErr> {
    Ok(match pred {
        Pred::Always => "1".into(),
        Pred::Eq(c, Value::Null) => format!("{alias}.{c} IS NULL"),
        Pred::Ne(c, Value::Null) => format!("{alias}.{c} IS NOT NULL"),
        Pred::Eq(c, v) => {
            params.push(v.clone());
            format!("{alias}.{c} = ?")
        }
        Pred::Ne(c, v) => {
            params.push(v.clone());
            format!("{alias}.{c} <> ?")
        }
        Pred::Lt(c, v) => {
            params.push(v.clone());
            format!("{alias}.{c} < ?")
        }
        Pred::Le(c, v) => {
            params.push(v.clone());
            format!("{alias}.{c} <= ?")
        }
        Pred::Gt(c, v) => {
            params.push(v.clone());
            format!("{alias}.{c} > ?")
        }
        Pred::Ge(c, v) => {
            params.push(v.clone());
            format!("{alias}.{c} >= ?")
        }
        Pred::Contains(c, s) => {
            params.push(Value::Text(s.clone()));
            format!("instr({alias}.{c}, ?) > 0")
        }
        Pred::ContainsCi(c, s) => {
            if !ctx.idx.unicode_fold() {
                return Err(CompileErr::NeedsFold);
            }
            params.push(Value::Text(s.to_lowercase()));
            format!("instr(day_fold({alias}.{c}), ?) > 0")
        }
        Pred::StartsWith(c, prefix) => {
            // NOT `LIKE`: SQLite's LIKE is case-INsensitive for ASCII by default, which
            // would quietly answer a different question. `substr` counts characters on
            // TEXT, which agrees with Rust's `starts_with` on any valid UTF-8.
            params.push(Value::Int(prefix.chars().count() as i64));
            params.push(Value::Text(prefix.clone()));
            format!("substr({alias}.{c}, 1, ?) = ?")
        }
        Pred::StartsWithCi(c, prefix) => {
            if !ctx.idx.unicode_fold() {
                return Err(CompileErr::NeedsFold);
            }
            let folded = prefix.to_lowercase();
            params.push(Value::Int(folded.chars().count() as i64));
            params.push(Value::Text(folded));
            format!("substr(day_fold({alias}.{c}), 1, ?) = ?")
        }
        Pred::In(c, set) | Pred::NotIn(c, set) => {
            // The empty set compiles to its three-valued constant: `IN ()` matches nothing;
            // `NOT IN ()` matches every row whose column is PRESENT — a NULL column is
            // UNKNOWN, not vacuously a non-member, which is the same rule the evaluator
            // applies (SQLite's own literal `NOT IN ()` extension answers TRUE for NULL,
            // and would let the two paths disagree).
            if set.is_empty() {
                return Ok(if matches!(pred, Pred::In(..)) {
                    "0".into()
                } else {
                    format!("{alias}.{c} IS NOT NULL")
                });
            }
            let negate = if matches!(pred, Pred::NotIn(..)) {
                "NOT "
            } else {
                ""
            };
            let marks = vec!["?"; set.len()].join(", ");
            params.extend(set.iter().cloned());
            format!("{alias}.{c} {negate}IN ({marks})")
        }
        Pred::IdIn(ids) => {
            if ids.is_empty() {
                return Ok("0".into());
            }
            for id in ids {
                params.push(crate::key_param(*id));
            }
            let marks = vec!["?"; ids.len()].join(", ");
            format!("{alias}.{key} IN ({marks})")
        }
        Pred::Between(c, lo, hi) => {
            params.push(lo.clone());
            params.push(hi.clone());
            format!("{alias}.{c} BETWEEN ? AND ?")
        }
        Pred::And(a, b) => format!(
            "({} AND {})",
            pred_sql(a, table, alias, key, ctx, params, aliases)?,
            pred_sql(b, table, alias, key, ctx, params, aliases)?
        ),
        Pred::Or(a, b) => format!(
            "({} OR {})",
            pred_sql(a, table, alias, key, ctx, params, aliases)?,
            pred_sql(b, table, alias, key, ctx, params, aliases)?
        ),
        Pred::Not(a) => format!(
            "NOT ({})",
            pred_sql(a, table, alias, key, ctx, params, aliases)?
        ),
        Pred::Raw(sql, args) => {
            params.extend(args.iter().cloned());
            format!("({sql})")
        }
        Pred::Matches { query, .. } => {
            if ctx.skip_matches {
                // The rank join already constrains to matching rows.
                "1".into()
            } else {
                let fts = ctx.idx.fts_of(table).ok_or(CompileErr::NoFts)?;
                params.push(Value::Text(query.clone()));
                // The MATCH operand is the index's BARE name even when the table is
                // schema-qualified (an attached database): `catalog.t_fts MATCH` is a
                // syntax error, `t_fts MATCH` inside `FROM catalog.t_fts` is not.
                let fts_bare = fts.rsplit('.').next().unwrap_or(&fts);
                format!("{alias}.{key} IN (SELECT rowid FROM {fts} WHERE {fts_bare} MATCH ?)")
            }
        }
        Pred::Within {
            lat,
            lon,
            min_lat,
            max_lat,
            min_lon,
            max_lon,
        } => {
            let exact = {
                params.push(Value::Real(*min_lat));
                params.push(Value::Real(*max_lat));
                params.push(Value::Real(*min_lon));
                params.push(Value::Real(*max_lon));
                format!("({alias}.{lat} BETWEEN ? AND ? AND {alias}.{lon} BETWEEN ? AND ?)")
            };
            // The R*Tree shadow narrows first when this is the declared pair: its 32-bit
            // entries are outward-rounded (a candidate superset, never a miss), and the exact
            // check above settles the edges.
            match ctx.idx.geo_of(table, lat, lon) {
                Some(geo) => {
                    params.push(Value::Real(*min_lat));
                    params.push(Value::Real(*max_lat));
                    params.push(Value::Real(*min_lon));
                    params.push(Value::Real(*max_lon));
                    format!(
                        "({alias}.{key} IN (SELECT {key} FROM {geo} WHERE \
                         max_lat >= ? AND min_lat <= ? AND max_lon >= ? AND min_lon <= ?) \
                         AND {exact})"
                    )
                }
                None => exact,
            }
        }
        Pred::Related {
            owner,
            field,
            target,
            quant,
            inner,
        } => {
            let rel = ctx
                .idx
                .relation(owner, field)
                .ok_or(CompileErr::Unwired(owner, field))?;
            let r = format!("day_r{}", *aliases);
            *aliases += 1;
            let (from, tie, inner_alias, inner_key) = match &rel {
                RelSql::Children {
                    target_key,
                    fk_col,
                    owner_key,
                } => (
                    format!("{target} AS {r}"),
                    format!("{r}.{fk_col} = {alias}.{owner_key}"),
                    r.clone(),
                    target_key.clone(),
                ),
                RelSql::Referent { target_key, fk_col } => (
                    format!("{target} AS {r}"),
                    format!("{r}.{target_key} = {alias}.{fk_col}"),
                    r.clone(),
                    target_key.clone(),
                ),
                RelSql::Linked {
                    local_col,
                    remote_col,
                    target_key,
                } => (
                    format!("{target} AS {r}"),
                    format!("{r}.{remote_col} = {alias}.{local_col}"),
                    r.clone(),
                    target_key.clone(),
                ),
                RelSql::Join {
                    join_table,
                    owner_col,
                    target_col,
                    owner_key,
                    target_key,
                } => {
                    let j = format!("day_j{}", *aliases);
                    *aliases += 1;
                    (
                        format!(
                            "{join_table} AS {j} JOIN {target} AS {r} ON {r}.{target_key} = {j}.{target_col}"
                        ),
                        format!("{j}.{owner_col} = {alias}.{owner_key}"),
                        r.clone(),
                        target_key.clone(),
                    )
                }
            };
            match quant {
                Quant::Empty => format!("NOT EXISTS (SELECT 1 FROM {from} WHERE {tie})"),
                Quant::CountGe(n) => {
                    params.push(Value::Int(*n as i64));
                    format!("(SELECT COUNT(*) FROM {from} WHERE {tie}) >= ?")
                }
                Quant::Any | Quant::None | Quant::All => {
                    let inner_sql = pred_sql(
                        inner,
                        target,
                        &inner_alias,
                        &inner_key,
                        ctx,
                        params,
                        aliases,
                    )?;
                    match quant {
                        Quant::Any => {
                            format!("EXISTS (SELECT 1 FROM {from} WHERE {tie} AND ({inner_sql}))")
                        }
                        Quant::None => format!(
                            "NOT EXISTS (SELECT 1 FROM {from} WHERE {tie} AND ({inner_sql}))"
                        ),
                        // Every related row DEFINITELY matches — `IS TRUE` keeps a related
                        // row whose inner is UNKNOWN failing the quantifier, the reading
                        // `All` documents.
                        Quant::All => format!(
                            "NOT EXISTS (SELECT 1 FROM {from} WHERE {tie} AND ({inner_sql}) IS NOT TRUE)"
                        ),
                        _ => unreachable!(),
                    }
                }
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

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
/// difference: `Real` compares by `total_cmp`, so a NaN that reaches the fallback evaluator
/// still orders deterministically instead of poisoning it.
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

// ---------------------------------------------------------------------------
// Typed builders
// ---------------------------------------------------------------------------

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

    /// No related rows at all — one `NOT EXISTS` over the indexed foreign key.
    pub fn is_empty(self) -> Pred {
        self.build(Quant::Empty, Pred::Always)
    }

    /// At least `n` related rows.
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
    /// FTS5 MATCH — one subquery over the shadow table; the query re-runs when an INDEXED
    /// column changes and ignores every other column, so the zero-cost tier survives search.
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
    /// Range comparisons over the two columns, narrowed through the R*Tree shadow when the
    /// pair is the model's declared one.
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
    /// Case-SENSITIVE prefix match.
    pub fn starts_with(self, prefix: impl Into<String>) -> Pred {
        Pred::StartsWith(self.column, prefix.into())
    }

    /// Case-insensitive prefix match.
    pub fn starts_with_ci(self, prefix: impl Into<String>) -> Pred {
        Pred::StartsWithCi(self.column, prefix.into())
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
    /// Order by FTS relevance (bm25) instead of a column — `column` is empty; compiles to a
    /// join against the FTS shadow so the engine orders by its own rank.
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
        self.pred.related_deps(&mut related);
        Deps { local, related }
    }
}

/// What a change did to the result set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Delta {
    Insert(usize, u64),
    Remove(usize, u64),
    Move(usize, usize, u64),
}

/// What a fetch reads, and therefore what can move a row through it.
///
/// Split by table on purpose: a query's own columns are one question, and the columns it
/// reads across a relation are another — a change to a related row the predicate never
/// mentions must stay as free as a change to a local column it never mentions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Deps {
    /// Columns of the query's own table — its predicate's and its sort's.
    pub local: Vec<&'static str>,
    /// One entry per relation the predicate crosses, at ANY depth.
    pub related: Vec<RelatedDep>,
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

/// What adopting a fresh answer did to a result set.
#[derive(Clone, PartialEq, Debug)]
pub enum SetChange {
    /// Identical — nothing downstream needs waking.
    Same,
    /// Changed by exactly these deltas — enough to animate a list rather than reload it.
    /// Removals come first in DESCENDING index order, then insertions in ascending order
    /// (each index valid at its point of application); a pure reposition is one `Move`.
    Deltas(Vec<Delta>),
    /// Too different to narrate row by row; a reload is honest.
    Reload,
}

/// A query's result set: the ids the SQL last answered, and the diff machinery that turns the
/// next answer into list deltas.
pub struct ResultSet {
    ids: Vec<u64>,
    fetch: Fetch,
    deps: Deps,
}

impl ResultSet {
    pub fn new(fetch: Fetch) -> ResultSet {
        ResultSet {
            ids: Vec::new(),
            deps: fetch.dependencies(),
            fetch,
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

    /// Replace the whole set without narrating (the seed fetch).
    pub fn reset(&mut self, ids: Vec<u64>) {
        self.ids = ids;
    }

    /// Adopt a requery's answer, narrating the difference. Removals and insertions (in any
    /// combination) come back as exact deltas, plus at most ONE reposition among the retained
    /// rows; anything more tangled reloads. Every delta list is verified by simulation before
    /// it is returned, so a consumer applying the deltas in order always lands on the new set.
    pub fn adopt(&mut self, new: Vec<u64>) -> SetChange {
        if self.ids == new {
            return SetChange::Same;
        }
        let old = std::mem::replace(&mut self.ids, new);
        let new = &self.ids;

        let new_set: std::collections::HashSet<u64> = new.iter().copied().collect();
        let old_set: std::collections::HashSet<u64> = old.iter().copied().collect();
        let retained_old: Vec<u64> = old
            .iter()
            .copied()
            .filter(|k| new_set.contains(k))
            .collect();
        let retained_new: Vec<u64> = new
            .iter()
            .copied()
            .filter(|k| old_set.contains(k))
            .collect();

        let mut deltas: Vec<Delta> = Vec::new();
        // Removals from the END first, so each index is valid at its point of application.
        for (i, k) in old.iter().enumerate().rev() {
            if !new_set.contains(k) {
                deltas.push(Delta::Remove(i, *k));
            }
        }
        let mut scratch = retained_old.clone();

        if retained_old != retained_new {
            // The retained rows reordered. One reposition — the shape a sort-column edit
            // produces — narrates as a Move; more than one reloads. With one row out of
            // place, it is the first mismatched key of one side or the other.
            let mismatch = retained_old
                .iter()
                .copied()
                .zip(retained_new.iter().copied())
                .find(|(a, b)| a != b);
            let mut moved = false;
            if let Some((a, b)) = mismatch {
                for k in [a, b] {
                    let from = scratch.iter().position(|x| *x == k);
                    let to = retained_new.iter().position(|x| *x == k);
                    if let (Some(from), Some(to)) = (from, to) {
                        let mut trial = scratch.clone();
                        let key = trial.remove(from);
                        trial.insert(to, key);
                        if trial == retained_new {
                            deltas.push(Delta::Move(from, to, k));
                            scratch = trial;
                            moved = true;
                            break;
                        }
                    }
                }
            }
            if !moved {
                return SetChange::Reload;
            }
        }

        // Insertions in ascending final position, each valid as applied.
        for (i, k) in new.iter().enumerate() {
            if !old_set.contains(k) {
                deltas.push(Delta::Insert(i, *k));
                if i <= scratch.len() {
                    scratch.insert(i, *k);
                } else {
                    return SetChange::Reload;
                }
            }
        }

        // The proof: applying the narration lands exactly on the new set.
        if scratch == *new {
            SetChange::Deltas(deltas)
        } else {
            SetChange::Reload
        }
    }
}

pub(crate) fn find_match_query(pred: &Pred) -> Option<String> {
    match pred {
        Pred::Matches { query, .. } => Some(query.clone()),
        Pred::And(a, b) | Pred::Or(a, b) => find_match_query(a).or_else(|| find_match_query(b)),
        Pred::Not(a) => find_match_query(a),
        _ => None,
    }
}
