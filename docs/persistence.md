---
title: "Persistence"
description: "day-persistence: SQLite storage for the observable model — ModelContainer, the Model derive, drivers and engines, migrations, codecs, and maintenance."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Persistence (`day-persistence`) — normative

`day-persistence` stores observable models in SQLite. The write half is not a new mechanism: it
is [day-model's change log](model.md), folded. A `ModelContainer` opens a database, creates or
migrates each model's table, loads the rows into an ordinary `Store<Keyed<M>>`, and then watches
the change log the UI already produces. At the end of any turn that touched a store, the
accumulated changes fold into the smallest statement list that expresses them — twenty
keystrokes into one field are one `UPDATE`, a row inserted and then filled is one `INSERT`, a
row deleted is one `DELETE` no matter what preceded it.

Enable it with the `day` facade's `persistence` feature (implies `model`):

```toml
day = { version = "0.2", features = ["persistence"] }   # bundled SQLite, the default engine
```

The engine is a feature, not a code change: `sqlite-system` links the OS's libsqlite3 instead of
compiling the bundled one; `sqlite-cipher` builds SQLCipher with vendored crypto. `use
day::prelude::*` brings `Model` (the derive and the trait), `ModelContainer`, `Sqlite`,
`Recorder`, `Secret`, the `schema!` macro, and the `day_persistence` crate name the derive's
generated code resolves against. The full API is `day::persistence::*`.

On wasm there is no filesystem and rusqlite is not built — gate the dependency to native
targets and keep the web build on plain stores (the Showcase's Model page shows the pattern).

## Declaring a model

`#[derive(Model)]` implies `Observable` and adds the schema half:

```rust
#[derive(Model, Clone, Default, PartialEq)]
#[model(table = "trips", index("start_day", "done"))]  // table name + a composite index
pub struct Trip {
    #[model(id)]
    pub id: u32,                     // the key: INTEGER PRIMARY KEY, and #[obs(key)] too

    #[model(unique)]
    pub name: String,                // TEXT NOT NULL UNIQUE

    #[model(index)]
    pub start_day: DayDate,          // INTEGER epoch days (the canonical DayDate form)

    pub rating: Option<f64>,         // REAL — Option drops NOT NULL, None stores NULL

    #[model(column = "note_text")]
    pub notes: String,               // the column name, where the derived one is not wanted

    #[model(with = Iso8601)]
    pub booked: DayDate,             // TEXT "2026-08-19", same Rust type, other stored form

    #[model(json)]
    pub tags: Vec<String>,           // any serde type as TEXT SQLite's json_* can reach

    #[model(transient)]
    pub is_selected: bool,           // observable, never written to the database
}
```

The rules are short. A struct maps to a snake_case table and a field to a column of the same
name, each overridable (`table =`, `column =`). Types map through one public trait
(`ColumnValue`, below): `String` → `TEXT`, integers and `bool` → `INTEGER`, `f32`/`f64` →
`REAL`, `Vec<u8>` → `BLOB`, `Option<T>` wraps any of them and owns `NULL`. `#[model(index)]` on
a field, `index("a", "b")` on the struct for composites, `unique` for constraints. Tables are
created `STRICT` when the engine accepts the keyword (always, for the bundled build) and plain
otherwise. `#[obs(skip)]` removes a field from both halves — a field the change log cannot name
could never mark its row dirty, so persisting it would silently lose edits.

A struct that must also compile for a web target keeps every `#[model(…)]` attribute and swaps
only the derive: `Observable` accepts `#[model(…)]`, reads `#[model(id)]` as the key, and
ignores the schema half.

```rust
#[cfg_attr(not(target_arch = "wasm32"), derive(Model))]
#[cfg_attr(target_arch = "wasm32", derive(Observable))]
```

## The container

```rust
let container = ModelContainer::open(Sqlite::app_data("trips.db")?, schema![Trip, Lodging])?;
let trips: Store<Keyed<Trip>> = container.store::<Trip>();
```

`open` migrates and then LOADS each model's table into its store — the document pattern; lazy
faulting is a later phase. The store is an ordinary day-model store: every binding, projection
and list source works on it unchanged, and nothing in the UI knows the container exists.

Autosave is on by default: a change sink marks rows dirty as the change log announces them, and
the end of any turn that touched a store flushes the fold in one transaction. `save()` flushes
now and returns the error at a known point; `set_autosave(false)` accumulates until you do.
Autosave failures land in `container.last_error()`, a tracked `Signal<Option<String>>` a status
line can watch. `record_sql(f)` runs a closure and returns the SQL one flush of everything it
changed issues — the headless persistence assert.

The fold's merge rules, per row: same-row changes coalesce (column names accumulate on one
`UPDATE`), an insert absorbs the edits that fill it, a delete absorbs everything, moves are
order-only and order is not persisted. Row values are read from the store at flush time — the
change log carries which rows and columns moved, never their contents. A wholesale
`Store::update` resyncs that whole table: upsert every row, delete the gone ones.

## Drivers and engines

The container speaks to SQLite through the `SqliteDriver` trait; two drivers are built in.

**`Sqlite`** (feature `driver-rusqlite`, on by default) is rusqlite. `Sqlite::at(path)` opens a
file, `Sqlite::memory()` an in-memory database, and `Sqlite::app_data("name.db")` the per-app
data directory: `DAY_DATA_DIR` when the host passes one (the Android and OpenHarmony hosts do),
the platform's app-data convention otherwise, under a `day-db/` leaf beside
[day-part-fs](fs.md)'s `day-fs/`. Builder options: `.key(Secret)` for SQLCipher,
`.cipher_migrate()` to accept an older cipher generation, `.wal(false)` to opt out of WAL, and
`.with_init(|conn| …)` — a hook over the raw rusqlite connection for loadable extensions,
custom SQL functions, and PRAGMAs the crate does not model.

**`Recorder`** is always compiled: it answers queries from fixtures and records every statement,
which is what keeps persistence assertable with no database on disk (see Testing below).

`capabilities()` reports what the open driver can actually do — `durable`, `encryption`,
`full_text_search`, `rtree` — so a settings page can offer only what is real. An external crate
can implement `SqliteDriver` for an engine the built-ins cannot be (libSQL, SEE, a proxy).

## Schema, fingerprints, migrations

Day claims only what it created. The derived DDL is documented, stable and readable by any
SQLite tool; the bookkeeping is one `_day_schema` table (`table_name`, `fingerprint`,
`version`), and every other table in the file is left alone — an app can point the container at
a database that also holds its own hand-made tables, and the two coexist.

Each model's declared schema — table, columns, types, flags, indexes — hashes to a fingerprint.
Equal fingerprints open instantly. A difference runs **lightweight migration**: added columns
are `ALTER TABLE … ADD COLUMN`ed and backfilled with the field's Rust default, dropped columns
are dropped. What lightweight migration cannot close — a rename, a type or codec change — needs
a **staged migration**:

```rust
let plan = MigrationPlan::new()
    .custom(1, 2, |conn| {
        conn.execute("ALTER TABLE trips RENAME COLUMN heading TO name", &[]).map(|_| ())
    });
let container = ModelContainer::open_with(Sqlite::app_data("trips.db")?, schema![Trip], plan)?;
```

Stages run in ascending order from the file's version, each in its own transaction; versions
only go up, a gap is an error at open, and a file NEWER than the build refuses to open — an old
app writing a new schema is how data rots. Lightweight migration runs after the stages, so a
stage handles the rename and the fingerprint closes the rest.

When a stored row will not decode, the read is lenient in exactly one case: `NULL` in any
column reads as the field's `Default` (this is what makes an added column's old rows readable
before their backfill). Everything else is a `Decode` error naming what disagreed.

