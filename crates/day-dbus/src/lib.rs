// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-dbus: a std-only client for the freedesktop session bus.
//!
//! The crate speaks the D-Bus wire protocol directly: little-endian marshaling (and reading of
//! both byte orders), SASL `EXTERNAL` authentication over a unix socket, `Hello`, and one
//! background reader thread per [`Connection`] that routes replies to blocked callers, signals to
//! subscribers and method calls to exported objects. There is no async runtime, no libdbus and no
//! zbus; everything is `std`.
//!
//! On top of the connection sit two small servers the Linux backends share:
//!
//! - [`sni`]: a `StatusNotifierItem` with its `com.canonical.dbusmenu` menu, the tray icon that
//!   GNOME (through the AppIndicator extension), KDE, and most panels show.
//! - [`launcher`]: the Unity `LauncherEntry` signal that docks read for a badge count, progress,
//!   and urgency.
//!
//! Only unix transports exist (`unix:path=` and, on Linux, `unix:abstract=`). Elsewhere the crate
//! still compiles, and [`Connection::session`] returns [`Error::Unsupported`].
//!
//! # Threads
//!
//! Exported handlers and signal callbacks run on the connection's reader thread. A blocking
//! [`Connection::call`] made from that thread would wait for a reply only that thread can read,
//! so it returns [`Error::ReaderThread`] instead; use [`Connection::call_no_reply`] or hand the
//! work to another thread.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::thread::{self, ThreadId};
use std::time::Duration;

pub mod sni;

/// How long [`Connection::call`] waits for a reply.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);

/// The bus daemon's own name, object path and interface.
pub const BUS_NAME: &str = "org.freedesktop.DBus";
/// The bus daemon's object path.
pub const BUS_PATH: &str = "/org/freedesktop/DBus";
/// The bus daemon's interface (`Hello`, `RequestName`, `AddMatch`, …).
pub const BUS_INTERFACE: &str = "org.freedesktop.DBus";
/// The standard properties interface (`Get`, `GetAll`, `Set`).
pub const PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";
/// The standard introspection interface (`Introspect`).
pub const INTROSPECTABLE_INTERFACE: &str = "org.freedesktop.DBus.Introspectable";
/// The standard peer interface (`Ping`, `GetMachineId`), answered by every connection.
pub const PEER_INTERFACE: &str = "org.freedesktop.DBus.Peer";

/// `RequestName` flag: let another connection take the name over.
pub const NAME_FLAG_ALLOW_REPLACEMENT: u32 = 0x1;
/// `RequestName` flag: take the name over from an owner that allows replacement.
pub const NAME_FLAG_REPLACE_EXISTING: u32 = 0x2;
/// `RequestName` flag: fail instead of waiting in the queue for the name.
pub const NAME_FLAG_DO_NOT_QUEUE: u32 = 0x4;
/// `RequestName` reply: the caller now owns the name.
pub const NAME_REPLY_PRIMARY_OWNER: u32 = 1;
/// `RequestName` reply: the caller waits in the queue for the name.
pub const NAME_REPLY_IN_QUEUE: u32 = 2;
/// `RequestName` reply: someone else owns the name and the caller did not queue.
pub const NAME_REPLY_EXISTS: u32 = 3;
/// `RequestName` reply: the caller already owned the name.
pub const NAME_REPLY_ALREADY_OWNER: u32 = 4;

/// Message flag: the sender does not want a reply.
pub const FLAG_NO_REPLY_EXPECTED: u8 = 0x1;
/// Message flag: the bus must not start a service to deliver the message.
pub const FLAG_NO_AUTO_START: u8 = 0x2;

/// The spec's limit on a whole message.
const MAX_MESSAGE: usize = 128 * 1024 * 1024;
/// The spec's limit on one array's payload.
const MAX_ARRAY: usize = 64 * 1024 * 1024;
/// Nesting limit for values and signatures (the spec allows 32 arrays plus 32 structs).
const MAX_DEPTH: u32 = 64;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// What can go wrong talking to the bus.
#[derive(Debug)]
pub enum Error {
    /// This platform has no D-Bus transport (it is not unix).
    Unsupported,
    /// No session bus address was found, or none of its transports could be used.
    NoBus(String),
    /// Reading or writing the socket failed.
    Io(io::Error),
    /// The bus refused authentication.
    Auth(String),
    /// A malformed message, signature, or name, ours or the peer's.
    Protocol(String),
    /// The connection is closed; no reply will come.
    Disconnected,
    /// No reply arrived within the timeout.
    Timeout,
    /// A blocking call was made from the reader thread (a handler or signal callback), where it
    /// could never receive its reply.
    ReaderThread,
    /// The peer answered with a D-Bus error.
    Remote {
        /// The error name, such as `org.freedesktop.DBus.Error.ServiceUnknown`.
        name: String,
        /// The error's message argument, or empty.
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported => f.write_str("D-Bus is not available on this platform"),
            Error::NoBus(why) => write!(f, "no session bus: {why}"),
            Error::Io(e) => write!(f, "D-Bus socket error: {e}"),
            Error::Auth(why) => write!(f, "D-Bus authentication failed: {why}"),
            Error::Protocol(why) => write!(f, "D-Bus protocol error: {why}"),
            Error::Disconnected => f.write_str("D-Bus connection closed"),
            Error::Timeout => f.write_str("D-Bus call timed out"),
            Error::ReaderThread => {
                f.write_str("blocking D-Bus call made from the connection's reader thread")
            }
            Error::Remote { name, message } if message.is_empty() => f.write_str(name),
            Error::Remote { name, message } => write!(f, "{name}: {message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

fn protocol(why: impl Into<String>) -> Error {
    Error::Protocol(why.into())
}

// ---------------------------------------------------------------------------
// Values and signatures
// ---------------------------------------------------------------------------

/// One D-Bus value. Containers carry enough type information to marshal even when empty: an
/// array names its element signature.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// `y`
    Byte(u8),
    /// `b`
    Bool(bool),
    /// `n`
    I16(i16),
    /// `q`
    U16(u16),
    /// `i`
    I32(i32),
    /// `u`
    U32(u32),
    /// `x`
    I64(i64),
    /// `t`
    U64(u64),
    /// `d`
    Double(f64),
    /// `s`
    Str(String),
    /// `o`
    ObjectPath(String),
    /// `g`
    Signature(String),
    /// `a…`: an array whose items all have the single complete type `elem`. A dictionary is an
    /// array of [`Value::DictEntry`] items with an `elem` such as `{sv}`.
    Array {
        /// The element signature, such as `s`, `(iiay)` or `{sv}`.
        elem: String,
        /// The items; each must have signature `elem`.
        items: Vec<Value>,
    },
    /// `(…)`: a struct of one or more fields.
    Struct(Vec<Value>),
    /// `{kv}`: a dictionary entry; only valid as an array item, with a basic-typed key.
    DictEntry(Box<Value>, Box<Value>),
    /// `v`: a value that carries its own signature.
    Variant(Box<Value>),
}

impl Value {
    /// A string value.
    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    /// An object path value.
    pub fn path(p: impl Into<String>) -> Value {
        Value::ObjectPath(p.into())
    }

    /// A variant wrapping `v`.
    pub fn variant(v: Value) -> Value {
        Value::Variant(Box::new(v))
    }

    /// An array with element signature `elem`.
    pub fn array(elem: impl Into<String>, items: Vec<Value>) -> Value {
        Value::Array {
            elem: elem.into(),
            items,
        }
    }

    /// A byte array (`ay`).
    pub fn bytes(bytes: &[u8]) -> Value {
        Value::array("y", bytes.iter().map(|b| Value::Byte(*b)).collect())
    }

    /// A string array (`as`).
    pub fn strings<S: AsRef<str>>(items: &[S]) -> Value {
        Value::array("s", items.iter().map(|s| Value::str(s.as_ref())).collect())
    }

    /// A dictionary `a{kv}` from key and value signatures and its entries.
    pub fn dict(
        key: &str,
        value: &str,
        entries: impl IntoIterator<Item = (Value, Value)>,
    ) -> Value {
        Value::array(
            format!("{{{key}{value}}}"),
            entries
                .into_iter()
                .map(|(k, v)| Value::DictEntry(Box::new(k), Box::new(v)))
                .collect(),
        )
    }

    /// The `a{sv}` property dictionary most interfaces use.
    pub fn props<K: Into<String>>(entries: impl IntoIterator<Item = (K, Value)>) -> Value {
        Value::dict(
            "s",
            "v",
            entries
                .into_iter()
                .map(|(k, v)| (Value::Str(k.into()), Value::variant(v))),
        )
    }

    /// This value's D-Bus signature, such as `s`, `a{sv}` or `(iiay)`.
    pub fn signature(&self) -> String {
        let mut s = String::new();
        self.write_signature(&mut s);
        s
    }

    fn write_signature(&self, out: &mut String) {
        match self {
            Value::Array { elem, .. } => {
                out.push('a');
                out.push_str(elem);
            }
            Value::Struct(fields) => {
                out.push('(');
                for f in fields {
                    f.write_signature(out);
                }
                out.push(')');
            }
            Value::DictEntry(k, v) => {
                out.push('{');
                k.write_signature(out);
                v.write_signature(out);
                out.push('}');
            }
            other => out.push(other.basic_code().unwrap_or('v')),
        }
    }

    /// The one-character code of a basic type or variant, `None` for a container.
    fn basic_code(&self) -> Option<char> {
        Some(match self {
            Value::Byte(_) => 'y',
            Value::Bool(_) => 'b',
            Value::I16(_) => 'n',
            Value::U16(_) => 'q',
            Value::I32(_) => 'i',
            Value::U32(_) => 'u',
            Value::I64(_) => 'x',
            Value::U64(_) => 't',
            Value::Double(_) => 'd',
            Value::Str(_) => 's',
            Value::ObjectPath(_) => 'o',
            Value::Signature(_) => 'g',
            Value::Variant(_) => 'v',
            Value::Array { .. } | Value::Struct(_) | Value::DictEntry(..) => return None,
        })
    }

    /// Whether this value has the single complete type `sig`, without allocating.
    fn conforms(&self, sig: &str) -> bool {
        match self {
            Value::Array { elem, .. } => sig.strip_prefix('a') == Some(elem.as_str()),
            Value::Struct(fields) => {
                let Some(mut inner) = sig.strip_prefix('(').and_then(|s| s.strip_suffix(')'))
                else {
                    return false;
                };
                for f in fields {
                    match split_first_type(inner) {
                        Ok((first, rest)) if f.conforms(first) => inner = rest,
                        _ => return false,
                    }
                }
                inner.is_empty() && !fields.is_empty()
            }
            Value::DictEntry(k, v) => {
                let Some(inner) = sig.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
                    return false;
                };
                match split_first_type(inner) {
                    Ok((first, rest)) => k.conforms(first) && v.conforms(rest),
                    Err(_) => false,
                }
            }
            basic => {
                let mut chars = sig.chars();
                basic.basic_code() == chars.next() && chars.next().is_none()
            }
        }
    }

    /// The string inside a `Str`, `ObjectPath` or `Signature` (looking through variants).
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) | Value::ObjectPath(s) | Value::Signature(s) => Some(s),
            Value::Variant(v) => v.as_str(),
            _ => None,
        }
    }

    /// The boolean inside a `Bool` (looking through variants).
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            Value::Variant(v) => v.as_bool(),
            _ => None,
        }
    }

    /// Any integer value widened to `i64` (looking through variants); `None` for a `U64` above
    /// `i64::MAX` or a non-integer.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Byte(n) => Some(i64::from(*n)),
            Value::I16(n) => Some(i64::from(*n)),
            Value::U16(n) => Some(i64::from(*n)),
            Value::I32(n) => Some(i64::from(*n)),
            Value::U32(n) => Some(i64::from(*n)),
            Value::I64(n) => Some(*n),
            Value::U64(n) => i64::try_from(*n).ok(),
            Value::Variant(v) => v.as_i64(),
            _ => None,
        }
    }

    /// The number inside a `Double`, or any integer converted (looking through variants).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Double(d) => Some(*d),
            Value::Variant(v) => v.as_f64(),
            other => other.as_i64().map(|n| n as f64),
        }
    }

    /// The items of an array (looking through variants).
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array { items, .. } => Some(items),
            Value::Variant(v) => v.as_array(),
            _ => None,
        }
    }

    /// The fields of a struct (looking through variants).
    pub fn as_struct(&self) -> Option<&[Value]> {
        match self {
            Value::Struct(fields) => Some(fields),
            Value::Variant(v) => v.as_struct(),
            _ => None,
        }
    }

    /// The value inside a variant, or the value itself when it is not one.
    pub fn unwrap_variant(&self) -> &Value {
        match self {
            Value::Variant(v) => v.unwrap_variant(),
            other => other,
        }
    }

    /// Looks `key` up in a string-keyed dictionary such as `a{sv}`; the result is the entry's
    /// value with any variant unwrapped.
    pub fn dict_get(&self, key: &str) -> Option<&Value> {
        self.as_array()?.iter().find_map(|item| match item {
            Value::DictEntry(k, v) if k.as_str() == Some(key) => Some(v.unwrap_variant()),
            _ => None,
        })
    }
}

