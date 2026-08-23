// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The built-in driver. On native targets the engine is rusqlite, chosen by cargo feature —
//! `bundled` (the default), `system` (link the OS's libsqlite3), or `cipher` (SQLCipher with
//! vendored crypto). On web-dom the engine is day-sqlite-worker: `:memory:` databases run
//! in-process, and file databases are proxied synchronously to the day-sql worker, which
//! holds them on real OPFS (see the web module at the end of this file). One driver type,
//! recompiled; not a crate per engine.

use std::rc::Rc;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
use rusqlite::Connection;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
type InitHook = Rc<dyn Fn(&Connection)>;

use crate::{
    Capabilities, DbError, DbErrorKind, Location, OpenOptions, Row, Secret, SqliteConnection,
    SqliteDriver, Value,
};

/// The built-in SQLite driver. `Sqlite::at(path)` or `Sqlite::memory()`, then builder options.
/// A statement-trace sink (`Sqlite::trace_sql`).
type TraceFn = Rc<dyn Fn(&str)>;

pub struct Sqlite {
    opts: OpenOptions,
    /// Runs against the raw rusqlite connection right after open — register a custom SQL
    /// function, load an extension, set a PRAGMA. The framework neither knows nor cares.
    /// Native only: the web engine lives in the day-sql worker, out of closure reach.
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    init: Option<InitHook>,
    /// The statement-trace sink (`trace_sql`).
    trace: Option<TraceFn>,
}

impl Sqlite {
    fn from_opts(opts: OpenOptions) -> Self {
        Sqlite {
            opts,
            #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
            init: None,
            trace: None,
        }
    }
    pub fn at(path: impl Into<std::path::PathBuf>) -> Self {
        Sqlite::from_opts(OpenOptions::new(Location::File(path.into())))
    }
    pub fn memory() -> Self {
        Sqlite::from_opts(OpenOptions::new(Location::Memory))
    }
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
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
        let dir = Self::app_data_dir()?;
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
    /// The directory [`Sqlite::app_data`] resolves into — for apps that manage their own
    /// document files there (numbering "Drawing 2", importing a picked file). Created lazily
    /// by `app_data`; this accessor only resolves it.
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    pub fn app_data_dir() -> Result<std::path::PathBuf, DbError> {
        app_data_root()
    }

    /// A per-connection hook over the raw rusqlite connection — the escape valve for loadable
    /// extensions, custom functions, and PRAGMAs this crate does not model. Native only.
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    pub fn with_init(mut self, f: impl Fn(&Connection) + 'static) -> Self {
        self.init = Some(Rc::new(f));
        self
    }

    /// Per-statement SQL tracing through the engine's own facility (`sqlite3_trace_v2` with
    /// `SQLITE_TRACE_STMT`): `f` sees every statement this connection executes — migrations,
    /// autosave flushes, live-query `SELECT`s, maintenance — with bound parameters expanded by
    /// the engine itself. Wire it to a logger in debug builds:
    ///
    /// ```ignore
    /// let driver = Sqlite::app_data("trips.db")?;
    /// let driver = if cfg!(debug_assertions) {
    ///     driver.trace_sql(|sql| log::trace!("sql: {sql}"))
    /// } else {
    ///     driver
    /// };
    /// ```
    ///
    /// The trace installs after any `PRAGMA key`, so a cipher key never reaches the sink. On
    /// web-dom a FILE database's engine runs in the day-sql worker, out of closure reach:
    /// there the statements log to the browser console (`[day-sql]` lines in devtools) and
    /// `f` is not called. `:memory:` databases call `f` on every target.
    pub fn trace_sql(mut self, f: impl Fn(&str) + 'static) -> Self {
        self.trace = Some(Rc::new(f));
        self
    }
}

impl SqliteDriver for Sqlite {
    type Connection = RusqliteConn;

    /// The proxy open: `:memory:` runs in-process (no file I/O exists in that engine shape);
    /// a file database opens inside the day-sql worker over the synchronous channel.
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    fn open(self) -> Result<RusqliteConn, DbError> {
        use day_sqlite_worker::protocol::{Reply as WReply, Req as WReq};
        if self.opts.key.is_some() {
            return Err(DbError::new(
                DbErrorKind::Unsupported,
                "this build has no encryption — the web engine does not include SQLCipher",
            ));
        }
        let conn = match &self.opts.location {
            Location::Memory => {
                let mut conn = day_sqlite_worker::Connection::open_memory()
                    .map_err(|e| DbError::driver(e.to_string()))?;
                if let Some(f) = &self.trace {
                    let f = f.clone();
                    conn.trace_stmt(Box::new(move |sql| f(sql)));
                }
                WebConn::Memory(conn)
            }
            Location::File(p) => {
                let name = p.to_string_lossy().into_owned();
                match web::channel::call(&WReq::Open {
                    name,
                    trace: self.trace.is_some(),
                })? {
                    WReply::Conn(id) => WebConn::Remote(id),
                    other => return Err(web::unexpected(other)),
                }
            }
        };
        let mut conn = RusqliteConn { conn };
        if self.opts.foreign_keys {
            conn.execute("PRAGMA foreign_keys = ON", &[])?;
        }
        Ok(conn)
    }

    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
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

