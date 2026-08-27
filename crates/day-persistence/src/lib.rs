// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! SQLite persistence for the observable model (docs/persistence.md).
//!
//! The engine owns the data; memory holds a WORKING SET. A [`ModelContainer`] opens a
//! database through a pluggable [`SqliteDriver`], creates or migrates each model's table,
//! and stops — no rows are read at open, so opening a million-row store costs the same as
//! opening an empty one. Rows enter memory by FAULTING: a typed [`ModelContainer::get`], a
//! batch [`ModelContainer::ensure_resident`], or a list binding materializing the rows it is
//! about to show. Each model's [`Store`] is that working set — an ordinary day-model store,
//! so every binding and accessor works on it unchanged — bounded by a per-table cache limit;
//! clean rows nothing observes leave silently and fault back on the next read.
//!
//! The write half is day-model's change log, folded. Any write requires its row resident
//! (editing is what faulted it in); at the end of any turn that touched a store (autosave,
//! the default) the accumulated changes fold into the smallest statement list that expresses
//! them — twenty keystrokes into one field is one `UPDATE`; a row inserted and then filled is
//! one `INSERT`; a thousand-child cascade is a handful of chunked `DELETE`s.
//!
//! Typed live queries ([`ModelContainer::query`]) compile ENTIRELY to SQL — predicate to
//! WHERE (relation crossings as correlated `EXISTS`, full-text as an FTS5 subquery, spatial
//! boxes through the R*Tree shadow), sort to ORDER BY with the key as tie-break, window to
//! LIMIT — so the engine's indexes answer them at any table size. Live maintenance is
//! dependency-gated: a change to a column no query mentions costs nothing; a change a query
//! does depend on marks it stale, and ONE requery after the turn's flush re-derives its id
//! set, diffed against the previous answer so a list animates the difference instead of
//! reloading. Rows behind the ids stay lazy: the query holds ids, and the list faults the
//! window it shows.
//!
//! The driver is a trait so the ENGINE is the app's choice: the built-in [`Sqlite`] driver
//! (feature `driver-rusqlite`, on by default) compiles a bundled SQLite, links the system one
//! (`system`), or builds SQLCipher (`cipher`); the [`Recorder`] answers from fixtures and
//! records every statement, which is what keeps persistence assertable headlessly — a test
//! can check the SQL a UI action produced without a database on disk.
//!
//! Another connection's committed writes arrive through [`ModelContainer::check_external`],
//! which re-reads only the RESIDENT rows and re-runs the live queries — O(working set), never
//! O(table). Row identity is the model's `#[model(id)]` key; display order is a projection
//! concern and is not persisted.

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use day_model::{Identified, Key, Keyed, ModelId, Op, Store};

mod queries;
pub use queries::{
    Col, Delta, Deps, Fetch, FtsRef, GeoRect, GeoRef, Pred, Quant, RelatedDep, RelationCol,
    ResultSet, RowView, SetChange, Sort, compare_values, encode_column, rank,
};
use queries::{CompileErr, RelSql, SqlIndex, compile_count, compile_fallback, compile_fetch};

mod relations;
pub use relations::{
    DeleteRule, Many, One, Registrar, RelationDef, RelationRef, wire_join, wire_to_many,
};

#[cfg(feature = "driver-rusqlite")]
mod rusqlite_driver;
#[cfg(feature = "driver-rusqlite")]
pub use rusqlite_driver::Sqlite;
#[cfg(all(
    feature = "driver-rusqlite",
    target_family = "wasm",
    target_os = "unknown"
))]
pub use rusqlite_driver::WebStorage;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why a persistence operation failed. `message` is for the log; `kind` is for the code path
/// that can do something about it.
#[derive(Clone, Debug)]
pub struct DbError {
    pub kind: DbErrorKind,
    pub message: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DbErrorKind {
    /// The engine said no — SQL error, I/O, constraint.
    Driver,
    /// The stored schema and the declared one disagree in a way lightweight migration cannot
    /// close (a type change, a rename) — the file is refused, never silently rewritten.
    Schema,
    /// The database would not open with the key it was given.
    BadKey,
    /// A stored value would not decode as the field's type.
    Decode,
    /// The driver cannot do this (encryption on a non-cipher build, …).
    Unsupported,
    /// A `DeleteRule::Deny` relation refused the delete — children still reference the row.
    Deny,
}

impl DbError {
    pub fn new(kind: DbErrorKind, message: impl Into<String>) -> Self {
        DbError {
            kind,
            message: message.into(),
        }
    }
    /// Engine-error shorthand for driver implementations (public: external drivers use it too).
    pub fn driver(message: impl Into<String>) -> Self {
        Self::new(DbErrorKind::Driver, message)
    }
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for DbError {}

// ---------------------------------------------------------------------------
// Values and rows
// ---------------------------------------------------------------------------

/// SQLite's five storage classes, and nothing else — no driver type reaches the layer above.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub fn as_int(&self) -> Result<i64, DbError> {
        match self {
            Value::Int(i) => Ok(*i),
            other => Err(DbError::new(
                DbErrorKind::Decode,
                format!("expected INTEGER, found {other:?}"),
            )),
        }
    }
    pub fn as_real(&self) -> Result<f64, DbError> {
        match self {
            Value::Real(r) => Ok(*r),
            Value::Int(i) => Ok(*i as f64),
            other => Err(DbError::new(
                DbErrorKind::Decode,
                format!("expected REAL, found {other:?}"),
            )),
        }
    }
    pub fn as_text(&self) -> Result<&str, DbError> {
        match self {
            Value::Text(t) => Ok(t),
            other => Err(DbError::new(
                DbErrorKind::Decode,
                format!("expected TEXT, found {other:?}"),
            )),
        }
    }
    pub fn as_blob(&self) -> Result<&[u8], DbError> {
        match self {
            Value::Blob(b) => Ok(b),
            other => Err(DbError::new(
                DbErrorKind::Decode,
                format!("expected BLOB, found {other:?}"),
            )),
        }
    }
}

/// A column's declared type — part of the schema fingerprint, so changing one is a migration.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SqlType {
    Integer,
    Real,
    Text,
    Blob,
}

impl SqlType {
    pub fn ddl(self) -> &'static str {
        match self {
            SqlType::Integer => "INTEGER",
            SqlType::Real => "REAL",
            SqlType::Text => "TEXT",
            SqlType::Blob => "BLOB",
        }
    }
}

/// One result row, by position.
pub trait Row {
    fn len(&self) -> usize;
    fn get(&self, i: usize) -> Value;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Row for Vec<Value> {
    fn len(&self) -> usize {
        Vec::len(self)
    }
    fn get(&self, i: usize) -> Value {
        self.as_slice().get(i).cloned().unwrap_or(Value::Null)
    }
}

// ---------------------------------------------------------------------------
// The driver seam
// ---------------------------------------------------------------------------

/// Where the database lives.
#[derive(Clone, Debug)]
pub enum Location {
    Memory,
    File(PathBuf),
}

/// A key for an encrypted database. Redacted in Debug and zeroed on drop — where it lives
/// between launches is the app's business, never this crate's.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(key: impl Into<String>) -> Self {
        Secret(key.into())
    }
    #[cfg(feature = "cipher")]
    pub(crate) fn reveal(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(…)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // Best-effort scrub; the allocation may have been moved by earlier clones.
        unsafe {
            // SAFETY: we own the String and overwrite only its initialized bytes in place.
            for b in self.0.as_bytes_mut() {
                *b = 0;
            }
        }
    }
}

/// How to open the database. Built by the driver's own constructors ([`Sqlite::at`] etc.);
/// public so alternative drivers speak the same vocabulary.
#[derive(Clone, Debug)]
pub struct OpenOptions {
    pub location: Location,
    /// SQLCipher key; drivers without encryption reject it loudly.
    pub key: Option<Secret>,
    /// Journal in WAL mode (files only; ignored in memory). Default true.
    pub wal: bool,
    /// Enforce foreign keys. Default true.
    pub foreign_keys: bool,
    pub busy_timeout_ms: u32,
    /// Upgrade a file written by an older SQLCipher generation (PRAGMA cipher_migrate).
    pub cipher_migrate: bool,
}

impl OpenOptions {
    pub fn new(location: Location) -> Self {
        OpenOptions {
            location,
            key: None,
            wal: true,
            foreign_keys: true,
            busy_timeout_ms: 5_000,
            cipher_migrate: false,
        }
    }
}

/// What a driver can and cannot do, so the container degrades honestly instead of silently.
#[derive(Clone, Copy, Debug, Default)]
pub struct Capabilities {
    /// Survives a relaunch (false for memory stores and the Recorder).
    pub durable: bool,
    pub encryption: bool,
    pub wal: bool,
    pub full_text_search: bool,
    pub rtree: bool,
    /// Another connection's committed writes are detectable (SQLite's `PRAGMA data_version`
    /// counter), so [`ModelContainer::check_external`] can merge them. The built-in driver
    /// claims it for file databases on native targets; memory databases (no second connection
    /// can reach one), the Recorder, and the web engine (its OPFS access is exclusive) do not.
    pub external_changes: bool,
    /// The connection has `day_fold` registered — Rust's full-Unicode `to_lowercase` as a
    /// scalar SQL function — so case-insensitive text predicates compile to EXACT SQL. The
    /// built-in native driver registers it at open; without it those predicates take the
    /// fallback path (SQL for everything else, the folding test re-checked in memory).
    pub unicode_fold: bool,
}

/// The engine seam. Object-safe on the connection side, so the container stores
/// `Box<dyn SqliteConnection>` and never names an engine type.
pub trait SqliteDriver {
    type Connection: SqliteConnection;
    fn open(self) -> Result<Self::Connection, DbError>;
    fn capabilities(&self) -> Capabilities;
}

impl SqliteConnection for Box<dyn SqliteConnection> {
    fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, DbError> {
        (**self).execute(sql, params)
    }
    fn query(
        &mut self,
        sql: &str,
        params: &[Value],
        row: &mut dyn FnMut(&dyn Row),
    ) -> Result<(), DbError> {
        (**self).query(sql, params, row)
    }
    fn execute_batch(&mut self, sql: &str) -> Result<(), DbError> {
        (**self).execute_batch(sql)
    }
    fn query_named(
        &mut self,
        sql: &str,
        params: &[Value],
        row: &mut dyn FnMut(&[String], &dyn Row),
    ) -> Result<(), DbError> {
        (**self).query_named(sql, params, row)
    }
}

/// One open connection. Everything above speaks plain SQL and [`Value`]s through it.
pub trait SqliteConnection: 'static {
    fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, DbError>;
    fn query(
        &mut self,
        sql: &str,
        params: &[Value],
        row: &mut dyn FnMut(&dyn Row),
    ) -> Result<(), DbError>;
    fn begin(&mut self) -> Result<(), DbError> {
        self.execute("BEGIN", &[]).map(|_| ())
    }
    fn commit(&mut self) -> Result<(), DbError> {
        self.execute("COMMIT", &[]).map(|_| ())
    }
    fn rollback(&mut self) -> Result<(), DbError> {
        self.execute("ROLLBACK", &[]).map(|_| ())
    }
    /// Several statements in one string, no parameters — a DDL script, a migration step
    /// (day-lite's storage speaks this). The default runs the string as ONE statement;
    /// drivers whose engine executes scripts override it (the built-in native driver does).
    fn execute_batch(&mut self, sql: &str) -> Result<(), DbError> {
        self.execute(sql, &[]).map(|_| ())
    }
    /// [`SqliteConnection::query`], with the result's column names alongside each row — for
    /// callers that surface rows as named objects (day-lite's JS bridge). A driver that
    /// cannot name columns refuses rather than guessing.
    fn query_named(
        &mut self,
        sql: &str,
        params: &[Value],
        row: &mut dyn FnMut(&[String], &dyn Row),
    ) -> Result<(), DbError> {
        let _ = (sql, params, row);
        Err(DbError::new(
            DbErrorKind::Unsupported,
            "this driver does not report column names",
        ))
    }
}

// ---------------------------------------------------------------------------
// The Recorder — the headless fake
// ---------------------------------------------------------------------------

/// A driver that answers from fixtures and records every statement — the piece that keeps
/// persistence assertable with no database on disk. `let (driver, log) = Recorder::new();`
///
/// Fixtures answer by TABLE, not by statement: any `SELECT … FROM <table>` serves every
/// fixture row registered for it, whatever the WHERE clause says. Faulting paths keep only
/// the keys they asked for, so `get` and `ensure_resident` behave; a QUERY against the
/// Recorder answers with all fixture keys — assert the SQL it recorded, and use
/// `Sqlite::memory()` where predicate results themselves are under test.
pub struct Recorder {
    state: Rc<RecorderState>,
}

struct RecorderState {
    log: RefCell<Vec<(String, Vec<Value>)>>,
    /// table name → rows served for a `SELECT … FROM <table>`.
    fixtures: RefCell<HashMap<String, Vec<Vec<Value>>>>,
}

/// The recorded statement log, shared with the test that holds it.
#[derive(Clone)]
pub struct RecorderLog {
    state: Rc<RecorderState>,
}

impl RecorderLog {
    /// Every statement so far, SQL text only.
    pub fn sql(&self) -> Vec<String> {
        self.state
            .log
            .borrow()
            .iter()
            .map(|(s, _)| s.clone())
            .collect()
    }
    /// Statements with their bound parameters.
    pub fn entries(&self) -> Vec<(String, Vec<Value>)> {
        self.state.log.borrow().clone()
    }
    pub fn clear(&self) {
        self.state.log.borrow_mut().clear();
    }
}

impl Recorder {
    pub fn new() -> (Recorder, RecorderLog) {
        let state = Rc::new(RecorderState {
            log: RefCell::new(Vec::new()),
            fixtures: RefCell::new(HashMap::new()),
        });
        (
            Recorder {
                state: state.clone(),
            },
            RecorderLog { state },
        )
    }

    /// Rows a table's `SELECT` will answer with.
    pub fn with_table(self, table: &str, rows: Vec<Vec<Value>>) -> Self {
        self.state.fixtures.borrow_mut().insert(table.into(), rows);
        self
    }
}

pub struct RecorderConn {
    state: Rc<RecorderState>,
}

impl SqliteDriver for Recorder {
    type Connection = RecorderConn;
    fn open(self) -> Result<RecorderConn, DbError> {
        Ok(RecorderConn { state: self.state })
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            durable: false,
            wal: false,
            // The fake answers everything: an FTS/spatial model must open against fixtures
            // (its reads just come back empty unless a fixture answers them), and folded
            // predicates compile as if day_fold existed — the SQL is recorded, never run.
            full_text_search: true,
            rtree: true,
            unicode_fold: true,
            ..Default::default()
        }
    }
}