/// Splits `sig` into its first single complete type and the rest.
fn split_first_type(sig: &str) -> Result<(&str, &str), Error> {
    let len = complete_type_len(sig.as_bytes(), 0)?;
    Ok(sig.split_at(len))
}

fn complete_type_len(sig: &[u8], depth: u32) -> Result<usize, Error> {
    if depth > MAX_DEPTH {
        return Err(protocol("signature nests too deeply"));
    }
    match sig.first() {
        None => Err(protocol("empty signature")),
        Some(
            b'y' | b'b' | b'n' | b'q' | b'i' | b'u' | b'x' | b't' | b'd' | b's' | b'o' | b'g'
            | b'v',
        ) => Ok(1),
        Some(b'a') => Ok(1 + complete_type_len(&sig[1..], depth + 1)?),
        Some(b'(') => {
            let mut at = 1;
            while sig.get(at) != Some(&b')') {
                if at >= sig.len() {
                    return Err(protocol("unterminated struct in signature"));
                }
                at += complete_type_len(&sig[at..], depth + 1)?;
            }
            if at == 1 {
                return Err(protocol("empty struct in signature"));
            }
            Ok(at + 1)
        }
        Some(b'{') => {
            let key = sig.get(1).copied();
            if !matches!(
                key,
                Some(
                    b'y' | b'b'
                        | b'n'
                        | b'q'
                        | b'i'
                        | b'u'
                        | b'x'
                        | b't'
                        | b'd'
                        | b's'
                        | b'o'
                        | b'g'
                )
            ) {
                return Err(protocol("dict entry key must be a basic type"));
            }
            let value = complete_type_len(&sig[2..], depth + 1)?;
            if sig.get(2 + value) != Some(&b'}') {
                return Err(protocol("dict entry must hold exactly two types"));
            }
            Ok(value + 3)
        }
        Some(c) => Err(protocol(format!(
            "unsupported type code {:?} in signature",
            char::from(*c)
        ))),
    }
}

/// Checks a whole signature (zero or more complete types, at most 255 bytes).
fn validate_signature(sig: &str) -> Result<(), Error> {
    if sig.len() > 255 {
        return Err(protocol("signature longer than 255 bytes"));
    }
    let mut rest = sig;
    while !rest.is_empty() {
        rest = split_first_type(rest)?.1;
    }
    Ok(())
}

/// The marshaling alignment of a type, by its first signature character.
fn alignment(sig: &str) -> usize {
    match sig.as_bytes().first() {
        Some(b'n' | b'q') => 2,
        Some(b'b' | b'i' | b'u' | b's' | b'o' | b'a' | b'h') => 4,
        Some(b'x' | b't' | b'd' | b'(' | b'{') => 8,
        _ => 1,
    }
}

