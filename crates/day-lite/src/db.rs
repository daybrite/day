// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Per-app sqlite with append-only migrations (docs/lite.md §7.1). One database file per
//! app id; `user_version` tracks how many migration steps have applied, and a `_day_lite_
//! migrations` table records each step's content hash so editing history (instead of
//! appending) is caught rather than silently divergent.
//!
//! The engine is day-persistence's driver ([`day_persistence::Sqlite`]): a superapp carrying
//! both crates compiles ONE SQLite, and the app's engine features (`sqlite-system`,
//! `sqlite-cipher`) govern miniapp storage too. Journal mode stays SQLite's default, as it
//! always was for these files.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use day_persistence::{Sqlite, SqliteConnection, SqliteDriver, Value};

#[derive(Debug, Clone)]
pub struct DbError(pub String);

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for DbError {}

fn err<E: std::fmt::Display>(e: E) -> DbError {
    DbError(e.to_string())
}

/// A JSON-ish value crossing the JS boundary as a sqlite parameter or cell.
#[derive(Clone, Debug)]
pub enum Cell {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
}

impl Cell {
    fn to_value(&self) -> Value {
        match self {
            Cell::Null => Value::Null,
            Cell::Int(i) => Value::Int(*i),
            Cell::Real(f) => Value::Real(*f),
            Cell::Text(t) => Value::Text(t.clone()),
        }
    }

    fn from_value(v: Value) -> Cell {
        match v {
            Value::Null => Cell::Null,
            Value::Int(i) => Cell::Int(i),
            Value::Real(f) => Cell::Real(f),
            Value::Text(t) => Cell::Text(t),
            Value::Blob(b) => Cell::Text(String::from_utf8_lossy(&b).into_owned()),
        }
    }
}

/// The app-scoped database handle (main thread only, like everything in the runtime).
#[derive(Clone)]
pub struct Db(Rc<RefCell<Box<dyn SqliteConnection>>>);

impl Db {
    /// Open (creating parents) the app's database.
    pub fn open(path: PathBuf) -> Result<Db, DbError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(err)?;
        }
        let conn: Box<dyn SqliteConnection> =
            Box::new(Sqlite::at(path).wal(false).open().map_err(err)?);
        Ok(Db(Rc::new(RefCell::new(conn))))
    }

    /// In-memory database (tests).
    pub fn memory() -> Result<Db, DbError> {
        let conn: Box<dyn SqliteConnection> = Box::new(Sqlite::memory().open().map_err(err)?);
        Ok(Db(Rc::new(RefCell::new(conn))))
    }

    /// Apply the tail of an append-only migration history (docs/lite.md §7.1).
    pub fn migrate(&self, steps: &[String]) -> Result<u64, DbError> {
        let mut conn = self.0.borrow_mut();
        conn.execute_batch(
            "create table if not exists _day_lite_migrations (
                 step integer primary key, hash text not null
             );",
        )
        .map_err(err)?;
        let mut applied: u64 = 0;
        conn.query("pragma user_version", &[], &mut |row| {
            if let Ok(v) = row.get(0).as_int() {
                applied = v as u64;
            }
        })
        .map_err(err)?;
        if (steps.len() as u64) < applied {
            return Err(DbError(format!(
                "migration history shrank: {applied} applied, {} provided",
                steps.len()
            )));
        }
        // Recorded prefix must match verbatim — history is append-only.
        for (i, step) in steps.iter().take(applied as usize).enumerate() {
            let mut want: Option<String> = None;
            conn.query(
                "select hash from _day_lite_migrations where step = ?1",
                &[Value::Int(i as i64)],
                &mut |row| {
                    if let Ok(h) = row.get(0).as_text() {
                        want = Some(h.to_string());
                    }
                },
            )
            .map_err(err)?;
            let want = want
                .ok_or_else(|| DbError(format!("migration {i} was applied but is unrecorded")))?;
            if want != content_hash(step) {
                return Err(DbError(format!(
                    "migration {i} changed after it was applied — migrations are append-only"
                )));
            }
        }
        for (i, step) in steps.iter().enumerate().skip(applied as usize) {
            conn.execute_batch(step)
                .map_err(|e| DbError(format!("migration {i}: {e}")))?;
            conn.execute(
                "insert into _day_lite_migrations (step, hash) values (?1, ?2)",
                &[Value::Int(i as i64), Value::Text(content_hash(step))],
            )
            .map_err(err)?;
            conn.execute(&format!("pragma user_version = {}", i + 1), &[])
                .map_err(err)?;
        }
        Ok(steps.len() as u64)
    }

    pub fn exec(&self, sql: &str, params: &[Cell]) -> Result<(u64, i64), DbError> {
        let mut conn = self.0.borrow_mut();
        let vals: Vec<Value> = params.iter().map(Cell::to_value).collect();
        let changes = conn.execute(sql, &vals).map_err(err)?;
        let mut rowid = 0i64;
        conn.query("select last_insert_rowid()", &[], &mut |row| {
            if let Ok(v) = row.get(0).as_int() {
                rowid = v;
            }
        })
        .map_err(err)?;
        Ok((changes, rowid))
    }

    pub fn query(&self, sql: &str, params: &[Cell]) -> Result<Vec<Vec<(String, Cell)>>, DbError> {
        let mut conn = self.0.borrow_mut();
        let vals: Vec<Value> = params.iter().map(Cell::to_value).collect();
        let mut out = Vec::new();
        conn.query_named(sql, &vals, &mut |names, row| {
            let mut obj = Vec::with_capacity(names.len());
            for (i, name) in names.iter().enumerate() {
                obj.push((name.clone(), Cell::from_value(row.get(i))));
            }
            out.push(obj);
        })
        .map_err(err)?;
        Ok(out)
    }
}

fn content_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x1_0000_01b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_once_and_are_append_only() {
        let db = Db::memory().unwrap();
        let m1 = vec!["create table todos (id integer primary key, title text);".to_string()];
        assert_eq!(db.migrate(&m1).unwrap(), 1);
        assert_eq!(db.migrate(&m1).unwrap(), 1); // reapply: no-op

        let mut m2 = m1.clone();
        m2.push("alter table todos add column done integer not null default 0;".into());
        assert_eq!(db.migrate(&m2).unwrap(), 2);

        // Editing an applied step is refused.
        let mut edited = m2.clone();
        edited[0] = "create table todos (id integer primary key);".into();
        assert!(db.migrate(&edited).is_err());
        // Shrinking history is refused.
        assert!(db.migrate(&m1).is_err());
    }

    #[test]
    fn exec_and_query_roundtrip() {
        let db = Db::memory().unwrap();
        db.migrate(&["create table t (n integer, s text);".into()])
            .unwrap();
        let (changes, _row) = db
            .exec(
                "insert into t (n, s) values (?1, ?2)",
                &[Cell::Int(7), Cell::Text("x".into())],
            )
            .unwrap();
        assert_eq!(changes, 1);
        let rows = db.query("select n, s from t", &[]).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(matches!(rows[0][0].1, Cell::Int(7)));
    }
}
