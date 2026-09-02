---
title: "Persistence"
description: "day-persistence: SQLite storage for the observable model — ModelContainer, the Model derive, drivers and engines, migrations, codecs, and maintenance."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Persistence (`day-persistence`) — normative

`day-persistence` stores observable models in SQLite, and SQLite keeps owning the data: a
`ModelContainer` opens a database, creates or migrates each model's table, and reads no rows,
so opening a million-row store costs what opening an empty one does. Rows enter memory
by *faulting* (a `get`, a batch `ensure_resident`, a list materializing the rows it shows)
into a bounded per-model cache, and queries compile to SQL the engine answers from its
indexes, so memory holds a working set rather than the table.

The write half reuses [day-model's change log](model.md), folded. The container watches the
change log the UI already produces, and at the end of any turn that touched a store the
accumulated changes fold into the smallest statement list that expresses them: twenty
keystrokes into one field are one `UPDATE`, a row inserted and then filled is one `INSERT`, and
a row deleted is one `DELETE` no matter what preceded it.

Enable it with the `day` facade's `persistence` feature (implies `model`):

```toml
day = { version = "0.2", features = ["persistence"] }   # bundled SQLite, the default engine
```

A cargo feature selects the engine: `sqlite-system` links the OS's libsqlite3 instead of
compiling the bundled one; `sqlite-cipher` builds SQLCipher with vendored crypto. `use
day::prelude::*` brings `Model` (the derive and the trait), `ModelContainer`, `Sqlite`,
`Recorder`, `Secret`, the `schema!` macro, and the `day_persistence` crate name the derive's
generated code resolves against. The full API is `day::persistence::*`.

The same API works on web-dom: `:memory:` databases run in-process, and a file database
lives in the browser's origin-private file system, held by the day-sql worker and reached
synchronously; see [The web](#the-web) below. rusqlite itself is native-only; the wasm build
compiles the `day-sqlite-worker` engine instead, with FTS5 and R*Tree included. Compiling it
needs a clang with a wasm32 backend; [docs/web.md](web.md) has the setup.

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
otherwise. `#[obs(skip)]` removes a field from both halves; a field the change log cannot name
could never mark its row dirty, so persisting it would silently lose edits.

`derive(Model)` compiles on every target, the web included. A build that keeps its web tier
on plain stores can swap the derive instead: `Observable` accepts `#[model(…)]`,
reads `#[model(id)]` as the key, and ignores the schema half (the Showcase's Query page keeps
its ten-thousand-row demo in memory this way):

```rust
#[cfg_attr(not(target_arch = "wasm32"), derive(Model))]
#[cfg_attr(target_arch = "wasm32", derive(Observable))]
```

## The container

```rust
let container = ModelContainer::open(Sqlite::app_data("trips.db")?, schema![Trip, Lodging])?;
let trips: Store<Keyed<Trip>> = container.cache::<Trip>();
```

`open` migrates and stops. `cache::<M>()` is the model's **working set**: an ordinary
day-model store holding the rows currently resident. Every binding, projection and list
source works on it unchanged, and nothing in the UI knows the container exists, but its
`keys()` are whatever happens to be faulted in, never the whole table. Enumerate rows through
a query; read one row through the container:

```rust
let trip = container.get::<Trip>(id);          // resident, faulted from the file if needed
container.ensure_resident::<Trip>(&keys)?;     // a batch, in one chunked SELECT
container.insert(Trip { id, ..Default::default() });   // resident + dirty + announced
let n = container.table_count::<Trip>()?;      // SELECT COUNT(*) — the table's true size
container.warm::<Trip>()?;                     // the document pattern: fault EVERY row in
```

Editing requires residency (writing a row faults it in), and a delete works either way (a
value-free delete folds to one statement; with an undo stack installed the row
materializes first so the history can restore it). The cache is bounded:
`set_cache_limit(rows)` (default `DEFAULT_CACHE_LIMIT`, 8192 per model) evicts the oldest
clean rows past the limit, sparing anything dirty and anything a binding still observes;
evicted rows fault back on the next read. A row deleted this turn does not resurrect through
a fault, even though the file still holds it until the flush.

A document container (a sketch whose canvas draws the whole scene) says so explicitly:
`set_cache_limit(usize::MAX)` then `warm::<M>()` per model at open. The file-sized working
set then behaves exactly as the old load-at-open engine did, as an explicit choice instead of
the default.

Autosave is on by default: a change sink marks rows dirty as the change log announces them, and
the end of any turn that touched a store flushes the fold in one transaction. `save()` flushes
now and returns the error at a known point; `set_autosave(false)` accumulates until you do.
Autosave failures land in `container.last_error()`, a tracked `Signal<Option<String>>` a status
line can watch. `record_sql(f)` runs a closure and returns the SQL one flush of everything it
changed issues; that is the headless persistence assertion.

The fold's merge rules, per row: same-row changes coalesce (column names accumulate on one
`UPDATE`), an insert absorbs the edits that fill it, a delete absorbs everything, moves are
order-only and order is not persisted. Row values are read from the store at flush time; the
change log carries which rows and columns moved, never their contents. A wholesale
`Store::update` upserts every resident row: the cache is a working set, so a rewrite covers
what it holds and never infers deletions from absence. Deleting is an explicit act.

Rows then merge across each other where one statement can carry them. Deletes from the same
table become a single `DELETE … WHERE id IN (?, ?, …)`, and updates join when they set the
same columns to the same values (the shape a multi-selection edit makes, where one field is
written across every selected row). Updates carrying different values keep their own statements,
because `SET` holds one value per column. Deleting a group of five shapes is one statement, and
renaming twenty rows to the same name is one statement. Rows keyed by more than one column (a
join table's membership, which no single-column `IN` names) stay unbatched, and so do inserts,
which already fold to one multi-row upsert. A batch longer than SQLite's bound
parameter limit splits into chunks inside the same transaction, so the whole flush still
commits or rolls back together. `record_sql` shows the batched form, which is what to assert.

Within the transaction, deletes always come last. A relation column's `ON DELETE CASCADE`
fires per statement (`DEFERRABLE` defers the constraint check, never the action), so a
parent row's delete emitted mid-batch would take every child the batch had not yet
re-parented (an ungroup detaches its members and deletes the group in one turn). With
inserts and updates flushed first, a row still referencing a deleted parent at delete time
is one the batch never detached, an orphan the cascade rule is supposed to take.

## Drivers and engines

The container speaks to SQLite through the `SqliteDriver` trait; two drivers are built in.

**`Sqlite`** (feature `driver-rusqlite`, on by default) is rusqlite. `Sqlite::at(path)` opens a
file, `Sqlite::memory()` an in-memory database, and `Sqlite::app_data("name.db")` the per-app
data directory: `DAY_DATA_DIR` when the host passes one (the Android and OpenHarmony hosts do),
the platform's app-data convention otherwise, under a `day-db/` leaf beside
[day-part-fs](fs.md)'s `day-fs/`. Builder options: `.key(Secret)` for SQLCipher,
`.cipher_migrate()` to accept an older cipher generation, `.wal(false)` to opt out of WAL, and
`.with_init(|conn| …)`, a hook over the raw rusqlite connection for loadable extensions,
custom SQL functions, and PRAGMAs the crate does not model.

**`Recorder`** is always compiled: it answers queries from fixtures and records every statement,
which makes persistence assertable with no database on disk (see Testing below).

`capabilities()` reports what the open driver can do (`durable`, `encryption`,
`full_text_search`, `rtree`, `external_changes`), so a settings page can offer only what is
real. An external crate can implement `SqliteDriver` for an engine the built-ins cannot be
(libSQL, SEE, a proxy). Beyond `execute`/`query`, the connection trait carries `execute_batch`
(a statement script — a migration step) and `query_named` (rows with their column names, for
callers that surface rows as named objects); the built-ins implement both, defaults keep
external drivers compiling. [day-lite](lite.md)'s per-miniapp storage rides this same driver,
so a superapp carrying both crates compiles one SQLite and the app's engine features govern
miniapp storage too.

## Schema, fingerprints, migrations

Day claims only what it created. The derived DDL is documented, stable and readable by any
SQLite tool; the bookkeeping is one `_day_schema` table (`table_name`, `fingerprint`,
`version`), and every other table in the file is left alone, so an app can point the container
at a database that also holds its own hand-made tables, and the two coexist.

Each model's declared schema (table, columns, types, flags, indexes) hashes to a fingerprint.
Equal fingerprints open instantly. A difference runs **lightweight migration**: added columns
are `ALTER TABLE … ADD COLUMN`ed and backfilled with the field's Rust default, dropped columns
are dropped. What lightweight migration cannot close (a rename, a type or codec change) needs
a **staged migration**:

```rust
let plan = MigrationPlan::new()
    .custom(1, 2, |conn| {
        conn.execute("ALTER TABLE trips RENAME COLUMN heading TO name", &[]).map(|_| ())
    });
let container = ModelContainer::open_with(Sqlite::app_data("trips.db")?, schema![Trip], plan)?;
```

Stages run in ascending order from the file's version, each in its own transaction; versions
only go up, a gap is an error at open, and a file newer than the build refuses to open, because
an old app writing a new schema would corrupt the data. Lightweight migration runs after the
stages, so a
stage handles the rename and the fingerprint closes the rest.

When a stored row will not decode, the read is lenient in exactly one case: `NULL` in any
column reads as the field's `Default` (this makes an added column's old rows readable before
their backfill). Everything else is a `Decode` error naming what disagreed.

## Value codecs

`ColumnValue` is the one public trait behind the type-mapping table; implement it and your
type is a column everywhere a built-in is:

```rust
impl ColumnValue for Rgb {
    const SQL_TYPE: SqlType = SqlType::Text;
    fn to_sqlite_value(&self) -> Value { Value::Text(format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)) }
    fn from_sqlite_value(v: Value) -> Result<Self, DbError> { Rgb::parse(v.as_text()?) }
}
```

`ValueCodec<T>` is the named-alternative form, serde's `#[serde(with = …)]` idiom for columns.
Implement it on a unit struct and select it per field with `#[model(with = …)]`; `#[model(json)]`
is sugar for the built-in `Json` codec. `Option` handling belongs to the framework: an impl
never sees `NULL` and cannot disagree about it.

[day-piece-datetime](datepicker.md) ships the date and time impls behind its `persistence`
feature: `DayDate` and `DayTime` canonically as `INTEGER` (epoch days, seconds-of-day), plus
`Iso8601` (`TEXT`, lexicographic order is chronological order), `EpochSeconds` and
`EpochMillis` for interop that expects Unix time.

## Queries

A `Query` is a predicate, a sort and a window over one model's table: ids only, and a tracked
read. `ids()`/`count()`/`first()` re-run their caller exactly when the result set changes.
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

// The badge form: a live COUNT holding no ids — `SELECT COUNT(*)` behind the same
// dependency gate, O(1) memory at any result size. `count_fn` is its reactive twin.
let unread = container.query::<Trip>().filter(Trip::done().eq(false)).live_count();
label(move || unread.get().to_string());
```

### The predicate vocabulary

Comparisons (`eq`, `ne`, `lt`, `le`, `gt`, `ge`, `between`), text (`contains`, `contains_ci`,
`starts_with`, `starts_with_ci`), sets (`is_in`, `not_in`), and presence (`is_null`,
`is_not_null`). A reference column adds `is(id)`, `is_one_of(ids)`, `is_set` and `is_unset`.
They compose with `&`, `|` and `!`.

```rust
container.query::<Paper>().filter(Paper::pages().is_in([12, 45, 96]))
container.query::<Paper>().filter(Paper::title().starts_with("Quick"))   // case-SENSITIVE
container.query::<Paper>().filter(Paper::shelf().is_null())
container.query::<Essay>().filter(Essay::author().is(ada) | Essay::author().is_unset())
```

A set is sorted once when the predicate is built, so membership is a binary search rather than
a scan, which keeps a large `is_in` cheap. **An empty set matches nothing** (in SQL it
compiles to a constant, because `IN ()` is a syntax error there rather than an empty set).
`starts_with` is not built on `LIKE`, because `LIKE`'s SQLite default is case-*insensitive*
for ASCII and would answer a different question.

### Asking about a row's relatives

A predicate can cross a declared relation. The derive emits `Trip::lodging()` (no receiver)
beside the instance accessor `trip.lodging()` that reads and writes it, and one quantifier
vocabulary covers both cardinalities, since a to-one is a to-many of at most one.

```rust
// Parent → children.
container.query::<Trip>().filter(Trip::lodging().any(Lodging::name().contains("Kyoto")))
container.query::<Trip>().filter(Trip::lodging().is_empty())
container.query::<Trip>().filter(Trip::lodging().count_ge(3))
container.query::<Trip>().filter(Trip::lodging().none(Lodging::confirmed().eq(false)))

// Child → the row its reference names.
container.query::<Lodging>().filter(Lodging::trip().any(Trip::done().eq(true)))

// Many-to-many, either direction, same vocabulary.
container.query::<Note>().filter(Note::tags().any(Tag::name().eq("rust".to_string())))
container.query::<Tag>().filter(Tag::notes().any(Note::title().starts_with("Draft")))

// They are ordinary predicates, so they compose.
container.query::<Trip>().filter(
    Trip::lodging().any(Lodging::confirmed().eq(true)) & Trip::done().eq(false),
)
```

| Form | Reads as | Over an empty relation |
|---|---|---|
| `any(p)` | some related row matches | `false` |
| `none(p)` | no related row matches | `true` |
| `all(p)` | every related row matches | **`true`** — vacuously, as in SQL |
| `is_empty()` | no related rows | `true` |
| `count_ge(n)` | at least `n` related rows | `n == 0` |

`all` over an empty relation is true, which is the reason `none` sits beside it: "no
unconfirmed lodging" and "every lodging confirmed" differ exactly for the rows with nothing
related, and an app usually means the former.

**Crossing a relation keeps the zero-cost tier.** A relation predicate compiles to a
correlated `EXISTS` over the relation's indexed foreign key (a join crosses through the join
table; nesting is unlimited, and each level is another subquery the engine plans). Its
dependency set names the far table's columns the inner predicate reads, so a related column
the predicate never mentions costs nothing at all; one it does read marks the query stale for
one requery. `is_empty` and `count_ge` compile to `NOT EXISTS` and a correlated `COUNT` over
the same index.

### NULL follows SQL's rule, in both paths

A comparison against a NULL column is **UNKNOWN**, not false, and UNKNOWN is not a match, so
`Paper::shelf().ne(Some("A".into()))` does not select a row whose shelf is NULL, exactly as
`WHERE shelf <> 'A'` does not. (A nullable column's `Col` is `Col<Option<T>>`, so its
comparisons take the wrapped value; `is_null`/`is_not_null` need no operand at all.) UNKNOWN propagates through `&`, `|` and `!` by SQL's own
three-valued logic, which lets the in-memory evaluation and the SQL form agree about NULL.
`is_null`/`is_not_null` stay definite, as `IS NULL` does.

Sorting is unaffected and keeps SQLite's `ORDER BY` rule, where NULL sorts below numbers;
ordering and comparison are different questions and day answers each the way SQL does.

### One evaluation path: the engine's

A fetch compiles whole (predicate to WHERE, sorts to ORDER BY with the key as a
deterministic tie-break, window to LIMIT), and the engine answers it from its indexes, at any
table size. Case-insensitive text predicates stay exact: the native driver registers
`day_fold` (Rust's full-Unicode `to_lowercase` as a scalar SQL function) at open, so `ÉCOLE`
folds in SQL exactly as it folds in Rust; SQLite's own `lower()` folds ASCII only and would
answer a different question. A driver without the function
(`Capabilities::unicode_fold`, false for the web engine today) takes a fallback: SQL runs the
exact conjuncts and carries the dependency columns, and the folding test re-checks over them
in memory, preserving order and window.

Liveness is dependency-gated invalidation: a change to a column the query never mentions
costs nothing (no SQL runs and nothing wakes); a change the query does depend on marks it
stale, and one requery after the turn's flush re-derives its ids. The fresh answer is diffed
against the previous one into `Insert`/`Remove`/`Move` deltas a list can animate (verified by
simulation before delivery; a delta list that would not land exactly on the new set reloads
instead). Reads are always current: `ids()`, `count()`, `take_events()` and the rest settle
pending staleness first, so imperative same-turn code never sees yesterday's answer. With
autosave off, queries answer from the last save, since only the file can answer them; that is
the one read-side consequence of deferring writes. The 600-edit agreement test in
`tests/liveset.rs` pins the property the deltas must hold: a mirror maintained purely from
the deltas lands, id for id, where a fresh fetch does.

`list(query, row)` (facade feature `persistence`) makes a query a row source: the query
holds ids, and rows fault in as the list binds them, a window at a time
(`Query::materialize(range)` is the same path, callable directly), so showing row 40,000 of
a large result costs one windowed `SELECT` rather than 40,000 resident rows. Bound rows stay
resident (observed rows never evict); scrolled-away rows may leave and fault back. Set
changes reach the native list as `ListPatch::Splice` row deltas it can animate; an
NSTableView slides the row out instead of reloading. Hosts that cannot animate treat a
splice as a reload.

There are three tiers of SQL access, each with its own price: the typed builder (incremental,
the default); `query_raw::<M>(sql, params, &["tables"])`, a read-only SELECT of ids, re-run
whole whenever a flush touches a named table; and `with_connection(|conn| …)`, the driver's
connection for maintenance and imports, whose writes bypass the change log entirely. Call
`rescan()` afterward: it re-reads the resident rows from the file (without re-marking them
dirty) and re-runs every query, at O(working set + results), never O(table).

## Full-text and spatial

Both of SQLite's own answers ship in the derive:

```rust
#[derive(Model, Clone, Default, PartialEq)]
#[model(table = "posts", fts("title", "body", tokenize = "unicode61 remove_diacritics 2"),
        spatial(lat = "lat", lon = "lon"))]
