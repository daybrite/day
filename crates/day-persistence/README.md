<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-persistence

SQLite storage for [Day](https://daybrite.dev)'s observable model. A `ModelContainer` opens a
database, creates or migrates each model's table, and reads no rows; rows enter memory when a
view or a query asks for them. Edits made through the model's bindings fold into the smallest
statement list at the end of each turn, so twenty keystrokes into one field are one `UPDATE`.

```toml
[dependencies]
day = { git = "https://github.com/daybrite/day.git", features = ["model"] }
day-persistence = { git = "https://github.com/daybrite/day.git" }
```

```rust
use day::prelude::*;
use day_persistence::prelude::*;

#[derive(Model, Clone, Default, PartialEq)]
pub struct Trip {
    #[model(id)]
    pub id: u32,
    #[model(index)]
    pub name: String,
    pub done: bool,
}

let container = ModelContainer::open(Sqlite::app_data("trips.db")?, schema![Trip])?;
let trips = container.cache::<Trip>();
let open = container
    .query::<Trip>()
    .filter(Trip::done().eq(false))
    .sort(Trip::name().asc())
    .live();
```

`list(open, row)` shows the query as a native list that faults in the rows it displays and
animates changes to the result set.

## Features

| Feature | Default | What it adds |
|---|---|---|
| `driver-rusqlite` | yes | The `Sqlite` driver: rusqlite on native targets, `day-sqlite-worker` over OPFS on the web. |
| `bundled` | yes | Compiles SQLite into the app. |
| `system` | | Links the operating system's libsqlite3. Turn `bundled` off with `default-features = false`. |
| `cipher` | | Builds SQLCipher with vendored crypto, for `.key(Secret)`. |
| `pieces` | yes | `list(query, row)`: a query as a day-pieces row source. |
| `datetime` | | Column codecs for day-piece-datetime's `DayDate` and `DayTime`. |

A crate that only implements a codec or a driver depends on it with
`default-features = false`, which compiles the traits and the `Recorder` without an engine.

## Layout

- `day-persistence`: the container, the fold, queries, relations, drivers.
- `day-persistence-macros`: `#[derive(Model)]`, re-exported as `day_persistence::Model`.
- `day-sqlite-worker`: the web engine. It carries the vendored SQLite build for wasm and, under
  `web/`, the two scripts that run it in a worker.

The three depend on Day through its public API, and the Day host page finds the web scripts
through `[package.metadata.day.web]` in `day-sqlite-worker`'s manifest.

## Documentation

[docs/persistence.md](../../docs/persistence.md) is the reference: models, the container,
drivers and engines, migrations, codecs, queries, relations, undo, encryption, and the web.
Its last section lists every framework API the crate uses.

## Tests

```sh
cargo test -p day-persistence --features datetime
cargo test -p day-persistence --features cipher --test cipher
```

The `Recorder` driver records every statement, so most suites assert the SQL a change produced
without a database on disk. `tests/sqlite.rs` and its neighbors run the same paths against real
files.
