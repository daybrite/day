// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! SQLite persistence for the observable model (docs/persistence.md).
//!
//! The write half is not a new mechanism: it is day-model's change log, folded. A
//! [`ModelContainer`] opens a database through a pluggable [`SqliteDriver`], creates or
//! migrates each model's table, loads the rows into an ordinary `Store<Keyed<M>>`, and then
//! watches the change log the UI already produces. At the end of any turn that touched a store
//! (autosave, the default) the accumulated changes fold into the smallest statement list that
//! expresses them — twenty keystrokes into one field is one `UPDATE`; a row inserted and then
//! filled is one `INSERT`; a row deleted is one `DELETE`, whatever preceded it.
//!
//! The driver is a trait so the ENGINE is the app's choice: the built-in [`Sqlite`] driver
//! (feature `driver-rusqlite`, on by default) compiles a bundled SQLite, links the system one
//! (`system`), or builds SQLCipher (`cipher`); the [`Recorder`] answers from fixtures and
//! records every statement, which is what keeps persistence assertable headlessly — a test can
//! check the SQL a UI action produced without a database on disk.
//!
//! Typed live queries are [`ModelContainer::query`] and friends; another connection's committed
//! writes arrive through [`ModelContainer::check_external`]. What this version deliberately does
//! not do yet: fault rows lazily — a container LOADS each table at open, the document pattern.
//! Row identity is the model's `#[model(id)]` key, stored as `INTEGER`; display order is a
//! projection concern and is not persisted.

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use day_model::{Identified, Key, Keyed, ModelId, Op, Store};

