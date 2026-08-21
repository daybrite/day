// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The container against the Recorder: the change-log fold as SQL, asserted headlessly with a
//! hand-written `Model` impl (what `#[derive(Model)]` will emit).

use day_macros::Observable;
use day_model::{Keyed, Op};
use day_persistence::{
    ColumnDef, ColumnValue, DbError, DbErrorKind, MigrationPlan, Model, ModelContainer, Recorder,
    Row, SqlType, Value, schema,
};
use day_reactive::Binding;

#[derive(Observable, Clone, Default, PartialEq, Debug)]
struct Note {
    #[obs(key)]
    id: u32,
    title: String,
    body: String,
    pinned: bool,
}

impl Model for Note {
    const TABLE: &'static str = "notes";
    const KEY: &'static str = "id";
    const COLUMNS: &'static [ColumnDef] = &[
        ColumnDef {
            name: "id",
            field: "id",
            sql: SqlType::Integer,
            not_null: true,
            unique: false,
            indexed: false,
        },
        ColumnDef {
            name: "title",
            field: "title",
            sql: SqlType::Text,
            not_null: true,
            unique: false,
            indexed: false,
        },
        ColumnDef {
            name: "body",
            field: "body",
            sql: SqlType::Text,
            not_null: true,
            unique: false,
            indexed: false,
        },
        ColumnDef {
            name: "pinned",
            field: "pinned",
            sql: SqlType::Integer,
            not_null: true,
            unique: false,
            indexed: false,
        },
    ];
    fn to_row(&self) -> Vec<Value> {
        vec![
            self.id.to_sqlite_value(),
            self.title.to_sqlite_value(),
            self.body.to_sqlite_value(),
            self.pinned.to_sqlite_value(),
        ]
    }
    fn from_row(row: &dyn Row) -> Result<Self, DbError> {
        fn col<T: ColumnValue + Default>(row: &dyn Row, i: usize) -> Result<T, DbError> {
            match row.get(i) {
                Value::Null => Ok(T::default()),
                v => T::from_sqlite_value(v),
            }
        }
        Ok(Note {
            id: col(row, 0)?,
            title: col(row, 1)?,
            body: col(row, 2)?,
            pinned: col(row, 3)?,
        })
    }
    fn default_row() -> Vec<Value> {
        Note::default().to_row()
    }
}

fn note_row(id: i64, title: &str, body: &str, pinned: i64) -> Vec<Value> {
    vec![
        Value::Int(id),
        Value::Text(title.into()),
        Value::Text(body.into()),
        Value::Int(pinned),
    ]
}

fn open_with_one_note() -> (ModelContainer, day_persistence::RecorderLog) {
    let (driver, log) = Recorder::new();
    let driver = driver.with_table("notes", vec![note_row(1, "before", "…", 0)]);
    let container = ModelContainer::open(driver, schema![Note]).expect("recorder open");
    log.clear();
    (container, log)
}

#[test]
fn twenty_keystrokes_fold_to_one_update() {
    let (container, _log) = open_with_one_note();
    let store = container.store::<Note>();

    let mut title = String::new();
    let sql = container
        .record_sql(|| {
            for ch in "typing, one keystroke".chars().take(20) {
                title.push(ch);
                store.elem(1).title().write(title.clone());
            }
        })
        .expect("save");

    assert_eq!(sql, ["UPDATE notes SET title = ? WHERE id = ?"]);
}

#[test]
fn the_update_carries_the_final_value() {
    let (container, log) = open_with_one_note();
    let store = container.store::<Note>();

    container
        .record_sql(|| {
            store.elem(1).title().write("draft".into());
            store.elem(1).title().write("final".into());
        })
        .expect("save");

    let entries = log.entries();
    let update = entries
        .iter()
        .find(|(sql, _)| sql.starts_with("UPDATE"))
        .expect("an UPDATE was issued");
    assert_eq!(
        update.1,
        vec![Value::Text("final".into()), Value::Int(1)],
        "the row is read at flush time, so only the last value reaches SQL"
    );
}

#[test]
fn two_fields_fold_into_one_update() {
    let (container, _log) = open_with_one_note();
    let store = container.store::<Note>();

    let sql = container
        .record_sql(|| {
            store.elem(1).title().write("t".into());
            store.elem(1).pinned().write(true);
            store.elem(1).title().write("tt".into());
        })
        .expect("save");

    assert_eq!(sql, ["UPDATE notes SET title = ?, pinned = ? WHERE id = ?"]);
}