/// Whether `p` is a valid object path: `/`, or `/`-separated non-empty `[A-Za-z0-9_]` segments.
pub fn is_valid_object_path(p: &str) -> bool {
    if p == "/" {
        return true;
    }
    let Some(rest) = p.strip_prefix('/') else {
        return false;
    };
    rest.split('/')
        .all(|seg| !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
}

/// Whether `name` is a valid interface or error name: two or more dot-separated elements of
/// `[A-Za-z0-9_]`, none starting with a digit, at most 255 bytes.
fn is_valid_interface(name: &str) -> bool {
    name.len() <= 255
        && name.split('.').count() >= 2
        && name.split('.').all(|el| {
            !el.is_empty()
                && !el.starts_with(|c: char| c.is_ascii_digit())
                && el.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
}

/// Whether `name` is a valid member name.
fn is_valid_member(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Whether `name` is a plausible bus name (unique `:1.42` or well-known `org.example.App`).
fn is_valid_bus_name(name: &str) -> bool {
    let (unique, body) = match name.strip_prefix(':') {
        Some(rest) => (true, rest),
        None => (false, name),
    };
    name.len() <= 255
        && body.split('.').count() >= 2
        && body.split('.').all(|el| {
            !el.is_empty()
                && (unique || !el.starts_with(|c: char| c.is_ascii_digit()))
                && el
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}

// ---------------------------------------------------------------------------
// Marshaling
// ---------------------------------------------------------------------------

/// Writes values at offsets relative to the start of the buffer, which is the start of the
/// message (the body starts 8-aligned, so a body marshaled alone lines up the same way).
struct Encoder {
    buf: Vec<u8>,
    big: bool,
}

impl Encoder {
    fn new(big: bool) -> Self {
        Encoder {
            buf: Vec::new(),
            big,
        }
    }

    fn pad(&mut self, align: usize) {
        let padded = self.buf.len().next_multiple_of(align);
        self.buf.resize(padded, 0);
    }

    fn put(&mut self, le: &[u8], be: &[u8]) {
        self.pad(le.len());
        self.buf.extend_from_slice(if self.big { be } else { le });
    }

    fn u32(&mut self, v: u32) {
        self.put(&v.to_le_bytes(), &v.to_be_bytes());
    }

    fn string(&mut self, s: &str) -> Result<(), Error> {
        if s.contains('\0') {
            return Err(protocol("string contains a NUL byte"));
        }
        let len = u32::try_from(s.len()).map_err(|_| protocol("string too long"))?;
        self.u32(len);
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.push(0);
        Ok(())
    }

    fn signature(&mut self, s: &str) -> Result<(), Error> {
        validate_signature(s)?;
        // validate_signature bounds the length at 255.
        self.buf.push(s.len() as u8);
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.push(0);
        Ok(())
    }

    fn value(&mut self, v: &Value, depth: u32) -> Result<(), Error> {
        if depth > MAX_DEPTH {
            return Err(protocol("value nests too deeply"));
        }
        match v {
            Value::Byte(b) => self.buf.push(*b),
            Value::Bool(b) => self.u32(u32::from(*b)),
            Value::I16(n) => self.put(&n.to_le_bytes(), &n.to_be_bytes()),
            Value::U16(n) => self.put(&n.to_le_bytes(), &n.to_be_bytes()),
            Value::I32(n) => self.put(&n.to_le_bytes(), &n.to_be_bytes()),
            Value::U32(n) => self.u32(*n),
            Value::I64(n) => self.put(&n.to_le_bytes(), &n.to_be_bytes()),
            Value::U64(n) => self.put(&n.to_le_bytes(), &n.to_be_bytes()),
            Value::Double(d) => self.put(&d.to_le_bytes(), &d.to_be_bytes()),
            Value::Str(s) => self.string(s)?,
            Value::ObjectPath(p) => {
                if !is_valid_object_path(p) {
                    return Err(protocol(format!("invalid object path {p:?}")));
                }
                self.string(p)?;
            }
            Value::Signature(s) => self.signature(s)?,
            Value::Array { elem, items } => {
                let (_, rest) = split_first_type(elem)?;
                if !rest.is_empty() {
                    return Err(protocol(format!(
                        "array element signature {elem:?} is not a single complete type"
                    )));
                }
                self.pad(4);
                let len_at = self.buf.len();
                self.buf.extend_from_slice(&[0; 4]);
                // The padding before the first element is not part of the array's length, and
                // is present even when the array is empty.
                self.pad(alignment(elem));
                let start = self.buf.len();
                for item in items {
                    if !item.conforms(elem) {
                        return Err(protocol(format!(
                            "array item {} does not match element signature {elem}",
                            item.signature()
                        )));
                    }
                    self.value(item, depth + 1)?;
                }
                let len = self.buf.len() - start;
                if len > MAX_ARRAY {
                    return Err(protocol("array longer than 64 MiB"));
                }
                // MAX_ARRAY fits in u32.
                let len = len as u32;
                let bytes = if self.big {
                    len.to_be_bytes()
                } else {
                    len.to_le_bytes()
                };
                self.buf[len_at..len_at + 4].copy_from_slice(&bytes);
            }
            Value::Struct(fields) => {
                if fields.is_empty() {
                    return Err(protocol("a struct needs at least one field"));
                }
                self.pad(8);
                for f in fields {
                    self.value(f, depth + 1)?;
                }
            }
            Value::DictEntry(k, val) => {
                if matches!(
                    **k,
                    Value::Variant(_)
                        | Value::Array { .. }
                        | Value::Struct(_)
                        | Value::DictEntry(..)
                ) {
                    return Err(protocol("dict entry key must be a basic type"));
                }
                self.pad(8);
                self.value(k, depth + 1)?;
                self.value(val, depth + 1)?;
            }
            Value::Variant(inner) => {
                let sig = inner.signature();
                self.signature(&sig)?;
                self.value(inner, depth + 1)?;
            }
        }
        Ok(())
    }
}

/// Reads values from a whole message, honoring its byte order.
struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
    big: bool,
}

impl<'a> Decoder<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.data.len())
            .ok_or_else(|| protocol("message truncated"))?;
        let bytes = &self.data[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    fn align(&mut self, a: usize) -> Result<(), Error> {
        let to = self.pos.next_multiple_of(a);
        if to > self.data.len() {
            return Err(protocol("message truncated in padding"));
        }
        self.pos = to;
        Ok(())
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        self.align(N)?;
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn u32(&mut self) -> Result<u32, Error> {
        let b = self.fixed::<4>()?;
        Ok(if self.big {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        })
    }

    fn string(&mut self) -> Result<String, Error> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        if self.take(1)? != [0] || bytes.contains(&0) {
            return Err(protocol("string is not NUL-terminated"));
        }
        String::from_utf8(bytes.to_vec()).map_err(|_| protocol("string is not UTF-8"))
    }

    fn signature(&mut self) -> Result<String, Error> {
        let len = usize::from(self.take(1)?[0]);
        let bytes = self.take(len)?;
        if self.take(1)? != [0] {
            return Err(protocol("signature is not NUL-terminated"));
        }
        let sig = std::str::from_utf8(bytes).map_err(|_| protocol("signature is not ASCII"))?;
        validate_signature(sig)?;
        Ok(sig.to_owned())
    }

    /// Reads one value of the single complete type `sig`.
    fn value(&mut self, sig: &str, depth: u32) -> Result<Value, Error> {
        if depth > MAX_DEPTH {
            return Err(protocol("value nests too deeply"));
        }
        let big = self.big;
        macro_rules! num {
            ($t:ty, $n:literal, $variant:ident) => {{
                let b = self.fixed::<$n>()?;
                Value::$variant(if big {
                    <$t>::from_be_bytes(b)
                } else {
                    <$t>::from_le_bytes(b)
                })
            }};
        }
        Ok(match sig.as_bytes().first() {
            Some(b'y') => Value::Byte(self.take(1)?[0]),
            Some(b'b') => match self.u32()? {
                0 => Value::Bool(false),
                1 => Value::Bool(true),
                n => return Err(protocol(format!("boolean holds {n}"))),
            },
            Some(b'n') => num!(i16, 2, I16),
            Some(b'q') => num!(u16, 2, U16),
            Some(b'i') => num!(i32, 4, I32),
            Some(b'u') => num!(u32, 4, U32),
            Some(b'x') => num!(i64, 8, I64),
            Some(b't') => num!(u64, 8, U64),
            Some(b'd') => num!(f64, 8, Double),
            Some(b's') => Value::Str(self.string()?),
            Some(b'o') => {
                let p = self.string()?;
                if !is_valid_object_path(&p) {
                    return Err(protocol(format!("invalid object path {p:?}")));
                }
                Value::ObjectPath(p)
            }
            Some(b'g') => Value::Signature(self.signature()?),
            Some(b'v') => {
                let inner = self.signature()?;
                let (first, rest) = split_first_type(&inner)?;
                if !rest.is_empty() {
                    return Err(protocol("variant signature is not a single complete type"));
                }
                Value::Variant(Box::new(self.value(first, depth + 1)?))
            }
            Some(b'a') => {
                let elem = &sig[1..];
                let len = self.u32()? as usize;
                if len > MAX_ARRAY {
                    return Err(protocol("array longer than 64 MiB"));
                }
                self.align(alignment(elem))?;
                let end = self
                    .pos
                    .checked_add(len)
                    .filter(|end| *end <= self.data.len())
                    .ok_or_else(|| protocol("array runs past the message"))?;
                let mut items = Vec::new();
                while self.pos < end {
                    items.push(self.value(elem, depth + 1)?);
                }
                if self.pos != end {
                    return Err(protocol("array items overrun the array length"));
                }
                Value::Array {
                    elem: elem.to_owned(),
                    items,
                }
            }
            Some(b'(') => {
                self.align(8)?;
                let mut inner = &sig[1..sig.len() - 1];
                let mut fields = Vec::new();
                while !inner.is_empty() {
                    let (first, rest) = split_first_type(inner)?;
                    fields.push(self.value(first, depth + 1)?);
                    inner = rest;
                }
                Value::Struct(fields)
            }
            Some(b'{') => {
                self.align(8)?;
                let (key, rest) = split_first_type(&sig[1..sig.len() - 1])?;
                let k = self.value(key, depth + 1)?;
                let v = self.value(rest, depth + 1)?;
                Value::DictEntry(Box::new(k), Box::new(v))
            }
            _ => return Err(protocol(format!("cannot read type {sig:?}"))),
        })
    }
}

/// Marshals `values` as a little-endian body (offsets relative to an 8-aligned start).
pub fn marshal(values: &[Value]) -> Result<Vec<u8>, Error> {
    let mut e = Encoder::new(false);
    for v in values {
        e.value(v, 0)?;
    }
    Ok(e.buf)
}

/// Unmarshals a body with signature `signature` from `bytes` (little-endian unless
/// `big_endian`), requiring every byte to be consumed.
pub fn unmarshal(bytes: &[u8], signature: &str, big_endian: bool) -> Result<Vec<Value>, Error> {
    validate_signature(signature)?;
    let mut d = Decoder {
        data: bytes,
        pos: 0,
        big: big_endian,
    };
    let mut out = Vec::new();
    let mut rest = signature;
    while !rest.is_empty() {
        let (first, tail) = split_first_type(rest)?;
        out.push(d.value(first, 0)?);
        rest = tail;
    }
    if d.pos != bytes.len() {
        return Err(protocol("trailing bytes after the body"));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// The four kinds of D-Bus message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageType {
    /// A method call.
    MethodCall,
    /// A successful reply.
    MethodReturn,
    /// An error reply.
    Error,
    /// A signal.
    Signal,
}

impl MessageType {
    fn code(self) -> u8 {
        match self {
            MessageType::MethodCall => 1,
            MessageType::MethodReturn => 2,
            MessageType::Error => 3,
            MessageType::Signal => 4,
        }
    }
}

/// One D-Bus message: the header fields this crate understands plus the body. The body's
/// signature is the concatenation of its values' signatures.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    /// The message kind.
    pub message_type: MessageType,
    /// `FLAG_*` bits.
    pub flags: u8,
    /// The sender-assigned serial, never zero once sent.
    pub serial: u32,
    /// `PATH`: the object a call targets or a signal comes from.
    pub path: Option<String>,
    /// `INTERFACE`.
    pub interface: Option<String>,
    /// `MEMBER`: the method or signal name.
    pub member: Option<String>,
    /// `ERROR_NAME`, on error replies.
    pub error_name: Option<String>,
    /// `REPLY_SERIAL`, on replies.
    pub reply_serial: Option<u32>,
    /// `DESTINATION`.
    pub destination: Option<String>,
    /// `SENDER`, filled in by the bus.
    pub sender: Option<String>,
    /// The arguments.
    pub body: Vec<Value>,
}

impl Message {
    fn empty(message_type: MessageType) -> Message {
        Message {
            message_type,
            flags: 0,
            serial: 0,
            path: None,
            interface: None,
            member: None,
            error_name: None,
            reply_serial: None,
            destination: None,
            sender: None,
            body: Vec::new(),
        }
    }

    /// A method call to `member` of `interface` on object `path` of `destination`.
    pub fn method_call(
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        body: Vec<Value>,
    ) -> Message {
        Message {
            path: Some(path.to_owned()),
            interface: Some(interface.to_owned()),
            member: Some(member.to_owned()),
            destination: Some(destination.to_owned()),
            body,
            ..Message::empty(MessageType::MethodCall)
        }
    }

    /// A broadcast signal `member` of `interface` from object `path`.
    pub fn signal(path: &str, interface: &str, member: &str, body: Vec<Value>) -> Message {
        Message {
            path: Some(path.to_owned()),
            interface: Some(interface.to_owned()),
            member: Some(member.to_owned()),
            body,
            ..Message::empty(MessageType::Signal)
        }
    }

    fn reply_to(call: &Message, body: Vec<Value>) -> Message {
        Message {
            reply_serial: Some(call.serial),
            destination: call.sender.clone(),
            flags: FLAG_NO_REPLY_EXPECTED,
            body,
            ..Message::empty(MessageType::MethodReturn)
        }
    }

    fn error_to(call: &Message, name: &str, text: &str) -> Message {
        Message {
            reply_serial: Some(call.serial),
            destination: call.sender.clone(),
            error_name: Some(name.to_owned()),
            flags: FLAG_NO_REPLY_EXPECTED,
            body: vec![Value::str(text)],
            ..Message::empty(MessageType::Error)
        }
    }

    /// The body signature.
    pub fn signature(&self) -> String {
        let mut s = String::new();
        for v in &self.body {
            v.write_signature(&mut s);
        }
        s
    }

    /// Serializes the message in little-endian byte order.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        self.encode_with(false)
    }

    fn encode_with(&self, big: bool) -> Result<Vec<u8>, Error> {
        self.check_fields()?;
        let mut body = Encoder::new(big);
        for v in &self.body {
            body.value(v, 0)?;
        }
        let sig = self.signature();
        validate_signature(&sig)?;

        let field = |code: u8, v: Value| Value::Struct(vec![Value::Byte(code), Value::variant(v)]);
        let mut fields = Vec::new();
        if let Some(p) = &self.path {
            fields.push(field(1, Value::path(p)));
        }
        if let Some(s) = &self.interface {
            fields.push(field(2, Value::str(s)));
        }
        if let Some(s) = &self.member {
            fields.push(field(3, Value::str(s)));
        }
        if let Some(s) = &self.error_name {
            fields.push(field(4, Value::str(s)));
        }
        if let Some(n) = self.reply_serial {
            fields.push(field(5, Value::U32(n)));
        }
        if let Some(s) = &self.destination {
            fields.push(field(6, Value::str(s)));
        }
        if let Some(s) = &self.sender {
            fields.push(field(7, Value::str(s)));
        }
        if !sig.is_empty() {
            fields.push(field(8, Value::Signature(sig)));
        }

        let mut e = Encoder::new(big);
        e.buf.extend_from_slice(&[
            if big { b'B' } else { b'l' },
            self.message_type.code(),
            self.flags,
            1,
        ]);
        let body_len = u32::try_from(body.buf.len()).map_err(|_| protocol("body too long"))?;
        e.u32(body_len);
        e.u32(self.serial);
        e.value(&Value::array("(yv)", fields), 0)?;
        e.pad(8);
        e.buf.extend_from_slice(&body.buf);
        if e.buf.len() > MAX_MESSAGE {
            return Err(protocol("message longer than 128 MiB"));
        }
        Ok(e.buf)
    }

    fn check_fields(&self) -> Result<(), Error> {
        let missing = |what: &str| protocol(format!("{:?} without {what}", self.message_type));
        match self.message_type {
            MessageType::MethodCall => {
                if self.path.is_none() {
                    return Err(missing("a path"));
                }
                if self.member.is_none() {
                    return Err(missing("a member"));
                }
            }
            MessageType::Signal => {
                if self.path.is_none() || self.interface.is_none() || self.member.is_none() {
                    return Err(missing("a path, interface and member"));
                }
            }
            MessageType::Error => {
                if self.error_name.is_none() || self.reply_serial.is_none() {
                    return Err(missing("an error name and reply serial"));
                }
            }
            MessageType::MethodReturn => {
                if self.reply_serial.is_none() {
                    return Err(missing("a reply serial"));
                }
            }
        }
        if let Some(p) = &self.path
            && !is_valid_object_path(p)
        {
            return Err(protocol(format!("invalid object path {p:?}")));
        }
        if let Some(i) = &self.interface
            && !is_valid_interface(i)
        {
            return Err(protocol(format!("invalid interface name {i:?}")));
        }
        if let Some(e) = &self.error_name
            && !is_valid_interface(e)
        {
            return Err(protocol(format!("invalid error name {e:?}")));
        }
        if let Some(m) = &self.member
            && !is_valid_member(m)
        {
            return Err(protocol(format!("invalid member name {m:?}")));
        }
        if let Some(d) = &self.destination
            && !is_valid_bus_name(d)
        {
            return Err(protocol(format!("invalid bus name {d:?}")));
        }
        Ok(())
    }

    /// Parses one complete message of either byte order. `bytes` must hold exactly that one
    /// message, with no trailing data.
    pub fn decode(bytes: &[u8]) -> Result<Message, Error> {
        let big = match bytes.first() {
            Some(b'l') => false,
            Some(b'B') => true,
            _ => return Err(protocol("unknown byte order mark")),
        };
        let mut d = Decoder {
            data: bytes,
            pos: 0,
            big,
        };
        let head = d.take(4)?;
        let message_type = match head[1] {
            1 => MessageType::MethodCall,
            2 => MessageType::MethodReturn,
            3 => MessageType::Error,
            4 => MessageType::Signal,
            n => return Err(protocol(format!("unknown message type {n}"))),
        };
        if head[3] != 1 {
            return Err(protocol(format!(
                "unsupported protocol version {}",
                head[3]
            )));
        }
        let mut msg = Message::empty(message_type);
        msg.flags = head[2];
        let body_len = d.u32()? as usize;
        msg.serial = d.u32()?;
        let fields = d.value("a(yv)", 0)?;
        let mut signature = String::new();
        for field in fields.as_array().unwrap_or_default() {
            let Some([Value::Byte(code), Value::Variant(v)]) = field.as_struct() else {
                continue;
            };
            let text = || v.as_str().map(str::to_owned);
            match code {
                1 => msg.path = text(),
                2 => msg.interface = text(),
                3 => msg.member = text(),
                4 => msg.error_name = text(),
                5 => {
                    if let Value::U32(n) = **v {
                        msg.reply_serial = Some(n);
                    }
                }
                6 => msg.destination = text(),
                7 => msg.sender = text(),
                8 => signature = text().unwrap_or_default(),
                9 => {
                    if matches!(**v, Value::U32(n) if n > 0) {
                        return Err(protocol("unix fds were not negotiated"));
                    }
                }
                _ => {}
            }
        }
        d.align(8)?;
        if d.data.len() - d.pos != body_len {
            return Err(protocol("body length does not match the header"));
        }
        let mut rest = signature.as_str();
        while !rest.is_empty() {
            let (first, tail) = split_first_type(rest)?;
            msg.body.push(d.value(first, 0)?);
            rest = tail;
        }
        if d.pos != bytes.len() {
            return Err(protocol("body shorter than its length"));
        }
        Ok(msg)
    }
}

/// Reads one whole message's bytes from `r` using the fixed header's lengths.
fn read_frame(r: &mut dyn Read) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; 16];
    r.read_exact(&mut buf)?;
    let word = |at: usize| {
        let b = [buf[at], buf[at + 1], buf[at + 2], buf[at + 3]];
        match buf[0] {
            b'l' => Ok(u32::from_le_bytes(b) as usize),
            b'B' => Ok(u32::from_be_bytes(b) as usize),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unknown byte order mark",
            )),
        }
    };
    let body_len = word(4)?;
    let fields_len = word(12)?;
    if fields_len > MAX_MESSAGE || body_len > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message too long",
        ));
    }
    let total = (16 + fields_len).next_multiple_of(8) + body_len;
    if total > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message too long",
        ));
    }
    buf.resize(total, 0);
    r.read_exact(&mut buf[16..])?;
    Ok(buf)
}