impl SqliteConnection for RecorderConn {
    fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, DbError> {
        self.state
            .log
            .borrow_mut()
            .push((sql.to_string(), params.to_vec()));
        Ok(0)
    }
    fn query(
        &mut self,
        sql: &str,
        params: &[Value],
        row: &mut dyn FnMut(&dyn Row),
    ) -> Result<(), DbError> {
        self.state
            .log
            .borrow_mut()
            .push((sql.to_string(), params.to_vec()));
        // Serve a fixture when the statement reads FROM a table one was registered for.
        if let Some(from) = sql.split(" FROM ").nth(1) {
            let table = from.split_whitespace().next().unwrap_or("");
            if let Some(rows) = self.state.fixtures.borrow().get(table) {
                for r in rows {
                    row(r);
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Column values and codecs (docs/persistence.md)
// ---------------------------------------------------------------------------

/// A Rust type that knows how to be a column. One impl per type — its canonical form.
pub trait ColumnValue: Sized + 'static {
    /// The declared column type. Part of the schema fingerprint, so changing it is a
    /// migration, never a surprise.
    const SQL_TYPE: SqlType;
    fn to_sqlite_value(&self) -> Value;
    fn from_sqlite_value(v: Value) -> Result<Self, DbError>;
}

/// A NAMED alternative representation for `T` — serde's `#[serde(with = …)]` idiom.
/// Implement on a unit struct; select it per field with `#[model(with = …)]`.
pub trait ValueCodec<T>: 'static {
    const SQL_TYPE: SqlType;
    fn to_sqlite_value(v: &T) -> Value;
    fn from_sqlite_value(v: Value) -> Result<T, DbError>;
}

macro_rules! int_column {
    ($($t:ty),*) => {$(
        impl ColumnValue for $t {
            const SQL_TYPE: SqlType = SqlType::Integer;
            fn to_sqlite_value(&self) -> Value {
                Value::Int(*self as i64)
            }
            fn from_sqlite_value(v: Value) -> Result<Self, DbError> {
                Ok(v.as_int()? as $t)
            }
        }
    )*};
}
int_column!(i8, i16, i32, i64, u8, u16, u32, u64, usize, isize);

impl ColumnValue for bool {
    const SQL_TYPE: SqlType = SqlType::Integer;
    fn to_sqlite_value(&self) -> Value {
        Value::Int(*self as i64)
    }
    fn from_sqlite_value(v: Value) -> Result<Self, DbError> {
        Ok(v.as_int()? != 0)
    }
}

impl ColumnValue for f64 {
    const SQL_TYPE: SqlType = SqlType::Real;
    fn to_sqlite_value(&self) -> Value {
        Value::Real(*self)
    }
    fn from_sqlite_value(v: Value) -> Result<Self, DbError> {
        v.as_real()
    }
}

impl ColumnValue for f32 {
    const SQL_TYPE: SqlType = SqlType::Real;
    fn to_sqlite_value(&self) -> Value {
        Value::Real(*self as f64)
    }
    fn from_sqlite_value(v: Value) -> Result<Self, DbError> {
        Ok(v.as_real()? as f32)
    }
}

impl ColumnValue for String {
    const SQL_TYPE: SqlType = SqlType::Text;
    fn to_sqlite_value(&self) -> Value {
        Value::Text(self.clone())
    }
    fn from_sqlite_value(v: Value) -> Result<Self, DbError> {
        Ok(v.as_text()?.to_string())
    }
}

impl ColumnValue for Vec<u8> {
    const SQL_TYPE: SqlType = SqlType::Blob;
    fn to_sqlite_value(&self) -> Value {
        Value::Blob(self.clone())
    }
    fn from_sqlite_value(v: Value) -> Result<Self, DbError> {
        Ok(v.as_blob()?.to_vec())
    }
}

/// A Uuid is a 16-byte `BLOB` — compact, indexable, and readable by any tool that knows the
/// convention. (`day_model::Uuid` IS `uuid::Uuid`, so no second dependency edge exists.)
impl ColumnValue for day_model::Uuid {
    const SQL_TYPE: SqlType = SqlType::Blob;
    fn to_sqlite_value(&self) -> Value {
        Value::Blob(self.as_bytes().to_vec())
    }
    fn from_sqlite_value(v: Value) -> Result<Self, DbError> {
        day_model::Uuid::from_slice(v.as_blob()?)
            .map_err(|e| DbError::new(DbErrorKind::Decode, format!("uuid: {e}")))
    }
}

/// A key handle, as the bound parameter its stored form takes: `INTEGER` for integer keys,
/// a 16-byte `BLOB` for Uuid keys, `TEXT` for string keys. `Pair` keys (join rows) bind
/// through their own two-column clause, never through this.
fn key_param(handle: u64) -> Value {
    match Key::of_handle(handle) {
        Some(Key::U64(k)) => Value::Int(k as i64),
        Some(Key::Uuid(u)) => Value::Blob(day_model::Uuid::from_u128(u).as_bytes().to_vec()),
        Some(Key::Str(s)) => Value::Text(s.to_string()),
        Some(Key::Pair(..)) | None => Value::Null,
    }
}

/// A stored key value, back as its path handle. `None` for shapes no key takes (a negative
/// integer, a blob that is not 16 bytes) — the caller treats those rows as unaddressable.
fn value_to_handle(v: &Value) -> Option<u64> {
    match v {
        Value::Int(i) if *i >= 0 => Some(Key::U64(*i as u64).handle()),
        Value::Blob(b) if b.len() == 16 => day_model::Uuid::from_slice(b)
            .ok()
            .map(|u| Key::Uuid(u.as_u128()).handle()),
        Value::Text(t) => Some(Key::Str(t.as_str().into()).handle()),
        _ => None,
    }
}

/// `NULL` belongs to the framework: `Option` wraps any column type, encoding is called only on
/// present values, and a NULL decodes to `None` — an impl never sees `Null` and cannot
/// disagree about it.
impl<T: ColumnValue> ColumnValue for Option<T> {
    const SQL_TYPE: SqlType = T::SQL_TYPE;
    fn to_sqlite_value(&self) -> Value {
        match self {
            Some(v) => v.to_sqlite_value(),
            None => Value::Null,
        }
    }
    fn from_sqlite_value(v: Value) -> Result<Self, DbError> {
        match v {
            Value::Null => Ok(None),
            v => Ok(Some(T::from_sqlite_value(v)?)),
        }
    }
}

/// The serde codec: any `Serialize + Deserialize` type as a `TEXT` column SQLite's own
/// `json_*` functions can reach. `#[model(json)]` is sugar for `#[model(with = Json)]`.
pub struct Json;

impl<T> ValueCodec<T> for Json
where
    T: serde::Serialize + serde::de::DeserializeOwned + 'static,
{
    const SQL_TYPE: SqlType = SqlType::Text;
    fn to_sqlite_value(v: &T) -> Value {
        match serde_json::to_string(v) {
            Ok(s) => Value::Text(s),
            // Serialization of an in-memory value failing is a bug in the type's serde impl;
            // surface it as a NULL rather than a panic, and the decode side will report it.
            Err(_) => Value::Null,
        }
    }
    fn from_sqlite_value(v: Value) -> Result<T, DbError> {
        serde_json::from_str(v.as_text()?)
            .map_err(|e| DbError::new(DbErrorKind::Decode, format!("json: {e}")))
    }
}

// ---------------------------------------------------------------------------
// The Model trait — what #[derive(Model)] implements
// ---------------------------------------------------------------------------

/// One column of a model's table, as the derive declares it. `field` is the STRUCT field the
/// column stores — the change log speaks field names, the SQL speaks column names, and this is
/// where the two meet (they differ only under `#[model(column = "…")]`).
#[derive(Clone, Copy, Debug)]
pub struct ColumnDef {
    pub name: &'static str,
    pub field: &'static str,
    pub sql: SqlType,
    pub not_null: bool,
    pub unique: bool,
    pub indexed: bool,
}

/// The column pair a `#[model(spatial(lat = …, lon = …))]` declaration names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpatialCols {
    pub lat: &'static str,
    pub lon: &'static str,
}

/// A persistable model: an [`Identified`] observable struct with a schema half. Implemented by
/// `#[derive(Model)]`; the container consumes it.
pub trait Model: Identified + day_model::ApplyField + Clone + 'static {
    const TABLE: &'static str;
    /// The key column's name (the `#[model(id)]` field). Also the first entry of `COLUMNS`.
    const KEY: &'static str;
    const COLUMNS: &'static [ColumnDef];
    /// Composite indexes from struct-level `#[model(index("a", "b"))]`.
    const COMPOSITE_INDEXES: &'static [&'static [&'static str]] = &[];
    /// Full-text-indexed columns from struct-level `#[model(fts("a", "b"))]` — an
    /// external-content FTS5 shadow table plus sync triggers, generated at open.
    const FTS_COLUMNS: &'static [&'static str] = &[];
    /// The FTS5 tokenizer, from `#[model(fts(…, tokenize = "…"))]` — e.g.
    /// `"unicode61 remove_diacritics 2"` for diacritics-insensitive search. `None` keeps
    /// FTS5's default. Part of the schema fingerprint, so changing it rebuilds the shadow.
    const FTS_TOKENIZE: Option<&'static str> = None;
    /// The R*Tree pair from struct-level `#[model(spatial(lat = "…", lon = "…"))]`.
    const SPATIAL: Option<SpatialCols> = None;
    /// The key column's SQL shape — what a `One<Self>` foreign-key column stores.
    const KEY_SQL: SqlType = SqlType::Integer;
    /// Relations declared on this model's `Many` fields (`#[model(relation(…))]`).
    const RELATIONS: &'static [RelationDef] = &[];
    /// One [`Value`] per column, in `COLUMNS` order.
    fn to_row(&self) -> Vec<Value>;
    fn from_row(row: &dyn Row) -> Result<Self, DbError>;
    /// Each column's Rust-default value — what an added column backfills with.
    fn default_row() -> Vec<Value>;
    /// Wire this model's declared relations — generated; the default declares none.
    fn wire(reg: &mut Registrar<'_>) {
        let _ = reg;
    }
}

/// The declared schema's fingerprint: table, columns, types, flags, indexes. Equal fingerprints
/// open instantly; a difference migrates or refuses — never silently.
pub fn model_fingerprint<M: Model>() -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x1000_0000_01b3);
        }
    };
    eat(M::TABLE.as_bytes());
    eat(M::KEY.as_bytes());
    for c in M::COLUMNS {
        eat(c.name.as_bytes());
        eat(c.sql.ddl().as_bytes());
        eat(&[c.not_null as u8, c.unique as u8, c.indexed as u8]);
    }
    for idx in M::COMPOSITE_INDEXES {
        for c in *idx {
            eat(c.as_bytes());
        }
        eat(b"|");
    }
    for c in M::FTS_COLUMNS {
        eat(b"fts:");
        eat(c.as_bytes());
    }
    if let Some(t) = M::FTS_TOKENIZE {
        eat(b"tok:");
        eat(t.as_bytes());
    }
    if let Some(s) = M::SPATIAL {
        eat(b"geo:");
        eat(s.lat.as_bytes());
        eat(s.lon.as_bytes());
    }
    h
}

// ---------------------------------------------------------------------------
// Schema sets and migrations
// ---------------------------------------------------------------------------

/// The set of models a container manages — build with [`schema!`].
#[derive(Default)]
pub struct Schema {
    installers: Vec<Installer>,
    /// Run after every table is attached — relation wiring needs both ends present.
    wirers: Vec<Installer>,
    /// Foreign-key clauses each declared relation contributes to its TARGET's table.
    fk_specs: Vec<FkSpec>,
}

type Installer = Box<dyn FnOnce(&ModelContainer) -> Result<(), DbError>>;
/// Decode raw rows into the cache silently, answering their handles.
type AbsorbFn = Rc<dyn Fn(Vec<Vec<Value>>) -> Result<Vec<u64>, DbError>>;
/// Diff raw rows (selected by the resident keys) against the cache: per-field announcements
/// authored [`ModelContainer::EXTERNAL_AUTHOR`], resident rows missing from the input deleted
/// the same way. Returns whether anything differed.
type RefreshFn = Rc<dyn Fn(Vec<Vec<Value>>) -> Result<bool, DbError>>;
/// A WHERE clause plus its parameters, addressing one row by handle.
type KeyWhere = Rc<dyn Fn(u64) -> (String, Vec<Value>)>;
/// A WHERE clause plus its parameters, addressing a SET of rows by handle.
type KeysWhere = Rc<dyn Fn(&[u64]) -> (String, Vec<Value>)>;
/// Evict up to N clean rows from an ordered candidate list, answering how many left.
type EvictFn = Rc<dyn Fn(&[u64], usize) -> usize>;

/// One foreign-key clause, resolved from a `RelationDef` at `Schema::with` time — the child's
/// column carries `REFERENCES parent(key) ON DELETE …` in its generated DDL.
#[derive(Clone, Copy, Debug)]
struct FkSpec {
    child_table: &'static str,
    child_field: &'static str,
    parent_table: &'static str,
    parent_key: &'static str,
    rule: DeleteRule,
}

impl Schema {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with<M: Model>(mut self) -> Self {
        for def in M::RELATIONS {
            if def.join.is_none() {
                self.fk_specs.push(FkSpec {
                    child_table: def.target_table,
                    child_field: def.inverse,
                    parent_table: M::TABLE,
                    parent_key: M::KEY,
                    rule: def.delete,
                });
            }
        }
        self.installers.push(Box::new(|c| c.attach::<M>()));
        self.wirers.push(Box::new(|c| {
            let mut reg = Registrar {
                container: c,
                error: None,
            };
            M::wire(&mut reg);
            match reg.error {
                Some(e) => Err(e),
                None => Ok(()),
            }
        }));
        self
    }
}

/// `schema![Trip, Lodging]` — the models a container manages.
#[macro_export]
macro_rules! schema {
    ($($m:ty),+ $(,)?) => {{
        let s = $crate::Schema::new();
        $(let s = s.with::<$m>();)+
        s
    }};
}

/// Staged migrations: anything with intent behind it (a rename, a split) gets a stage the app
/// writes; each stage runs in its own transaction against the raw connection, and versions only
/// go up. Lightweight migration (added/dropped columns and indexes) runs afterward, inferred
/// from the schema fingerprint.
#[derive(Default)]
pub struct MigrationPlan {
    stages: Vec<Stage>,
}

type StageFn = Box<dyn FnOnce(&mut dyn SqliteConnection) -> Result<(), DbError>>;

struct Stage {
    from: u32,
    to: u32,
    run: StageFn,
}

