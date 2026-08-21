// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The built-in driver: rusqlite, whose ENGINE is a cargo feature — `bundled` (the default),
//! `system` (link the OS's libsqlite3), or `cipher` (SQLCipher with vendored crypto). One
//! driver, recompiled; not a crate per engine.

use std::rc::Rc;

use rusqlite::Connection;

type InitHook = Rc<dyn Fn(&Connection)>;

use crate::{
    Capabilities, DbError, DbErrorKind, Location, OpenOptions, Row, Secret, SqliteConnection,
    SqliteDriver, Value,
};

/// The built-in SQLite driver. `Sqlite::at(path)` or `Sqlite::memory()`, then builder options.
pub struct Sqlite {
    opts: OpenOptions,
    /// Runs against the raw rusqlite connection right after open — register a custom SQL
    /// function, load an extension, set a PRAGMA. The framework neither knows nor cares.
    init: Option<InitHook>,
}

impl Sqlite {
    pub fn at(path: impl Into<std::path::PathBuf>) -> Self {
        Sqlite {
            opts: OpenOptions::new(Location::File(path.into())),
            init: None,
        }
    }
    pub fn memory() -> Self {
        Sqlite {
            opts: OpenOptions::new(Location::Memory),
            init: None,
        }
    }
    /// The database named `name` in the per-app data directory, created if missing — the
    /// day-part-fs root rules (docs/fs.md): `DAY_DATA_DIR` when the host passes one (the
    /// Android and OpenHarmony hosts do), the platform's app-data convention otherwise, and a
    /// `day-db/` leaf of its own beside day-part-fs' `day-fs/`. `name` is a file name, not a
    /// path. Errs where no such directory exists (web has no filesystem — keep a wasm build on
    /// [`Sqlite::memory`] or its own driver).
    pub fn app_data(name: impl AsRef<str>) -> Result<Self, DbError> {
        let name = name.as_ref();
        if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
            return Err(DbError::new(
                DbErrorKind::Unsupported,
                format!("app_data takes a file name, not a path: {name:?}"),
            ));
        }
        let dir = app_data_root()?;
        std::fs::create_dir_all(&dir)
            .map_err(|e| DbError::driver(format!("creating {}: {e}", dir.display())))?;
        Ok(Sqlite::at(dir.join(name)))
    }
    /// The database key (feature `cipher`; other builds refuse it at open, loudly).
    pub fn key(mut self, key: Secret) -> Self {
        self.opts.key = Some(key);
        self
    }
    /// Accept a file written by an older SQLCipher generation (`PRAGMA cipher_migrate`).
    pub fn cipher_migrate(mut self) -> Self {
        self.opts.cipher_migrate = true;
        self
    }
    pub fn wal(mut self, on: bool) -> Self {
        self.opts.wal = on;
        self
    }
    /// A per-connection hook over the raw rusqlite connection — the escape valve for loadable
    /// extensions, custom functions, and PRAGMAs this crate does not model.
    pub fn with_init(mut self, f: impl Fn(&Connection) + 'static) -> Self {
        self.init = Some(Rc::new(f));
        self
    }
}

impl SqliteDriver for Sqlite {
    type Connection = RusqliteConn;

    fn open(self) -> Result<RusqliteConn, DbError> {
        let conn = match &self.opts.location {
            Location::Memory => Connection::open_in_memory(),
            Location::File(p) => Connection::open(p),
        }
        .map_err(|e| DbError::driver(e.to_string()))?;

        if let Some(key) = &self.opts.key {
            #[cfg(feature = "cipher")]
            {
                // PRAGMA key takes no bound parameter; single-quote-escape the literal.
                let quoted = key.reveal().replace('\'', "''");
                conn.execute_batch(&format!("PRAGMA key = '{quoted}';"))
                    .map_err(|e| DbError::driver(e.to_string()))?;
                if self.opts.cipher_migrate {
                    let _ = conn.execute_batch("PRAGMA cipher_migrate;");
                }
            }
            #[cfg(not(feature = "cipher"))]
            {
                let _ = key;
                return Err(DbError::new(
                    DbErrorKind::Unsupported,
                    "this build has no encryption — enable the `cipher` feature",
                ));
            }
        }

        // A wrong (or missing) key surfaces on the first real read, not at PRAGMA time — probe
        // now so the caller gets BadKey at open instead of a raw engine error later. Only the
        // cipher build maps the failure to BadKey; a plaintext build's unreadable file is
        // corruption, not a key problem.
        #[cfg(feature = "cipher")]
        if conn
            .prepare("SELECT count(*) FROM sqlite_master")
            .and_then(|mut s| s.query_row([], |_| Ok(())))
            .is_err()
        {
            return Err(DbError::new(
                DbErrorKind::BadKey,
                "the database would not open with this key",
            ));
        }

        conn.busy_timeout(std::time::Duration::from_millis(
            self.opts.busy_timeout_ms as u64,
        ))
        .map_err(|e| DbError::driver(e.to_string()))?;
        if self.opts.foreign_keys {
            conn.execute_batch("PRAGMA foreign_keys = ON;")
                .map_err(|e| DbError::driver(e.to_string()))?;
        }
        if self.opts.wal && matches!(self.opts.location, Location::File(_)) {
            conn.execute_batch("PRAGMA journal_mode = WAL;")
                .map_err(|e| DbError::driver(e.to_string()))?;
        }
        if let Some(init) = &self.init {
            init(&conn);
        }
        Ok(RusqliteConn { conn })
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            durable: matches!(self.opts.location, Location::File(_)),
            encryption: cfg!(feature = "cipher"),
            wal: true,
            // The bundled builds compile FTS5/R*Tree in; a system libsqlite3 may or may not,
            // and claiming what we have not verified would be a lie the schema layer acts on.
            full_text_search: cfg!(any(feature = "bundled", feature = "cipher")),
            rtree: cfg!(any(feature = "bundled", feature = "cipher")),
            external_changes: false,
        }
    }
}