        // Trace AFTER the key PRAGMA above (a cipher key must never reach the sink) and
        // before everything else, so the remaining setup PRAGMAs log too.
        let trace = self
            .trace
            .as_ref()
            .map(|f| install_sql_trace(&conn, f.clone()));

        conn.busy_timeout(std::time::Duration::from_millis(
            self.opts.busy_timeout_ms as u64,
        ))
        .map_err(|e| DbError::driver(e.to_string()))?;
        if self.opts.foreign_keys {
            conn.execute_batch("PRAGMA foreign_keys = ON;")
                .map_err(|e| DbError::driver(e.to_string()))?;
        }
        #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
        if self.opts.wal && matches!(self.opts.location, Location::File(_)) {
            conn.execute_batch("PRAGMA journal_mode = WAL;")
                .map_err(|e| DbError::driver(e.to_string()))?;
        }
        if let Some(init) = &self.init {
            init(&conn);
        }
        Ok(RusqliteConn {
            conn,
            _trace: trace,
        })
    }

    fn capabilities(&self) -> Capabilities {
        #[cfg(all(target_family = "wasm", target_os = "unknown"))]
        {
            Capabilities {
                // A file database is durable when the day-sql worker holds it on OPFS: the
                // worker fsyncs before each reply, so a commit that returned has landed. No
                // channel (a host serving without cross-origin isolation) = no file databases
                // at all — reported here, refused loudly at open.
                durable: matches!(self.opts.location, Location::File(_)) && web::channel::ready(),
                encryption: false,
                wal: false,
                // day-sqlite-worker pins FTS5 and R*Tree into its build.
                full_text_search: true,
                rtree: true,
                external_changes: false,
            }
        }
        #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
        {
            Capabilities {
                durable: matches!(self.opts.location, Location::File(_)),
                encryption: cfg!(feature = "cipher"),
                wal: true,
                // The bundled builds compile FTS5/R*Tree in; a system libsqlite3 may or may
                // not, and claiming what we have not verified would be a lie the schema layer
                // acts on.
                full_text_search: cfg!(any(feature = "bundled", feature = "cipher")),
                rtree: cfg!(any(feature = "bundled", feature = "cipher")),
                // `PRAGMA data_version` is core SQLite: another connection's committed writes
                // are detectable on any file database. A memory database has no second
                // connection to detect.
                external_changes: matches!(self.opts.location, Location::File(_)),
            }
        }
    }
}

/// The `day-db/` sibling of day-part-fs' root — same resolution order, own leaf, so the
/// database never collides with other day state in the directory.
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
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

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub struct RusqliteConn {
    conn: Connection,
    /// Keeps the installed trace closure alive as long as the engine may call it (the box's
    /// heap address is what `sqlite3_trace_v2` holds). Dropped after `conn` closes.
    _trace: Option<Box<TraceFn>>,
}

/// Install the engine's per-statement trace on a raw connection handle. rusqlite's own
/// `trace_v2` takes a plain fn pointer, so the closure rides the C-level context instead.
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
fn install_sql_trace(conn: &Connection, f: TraceFn) -> Box<TraceFn> {
    use rusqlite::ffi;
    unsafe extern "C" fn cb(
        ev: std::ffi::c_uint,
        ctx: *mut std::ffi::c_void,
        p: *mut std::ffi::c_void,
        x: *mut std::ffi::c_void,
    ) -> std::ffi::c_int {
        if ev != ffi::SQLITE_TRACE_STMT || ctx.is_null() {
            return 0;
        }
        // SAFETY: ctx is the connection's boxed trace closure (returned below and stored on
        // RusqliteConn), alive until after the connection closes; day's connections are
        // single-threaded, so the callback never races the closure.
        let f = unsafe { &*(ctx as *const TraceFn) };
        unsafe {
            let expanded = ffi::sqlite3_expanded_sql(p.cast());
            if !expanded.is_null() {
                if let Ok(s) = std::ffi::CStr::from_ptr(expanded).to_str() {
                    f(s);
                }
                ffi::sqlite3_free(expanded.cast());
            } else if !x.is_null() {
                // The engine could not expand (OOM, or a trigger frame): unexpanded text.
                if let Ok(s) = std::ffi::CStr::from_ptr(x.cast()).to_str() {
                    f(s);
                }
            }
        }
        0
    }
    let boxed = Box::new(f);
    let ctx = &*boxed as *const TraceFn as *mut std::ffi::c_void;
    // SAFETY: the handle is this connection's own; ctx points into `boxed`, whose heap slot
    // outlives the connection via RusqliteConn::_trace.
    unsafe {
        ffi::sqlite3_trace_v2(conn.handle(), ffi::SQLITE_TRACE_STMT, Some(cb), ctx);
    }
    boxed
}