impl MigrationPlan {
    pub fn new() -> Self {
        Self::default()
    }
    /// A hand-written stage taking the database from `from` to `to`. Stages run in ascending
    /// order from the file's current version; a gap is an error at open.
    pub fn custom(
        mut self,
        from: u32,
        to: u32,
        f: impl FnOnce(&mut dyn SqliteConnection) -> Result<(), DbError> + 'static,
    ) -> Self {
        self.stages.push(Stage {
            from,
            to,
            run: Box::new(f),
        });
        self
    }
    fn target_version(&self) -> u32 {
        self.stages.iter().map(|s| s.to).max().unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Dirty tracking — the fold, applied live
// ---------------------------------------------------------------------------

/// What one row needs at the next flush. The merge rules are the change-log fold: same-row
/// changes coalesce, an INSERT absorbs later field writes, a DELETE absorbs everything.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DirtyRow {
    Insert,
    Update(Vec<&'static str>),
    Delete,
}

#[derive(Default)]
struct DirtyState {
    /// (store, key) → pending statement kind, in first-touch order.
    rows: HashMap<(u64, u64), DirtyRow>,
    order: Vec<(u64, u64)>,
    /// Stores whose WHOLE value was rewritten (a wholesale `Store::update`) — flushed by
    /// upserting every RESIDENT row. With a lazy cache no "delete the rest" is possible (the
    /// rest was never loaded), so wholesale rewrites are a resync of the working set only.
    full: Vec<u64>,
}

impl DirtyState {
    fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.full.is_empty()
    }

    fn note(&mut self, change: &day_model::Change, known_store: impl Fn(u64) -> bool) {
        let Some(&store) = change.components.first() else {
            return;
        };
        if !known_store(store) {
            return;
        }
        let Some(&key) = change.components.get(1) else {
            // A store-level change: the whole value may have been replaced.
            if !self.full.contains(&store) {
                self.full.push(store);
            }
            return;
        };
        if key == day_model::STRUCTURE {
            return; // the shape path is for the UI; the row paths carry the work
        }
        let id = (store, key);
        if !self.rows.contains_key(&id) {
            self.order.push(id);
        }
        let slot = self.rows.entry(id).or_insert(DirtyRow::Update(Vec::new()));
        match (change.op, &mut *slot) {
            (Op::Delete, _) => *slot = DirtyRow::Delete,
            (Op::Insert, _) => *slot = DirtyRow::Insert,
            (Op::Set, DirtyRow::Insert) | (Op::Set, DirtyRow::Delete) => {}
            (Op::Set, DirtyRow::Update(columns)) => {
                if change.components.len() >= 3 && !columns.contains(&change.label) {
                    columns.push(change.label);
                }
            }
            (Op::Move, _) => {}
        }
    }
}

// ---------------------------------------------------------------------------
// The container
// ---------------------------------------------------------------------------

/// Everything the container knows how to do for one attached model type, monomorphized at
/// [`Schema::with`] and stored type-erased. Join tables (relations.rs) hand-build one over
/// their pair-keyed membership cache, which is why keys are plural and clause-shaped here.
pub(crate) struct TableHooks {
    pub(crate) table: &'static str,
    /// The key column(s), in order: the key SELECT list, the upsert's conflict target, and
    /// what its DO UPDATE leaves alone. One entry everywhere but join tables.
    pub(crate) key_cols: Vec<String>,
    /// WHERE clause + params addressing one row by its key handle.
    pub(crate) key_where: KeyWhere,
    pub(crate) columns: Vec<String>,
    /// Same order as `columns`; what a change's label matches against.
    pub(crate) fields: Vec<String>,
    /// The FTS5 shadow's table name, when the model declares `fts(…)`.
    pub(crate) fts: Option<String>,
    /// The R*Tree shadow: (lat column, lon column, shadow table name).
    pub(crate) spatial: Option<(String, String, String)>,
    /// Current row values by key, read from the CACHE at flush time — the change log carries
    /// WHICH rows and columns moved, never their contents.
    pub(crate) row_for: Rc<dyn Fn(u64) -> Option<Vec<Value>>>,
    /// Every (key, row) currently resident, for wholesale resyncs.
    pub(crate) all_rows: Rc<dyn Fn() -> Vec<(u64, Vec<Value>)>>,
    /// The resident keys, in cache (≈ fault) order — what eviction walks.
    pub(crate) resident_keys: Rc<dyn Fn() -> Vec<u64>>,
    /// The resident COUNT, O(1) — the fault path's eviction gate.
    pub(crate) resident_len: Rc<dyn Fn() -> usize>,
    pub(crate) is_resident: Rc<dyn Fn(u64) -> bool>,
    /// Decode raw rows into the cache silently (a fault landing).
    pub(crate) absorb: AbsorbFn,
    /// Diff raw rows against the resident set, announcing as the external author.
    pub(crate) refresh: RefreshFn,
    /// Silently drop up to `want` CLEAN rows from the cache, taking candidates in order and
    /// skipping anything observed; answers how many left. One retain over the store, so an
    /// eviction pass costs O(cache), not O(cache × evicted). (Dirtiness is the container's
    /// knowledge — checked before this is called.)
    pub(crate) evict: EvictFn,
    /// Bring this store under an undo history — captured here because the model TYPE is known
    /// only at attach time.
    pub(crate) watch_undo: Rc<dyn Fn(&day_model::UndoStack)>,
}

pub(crate) struct ContainerInner {
    conn: RefCell<Box<dyn SqliteConnection>>,
    caps: Capabilities,
    /// store root id → hooks.
    tables: RefCell<HashMap<u64, TableHooks>>,
    /// model TypeId → the `Store<Keyed<M>>` handle, boxed.
    stores: RefCell<HashMap<TypeId, Box<dyn Any>>>,
    dirty: RefCell<DirtyState>,
    /// Live query result sets, marked stale from the change sink. Weak: the app's `Query`
    /// handles own the state; dead entries are pruned on dispatch.
    queries: RefCell<Vec<std::rc::Weak<QueryState>>>,
    /// The engine's cross-connection change counter as of the last look (`PRAGMA
    /// data_version` — it moves only when ANOTHER connection commits to the file). `None`
    /// where the driver reports no external-change detection.
    data_version: Cell<Option<i64>>,
    /// Foreign-key clauses relations contribute to their targets' tables, set before attach.
    fk_specs: RefCell<Vec<FkSpec>>,
    /// The wired relations — maintained from the change sink, read by `RelationRef`s.
    pub(crate) relations: RefCell<Vec<Rc<relations::ToOneRel>>>,
    /// The wired many-to-manys, each over its own membership cache.
    pub(crate) joins: RefCell<Vec<Rc<relations::JoinRel>>>,
    sink: Cell<Option<day_model::ChangeSinkId>>,
    autosave: Cell<bool>,
    /// Soft per-table bound on resident rows. Dirty and observed rows never evict, so the
    /// working set can exceed it; everything else does not.
    cache_limit: Cell<usize>,
    /// The last autosave failure, observable by the UI (`when(container.last_error()…)`).
    pub(crate) error: day_reactive::Signal<Option<String>>,
    /// Guards re-entrant flushes (a turn-end firing during an explicit save).
    flushing: Cell<bool>,
}

/// An open database and the working-set caches over it. Clone is shallow — clones share the
/// connection and the dirty state.
#[derive(Clone)]
pub struct ModelContainer {
    pub(crate) inner: Rc<ContainerInner>,
}

/// The default per-table bound on resident rows ([`ModelContainer::set_cache_limit`]).
pub const DEFAULT_CACHE_LIMIT: usize = 8_192;

impl ModelContainer {
    /// Open through `driver`, migrate, and attach every model in `schema` — attaching creates
    /// or migrates the table and NOTHING more: no rows load, so open cost does not grow with
    /// the file. Autosave is on: any turn that touched a store flushes at its end.
    pub fn open<D: SqliteDriver>(driver: D, schema: Schema) -> Result<ModelContainer, DbError>
    where
        D::Connection: 'static,
    {
        Self::open_with(driver, schema, MigrationPlan::new())
    }

    /// [`ModelContainer::open`], with hand-written migration stages that run first.
    pub fn open_with<D: SqliteDriver>(
        driver: D,
        schema: Schema,
        plan: MigrationPlan,
    ) -> Result<ModelContainer, DbError>
    where
        D::Connection: 'static,
    {
        let caps = driver.capabilities();
        let conn: Box<dyn SqliteConnection> = Box::new(driver.open()?);
        let error = day_reactive::Scope::detached().enter(|| day_reactive::Signal::new(None));
        let container = ModelContainer {
            inner: Rc::new(ContainerInner {
                conn: RefCell::new(conn),
                caps,
                tables: RefCell::new(HashMap::new()),
                stores: RefCell::new(HashMap::new()),
                dirty: RefCell::new(DirtyState::default()),
                queries: RefCell::new(Vec::new()),
                data_version: Cell::new(None),
                fk_specs: RefCell::new(Vec::new()),
                relations: RefCell::new(Vec::new()),
                joins: RefCell::new(Vec::new()),
                sink: Cell::new(None),
                autosave: Cell::new(true),
                cache_limit: Cell::new(DEFAULT_CACHE_LIMIT),
                error,
                flushing: Cell::new(false),
            }),
        };

        container.ensure_schema_table()?;
        container.run_stages(plan)?;
        let Schema {
            installers,
            wirers,
            fk_specs,
        } = schema;
        *container.inner.fk_specs.borrow_mut() = fk_specs;
        for install in installers {
            install(&container)?;
        }
        // Relations wire once every table is attached — both ends exist — and only then does
        // the sink go live.
        for wire in wirers {
            wire(&container)?;
        }
        relations::register_container(&container.inner);
        container.install_sink();
        container.install_autosave();
        if caps.external_changes {
            container
                .inner
                .data_version
                .set(container.file_data_version()?);
        }
        Ok(container)
    }

    /// The author tag on changes [`ModelContainer::check_external`] merges in — the database's
    /// own contents arriving, distinguishable by every change consumer and never persisted
    /// back. Reserved: an app writing stores under this tag would have its writes dropped by
    /// the autosave fold.
    pub const EXTERNAL_AUTHOR: &'static str = "database";

    /// The WORKING-SET cache for `M` — an ordinary day-model store holding the rows currently
    /// resident, NOT the table: its `keys()` are whatever happens to be faulted in. Bindings
    /// and accessors work on it unchanged; enumerate rows through a [`ModelContainer::query`],
    /// never through the cache. Panics only if `M` was not in the container's `schema!`,
    /// which is a wiring bug worth stopping on.
    pub fn cache<M: Model>(&self) -> Store<Keyed<M>> {
        let stores = self.inner.stores.borrow();
        let any = stores
            .get(&TypeId::of::<M>())
            .unwrap_or_else(|| panic!("model `{}` is not in this container's schema!", M::TABLE));
        *any.downcast_ref::<Store<Keyed<M>>>()
            .expect("store map holds the exact type it was keyed by")
    }

    /// What the driver can do — the container's honesty surface.
    pub fn capabilities(&self) -> Capabilities {
        self.inner.caps
    }

    /// Autosave on/off (default on). Off, changes accumulate until [`ModelContainer::save`] —
    /// and live queries answer from the last save, since only the file can answer them.
    pub fn set_autosave(&self, on: bool) {
        self.inner.autosave.set(on);
    }

    /// The soft per-table bound on resident rows (default [`DEFAULT_CACHE_LIMIT`]). Dirty
    /// rows and rows something observes never evict; beyond that, faulting past the limit
    /// releases the oldest resident rows. `usize::MAX` disables eviction. Lowering the limit
    /// enforces it immediately.
    pub fn set_cache_limit(&self, rows: usize) {
        self.inner.cache_limit.set(rows.max(1));
        let store_ids: Vec<u64> = self.inner.tables.borrow().keys().copied().collect();
        for id in store_ids {
            self.enforce_cache_limit(id, &[], true);
        }
    }

    /// The last autosave failure, if any — a tracked signal the UI can watch.
    pub fn last_error(&self) -> day_reactive::Signal<Option<String>> {
        self.inner.error
    }

    /// Flush every pending change now, in one transaction. Autosave calls this at turn end;
    /// call it directly where the error matters at a known point.
    pub fn save(&self) -> Result<(), DbError> {
        self.save_now().map(|_| ())
    }

    /// Run `f` and return the SQL one flush of everything it changed issues — the headless
    /// persistence assert (autosave is suspended for the duration so the statements land
    /// here). Twenty keystrokes into one field come back as one `UPDATE`.
    pub fn record_sql(&self, f: impl FnOnce()) -> Result<Vec<String>, DbError> {
        let prior = self.inner.autosave.replace(false);
        f();
        let result = self.save_now();
        self.inner.autosave.set(prior);
        result.map(|stmts| stmts.into_iter().map(|(sql, _)| sql).collect())
    }

    fn save_now(&self) -> Result<Vec<(String, Vec<Value>)>, DbError> {
        if self.inner.flushing.get() {
            return Ok(Vec::new());
        }
        let dirty = {
            let mut d = self.inner.dirty.borrow_mut();
            if d.is_empty() {
                drop(d);
                // Nothing to flush — but a stale query may still be waiting on its deferred
                // requery (a fetch swap on a clean container lands here).
                self.run_deferred_requeries(&[]);
                return Ok(Vec::new());
            }
            std::mem::take(&mut *d)
        };
        self.inner.flushing.set(true);
        let result = self.flush(dirty);
        self.inner.flushing.set(false);
        match &result {
            Ok(stmts) => {
                self.inner.error.set_if_changed(None);
                self.run_deferred_requeries(stmts);
            }
            Err(e) => self.inner.error.set(Some(e.to_string())),
        }
        result
    }

    // --- faulting ----------------------------------------------------------------------------

    /// One row, resident — faulted from the file if it was not. `None` when no such row
    /// exists (or a driver error surfaced, observable through [`ModelContainer::last_error`]).
    pub fn get<M: Model>(&self, id: impl Into<ModelId<M>>) -> Option<day_model::Elem<M>> {
        let store = self.cache::<M>();
        let h = id.into().handle();
        if !store.with_untracked(|k| k.get(h).is_some())
            && let Err(e) = self.ensure_resident::<M>(&[h])
        {
            self.inner.error.set(Some(e.to_string()));
            return None;
        }
        store
            .with_untracked(|k| k.get(h).is_some())
            .then(|| store.elem(h))
    }

    /// Make these rows resident, faulting the missing ones in ONE chunked `SELECT`. Rows the
    /// file does not have are simply not resident afterwards; rows deleted this turn are not
    /// resurrected.
    pub fn ensure_resident<M: Model>(&self, keys: &[u64]) -> Result<(), DbError> {
        let store = self.cache::<M>();
        self.fault_into(store.store_id(), keys)
    }

    /// [`ModelContainer::ensure_resident`], typed ids in.
    pub fn ensure<M: Model>(
        &self,
        ids: impl IntoIterator<Item = impl Into<ModelId<M>>>,
    ) -> Result<(), DbError> {
        let keys: Vec<u64> = ids.into_iter().map(|i| i.into().handle()).collect();
        self.ensure_resident::<M>(&keys)
    }

    /// Fault EVERY row of `M`'s table in — the document pattern, said explicitly: a sketch's
    /// scene or a settings table IS the working set, and the app that draws all of it warms
    /// it once at open. Raise the cache limit first (`set_cache_limit(usize::MAX)` for a
    /// document container) so the warmed rows are not immediately eligible to leave; rows
    /// deleted this turn stay deleted. Returns how many rows are resident afterward.
    pub fn warm<M: Model>(&self) -> Result<usize, DbError> {
        let store = self.cache::<M>();
        let keys = self.select_id_column_checked(&format!(
            "SELECT {} FROM {} ORDER BY {}",
            M::KEY,
            M::TABLE,
            M::KEY
        ))?;
        self.fault_into(store.store_id(), &keys)?;
        Ok(store.with_untracked(|k| k.len()))
    }

    /// Insert one row through the front door: it becomes resident and dirty, announces, and
    /// folds to one `INSERT` at the next flush.
    pub fn insert<M: Model>(&self, row: M) {
        let store = self.cache::<M>();
        let h = row.handle();
        store.restructure("create", Op::Insert, h, move |k| k.push(row));
    }

    /// `SELECT COUNT(*)` — the table's true size, which the cache cannot know. Settles
    /// pending writes first (with autosave), like every other read.
    pub fn table_count<M: Model>(&self) -> Result<u64, DbError> {
        if self.inner.autosave.get() && !self.inner.dirty.borrow().is_empty() {
            self.save()?;
        }
        let mut n = 0i64;
        self.conn().query(
            &format!("SELECT COUNT(*) FROM {}", M::TABLE),
            &[],
            &mut |row| {
                n = row.get(0).as_int().unwrap_or(0);
            },
        )?;
        Ok(n.max(0) as u64)
    }

    /// The untyped fault: bring `keys` into `store_id`'s cache.
    pub(crate) fn fault_into(&self, store_id: u64, keys: &[u64]) -> Result<(), DbError> {
        let (is_resident, absorb, columns, table, keys_where) = {
            let tables = self.inner.tables.borrow();
            let Some(hooks) = tables.get(&store_id) else {
                return Ok(());
            };
            (
                hooks.is_resident.clone(),
                hooks.absorb.clone(),
                hooks.columns.join(", "),
                hooks.table,
                keys_where_of(hooks),
            )
        };
        let missing: Vec<u64> = {
            let dirty = self.inner.dirty.borrow();
            keys.iter()
                .copied()
                .filter(|k| !is_resident(*k))
                // A row deleted this turn still exists in the file until the flush; faulting
                // it back would resurrect it.
                .filter(|k| dirty.rows.get(&(store_id, *k)) != Some(&DirtyRow::Delete))
                .collect()
        };
        if missing.is_empty() {
            return Ok(());
        }
        let mut faulted: Vec<u64> = Vec::with_capacity(missing.len());
        for chunk in missing.chunks(MAX_BOUND_PARAMS / 2) {
            let (clause, params) = keys_where(chunk);
            let mut rows: Vec<Vec<Value>> = Vec::new();
            self.conn().query(
                &format!("SELECT {columns} FROM {table} WHERE {clause}"),
                &params,
                &mut |row| {
                    let n = Row::len(row);
                    rows.push((0..n).map(|i| row.get(i)).collect());
                },
            )?;
            faulted.extend(absorb(rows)?);
        }
        // The Recorder serves whole fixture tables whatever the WHERE says; keep only what
        // was asked for so faulting stays precise there too. (By SET — a linear scan here
        // made warming half a million rows quadratic.)
        let wanted: std::collections::HashSet<u64> = missing.iter().copied().collect();
        faulted.retain(|k| wanted.contains(k));
        self.enforce_cache_limit(store_id, &faulted, false);
        Ok(())
    }