mod queries;
pub use queries::{
    Col, Delta, Fetch, FtsRef, GeoRect, GeoRef, LiveSet, Outcome, Pred, RowView, RowsView, Sort,
    compare_values, encode_column, rank,
};

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

    /// Rows a table's load `SELECT` will answer with.
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
            // (its reads just come back empty unless a fixture answers them).
            full_text_search: true,
            rtree: true,
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
// Column values and codecs (docs/persistence.md; the plan's §13)
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
type ReloadFn = Rc<dyn Fn(Vec<Vec<Value>>) -> Result<(), DbError>>;
type MergeFn = Rc<dyn Fn(Vec<Vec<Value>>) -> Result<bool, DbError>>;
/// A key handle read back out of a key-column-shaped row (one column, or a join's pair).
type KeyFromRow = Rc<dyn Fn(&dyn Row) -> Option<u64>>;
/// A WHERE clause plus its parameters, addressing one row by handle.
type KeyWhere = Rc<dyn Fn(u64) -> (String, Vec<Value>)>;

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
enum DirtyRow {
    Insert,
    Update(Vec<&'static str>),
    Delete,
}

#[derive(Default)]
struct DirtyState {
    /// (store, key) → pending statement kind, in first-touch order.
    rows: HashMap<(u64, u64), DirtyRow>,
    order: Vec<(u64, u64)>,
    /// Stores whose WHOLE value was rewritten (a wholesale `Store::update`) — flushed as a
    /// full resync: upsert every row, delete the gone ones.
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
/// their pair-keyed internal store, which is why keys are plural and clause-shaped here.
struct TableHooks {
    table: &'static str,
    /// The key column(s), in order: the key SELECT list, the upsert's conflict target, and
    /// what its DO UPDATE leaves alone. One entry everywhere but join tables.
    key_cols: Vec<String>,
    /// WHERE clause + params addressing one row by its key handle.
    key_where: KeyWhere,
    /// A key handle read back out of a `key_cols`-shaped row.
    key_from_row: KeyFromRow,
    columns: Vec<String>,
    /// Same order as `columns`; what a change's label matches against.
    fields: Vec<String>,
    /// Current row values by key, read from the store at flush time — the change log carries
    /// WHICH rows and columns moved, never their contents.
    row_for: Rc<dyn Fn(u64) -> Option<Vec<Value>>>,
    /// Every (key, row) currently in the store, for full resyncs.
    all_rows: Rc<dyn Fn() -> Vec<(u64, Vec<Value>)>>,
    /// Replace the store's contents from raw rows — `rescan`'s write-back path.
    reload: ReloadFn,
    /// Diff raw rows against the store and feed only the differences through — precise
    /// per-field announcements, authored [`ModelContainer::EXTERNAL_AUTHOR`], never echoed
    /// back to the file. Returns whether anything differed.
    merge: MergeFn,
    /// Bring this store under an undo history — captured here because the model TYPE is known
    /// only at attach time.
    watch_undo: Rc<dyn Fn(&day_model::UndoStack)>,
}

struct ContainerInner {
    conn: RefCell<Box<dyn SqliteConnection>>,
    caps: Capabilities,
    /// store root id → hooks.
    tables: RefCell<HashMap<u64, TableHooks>>,
    /// model TypeId → the `Store<Keyed<M>>` handle, boxed.
    stores: RefCell<HashMap<TypeId, Box<dyn Any>>>,
    dirty: RefCell<DirtyState>,
    /// Live query result sets, dispatched to from the change sink. Weak: the app's `Query`
    /// handles own the state; dead entries are pruned on dispatch.
    queries: RefCell<Vec<std::rc::Weak<QueryState>>>,
    /// True while `rescan` reloads stores from the file — the sink ignores those writes (they
    /// are the database's own contents coming back, not edits to persist).
    quiet: Cell<bool>,
    /// The engine's cross-connection change counter as of the last look (`PRAGMA
    /// data_version` — it moves only when ANOTHER connection commits to the file). `None`
    /// where the driver reports no external-change detection.
    data_version: Cell<Option<i64>>,
    /// Foreign-key clauses relations contribute to their targets' tables, set before attach.
    fk_specs: RefCell<Vec<FkSpec>>,
    /// The wired relations — maintained from the change sink, read by `RelationRef`s.
    pub(crate) relations: RefCell<Vec<Rc<relations::ToOneRel>>>,
    /// The wired many-to-manys, each over its own join store.
    pub(crate) joins: RefCell<Vec<Rc<relations::JoinRel>>>,
    sink: Cell<Option<day_model::ChangeSinkId>>,
    autosave: Cell<bool>,
    /// The last autosave failure, observable by the UI (`when(container.last_error()…)`).
    error: day_reactive::Signal<Option<String>>,
    /// Guards re-entrant flushes (a turn-end firing during an explicit save).
    flushing: Cell<bool>,
}

/// An open database and the stores loaded from it. Clone is shallow — clones share the
/// connection and the dirty state.
#[derive(Clone)]
pub struct ModelContainer {
    inner: Rc<ContainerInner>,
}

impl ModelContainer {
    /// Open through `driver`, migrate, and load every model in `schema`. Autosave is on: any
    /// turn that touched a loaded store flushes at its end.
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
                quiet: Cell::new(false),
                data_version: Cell::new(None),
                fk_specs: RefCell::new(Vec::new()),
                relations: RefCell::new(Vec::new()),
                joins: RefCell::new(Vec::new()),
                sink: Cell::new(None),
                autosave: Cell::new(true),
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
        // Relations wire once every table is attached — both ends exist, the indexes seed
        // from the loaded rows, and only then does the sink go live.
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

