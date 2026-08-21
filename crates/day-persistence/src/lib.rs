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
//! What this version deliberately does not do yet: fault rows lazily (a container LOADS each
//! table at open — the document pattern), run queries (the typed builder is the next phase), or
//! watch other connections' writes. Row identity is the model's `#[model(id)]` key, stored as
//! `INTEGER`; display order is a projection concern and is not persisted.

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use day_model::{Identified, Keyed, Op, Store};

#[cfg(feature = "driver-rusqlite")]
mod rusqlite_driver;
#[cfg(feature = "driver-rusqlite")]
pub use rusqlite_driver::Sqlite;

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
    /// Reports other connections' writes (a later phase; no built-in driver claims it yet).
    pub external_changes: bool,
}

/// The engine seam. Object-safe on the connection side, so the container stores
/// `Box<dyn SqliteConnection>` and never names an engine type.
pub trait SqliteDriver {
    type Connection: SqliteConnection;
    fn open(self) -> Result<Self::Connection, DbError>;
    fn capabilities(&self) -> Capabilities;
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

/// A persistable model: an [`Identified`] observable struct with a schema half. Implemented by
/// `#[derive(Model)]`; the container consumes it.
pub trait Model: Identified + Clone + 'static {
    const TABLE: &'static str;
    /// The key column's name (the `#[model(id)]` field). Also the first entry of `COLUMNS`.
    const KEY: &'static str;
    const COLUMNS: &'static [ColumnDef];
    /// Composite indexes from struct-level `#[model(index("a", "b"))]`.
    const COMPOSITE_INDEXES: &'static [&'static [&'static str]] = &[];
    /// One [`Value`] per column, in `COLUMNS` order.
    fn to_row(&self) -> Vec<Value>;
    fn from_row(row: &dyn Row) -> Result<Self, DbError>;
    /// Each column's Rust-default value — what an added column backfills with.
    fn default_row() -> Vec<Value>;
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
    h
}

// ---------------------------------------------------------------------------
// Schema sets and migrations
// ---------------------------------------------------------------------------

/// The set of models a container manages — build with [`schema!`].
#[derive(Default)]
pub struct Schema {
    installers: Vec<Installer>,
}

type Installer = Box<dyn FnOnce(&ModelContainer) -> Result<(), DbError>>;

impl Schema {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with<M: Model>(mut self) -> Self {
        self.installers.push(Box::new(|c| c.attach::<M>()));
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
/// [`Schema::with`] and stored type-erased.
struct TableHooks {
    table: &'static str,
    key_col: &'static str,
    columns: Vec<&'static str>,
    /// Same order as `columns`; what a change's label matches against.
    fields: Vec<&'static str>,
    /// Current row values by key, read from the store at flush time — the change log carries
    /// WHICH rows and columns moved, never their contents.
    row_for: Rc<dyn Fn(u64) -> Option<Vec<Value>>>,
    /// Every (key, row) currently in the store, for full resyncs.
    all_rows: Rc<dyn Fn() -> Vec<(u64, Vec<Value>)>>,
}

struct ContainerInner {
    conn: RefCell<Box<dyn SqliteConnection>>,
    caps: Capabilities,
    /// store root id → hooks.
    tables: RefCell<HashMap<u64, TableHooks>>,
    /// model TypeId → the `Store<Keyed<M>>` handle, boxed.
    stores: RefCell<HashMap<TypeId, Box<dyn Any>>>,
    dirty: RefCell<DirtyState>,
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
                sink: Cell::new(None),
                autosave: Cell::new(true),
                error,
                flushing: Cell::new(false),
            }),
        };

        container.ensure_schema_table()?;
        container.run_stages(plan)?;
        for install in schema.installers {
            install(&container)?;
        }
        container.install_sink();
        container.install_autosave();
        Ok(container)
    }

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
                return Ok(Vec::new());
            }
            std::mem::take(&mut *d)
        };
        self.inner.flushing.set(true);
        let result = self.flush(dirty);
        self.inner.flushing.set(false);
        match &result {
            Ok(_) => self.inner.error.set_if_changed(None),
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

        let columns: Vec<&'static str> = M::COLUMNS.iter().map(|c| c.name).collect();
        let fields: Vec<&'static str> = M::COLUMNS.iter().map(|c| c.field).collect();
        let row_for = {
            Rc::new(move |key: u64| store.with_untracked(|k| k.get(key).map(|m| m.to_row())))
                as Rc<dyn Fn(u64) -> Option<Vec<Value>>>
        };
        let all_rows = {
            Rc::new(move || {
                store.with_untracked(|k| {
                    k.items()
                        .iter()
                        .map(|m| (m.obs_key(), m.to_row()))
                        .collect()
                })
            }) as Rc<dyn Fn() -> Vec<(u64, Vec<Value>)>>
        };

        self.inner.tables.borrow_mut().insert(
            store_id,
            TableHooks {
                table: M::TABLE,
                key_col: M::KEY,
                columns,
                fields,
                row_for,
                all_rows,
            },
        );
        self.inner
            .stores
            .borrow_mut()
            .insert(TypeId::of::<M>(), Box::new(store));
        Ok(())
    }

    fn ensure_table<M: Model>(&self) -> Result<(), DbError> {
        let fp = format!("{:016x}", model_fingerprint::<M>());
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
        self.store_fingerprint(M::TABLE, &fp)?;
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
                let tables = inner.tables.borrow();
                inner
                    .dirty
                    .borrow_mut()
                    .note(change, |store| tables.contains_key(&store));
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
            let mut db_keys: Vec<i64> = Vec::new();
            self.conn().query(
                &format!("SELECT {} FROM {}", hooks.key_col, hooks.table),
                &[],
                &mut |row| {
                    if let Ok(k) = row.get(0).as_int() {
                        db_keys.push(k);
                    }
                },
            )?;
            for (_, row) in &rows {
                stmts.push(upsert_stmt(hooks, row));
            }
            for gone in db_keys
                .iter()
                .filter(|k| !rows.iter().any(|(key, _)| *key as i64 == **k))
            {
                stmts.push((
                    format!("DELETE FROM {} WHERE {} = ?", hooks.table, hooks.key_col),
                    vec![Value::Int(*gone)],
                ));
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
                    stmts.push((
                        format!("DELETE FROM {} WHERE {} = ?", hooks.table, hooks.key_col),
                        vec![Value::Int(key as i64)],
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
                    params.push(Value::Int(key as i64));
                    stmts.push((
                        format!(
                            "UPDATE {} SET {} WHERE {} = ?",
                            hooks.table,
                            sets.join(", "),
                            hooks.key_col
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
    (
        format!(
            "INSERT OR REPLACE INTO {} ({cols}) VALUES ({marks})",
            hooks.table
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
