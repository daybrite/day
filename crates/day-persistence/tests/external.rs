// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Another connection's committed writes, merged: `check_external` detects them through the
//! engine's `data_version` counter, feeds only the differences through the stores — precise
//! field announcements, structural inserts and deletes, live-query deltas — and never echoes
//! them back to the file.

use day_macros::Model;
use day_model::Op;
use day_persistence::{ModelContainer, Recorder, Sqlite, SqliteConnection, SqliteDriver, schema};
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
        "day-persistence-ext-{}-{}.sqlite",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    path
}

fn seeded(path: &std::path::Path) -> ModelContainer {
    let container = ModelContainer::open(Sqlite::at(path), schema![Note]).expect("open");
    let store = container.store::<Note>();
    for (id, title) in [(1, "first"), (2, "second"), (3, "third")] {
        store.restructure("add", Op::Insert, id, |v| {
            v.push(Note {
                id: id as u32,
                title: title.into(),
                pinned: false,
            });
        });
    }
    container.save().expect("save");
    container
}

/// A second connection to the same file — what another process looks like to this one.
fn second_connection(path: &std::path::Path) -> impl SqliteConnection {
    Sqlite::at(path).open().expect("second connection")
}

#[test]
fn another_connections_writes_arrive_precisely_and_do_not_echo() {
    let path = temp_db("merge");
    let container = seeded(&path);
    let store = container.store::<Note>();

    {
        let mut other = second_connection(&path);
        other
            .execute("UPDATE notes SET title = 'renamed' WHERE id = 1", &[])
            .expect("external update");
        other
            .execute(
                "INSERT INTO notes (id, title, pinned) VALUES (4, 'fourth', 1)",
                &[],
            )
            .expect("external insert");
        other
            .execute("DELETE FROM notes WHERE id = 3", &[])
            .expect("external delete");
    }

    let ((), changes) = day_model::record_changes(|| {
        assert!(container.check_external().expect("check_external"));
    });

    // Every announced change carries the external author, and the field change is per column:
    // `pinned` never changed on row 1, so only `title` announced there.
    assert!(!changes.is_empty());
    for c in &changes {
        assert_eq!(c.author, Some(ModelContainer::EXTERNAL_AUTHOR), "{:?}", c);
    }
    let row1: Vec<_> = changes
        .iter()
        .filter(|c| c.components.get(1) == Some(&1) && c.op == Op::Set)
        .collect();
    assert_eq!(row1.len(), 1);
    assert_eq!(row1[0].label, "title");

    // The store followed the file.
    assert_eq!(store.elem(1).title().peek(), "renamed");
    assert_eq!(store.elem(4).title().peek(), "fourth");
    assert!(!store.with_untracked(|k| k.get(3).is_some()), "row 3 gone");

    // Nothing echoes back: the merge left nothing dirty, so a flush issues no statements.
    let sql = container.record_sql(|| {}).expect("flush");
    assert!(
        sql.is_empty(),
        "external changes must not write back: {sql:?}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_live_query_follows_the_merge() {
    let path = temp_db("query");
    let container = seeded(&path);
    let q = container
        .query::<Note>()
        .filter(Note::pinned().eq(false))
        .live();
    assert_eq!(q.ids().len(), 3);
    let _ = q.take_events();

    {
        let mut other = second_connection(&path);
        other
            .execute("UPDATE notes SET pinned = 1 WHERE id = 2", &[])
            .expect("external update");
    }
    assert!(container.check_external().expect("check_external"));

    assert_eq!(q.ids(), [1, 3], "row 2 left the unpinned set");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_unchanged_file_and_our_own_writes_report_nothing() {
    let path = temp_db("quiet");
    let container = seeded(&path);
    let store = container.store::<Note>();

    // Nothing external happened.
    assert!(!container.check_external().expect("check_external"));

    // Our own writes never trip the detector: data_version moves only for OTHER connections.
    store.elem(1).title().write("mine".into());
    container.save().expect("save");
    assert!(!container.check_external().expect("check_external"));
    assert_eq!(store.elem(1).title().peek(), "mine", "local edit intact");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_unflushed_local_edit_survives_the_merge() {
    let path = temp_db("pending");
    let container = seeded(&path);
    container.set_autosave(false);
    let store = container.store::<Note>();

    // A local edit still pending when the external write arrives on a DIFFERENT row.
    store.elem(2).title().write("local, unflushed".into());
    {
        let mut other = second_connection(&path);
        other
            .execute("UPDATE notes SET title = 'external' WHERE id = 1", &[])
            .expect("external update");
    }
    assert!(container.check_external().expect("check_external"));

    // check_external flushed the local edit before diffing, so it read as ours — not as a
    // difference to revert.
    assert_eq!(store.elem(2).title().peek(), "local, unflushed");
    assert_eq!(store.elem(1).title().peek(), "external");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn merged_changes_are_not_the_users_history() {
    let path = temp_db("undo");
    let container = seeded(&path);
    let undo = container.undo(10);

    {
        let mut other = second_connection(&path);
        other
            .execute("UPDATE notes SET title = 'external' WHERE id = 1", &[])
            .expect("external update");
    }
    assert!(container.check_external().expect("check_external"));
    day_reactive::flush_sync();
    assert!(
        !undo.can_undo().get_untracked(),
        "another author's writes are not undoable"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn drivers_without_detection_say_so() {
    // Memory: no second connection can reach a private in-memory database.
    let memory = ModelContainer::open(Sqlite::memory(), schema![Note]).expect("open");
    assert!(!memory.capabilities().external_changes);
    assert!(!memory.check_external().expect("check_external"));

    // The Recorder answers from fixtures; there is no file for another connection to write.
    let (driver, _log) = Recorder::new();
    assert!(!driver.capabilities().external_changes);
    let recorded = ModelContainer::open(driver, schema![Note]).expect("open");
    assert!(!recorded.check_external().expect("check_external"));
}