    /// Release resident rows beyond the cache limit — oldest first, never a dirty row, never
    /// an observed one, never one just faulted for the caller still holding it.
    ///
    /// The pass walks every resident key, so it runs with HYSTERESIS on the fault path:
    /// nothing happens until the cache overshoots the limit by a slack margin, and one pass
    /// then brings it back to the limit — amortizing the walk over the faults that filled the
    /// slack, instead of paying O(resident) per fault batch. (Measured: a 50k-row relation
    /// traversal over a 500k-row table spent 99% of its time in the un-amortized walk.)
    /// `strict` skips the slack — `set_cache_limit` enforces its new bound immediately.
    fn enforce_cache_limit(&self, store_id: u64, protect: &[u64], strict: bool) {
        let limit = self.inner.cache_limit.get();
        if limit == usize::MAX {
            return;
        }
        let (resident_len, evict) = {
            let tables = self.inner.tables.borrow();
            let Some(hooks) = tables.get(&store_id) else {
                return;
            };
            ((hooks.resident_len)(), hooks.evict.clone())
        };
        let slack = if strict {
            0
        } else {
            (limit / 8).clamp(64, 4096)
        };
        if resident_len <= limit + slack {
            return;
        }
        let resident = {
            let tables = self.inner.tables.borrow();
            match tables.get(&store_id) {
                Some(hooks) => (hooks.resident_keys)(),
                None => return,
            }
        };
        let excess = resident.len().saturating_sub(limit);
        let candidates: Vec<u64> = {
            let dirty = self.inner.dirty.borrow();
            if dirty.full.contains(&store_id) {
                return; // a wholesale rewrite is pending; every row is implicitly dirty
            }
            let protect: std::collections::HashSet<u64> = protect.iter().copied().collect();
            resident
                .into_iter()
                .filter(|key| !protect.contains(key) && !dirty.rows.contains_key(&(store_id, *key)))
                .collect()
        };
        evict(&candidates, excess);
    }

    // --- internals ---------------------------------------------------------------------------

    pub(crate) fn conn(&self) -> std::cell::RefMut<'_, Box<dyn SqliteConnection>> {
        self.inner.conn.borrow_mut()
    }

    /// Run a one-column SELECT and hand back the values as key handles. Errors surface on the
    /// error signal (these run on read paths with no caller to give a `Result` to).
    /// One-column key SELECT with the error returned — for callers that have a `Result` to
    /// give it to (the read paths use [`ModelContainer::select_id_column`] instead).
    fn select_id_column_checked(&self, sql: &str) -> Result<Vec<u64>, DbError> {
        let mut ids = Vec::new();
        self.conn().query(sql, &[], &mut |row| {
            if let Some(h) = value_to_handle(&row.get(0)) {
                ids.push(h);
            }
        })?;
        Ok(ids)
    }

    pub(crate) fn select_id_column(&self, sql: &str, params: &[Value]) -> Vec<u64> {
        let mut ids = Vec::new();
        if let Err(e) = self.conn().query(sql, params, &mut |row| {
            if let Some(h) = value_to_handle(&row.get(0)) {
                ids.push(h);
            }
        }) {
            self.inner.error.set(Some(e.to_string()));
        }
        ids
    }

    /// Run a one-column SELECT for a single REAL.
    pub(crate) fn select_real(&self, sql: &str, params: &[Value]) -> Option<f64> {
        let mut v = None;
        if let Err(e) = self.conn().query(sql, params, &mut |row| {
            v = row.get(0).as_real().ok();
        }) {
            self.inner.error.set(Some(e.to_string()));
        }
        v
    }

    /// This turn's unflushed rows of one store — what relation reads overlay.
    pub(crate) fn dirty_rows_of(&self, store_id: u64) -> Vec<(u64, DirtyRow)> {
        self.inner
            .dirty
            .borrow()
            .rows
            .iter()
            .filter(|((s, _), _)| *s == store_id)
            .map(|((_, k), state)| (*k, state.clone()))
            .collect()
    }

    /// One row's pending state, if any.
    pub(crate) fn dirty_state_of(&self, store_id: u64, key: u64) -> Option<DirtyRow> {
        self.inner
            .dirty
            .borrow()
            .rows
            .get(&(store_id, key))
            .cloned()
    }

    fn ensure_schema_table(&self) -> Result<(), DbError> {
        self.conn().execute(
            "CREATE TABLE IF NOT EXISTS _day_schema (\
             table_name TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, version INTEGER NOT NULL)",
            &[],
        )?;
        Ok(())
    }

    fn db_version(&self) -> Result<u32, DbError> {
        let mut version = 0u32;
        self.conn().query(
            "SELECT version FROM _day_schema WHERE table_name = '_db'",
            &[],
            &mut |row| {
                version = row.get(0).as_int().unwrap_or(0) as u32;
            },
        )?;
        Ok(version)
    }

    fn set_db_version(&self, v: u32) -> Result<(), DbError> {
        self.conn().execute(
            "INSERT INTO _day_schema (table_name, fingerprint, version) VALUES ('_db', '', ?) \
             ON CONFLICT(table_name) DO UPDATE SET version = excluded.version",
            &[Value::Int(v as i64)],
        )?;
        Ok(())
    }

    fn run_stages(&self, plan: MigrationPlan) -> Result<(), DbError> {
        let target = plan.target_version();
        let mut current = self.db_version()?;
        if current > target && target > 0 {
            return Err(DbError::new(
                DbErrorKind::Schema,
                format!(
                    "database is at version {current}, newer than this build's {target} — \
                     refusing to open (an old app writing a new schema is how data rots)"
                ),
            ));
        }
        let mut stages = plan.stages;
        stages.sort_by_key(|s| s.from);
        for stage in stages {
            if stage.from != current {
                if stage.to <= current {
                    continue; // already applied
                }
                return Err(DbError::new(
                    DbErrorKind::Schema,
                    format!(
                        "migration stage {}→{} does not start at the database's version {current}",
                        stage.from, stage.to
                    ),
                ));
            }
            let mut conn = self.conn();
            conn.begin()?;
            match (stage.run)(conn.as_mut()) {
                Ok(()) => {
                    conn.commit()?;
                    drop(conn);
                    current = stage.to;
                    self.set_db_version(current)?;
                }
                Err(e) => {
                    let _ = conn.rollback();
                    return Err(e);
                }
            }
        }
        if target > 0 {
            self.set_db_version(target)?;
        }
        Ok(())
    }

    /// CREATE (or lightweight-migrate) `M`'s table and register its hooks. The cache starts
    /// EMPTY — this is the line where the old engine read the whole table, and the point of
    /// the lazy one is that nothing does.
    fn attach<M: Model>(&self) -> Result<(), DbError> {
        // FTS5's external-content table and the R*Tree address rows by ROWID — an i64 — so
        // both need the key column to BE the integer rowid. A wide-keyed model declaring one
        // is refused at open, naming the constraint, rather than silently mis-indexing.
        let key_sql = M::COLUMNS
            .iter()
            .find(|c| c.name == M::KEY)
            .map(|c| c.sql)
            .unwrap_or(SqlType::Integer);
        if key_sql != SqlType::Integer && (!M::FTS_COLUMNS.is_empty() || M::SPATIAL.is_some()) {
            return Err(DbError::new(
                DbErrorKind::Unsupported,
                format!(
                    "`{}` declares fts(…)/spatial(…), which address rows by ROWID — those \
                     need an integer `#[model(id)]`, not a Uuid or String key",
                    M::TABLE
                ),
            ));
        }
        self.ensure_table::<M>()?;

        let store: Store<Keyed<M>> = Store::new(Keyed::new(Vec::new()));
        let store_id = store.store_id();

        let columns: Vec<String> = M::COLUMNS.iter().map(|c| c.name.to_string()).collect();
        let fields: Vec<String> = M::COLUMNS.iter().map(|c| c.field.to_string()).collect();
        let row_for = {
            Rc::new(move |key: u64| store.with_untracked(|k| k.get(key).map(|m| m.to_row())))
                as Rc<dyn Fn(u64) -> Option<Vec<Value>>>
        };
        let all_rows = {
            Rc::new(move || {
                store.with_untracked(|k| {
                    k.items().iter().map(|m| (m.handle(), m.to_row())).collect()
                })
            }) as Rc<dyn Fn() -> Vec<(u64, Vec<Value>)>>
        };
        let resident_keys =
            Rc::new(move || store.with_untracked(|k| k.keys())) as Rc<dyn Fn() -> Vec<u64>>;
        let resident_len =
            Rc::new(move || store.with_untracked(|k| k.len())) as Rc<dyn Fn() -> usize>;
        let is_resident = Rc::new(move |h: u64| store.with_untracked(|k| k.get(h).is_some()))
            as Rc<dyn Fn(u64) -> bool>;

        let absorb = {
            Rc::new(
                move |raw_rows: Vec<Vec<Value>>| -> Result<Vec<u64>, DbError> {
                    let mut decoded: Vec<M> = Vec::with_capacity(raw_rows.len());
                    for r in raw_rows {
                        decoded.push(M::from_row(&r)?);
                    }
                    let keys: Vec<u64> = decoded.iter().map(|m| m.handle()).collect();
                    store.populate(decoded);
                    Ok(keys)
                },
            ) as AbsorbFn
        };
        let refresh = {
            Rc::new(move |raw_rows: Vec<Vec<Value>>| -> Result<bool, DbError> {
                let mut fresh: Vec<M> = Vec::with_capacity(raw_rows.len());
                for r in raw_rows {
                    fresh.push(M::from_row(&r)?);
                }
                let existing: Vec<u64> = store.with_untracked(|k| k.keys());
                let fresh_keys: std::collections::HashSet<u64> =
                    fresh.iter().map(|m| m.handle()).collect();
                let mut changed = false;
                day_model::with_author(ModelContainer::EXTERNAL_AUTHOR, || {
                    for m in fresh {
                        let key = m.handle();
                        match store.with_untracked(|k| k.get(key).map(|old| old.to_row())) {
                            // Selected by resident keys, so an unknown row means the cache
                            // moved underneath us — treat it as an arrival.
                            None => {
                                changed = true;
                                store.restructure("external", Op::Insert, key, |k| k.push(m));
                            }
                            Some(old) => {
                                let new_row = m.to_row();
                                let labels: Vec<&'static str> = M::COLUMNS
                                    .iter()
                                    .enumerate()
                                    .filter(|(i, _)| Row::get(&old, *i) != Row::get(&new_row, *i))
                                    .map(|(_, c)| c.field)
                                    .collect();
                                if !labels.is_empty() {
                                    changed = true;
                                    store.merge_row(key, m, &labels);
                                }
                            }
                        }
                    }
                    for gone in existing.into_iter().filter(|k| !fresh_keys.contains(k)) {
                        changed = true;
                        store.restructure("external", Op::Delete, gone, |k| {
                            k.remove(gone);
                        });
                    }
                });
                Ok(changed)
            }) as RefreshFn
        };
        let evict = Rc::new(move |candidates: &[u64], want: usize| {
            let doomed: Vec<u64> = candidates
                .iter()
                .copied()
                .filter(|h| !store.is_observed(*h))
                .take(want)
                .collect();
            store.depopulate_many(&doomed)
        }) as EvictFn;
        let watch_undo = Rc::new(move |stack: &day_model::UndoStack| stack.watch(store))
            as Rc<dyn Fn(&day_model::UndoStack)>;

        self.inner.tables.borrow_mut().insert(
            store_id,
            TableHooks {
                table: M::TABLE,
                key_cols: vec![M::KEY.to_string()],
                key_where: Rc::new(|h| (format!("{} = ?", M::KEY), vec![key_param(h)])),
                columns,
                fields,
                resident_len,
                fts: (!M::FTS_COLUMNS.is_empty()).then(|| format!("{}_fts", M::TABLE)),
                spatial: M::SPATIAL.map(|s| {
                    (
                        s.lat.to_string(),
                        s.lon.to_string(),
                        format!("{}_geo", M::TABLE),
                    )
                }),
                row_for,
                all_rows,
                resident_keys,
                is_resident,
                absorb,
                refresh,
                evict,
                watch_undo,
            },
        );
        self.inner
            .stores
            .borrow_mut()
            .insert(TypeId::of::<M>(), Box::new(store));
        Ok(())
    }

    fn ensure_table<M: Model>(&self) -> Result<(), DbError> {
        // Relation-contributed clauses fold into the fingerprint: adopting or changing a
        // delete rule re-runs this path. An EXISTING table cannot gain the SQL-level clause
        // (SQLite has no ALTER ADD CONSTRAINT) — the in-memory rule still enforces, and a
        // staged rebuild adopts the clause when the app wants the engine's backstop too.
        let mut fp_val = model_fingerprint::<M>();
        for s in self
            .inner
            .fk_specs
            .borrow()
            .iter()
            .filter(|s| s.child_table == M::TABLE)
        {
            for b in s
                .child_field
                .bytes()
                .chain(s.parent_table.bytes())
                .chain(s.rule.sql().bytes())
            {
                fp_val ^= b as u64;
                fp_val = fp_val.wrapping_mul(0x1000_0000_01b3);
            }
        }
        let fp = format!("{fp_val:016x}");
        let stored = self.stored_fingerprint(M::TABLE)?;
        if stored.as_deref() == Some(fp.as_str()) {
            return Ok(());
        }

        let exists = self.table_exists(M::TABLE)?;
        if !exists {
            self.create_table::<M>()?;
        } else {
            self.lightweight_migrate::<M>()?;
        }
        self.create_indexes::<M>()?;
        self.create_shadow_tables::<M>()?;
        self.store_fingerprint(M::TABLE, &fp)?;
        Ok(())
    }

