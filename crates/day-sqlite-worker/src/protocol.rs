// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The statement protocol the main thread speaks to the SQL worker (docs/persistence.md).
//! Both sides link this module — the day-persistence proxy driver encodes requests and
//! decodes replies; the worker loop does the reverse — so the wire format cannot drift.
//! Everything is little-endian, length-prefixed, and version-tagged; the JS shuttle between
//! them moves opaque bytes and never parses them (oversized replies are chunked JS-side,
//! invisible here).

/// First byte of every request; bumped only if the format ever changes shape.
pub const VERSION: u8 = 1;

/// A SQL value on the wire — the same five kinds SQLite itself has.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// One request from the main thread. Connection ids are worker-assigned.
#[derive(Clone, Debug, PartialEq)]
pub enum Req {
    /// Open (creating if absent) the named database file; answers `Reply::Conn`. `trace`
    /// installs the engine's statement trace on the connection, logged to the worker page's
    /// console (`sqlite3_trace_v2`; the driver's `trace_sql` sets it).
    Open {
        name: String,
        trace: bool,
    },
    Close {
        conn: u32,
    },
    /// Run semicolon-separated statements, no parameters, no rows; answers `Reply::Ok`.
    Batch {
        conn: u32,
        sql: String,
    },
    /// Run one statement with parameters; answers `Reply::Changes`.
    Exec {
        conn: u32,
        sql: String,
        params: Vec<Value>,
    },
    /// Run one statement with parameters; answers `Reply::Rows`.
    Query {
        conn: u32,
        sql: String,
        params: Vec<Value>,
    },
    /// Storage-pool verbs, by database name (the file-per-document surface).
    Exists {
        name: String,
    },
    List,
    Delete {
        name: String,
    },
    /// The database's bytes as a plain SQLite file image; answers `Reply::Bytes`.
    Export {
        name: String,
    },
    /// Write a SQLite file image under `name` (replacing any prior content).
    Import {
        name: String,
        bytes: Vec<u8>,
    },
}

/// The worker's answer. `Err` carries the SQLite (or pool) message.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Ok,
    Conn(u32),
    Changes(u64),
    Rows(Vec<Vec<Value>>),
    Bool(bool),
    Names(Vec<String>),
    Bytes(Vec<u8>),
    Err(String),
}

// --- encoding ------------------------------------------------------------------------------

fn put_str(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
}

fn put_bytes(buf: &mut Vec<u8>, b: &[u8]) {
    buf.extend_from_slice(&(b.len() as u32).to_le_bytes());
    buf.extend_from_slice(b);
}

fn put_value(buf: &mut Vec<u8>, v: &Value) {
    match v {
        Value::Null => buf.push(0),
        Value::Int(i) => {
            buf.push(1);
            buf.extend_from_slice(&i.to_le_bytes());
        }
        Value::Real(r) => {
            buf.push(2);
            buf.extend_from_slice(&r.to_le_bytes());
        }
        Value::Text(t) => {
            buf.push(3);
            put_str(buf, t);
        }
        Value::Blob(b) => {
            buf.push(4);
            put_bytes(buf, b);
        }
    }
}

fn put_values(buf: &mut Vec<u8>, vs: &[Value]) {
    buf.extend_from_slice(&(vs.len() as u32).to_le_bytes());
    for v in vs {
        put_value(buf, v);
    }
}

pub fn encode_req(req: &Req) -> Vec<u8> {
    let mut buf = vec![VERSION];
    match req {
        Req::Open { name, trace } => {
            buf.push(1);
            put_str(&mut buf, name);
            buf.push(u8::from(*trace));
        }
        Req::Close { conn } => {
            buf.push(2);
            buf.extend_from_slice(&conn.to_le_bytes());
        }
        Req::Batch { conn, sql } => {
            buf.push(3);
            buf.extend_from_slice(&conn.to_le_bytes());
            put_str(&mut buf, sql);
        }
        Req::Exec { conn, sql, params } => {
            buf.push(4);
            buf.extend_from_slice(&conn.to_le_bytes());
            put_str(&mut buf, sql);
            put_values(&mut buf, params);
        }
        Req::Query { conn, sql, params } => {
            buf.push(5);
            buf.extend_from_slice(&conn.to_le_bytes());
            put_str(&mut buf, sql);
            put_values(&mut buf, params);
        }
        Req::Exists { name } => {
            buf.push(6);
            put_str(&mut buf, name);
        }
        Req::List => buf.push(7),
        Req::Delete { name } => {
            buf.push(8);
            put_str(&mut buf, name);
        }
        Req::Export { name } => {
            buf.push(9);
            put_str(&mut buf, name);
        }
        Req::Import { name, bytes } => {
            buf.push(10);
            put_str(&mut buf, name);
            put_bytes(&mut buf, bytes);
        }
    }
    buf
}