#[test]
fn an_insert_absorbs_the_edits_that_fill_it() {
    let (container, _log) = open_with_one_note();
    let store = container.store::<Note>();

    let sql = container
        .record_sql(|| {
            store.restructure("add", Op::Insert, 2, |v| {
                v.push(Note {
                    id: 2,
                    ..Default::default()
                });
            });
            store.elem(2).title().write("new".into());
            store.elem(2).body().write("body".into());
        })
        .expect("save");

    assert_eq!(
        sql,
        [
            "INSERT INTO notes (id, title, body, pinned) VALUES (?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET title = excluded.title, body = excluded.body, pinned = excluded.pinned"
        ]
    );
}

#[test]
fn a_delete_absorbs_everything_before_it() {
    let (container, _log) = open_with_one_note();
    let store = container.store::<Note>();

    let sql = container
        .record_sql(|| {
            store.elem(1).title().write("doomed".into());
            store.restructure("remove", Op::Delete, 1, |v| {
                v.remove(1);
            });
        })
        .expect("save");

    assert_eq!(sql, ["DELETE FROM notes WHERE id = ?"]);
}

#[test]
fn fixture_rows_load_into_the_store() {
    let (driver, _log) = Recorder::new();
    let driver = driver.with_table(
        "notes",
        vec![note_row(1, "a", "…", 0), note_row(7, "b", "…", 1)],
    );
    let container = ModelContainer::open(driver, schema![Note]).expect("recorder open");
    let store = container.store::<Note>();

    let mut keys = store.keys();
    keys.sort_unstable();
    assert_eq!(keys, [1, 7]);
    assert!(store.elem(7).pinned().peek());
    assert_eq!(store.elem(1).title().peek(), "a");
}

#[test]
fn a_null_reads_as_the_field_default() {
    let (driver, _log) = Recorder::new();
    let driver = driver.with_table(
        "notes",
        vec![vec![Value::Int(1), Value::Null, Value::Null, Value::Null]],
    );
    let container = ModelContainer::open(driver, schema![Note]).expect("recorder open");
    let store = container.store::<Note>();

    assert_eq!(store.elem(1).title().peek(), "");
    assert!(!store.elem(1).pinned().peek());
}

#[test]
fn opening_creates_the_table_and_stores_a_fingerprint() {
    let (driver, log) = Recorder::new();
    let _container = ModelContainer::open(driver, schema![Note]).expect("recorder open");

    let sql = log.sql();
    assert!(
        sql.iter()
            .any(|s| s.starts_with("CREATE TABLE notes (id INTEGER PRIMARY KEY")),
        "table DDL was issued: {sql:?}"
    );
    assert!(
        sql.iter()
            .any(|s| s.contains("INSERT INTO _day_schema") && s.contains("fingerprint")),
        "fingerprint was stored: {sql:?}"
    );
}

#[test]
fn a_wholesale_rewrite_resyncs_the_table() {
    let (container, log) = open_with_one_note();
    let store = container.store::<Note>();

    store.update("import", |k| {
        *k = Keyed::new(vec![
            Note {
                id: 3,
                title: "kept".into(),
                ..Default::default()
            },
            Note {
                id: 4,
                title: "also".into(),
                ..Default::default()
            },
        ]);
    });
    log.clear();
    container.save().expect("save");

    let sql: Vec<String> = log
        .sql()
        .into_iter()
        .filter(|s| !matches!(s.as_str(), "BEGIN" | "COMMIT"))
        .collect();
    // The store no longer holds row 1: the resync upserts 3 and 4 and deletes 1 (the recorder's
    // fixture still answers the key scan with it).
    assert_eq!(sql.len(), 4, "{sql:?}");
    assert_eq!(
        sql.iter().filter(|s| s.starts_with("INSERT INTO")).count(),
        2
    );
    assert!(sql.iter().any(|s| s.starts_with("SELECT id FROM notes")));
    assert!(sql.iter().any(|s| s.starts_with("DELETE FROM notes")));
}

#[test]
fn autosave_flushes_at_turn_end() {
    let (container, log) = open_with_one_note();
    let store = container.store::<Note>();

    // A turn only drains when something observes — as the UI always does. One binding
    // stands in for it.
    day_reactive::Effect::new(move || {
        store.elem(1).title().read();
    });
    day_reactive::batch(|| {
        store.elem(1).title().write("typed".into());
    });

    assert!(
        log.sql()
            .iter()
            .any(|s| s.starts_with("UPDATE notes SET title")),
        "the turn's end flushed without an explicit save: {:?}",
        log.sql()
    );
}