    /// The FTS5 and R*Tree shadows a model declares: virtual tables plus `AFTER` triggers, so
    /// the indexes stay true inside the same transaction as every write — even a write made by
    /// another tool straight into the file. Backfilled when first created.
    fn create_shadow_tables<M: Model>(&self) -> Result<(), DbError> {
        let t = M::TABLE;
        let key = M::KEY;
        if !M::FTS_COLUMNS.is_empty() {
            if !self.inner.caps.full_text_search {
                return Err(DbError::new(
                    DbErrorKind::Unsupported,
                    format!(
                        "`{t}` declares fts(…) but this driver's engine has no FTS5 — the \
                         bundled and cipher engines compile it in"
                    ),
                ));
            }
            let cols = M::FTS_COLUMNS.join(", ");
            let fresh = !self.table_exists(&format!("{t}_fts"))?;
            // The tokenizer rides in the DDL; its value is a compile-time literal, quoted
            // through SQL's own escape so an apostrophe cannot break the statement.
            let tokenize = match M::FTS_TOKENIZE {
                Some(tok) => format!(", tokenize='{}'", tok.replace('\'', "''")),
                None => String::new(),
            };
            self.conn().execute(
                &format!(
                    "CREATE VIRTUAL TABLE IF NOT EXISTS {t}_fts USING fts5({cols}, \
                     content={t}, content_rowid={key}{tokenize})"
                ),
                &[],
            )?;
            let new_cols = M::FTS_COLUMNS
                .iter()
                .map(|c| format!("new.{c}"))
                .collect::<Vec<_>>()
                .join(", ");
            let old_cols = M::FTS_COLUMNS
                .iter()
                .map(|c| format!("old.{c}"))
                .collect::<Vec<_>>()
                .join(", ");
            self.conn().execute(
                &format!(
                    "CREATE TRIGGER IF NOT EXISTS {t}_fts_ai AFTER INSERT ON {t} BEGIN \
                     INSERT INTO {t}_fts(rowid, {cols}) VALUES (new.{key}, {new_cols}); END"
                ),
                &[],
            )?;
            self.conn().execute(
                &format!(
                    "CREATE TRIGGER IF NOT EXISTS {t}_fts_ad AFTER DELETE ON {t} BEGIN \
                     INSERT INTO {t}_fts({t}_fts, rowid, {cols}) VALUES ('delete', old.{key}, {old_cols}); END"
                ),
                &[],
            )?;
            self.conn().execute(
                &format!(
                    "CREATE TRIGGER IF NOT EXISTS {t}_fts_au AFTER UPDATE ON {t} BEGIN \
                     INSERT INTO {t}_fts({t}_fts, rowid, {cols}) VALUES ('delete', old.{key}, {old_cols}); \
                     INSERT INTO {t}_fts(rowid, {cols}) VALUES (new.{key}, {new_cols}); END"
                ),
                &[],
            )?;
            if fresh {
                // External-content rebuild: index whatever rows predate the declaration.
                self.conn().execute(
                    &format!("INSERT INTO {t}_fts({t}_fts) VALUES ('rebuild')"),
                    &[],
                )?;
            }
        }
        if let Some(s) = M::SPATIAL {
            if !self.inner.caps.rtree {
                return Err(DbError::new(
                    DbErrorKind::Unsupported,
                    format!(
                        "`{t}` declares spatial(…) but this driver's engine has no R*Tree — \
                         the bundled and cipher engines compile it in"
                    ),
                ));
            }
            let (lat, lon) = (s.lat, s.lon);
            let fresh = !self.table_exists(&format!("{t}_geo"))?;
            self.conn().execute(
                &format!(
                    "CREATE VIRTUAL TABLE IF NOT EXISTS {t}_geo USING rtree({key}, \
                     min_lat, max_lat, min_lon, max_lon)"
                ),
                &[],
            )?;
            self.conn().execute(
                &format!(
                    "CREATE TRIGGER IF NOT EXISTS {t}_geo_ai AFTER INSERT ON {t} BEGIN \
                     INSERT OR REPLACE INTO {t}_geo VALUES (new.{key}, new.{lat}, new.{lat}, new.{lon}, new.{lon}); END"
                ),
                &[],
            )?;
            self.conn().execute(
                &format!(
                    "CREATE TRIGGER IF NOT EXISTS {t}_geo_au AFTER UPDATE OF {lat}, {lon} ON {t} BEGIN \
                     INSERT OR REPLACE INTO {t}_geo VALUES (new.{key}, new.{lat}, new.{lat}, new.{lon}, new.{lon}); END"
                ),
                &[],
            )?;
            self.conn().execute(
                &format!(
                    "CREATE TRIGGER IF NOT EXISTS {t}_geo_ad AFTER DELETE ON {t} BEGIN \
                     DELETE FROM {t}_geo WHERE {key} = old.{key}; END"
                ),
                &[],
            )?;
            if fresh {
                self.conn().execute(
                    &format!(
                        "INSERT OR REPLACE INTO {t}_geo \
                         SELECT {key}, {lat}, {lat}, {lon}, {lon} FROM {t}"
                    ),
                    &[],
                )?;
            }
        }
        Ok(())
    }

    fn table_exists(&self, table: &str) -> Result<bool, DbError> {
        let mut found = false;
        self.conn().query(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?",
            &[Value::Text(table.into())],
            &mut |_| found = true,
        )?;
        Ok(found)
    }

    fn create_table<M: Model>(&self) -> Result<(), DbError> {
        let specs = self.inner.fk_specs.borrow();
        let cols: Vec<String> = M::COLUMNS
            .iter()
            .map(|c| {
                let mut s = format!("{} {}", c.name, c.sql.ddl());
                if c.name == M::KEY {
                    s.push_str(" PRIMARY KEY");
                } else if c.not_null {
                    s.push_str(" NOT NULL");
                }
                if c.unique && c.name != M::KEY {
                    s.push_str(" UNIQUE");
                }
                // A declared relation's foreign key: enforced by the engine too, so another
                // process honors the same delete rule — and so a cascade's reach into rows
                // this process never faulted still lands in the file. Deferred, so
                // within-transaction statement order (a cascade's children, an undo's
                // re-inserts) never trips it.
                if let Some(spec) = specs
                    .iter()
                    .find(|f| f.child_table == M::TABLE && f.child_field == c.field)
                {
                    s.push_str(&format!(
                        " REFERENCES {}({}) ON DELETE {} DEFERRABLE INITIALLY DEFERRED",
                        spec.parent_table,
                        spec.parent_key,
                        spec.rule.sql()
                    ));
                }
                s
            })
            .collect();
        let body = format!("CREATE TABLE {} ({})", M::TABLE, cols.join(", "));
        // STRICT where the engine allows it (always, for the bundled build); older system
        // SQLites reject the keyword, and the plain form is the honest fallback.
        if self.conn().execute(&format!("{body} STRICT"), &[]).is_err() {
            self.conn().execute(&body, &[])?;
        }
        Ok(())
    }

    fn create_indexes<M: Model>(&self) -> Result<(), DbError> {
        // A declared relation's foreign-key column is indexed WITHOUT being asked: it is what
        // `children_of` selects on, what relation predicates correlate on, and what the
        // engine scans to enforce `ON DELETE`. Unindexed, every one of those is a full scan
        // of the child table.
        let fk_cols: Vec<&str> = self
            .inner
            .fk_specs
            .borrow()
            .iter()
            .filter(|s| s.child_table == M::TABLE)
            .filter_map(|s| {
                M::COLUMNS
                    .iter()
                    .find(|c| c.field == s.child_field)
                    .map(|c| c.name)
            })
            .collect();
        for c in M::COLUMNS
            .iter()
            .filter(|c| c.indexed || (fk_cols.contains(&c.name) && c.name != M::KEY))
        {
            self.conn().execute(
                &format!(
                    "CREATE INDEX IF NOT EXISTS day_idx_{t}_{c} ON {t}({c})",
                    t = M::TABLE,
                    c = c.name
                ),
                &[],
            )?;
        }
        for idx in M::COMPOSITE_INDEXES {
            let cols = idx.join(", ");
            let name = idx.join("_");
            self.conn().execute(
                &format!(
                    "CREATE INDEX IF NOT EXISTS day_idx_{t}_{name} ON {t}({cols})",
                    t = M::TABLE
                ),
                &[],
            )?;
        }
        Ok(())
    }

    /// Close the gap between the file's columns and the declared ones — additions and drops
    /// only. A type change or a rename is refused with both names in the error; that is what
    /// staged migrations are for.
    fn lightweight_migrate<M: Model>(&self) -> Result<(), DbError> {
        let mut existing: Vec<String> = Vec::new();
        self.conn().query(
            &format!("PRAGMA table_info({})", M::TABLE),
            &[],
            &mut |row| {
                if let Ok(name) = row.get(1).as_text() {
                    existing.push(name.to_string());
                }
            },
        )?;

        let declared: Vec<&'static str> = M::COLUMNS.iter().map(|c| c.name).collect();
        let defaults = M::default_row();

        for (i, col) in M::COLUMNS.iter().enumerate() {
            if !existing.iter().any(|e| e == col.name) {
                // Add nullable, then backfill with the field's Rust default.
                self.conn().execute(
                    &format!(
                        "ALTER TABLE {} ADD COLUMN {} {}",
                        M::TABLE,
                        col.name,
                        col.sql.ddl()
                    ),
                    &[],
                )?;
                let default = Row::get(&defaults, i);
                if default != Value::Null {
                    self.conn().execute(
                        &format!("UPDATE {} SET {} = ?", M::TABLE, col.name),
                        &[default],
                    )?;
                }
            }
        }
        for gone in existing.iter().filter(|e| !declared.iter().any(|d| d == e)) {
            self.conn()
                .execute(&format!("ALTER TABLE {} DROP COLUMN {gone}", M::TABLE), &[])?;
        }
        Ok(())
    }

    fn stored_fingerprint(&self, table: &str) -> Result<Option<String>, DbError> {
        let mut fp = None;
        self.conn().query(
            "SELECT fingerprint FROM _day_schema WHERE table_name = ?",
            &[Value::Text(table.into())],
            &mut |row| {
                if let Ok(t) = row.get(0).as_text() {
                    fp = Some(t.to_string());
                }
            },
        )?;
        Ok(fp)
    }

    fn store_fingerprint(&self, table: &str, fp: &str) -> Result<(), DbError> {
        self.conn().execute(
            "INSERT INTO _day_schema (table_name, fingerprint, version) VALUES (?, ?, 0) \
             ON CONFLICT(table_name) DO UPDATE SET fingerprint = excluded.fingerprint",
            &[Value::Text(table.into()), Value::Text(fp.into())],
        )?;
        Ok(())
    }

    fn install_sink(&self) {
        let inner = Rc::downgrade(&self.inner);
        let sink = day_model::install_change_sink(move |change| {
            if let Some(inner) = inner.upgrade() {
                // An external merge's changes are the database's own contents arriving:
                // queries go stale on them like any edit, but nothing goes back to the file.
                if change.author != Some(ModelContainer::EXTERNAL_AUTHOR) {
                    let tables = inner.tables.borrow();
                    inner
                        .dirty
                        .borrow_mut()
                        .note(change, |store| tables.contains_key(&store));
                }
                let container = ModelContainer { inner };
                container.mark_queries_stale(change);
                container.relations_on_change(change);
            }
        });
        self.inner.sink.set(Some(sink));
    }

    fn install_autosave(&self) {
        // Weak: `on_turn_end` registrations live for the process, and a strong clone here
        // would keep the container (and its connection) alive forever.
        let inner = Rc::downgrade(&self.inner);
        day_reactive::on_turn_end(move || {
            let Some(inner) = inner.upgrade() else { return };
            let this = ModelContainer { inner };
            if this.inner.autosave.get() && !this.inner.dirty.borrow().is_empty() {
                // The error lands in the signal; a turn end has no caller to give a Result to.
                let _ = this.save();
            }
        });
    }

    fn flush(&self, dirty: DirtyState) -> Result<Vec<(String, Vec<Value>)>, DbError> {
        let stmts = self.fold(&dirty)?;
        if stmts.is_empty() {
            return Ok(stmts);
        }
        let mut conn = self.conn();
        conn.begin()?;
        for (sql, params) in &stmts {
            if let Err(e) = conn.execute(sql, params) {
                let _ = conn.rollback();
                return Err(e);
            }
        }
        conn.commit()?;
        Ok(stmts)
    }

    /// The fold, materialized: the smallest statement list that expresses `dirty`, in
    /// first-touch order. Row values come from the caches NOW — the change log carried which
    /// rows and columns moved, never their contents. Reads only; [`ModelContainer::flush`]
    /// executes the list in one transaction.
    fn fold(&self, dirty: &DirtyState) -> Result<Vec<(String, Vec<Value>)>, DbError> {
        let tables = self.inner.tables.borrow();
        let mut stmts: Vec<(String, Vec<Value>)> = Vec::new();

        // Wholesale rewrites first: upsert every RESIDENT row. (The cache is a working set —
        // rows outside it were never part of the rewrite, and deleting "the rest" would
        // delete data the rewrite never saw.)
        for store_id in &dirty.full {
            let Some(hooks) = tables.get(store_id) else {
                continue;
            };
            for (_, row) in (hooks.all_rows)() {
                stmts.push(upsert_stmt(hooks, &row));
            }
        }

        // Per-row work, into batches that can merge. Deletes of one table merge into one
        // `IN`; updates merge when they write the SAME columns to the SAME values, which is
        // what a multi-selection edit produces ("set fill on twelve shapes").
        let mut batches: Vec<Batch> = Vec::new();
        // (store, statement shape, a fingerprint of the SET values) → the batch it joins.
        let mut open: HashMap<(u64, String, u64), usize> = HashMap::new();

        for id in &dirty.order {
            let Some(state) = dirty.rows.get(id) else {
                continue;
            };
            let (store_id, key) = *id;
            if dirty.full.contains(&store_id) {
                continue; // the resync above already covered it
            }
            let Some(hooks) = tables.get(&store_id) else {
                continue;
            };
            // A join row is addressed by a PAIR of columns, which no single-column `IN` can
            // express, so those keep one statement each.
            let batchable = hooks.key_cols.len() == 1;
            match state {
                DirtyRow::Delete => {
                    if !batchable {
                        let (clause, params) = (hooks.key_where)(key);
                        batches.push(Batch::Single(
                            format!("DELETE FROM {} WHERE {clause}", hooks.table),
                            params,
                        ));
                        continue;
                    }
                    join_batch(&mut batches, &mut open, store_id, None, key);
                }
                DirtyRow::Insert => {
                    if let Some(row) = (hooks.row_for)(key) {
                        let (sql, params) = upsert_stmt(hooks, &row);
                        batches.push(Batch::Single(sql, params));
                    }
                }
                DirtyRow::Update(cols) => {
                    let Some(row) = (hooks.row_for)(key) else {
                        continue; // deleted since — its own Delete entry handles it
                    };
                    if cols.is_empty() {
                        // A row-level replacement named no columns — write them all.
                        let (sql, params) = upsert_stmt(hooks, &row);
                        batches.push(Batch::Single(sql, params));
                        continue;
                    }
                    let mut sets: Vec<String> = Vec::with_capacity(cols.len());
                    let mut set_params: Vec<Value> = Vec::with_capacity(cols.len());
                    for c in cols {
                        let Some(i) = hooks.fields.iter().position(|n| n == c) else {
                            continue; // a transient field's label — never a column
                        };
                        sets.push(format!("{} = ?", hooks.columns[i]));
                        set_params.push(row.get(i));
                    }
                    if set_params.is_empty() {
                        continue; // only transient fields changed
                    }
                    let clause = sets.join(", ");
                    if !batchable {
                        let (where_clause, mut where_params) = (hooks.key_where)(key);
                        let mut params = set_params;
                        params.append(&mut where_params);
                        batches.push(Batch::Single(
                            format!("UPDATE {} SET {clause} WHERE {where_clause}", hooks.table),
                            params,
                        ));
                        continue;
                    }
                    join_batch(
                        &mut batches,
                        &mut open,
                        store_id,
                        Some((clause, set_params)),
                        key,
                    );
                }
            }
        }

        for batch in batches {
            match batch {
                Batch::Single(sql, params) => stmts.push((sql, params)),
                Batch::Keyed { store, set, keys } => {
                    let Some(hooks) = tables.get(&store) else {
                        continue;
                    };
                    materialize(hooks, set, &keys, &mut stmts);
                }
            }
        }
        Ok(stmts)
    }

    // --- maintenance (docs/persistence.md) ---------------------------------------------------

    /// `VACUUM INTO`: a transactionally consistent, already-compacted snapshot, taken while
    /// the app keeps writing. Restore is an open, not a verb.
    pub fn backup_to(&self, path: &Path) -> Result<(), DbError> {
        self.save()?;
        self.conn().execute(
            "VACUUM INTO ?",
            &[Value::Text(path.to_string_lossy().into_owned())],
        )?;
        Ok(())
    }

    /// `PRAGMA integrity_check`, parsed: an empty list means the database is sound.
    pub fn integrity_check(&self) -> Result<Vec<String>, DbError> {
        let mut findings = Vec::new();
        self.conn()
            .query("PRAGMA integrity_check", &[], &mut |row| {
                if let Ok(t) = row.get(0).as_text()
                    && t != "ok"
                {
                    findings.push(t.to_string());
                }
            })?;
        Ok(findings)
    }

    /// Fold the WAL into the main file — before the OS copies it whole (device backup).
    pub fn checkpoint(&self) -> Result<(), DbError> {
        // The PRAGMA answers with a status row; a query with an ignored callback consumes it.
        self.conn()
            .query("PRAGMA wal_checkpoint(TRUNCATE)", &[], &mut |_| {})
    }

    pub fn vacuum(&self) -> Result<(), DbError> {
        self.save()?;
        self.conn().execute("VACUUM", &[]).map(|_| ())
    }

    /// Re-encrypt in place under a new key (`PRAGMA rekey`; feature `cipher`).
    #[cfg(feature = "cipher")]
    pub fn rekey(&self, new: Secret) -> Result<(), DbError> {
        self.save()?;
        let quoted = new.reveal().replace('\'', "''");
        // The PRAGMA answers with a status row; consume it as a query.
        self.conn()
            .query(&format!("PRAGMA rekey = '{quoted}'"), &[], &mut |_| {})
    }