/// The web connection: in-process for `:memory:`, a worker connection id for files.
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
enum WebConn {
    Memory(day_sqlite_worker::Connection),
    Remote(u32),
}

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub struct RusqliteConn {
    conn: WebConn,
}

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
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

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
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

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
impl SqliteConnection for RusqliteConn {
    fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, DbError> {
        use day_sqlite_worker::protocol::{Reply as WReply, Req as WReq};
        match &self.conn {
            WebConn::Memory(c) => c
                .execute(sql, &web::to_wire(params))
                .map_err(|e| DbError::driver(format!("{e} in `{sql}`"))),
            WebConn::Remote(id) => match web::channel::call(&WReq::Exec {
                conn: *id,
                sql: sql.to_string(),
                params: web::to_wire(params),
            })? {
                WReply::Changes(n) => Ok(n),
                other => Err(web::unexpected(other)),
            },
        }
    }

    fn query(
        &mut self,
        sql: &str,
        params: &[Value],
        row: &mut dyn FnMut(&dyn Row),
    ) -> Result<(), DbError> {
        use day_sqlite_worker::protocol::{Reply as WReply, Req as WReq};
        match &self.conn {
            WebConn::Memory(c) => c
                .query(sql, &web::to_wire(params), &mut |r| {
                    let vals: Vec<Value> = r.into_iter().map(web::from_wire).collect();
                    row(&vals);
                })
                .map_err(|e| DbError::driver(format!("{e} in `{sql}`"))),
            WebConn::Remote(id) => match web::channel::call(&WReq::Query {
                conn: *id,
                sql: sql.to_string(),
                params: web::to_wire(params),
            })? {
                WReply::Rows(rows) => {
                    for r in rows {
                        let vals: Vec<Value> = r.into_iter().map(web::from_wire).collect();
                        row(&vals);
                    }
                    Ok(())
                }
                other => Err(web::unexpected(other)),
            },
        }
    }
}

/// A dropped remote connection closes its worker side — best effort, like a file handle.
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
impl Drop for RusqliteConn {
    fn drop(&mut self) {
        if let WebConn::Remote(id) = self.conn {
            let _ = web::channel::call(&day_sqlite_worker::protocol::Req::Close { conn: id });
        }
    }
}

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
impl SqliteConnection for RusqliteConn {
    fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, DbError> {
        // Cached, not re-prepared: the fold emits RUNS of identical SQL (one INSERT shape per
        // table, one DELETE, one UPDATE per changed column set), so a bulk flush would
        // otherwise re-parse the same statement thousands of times.
        let mut stmt = self
            .conn
            .prepare_cached(sql)
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
            .prepare_cached(sql)
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

    fn execute_batch(&mut self, sql: &str) -> Result<(), DbError> {
        self.conn
            .execute_batch(sql)
            .map_err(|e| DbError::driver(format!("{e} in batch")))
    }

    fn query_named(
        &mut self,
        sql: &str,
        params: &[Value],
        row: &mut dyn FnMut(&[String], &dyn Row),
    ) -> Result<(), DbError> {
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| DbError::driver(format!("{e} in `{sql}`")))?;
        let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
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
                    row(&names, &vals);
                }
                Ok(None) => break,
                Err(e) => return Err(DbError::driver(e.to_string())),
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// The web leg (wasm32-unknown-unknown): the synchronous channel to the day-sql worker.
//
// OPFS's only random-access synchronous API (`createSyncAccessHandle`) exists solely in
// dedicated workers, so the engine runs in day-cli's day-sql worker — the same wasm module,
// instantiated there — and this side is a proxy. Each call encodes one request in
// day-sqlite-worker's protocol, hands it to the shim, and the main thread blocks the few
// microseconds until the worker's reply lands back through the SharedArrayBuffer. Fully
// synchronous from Rust's point of view, fully durable at commit, one code path with every
// other platform. The channel needs cross-origin isolation (COOP/COEP; `day launch` serves
// it — docs/web.md); without it File opens fail loudly and capabilities say durable: false.
// ---------------------------------------------------------------------------

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
mod web {
    use super::*;
    use day_sqlite_worker::protocol::{self, Reply as WReply, Req as WReq, Value as WValue};

    pub(super) mod channel {
        use super::*;