    /// The loaded store for `M` — an ordinary day-model store; every binding and list source
    /// works on it unchanged. Panics only if `M` was not in the container's `schema!`, which is
    /// a wiring bug worth stopping on.
    pub fn store<M: Model>(&self) -> Store<Keyed<M>> {
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

    /// Autosave on/off (default on). Off, changes accumulate until [`ModelContainer::save`].
    pub fn set_autosave(&self, on: bool) {
        self.inner.autosave.set(on);
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
                // Nothing to flush — but an SQL-backed query may still be waiting on its
                // deferred requery (a fetch swap on a clean container lands here).
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

    // --- internals -------------------------------------------------------------------------

    fn conn(&self) -> std::cell::RefMut<'_, Box<dyn SqliteConnection>> {
        self.inner.conn.borrow_mut()
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

    /// CREATE (or lightweight-migrate) `M`'s table, load its rows, register its hooks.
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

        // Load. NULL in a NOT NULL column reads as the field's Default — that is what makes an
        // added column's old rows readable before their backfill, and it matches the model's
        // own deleted-row semantics.
        let cols = M::COLUMNS
            .iter()
            .map(|c| c.name)
            .collect::<Vec<_>>()
            .join(", ");
        let mut rows: Vec<M> = Vec::new();
        let mut decode_error: Option<DbError> = None;
        self.conn().query(
            &format!("SELECT {cols} FROM {}", M::TABLE),
            &[],
            &mut |row| match M::from_row(row) {
                Ok(m) => rows.push(m),
                Err(e) => decode_error = Some(e),
            },
        )?;
        if let Some(e) = decode_error {
            return Err(e);
        }

        let store = Store::new(Keyed::new(rows));
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

        let reload = {
            Rc::new(move |raw_rows: Vec<Vec<Value>>| -> Result<(), DbError> {
                let mut decoded = Vec::with_capacity(raw_rows.len());
                for r in raw_rows {
                    decoded.push(M::from_row(&r)?);
                }
                let fresh = Keyed::new(decoded);
                store.update("rescan", move |k| *k = fresh);
                Ok(())
            }) as Rc<dyn Fn(Vec<Vec<Value>>) -> Result<(), DbError>>
        };
        let merge = {
            Rc::new(move |raw_rows: Vec<Vec<Value>>| -> Result<bool, DbError> {
                let mut fresh: Vec<M> = Vec::with_capacity(raw_rows.len());
                for r in raw_rows {
                    fresh.push(M::from_row(&r)?);
                }
                let existing: Vec<u64> = store.with_untracked(|k| k.keys().to_vec());
                let fresh_keys: std::collections::HashSet<u64> =
                    fresh.iter().map(|m| m.handle()).collect();
                let mut changed = false;
                day_model::with_author(ModelContainer::EXTERNAL_AUTHOR, || {
                    for m in fresh {
                        let key = m.handle();
                        match store.with_untracked(|k| k.get(key).map(|old| old.to_row())) {
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
            }) as MergeFn
        };
        let watch_undo = Rc::new(move |stack: &day_model::UndoStack| stack.watch(store))
            as Rc<dyn Fn(&day_model::UndoStack)>;
        self.inner.tables.borrow_mut().insert(
            store_id,
            TableHooks {
                table: M::TABLE,
                key_cols: vec![M::KEY.to_string()],
                key_where: Rc::new(|h| (format!("{} = ?", M::KEY), vec![key_param(h)])),
                key_from_row: Rc::new(|row| value_to_handle(&row.get(0))),
                columns,
                fields,
                row_for,
                all_rows,
                reload,
                merge,
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
            self.conn().execute(
                &format!(
                    "CREATE VIRTUAL TABLE IF NOT EXISTS {t}_fts USING fts5({cols}, \
                     content={t}, content_rowid={key})"
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
                // process honors the same delete rule. Deferred, so within-transaction
                // statement order (a cascade's children, an undo's re-inserts) never trips it.
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
        for c in M::COLUMNS.iter().filter(|c| c.indexed) {
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
                if inner.quiet.get() {
                    return;
                }
                // An external merge's changes are the database's own contents arriving:
                // queries dispatch on them like any edit, but nothing goes back to the file.
                if change.author != Some(ModelContainer::EXTERNAL_AUTHOR) {
                    let tables = inner.tables.borrow();
                    inner
                        .dirty
                        .borrow_mut()
                        .note(change, |store| tables.contains_key(&store));
                }
                let container = ModelContainer { inner };
                container.dispatch_to_queries(change);
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
    /// first-touch order. Row values come from the stores NOW — the change log carried which
    /// rows and columns moved, never their contents. Reads only; [`ModelContainer::flush`]
    /// executes the list in one transaction.
    fn fold(&self, dirty: &DirtyState) -> Result<Vec<(String, Vec<Value>)>, DbError> {
        let tables = self.inner.tables.borrow();
        let mut stmts: Vec<(String, Vec<Value>)> = Vec::new();

        // Wholesale rewrites first: resync the entire table against the store.
        for store_id in &dirty.full {
            let Some(hooks) = tables.get(store_id) else {
                continue;
            };
            let rows = (hooks.all_rows)();
            // Stored key values, with the raw first column kept: a row whose key cannot map
            // to a handle (foreign shape, corruption) still deletes, by its own raw value.
            let mut db_keys: Vec<(Option<u64>, Value)> = Vec::new();
            self.conn().query(
                &format!("SELECT {} FROM {}", hooks.key_cols.join(", "), hooks.table),
                &[],
                &mut |row| db_keys.push(((hooks.key_from_row)(row), row.get(0))),
            )?;
            for (_, row) in &rows {
                stmts.push(upsert_stmt(hooks, row));
            }
            for (h, raw) in db_keys
                .iter()
                .filter(|(h, _)| !matches!(h, Some(h) if rows.iter().any(|(key, _)| key == h)))
            {
                match h {
                    Some(h) => {
                        let (clause, params) = (hooks.key_where)(*h);
                        stmts.push((
                            format!("DELETE FROM {} WHERE {clause}", hooks.table),
                            params,
                        ));
                    }
                    None => stmts.push((
                        format!(
                            "DELETE FROM {} WHERE {} = ?",
                            hooks.table, hooks.key_cols[0]
                        ),
                        vec![raw.clone()],
                    )),
                }
            }
        }

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
            match state {
                DirtyRow::Delete => {
                    let (clause, params) = (hooks.key_where)(key);
                    stmts.push((
                        format!("DELETE FROM {} WHERE {clause}", hooks.table),
                        params,
                    ));
                }
                DirtyRow::Insert => {
                    if let Some(row) = (hooks.row_for)(key) {
                        stmts.push(upsert_stmt(hooks, &row));
                    }
                }
                DirtyRow::Update(cols) => {
                    let Some(row) = (hooks.row_for)(key) else {
                        continue; // deleted since — its own Delete entry handles it
                    };
                    if cols.is_empty() {
                        // A row-level replacement named no columns — write them all.
                        stmts.push(upsert_stmt(hooks, &row));
                        continue;
                    }
                    let mut sets: Vec<String> = Vec::with_capacity(cols.len());
                    let mut params: Vec<Value> = Vec::with_capacity(cols.len() + 1);
                    for c in cols {
                        let Some(i) = hooks.fields.iter().position(|n| n == c) else {
                            continue; // a transient field's label — never a column
                        };
                        sets.push(format!("{} = ?", hooks.columns[i]));
                        params.push(row.get(i));
                    }
                    if params.is_empty() {
                        continue; // only transient fields changed
                    }
                    let (clause, mut where_params) = (hooks.key_where)(key);
                    params.append(&mut where_params);
                    stmts.push((
                        format!(
                            "UPDATE {} SET {} WHERE {clause}",
                            hooks.table,
                            sets.join(", "),
                        ),
                        params,
                    ));
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
// Live queries (docs/persistence.md; queries.rs holds the pure maintainer)
// ---------------------------------------------------------------------------

/// What a query's consumers have not yet seen. `Deltas` can animate a list row by row;
/// `Reload` means the whole set moved (a requery, a fetch swap) and a reload is honest.
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

struct QueryState {
    store_id: u64,
    table: &'static str,
    set: RefCell<LiveSet>,
    /// Bumped whenever the result set changes; `ids()`/`count()` track it.
    version: day_reactive::Signal<u64>,
    pending: RefCell<QueryEvents>,
    /// An SQL-backed fetch (raw / FTS / rank) whose answer went stale mid-turn; resolved
    /// after the flush, when the statements (and their index triggers) have landed.
    needs_sql: Cell<bool>,
    /// `query_raw` only: the statement and the tables whose commit re-runs it.
    raw: Option<RawQuery>,
}

struct RawQuery {
    sql: String,
    params: Vec<Value>,
    tables: Vec<String>,
}

/// A live, typed result set over one model's table — ids only, maintained incrementally
/// against the change log ([`LiveSet`]'s rules). Clone shares the same set.
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
        let _ = self.state.version.get();
        self.state.set.borrow().ids().len()
    }

    /// The first result, tracked.
    pub fn first(&self) -> Option<ModelId<M>> {
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
        let _ = self.state.version.get();
        self.state.set.borrow().ids().contains(&id.into().handle())
    }

    /// Untracked snapshot.
    pub fn ids_untracked(&self) -> Vec<ModelId<M>> {
        self.state
            .set
            .borrow()
            .ids()
            .iter()
            .map(|h| ModelId::from_handle(*h))
            .collect()
    }

    /// Predicate/sort evaluations so far — the cost the incremental path avoids, exposed so
    /// a page (or a test) can show it staying flat.
    pub fn evaluations(&self) -> usize {
        self.state.set.borrow().evaluations()
    }

    /// Swap the fetch (a changed filter or sort): reseeds and reloads consumers. No-op when
    /// equal to the current one, so a `query_fn` closure can re-run cheaply.
    pub fn set_fetch(&self, fetch: Fetch) {
        if *self.state.set.borrow().fetch() == fetch {
            return;
        }
        let sql_backed = !fetch.evaluable();
        {
            let mut set = self.state.set.borrow_mut();
            *set = LiveSet::new(fetch);
        }
        if sql_backed {
            // Resolve through the database once pending statements land; mid-turn the index
            // triggers have not run yet.
            self.state.needs_sql.set(true);
            let _ = self.container.save();
        } else {
            self.container.reseed_in_memory(&self.state);
        }
        self.bump(true);
    }

    /// Drain what changed since the last call — the list source's feed.
    pub fn take_events(&self) -> QueryEvents {
        std::mem::replace(&mut *self.state.pending.borrow_mut(), QueryEvents::None)
    }

    fn bump(&self, reload: bool) {
        if reload {
            self.state.pending.borrow_mut().reload();
        }
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
    /// Seed the set and keep it live against the change log.
    pub fn live(self) -> Query<M> {
        self.container.install_query::<M>(self.fetch, None)
    }
}

/// Adapter: a table's rows, as predicates read them (column name → stored value).
struct HooksRows<'a> {
    hooks: &'a TableHooks,
}

struct ColsRow<'a> {
    cols: &'a [String],
    values: Vec<Value>,
}

impl RowView for ColsRow<'_> {
    fn col(&self, c: &str) -> Option<Value> {
        self.cols
            .iter()
            .position(|n| n == c)
            .map(|i| Row::get(&self.values, i))
    }
}

impl RowsView for HooksRows<'_> {
    fn row_view(&self, key: u64) -> Option<Box<dyn RowView + '_>> {
        (self.hooks.row_for)(key).map(|values| {
            Box::new(ColsRow {
                cols: &self.hooks.columns,
                values,
            }) as Box<dyn RowView>
        })
    }
}

impl ModelContainer {
    /// Start a typed query over `M`'s rows. Panics (like [`ModelContainer::store`]) if `M` is
    /// not in this container's `schema!` — a wiring bug worth stopping on.
    pub fn query<M: Model>(&self) -> QueryBuilder<'_, M> {
        let _ = self.store::<M>();
        QueryBuilder {
            container: self,
            fetch: Fetch::new(),
            _p: std::marker::PhantomData,
        }
    }

    /// The reactive-fetch form: `f` is a computation — a query whose FETCH depends on signals
    /// (a search term, a filter toggle) re-seeds itself when they change.
    pub fn query_fn<M: Model>(&self, f: impl Fn() -> Fetch + 'static) -> Query<M> {
        let q = self.install_query::<M>(f(), None);
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

    /// Opt into undo: ONE history over every store this container loaded, `levels` deep —
    /// SwiftData's `mainContext.undoManager = UndoManager()`, as one call. Undo/redo replay
    /// flows through the same change pipeline as the user's edits, so autosave writes the
    /// inverse statements and live queries animate rows back. Call it once, after open;
    /// clear it on migration (`UndoStack::clear`).
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

    /// Reload every store from the file and re-run every query — the recovery from writes
    /// that bypassed the change log ([`ModelContainer::with_connection`], another process).
    pub fn rescan(&self) -> Result<(), DbError> {
        self.save()?;
        self.inner.quiet.set(true);
        let result = self.reload_stores();
        self.inner.quiet.set(false);
        result?;
        // Sets may have changed wholesale; reseed everything.
        let states: Vec<Rc<QueryState>> = self
            .inner
            .queries
            .borrow()
            .iter()
            .filter_map(|w| w.upgrade())
            .collect();
        for state in states {
            if state.raw.is_some() || !state.set.borrow().fetch().evaluable() {
                self.run_sql_backed(&state);
            } else {
                self.reseed_in_memory(&state);
            }
            state.pending.borrow_mut().reload();
            state
                .version
                .set(state.version.get_untracked().wrapping_add(1));
        }
        // The store now equals the file; a later check_external should not re-merge for
        // whatever external commits this reload already picked up.
        if self.inner.caps.external_changes {
            self.inner.data_version.set(self.file_data_version()?);
        }
        Ok(())
    }

    /// Look for OTHER connections' committed writes — another process, a sync engine, a CLI —
    /// and merge what changed into the loaded stores. Detection is one `PRAGMA data_version`
    /// (the counter moves only when another connection commits, never for this one's own
    /// writes), so this is cheap enough to wire to app foreground, window focus, or a timer.
    ///
    /// When the counter moved, pending local edits flush first and each table is diffed
    /// against its store; only the differences feed through — changed fields announce per
    /// column, inserts and deletes take the structural path, live queries emit their usual
    /// precise deltas, and the autosave fold declines the echo: the changes are the database's
    /// own contents, tagged [`ModelContainer::EXTERNAL_AUTHOR`], and never write back. An
    /// installed undo stack skips them too — another author's writes are not the user's
    /// history. A row another connection rewrote arrives whole, so its `#[model(transient)]`
    /// fields reset to their defaults, exactly as at load.
    ///
    /// Returns whether anything arrived. On a driver without detection
    /// ([`Capabilities::external_changes`]) this is `Ok(false)`, honestly; writes made through
    /// [`ModelContainer::with_connection`] are this connection's own and stay
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
        // Local edits flush first, so the diff compares the file against a store with nothing
        // pending — an unflushed local edit must not read as the other side's deletion.
        self.save()?;
        self.merge_from_file()
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

    /// Diff every table against the file, feeding differences through the stores' merge seam.
    /// SQL-backed queries (raw, FTS, rank) whose tables moved re-resolve afterward — their
    /// answers live in the database, so the change log alone cannot carry the merge to them.
    fn merge_from_file(&self) -> Result<bool, DbError> {
        let hooks_list: Vec<(u64, &'static str)> = self
            .inner
            .tables
            .borrow()
            .iter()
            .map(|(id, h)| (*id, h.table))
            .collect();
        let mut changed_tables: Vec<&'static str> = Vec::new();
        for (store_id, table) in hooks_list {
            let (columns, merge) = {
                let tables = self.inner.tables.borrow();
                let hooks = &tables[&store_id];
                (hooks.columns.join(", "), hooks.merge.clone())
            };
            let mut rows: Vec<Vec<Value>> = Vec::new();
            self.conn()
                .query(&format!("SELECT {columns} FROM {table}"), &[], &mut |row| {
                    let n = Row::len(row);
                    rows.push((0..n).map(|i| row.get(i)).collect());
                })?;
            if merge(rows)? {
                changed_tables.push(table);
            }
        }
        if !changed_tables.is_empty() {
            let states: Vec<Rc<QueryState>> = self
                .inner
                .queries
                .borrow()
                .iter()
                .filter_map(|w| w.upgrade())
                .collect();
            for state in states {
                let affected = match &state.raw {
                    Some(raw) => raw
                        .tables
                        .iter()
                        .any(|t| changed_tables.contains(&t.as_str())),
                    None => {
                        !state.set.borrow().fetch().evaluable()
                            && changed_tables.contains(&state.table)
                    }
                };
                if affected {
                    self.run_sql_backed(&state);
                    state.pending.borrow_mut().reload();
                    state
                        .version
                        .set(state.version.get_untracked().wrapping_add(1));
                }
            }
        }
        Ok(!changed_tables.is_empty())
    }

    fn install_query<M: Model>(&self, fetch: Fetch, raw: Option<RawQuery>) -> Query<M> {
        let store = self.store::<M>();
        let sql_backed = raw.is_some() || !fetch.evaluable();
        let state = Rc::new(QueryState {
            store_id: store.store_id(),
            table: M::TABLE,
            set: RefCell::new(LiveSet::new(fetch)),
            version: day_reactive::Scope::detached().enter(|| day_reactive::Signal::new(0)),
            pending: RefCell::new(QueryEvents::None),
            needs_sql: Cell::new(false),
            raw,
        });
        self.inner.queries.borrow_mut().push(Rc::downgrade(&state));
        let q = Query {
            state,
            container: self.clone(),
            _p: std::marker::PhantomData,
        };
        if sql_backed {
            q.state.needs_sql.set(true);
            let _ = self.save(); // resolves the deferred requery immediately when clean
            if q.state.needs_sql.get() {
                // Nothing was dirty, so save() had nothing to flush — run it now.
                self.run_sql_backed(&q.state);
                q.state.needs_sql.set(false);
            }
        } else {
            self.reseed_in_memory(&q.state);
        }
        q
    }

    /// In-memory seed over the store — the document pattern's fetch, no SQL at all.
    fn reseed_in_memory(&self, state: &Rc<QueryState>) {
        let tables = self.inner.tables.borrow();
        let Some(hooks) = tables.get(&state.store_id) else {
            return;
        };
        let keys: Vec<u64> = (hooks.all_rows)().iter().map(|(k, _)| *k).collect();
        state.set.borrow_mut().seed(&keys, &HooksRows { hooks });
    }

    /// One announced change, routed to every query on that store.
    fn dispatch_to_queries(&self, change: &day_model::Change) {
        let Some(&store) = change.components.first() else {
            return;
        };
        let states: Vec<Rc<QueryState>> = {
            let mut queries = self.inner.queries.borrow_mut();
            queries.retain(|w| w.strong_count() > 0);
            queries
                .iter()
                .filter_map(|w| w.upgrade())
                .filter(|s| s.store_id == store)
                .collect()
        };
        if states.is_empty() {
            return;
        }
        let tables = self.inner.tables.borrow();
        let Some(hooks) = tables.get(&store) else {
            return;
        };

        // A store-level change (wholesale update): every set may have moved.
        let Some(&key) = change.components.get(1) else {
            for state in &states {
                self.requery(state, hooks);
            }
            return;
        };
        if key == day_model::STRUCTURE {
            return; // the shape path duplicates the row paths
        }
        // The change log speaks FIELD names; predicates speak COLUMN names.
        let column: &str = if change.components.len() >= 3 {
            hooks
                .fields
                .iter()
                .position(|f| f == change.label)
                .map(|i| hooks.columns[i].as_str())
                .unwrap_or("\u{0}") // a transient field: never a dependency column
        } else {
            ""
        };

        for state in &states {
            let outcome =
                state
                    .set
                    .borrow_mut()
                    .apply(key, column, change.op, &HooksRows { hooks });
            match outcome {
                Outcome::Unaffected => {}
                Outcome::Changed(deltas) => {
                    state.pending.borrow_mut().push(&deltas);
                    state
                        .version
                        .set(state.version.get_untracked().wrapping_add(1));
                }
                Outcome::Requery => self.requery(state, hooks),
            }
        }
    }

    /// Resolve a Requery now if memory can answer it; defer to the post-flush hook if only
    /// the database can (raw / FTS / rank — their SQL must see this turn's statements).
    fn requery(&self, state: &Rc<QueryState>, hooks: &TableHooks) {
        if state.raw.is_none() && state.set.borrow().fetch().evaluable() {
            let before = state.set.borrow().ids().to_vec();
            let keys: Vec<u64> = (hooks.all_rows)().iter().map(|(k, _)| *k).collect();
            state.set.borrow_mut().seed(&keys, &HooksRows { hooks });
            if state.set.borrow().ids() != before.as_slice() {
                state.pending.borrow_mut().reload();
                state
                    .version
                    .set(state.version.get_untracked().wrapping_add(1));
            }
        } else {
            state.needs_sql.set(true);
        }
    }

    /// After a flush: SQL-backed queries whose dependencies moved re-run against the file.
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
            if let Some(raw) = &state.raw {
                // A raw query re-runs when a flush touched one of its declared tables.
                if !due {
                    due = stmts
                        .iter()
                        .any(|(sql, _)| raw.tables.iter().any(|t| statement_touches(sql, t)));
                }
            }
            if due {
                self.run_sql_backed(&state);
                state.pending.borrow_mut().reload();
                state
                    .version
                    .set(state.version.get_untracked().wrapping_add(1));
            }
        }
    }

    /// Answer a fetch through the database: raw SQL verbatim, or FTS candidates narrowed by
    /// the evaluable remainder in memory.
    fn run_sql_backed(&self, state: &Rc<QueryState>) {
        let ids = if let Some(raw) = &state.raw {
            self.select_ids(&raw.sql, &raw.params)
        } else {
            let fetch = state.set.borrow().fetch().clone();
            self.fts_fetch(state, &fetch)
        };
        state.set.borrow_mut().reset(ids);
    }

    fn select_ids(&self, sql: &str, params: &[Value]) -> Vec<u64> {
        let mut ids = Vec::new();
        if let Err(e) = self.conn().query(sql, params, &mut |row| {
            // Any key shape: INTEGER handles pass through, BLOB uuids and TEXT keys intern.
            if let Some(h) = value_to_handle(&row.get(0)) {
                ids.push(h);
            }
        }) {
            // A malformed FTS query or raw statement must not read as "no results": surface
            // it where autosave failures already go, and leave the set empty.
            self.inner.error.set(Some(e.to_string()));
        }
        ids
    }

    /// FTS candidates (ranked when asked), then the remaining predicate and sort in memory.
    fn fts_fetch(&self, state: &Rc<QueryState>, fetch: &Fetch) -> Vec<u64> {
        let tables = self.inner.tables.borrow();
        let Some(hooks) = tables.get(&state.store_id) else {
            return Vec::new();
        };
        let by_rank = fetch.sort.iter().any(|s| s.by_rank);
        let match_query = find_match_query(&fetch.pred);

        let mut candidates: Vec<u64> = match match_query {
            Some(q) => {
                let fts = format!("{}_fts", state.table);
                let order = if by_rank { " ORDER BY rank" } else { "" };
                self.select_ids(
                    &format!("SELECT rowid FROM {fts} WHERE {fts} MATCH ?{order}"),
                    &[Value::Text(q)],
                )
            }
            None => (hooks.all_rows)().iter().map(|(k, _)| *k).collect(),
        };
        // The evaluable remainder (Matches itself answers true in eval).
        let rows = HooksRows { hooks };
        candidates.retain(|k| {
            rows.row_view(*k)
                .map(|r| fetch.pred.eval(r.as_ref()))
                .unwrap_or(false)
        });
        if !by_rank && !fetch.sort.is_empty() {
            let mut sorter = LiveSet::new(Fetch {
                pred: Pred::Always,
                sort: fetch.sort.clone(),
                limit: None,
            });
            sorter.seed(&candidates, &rows);
            candidates = sorter.ids().to_vec();
        }
        if let Some(n) = fetch.limit {
            candidates.truncate(n);
        }
        candidates
    }

    fn reload_stores(&self) -> Result<(), DbError> {
        let hooks_list: Vec<(u64, &'static str)> = self
            .inner
            .tables
            .borrow()
            .iter()
            .map(|(id, h)| (*id, h.table))
            .collect();
        for (store_id, table) in hooks_list {
            let (columns, reload) = {
                let tables = self.inner.tables.borrow();
                let hooks = &tables[&store_id];
                (hooks.columns.join(", "), hooks.reload.clone())
            };
            let mut rows: Vec<Vec<Value>> = Vec::new();
            self.conn()
                .query(&format!("SELECT {columns} FROM {table}"), &[], &mut |row| {
                    let n = Row::len(row);
                    rows.push((0..n).map(|i| row.get(i)).collect());
                })?;
            reload(rows)?;
        }
        Ok(())
    }
}

/// Whether a statement's target table is `table` — a plain-text check over the SQL the fold
/// itself produced (`INSERT OR REPLACE INTO t …`, `UPDATE t SET …`, `DELETE FROM t …`).
fn statement_touches(sql: &str, table: &str) -> bool {
    for prefix in ["INSERT OR REPLACE INTO ", "UPDATE ", "DELETE FROM "] {
        if let Some(rest) = sql.strip_prefix(prefix) {
            return rest.split([' ', '(']).next() == Some(table);
        }
    }
    false
}

fn find_match_query(pred: &Pred) -> Option<String> {
    match pred {
        Pred::Matches { query, .. } => Some(query.clone()),
        Pred::And(a, b) | Pred::Or(a, b) => find_match_query(a).or_else(|| find_match_query(b)),
        Pred::Not(a) => find_match_query(a),
        _ => None,
    }
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

    /// `list(query, row)` — the query's ids are the display order, rows bind through
    /// [`ModelSlot`] exactly as a store source's do, and set changes arrive as row deltas the
    /// native list can animate.
    impl<M: Model> RowSource for Query<M> {
        type Slot = ModelSlot<M>;
        type Ref = Elem<M>;
        type Conn = QueryConn<M>;
        fn connect(self) -> QueryConn<M> {
            let store = self.container.store::<M>();
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
            Some(ModelSlot::for_key(self.store, key))
        }
        fn rebind(&self, slot: &ModelSlot<M>, index: usize) {
            if let Some(key) = self.keys.borrow().get(index).copied() {
                slot.rebind_key(key);
            }
        }
        fn select_ref(&self, index: usize) -> Option<Elem<M>> {
            self.keys
                .borrow()
                .get(index)
                .copied()
                .map(|k| self.store.elem(k))
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