struct Post { /* … */ }

query.filter(Post::fts().matches("kyoto OR osaka")).sort(rank())   // bm25, best first
query.filter(Post::geo().within(GeoRect { min_lat, max_lat, min_lon, max_lon }))

// Crossing a relation composes with a match: the search box that reads two tables.
query.filter(Post::title().contains_ci(t) | Post::chapters().any(Chapter::fts().matches(t)))
```

`tokenize = "…"` selects the FTS5 tokenizer (the diacritics-folding `unicode61` form is what
a search field usually wants); it is part of the schema fingerprint, so changing it rebuilds
the shadow. A `matches` predicate works inside a relation crossing too (the compiled
subquery resolves the target model's shadow), which is how an app whose bodies live in a
separate model searches titles and bodies in one fetch.

The schema derive generates the standard patterns rather than asking you to hand-write them: an
external-content FTS5 table (`posts_fts`, `content=posts`) and an R*Tree table (`posts_geo`),
each kept true by three `AFTER INSERT/UPDATE/DELETE` triggers, inside the same transaction as
every write, and correct even for rows another tool writes into the file. (This is also why the
fold's upsert is a true `ON CONFLICT DO UPDATE` and not `INSERT OR REPLACE`: the latter's
implicit delete skips the delete triggers unless `recursive_triggers` is on, and the index
would rot.) A freshly created shadow backfills from existing rows.

Both predicates compile through their shadows. `matches` is a subquery over the FTS5 table
(`rank()` joins it so bm25 orders the result), and its dependency set is the indexed columns:
a change to one of them re-queries after the flush, when the triggers have run, and a change
to any column outside the set still costs nothing. `within` narrows through the R*Tree first
and re-checks the exact ranges; the shadow stores 32-bit outward-rounded entries, a
candidate superset that can never miss. A driver whose engine lacks FTS5 or R*Tree fails
`open` with the module named, an error at open rather than a silent downgrade at query time.

## Undo

```rust
let stack = container.undo(100);   // one history over every store, 100 units deep
day::install_undo(&stack);         // native fronts where the platform has them
```

The unit of undo is a turn (everything one event's dispatch changed), inverted from the same
change log the SQL fold reads: a `Set` inverts by writing the prior back, a `Delete` carries
the row it removed and inverts to an insert, an `Insert` inverts to a delete. An undo replays
those inverses (in reverse, in one batch) tagged with the author `undo`, and everything
downstream handles them as ordinary changes: field triggers wake exactly what changed back,
live queries animate the row's return, and autosave writes the inverse statements, so undoing
a delete is one `INSERT` rather than a snapshot. `grouped(label, f)` makes a multi-write
gesture one unit; `set_label_resolver` turns unit labels into display text;
`can_undo`/`undo_label` are signals a button or menu wires to directly. The stack lives in
day-model, so a plain in-memory store undoes identically; undo does not depend on persistence.

Where the platform has its own undo system, `day::install_undo` mirrors the stack into a native
front (`Cap::UndoBridge`): on macOS the window answers `undoManager` with a Day-owned
`NSUndoManager` subclass, so the stock Edit menu retitles and enables itself and ⌘Z lands
through the responder chain, while a focused text field's own manager keeps precedence, which
is the typing rule. iOS gets the same front through the root view controller, so the three-finger
gestures, shake-to-undo and the iPad menu bar all reach the one stack. Everywhere else the
app's own affordances call `stack.undo()` directly.

## Sessions: preview, then commit

A slider mid-drag must move the model live (labels and swatches render from it) while the
durable fact is the settled value. The `Binding` trait carries the split, `write_preview`
(from `Event::ValueChanged`) and `write_commit` (from `Event::ValueCommitted`), and a
day-model field implements it as a session: previews land in the store and wake this field's
readers, but no change record exists until the commit, which announces one record whose prior
is the pre-drag value. Sixty thumb positions cost sixty UI wakes (their job), one undo unit,
and one `UPDATE`. Text fields ride the same machinery: each keystroke previews, and Return or
focus loss commits; that is the typing coalescer. `field.session()` gives the explicit form
(`preview`/`commit`/`cancel`; Escape restores and writes zero records); a plain `Signal`
binding defaults both methods to `write`, so nothing changes where no session semantics exist.
A session left open (focus never leaves a field before the process dies) has not committed,
so its typing is not yet in the change log or the file.

## Encryption

With the `sqlite-cipher` engine, `.key(Secret::new(…))` opens an encrypted database; a wrong or
missing key is `DbErrorKind::BadKey` at open, not a decode error later. `rekey` re-encrypts in
place; `encrypt_to`/`decrypt_to` write converted copies (SQLCipher's own export path; the
conversion cannot happen in place). Day does not store the key: it arrives as a `Secret` that
never prints and is zeroed on drop, and where it lives between launches (the OS keychain is the
usual place) is the app's decision. Builds without the engine reject `.key` with an error at
open.

## Maintenance

`backup_to(path)` is `VACUUM INTO`: a transactionally consistent, already-compacted snapshot
taken while the app keeps writing, after flushing what is pending; there is no restore verb,
because restoring is opening the copy. `integrity_check()` parses the PRAGMA into findings
(empty means sound).
`checkpoint()` folds the WAL into the main file; run it before the OS copies the file whole
(device backup). `vacuum()` compacts in place; `size_bytes()` answers the settings-page
question.

## Relations

`One<M>` is a to-one reference: the foreign-key column on the child, stored in the target's
own key shape. `Many<M>` is the other side: a marker field storing nothing, whose accessor is
a view over the children's foreign keys, one indexed `SELECT` on a parent's first read,
memoized until a membership write invalidates it, and overlaid with the turn's unflushed
edits so mid-turn reads include them. The child's foreign key is the only stored fact, so
maintained inverses fall out of the pipeline that already exists, with no parallel
bookkeeping. The API shape (`@Model`-style declarations, delete rules on the relationship)
follows SwiftData's vocabulary, so the concepts transfer:

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
wakes exactly the readers of that parent's relation rather than every reader of the table.

**Delete rules** are declared on the `Many` side and default to `nullify` (SwiftData's):
children survive and their references clear, which requires `Option<One<M>>`; wiring refuses
`nullify` over a required reference, naming the fix. `cascade` deletes the children with the
parent, recursively and through the normal pipeline: the children come from one indexed
`SELECT`, their deletes fold into chunked statements, and the engine's own `ON DELETE`
clause backstops rows this process never saw. With an undo stack installed the cascade
materializes the rows it removes, so it is one undo unit that restores the whole subtree.
`deny` refuses while children remain, through `container.delete::<M>(id)`, the checked path; a
raw `restructure` delete cannot be refused after the fact, so deny is the one rule that needs
it. The generated DDL carries the matching `REFERENCES … ON DELETE …` clause, `DEFERRABLE
INITIALLY DEFERRED` so statement order within a transaction never trips it, which also
enforces the same rules on any other process writing the file.

**Ordered to-many** (`ordered = "field"`) names a `f64` field of the child that holds its
position, a real, visible column. Placement is fractional, so a drag writes **one row**; when a
gap bisects away the siblings rebalance to whole numbers first (O(n), rare, still one statement
per row). `insert_at(child, i)` and `move_to(child, i)` are the verbs; an unordered relation
refuses both.

**Many-to-many** declares `join = "table"` instead of an inverse. The derive generates the join
table (the pair as its primary key, a foreign key per side, an index on the reverse side), and
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

## Attached databases and links

A container can hold a second SQLite file that someone else owns — a catalog an app
downloads and replaces wholesale, a database another tool builds — and read it through the
same caches, queries, and lists as its own tables:

```rust
#[derive(Model, Clone, Default, PartialEq)]
#[model(table = "stations", external = "catalog", fts("name"))]
pub struct Station {
    #[model(id, column = "rowid")]
    pub id: u64,                      // the file's implicit rowid: stable within one file
    pub uuid: Vec<u8>,                // the catalog's own key, stable across files
    pub name: String,
    pub country: String,
    #[model(link(target = Favorite, local = "uuid", remote = "station"))]
    pub favorites: Linked<Favorite>,  // a marker: no column
}