pub fn encode_reply(reply: &Reply) -> Vec<u8> {
    let mut buf = Vec::new();
    match reply {
        Reply::Ok => buf.push(0),
        Reply::Conn(id) => {
            buf.push(1);
            buf.extend_from_slice(&id.to_le_bytes());
        }
        Reply::Changes(n) => {
            buf.push(2);
            buf.extend_from_slice(&n.to_le_bytes());
        }
        Reply::Rows(rows) => {
            buf.push(3);
            buf.extend_from_slice(&(rows.len() as u32).to_le_bytes());
            for row in rows {
                put_values(&mut buf, row);
            }
        }
        Reply::Bool(b) => {
            buf.push(4);
            buf.push(u8::from(*b));
        }
        Reply::Names(names) => {
            buf.push(5);
            buf.extend_from_slice(&(names.len() as u32).to_le_bytes());
            for n in names {
                put_str(&mut buf, n);
            }
        }
        Reply::Bytes(b) => {
            buf.push(6);
            put_bytes(&mut buf, b);
        }
        Reply::Err(msg) => {
            buf.push(255);
            put_str(&mut buf, msg);
        }
    }
    buf
}

// --- decoding ------------------------------------------------------------------------------

/// A truncated or malformed buffer. Both sides treat it as a protocol bug, not user error.
#[derive(Clone, Debug, PartialEq)]
pub struct WireError;

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self.pos.checked_add(n).ok_or(WireError)?;
        if end > self.buf.len() {
            return Err(WireError);
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, WireError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn f64(&mut self) -> Result<f64, WireError> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn str(&mut self) -> Result<String, WireError> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| WireError)
    }
    fn bytes(&mut self) -> Result<Vec<u8>, WireError> {
        let len = self.u32()? as usize;
        Ok(self.take(len)?.to_vec())
    }
    fn value(&mut self) -> Result<Value, WireError> {
        Ok(match self.u8()? {
            0 => Value::Null,
            1 => Value::Int(self.i64()?),
            2 => Value::Real(self.f64()?),
            3 => Value::Text(self.str()?),
            4 => Value::Blob(self.bytes()?),
            _ => return Err(WireError),
        })
    }
    fn values(&mut self) -> Result<Vec<Value>, WireError> {
        let n = self.u32()? as usize;
        let mut vs = Vec::with_capacity(n.min(1024));
        for _ in 0..n {
            vs.push(self.value()?);
        }
        Ok(vs)
    }
    fn done(&self) -> Result<(), WireError> {
        if self.pos == self.buf.len() {
            Ok(())
        } else {
            Err(WireError)
        }
    }
}

pub fn decode_req(buf: &[u8]) -> Result<Req, WireError> {
    let mut r = Reader { buf, pos: 0 };
    if r.u8()? != VERSION {
        return Err(WireError);
    }
    let req = match r.u8()? {
        1 => Req::Open {
            name: r.str()?,
            trace: r.u8()? != 0,
        },
        2 => Req::Close { conn: r.u32()? },
        3 => Req::Batch {
            conn: r.u32()?,
            sql: r.str()?,
        },
        4 => Req::Exec {
            conn: r.u32()?,
            sql: r.str()?,
            params: r.values()?,
        },
        5 => Req::Query {
            conn: r.u32()?,
            sql: r.str()?,
            params: r.values()?,
        },
        6 => Req::Exists { name: r.str()? },
        7 => Req::List,
        8 => Req::Delete { name: r.str()? },
        9 => Req::Export { name: r.str()? },
        10 => Req::Import {
            name: r.str()?,
            bytes: r.bytes()?,
        },
        _ => return Err(WireError),
    };
    r.done()?;
    Ok(req)
}