    /// Write an ENCRYPTED copy at `path` — SQLCipher's own conversion path (`ATTACH` +
    /// `sqlcipher_export`); plaintext↔encrypted cannot happen in place.
    #[cfg(feature = "cipher")]
    pub fn encrypt_to(&self, path: &Path, key: Secret) -> Result<(), DbError> {
        self.export_to(path, Some(key))
    }

    /// Write a PLAINTEXT copy at `path`, same mechanism with an empty key.
    #[cfg(feature = "cipher")]
    pub fn decrypt_to(&self, path: &Path) -> Result<(), DbError> {
        self.export_to(path, None)
    }

    #[cfg(feature = "cipher")]
    fn export_to(&self, path: &Path, key: Option<Secret>) -> Result<(), DbError> {
        self.save()?;
        let quoted_path = path.to_string_lossy().replace('\'', "''");
        let quoted_key = key
            .as_ref()
            .map(|k| k.reveal().replace('\'', "''"))
            .unwrap_or_default();
        let mut conn = self.conn();
        conn.execute(
            &format!("ATTACH DATABASE '{quoted_path}' AS day_export KEY '{quoted_key}'"),
            &[],
        )?;
        conn.query("SELECT sqlcipher_export('day_export')", &[], &mut |_| {})
            .and_then(|_| conn.execute("DETACH DATABASE day_export", &[]).map(|_| ()))
    }

    /// Total size in bytes (page_count × page_size).
    pub fn size_bytes(&self) -> Result<u64, DbError> {
        let mut pages = 0i64;
        let mut page_size = 0i64;
        self.conn().query("PRAGMA page_count", &[], &mut |row| {
            pages = row.get(0).as_int().unwrap_or(0);
        })?;
        self.conn().query("PRAGMA page_size", &[], &mut |row| {
            page_size = row.get(0).as_int().unwrap_or(0);
        })?;
        Ok((pages * page_size).max(0) as u64)
    }
}

/// The multi-key WHERE builder for a hooks entry: `key IN (…)` for single-column keys, an OR
/// list of pair clauses for join tables.
fn keys_where_of(hooks: &TableHooks) -> KeysWhere {
    if hooks.key_cols.len() == 1 {
        let col = hooks.key_cols[0].clone();
        Rc::new(move |keys: &[u64]| {
            let marks = vec!["?"; keys.len()].join(", ");
            (
                format!("{col} IN ({marks})"),
                keys.iter().map(|k| key_param(*k)).collect(),
            )
        })
    } else {
        let key_where = hooks.key_where.clone();
        Rc::new(move |keys: &[u64]| {
            let mut clauses = Vec::with_capacity(keys.len());
            let mut params = Vec::with_capacity(keys.len() * 2);
            for k in keys {
                let (c, mut p) = key_where(*k);
                clauses.push(format!("({c})"));
                params.append(&mut p);
            }
            (clauses.join(" OR "), params)
        })
    }
}

/// One statement the fold will write, before the mergeable ones have merged.
enum Batch {
    /// Stands alone: an upsert (its values are the row's own), or a row addressed by a
    /// composite key, which no single-column `IN` can express.
    Single(String, Vec<Value>),
    /// Same table, same operation, same values — one statement per chunk of keys.
    Keyed {
        store: u64,
        /// The `SET` clause and its parameters for an update; `None` for a delete.
        set: Option<(String, Vec<Value>)>,
        keys: Vec<u64>,
    },
}

/// SQLite's conservative bound-parameter ceiling. Modern builds allow far more, but a
/// `system` engine may be an older one, and chunking costs nothing to write.
const MAX_BOUND_PARAMS: usize = 900;

/// Add `key` to the batch sharing this table, clause and values — or open a new one.
///
/// The slot key separates a delete from an update (their clauses differ), and separates
/// updates writing DIFFERENT values, which must stay different statements. Grouping is by
/// hash so a large flush stays linear; the values are compared before merging, so a hash
/// collision costs a second statement rather than a wrong one.
fn join_batch(
    batches: &mut Vec<Batch>,
    open: &mut HashMap<(u64, String, u64), usize>,
    store: u64,
    set: Option<(String, Vec<Value>)>,
    key: u64,
) {
    let clause = set.as_ref().map(|(c, _)| c.clone()).unwrap_or_default();
    let print = set.as_ref().map(|(_, p)| fingerprint(p)).unwrap_or(0);
    let slot = (store, clause, print);
    if let Some(&i) = open.get(&slot)
        && let Batch::Keyed {
            set: existing,
            keys,
            ..
        } = &mut batches[i]
        && existing.as_ref().map(|(_, p)| p.as_slice()) == set.as_ref().map(|(_, p)| p.as_slice())
    {
        keys.push(key);
        return;
    }
    open.insert(slot, batches.len());
    batches.push(Batch::Keyed {
        store,
        set,
        keys: vec![key],
    });
}

/// A cheap order-sensitive hash of bound values, for grouping identical updates. Equality is
/// still checked before two rows share a statement — this only decides who to compare with.
fn fingerprint(values: &[Value]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x1000_0000_01b3);
        }
    };
    for v in values {
        match v {
            Value::Null => eat(&[0]),
            Value::Int(i) => {
                eat(&[1]);
                eat(&i.to_le_bytes());
            }
            // Bit pattern, not numeric value: two values that hash apart are only compared
            // separately, never merged wrongly.
            Value::Real(r) => {
                eat(&[2]);
                eat(&r.to_bits().to_le_bytes());
            }
            Value::Text(t) => {
                eat(&[3]);
                eat(t.as_bytes());
            }
            Value::Blob(b) => {
                eat(&[4]);
                eat(b);
            }
        }
    }
    h
}

/// Write a batch out: the familiar `= ?` form for one key, chunked `IN` statements for more.
fn materialize(
    hooks: &TableHooks,
    set: Option<(String, Vec<Value>)>,
    keys: &[u64],
    out: &mut Vec<(String, Vec<Value>)>,
) {
    // One key keeps the `= ?` form: it is the common case by far, it reads plainly in a
    // trace, and it is one more statement shape the cache can hold.
    if keys.len() == 1 {
        let (where_clause, where_params) = (hooks.key_where)(keys[0]);
        match set {
            Some((clause, mut params)) => {
                params.extend(where_params);
                out.push((
                    format!("UPDATE {} SET {clause} WHERE {where_clause}", hooks.table),
                    params,
                ));
            }
            None => out.push((
                format!("DELETE FROM {} WHERE {where_clause}", hooks.table),
                where_params,
            )),
        }
        return;
    }

    let key_col = &hooks.key_cols[0];
    let set_len = set.as_ref().map(|(_, p)| p.len()).unwrap_or(0);
    let cap = MAX_BOUND_PARAMS.saturating_sub(set_len).max(1);
    for chunk in keys.chunks(cap) {
        let mut params: Vec<Value> = Vec::with_capacity(set_len + chunk.len());
        if let Some((_, set_params)) = &set {
            params.extend(set_params.iter().cloned());
        }
        params.extend(chunk.iter().map(|k| key_param(*k)));
        let marks = vec!["?"; chunk.len()].join(", ");
        let sql = match &set {
            Some((clause, _)) => format!(
                "UPDATE {} SET {clause} WHERE {key_col} IN ({marks})",
                hooks.table
            ),
            None => format!("DELETE FROM {} WHERE {key_col} IN ({marks})", hooks.table),
        };
        out.push((sql, params));
    }
}

fn upsert_stmt(hooks: &TableHooks, row: &[Value]) -> (String, Vec<Value>) {
    let cols = hooks.columns.join(", ");
    let marks = vec!["?"; hooks.columns.len()].join(", ");
    // A true UPSERT, not INSERT OR REPLACE: the conflict path fires UPDATE triggers, which is
    // what keeps the generated FTS/R*Tree shadows in sync (OR REPLACE's implicit delete skips
    // AFTER DELETE triggers unless recursive_triggers is on).
    let sets = hooks
        .columns
        .iter()
        .filter(|c| !hooks.key_cols.contains(c))
        .map(|c| format!("{c} = excluded.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    let conflict = if sets.is_empty() {
        "DO NOTHING".to_string()
    } else {
        format!("DO UPDATE SET {sets}")
    };
    (
        format!(
            "INSERT INTO {} ({cols}) VALUES ({marks}) ON CONFLICT({}) {conflict}",
            hooks.table,
            hooks.key_cols.join(", ")
        ),
        row.to_vec(),
    )
}

impl Drop for ContainerInner {
    fn drop(&mut self) {
        if let Some(sink) = self.sink.take() {
            day_model::remove_change_sink(sink);
        }
    }
}

// ---------------------------------------------------------------------------
// Live queries (docs/persistence.md; queries.rs holds the compiler and the diff)
// ---------------------------------------------------------------------------

/// What a query's consumers have not yet seen. `Deltas` can animate a list row by row;
/// `Reload` means the whole set moved (a fetch swap, a tangled requery) and a reload is
/// honest.
#[derive(Clone, Debug, PartialEq)]
pub enum QueryEvents {
    None,
    Deltas(Vec<Delta>),
    Reload,
}

impl QueryEvents {
    fn push(&mut self, deltas: &[Delta]) {
        match self {
            QueryEvents::None => *self = QueryEvents::Deltas(deltas.to_vec()),
            QueryEvents::Deltas(d) => d.extend_from_slice(deltas),
            QueryEvents::Reload => {}
        }
    }
    fn reload(&mut self) {
        *self = QueryEvents::Reload;
    }
}

/// What one relation crossing subscribes a query to — precomputed at install (and at every
/// fetch swap) so the change sink's staleness check is a few comparisons.
struct RelWatch {
    /// The store on the far side of the crossing (the related table's cache).
    store: u64,
    /// The join membership store, when the crossing is a many-to-many — every link, unlink
    /// and reposition there is a membership change.
    join_store: Option<u64>,
    /// FIELD names on the far store whose write is a membership change (the foreign key, the
    /// order field) even though no predicate reads them.
    fields: Vec<&'static str>,
    /// COLUMN names of the far table the inner predicate reads.
    columns: Vec<&'static str>,
}

struct QueryState {
    store_id: u64,
    table: &'static str,
    /// The relation subscriptions — recomputed when the fetch is swapped.
    watches: RefCell<Vec<RelWatch>>,
    /// Extra LOCAL columns whose write is a membership change (a `One` column crossed by the
    /// query's own table).
    local_extra: RefCell<Vec<&'static str>>,
    set: RefCell<ResultSet>,
    /// Bumped whenever the result set changes; `ids()`/`count()` track it.
    version: day_reactive::Signal<u64>,
    pending: RefCell<QueryEvents>,
    /// A dependency-touching change arrived; the answer re-derives after the flush.
    needs_sql: Cell<bool>,
    /// `query_raw` only: the statement and the tables whose flush re-runs it.
    raw: Option<RawQuery>,
    /// A COUNT-shaped query: `count` is the answer, the id set stays empty, and requeries
    /// run `SELECT COUNT(*)` — the badge query, O(1) memory at any result size.
    count_only: bool,
    count: Cell<usize>,
}

struct RawQuery {
    sql: String,
    params: Vec<Value>,
    tables: Vec<String>,
}

/// A live, typed result set over one model's table — ids only, answered by the engine and
/// kept current by dependency-gated requeries. Rows stay lazy: fault the ones you show
/// ([`Query::materialize`], or a list source doing it for you). Clone shares the same set.
pub struct Query<M: Model> {
    state: Rc<QueryState>,
    container: ModelContainer,
    _p: std::marker::PhantomData<fn() -> M>,
}

impl<M: Model> Clone for Query<M> {
    fn clone(&self) -> Self {
        Query {
            state: self.state.clone(),
            container: self.container.clone(),
            _p: std::marker::PhantomData,
        }
    }
}

impl<M: Model> Query<M> {
    /// The result ids, typed and in query order — a TRACKED read: the caller re-runs when
    /// the set changes, and only then.
    pub fn ids(&self) -> Vec<ModelId<M>> {
        self.refresh_if_stale();
        let _ = self.state.version.get();
        self.state
            .set
            .borrow()
            .ids()
            .iter()
            .map(|h| ModelId::from_handle(*h))
            .collect()
    }

    /// The result count, tracked like [`Query::ids`].
    pub fn count(&self) -> usize {
        self.refresh_if_stale();
        let _ = self.state.version.get();
        self.state.set.borrow().ids().len()
    }

    /// The first result, tracked.
    pub fn first(&self) -> Option<ModelId<M>> {
        self.refresh_if_stale();
        let _ = self.state.version.get();
        self.state
            .set
            .borrow()
            .ids()
            .first()
            .map(|h| ModelId::from_handle(*h))
    }

    /// Tracked membership test.
    pub fn contains(&self, id: impl Into<ModelId<M>>) -> bool {
        self.refresh_if_stale();
        let _ = self.state.version.get();
        self.state.set.borrow().ids().contains(&id.into().handle())
    }

    /// Untracked snapshot — reactively silent, but still CURRENT: pending staleness settles
    /// first, like every other read.
    pub fn ids_untracked(&self) -> Vec<ModelId<M>> {
        self.refresh_if_stale();
        self.state
            .set
            .borrow()
            .ids()
            .iter()
            .map(|h| ModelId::from_handle(*h))
            .collect()
    }

    /// Bring this window of the result into the cache — what a list binding calls for the
    /// rows it is about to show, batched into one `SELECT`.
    pub fn materialize(&self, range: std::ops::Range<usize>) {
        let keys: Vec<u64> = {
            let set = self.state.set.borrow();
            let ids = set.ids();
            let end = range.end.min(ids.len());
            let start = range.start.min(end);
            ids[start..end].to_vec()
        };
        if let Err(e) = self.container.ensure_resident::<M>(&keys) {
            self.container.inner.error.set(Some(e.to_string()));
        }
    }

    /// Swap the fetch (a changed filter or sort): requeries and reloads consumers. No-op when
    /// equal to the current one, so a `query_fn` closure can re-run cheaply.
    pub fn set_fetch(&self, fetch: Fetch) {
        if *self.state.set.borrow().fetch() == fetch {
            return;
        }
        self.container.rewire_watches(&self.state, &fetch);
        {
            let mut set = self.state.set.borrow_mut();
            *set = ResultSet::new(fetch);
        }
        // Resolve once pending statements land; mid-turn the flush has not run yet.
        self.state.needs_sql.set(true);
        self.refresh_if_stale();
        self.state.pending.borrow_mut().reload();
        self.bump();
    }

    /// Drain what changed since the last call — the list source's feed. Pending staleness
    /// settles first, so a drain right after an edit already narrates it.
    pub fn take_events(&self) -> QueryEvents {
        self.refresh_if_stale();
        std::mem::replace(&mut *self.state.pending.borrow_mut(), QueryEvents::None)
    }

    /// A dependency-touching change arrived this turn and the flush has not run yet: settle
    /// it now. With autosave the pending statements flush (which requeries everything stale);
    /// with autosave off the query re-derives against the last save — the file is the only
    /// thing that can answer it.
    fn refresh_if_stale(&self) {
        if !self.state.needs_sql.get() {
            return;
        }
        if self.container.inner.autosave.get() {
            let _ = self.container.save();
        }
        if self.state.needs_sql.replace(false) {
            self.container.requery(&self.state);
        }
    }

    fn bump(&self) {
        self.state
            .version
            .set(self.state.version.get_untracked().wrapping_add(1));
    }
}

/// Builder for a [`Query`] — `container.query::<Trip>().filter(…).sort(…).live()`.
pub struct QueryBuilder<'c, M: Model> {
    container: &'c ModelContainer,
    fetch: Fetch,
    _p: std::marker::PhantomData<fn() -> M>,
}