#[derive(Model, Clone, Default, PartialEq)]
#[model(table = "favorites")]
pub struct Favorite {
    #[model(id)]
    pub id: u64,
    pub station: Vec<u8>,             // the catalog row's uuid, by VALUE
    #[model(link(target = Station, local = "station", remote = "uuid"))]
    pub link: Linked<Station>,
}

let container = ModelContainer::open(Sqlite::app_data("tunes.db")?, schema![Favorite])?;
container.attach_database("catalog", path_to_stations_sqlite, schema![Station])?;

container.query::<Favorite>().filter(Favorite::link().any(Station::country().eq("US")))
container.query::<Station>().filter(Station::favorites().is_empty())
```

`external = "alias"` marks a model whose table lives in the database ATTACHed under `alias`:
its `TABLE` is the qualified `alias.table`, and the container creates, migrates, and
fingerprints nothing for it — the file is read as it is, and attached read-only where the
engine honors `mode=ro`, so a write to an external row fails at flush. Columns map by name
like any model's; a `NULL` reads as the field's default, which is how a catalog's nullable
`TEXT` lands in a plain `String`. A table whose primary key is not an integer (a `BLOB` uuid,
a composite) is addressed through its implicit rowid: `#[model(id, column = "rowid")]`. A
`WITHOUT ROWID` table has no such handle and is not a model; read it with `with_connection`.
An external model may declare `fts(…)`, which names an index the file already carries
(`alias.table_fts`, the same `content=` or contentless FTS5 the file's builder made);
`matches`, `rank()`, and their dependency tracking work unchanged.

`attach_database(alias, path, schema)` attaches the file and installs the schema's models;
calling it again with the same alias swaps files: the old one is detached, every resident row
of its models is dropped, and every live query re-runs — a downloaded update lands with one
call. `detach_database(alias)` leaves the models registered and empty. Both flush first, since
SQLite refuses `ATTACH`/`DETACH` inside an open write transaction.

**Links** relate rows by value rather than by key: `#[model(link(target = T, local =
"column", remote = "column"))]` on a `Linked<T>` marker field declares that this model's
`local` column names rows of `T` whose `remote` column equals it. No foreign key is written
(SQLite cannot enforce one across databases, and the catalog side is not ours to constrain),
so a favorite whose station left the catalog simply has an empty link, which `is_empty()`
finds. A link's predicate builder is the relation vocabulary — `any`, `none`, `all`,
`is_empty`, `count_ge` — compiled to the same correlated `EXISTS`, joined on the two columns;
declaring the link on both sides gives both directions. The query watcher treats the owner's
`local` column as membership: rewriting it moves the row across the link. Links work between
two of the container's own tables too; they are simply relations without a key.

## External changes

Another connection's committed writes (another process, a sync engine, a CLI editing the
file) do not announce themselves. `check_external()` looks: detection is one
`PRAGMA data_version` (the counter moves only when another connection commits, never for this
one's own writes), so it is cheap enough to wire to app foreground, window focus, or a timer.
When the counter moved, pending local edits flush first, then only the resident rows re-read
and diff, at O(working set) rather than O(table). Changed fields announce per column,
disappearances take the structural path, every live query re-derives (a list animates the
row another process inserted), and rows that arrived elsewhere become visible through the
re-run queries and fault in like any other row. The merged changes are tagged
`ModelContainer::EXTERNAL_AUTHOR` (`"database"`): the autosave fold declines the echo, an undo
stack skips them (another author's writes are not the user's history), and any change sink can
tell them from the user's edits. A row another connection rewrote arrives whole, so its
`#[model(transient)]` fields reset to their defaults, exactly as at fault.

`Capabilities::external_changes` says whether detection is real: the built-in driver claims it
for file databases on native targets; memory databases (no second connection can reach one),
the Recorder, and the web engine (its OPFS access is exclusive) answer `Ok(false)`. Writes
made through `with_connection` are this connection's own, so the counter does not move for
them; that recovery stays `rescan()`. `tests/external.rs` is the reference suite.

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

`with_table` registers the rows any `SELECT … FROM` that table answers with, by table rather
than by statement, so faulting paths keep only the keys they asked for, and a query against
the Recorder answers with every fixture key whatever its predicate says. Assert the SQL the
query compiled (the Recorder's real value), and use `Sqlite::memory()` where predicate
results themselves are under test. `log.sql()` and
`log.entries()` expose everything issued, parameters included. The crate's own suites in
`crates/day-persistence/tests/` are the reference: the fold rules against the Recorder
(`container.rs`), the derive (`derive.rs`), real files, coexistence, backups and migration
(`sqlite.rs`), and the cipher lifecycle (`cipher.rs`, `--features cipher`).

## Not in this version

External storage (`#[model(external)]`) is a later phase, and eviction order is
insertion-order rather than true LRU (true LRU is proposed, not promised). Row
identity is the `#[model(id)]` key in its own stored shape (`INTEGER`, a 16-byte `BLOB` for
`Uuid`, `TEXT` for a string key; see [model.md](model.md)); a model's own display order is a
projection concern and is not persisted, though an ordered relation's position is.

The key layer refuses two shapes rather than mis-serving them. `fts(…)`/`spatial(…)` need an
integer key (both address rows by SQLite's ROWID), and a key field takes no codec: its stored
form is the key's own canonical one, because the fold's `WHERE` parameters and the merge's
decoding derive it from the key kind.

## Watching the SQL

`Sqlite::trace_sql(f)` installs the engine's own statement trace (`sqlite3_trace_v2` with
`SQLITE_TRACE_STMT`): `f` sees every statement the connection executes (migrations, autosave
flushes, live-query `SELECT`s, undo replays, maintenance) with bound parameters expanded by
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

OPFS, the browser's origin-private file system, exposes its only synchronous random-access
API inside dedicated workers, so on web-dom the engine runs in one: the **day-sql worker**, a
second instantiation of the app's own wasm module that day-cli's host page spawns at boot.
Every `SqliteConnection` call crosses a SharedArrayBuffer as one request; the main thread
blocks the few microseconds until the reply state flips, and the worker flushes to storage
before answering, so a commit that returned has landed and `capabilities().durable` is true.
`:memory:` databases skip the channel and run in-process. Apps see none of this: `Sqlite::at`,
autosave, undo, queries, and `backup_to` behave as they do everywhere else.

What the page must provide:

- **Cross-origin isolation.** SharedArrayBuffer exists only under COOP/COEP response headers.
  `day launch` sends them; a static deployment must too ([docs/web.md](web.md)). Without
  isolation there is no worker: memory databases still work, file opens fail with
  `Unsupported`, and `durable` reads false, which the app can show in its UI.
- **One tab.** OPFS access handles are exclusive, so the first tab of the app owns its
  databases; a second tab runs memory-only until the first closes.

`Sqlite::web_storage()` is the document-pool surface for file-per-document apps:
`exists`/`list` for numbering a fresh document, `import_db` for landing an Open… pick,
`export_db` for handing bytes to a download (flush first, so the image includes the current
turn). Database names are the "paths" the rest of the app already uses; `Sqlite::at("a.db")`
on the web opens the pool entry named `a.db`.
