// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The real driver, on real files: rows survive a reopen, hand-made tables coexist, backups
//! open clean, and lightweight migration closes column gaps without losing data.

use day_macros::Model;
use day_model::Op;
use day_persistence::{ModelContainer, Sqlite, schema};
use day_reactive::Binding;

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "notes")]
struct Note {
    #[model(id)]
    id: u32,
    title: String,
    pinned: bool,
}

/// A fresh path under the OS temp dir; the test owns it for its lifetime.
fn temp_db(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "day-persistence-{}-{}.sqlite",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    path
}

#[test]
fn rows_survive_a_reopen() {
    let path = temp_db("reopen");
    {
        let container = ModelContainer::open(Sqlite::at(&path), schema![Note]).expect("open");
        let store = container.store::<Note>();
        store.restructure("add", Op::Insert, 1, |v| {
            v.push(Note {
                id: 1,
                title: "first".into(),
                pinned: false,
            });
        });
        store.restructure("add", Op::Insert, 2, |v| {
            v.push(Note {
                id: 2,
                title: "second".into(),
                pinned: true,
            });
        });
        store.elem(1).title().write("first, edited".into());
        store.restructure("remove", Op::Delete, 2, |v| {
            v.remove(2);
        });
        container.save().expect("save");
    }
    {
        let container = ModelContainer::open(Sqlite::at(&path), schema![Note]).expect("reopen");
        let store = container.store::<Note>();
        assert_eq!(store.keys(), [1]);
        assert_eq!(store.elem(1).title().peek(), "first, edited");
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn hand_made_tables_coexist_with_days() {
    let path = temp_db("coexist");
    {
        let conn = rusqlite::Connection::open(&path).expect("raw open");
        conn.execute_batch(
            "CREATE TABLE app_journal (entry TEXT); INSERT INTO app_journal VALUES ('mine');",
        )
        .expect("hand-made table");
    }
    {
        let container = ModelContainer::open(Sqlite::at(&path), schema![Note]).expect("open");
        let store = container.store::<Note>();
        store.restructure("add", Op::Insert, 1, |v| {
            v.push(Note {
                id: 1,
                title: "days".into(),
                pinned: false,
            });
        });
        container.save().expect("save");
        container.checkpoint().expect("checkpoint");
    }
    {
        let conn = rusqlite::Connection::open(&path).expect("raw reopen");
        let entry: String = conn
            .query_row("SELECT entry FROM app_journal", [], |r| r.get(0))
            .expect("hand-made row untouched");
        assert_eq!(entry, "mine");
        let title: String = conn
            .query_row("SELECT title FROM notes WHERE id = 1", [], |r| r.get(0))
            .expect("day row present");
        assert_eq!(title, "days");
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_backup_taken_mid_write_opens_clean() {
    let path = temp_db("backup-src");
    let backup = temp_db("backup-dst");
    let container = ModelContainer::open(Sqlite::at(&path), schema![Note]).expect("open");
    let store = container.store::<Note>();
    store.restructure("add", Op::Insert, 1, |v| {
        v.push(Note {
            id: 1,
            title: "saved".into(),
            pinned: false,
        });
    });
    container.save().expect("save");
    // Edits still pending when the backup is asked for — backup_to flushes them first, so the
    // snapshot is transactionally consistent and complete.
    container.set_autosave(false);
    store.elem(1).title().write("saved, then edited".into());
    container.backup_to(&backup).expect("backup");

    let restored = ModelContainer::open(Sqlite::at(&backup), schema![Note]).expect("open backup");
    assert_eq!(
        restored.store::<Note>().elem(1).title().peek(),
        "saved, then edited"
    );
    assert_eq!(
        restored.integrity_check().expect("check"),
        Vec::<String>::new()
    );
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&backup);
}

#[test]
fn an_added_column_backfills_and_a_dropped_one_goes() {
    // The same table, declared twice: yesterday's shape and today's.
    #[derive(Model, Clone, Default, PartialEq, Debug)]
    #[model(table = "tasks")]
    struct TaskV1 {
        #[model(id)]
        id: u32,
        title: String,
        legacy: String,
    }
    #[derive(Model, Clone, Default, PartialEq, Debug)]
    #[model(table = "tasks")]
    struct TaskV2 {
        #[model(id)]
        id: u32,
        title: String,
        count: i64,
    }

    let path = temp_db("migrate");
    {
        let container = ModelContainer::open(Sqlite::at(&path), schema![TaskV1]).expect("v1");
        container
            .store::<TaskV1>()
            .restructure("add", Op::Insert, 1, |v| {
                v.push(TaskV1 {
                    id: 1,
                    title: "carried".into(),
                    legacy: "gone tomorrow".into(),
                });
            });
        container.save().expect("save");
        container.checkpoint().expect("checkpoint");
    }
    {
        let container = ModelContainer::open(Sqlite::at(&path), schema![TaskV2]).expect("v2");
        let store = container.store::<TaskV2>();
        assert_eq!(store.elem(1).title().peek(), "carried");
        assert_eq!(store.elem(1).count().peek(), 0, "added column backfilled");
        container.checkpoint().expect("checkpoint");
    }
    {
        let conn = rusqlite::Connection::open(&path).expect("raw");
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(tasks)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(|c| c.unwrap())
            .collect();
        assert_eq!(
            cols,
            ["id", "title", "count"],
            "legacy dropped, count added"
        );
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_created_table_is_strict_and_readable_by_any_tool() {
    let path = temp_db("ddl");
    {
        let _c = ModelContainer::open(Sqlite::at(&path), schema![Note]).expect("open");
    }
    let conn = rusqlite::Connection::open(&path).expect("raw");
    let ddl: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'notes'",
            [],
            |r| r.get(0),
        )
        .expect("schema row");
    assert!(ddl.ends_with("STRICT"), "{ddl}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn size_and_vacuum_report_something_sane() {
    let path = temp_db("size");
    let container = ModelContainer::open(Sqlite::at(&path), schema![Note]).expect("open");
    container
        .store::<Note>()
        .restructure("add", Op::Insert, 1, |v| {
            v.push(Note {
                id: 1,
                title: "x".repeat(4096),
                pinned: false,
            });
        });
    container.save().expect("save");
    container.vacuum().expect("vacuum");
    assert!(container.size_bytes().expect("size") > 0);
    let _ = std::fs::remove_file(&path);
}

/// `trace_sql` rides the engine's own trace: migrations, the autosave fold's statements, and
/// query `SELECT`s all land in the sink, with bound parameters expanded by SQLite itself.
#[test]
fn trace_sql_logs_every_statement_the_engine_runs() {
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Model, Clone, Default, PartialEq)]
    #[model(table = "traced_notes")]
    struct TracedNote {
        #[model(id)]
        id: u32,
        body: String,
    }

    let seen: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    let driver = Sqlite::memory().trace_sql(move |sql| sink.borrow_mut().push(sql.to_string()));
    let container = ModelContainer::open(driver, schema![TracedNote]).expect("open");
    let store = container.store::<TracedNote>();
    store.update("seed", |k| {
        k.push(TracedNote {
            id: 1,
            body: "rain later".into(),
        });
    });
    container.save().expect("flush");

    let log = seen.borrow();
    assert!(
        log.iter()
            .any(|s| s.contains("CREATE TABLE") && s.contains("traced_notes")),
        "migration logged: {log:?}"
    );
    assert!(
        log.iter()
            .any(|s| s.contains("INSERT INTO traced_notes") && s.contains("'rain later'")),
        "flush logged with parameters expanded: {log:?}"
    );
}