impl<M: Model> QueryBuilder<'_, M> {
    pub fn filter(mut self, p: Pred) -> Self {
        self.fetch = self.fetch.filter(p);
        self
    }
    pub fn sort(mut self, s: Sort) -> Self {
        self.fetch = self.fetch.sort(s);
        self
    }
    pub fn limit(mut self, n: usize) -> Self {
        self.fetch = self.fetch.limit(n);
        self
    }
    /// Run the fetch and keep it live against the change log.
    pub fn live(self) -> Query<M> {
        self.container.install_query::<M>(self.fetch, None)
    }

    /// Keep only the COUNT live — the badge form: no id vector, one `SELECT COUNT(*)` per
    /// requery, the same dependency gating.
    pub fn live_count(self) -> CountQuery<M> {
        CountQuery {
            state: self.container.install_state::<M>(self.fetch, None, true),
            container: self.container.clone(),
            _p: std::marker::PhantomData,
        }
    }
}

/// A live COUNT over one model's table — the badge query: `SELECT COUNT(*)` behind the same
/// dependency-gated staleness as [`Query`], holding no id vector, so a count over a
/// million-row result costs O(1) memory. Clone shares the same state.
pub struct CountQuery<M: Model> {
    state: Rc<QueryState>,
    container: ModelContainer,
    _p: std::marker::PhantomData<fn() -> M>,
}

impl<M: Model> Clone for CountQuery<M> {
    fn clone(&self) -> Self {
        CountQuery {
            state: self.state.clone(),
            container: self.container.clone(),
            _p: std::marker::PhantomData,
        }
    }
}

impl<M: Model> CountQuery<M> {
    /// The count — a TRACKED read: the caller re-runs when it changes, and only then.
    pub fn get(&self) -> usize {
        self.refresh_if_stale();
        let _ = self.state.version.get();
        self.state.count.get()
    }

    /// Untracked snapshot — reactively silent, still current.
    pub fn get_untracked(&self) -> usize {
        self.refresh_if_stale();
        self.state.count.get()
    }

    /// Swap the fetch (a changed filter): re-counts. No-op when equal, so a `count_fn`
    /// closure can re-run cheaply.
    pub fn set_fetch(&self, fetch: Fetch) {
        if *self.state.set.borrow().fetch() == fetch {
            return;
        }
        self.container.rewire_watches(&self.state, &fetch);
        *self.state.set.borrow_mut() = ResultSet::new(fetch);
        self.state.needs_sql.set(true);
        self.refresh_if_stale();
    }

    fn refresh_if_stale(&self) {
        if !self.state.needs_sql.get() {
            return;
        }
        if self.container.inner.autosave.get() {
            let _ = self.container.save();
        }
        if self.state.needs_sql.replace(false) {
            self.container.requery(&self.state);
        }
    }
}

/// A fallback row: the key plus the dependency columns the fallback `SELECT` carried.
struct FallbackRow<'a> {
    cols: &'a [&'static str],
    /// values[0] is the key; values[1..] align with `cols`.
    values: &'a [Value],
}

impl RowView for FallbackRow<'_> {
    fn col(&self, c: &str) -> Option<Value> {
        self.cols
            .iter()
            .position(|n| *n == c)
            .and_then(|i| self.values.get(i + 1).cloned())
    }
}

impl ModelContainer {
    /// Start a typed query over `M`'s rows. Panics (like [`ModelContainer::cache`]) if `M` is
    /// not in this container's `schema!` — a wiring bug worth stopping on.
    pub fn query<M: Model>(&self) -> QueryBuilder<'_, M> {
        let _ = self.cache::<M>();
        QueryBuilder {
            container: self,
            fetch: Fetch::new(),
            _p: std::marker::PhantomData,
        }
    }

    /// The reactive-fetch form: `f` is a computation — a query whose FETCH depends on signals
    /// (a search term, a filter toggle) re-derives itself when they change.
    pub fn query_fn<M: Model>(&self, f: impl Fn() -> Fetch + 'static) -> Query<M> {
        let q = self.install_query::<M>(f(), None);
        let q2 = q.clone();
        day_reactive::bind(f, move |fetch| q2.set_fetch(fetch.clone()));
        q
    }

    /// The reactive-fetch COUNT: a badge whose FETCH depends on signals (the "Today" cutoff,
    /// a filter toggle) re-counts itself when they change.
    pub fn count_fn<M: Model>(&self, f: impl Fn() -> Fetch + 'static) -> CountQuery<M> {
        let q = CountQuery::<M> {
            state: self.install_state::<M>(f(), None, true),
            container: self.clone(),
            _p: std::marker::PhantomData,
        };
        let q2 = q.clone();
        day_reactive::bind(f, move |fetch| q2.set_fetch(fetch.clone()));
        q
    }

    /// Raw SQL, declared: a read-only SELECT returning ids, re-run whenever a flush touches
    /// one of the named tables. The price of the escape hatch is whole-query invalidation.
    pub fn query_raw<M: Model>(
        &self,
        sql: impl Into<String>,
        params: Vec<Value>,
        tables: &[&str],
    ) -> Query<M> {
        self.install_query::<M>(
            Fetch::new().filter(Pred::Raw(String::new(), Vec::new())),
            Some(RawQuery {
                sql: sql.into(),
                params,
                tables: tables.iter().map(|s| s.to_string()).collect(),
            }),
        )
    }

    /// Opt into undo: ONE history over every store this container manages, `levels` deep.
    /// Undo/redo replay flows through the same change pipeline as the user's edits, so
    /// autosave writes the inverse statements and live queries hear rows come back. Call it
    /// once, after open; clear it on migration (`UndoStack::clear`). While a stack is
    /// installed, deletes materialize the rows they remove (a cascade included) so the
    /// history can restore them.
    pub fn undo(&self, levels: usize) -> day_model::UndoStack {
        let stack = day_model::UndoStack::new(levels);
        for hooks in self.inner.tables.borrow().values() {
            (hooks.watch_undo)(&stack);
        }
        stack
    }

    /// The driver's connection, directly — for maintenance, imports, an extension's own
    /// statements. Pending changes flush first; writes made here bypass the change log, so
    /// call [`ModelContainer::rescan`] afterward if they touched Day's tables.
    pub fn with_connection<R>(&self, f: impl FnOnce(&mut dyn SqliteConnection) -> R) -> R {
        let _ = self.save();
        f(self.conn().as_mut())
    }

    /// Re-read the RESIDENT rows from the file and re-run every query — the recovery from
    /// writes that bypassed the change log ([`ModelContainer::with_connection`], another
    /// process). O(working set + query results), never O(table).
    pub fn rescan(&self) -> Result<(), DbError> {
        self.save()?;
        self.refresh_resident()?;
        self.invalidate_relation_memos();
        self.requery_all();
        // The cache now equals the file; a later check_external should not re-merge for
        // whatever external commits this refresh already picked up.
        if self.inner.caps.external_changes {
            self.inner.data_version.set(self.file_data_version()?);
        }
        Ok(())
    }

    /// Look for OTHER connections' committed writes — another process, a sync engine, a CLI —
    /// and merge what changed. Detection is one `PRAGMA data_version` (the counter moves only
    /// when another connection commits, never for this one's own writes), so this is cheap
    /// enough to wire to app foreground, window focus, or a timer.
    ///
    /// When the counter moved, pending local edits flush first; then the RESIDENT rows are
    /// re-read and diffed — changed fields announce per column, disappearances take the
    /// structural path, every live query re-derives — all authored
    /// [`ModelContainer::EXTERNAL_AUTHOR`], so the autosave fold declines the echo and an
    /// installed undo stack skips them (another author's writes are not the user's history).
    /// Rows that arrived elsewhere become visible through the re-run queries and fault in
    /// like any other row. A row another connection rewrote arrives whole, so its
    /// `#[model(transient)]` fields reset to their defaults, exactly as at fault.
    ///
    /// Returns whether anything arrived. On a driver without detection
    /// ([`Capabilities::external_changes`]) this is `Ok(false)`, honestly; writes made
    /// through [`ModelContainer::with_connection`] are this connection's own and stay
    /// [`ModelContainer::rescan`]'s job.
    pub fn check_external(&self) -> Result<bool, DbError> {
        if !self.inner.caps.external_changes {
            return Ok(false);
        }
        let Some(current) = self.file_data_version()? else {
            return Ok(false);
        };
        if self.inner.data_version.get() == Some(current) {
            return Ok(false);
        }
        self.inner.data_version.set(Some(current));
        // Local edits flush first, so the diff compares the file against a cache with nothing
        // pending — an unflushed local edit must not read as the other side's deletion.
        self.save()?;
        let changed = self.refresh_resident()?;
        self.invalidate_relation_memos();
        self.requery_all();
        Ok(changed)
    }

    /// `PRAGMA data_version` — `None` where the engine did not answer (the Recorder).
    fn file_data_version(&self) -> Result<Option<i64>, DbError> {
        let mut v = None;
        self.conn().query("PRAGMA data_version", &[], &mut |row| {
            if let Ok(i) = row.get(0).as_int() {
                v = Some(i);
            }
        })?;
        Ok(v)
    }

    /// Re-select every RESIDENT row and feed the differences through the caches — per-field
    /// announcements, external deletions, all authored [`ModelContainer::EXTERNAL_AUTHOR`].
    fn refresh_resident(&self) -> Result<bool, DbError> {
        struct RefreshJob {
            resident_keys: Rc<dyn Fn() -> Vec<u64>>,
            refresh: RefreshFn,
            columns: String,
            table: &'static str,
            keys_where: KeysWhere,
        }
        let jobs: Vec<RefreshJob> = {
            let tables = self.inner.tables.borrow();
            tables
                .values()
                .map(|h| RefreshJob {
                    resident_keys: h.resident_keys.clone(),
                    refresh: h.refresh.clone(),
                    columns: h.columns.join(", "),
                    table: h.table,
                    keys_where: keys_where_of(h),
                })
                .collect()
        };
        let mut changed = false;
        for RefreshJob {
            resident_keys,
            refresh,
            columns,
            table,
            keys_where,
        } in jobs
        {
            let resident = resident_keys();
            if resident.is_empty() {
                continue;
            }
            let mut rows: Vec<Vec<Value>> = Vec::new();
            for chunk in resident.chunks(MAX_BOUND_PARAMS / 2) {
                let (clause, params) = keys_where(chunk);
                self.conn().query(
                    &format!("SELECT {columns} FROM {table} WHERE {clause}"),
                    &params,
                    &mut |row| {
                        let n = Row::len(row);
                        rows.push((0..n).map(|i| row.get(i)).collect());
                    },
                )?;
            }
            // The connection borrow is released before refresh announces — a sink hearing
            // these changes may read back through SQL.
            if refresh(rows)? {
                changed = true;
            }
        }
        Ok(changed)
    }

    fn install_state<M: Model>(
        &self,
        fetch: Fetch,
        raw: Option<RawQuery>,
        count_only: bool,
    ) -> Rc<QueryState> {
        let store = self.cache::<M>();
        let state = Rc::new(QueryState {
            store_id: store.store_id(),
            table: M::TABLE,
            watches: RefCell::new(Vec::new()),
            local_extra: RefCell::new(Vec::new()),
            set: RefCell::new(ResultSet::new(fetch.clone())),
            version: day_reactive::Scope::detached().enter(|| day_reactive::Signal::new(0)),
            pending: RefCell::new(QueryEvents::None),
            needs_sql: Cell::new(false),
            raw,
            count_only,
            count: Cell::new(0),
        });
        self.rewire_watches(&state, &fetch);
        self.inner.queries.borrow_mut().push(Rc::downgrade(&state));
        // The seed: flush anything pending (with autosave; without it the query answers from
        // the last save, documented), then one SQL round trip.
        if self.inner.autosave.get() && !self.inner.dirty.borrow().is_empty() {
            let _ = self.save();
        }
        if count_only {
            state.count.set(self.answer_count(&state));
        } else {
            let ids = self.answer(&state);
            state.set.borrow_mut().reset(ids);
        }
        state
    }

    fn install_query<M: Model>(&self, fetch: Fetch, raw: Option<RawQuery>) -> Query<M> {
        Query {
            state: self.install_state::<M>(fetch, raw, false),
            container: self.clone(),
            _p: std::marker::PhantomData,
        }
    }

    /// (Re)compute a query's relation subscriptions from its fetch.
    fn rewire_watches(&self, state: &Rc<QueryState>, fetch: &Fetch) {
        let deps = fetch.dependencies();
        let mut watches: Vec<RelWatch> = Vec::new();
        let mut local_extra: Vec<&'static str> = Vec::new();
        let relations = self.inner.relations.borrow();
        let joins = self.inner.joins.borrow();
        for dep in &deps.related {
            let mut resolved = false;
            for r in relations.iter() {
                if r.parent_table == dep.owner && r.parent_field == dep.field {
                    watches.push(RelWatch {
                        store: r.child_store,
                        join_store: None,
                        fields: {
                            let mut f = vec![r.fk_field];
                            if let Some(o) = r.ordered {
                                f.push(o);
                            }
                            f
                        },
                        columns: dep.columns.clone(),
                    });
                    resolved = true;
                    break;
                }
                if r.child_table == dep.owner && r.fk_field == dep.field {
                    // The membership column lives on the QUERY's own table: a rewrite of the
                    // reference must mark it stale even though no predicate reads it.
                    if dep.owner == state.table && !local_extra.contains(&r.fk_col) {
                        local_extra.push(r.fk_col);
                    }
                    watches.push(RelWatch {
                        store: r.parent_store,
                        join_store: None,
                        fields: Vec::new(),
                        columns: dep.columns.clone(),
                    });
                    resolved = true;
                    break;
                }
            }
            if resolved {
                continue;
            }
            for j in joins.iter() {
                let side = if j.a_table == dep.owner && j.a_field == dep.field {
                    Some(j.b_store)
                } else if j.b_table == dep.owner && j.b_field.get() == Some(dep.field) {
                    Some(j.a_store)
                } else {
                    None
                };
                if let Some(other) = side {
                    watches.push(RelWatch {
                        store: other,
                        join_store: Some(j.join_store),
                        fields: Vec::new(),
                        columns: dep.columns.clone(),
                    });
                    break;
                }
            }
        }
        *state.watches.borrow_mut() = watches;
        *state.local_extra.borrow_mut() = local_extra;
    }

    /// One announced change: decide, per live query, whether it can move the result — and
    /// mark stale where it can. No SQL runs here; the requery happens once, after the flush.
    fn mark_queries_stale(&self, change: &day_model::Change) {
        let Some(&store) = change.components.first() else {
            return;
        };
        let states: Vec<Rc<QueryState>> = {
            let mut queries = self.inner.queries.borrow_mut();
            queries.retain(|w| w.strong_count() > 0);
            queries.iter().filter_map(|w| w.upgrade()).collect()
        };
        if states.is_empty() {
            return;
        }
        let key = change.components.get(1).copied();
        if key == Some(day_model::STRUCTURE) {
            return; // the shape path duplicates the row paths
        }
        // The change log speaks FIELD names; predicates speak COLUMN names.
        let (column, changed_table): (Option<String>, Option<&'static str>) = {
            let tables = self.inner.tables.borrow();
            let hooks = tables.get(&store);
            let column = if change.components.len() >= 3 {
                hooks.map(|h| {
                    h.fields
                        .iter()
                        .position(|f| f == change.label)
                        .map(|i| h.columns[i].clone())
                        // A transient field: never a dependency column.
                        .unwrap_or_else(|| "\u{0}".to_string())
                })
            } else {
                None
            };
            (column, hooks.map(|h| h.table))
        };
        let column = column.as_deref();

        for state in &states {
            if state.needs_sql.get() {
                continue; // already stale; nothing to learn
            }
            if let Some(raw) = &state.raw {
                if changed_table.is_some_and(|t| raw.tables.iter().any(|r| r == t)) {
                    state.needs_sql.set(true);
                }
                continue;
            }
            // BOTH gates apply: a self-referential relation makes the query's own store a
            // related store too, so the own-table check must not shadow the watch check.
            let own_stale = state.store_id == store
                && match (key, change.op) {
                    (None, _) => true, // a wholesale rewrite: anything may have moved
                    (Some(_), Op::Insert) | (Some(_), Op::Delete) => true,
                    (Some(_), Op::Set) => match column {
                        // A row-level merge without a field name: assume the worst.
                        None => true,
                        Some(c) => {
                            let set = state.set.borrow();
                            set.deps().touches_local(c) || state.local_extra.borrow().contains(&c)
                        }
                    },
                    (Some(_), Op::Move) => false, // user order is not a query's business
                };
            let stale = own_stale || {
                let watches = state.watches.borrow();
                watches.iter().any(|w| {
                    if w.join_store == Some(store) {
                        return true; // every link/unlink/reposition is a membership change
                    }
                    if w.store != store {
                        return false;
                    }
                    match (key, change.op) {
                        (None, _) => true,
                        (Some(_), Op::Insert) | (Some(_), Op::Delete) => true,
                        (Some(_), Op::Set) => {
                            w.fields.contains(&change.label)
                                || column.is_some_and(|c| w.columns.contains(&c))
                        }
                        (Some(_), Op::Move) => false,
                    }
                })
            };
            if stale {
                state.needs_sql.set(true);
            }
        }
    }

    /// After a flush: every stale query re-derives (plus raw queries whose tables the flush
    /// touched), each answer diffed into list deltas.
    fn run_deferred_requeries(&self, stmts: &[(String, Vec<Value>)]) {
        let states: Vec<Rc<QueryState>> = self
            .inner
            .queries
            .borrow()
            .iter()
            .filter_map(|w| w.upgrade())
            .collect();
        for state in states {
            let mut due = state.needs_sql.replace(false);
            if let Some(raw) = &state.raw
                && !due
            {
                due = stmts
                    .iter()
                    .any(|(sql, _)| raw.tables.iter().any(|t| statement_touches(sql, t)));
            }
            if due {
                self.requery(&state);
            }
        }
    }

    /// Mark every query stale and requery now — the external-change and rescan path.
    fn requery_all(&self) {
        let states: Vec<Rc<QueryState>> = self
            .inner
            .queries
            .borrow()
            .iter()
            .filter_map(|w| w.upgrade())
            .collect();
        for state in states {
            state.needs_sql.set(false);
            self.requery(&state);
        }
    }

    /// One requery: answer through the engine, adopt, narrate.
    fn requery(&self, state: &Rc<QueryState>) {
        if state.count_only {
            let n = self.answer_count(state);
            if n != state.count.get() {
                state.count.set(n);
                state
                    .version
                    .set(state.version.get_untracked().wrapping_add(1));
            }
            return;
        }
        let ids = self.answer(state);
        let change = state.set.borrow_mut().adopt(ids);
        match change {
            SetChange::Same => {}
            SetChange::Deltas(d) => {
                state.pending.borrow_mut().push(&d);
                state
                    .version
                    .set(state.version.get_untracked().wrapping_add(1));
            }
            SetChange::Reload => {
                state.pending.borrow_mut().reload();
                state
                    .version
                    .set(state.version.get_untracked().wrapping_add(1));
            }
        }
    }

    /// Answer a COUNT-shaped fetch: one `SELECT COUNT(*)`, capped by the fetch's limit
    /// (a limited set's length is `min(count, limit)`). The fallback form counts its
    /// re-checked rows the same way.
    fn answer_count(&self, state: &Rc<QueryState>) -> usize {
        if let Some(raw) = &state.raw {
            return self.select_id_column(&raw.sql, &raw.params).len();
        }
        let fetch = state.set.borrow().fetch().clone();
        let cap = fetch.limit.unwrap_or(usize::MAX);
        let snapshot = self.sql_snapshot();
        match compile_count(state.table, &fetch, &snapshot) {
            Ok(q) => {
                let mut n = 0usize;
                if let Err(e) = self.conn().query(&q.sql, &q.params, &mut |row| {
                    n = row.get(0).as_int().unwrap_or(0).max(0) as usize;
                }) {
                    self.inner.error.set(Some(e.to_string()));
                    return 0;
                }
                n.min(cap)
            }
            Err(CompileErr::NeedsFold) => match compile_fallback(state.table, &fetch, &snapshot) {
                Ok((q, cols)) => {
                    let mut n = 0usize;
                    if let Err(e) = self.conn().query(&q.sql, &q.params, &mut |row| {
                        let len = Row::len(row);
                        let values: Vec<Value> = (0..len).map(|i| row.get(i)).collect();
                        let Some(h) = value_to_handle(&values[0]) else {
                            return;
                        };
                        let view = FallbackRow {
                            cols: &cols,
                            values: &values,
                        };
                        if fetch.pred.eval(h, &view) {
                            n += 1;
                        }
                    }) {
                        self.inner.error.set(Some(e.to_string()));
                        return 0;
                    }
                    n.min(cap)
                }
                Err(e) => {
                    self.inner.error.set(Some(e.message()));
                    0
                }
            },
            Err(e) => {
                self.inner.error.set(Some(e.message()));
                0
            }
        }
    }

    /// Answer a query's fetch through the engine — the compiled form, the fallback form for
    /// drivers without `day_fold`, or the raw statement.
    fn answer(&self, state: &Rc<QueryState>) -> Vec<u64> {
        if let Some(raw) = &state.raw {
            return self.select_id_column(&raw.sql, &raw.params);
        }
        let fetch = state.set.borrow().fetch().clone();
        let snapshot = self.sql_snapshot();
        match compile_fetch(state.table, &fetch, &snapshot) {
            Ok(q) => self.select_id_column(&q.sql, &q.params),
            Err(CompileErr::NeedsFold) => match compile_fallback(state.table, &fetch, &snapshot) {
                Ok((q, cols)) => {
                    let mut ids = Vec::new();
                    let mut err = None;
                    if let Err(e) = self.conn().query(&q.sql, &q.params, &mut |row| {
                        let n = Row::len(row);
                        let values: Vec<Value> = (0..n).map(|i| row.get(i)).collect();
                        let Some(h) = value_to_handle(&values[0]) else {
                            return;
                        };
                        let view = FallbackRow {
                            cols: &cols,
                            values: &values,
                        };
                        if fetch.pred.eval(h, &view) {
                            ids.push(h);
                        }
                    }) {
                        err = Some(e);
                    }
                    if let Some(e) = err {
                        self.inner.error.set(Some(e.to_string()));
                        return Vec::new();
                    }
                    if let Some(n) = fetch.limit {
                        ids.truncate(n);
                    }
                    ids
                }
                Err(e) => {
                    self.inner.error.set(Some(e.message()));
                    Vec::new()
                }
            },
            Err(e) => {
                self.inner.error.set(Some(e.message()));
                Vec::new()
            }
        }
    }

    /// A point-in-time view of names for the SQL compiler: tables, keys, shadows, wired
    /// relations, and whether the driver folds case.
    fn sql_snapshot(&self) -> SqlSnapshot {
        let tables = self.inner.tables.borrow();
        let relations = self.inner.relations.borrow();
        let joins = self.inner.joins.borrow();
        SqlSnapshot {
            tables: tables
                .values()
                .map(|h| TableInfo {
                    table: h.table.to_string(),
                    key: h.key_cols[0].clone(),
                    fts: h.fts.clone(),
                    spatial: h.spatial.clone(),
                })
                .collect(),
            rels: relations
                .iter()
                .map(|r| RelInfo {
                    parent_table: r.parent_table,
                    parent_field: r.parent_field,
                    parent_key: r.parent_key_col,
                    child_table: r.child_table,
                    child_key: r.child_key_col,
                    fk_field: r.fk_field,
                    fk_col: r.fk_col,
                })
                .collect(),
            joins: joins
                .iter()
                .map(|j| JoinInfo {
                    join_table: j.join_table,
                    a_table: j.a_table,
                    a_field: j.a_field,
                    a_key: j.a_key_col,
                    b_table: j.b_table,
                    b_field: j.b_field.get(),
                    b_key: j.b_key_col,
                    parent_col: j.parent_col,
                    child_col: j.child_col,
                })
                .collect(),
            fold: self.inner.caps.unicode_fold,
        }
    }
}

