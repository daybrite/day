// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Failure injection over the real engine: cache/transaction retry contracts.
use day_macros::Model;
use day_persistence::{
    Capabilities, DbError, ModelContainer, Row, Sqlite, SqliteConnection, SqliteDriver, Value,
    schema,
};
use day_reactive::Binding;
use std::{cell::Cell, rc::Rc};

mod fixture {
    #[derive(Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    pub struct Metadata {
        pub title: String,
    }
}
#[derive(Model, Clone, Default, PartialEq)]
#[model(table = "recovery_books")]
struct Book {
    #[model(id)]
    id: String,
    title: String,
    #[model(json)]
    metadata: fixture::Metadata,
}
struct Driver(Rc<Cell<u8>>);
struct Connection(Box<dyn SqliteConnection>, Rc<Cell<u8>>);
impl SqliteDriver for Driver {
    type Connection = Connection;
    fn open(self) -> Result<Connection, DbError> {
        Ok(Connection(Box::new(Sqlite::memory().open()?), self.0))
    }
    fn capabilities(&self) -> Capabilities {
        Sqlite::memory().capabilities()
    }
}
impl SqliteConnection for Connection {
    fn execute(&mut self, sql: &str, p: &[Value]) -> Result<u64, DbError> {
        if (self.1.get() == 1 && sql == "BEGIN")
            || (self.1.get() == 2 && sql.starts_with("INSERT INTO recovery_books"))
            || (self.1.get() == 3 && sql == "COMMIT")
        {
            return Err(DbError::driver("synthetic write failure"));
        }
        self.0.execute(sql, p)
    }
    fn query(
        &mut self,
        sql: &str,
        p: &[Value],
        row: &mut dyn FnMut(&dyn Row),
    ) -> Result<(), DbError> {
        if self.1.get() == 4 {
            return Err(DbError::driver("synthetic read failure"));
        }
        self.0.query(sql, p, row)
    }
    fn execute_batch(&mut self, sql: &str) -> Result<(), DbError> {
        self.0.execute_batch(sql)
    }
}
#[test]
fn failed_begin_statement_and_commit_preserve_pending_changes_for_retry() {
    for stage in [1, 2, 3] {
        let fail = Rc::new(Cell::new(0));
        let db = ModelContainer::open(Driver(fail.clone()), schema![Book]).unwrap();
        db.set_autosave(false);
        db.insert(Book {
            id: "fixture".into(),
            title: "before".into(),
            ..Default::default()
        });
        let q = db.query::<Book>().live();
        fail.set(stage);
        assert!(db.save().is_err());
        assert_eq!(
            db.table_count::<Book>().unwrap(),
            0,
            "SQL was rolled back at stage {stage}"
        );
        assert!(db.try_get::<Book>("fixture".to_owned()).unwrap().is_some());
        let called = Cell::new(false);
        assert!(
            db.try_with_connection(|_| {
                called.set(true);
                Ok(())
            })
            .is_err()
        );
        assert!(!called.get(), "maintenance must not run over unsaved data");
        db.try_get::<Book>("fixture".to_owned())
            .unwrap()
            .unwrap()
            .title()
            .write("after failure".into());
        fail.set(0);
        db.save().unwrap();
        assert_eq!(db.table_count::<Book>().unwrap(), 1);
        assert_eq!(q.count(), 1);
        assert_eq!(db.last_error().get_untracked(), None);
        let mut title = String::new();
        db.try_with_connection(|c| {
            c.query("SELECT title FROM recovery_books", &[], &mut |r| {
                title = r.get(0).as_text().unwrap().to_owned()
            })
        })
        .unwrap();
        assert_eq!(title, "after failure");
    }
}
#[test]
fn checked_get_distinguishes_absence_from_failure() {
    let fail = Rc::new(Cell::new(0));
    let db = ModelContainer::open(Driver(fail.clone()), schema![Book]).unwrap();
    assert!(db.try_get::<Book>("absent".to_owned()).unwrap().is_none());
    fail.set(4);
    assert!(db.try_get::<Book>("absent".to_owned()).is_err());
    assert!(db.get::<Book>("absent".to_owned()).is_none());
    assert!(db.last_error().get_untracked().is_some());
}
#[test]
fn qualified_json_type_round_trips_through_generated_accessors() {
    let db = ModelContainer::open(Sqlite::memory(), schema![Book]).unwrap();
    db.insert(Book {
        id: "fixture".into(),
        metadata: fixture::Metadata {
            title: "qualified type".into(),
        },
        ..Default::default()
    });
    db.save().unwrap();
    db.rescan().unwrap();
    assert_eq!(
        db.try_get::<Book>("fixture".to_owned())
            .unwrap()
            .unwrap()
            .metadata()
            .read()
            .title,
        "qualified type"
    );
}

#[test]
fn query_reads_before_commit_do_not_consume_post_commit_notifications() {
    let db = ModelContainer::open(Sqlite::memory(), schema![Book]).unwrap();
    db.set_autosave(false);
    let rows = db.query::<Book>().live();
    let count = db.query::<Book>().live_count();
    db.insert(Book {
        id: "fixture".into(),
        ..Default::default()
    });
    assert_eq!(rows.count(), 0);
    assert_eq!(count.get(), 0);
    let late = db.query::<Book>().live();
    assert_eq!(late.count(), 0);
    db.save().unwrap();
    assert_eq!((rows.count(), count.get(), late.count()), (1, 1, 1));
}

#[test]
fn a_failed_query_keeps_its_last_result_and_checked_reads_report_the_error() {
    let fail = Rc::new(Cell::new(0));
    let db = ModelContainer::open(Driver(fail.clone()), schema![Book]).unwrap();
    db.insert(Book {
        id: "fixture".into(),
        ..Default::default()
    });
    db.save().unwrap();
    let q = db.query::<Book>().live();
    let count = db.query::<Book>().live_count();
    assert_eq!(q.try_collect().unwrap().len(), 1);
    fail.set(4);
    q.refresh();
    count.refresh();
    assert!(count.try_get().is_err());
    assert_eq!(count.get(), 1);
    assert!(q.try_ids().is_err());
    assert!(q.try_collect().is_err());
    assert_eq!(q.count(), 1, "failure must not masquerade as an empty list");
    fail.set(0);
    q.refresh();
    assert!(q.error().get_untracked().is_none());
    assert_eq!(q.try_collect().unwrap().len(), 1);
}