#[test]
fn migration_stages_run_in_order_and_move_the_version() {
    let (driver, log) = Recorder::new();
    let plan = MigrationPlan::new()
        .custom(1, 2, |conn| {
            conn.execute("ALTER TABLE notes RENAME COLUMN heading TO title", &[])
                .map(|_| ())
        })
        .custom(0, 1, |conn| {
            conn.execute("UPDATE notes SET pinned = 0", &[]).map(|_| ())
        });
    let _container = ModelContainer::open_with(driver, schema![Note], plan).expect("recorder open");

    let sql = log.sql();
    let backfill = sql
        .iter()
        .position(|s| s.starts_with("UPDATE notes SET pinned"))
        .expect("stage 0→1 ran");
    let rename = sql
        .iter()
        .position(|s| s.starts_with("ALTER TABLE notes RENAME"))
        .expect("stage 1→2 ran");
    assert!(backfill < rename, "stages sorted by from-version: {sql:?}");
}

#[test]
fn a_newer_file_is_refused() {
    let (driver, _log) = Recorder::new();
    // The version probe reads column 0 of its SELECT; serve it a bare version row.
    let driver = driver.with_table("_day_schema", vec![vec![Value::Int(9)]]);
    let plan = MigrationPlan::new().custom(0, 1, |_| Ok(()));

    let err = ModelContainer::open_with(driver, schema![Note], plan)
        .err()
        .expect("open refused");
    assert_eq!(err.kind, DbErrorKind::Schema);
    assert!(err.message.contains("newer"), "{}", err.message);
}

#[test]
fn a_gap_in_the_stages_is_an_error() {
    let (driver, _log) = Recorder::new();
    let plan = MigrationPlan::new().custom(3, 4, |_| Ok(()));

    let err = ModelContainer::open_with(driver, schema![Note], plan)
        .err()
        .expect("open refused");
    assert_eq!(err.kind, DbErrorKind::Schema);
    assert!(err.message.contains("does not start"), "{}", err.message);
}

#[test]
fn fingerprints_answer_to_every_declared_detail() {
    // Same model, twice: stable.
    assert_eq!(
        day_persistence::model_fingerprint::<Note>(),
        day_persistence::model_fingerprint::<Note>()
    );

    #[derive(Observable, Clone, Default, PartialEq, Debug)]
    struct Other {
        #[obs(key)]
        id: u32,
        title: String,
    }
    impl Model for Other {
        const TABLE: &'static str = "notes";
        const KEY: &'static str = "id";
        const COLUMNS: &'static [ColumnDef] = &[
            ColumnDef {
                name: "id",
                field: "id",
                sql: SqlType::Integer,
                not_null: true,
                unique: false,
                indexed: false,
            },
            ColumnDef {
                name: "title",
                field: "title",
                sql: SqlType::Text,
                not_null: true,
                unique: false,
                indexed: false,
            },
        ];
        fn to_row(&self) -> Vec<Value> {
            vec![self.id.to_sqlite_value(), self.title.to_sqlite_value()]
        }
        fn from_row(row: &dyn Row) -> Result<Self, DbError> {
            Ok(Other {
                id: u32::from_sqlite_value(row.get(0))?,
                title: String::from_sqlite_value(row.get(1))?,
            })
        }
        fn default_row() -> Vec<Value> {
            Other::default().to_row()
        }
    }
    assert_ne!(
        day_persistence::model_fingerprint::<Note>(),
        day_persistence::model_fingerprint::<Other>(),
        "fewer columns, different fingerprint"
    );
}

#[test]
fn a_transient_labeled_change_writes_nothing() {
    let (container, _log) = open_with_one_note();
    let store = container.store::<Note>();

    // A row-level restructure that names no column: the fold treats it as a full-row write.
    let sql = container
        .record_sql(|| {
            store.restructure("touch", Op::Set, 1, |v| {
                if let Some(n) = v.get_mut(1) {
                    n.title = "still".into();
                }
            });
        })
        .expect("save");
    assert_eq!(
        sql,
        [
            "INSERT INTO notes (id, title, body, pinned) VALUES (?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET title = excluded.title, body = excluded.body, pinned = excluded.pinned"
        ],
        "a change that names no column resolves to a whole-row upsert, never silence"
    );
}