        #[link(wasm_import_module = "env")]
        unsafe extern "C" {
            /// 1 while the day-sql worker is up and the SharedArrayBuffer channel is live.
            fn day_dom_sql_ready() -> i32;
            /// Send one request; answers the reply's byte length, or -1 with no channel.
            fn day_dom_sql_call(req: *const u8, len: usize) -> f64;
            /// Copy the staged reply (`day_dom_sql_call`'s length) into `dst`.
            fn day_dom_sql_reply(dst: *mut u8);
        }

        pub(crate) fn ready() -> bool {
            unsafe { day_dom_sql_ready() == 1 }
        }

        /// One round trip. Worker-reported SQL errors come back as [`DbError`]s here, so
        /// callers only ever match the success variants.
        pub(crate) fn call(req: &WReq) -> Result<WReply, DbError> {
            let bytes = protocol::encode_req(req);
            let len = unsafe { day_dom_sql_call(bytes.as_ptr(), bytes.len()) };
            if len < 0.0 {
                return Err(DbError::new(
                    DbErrorKind::Unsupported,
                    "no SQL worker — file databases on the web need the day-sql worker, \
                     which needs cross-origin isolation (COOP/COEP headers; docs/web.md)",
                ));
            }
            let mut reply = vec![0u8; len as usize];
            unsafe { day_dom_sql_reply(reply.as_mut_ptr()) };
            match protocol::decode_reply(&reply) {
                Ok(WReply::Err(m)) => Err(DbError::driver(m)),
                Ok(r) => Ok(r),
                Err(_) => Err(DbError::driver("malformed reply from the SQL worker")),
            }
        }
    }

    pub(super) fn unexpected(r: WReply) -> DbError {
        DbError::driver(format!("unexpected SQL worker reply: {r:?}"))
    }

    pub(super) fn to_wire(params: &[Value]) -> Vec<WValue> {
        params
            .iter()
            .map(|v| match v {
                Value::Null => WValue::Null,
                Value::Int(i) => WValue::Int(*i),
                Value::Real(r) => WValue::Real(*r),
                Value::Text(t) => WValue::Text(t.clone()),
                Value::Blob(b) => WValue::Blob(b.clone()),
            })
            .collect()
    }

    pub(super) fn from_wire(v: WValue) -> Value {
        match v {
            WValue::Null => Value::Null,
            WValue::Int(i) => Value::Int(i),
            WValue::Real(r) => Value::Real(r),
            WValue::Text(t) => Value::Text(t),
            WValue::Blob(b) => Value::Blob(b),
        }
    }

    /// The origin's database pool — names in, bytes out, OPFS underneath. The document
    /// surface a file-per-document app needs on the web: numbering a fresh drawing,
    /// importing an Open… pick, exporting a download.
    #[derive(Clone)]
    pub struct WebStorage;

    impl WebStorage {
        pub fn exists(&self, name: &str) -> bool {
            matches!(
                channel::call(&WReq::Exists {
                    name: name.to_string()
                }),
                Ok(WReply::Bool(true))
            )
        }

        /// Every database name in the pool.
        pub fn list(&self) -> Vec<String> {
            match channel::call(&WReq::List) {
                Ok(WReply::Names(names)) => names,
                _ => Vec::new(),
            }
        }

        /// The database's bytes — a plain SQLite file image, downloadable as a file. Flush
        /// (`ModelContainer::save`) first for an image that includes this turn's edits.
        pub fn export_db(&self, name: &str) -> Result<Vec<u8>, DbError> {
            match channel::call(&WReq::Export {
                name: name.to_string(),
            })? {
                WReply::Bytes(b) => Ok(b),
                other => Err(unexpected(other)),
            }
        }

        /// Write a SQLite file image under `name` (an Open… flow's landing). A connection
        /// still open on that name is stale afterwards — close (drop) it before importing.
        pub fn import_db(&self, name: &str, bytes: &[u8]) -> Result<(), DbError> {
            match channel::call(&WReq::Import {
                name: name.to_string(),
                bytes: bytes.to_vec(),
            })? {
                WReply::Ok => Ok(()),
                other => Err(unexpected(other)),
            }
        }

        pub fn delete_db(&self, name: &str) {
            let _ = channel::call(&WReq::Delete {
                name: name.to_string(),
            });
        }
    }

    impl Sqlite {
        /// The pool surface, when the worker channel is up.
        pub fn web_storage() -> Result<WebStorage, DbError> {
            if channel::ready() {
                Ok(WebStorage)
            } else {
                Err(DbError::new(
                    DbErrorKind::Unsupported,
                    "no SQL worker — file databases on the web need cross-origin isolation \
                     (COOP/COEP headers; docs/web.md)",
                ))
            }
        }
    }
}

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub use web::WebStorage;