## Value codecs

`ColumnValue` is the one public trait behind the type-mapping table — implement it and your
type is a column everywhere a built-in is:

```rust
impl ColumnValue for Rgb {
    const SQL_TYPE: SqlType = SqlType::Text;
    fn to_sqlite_value(&self) -> Value { Value::Text(format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)) }
    fn from_sqlite_value(v: Value) -> Result<Self, DbError> { Rgb::parse(v.as_text()?) }
}
```

`ValueCodec<T>` is the named-alternative form — serde's `#[serde(with = …)]` idiom for columns.
Implement it on a unit struct and select it per field with `#[model(with = …)]`; `#[model(json)]`
is sugar for the built-in `Json` codec. `Option` handling belongs to the framework: an impl
never sees `NULL` and cannot disagree about it.

[day-piece-datetime](datepicker.md) ships the date and time impls behind its `persistence`
feature: `DayDate` and `DayTime` canonically as `INTEGER` (epoch days, seconds-of-day), plus
`Iso8601` (`TEXT`, lexicographic order is chronological order), `EpochSeconds` and
`EpochMillis` for interop that expects Unix time.

## Encryption

With the `sqlite-cipher` engine, `.key(Secret::new(…))` opens an encrypted database; a wrong or
missing key is `DbErrorKind::BadKey` at open, not a decode error later. `rekey` re-encrypts in
place; `encrypt_to`/`decrypt_to` write converted copies (SQLCipher's own export path — the
conversion cannot happen in place). Day refuses to store the key: it arrives as a `Secret` that
never prints and is zeroed on drop, and where it lives between launches — the OS keychain being
the obvious place — is the app's business. Builds without the engine reject `.key` loudly at
open.

## Maintenance

`backup_to(path)` is `VACUUM INTO`: a transactionally consistent, already-compacted snapshot
taken while the app keeps writing, after flushing what is pending; restore is an open, not a
verb. `integrity_check()` parses the PRAGMA into findings (empty means sound).
`checkpoint()` folds the WAL into the main file — run it before the OS copies the file whole
(device backup). `vacuum()` compacts in place; `size_bytes()` answers the settings-page
question.

## Testing

The `Recorder` makes persistence a unit-test subject:

```rust
let (driver, log) = Recorder::new().with_table("trips", fixtures);
let container = ModelContainer::open(driver, schema![Trip])?;
let sql = container.record_sql(|| {
    trips.elem(1).name().write("Osaka".into());
    trips.elem(1).name().write("Nara".into());
})?;
assert_eq!(sql, ["UPDATE trips SET name = ? WHERE id = ?"]);
```

`with_table` registers the rows a table's load `SELECT` answers with; `log.sql()` and
`log.entries()` expose everything issued, parameters included. The crate's own suites in
`crates/day-persistence/tests/` are the reference: the fold rules against the Recorder
(`container.rs`), the derive (`derive.rs`), real files, coexistence, backups and migration
(`sqlite.rs`), and the cipher lifecycle (`cipher.rs`, `--features cipher`).

## Not in this version

Typed queries (predicates over indexed columns, partial loading), lazy row faulting,
relations (`#[model(relation)]`), external storage (`#[model(external)]`), full-text and
spatial indexes, and watching other connections' writes are later phases — the plan's design
for them shapes what exists today (visible schema, `ColumnValue` everywhere, capabilities), but
none of it is API yet. Row identity is the `#[model(id)]` key stored as `INTEGER`; display
order is a projection concern and is not persisted.