// ---------------------------------------------------------------------------
// Authentication and Hello
// ---------------------------------------------------------------------------

fn read_line(s: &mut dyn Read) -> Result<String, Error> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while !line.ends_with(b"\r\n") {
        if line.len() > 1024 {
            return Err(Error::Auth("overlong reply line".into()));
        }
        if s.read(&mut byte)? == 0 {
            return Err(Error::Disconnected);
        }
        line.push(byte[0]);
    }
    line.truncate(line.len() - 2);
    String::from_utf8(line).map_err(|_| Error::Auth("reply is not text".into()))
}

/// The SASL `EXTERNAL` handshake. With a uid the client names itself; without one (where the uid
/// cannot be read without libc) it sends empty data and lets the server use the socket
/// credentials, which every bus implementation accepts.
#[cfg_attr(
    not(unix),
    allow(dead_code, reason = "only unix transports authenticate")
)]
fn authenticate<S: Read + Write>(s: &mut S, uid: Option<u32>) -> Result<(), Error> {
    s.write_all(b"\0")?;
    match uid {
        Some(uid) => {
            let hex: String = uid
                .to_string()
                .bytes()
                .map(|b| format!("{b:02x}"))
                .collect();
            s.write_all(format!("AUTH EXTERNAL {hex}\r\n").as_bytes())?;
        }
        None => s.write_all(b"AUTH EXTERNAL\r\n")?,
    }
    let mut reply = read_line(s)?;
    if reply == "DATA" || reply.starts_with("DATA ") {
        s.write_all(b"DATA\r\n")?;
        reply = read_line(s)?;
    }
    if !(reply == "OK" || reply.starts_with("OK ")) {
        let _ = s.write_all(b"CANCEL\r\n");
        return Err(Error::Auth(reply));
    }
    s.write_all(b"BEGIN\r\n")?;
    s.flush()?;
    Ok(())
}

/// The calling user's uid, read without libc where `/proc` offers it.
#[cfg_attr(
    not(unix),
    allow(dead_code, reason = "only unix transports authenticate")
)]
fn current_uid() -> Option<u32> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata("/proc/self").ok().map(|m| m.uid())
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        None
    }
}

// ---------------------------------------------------------------------------
// Connection
// ---------------------------------------------------------------------------

/// A method call delivered to an exported object.
#[derive(Clone, Debug)]
pub struct MethodCall {
    /// The connection the call arrived on, for emitting signals from a handler.
    pub connection: Connection,
    /// The object path called.
    pub path: String,
    /// The interface named by the caller, if any (it is optional in the protocol).
    pub interface: Option<String>,
    /// The method name.
    pub member: String,
    /// The arguments.
    pub args: Vec<Value>,
    /// The caller's unique bus name.
    pub sender: Option<String>,
}

/// A D-Bus error a handler answers with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodError {
    /// The error name, such as `org.freedesktop.DBus.Error.InvalidArgs`.
    pub name: String,
    /// A human-readable message.
    pub message: String,
}

impl MethodError {
    /// An error with any valid error name.
    pub fn new(name: impl Into<String>, message: impl Into<String>) -> Self {
        MethodError {
            name: name.into(),
            message: message.into(),
        }
    }

    /// `org.freedesktop.DBus.Error.UnknownMethod`.
    pub fn unknown_method(call: &MethodCall) -> Self {
        MethodError::new(
            "org.freedesktop.DBus.Error.UnknownMethod",
            format!(
                "no method {}.{} on {}",
                call.interface.as_deref().unwrap_or("*"),
                call.member,
                call.path
            ),
        )
    }

    /// `org.freedesktop.DBus.Error.InvalidArgs`.
    pub fn invalid_args(message: impl Into<String>) -> Self {
        MethodError::new("org.freedesktop.DBus.Error.InvalidArgs", message)
    }

    /// `org.freedesktop.DBus.Error.UnknownProperty`.
    pub fn unknown_property(name: &str) -> Self {
        MethodError::new(
            "org.freedesktop.DBus.Error.UnknownProperty",
            format!("no property {name}"),
        )
    }

    /// `org.freedesktop.DBus.Error.Failed`.
    pub fn failed(message: impl Into<String>) -> Self {
        MethodError::new("org.freedesktop.DBus.Error.Failed", message)
    }

    fn is_unknown_method(&self) -> bool {
        self.name == "org.freedesktop.DBus.Error.UnknownMethod"
    }
}

/// Which signals a callback registered with [`Connection::on_signal`] receives. Unset fields
/// match anything. This filters locally; the bus only sends signals a match rule asked for
/// ([`Connection::add_match`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignalFilter {
    /// The sender's bus name, compared literally (signals carry the unique name, except the
    /// bus's own, which carry `org.freedesktop.DBus`).
    pub sender: Option<String>,
    /// The emitting object path.
    pub path: Option<String>,
    /// The signal's interface.
    pub interface: Option<String>,
    /// The signal's name.
    pub member: Option<String>,
}

impl SignalFilter {
    /// A filter matching every signal.
    pub fn new() -> Self {
        Self::default()
    }

    /// Restricts to one sender.
    pub fn sender(mut self, sender: &str) -> Self {
        self.sender = Some(sender.to_owned());
        self
    }

    /// Restricts to one object path.
    pub fn path(mut self, path: &str) -> Self {
        self.path = Some(path.to_owned());
        self
    }

    /// Restricts to one interface.
    pub fn interface(mut self, interface: &str) -> Self {
        self.interface = Some(interface.to_owned());
        self
    }

    /// Restricts to one signal name.
    pub fn member(mut self, member: &str) -> Self {
        self.member = Some(member.to_owned());
        self
    }

    fn matches(&self, m: &Message) -> bool {
        let field = |want: &Option<String>, have: &Option<String>| {
            want.as_ref().is_none_or(|w| have.as_ref() == Some(w))
        };
        field(&self.sender, &m.sender)
            && field(&self.path, &m.path)
            && field(&self.interface, &m.interface)
            && field(&self.member, &m.member)
    }
}

/// Identifies a signal subscription for [`Connection::remove_signal_handler`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SubscriptionId(u64);

type HandlerFn = dyn Fn(&MethodCall) -> Result<Vec<Value>, MethodError> + Send + Sync;
type SignalFn = dyn Fn(&Connection, &Message) + Send + Sync;

struct State {
    closed: bool,
    pending: HashMap<u32, mpsc::Sender<Message>>,
    objects: HashMap<String, Arc<HandlerFn>>,
    signals: Vec<(SubscriptionId, SignalFilter, Arc<SignalFn>)>,
    next_subscription: u64,
}

