//! Per-app sqlite with append-only migrations (docs/lite.md §7.1). One database file per
//! app id; `user_version` tracks how many migration steps have applied, and a `_day_lite_
//! migrations` table records each step's content hash so editing history (instead of
//! appending) is caught rather than silently divergent.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use rusqlite::Connection;
use rusqlite::types::Value as SqlValue;

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
    fn to_sql(&self) -> SqlValue {
        match self {
            Cell::Null => SqlValue::Null,
            Cell::Int(i) => SqlValue::Integer(*i),
            Cell::Real(f) => SqlValue::Real(*f),
            Cell::Text(t) => SqlValue::Text(t.clone()),
        }
    }

    fn from_sql(v: SqlValue) -> Cell {
        match v {
            SqlValue::Null => Cell::Null,
            SqlValue::Integer(i) => Cell::Int(i),
            SqlValue::Real(f) => Cell::Real(f),
            SqlValue::Text(t) => Cell::Text(t),
            SqlValue::Blob(b) => Cell::Text(String::from_utf8_lossy(&b).into_owned()),
        }
    }
}

/// The app-scoped database handle (main thread only, like everything in the runtime).
#[derive(Clone)]
pub struct Db(Rc<RefCell<Connection>>);

impl Db {
    /// Open (creating parents) the app's database.
    pub fn open(path: PathBuf) -> Result<Db, DbError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(err)?;
        }
        let conn = Connection::open(&path).map_err(err)?;
        conn.execute_batch("pragma foreign_keys = on;")
            .map_err(err)?;
        Ok(Db(Rc::new(RefCell::new(conn))))
    }

    /// In-memory database (tests).
    pub fn memory() -> Result<Db, DbError> {
        let conn = Connection::open_in_memory().map_err(err)?;
        conn.execute_batch("pragma foreign_keys = on;")
            .map_err(err)?;
        Ok(Db(Rc::new(RefCell::new(conn))))
    }

    /// Apply the tail of an append-only migration history (docs/lite.md §7.1).
    pub fn migrate(&self, steps: &[String]) -> Result<u64, DbError> {
        let conn = self.0.borrow_mut();
        conn.execute_batch(
            "create table if not exists _day_lite_migrations (
                 step integer primary key, hash text not null
             );",
        )
        .map_err(err)?;
        let applied: u64 = conn
            .query_row("pragma user_version", [], |r| r.get::<_, i64>(0))
            .map_err(err)? as u64;
        if (steps.len() as u64) < applied {
            return Err(DbError(format!(
                "migration history shrank: {applied} applied, {} provided",
                steps.len()
            )));
        }
        // Recorded prefix must match verbatim — history is append-only.
        for (i, step) in steps.iter().take(applied as usize).enumerate() {
            let want: String = conn
                .query_row(
                    "select hash from _day_lite_migrations where step = ?1",
                    [i as i64],
                    |r| r.get(0),
                )
                .map_err(|_| DbError(format!("migration {i} was applied but is unrecorded")))?;
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
                rusqlite::params![i as i64, content_hash(step)],
            )
            .map_err(err)?;
            conn.pragma_update(None, "user_version", (i + 1) as i64)
                .map_err(err)?;
        }
        Ok(steps.len() as u64)
    }

    pub fn exec(&self, sql: &str, params: &[Cell]) -> Result<(u64, i64), DbError> {
        let conn = self.0.borrow_mut();
        let mut stmt = conn.prepare(sql).map_err(err)?;
        let sql_params: Vec<SqlValue> = params.iter().map(Cell::to_sql).collect();
        let changes = stmt
            .execute(rusqlite::params_from_iter(sql_params))
            .map_err(err)? as u64;
        Ok((changes, conn.last_insert_rowid()))
    }

    pub fn query(&self, sql: &str, params: &[Cell]) -> Result<Vec<Vec<(String, Cell)>>, DbError> {
        let conn = self.0.borrow_mut();
        let mut stmt = conn.prepare(sql).map_err(err)?;
        let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let sql_params: Vec<SqlValue> = params.iter().map(Cell::to_sql).collect();
        let mut rows = stmt
            .query(rusqlite::params_from_iter(sql_params))
            .map_err(err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(err)? {
            let mut obj = Vec::with_capacity(names.len());
            for (i, name) in names.iter().enumerate() {
                let v: SqlValue = row.get(i).map_err(err)?;
                obj.push((name.clone(), Cell::from_sql(v)));
            }
            out.push(obj);
        }
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