struct TableInfo {
    table: String,
    key: String,
    fts: Option<String>,
    spatial: Option<(String, String, String)>,
}

struct RelInfo {
    parent_table: &'static str,
    parent_field: &'static str,
    parent_key: &'static str,
    child_table: &'static str,
    child_key: &'static str,
    fk_field: &'static str,
    fk_col: &'static str,
}

struct JoinInfo {
    join_table: &'static str,
    a_table: &'static str,
    a_field: &'static str,
    a_key: &'static str,
    b_table: &'static str,
    b_field: Option<&'static str>,
    b_key: &'static str,
    parent_col: &'static str,
    child_col: &'static str,
}

struct SqlSnapshot {
    tables: Vec<TableInfo>,
    rels: Vec<RelInfo>,
    joins: Vec<JoinInfo>,
    fold: bool,
}

impl SqlIndex for SqlSnapshot {
    fn relation(&self, owner: &str, field: &str) -> Option<RelSql> {
        for r in &self.rels {
            if r.parent_table == owner && r.parent_field == field {
                return Some(RelSql::Children {
                    target_key: r.child_key.to_string(),
                    fk_col: r.fk_col.to_string(),
                    owner_key: r.parent_key.to_string(),
                });
            }
            if r.child_table == owner && r.fk_field == field {
                return Some(RelSql::Referent {
                    target_key: r.parent_key.to_string(),
                    fk_col: r.fk_col.to_string(),
                });
            }
        }
        for j in &self.joins {
            if j.a_table == owner && j.a_field == field {
                return Some(RelSql::Join {
                    join_table: j.join_table.to_string(),
                    owner_col: j.parent_col.to_string(),
                    target_col: j.child_col.to_string(),
                    owner_key: j.a_key.to_string(),
                    target_key: j.b_key.to_string(),
                });
            }
            if j.b_table == owner && j.b_field == Some(field) {
                return Some(RelSql::Join {
                    join_table: j.join_table.to_string(),
                    owner_col: j.child_col.to_string(),
                    target_col: j.parent_col.to_string(),
                    owner_key: j.b_key.to_string(),
                    target_key: j.a_key.to_string(),
                });
            }
        }
        None
    }

    fn key_of(&self, table: &str) -> Option<String> {
        self.tables
            .iter()
            .find(|t| t.table == table)
            .map(|t| t.key.clone())
    }

    fn fts_of(&self, table: &str) -> Option<String> {
        self.tables
            .iter()
            .find(|t| t.table == table)
            .and_then(|t| t.fts.clone())
    }

    fn geo_of(&self, table: &str, lat: &str, lon: &str) -> Option<String> {
        self.tables
            .iter()
            .find(|t| t.table == table)
            .and_then(|t| t.spatial.as_ref())
            .filter(|(a, o, _)| a == lat && o == lon)
            .map(|(_, _, g)| g.clone())
    }

    fn unicode_fold(&self) -> bool {
        self.fold
    }
}

/// Whether a statement's target table is `table` — a plain-text check over the SQL the fold
/// itself produced (`INSERT INTO t … ON CONFLICT`, `UPDATE t SET …`, `DELETE FROM t …`).
fn statement_touches(sql: &str, table: &str) -> bool {
    for prefix in [
        "INSERT INTO ",
        "INSERT OR REPLACE INTO ",
        "UPDATE ",
        "DELETE FROM ",
    ] {
        if let Some(rest) = sql.strip_prefix(prefix) {
            return rest.split([' ', '(']).next() == Some(table);
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Query as a list row source (feature `pieces`)
// ---------------------------------------------------------------------------

#[cfg(feature = "pieces")]
mod pieces_glue {
    use std::cell::RefCell;

    use day_model::{Elem, Keyed, Store};
    use day_pieces::{ModelSlot, RowConn, RowSource};

    use crate::{Delta, Model, Query, QueryEvents};

    /// How far around a bound row the list source faults in one batch — the price of showing
    /// row N is one `SELECT` that also covers the rows about to scroll in.
    const FAULT_BEHIND: usize = 16;
    const FAULT_AHEAD: usize = 64;

    /// `list(query, row)` — the query's ids are the display order, rows FAULT IN as the list
    /// binds them (a window at a time), and set changes arrive as row deltas the native list
    /// can animate.
    impl<M: Model> RowSource for Query<M> {
        type Slot = ModelSlot<M>;
        type Ref = Elem<M>;
        type Conn = QueryConn<M>;
        fn connect(self) -> QueryConn<M> {
            let store = self.container.cache::<M>();
            QueryConn {
                query: self,
                store,
                keys: RefCell::new(Vec::new()),
            }
        }
    }

    pub struct QueryConn<M: Model> {
        query: Query<M>,
        store: Store<Keyed<M>>,
        keys: RefCell<Vec<u64>>,
    }

    impl<M: Model> QueryConn<M> {
        /// Make the row at `index` (and its neighborhood) resident before anything binds it.
        fn fault_window(&self, index: usize) {
            let len = self.keys.borrow().len();
            let start = index.saturating_sub(FAULT_BEHIND);
            let end = (index + FAULT_AHEAD).min(len);
            self.query.materialize(start..end);
        }
    }

    impl<M: Model> RowConn for QueryConn<M> {
        type Slot = ModelSlot<M>;
        type Ref = Elem<M>;

        fn refresh(&self) -> Vec<u64> {
            let keys: Vec<u64> = self.query.ids().iter().map(|id| id.handle()).collect();
            *self.keys.borrow_mut() = keys.clone();
            keys
        }
        fn len(&self) -> usize {
            self.keys.borrow().len()
        }
        fn token_at(&self, index: usize) -> u64 {
            self.keys.borrow().get(index).copied().unwrap_or(0)
        }
        fn tokens_now(&self) -> Vec<u64> {
            self.keys.borrow().clone()
        }
        fn slot_at(&self, index: usize) -> Option<ModelSlot<M>> {
            let key = self.keys.borrow().get(index).copied()?;
            if !self.store.with_untracked(|k| k.get(key).is_some()) {
                self.fault_window(index);
            }
            Some(ModelSlot::for_key(self.store, key))
        }
        fn rebind(&self, slot: &ModelSlot<M>, index: usize) {
            if let Some(key) = self.keys.borrow().get(index).copied() {
                if !self.store.with_untracked(|k| k.get(key).is_some()) {
                    self.fault_window(index);
                }
                slot.rebind_key(key);
            }
        }
        fn select_ref(&self, index: usize) -> Option<Elem<M>> {
            let key = self.keys.borrow().get(index).copied()?;
            if !self.store.with_untracked(|k| k.get(key).is_some()) {
                self.fault_window(index);
            }
            Some(self.store.elem(key))
        }
        fn values_flow_by_reload(&self) -> bool {
            false // row values flow through the store's own per-field notifications
        }
        fn take_row_events(&self) -> Option<Vec<day_spec::props::RowDelta>> {
            match self.query.take_events() {
                QueryEvents::Deltas(d) => Some(
                    d.into_iter()
                        .map(|d| match d {
                            Delta::Insert(i, _) => day_spec::props::RowDelta::Insert(i),
                            Delta::Remove(i, _) => day_spec::props::RowDelta::Remove(i),
                            Delta::Move(from, to, _) => day_spec::props::RowDelta::Move(from, to),
                        })
                        .collect(),
                ),
                // Reload means "the whole set moved"; None here means the same to the list.
                QueryEvents::Reload | QueryEvents::None => None,
            }
        }
        fn commit_move(&self, _from: usize, _to: usize) {
            // A sorted result set does not take user reordering; the app decides what a drag
            // means and writes the model, which flows back through the query.
        }
        fn commit_delete(&self, index: usize) {
            // The native delete affordance: drop from the local snapshot; the app's on_delete
            // writes the store, and the query's own delta follows as the echo.
            let mut keys = self.keys.borrow_mut();
            if index < keys.len() {
                keys.remove(index);
            }
        }
    }
}

#[cfg(feature = "pieces")]
pub use pieces_glue::QueryConn;