struct Inner {
    writer: Mutex<Box<dyn Write + Send>>,
    /// Shuts the socket down, which ends the reader thread's blocking read.
    close: Box<dyn Fn() + Send + Sync>,
    serial: AtomicU32,
    unique_name: String,
    state: Mutex<State>,
    reader: OnceLock<ThreadId>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        (self.close)();
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A handler that panicked while another thread held a lock leaves plain data behind; the
    // connection keeps working rather than spreading the panic.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A connection to a message bus. Cloning is cheap and every clone shares the socket and the
/// reader thread; the socket closes when the last clone drops (or on [`close`](Self::close)).
///
/// Exported handlers and signal callbacks that capture a clone keep the connection alive, so a
/// long-lived owner calls [`unexport`](Self::unexport) / [`remove_signal_handler`](Self::remove_signal_handler)
/// or [`close`](Self::close) when done. Handlers receive the connection in [`MethodCall`] and
/// signal callbacks as an argument, so they rarely need to capture one.
#[derive(Clone)]
pub struct Connection {
    inner: Arc<Inner>,
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection")
            .field("unique_name", &self.inner.unique_name)
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl Connection {
    /// Connects to the session bus named by `DBUS_SESSION_BUS_ADDRESS`, or to
    /// `$XDG_RUNTIME_DIR/bus` when the variable is unset.
    pub fn session() -> Result<Connection, Error> {
        if let Some(address) = std::env::var_os("DBUS_SESSION_BUS_ADDRESS")
            && !address.is_empty()
        {
            let address = address
                .into_string()
                .map_err(|_| Error::NoBus("DBUS_SESSION_BUS_ADDRESS is not UTF-8".into()))?;
            return Connection::connect(&address);
        }
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|d| !d.is_empty())
            .ok_or_else(|| {
                Error::NoBus("neither DBUS_SESSION_BUS_ADDRESS nor XDG_RUNTIME_DIR is set".into())
            })?;
        let path = std::path::Path::new(&runtime).join("bus");
        let path = path
            .to_str()
            .ok_or_else(|| Error::NoBus("XDG_RUNTIME_DIR is not UTF-8".into()))?;
        Connection::connect(&format!("unix:path={}", escape_address(path)))
    }

    /// Connects to a bus at a D-Bus server address such as `unix:path=/run/user/1000/bus`.
    /// Several `;`-separated addresses are tried in order.
    pub fn connect(address: &str) -> Result<Connection, Error> {
        #[cfg(unix)]
        {
            let stream = unix::connect_address(address)?;
            Connection::from_unix_stream(stream)
        }
        #[cfg(not(unix))]
        {
            let _ = address;
            Err(Error::Unsupported)
        }
    }

    #[cfg(unix)]
    fn from_unix_stream(mut stream: std::os::unix::net::UnixStream) -> Result<Connection, Error> {
        use std::net::Shutdown;
        // The handshake is bounded; the reader thread later blocks without a timeout.
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        authenticate(&mut stream, current_uid())?;
        let reader = stream.try_clone()?;
        let closer = stream.try_clone()?;
        let mut hello_reader = stream.try_clone()?;
        let unique_name = hello(&mut stream, &mut hello_reader)?;
        stream.set_read_timeout(None)?;
        Connection::start(
            Box::new(reader),
            Box::new(stream),
            Box::new(move || {
                let _ = closer.shutdown(Shutdown::Both);
            }),
            unique_name,
        )
    }

    /// Wraps an authenticated, `Hello`-ed transport and starts its reader thread.
    #[cfg_attr(not(unix), allow(dead_code, reason = "only unix transports connect"))]
    fn start(
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        close: Box<dyn Fn() + Send + Sync>,
        unique_name: String,
    ) -> Result<Connection, Error> {
        let inner = Arc::new(Inner {
            writer: Mutex::new(writer),
            close,
            // Serial 1 was the Hello.
            serial: AtomicU32::new(2),
            unique_name,
            state: Mutex::new(State {
                closed: false,
                pending: HashMap::new(),
                objects: HashMap::new(),
                signals: Vec::new(),
                next_subscription: 1,
            }),
            reader: OnceLock::new(),
        });
        let weak = Arc::downgrade(&inner);
        let handle = thread::Builder::new()
            .name("day-dbus".into())
            .spawn(move || reader_loop(reader, weak))?;
        let _ = inner.reader.set(handle.thread().id());
        Ok(Connection { inner })
    }

    /// This connection's unique bus name, such as `:1.42`.
    pub fn unique_name(&self) -> &str {
        &self.inner.unique_name
    }

    /// Whether the connection has closed (the bus went away, or [`close`](Self::close)).
    pub fn is_closed(&self) -> bool {
        lock(&self.inner.state).closed
    }

    /// Closes the socket. Pending and later calls fail with [`Error::Disconnected`], and the
    /// reader thread exits.
    pub fn close(&self) {
        mark_closed(&self.inner);
        (self.inner.close)();
    }

    fn next_serial(&self) -> u32 {
        loop {
            let s = self.inner.serial.fetch_add(1, Ordering::Relaxed);
            if s != 0 {
                return s;
            }
        }
    }

    /// Sends `msg` with a fresh serial, returning that serial.
    pub fn send(&self, mut msg: Message) -> Result<u32, Error> {
        if self.is_closed() {
            return Err(Error::Disconnected);
        }
        msg.serial = self.next_serial();
        let bytes = msg.encode()?;
        self.write(&bytes)?;
        Ok(msg.serial)
    }

    fn write(&self, bytes: &[u8]) -> Result<(), Error> {
        let mut w = lock(&self.inner.writer);
        w.write_all(bytes)?;
        w.flush()?;
        Ok(())
    }

    /// Calls a method and waits up to [`DEFAULT_TIMEOUT`] for its reply's arguments.
    pub fn call(
        &self,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        args: &[Value],
    ) -> Result<Vec<Value>, Error> {
        self.call_with_timeout(destination, path, interface, member, args, DEFAULT_TIMEOUT)
    }

    /// [`call`](Self::call) with an explicit timeout.
    pub fn call_with_timeout(
        &self,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        args: &[Value],
        timeout: Duration,
    ) -> Result<Vec<Value>, Error> {
        if self.inner.reader.get() == Some(&thread::current().id()) {
            return Err(Error::ReaderThread);
        }
        let mut msg = Message::method_call(destination, path, interface, member, args.to_vec());
        msg.serial = self.next_serial();
        let bytes = msg.encode()?;
        let (tx, rx) = mpsc::channel();
        {
            let mut state = lock(&self.inner.state);
            if state.closed {
                return Err(Error::Disconnected);
            }
            state.pending.insert(msg.serial, tx);
        }
        if let Err(e) = self.write(&bytes) {
            lock(&self.inner.state).pending.remove(&msg.serial);
            return Err(e);
        }
        match rx.recv_timeout(timeout) {
            Ok(reply) if reply.message_type == MessageType::Error => Err(Error::Remote {
                name: reply.error_name.unwrap_or_default(),
                message: reply
                    .body
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            }),
            Ok(reply) => Ok(reply.body),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                lock(&self.inner.state).pending.remove(&msg.serial);
                Err(Error::Timeout)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(Error::Disconnected),
        }
    }

    /// Calls a method without waiting, asking the peer not to reply. Safe from handlers.
    pub fn call_no_reply(
        &self,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        args: &[Value],
    ) -> Result<(), Error> {
        let mut msg = Message::method_call(destination, path, interface, member, args.to_vec());
        msg.flags |= FLAG_NO_REPLY_EXPECTED;
        self.send(msg).map(|_| ())
    }

    /// Broadcasts a signal.
    pub fn emit_signal(
        &self,
        path: &str,
        interface: &str,
        member: &str,
        args: &[Value],
    ) -> Result<(), Error> {
        self.send(Message::signal(path, interface, member, args.to_vec()))
            .map(|_| ())
    }

    fn bus_call(&self, member: &str, args: &[Value]) -> Result<Vec<Value>, Error> {
        self.call(BUS_NAME, BUS_PATH, BUS_INTERFACE, member, args)
    }

    /// Asks the bus for a well-known name; returns a `NAME_REPLY_*` code.
    pub fn request_name(&self, name: &str, flags: u32) -> Result<u32, Error> {
        let reply = self.bus_call("RequestName", &[Value::str(name), Value::U32(flags)])?;
        match reply.first() {
            Some(Value::U32(code)) => Ok(*code),
            _ => Err(protocol("RequestName returned no code")),
        }
    }

    /// Gives a well-known name back without waiting for the bus's answer.
    pub fn release_name(&self, name: &str) -> Result<(), Error> {
        self.call_no_reply(
            BUS_NAME,
            BUS_PATH,
            BUS_INTERFACE,
            "ReleaseName",
            &[Value::str(name)],
        )
    }

    /// Whether some connection owns `name`; `false` when the bus cannot be asked.
    pub fn name_has_owner(&self, name: &str) -> bool {
        matches!(
            self.bus_call("NameHasOwner", &[Value::str(name)])
                .as_deref(),
            Ok([Value::Bool(true)])
        )
    }

    /// Asks the bus to route messages matching `rule` (such as
    /// `type='signal',interface='org.example.Iface'`) to this connection.
    pub fn add_match(&self, rule: &str) -> Result<(), Error> {
        self.bus_call("AddMatch", &[Value::str(rule)]).map(|_| ())
    }

    /// Withdraws a rule added with [`add_match`](Self::add_match), without waiting.
    pub fn remove_match(&self, rule: &str) -> Result<(), Error> {
        self.call_no_reply(
            BUS_NAME,
            BUS_PATH,
            BUS_INTERFACE,
            "RemoveMatch",
            &[Value::str(rule)],
        )
    }

    /// Calls `callback` on the reader thread for every incoming signal that `filter` matches.
    pub fn on_signal<F>(&self, filter: SignalFilter, callback: F) -> SubscriptionId
    where
        F: Fn(&Connection, &Message) + Send + Sync + 'static,
    {
        let mut state = lock(&self.inner.state);
        let id = SubscriptionId(state.next_subscription);
        state.next_subscription += 1;
        state.signals.push((id, filter, Arc::new(callback)));
        id
    }

    /// Removes a subscription made with [`on_signal`](Self::on_signal).
    pub fn remove_signal_handler(&self, id: SubscriptionId) {
        lock(&self.inner.state).signals.retain(|(s, ..)| *s != id);
    }

    /// Answers method calls on object `path` with `handler`, which runs on the reader thread.
    ///
    /// The connection answers `org.freedesktop.DBus.Peer` itself. For
    /// `org.freedesktop.DBus.Introspectable.Introspect` it asks the handler first and, when the
    /// handler answers `UnknownMethod`, replies with a minimal document listing the standard
    /// interfaces and any exported child objects ([`introspection_xml`] builds a handler's own).
    pub fn export<F>(&self, path: &str, handler: F) -> Result<(), Error>
    where
        F: Fn(&MethodCall) -> Result<Vec<Value>, MethodError> + Send + Sync + 'static,
    {
        if !is_valid_object_path(path) {
            return Err(protocol(format!("invalid object path {path:?}")));
        }
        let mut state = lock(&self.inner.state);
        if state.objects.contains_key(path) {
            return Err(protocol(format!("{path} is already exported")));
        }
        state.objects.insert(path.to_owned(), Arc::new(handler));
        Ok(())
    }