pub fn decode_reply(buf: &[u8]) -> Result<Reply, WireError> {
    let mut r = Reader { buf, pos: 0 };
    let reply = match r.u8()? {
        0 => Reply::Ok,
        1 => Reply::Conn(r.u32()?),
        2 => Reply::Changes(r.u64()?),
        3 => {
            let n = r.u32()? as usize;
            let mut rows = Vec::with_capacity(n.min(1024));
            for _ in 0..n {
                rows.push(r.values()?);
            }
            Reply::Rows(rows)
        }
        4 => Reply::Bool(r.u8()? != 0),
        5 => {
            let n = r.u32()? as usize;
            let mut names = Vec::with_capacity(n.min(1024));
            for _ in 0..n {
                names.push(r.str()?);
            }
            Reply::Names(names)
        }
        6 => Reply::Bytes(r.bytes()?),
        255 => Reply::Err(r.str()?),
        _ => return Err(WireError),
    };
    r.done()?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_req(req: Req) {
        assert_eq!(decode_req(&encode_req(&req)), Ok(req));
    }

    fn round_trip_reply(reply: Reply) {
        assert_eq!(decode_reply(&encode_reply(&reply)), Ok(reply));
    }

    #[test]
    fn every_request_round_trips() {
        round_trip_req(Req::Open {
            name: "Drawing 1.daysketch".into(),
            trace: false,
        });
        round_trip_req(Req::Open {
            name: "traced.db".into(),
            trace: true,
        });
        round_trip_req(Req::Close { conn: 7 });
        round_trip_req(Req::Batch {
            conn: 1,
            sql: "BEGIN; COMMIT;".into(),
        });
        round_trip_req(Req::Exec {
            conn: 2,
            sql: "INSERT INTO t VALUES (?, ?, ?, ?, ?)".into(),
            params: vec![
                Value::Null,
                Value::Int(-42),
                Value::Real(2.5),
                Value::Text("héllo\u{1f}".into()),
                Value::Blob(vec![0, 255, 1]),
            ],
        });
        round_trip_req(Req::Query {
            conn: 3,
            sql: "SELECT 1".into(),
            params: vec![],
        });
        round_trip_req(Req::Exists { name: "a".into() });
        round_trip_req(Req::List);
        round_trip_req(Req::Delete { name: "a".into() });
        round_trip_req(Req::Export { name: "a".into() });
        round_trip_req(Req::Import {
            name: "a".into(),
            bytes: vec![1, 2, 3],
        });
    }

    #[test]
    fn every_reply_round_trips() {
        round_trip_reply(Reply::Ok);
        round_trip_reply(Reply::Conn(9));
        round_trip_reply(Reply::Changes(u64::MAX));
        round_trip_reply(Reply::Rows(vec![
            vec![Value::Int(1), Value::Text("a".into())],
            vec![Value::Null, Value::Real(f64::MIN)],
            vec![],
        ]));
        round_trip_reply(Reply::Bool(true));
        round_trip_reply(Reply::Names(vec!["a.db".into(), "b.db".into()]));
        round_trip_reply(Reply::Bytes(vec![0; 100_000]));
        round_trip_reply(Reply::Err("no such table: t".into()));
    }

    #[test]
    fn truncated_and_alien_buffers_are_errors_not_panics() {
        let good = encode_req(&Req::Exec {
            conn: 1,
            sql: "SELECT ?".into(),
            params: vec![Value::Text("x".into())],
        });
        for cut in 0..good.len() {
            assert_eq!(decode_req(&good[..cut]), Err(WireError), "cut at {cut}");
        }
        assert_eq!(decode_req(&[]), Err(WireError));
        assert_eq!(decode_req(&[VERSION, 99]), Err(WireError));
        assert_eq!(decode_req(&[9, 1]), Err(WireError), "wrong version");
        // Trailing garbage is rejected too — a length desync must not pass silently.
        let mut padded = good.clone();
        padded.push(0);
        assert_eq!(decode_req(&padded), Err(WireError));
        assert_eq!(decode_reply(&[77]), Err(WireError));
        // A length prefix pointing past the buffer must not allocate or panic.
        let mut lying = vec![VERSION, 1];
        lying.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode_req(&lying), Err(WireError));
    }
}
