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

The same API works on web-dom: `:memory:` databases run in-process, and a file database
lives in the browser's origin-private file system, held by the day-sql worker and reached
synchronously — see [The web](#the-web) below. rusqlite itself is native-only; the wasm build
compiles the `day-sqlite-worker` engine instead, with FTS5 and R*Tree included. Compiling it
needs a clang with a wasm32 backend — [docs/web.md](web.md) has the setup.

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

`derive(Model)` compiles on every target, the web included. A build that deliberately keeps
its web tier on plain stores can swap the derive instead — `Observable` accepts `#[model(…)]`,
reads `#[model(id)]` as the key, and ignores the schema half (the Showcase's Query page keeps
its ten-thousand-row demo in memory this way):

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
`full_text_search`, `rtree`, `external_changes` — so a settings page can offer only what is
real. An external crate can implement `SqliteDriver` for an engine the built-ins cannot be
(libSQL, SEE, a proxy). Beyond `execute`/`query`, the connection trait carries `execute_batch`
(a statement script — a migration step) and `query_named` (rows with their column names, for
callers that surface rows as named objects); the built-ins implement both, defaults keep
external drivers compiling. [day-lite](lite.md)'s per-miniapp storage rides this same driver,
so a superapp carrying both crates compiles ONE SQLite and the app's engine features govern
miniapp storage too.

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

## Queries

A `Query` is a predicate, a sort and a window over one model's table — ids only, and a TRACKED
read: `ids()`/`count()`/`first()` re-run their caller exactly when the result set changes.
Predicates build from the same names that bind controls: the derive emits `Trip::name()` (a
typed column ref) beside the accessor `trip.name()`, so a typo is a compile error and every
argument encodes through the field's own codec.

```rust
let trips = container.query::<Trip>()
    .filter(Trip::done().eq(false) & Trip::start_day().between(from, to))
    .sort(Trip::start_day().asc())
    .limit(200)
    .live();

// The reactive form: the fetch re-derives when its signals change.
let results = container.query_fn::<Trip>(move || {
    let t = term.get();
    Fetch::new().filter(Trip::name().contains_ci(t)).sort(Trip::name().asc())
});
```

The result set is maintained INCREMENTALLY against the change log (`LiveSet`, the
fetched-results-controller algorithm with one better tier): a change to a column the query
never mentions costs nothing at all — no predicate evaluation, no waking; a predicate or sort
column evaluates exactly the changed row and emits an `Insert`/`Remove`/`Move` delta; only a
windowed (`limit`) boundary crossing re-derives the set. The 600-edit agreement test in
`tests/liveset.rs` pins the property that makes skipping the database safe: the incremental
path lands, id for id, where a fresh fetch would.

`list(query, row)` (facade feature `persistence`) makes a query a row source: rows bind through
`ModelSlot` exactly as a store source's do, and set changes reach the native list as
`ListPatch::Splice` row deltas it can animate — an NSTableView slides the row out instead of
reloading. Hosts that cannot animate treat a splice as a reload.

Three tiers of SQL access, each stating its price: the typed builder (incremental, the
default); `query_raw::<M>(sql, params, &["tables"])` — a read-only SELECT of ids, re-run whole
whenever a flush touches a named table; and `with_connection(|conn| …)` — the driver's
connection for maintenance and imports, whose writes bypass the change log entirely. Call
`rescan()` afterward: it reloads every store from the file (without re-marking them dirty) and
re-runs every query.

## Full-text and spatial

Both of SQLite's own answers ship in the derive:

```rust
#[derive(Model, Clone, Default, PartialEq)]
#[model(table = "posts", fts("title", "body"), spatial(lat = "lat", lon = "lon"))]
struct Post { /* … */ }

query.filter(Post::fts().matches("kyoto OR osaka")).sort(rank())   // bm25, best first
query.filter(Post::geo().within(GeoRect { min_lat, max_lat, min_lon, max_lon }))
```

The schema derive generates the standard patterns rather than asking you to hand-write them: an
external-content FTS5 table (`posts_fts`, `content=posts`) and an R*Tree table (`posts_geo`),
each kept true by three `AFTER INSERT/UPDATE/DELETE` triggers — inside the same transaction as
every write, and correct even for rows another tool writes into the file. (This is also why the
fold's upsert is a true `ON CONFLICT DO UPDATE` and not `INSERT OR REPLACE`: the latter's
implicit delete skips the delete triggers unless `recursive_triggers` is on, and the index
would rot.) A freshly created shadow backfills from existing rows.

The two predicates sit at different tiers, honestly. `within` is range comparisons over two
REAL columns, so it evaluates in memory — a moved pin is one evaluation and one `Move` delta.
`matches` cannot be evaluated without reimplementing the tokenizer, so it declares the INDEXED
columns as its dependency set: a change to one of them re-queries (deferred until after the
flush, when the triggers have run), and a change to any column outside the set still costs
nothing. A driver whose engine lacks FTS5 or R*Tree fails `open` with the module named — an
error at open, never a silent downgrade at query time.

## Undo

```rust
let stack = container.undo(100);   // one history over every store, 100 units deep
day::install_undo(&stack);         // native fronts where the platform has them
```

The unit of undo is a TURN — everything one event's dispatch changed — inverted from the same
change log the SQL fold reads: a `Set` inverts by writing the prior back, a `Delete` carries
the row it removed and inverts to an insert, an `Insert` inverts to a delete. An undo replays
those inverses (in reverse, in one batch) tagged with the author `undo`, and everything
downstream just works: field triggers wake exactly what changed back, live queries animate the
row's return, autosave writes the inverse statements — undoing a delete is ONE `INSERT`, never
a snapshot. `grouped(label, f)` makes a multi-write gesture one unit; `set_label_resolver`
turns unit labels into display text; `can_undo`/`undo_label` are signals a button or menu
wires to directly. The stack lives in day-model, so a plain in-memory store undoes identically
— persistence is optional to undo, not the other way around.

Where the platform has its own undo system, `day::install_undo` mirrors the stack into a native
FRONT (`Cap::UndoBridge`): on macOS the window answers `undoManager` with a Day-owned
`NSUndoManager` subclass, so the stock Edit menu retitles and enables itself and ⌘Z lands
through the responder chain — and a focused text field's own manager keeps precedence, which is
the typing rule. iOS gets the same front through the root view controller, so the three-finger
gestures, shake-to-undo and the iPad menu bar all reach the one stack. Everywhere else the
app's own affordances call `stack.undo()` directly.

## Sessions: preview, then commit

A slider mid-drag must move the model LIVE (labels and swatches render from it) while the
durable fact is the settled value. The `Binding` trait carries the split — `write_preview`
(from `Event::ValueChanged`) and `write_commit` (from `Event::ValueCommitted`) — and a
day-model field implements it as a session: previews land in the store and wake this field's
readers, but no change record exists until the commit, which announces ONE record whose prior
is the pre-drag value. Sixty thumb positions cost sixty UI wakes (their job), one undo unit,
and one `UPDATE`. Text fields ride the same machinery: each keystroke previews, Return or
focus loss commits — the typing coalescer. `field.session()` gives the explicit form
(`preview`/`commit`/`cancel` — Escape restores, zero records); a plain `Signal` binding
defaults both methods to `write`, so nothing changes where no session semantics exist. One
edge to know: a session left open (focus never leaves a field before the process dies) has not
committed, and its typing is not yet in the change log or the file.

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

## Relations

`One<M>` is a to-one reference — the foreign-key column on the child, stored in the target's
own key shape. `Many<M>` is the other side: a marker field storing nothing, whose accessor
reads an index the container maintains from the change log. That is the whole design. There is
**one source of truth** (the child's foreign key), so the maintained inverses SwiftData
promises fall out of the pipeline that already exists rather than parallel bookkeeping:

```rust
#[derive(Model, Clone, Default, PartialEq)]
#[model(table = "trips")]
struct Trip {
    #[model(id)] id: Uuid,
    name: String,
    #[model(relation(target = Lodging, inverse = "trip", delete = "cascade"))]
    lodging: Many<Lodging>,
}

#[derive(Model, Clone, Default, PartialEq)]
#[model(table = "lodging")]
struct Lodging {
    #[model(id)] id: Uuid,
    name: String,
    trip: One<Trip>,            // Option<One<Trip>> for a nullable reference
}

trip.lodging().ids()            // tracked: the children, in relation order
trip.lodging().add(lodging_id)  // writes the child's FK — one UPDATE
lodging.trip().write(One::to(other_trip));   // …and the same thing, from the child
```

Write either side and both wake: `add` goes through the child's foreign key, so the change
announces, captures for undo, folds to one statement, and reaches any live query watching
either table. Reads are tracked through the parent's own field path, so a membership change
wakes exactly the readers of that parent's relation — not every reader of the table.

**Delete rules** are declared on the `Many` side and default to `nullify` (SwiftData's):
children survive and their references clear, which requires `Option<One<M>>` — wiring refuses
`nullify` over a required reference, naming the fix. `cascade` deletes the children with the
parent, recursively and through the normal pipeline, so a cascade is one undo unit that
restores the whole subtree and a list animates the rows out. `deny` refuses while children
remain, through `container.delete::<M>(id)` — the checked door; a raw `restructure` delete
cannot be refused after the fact, so deny is the one rule that needs it. The generated DDL
carries the matching `REFERENCES … ON DELETE …` clause, `DEFERRABLE INITIALLY DEFERRED` so
statement order within a transaction never trips it, which also keeps another process honest
about the same rules.

**Ordered to-many** (`ordered = "field"`) names a `f64` field of the child that holds its
position — a real, visible column, not hidden state. Placement is fractional, so a drag writes
**one row**; when a gap bisects away the siblings rebalance to whole numbers first (O(n), rare,
still one statement per row). `insert_at(child, i)` and `move_to(child, i)` are the verbs; an
unordered relation refuses both rather than pretending.

**Many-to-many** declares `join = "table"` instead of an inverse. The derive generates the join
table — the pair as its primary key, a foreign key per side, an index on the reverse side — and
memberships are ordinary rows keyed by the *pair*, so they fold to SQL, undo, and merge through
the same machinery every other row uses. Declare it on both models to read it from both (one
relation, one store, two views); order lives on the membership, declared with a bare `ordered`
on the side that owns it, because the same child sits at different positions under different
parents.

```rust
#[model(relation(target = Note, join = "note_tags"))]
notes: Many<Note>,                       // on Tag
#[model(relation(target = Tag, join = "note_tags"))]
tags: Many<Tag>,                         // on Note — the same memberships, read the other way

tag.notes().add(note_id);                // one INSERT into note_tags
note.tags().contains(tag_id)             // true, from the same row
```

Deleting either side drops its memberships; under `cascade` it also takes the rows across the
join that no other row still holds, so deleting one album never takes a shared photo with it.
`Model::RELATIONS` exposes every declaration as data, for tools and tests.

## External changes

Another connection's committed writes — another process, a sync engine, a CLI editing the
file — do not announce themselves. `check_external()` looks: detection is one
`PRAGMA data_version` (the counter moves only when another connection commits, never for this
one's own writes), so it is cheap enough to wire to app foreground, window focus, or a timer.
When the counter moved, pending local edits flush first, each table is diffed against its
store, and only the differences feed through: changed fields announce per column, inserts and
deletes take the structural path, and live queries emit their usual precise deltas — a list
animates the row another process inserted. The merged changes are tagged
`ModelContainer::EXTERNAL_AUTHOR` (`"database"`): the autosave fold declines the echo, an undo
stack skips them (another author's writes are not the user's history), and any change sink can
tell them from the user's edits. A row another connection rewrote arrives whole, so its
`#[model(transient)]` fields reset to their defaults, exactly as at load.

`Capabilities::external_changes` says whether detection is real: the built-in driver claims it
for file databases on native targets; memory databases (no second connection can reach one),
the Recorder, and the web engine (its OPFS access is exclusive) answer `Ok(false)` honestly.
Writes made through `with_connection` are this connection's own — the counter does not move
for them; that recovery stays `rescan()`. `tests/external.rs` is the reference suite.

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

Lazy row faulting (a container LOADS each table at open — the document pattern) and external
storage (`#[model(external)]`) are later phases — the plan's design for them shapes what exists
today (visible schema, `ColumnValue` everywhere, capabilities), but neither is API yet. Row
identity is the `#[model(id)]` key in its own stored shape (`INTEGER`, a 16-byte `BLOB` for
`Uuid`, `TEXT` for a string key — [model.md](model.md)); a model's own display order is a
projection concern and is not persisted, though an ordered relation's position is.

Two shapes the key layer refuses rather than mis-serving: `fts(…)`/`spatial(…)` need an
integer key (both address rows by SQLite's ROWID), and a key field takes no codec — its stored
form is the key's own canonical one, because the fold's `WHERE` parameters and the merge's
decoding derive it from the key kind.

## Watching the SQL

`Sqlite::trace_sql(f)` installs the engine's own statement trace (`sqlite3_trace_v2` with
`SQLITE_TRACE_STMT`): `f` sees every statement the connection executes — migrations, autosave
flushes, live-query `SELECT`s, undo replays, maintenance — with bound parameters expanded by
SQLite itself. The usual shape is a debug-only logger:

```rust
let driver = Sqlite::app_data("trips.db")?;
let driver = if cfg!(debug_assertions) {
    driver.trace_sql(|sql| trace!("sql: {sql}"))
} else {
    driver
};
```

The trace installs after any `PRAGMA key`, so a cipher key never reaches the sink. On web-dom
a file database's engine runs in the day-sql worker, out of closure reach: its statements log
to the browser console as `[day-sql]` lines instead of calling `f` (`:memory:` databases call
`f` on every target).

## The web

OPFS — the browser's origin-private file system — exposes its only synchronous random-access
API inside dedicated workers, so on web-dom the engine runs in one: the **day-sql worker**, a
second instantiation of the app's own wasm module that day-cli's host page spawns at boot.
Every `SqliteConnection` call crosses a SharedArrayBuffer as one request; the main thread
blocks the few microseconds until the reply state flips, and the worker flushes to storage
before answering — a commit that returned has landed, and `capabilities().durable` is true.
`:memory:` databases skip the channel and run in-process. Apps see none of this: `Sqlite::at`,
autosave, undo, queries, and `backup_to` behave as they do everywhere else.

What the page must provide:

- **Cross-origin isolation.** SharedArrayBuffer exists only under COOP/COEP response headers.
  `day launch` sends them; a static deployment must too ([docs/web.md](web.md)). Without
  isolation there is no worker: memory databases still work, file opens fail with
  `Unsupported`, and `durable` reads false — the app can present that honestly.
- **One tab.** OPFS access handles are exclusive, so the first tab of the app owns its
  databases; a second tab runs memory-only until the first closes.

`Sqlite::web_storage()` is the document-pool surface for file-per-document apps:
`exists`/`list` for numbering a fresh document, `import_db` for landing an Open… pick,
`export_db` for handing bytes to a download (flush first, so the image includes the current
turn). Database names are the "paths" the rest of the app already uses — `Sqlite::at("a.db")`
on the web opens the pool entry named `a.db`.