    /// Stops answering calls on `path`.
    pub fn unexport(&self, path: &str) {
        lock(&self.inner.state).objects.remove(path);
    }
}

#[cfg(unix)]
mod unix {
    use super::Error;
    use std::io;
    use std::os::unix::net::UnixStream;

    /// Decodes a D-Bus address value's `%XX` escapes.
    fn unescape(value: &str) -> Vec<u8> {
        let bytes = value.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            let hex = bytes
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            match (bytes[i], hex) {
                (b'%', Some(b)) => {
                    out.push(b);
                    i += 3;
                }
                (b, _) => {
                    out.push(b);
                    i += 1;
                }
            }
        }
        out
    }

    pub(super) fn connect_address(address: &str) -> Result<UnixStream, Error> {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let mut last = Error::NoBus(format!("no unix transport in {address:?}"));
        for entry in address.split(';').filter(|e| !e.is_empty()) {
            let Some(("unix", params)) = entry.split_once(':') else {
                continue;
            };
            let param = |key: &str| {
                params.split(',').find_map(|kv| {
                    kv.split_once('=')
                        .filter(|(k, _)| *k == key)
                        .map(|(_, v)| unescape(v))
                })
            };
            let result = if let Some(path) = param("path") {
                UnixStream::connect(OsStr::from_bytes(&path))
            } else if let Some(name) = param("abstract") {
                connect_abstract(&name)
            } else {
                continue;
            };
            match result {
                Ok(stream) => return Ok(stream),
                Err(e) => last = Error::Io(e),
            }
        }
        Err(last)
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn connect_abstract(name: &[u8]) -> io::Result<UnixStream> {
        #[cfg(target_os = "android")]
        use std::os::android::net::SocketAddrExt;
        #[cfg(target_os = "linux")]
        use std::os::linux::net::SocketAddrExt;
        let addr = std::os::unix::net::SocketAddr::from_abstract_name(name)?;
        UnixStream::connect_addr(&addr)
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn connect_abstract(_name: &[u8]) -> io::Result<UnixStream> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "abstract unix sockets exist only on Linux",
        ))
    }
}

/// Escapes a path for a D-Bus address value.
fn escape_address(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_/.\\*".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02x}")
            }
        })
        .collect()
}

/// Sends `Hello` (serial 1) and waits for the unique name it returns.
#[cfg_attr(not(unix), allow(dead_code, reason = "only unix transports connect"))]
fn hello(w: &mut dyn Write, r: &mut dyn Read) -> Result<String, Error> {
    let mut msg = Message::method_call(BUS_NAME, BUS_PATH, BUS_INTERFACE, "Hello", Vec::new());
    msg.serial = 1;
    w.write_all(&msg.encode()?)?;
    w.flush()?;
    loop {
        let reply = Message::decode(&read_frame(r)?)?;
        if reply.reply_serial != Some(1) {
            continue;
        }
        if reply.message_type == MessageType::Error {
            return Err(Error::Remote {
                name: reply.error_name.unwrap_or_default(),
                message: String::new(),
            });
        }
        return match reply.body.first() {
            Some(Value::Str(name)) => Ok(name.clone()),
            _ => Err(protocol("Hello returned no name")),
        };
    }
}

fn mark_closed(inner: &Inner) {
    let mut state = lock(&inner.state);
    state.closed = true;
    // Dropping the senders wakes every waiting call with `Disconnected`.
    state.pending.clear();
}

#[cfg_attr(not(unix), allow(dead_code, reason = "only unix transports connect"))]
fn reader_loop(mut reader: Box<dyn Read + Send>, weak: Weak<Inner>) {
    loop {
        let frame = read_frame(&mut *reader);
        let Some(inner) = weak.upgrade() else {
            return;
        };
        let bytes = match frame {
            Ok(bytes) => bytes,
            Err(_) => {
                mark_closed(&inner);
                return;
            }
        };
        let conn = Connection { inner };
        // A message that frames correctly but does not parse is dropped; the stream stays in
        // step because framing only needs the fixed header.
        if let Ok(msg) = Message::decode(&bytes) {
            dispatch(&conn, msg);
        }
    }
}

fn dispatch(conn: &Connection, msg: Message) {
    match msg.message_type {
        MessageType::MethodReturn | MessageType::Error => {
            let Some(serial) = msg.reply_serial else {
                return;
            };
            let waiter = lock(&conn.inner.state).pending.remove(&serial);
            if let Some(tx) = waiter {
                let _ = tx.send(msg);
            }
        }
        MessageType::Signal => {
            let callbacks: Vec<Arc<SignalFn>> = lock(&conn.inner.state)
                .signals
                .iter()
                .filter(|(_, filter, _)| filter.matches(&msg))
                .map(|(_, _, cb)| cb.clone())
                .collect();
            for cb in callbacks {
                cb(conn, &msg);
            }
        }
        MessageType::MethodCall => {
            let reply = answer(conn, &msg);
            if msg.flags & FLAG_NO_REPLY_EXPECTED != 0 {
                return;
            }
            let out = match reply {
                Ok(body) => Message::reply_to(&msg, body),
                Err(e) => Message::error_to(&msg, &e.name, &e.message),
            };
            let out = match out.encode() {
                Ok(_) => out,
                // A handler's reply that cannot be marshaled becomes an error reply, so the
                // caller is not left waiting.
                Err(e) => Message::error_to(
                    &msg,
                    "org.freedesktop.DBus.Error.Failed",
                    &format!("reply could not be sent: {e}"),
                ),
            };
            let _ = conn.send(out);
        }
    }
}

fn answer(conn: &Connection, msg: &Message) -> Result<Vec<Value>, MethodError> {
    let path = msg.path.clone().unwrap_or_default();
    let member = msg.member.clone().unwrap_or_default();
    let interface = msg.interface.as_deref();
    if interface == Some(PEER_INTERFACE) {
        return match member.as_str() {
            "Ping" => Ok(Vec::new()),
            "GetMachineId" => machine_id()
                .map(|id| vec![Value::Str(id)])
                .ok_or_else(|| MethodError::failed("no machine id")),
            _ => Err(MethodError::new(
                "org.freedesktop.DBus.Error.UnknownMethod",
                format!("no method {PEER_INTERFACE}.{member}"),
            )),
        };
    }
    let introspect =
        member == "Introspect" && interface.is_none_or(|i| i == INTROSPECTABLE_INTERFACE);
    let handler = lock(&conn.inner.state).objects.get(&path).cloned();
    let call = MethodCall {
        connection: conn.clone(),
        path: path.clone(),
        interface: msg.interface.clone(),
        member,
        args: msg.body.clone(),
        sender: msg.sender.clone(),
    };
    match handler {
        Some(handler) => match handler(&call) {
            Err(e) if introspect && e.is_unknown_method() => Ok(vec![Value::Str(
                introspection_xml("", &children(conn, &path)),
            )]),
            other => other,
        },
        None => {
            let kids = children(conn, &path);
            if introspect && !kids.is_empty() {
                Ok(vec![Value::Str(introspection_xml("", &kids))])
            } else {
                Err(MethodError::new(
                    "org.freedesktop.DBus.Error.UnknownObject",
                    format!("no object at {path}"),
                ))
            }
        }
    }
}

/// The names of exported objects directly or indirectly below `path`, one segment deep.
fn children(conn: &Connection, path: &str) -> Vec<String> {
    let prefix = if path == "/" {
        "/".to_owned()
    } else {
        format!("{path}/")
    };
    let names: BTreeSet<String> = lock(&conn.inner.state)
        .objects
        .keys()
        .filter_map(|p| p.strip_prefix(&prefix))
        .filter_map(|rest| rest.split('/').next())
        .filter(|seg| !seg.is_empty())
        .map(str::to_owned)
        .collect();
    names.into_iter().collect()
}