/// The `day-db/` sibling of day-part-fs' root — same resolution order, own leaf, so the
/// database never collides with other day state in the directory.
fn app_data_root() -> Result<std::path::PathBuf, DbError> {
    use std::path::PathBuf;
    let no_dir = || {
        DbError::new(
            DbErrorKind::Unsupported,
            "no per-app data directory on this target",
        )
    };
    if let Some(dir) = std::env::var_os("DAY_DATA_DIR")
        && !dir.is_empty()
    {
        return Ok(PathBuf::from(dir).join("day-db"));
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        std::env::var_os("HOME")
            .map(|home| {
                PathBuf::from(home)
                    .join("Library/Application Support/day")
                    .join("day-db")
            })
            .ok_or_else(no_dir)
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(|app| PathBuf::from(app).join("day").join("day-db"))
            .ok_or_else(no_dir)
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows")))]
    {
        if let Some(dir) = std::env::var_os("XDG_DATA_HOME")
            && !dir.is_empty()
        {
            return Ok(PathBuf::from(dir).join("day").join("day-db"));
        }
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(".local/share/day").join("day-db"))
            .ok_or_else(no_dir)
    }
}

pub struct RusqliteConn {
    conn: Connection,
}

fn to_sql(v: &Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as R;
    match v {
        Value::Null => R::Null,
        Value::Int(i) => R::Integer(*i),
        Value::Real(f) => R::Real(*f),
        Value::Text(t) => R::Text(t.clone()),
        Value::Blob(b) => R::Blob(b.clone()),
    }
}

fn from_sql(v: rusqlite::types::ValueRef<'_>) -> Value {
    use rusqlite::types::ValueRef as R;
    match v {
        R::Null => Value::Null,
        R::Integer(i) => Value::Int(i),
        R::Real(f) => Value::Real(f),
        R::Text(t) => Value::Text(String::from_utf8_lossy(t).into_owned()),
        R::Blob(b) => Value::Blob(b.to_vec()),
    }
}

impl SqliteConnection for RusqliteConn {
    fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, DbError> {
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| DbError::driver(format!("{e} in `{sql}`")))?;
        let bound = rusqlite::params_from_iter(params.iter().map(to_sql));
        stmt.execute(bound)
            .map(|n| n as u64)
            .map_err(|e| DbError::driver(format!("{e} in `{sql}`")))
    }

    fn query(
        &mut self,
        sql: &str,
        params: &[Value],
        row: &mut dyn FnMut(&dyn Row),
    ) -> Result<(), DbError> {
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| DbError::driver(format!("{e} in `{sql}`")))?;
        let bound = rusqlite::params_from_iter(params.iter().map(to_sql));
        let mut rows = stmt
            .query(bound)
            .map_err(|e| DbError::driver(format!("{e} in `{sql}`")))?;
        loop {
            match rows.next() {
                Ok(Some(r)) => {
                    let n = r.as_ref().column_count();
                    let vals: Vec<Value> = (0..n)
                        .map(|i| r.get_ref(i).map(from_sql).unwrap_or(Value::Null))
                        .collect();
                    row(&vals);
                }
                Ok(None) => break,
                Err(e) => return Err(DbError::driver(e.to_string())),
            }
        }
        Ok(())
    }
}