fn machine_id() -> Option<String> {
    ["/etc/machine-id", "/var/lib/dbus/machine-id"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// An introspection document: the standard `Peer` and `Introspectable` interfaces, then
/// `interfaces` (raw `<interface>` elements), then a `<node>` per child name.
pub fn introspection_xml(interfaces: &str, children: &[String]) -> String {
    let mut xml = String::from(
        "<!DOCTYPE node PUBLIC \"-//freedesktop//DTD D-BUS Object Introspection 1.0//EN\"\n \
         \"http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd\">\n<node>\n \
         <interface name=\"org.freedesktop.DBus.Peer\">\n  <method name=\"Ping\"/>\n  \
         <method name=\"GetMachineId\"><arg type=\"s\" name=\"machine_uuid\" direction=\"out\"/></method>\n \
         </interface>\n <interface name=\"org.freedesktop.DBus.Introspectable\">\n  \
         <method name=\"Introspect\"><arg type=\"s\" name=\"xml_data\" direction=\"out\"/></method>\n \
         </interface>\n",
    );
    xml.push_str(interfaces);
    for child in children {
        xml.push_str(&format!(" <node name=\"{child}\"/>\n"));
    }
    xml.push_str("</node>\n");
    xml
}

// ---------------------------------------------------------------------------
// Unity launcher entry
// ---------------------------------------------------------------------------

/// The Unity `LauncherEntry` protocol: a badge count, a progress bar and an urgency flag on the
/// app's dock or launcher icon. Docks (Ubuntu Dock, Dash to Dock, Plasma's task manager,
/// Latte, Plank) listen for the `Update` signal and match it to the app by its desktop id.
pub mod launcher {
    use super::{Connection, Error, Value};

    /// The interface the signal belongs to.
    pub const INTERFACE: &str = "com.canonical.Unity.LauncherEntry";

    /// The launcher state to show. Each update states all of it: `None` hides the count or the
    /// progress bar and clears urgency.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct LauncherProps {
        /// The badge number.
        pub count: Option<i64>,
        /// Progress from 0.0 to 1.0 (clamped).
        pub progress: Option<f64>,
        /// Whether the icon asks for attention.
        pub urgent: Option<bool>,
    }

    /// The object path the signal is sent from: one per desktop id, as libunity does.
    pub fn object_path(desktop_id: &str) -> String {
        // FNV-1a, so the path is stable and valid whatever characters the id holds.
        let hash = desktop_id.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
        format!("/com/canonical/unity/launcherentry/{hash}")
    }

    /// Emits `com.canonical.Unity.LauncherEntry.Update` for `desktop_id` (the desktop file's
    /// name, with or without `.desktop`).
    pub fn update(conn: &Connection, desktop_id: &str, props: LauncherProps) -> Result<(), Error> {
        let id = desktop_id.strip_suffix(".desktop").unwrap_or(desktop_id);
        let uri = format!("application://{id}.desktop");
        let entries = vec![
            ("count", Value::I64(props.count.unwrap_or(0))),
            ("count-visible", Value::Bool(props.count.is_some())),
            (
                "progress",
                Value::Double(props.progress.unwrap_or(0.0).clamp(0.0, 1.0)),
            ),
            ("progress-visible", Value::Bool(props.progress.is_some())),
            ("urgent", Value::Bool(props.urgent.unwrap_or(false))),
        ];
        conn.emit_signal(
            &object_path(id),
            INTERFACE,
            "Update",
            &[Value::Str(uri), Value::props(entries)],
        )
    }
}

// ---------------------------------------------------------------------------
// Single-instance forwarding
// ---------------------------------------------------------------------------

/// Single-instance apps over the session bus, for backends whose toolkit does not provide it
/// (GTK has `GApplication`; Qt has nothing).
///
/// The first process to [`claim`](instance::claim) an app id owns a bus name derived from it and
/// exports `/dev/daybrite/Instance`. A later launch finds the name taken, forwards its
/// command-line arguments to the owner through `dev.daybrite.Instance.Forward(as)`, and gets
/// [`Claimed::Forwarded`](instance::Claimed::Forwarded) back, at which point it exits. Reusing
/// the app id as the bus name is safe because a process runs one toolkit, so it never also holds
/// GTK's `GApplication` name.
pub mod instance {
    use super::{
        Connection, Error, INTROSPECTABLE_INTERFACE, MethodError, NAME_FLAG_DO_NOT_QUEUE,
        NAME_REPLY_ALREADY_OWNER, NAME_REPLY_EXISTS, NAME_REPLY_PRIMARY_OWNER, Value,
        introspection_xml,
    };
    use std::fmt;

    /// The object the owning instance exports.
    pub const PATH: &str = "/dev/daybrite/Instance";
    /// The interface of [`PATH`], with the one method `Forward(as)`.
    pub const INTERFACE: &str = "dev.daybrite.Instance";

    const XML: &str = " <interface name=\"dev.daybrite.Instance\">\n  \
        <method name=\"Forward\"><arg type=\"as\" name=\"args\" direction=\"in\"/></method>\n \
        </interface>\n";

    /// Why [`claim`] did not make this process the owner.
    #[derive(Debug)]
    pub enum Claimed {
        /// Another instance owns the app id and received this launch's arguments; this process
        /// should exit.
        Forwarded,
        /// There is no usable session bus, or the owner did not take the arguments (it hung, or
        /// it is not a Day instance). This process runs as an ordinary, unclaimed instance.
        NoBus(Error),
    }

    impl fmt::Display for Claimed {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Claimed::Forwarded => f.write_str("forwarded to the running instance"),
                Claimed::NoBus(e) => write!(f, "single-instance claim unavailable: {e}"),
            }
        }
    }

    impl std::error::Error for Claimed {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            match self {
                Claimed::Forwarded => None,
                Claimed::NoBus(e) => Some(e),
            }
        }
    }

    /// This process's ownership of an app id. Dropping it releases the name, so the next
    /// launch becomes the owner.
    #[derive(Debug)]
    pub struct Claim {
        conn: Connection,
        name: String,
    }

    impl Claim {
        /// The bus name owned.
        pub fn name(&self) -> &str {
            &self.name
        }

        /// The connection holding the name.
        pub fn connection(&self) -> &Connection {
            &self.conn
        }
    }

    impl Drop for Claim {
        fn drop(&mut self) {
            self.conn.unexport(PATH);
            let _ = self.conn.release_name(&self.name);
        }
    }

    /// Turns an app id into a legal well-known bus name: characters outside `[A-Za-z0-9_-]`
    /// become `_`, an element that is empty or starts with a digit gains a leading `_`, an id
    /// with a single element is put under `dev.daybrite.instance.`, and an overlong one is
    /// replaced by a hash under that prefix.
    pub fn bus_name(app_id: &str) -> String {
        let elements: Vec<String> = app_id
            .split('.')
            .map(|el| {
                let mut out: String = el
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
                    out.insert(0, '_');
                }
                out
            })
            .collect();
        let name = if elements.len() < 2 {
            format!("dev.daybrite.instance.{}", elements.join("."))
        } else {
            elements.join(".")
        };
        if name.len() <= 255 {
            return name;
        }
        let hash = app_id.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
        format!("dev.daybrite.instance.h{hash:016x}")
    }

    /// Whether a failed `Forward` means the owner went away between `RequestName` and the call.
    fn owner_vanished(e: &Error) -> bool {
        match e {
            Error::Remote { name, .. } => matches!(
                name.as_str(),
                "org.freedesktop.DBus.Error.ServiceUnknown"
                    | "org.freedesktop.DBus.Error.NameHasNoOwner"
                    | "org.freedesktop.DBus.Error.NoReply"
            ),
            _ => false,
        }
    }

    /// Claims `app_id` for this process, or forwards `args` (normally
    /// `std::env::args().skip(1)`) to the process that already holds it.
    ///
    /// `on_forward` receives each later launch's arguments **on the connection's reader
    /// thread**; the caller hands them to its UI thread (for example through its platform's
    /// post-to-main mechanism) rather than touching UI state there. It must not block for long:
    /// the forwarding process waits for it to return.
    pub fn claim<F>(app_id: &str, args: Vec<String>, on_forward: F) -> Result<Claim, Claimed>
    where
        F: Fn(Vec<String>) + Send + Sync + 'static,
    {
        let conn = Connection::session().map_err(Claimed::NoBus)?;
        let name = bus_name(app_id);
        // Export before owning the name, so a launch that sees the name always finds the
        // object behind it.
        conn.export(PATH, move |call| {
            match (call.interface.as_deref(), call.member.as_str()) {
                (Some(INTERFACE) | None, "Forward") => {
                    let [Value::Array { elem, items }] = call.args.as_slice() else {
                        return Err(MethodError::invalid_args("expected (as)"));
                    };
                    if elem != "s" {
                        return Err(MethodError::invalid_args("expected (as)"));
                    }
                    on_forward(
                        items
                            .iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect(),
                    );
                    Ok(Vec::new())
                }
                (Some(INTROSPECTABLE_INTERFACE) | None, "Introspect") => {
                    Ok(vec![Value::Str(introspection_xml(XML, &[]))])
                }
                _ => Err(MethodError::unknown_method(call)),
            }
        })
        .map_err(Claimed::NoBus)?;
        let forwarded = Value::strings(&args);
        let mut last = Error::Disconnected;
        // One retry: the owner can exit between our RequestName and our Forward.
        for _ in 0..2 {
            match conn.request_name(&name, NAME_FLAG_DO_NOT_QUEUE) {
                Ok(NAME_REPLY_PRIMARY_OWNER | NAME_REPLY_ALREADY_OWNER) => {
                    return Ok(Claim { conn, name });
                }
                Ok(NAME_REPLY_EXISTS) => {}
                Ok(code) => {
                    last = Error::Protocol(format!("RequestName answered {code}"));
                    break;
                }
                Err(e) => {
                    last = e;
                    break;
                }
            }
            match conn.call(
                &name,
                PATH,
                INTERFACE,
                "Forward",
                std::slice::from_ref(&forwarded),
            ) {
                Ok(_) => {
                    conn.unexport(PATH);
                    return Err(Claimed::Forwarded);
                }
                Err(e) if owner_vanished(&e) => last = e,
                Err(e) => {
                    last = e;
                    break;
                }
            }
        }
        conn.unexport(PATH);
        Err(Claimed::NoBus(last))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(values: Vec<Value>) {
        let sig: String = values.iter().map(Value::signature).collect();
        let bytes = marshal(&values).expect("marshal");
        assert_eq!(unmarshal(&bytes, &sig, false).expect("unmarshal"), values);
        let mut e = Encoder::new(true);
        for v in &values {
            e.value(v, 0).expect("marshal big-endian");
        }
        assert_eq!(unmarshal(&e.buf, &sig, true).expect("unmarshal BE"), values);
    }

    #[test]
    fn every_basic_type_round_trips() {
        round_trip(vec![
            Value::Byte(0xab),
            Value::Bool(true),
            Value::Bool(false),
            Value::I16(-2),
            Value::U16(65535),
            Value::I32(-70000),
            Value::U32(0xdead_beef),
            Value::I64(i64::MIN),
            Value::U64(u64::MAX),
            Value::Double(-1.5),
            Value::str("héllo"),
            Value::str(""),
            Value::path("/org/example/Obj_1"),
            Value::Signature("a{sv}(iiay)".into()),
        ]);
    }

    #[test]
    fn containers_round_trip() {
        round_trip(vec![
            Value::Struct(vec![
                Value::I32(1),
                Value::str("x"),
                Value::bytes(&[1, 2, 3]),
            ]),
            Value::props([("a", Value::U32(7)), ("b", Value::strings(&["p", "q"]))]),
            Value::dict("i", "s", [(Value::I32(3), Value::str("three"))]),
            Value::array(
                "(iiay)",
                vec![Value::Struct(vec![
                    Value::I32(2),
                    Value::I32(2),
                    Value::bytes(&[0; 16]),
                ])],
            ),
        ]);
    }

    #[test]
    fn struct_after_a_byte_is_padded_to_eight() {
        let values = vec![Value::Byte(1), Value::Struct(vec![Value::Byte(2)])];
        let bytes = marshal(&values).expect("marshal");
        assert_eq!(bytes, [1, 0, 0, 0, 0, 0, 0, 0, 2]);
        round_trip(values);
    }

    #[test]
    fn empty_arrays_still_pad_to_their_element_alignment() {
        // y, then a(ii): length at 4, padding to 8, no elements.
        let values = vec![Value::Byte(9), Value::array("(ii)", Vec::new())];
        let bytes = marshal(&values).expect("marshal");
        assert_eq!(bytes, [9, 0, 0, 0, 0, 0, 0, 0]);
        round_trip(values);
        // a{sv} after a u32 lands its length at 4 and is padded to 8 too.
        let values = vec![Value::U32(1), Value::props(Vec::<(&str, Value)>::new())];
        assert_eq!(marshal(&values).expect("marshal").len(), 8);
        round_trip(values);
        // An empty array of bytes needs no padding after its length.
        let values = vec![Value::Byte(1), Value::bytes(&[]), Value::Byte(2)];
        assert_eq!(
            marshal(&values).expect("marshal"),
            [1, 0, 0, 0, 0, 0, 0, 0, 2]
        );
        round_trip(values);
    }

    #[test]
    fn array_length_excludes_the_leading_padding() {
        // a(i) holding one struct: length 4, padding to 8, then the i32.
        let bytes = marshal(&[Value::array(
            "(i)",
            vec![Value::Struct(vec![Value::I32(5)])],
        )])
        .expect("marshal");
        assert_eq!(bytes, [4, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0]);
    }

    #[test]
    fn nested_variants_round_trip() {
        let deep = Value::variant(Value::variant(Value::variant(Value::Struct(vec![
            Value::Byte(1),
            Value::variant(Value::array("v", vec![Value::variant(Value::I64(-1))])),
        ]))));
        assert_eq!(deep.signature(), "v");
        round_trip(vec![Value::Byte(7), deep]);
    }

    #[test]
    fn mismatched_array_items_are_rejected() {
        let bad = Value::array("s", vec![Value::I32(1)]);
        assert!(marshal(&[bad]).is_err());
        assert!(marshal(&[Value::path("no/slash")]).is_err());
        assert!(marshal(&[Value::Struct(Vec::new())]).is_err());
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        assert!(unmarshal(&[1, 0, 0, 0], "b", false).is_ok());
        assert!(unmarshal(&[2, 0, 0, 0], "b", false).is_err());
        assert!(unmarshal(&[0xff, 0xff, 0xff, 0x0f], "ay", false).is_err());
        assert!(unmarshal(&[3, 0, 0, 0, b'a', b'b'], "s", false).is_err());
        assert!(unmarshal(&[], "a{vs}", false).is_err());
        assert!(Message::decode(b"x").is_err());
        for len in 0..40 {
            let _ = Message::decode(&vec![b'l'; len]);
        }
    }

    #[test]
    fn signatures_parse() {
        assert_eq!(split_first_type("a{sv}i").expect("split"), ("a{sv}", "i"));
        assert_eq!(
            split_first_type("(ia(s))x").expect("split"),
            ("(ia(s))", "x")
        );
        for bad in ["(", "()", "a", "a{vs}", "a{s}", "a{sss}", "z", "(i"] {
            assert!(validate_signature(bad).is_err(), "{bad}");
        }
    }

    /// `Hello` (serial 1), assembled by hand from the specification's marshaling rules: the
    /// path, interface, member and destination header fields each 8-aligned, an empty body, no
    /// signature field, and the header padded to 128 bytes.
    #[test]
    fn hello_matches_the_spec_bytes() {
        let mut hello =
            Message::method_call(BUS_NAME, BUS_PATH, BUS_INTERFACE, "Hello", Vec::new());
        hello.serial = 1;
        let mut want: Vec<u8> = Vec::new();
        // l, METHOD_CALL, no flags, version 1; body length 0; serial 1; fields length 0x6d.
        want.extend_from_slice(&[b'l', 1, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0x6d, 0, 0, 0]);
        // PATH (1) 'o' "/org/freedesktop/DBus" (21 bytes) at 16.
        want.extend_from_slice(&[1, 1, b'o', 0, 21, 0, 0, 0]);
        want.extend_from_slice(b"/org/freedesktop/DBus\0");
        want.extend_from_slice(&[0, 0]); // to 48
        // INTERFACE (2) 's' "org.freedesktop.DBus" (20 bytes) at 48.
        want.extend_from_slice(&[2, 1, b's', 0, 20, 0, 0, 0]);
        want.extend_from_slice(b"org.freedesktop.DBus\0");
        want.extend_from_slice(&[0, 0, 0]); // to 80
        // MEMBER (3) 's' "Hello" at 80.
        want.extend_from_slice(&[3, 1, b's', 0, 5, 0, 0, 0]);
        want.extend_from_slice(b"Hello\0");
        want.extend_from_slice(&[0, 0]); // to 96
        // DESTINATION (6) 's' "org.freedesktop.DBus" at 96, ending at 125.
        want.extend_from_slice(&[6, 1, b's', 0, 20, 0, 0, 0]);
        want.extend_from_slice(b"org.freedesktop.DBus\0");
        want.extend_from_slice(&[0, 0, 0]); // header padded to 128
        assert_eq!(want.len(), 128);
        assert_eq!(hello.encode().expect("encode"), want);
        assert_eq!(Message::decode(&want).expect("decode"), hello);
    }

    #[test]
    fn big_endian_header_parses() {
        // A signal written by hand in big-endian order:
        // B, SIGNAL, flags 0, version 1; body length 4; serial 7.
        let mut m: Vec<u8> = vec![b'B', 4, 0, 1, 0, 0, 0, 4, 0, 0, 0, 7];
        let mut fields: Vec<u8> = Vec::new();
        // PATH "/a" at 16.
        fields.extend_from_slice(&[1, 1, b'o', 0, 0, 0, 0, 2, b'/', b'a', 0]);
        fields.extend_from_slice(&[0, 0, 0, 0, 0]); // 27 → 32
        // INTERFACE "x.y" at 32.
        fields.extend_from_slice(&[2, 1, b's', 0, 0, 0, 0, 3, b'x', b'.', b'y', 0]);
        fields.extend_from_slice(&[0, 0, 0, 0]); // 44 → 48
        // MEMBER "Z" at 48.
        fields.extend_from_slice(&[3, 1, b's', 0, 0, 0, 0, 1, b'Z', 0]);
        fields.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // 58 → 64
        // SENDER ":1.5" at 64, then SIGNATURE "u" at 80.
        fields.extend_from_slice(&[7, 1, b's', 0, 0, 0, 0, 4, b':', b'1', b'.', b'5', 0]);
        fields.extend_from_slice(&[0, 0, 0]); // 77 → 80
        fields.extend_from_slice(&[8, 1, b'g', 0, 1, b'u', 0]); // ends at 87
        let fields_len = fields.len() as u32;
        m.extend_from_slice(&fields_len.to_be_bytes());
        m.extend_from_slice(&fields);
        m.resize(88, 0); // header padded to 8
        m.extend_from_slice(&[1, 2, 3, 4]); // body: u32 0x01020304
        let msg = Message::decode(&m).expect("decode big-endian");
        assert_eq!(msg.message_type, MessageType::Signal);
        assert_eq!(msg.serial, 7);
        assert_eq!(msg.path.as_deref(), Some("/a"));
        assert_eq!(msg.interface.as_deref(), Some("x.y"));
        assert_eq!(msg.member.as_deref(), Some("Z"));
        assert_eq!(msg.sender.as_deref(), Some(":1.5"));
        assert_eq!(msg.body, vec![Value::U32(0x0102_0304)]);
        // Framing reads the big-endian lengths too.
        let framed = read_frame(&mut &m[..]).expect("frame");
        assert_eq!(framed.len(), m.len());
        // And our own big-endian encoder agrees byte for byte with the hand-built message.
        let mut ours = Message::signal("/a", "x.y", "Z", vec![Value::U32(0x0102_0304)]);
        ours.serial = 7;
        ours.sender = Some(":1.5".into());
        let decoded = Message::decode(&ours.encode_with(true).expect("encode BE")).expect("BE");
        assert_eq!(decoded, msg);
    }

    #[test]
    fn auth_sends_uid_as_hex_ascii() {
        struct Script {
            replies: io::Cursor<Vec<u8>>,
            sent: Vec<u8>,
        }
        impl Read for Script {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.replies.read(buf)
            }
        }
        impl Write for Script {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.sent.extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut s = Script {
            replies: io::Cursor::new(b"OK 1234deadbeef\r\n".to_vec()),
            sent: Vec::new(),
        };
        authenticate(&mut s, Some(1000)).expect("auth");
        assert_eq!(s.sent, b"\0AUTH EXTERNAL 31303030\r\nBEGIN\r\n");

        let mut s = Script {
            replies: io::Cursor::new(b"DATA\r\nOK 1234\r\n".to_vec()),
            sent: Vec::new(),
        };
        authenticate(&mut s, None).expect("auth without uid");
        assert_eq!(s.sent, b"\0AUTH EXTERNAL\r\nDATA\r\nBEGIN\r\n");

        let mut s = Script {
            replies: io::Cursor::new(b"REJECTED EXTERNAL\r\n".to_vec()),
            sent: Vec::new(),
        };
        assert!(matches!(authenticate(&mut s, Some(0)), Err(Error::Auth(_))));
    }

    #[test]
    fn instance_bus_names_are_legal() {
        use instance::bus_name;
        for (id, want) in [
            ("dev.daybrite.Notes", "dev.daybrite.Notes"),
            ("com.example.my-app", "com.example.my-app"),
            ("org.7zip.App", "org._7zip.App"),
            ("io.github.user.2048", "io.github.user._2048"),
            ("notes", "dev.daybrite.instance.notes"),
            ("a..b", "a._.b"),
            ("dev.sun rise/x.é", "dev.sun_rise_x._"),
            ("", "dev.daybrite.instance._"),
        ] {
            let name = bus_name(id);
            assert_eq!(name, want, "{id:?}");
            assert!(is_valid_bus_name(&name), "{name}");
        }
        let long = format!("dev.daybrite.{}", "x".repeat(300));
        let name = bus_name(&long);
        assert!(name.len() <= 255 && is_valid_bus_name(&name), "{name}");
        assert_eq!(name, bus_name(&long));
    }

    #[test]
    fn launcher_path_is_valid() {
        assert!(is_valid_object_path(&launcher::object_path(
            "dev.daybrite.Some-App"
        )));
    }

    /// A fake bus on a socket pair: it authenticates, answers `Hello`, echoes one method call,
    /// then hangs up, after which calls fail instead of hanging.
    #[cfg(unix)]
    #[test]
    fn connection_calls_and_disconnects() {
        use std::os::unix::net::UnixStream;
        let (client, mut bus) = UnixStream::pair().expect("pair");
        let peer = thread::spawn(move || {
            // NUL, then one request per line: AUTH (with or without a uid), DATA, BEGIN.
            let mut nul = [0u8];
            bus.read_exact(&mut nul).expect("read NUL");
            loop {
                let line = read_line(&mut bus).expect("read auth line");
                match line.as_str() {
                    "AUTH EXTERNAL" => bus.write_all(b"DATA\r\n").expect("write"),
                    "BEGIN" => break,
                    _ => bus.write_all(b"OK 0123456789abcdef\r\n").expect("write"),
                }
            }
            for _ in 0..2 {
                let call = Message::decode(&read_frame(&mut bus).expect("frame")).expect("msg");
                let name = if call.member.as_deref() == Some("Hello") {
                    vec![Value::str(":1.7")]
                } else {
                    call.body.clone()
                };
                let mut reply = Message::reply_to(&call, name);
                reply.serial = 100 + call.serial;
                bus.write_all(&reply.encode().expect("encode"))
                    .expect("write");
            }
            // Leave the third call unanswered and hang up.
            let _ = read_frame(&mut bus);
        });
        let conn = Connection::from_unix_stream(client).expect("connect");
        assert_eq!(conn.unique_name(), ":1.7");
        let echoed = conn
            .call("x.y", "/", "x.y", "Echo", &[Value::str("ping")])
            .expect("echo");
        assert_eq!(echoed, vec![Value::str("ping")]);
        let err = conn.call("x.y", "/", "x.y", "Never", &[]);
        assert!(matches!(err, Err(Error::Disconnected)), "{err:?}");
        peer.join().expect("peer");
        assert!(conn.is_closed());
        assert!(matches!(
            conn.emit_signal("/", "x.y", "Z", &[]),
            Err(Error::Disconnected)
        ));
    }
}
