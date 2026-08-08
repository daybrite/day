// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The report data model and its two serialization formats.
//!
//! Crash artifacts are written in a line-oriented `key=value` **kv** format, never JSON: the signal
//! handler ([`crate::signals_unix`]) can only emit ASCII from an async-signal-safe context, and the
//! Android Java shim emits the same format trivially. On the NEXT launch, [`crate::store`] reads the
//! kv artifacts plus the session sentinel and composes a [`Report`], which is written out as the
//! stable, schema-versioned JSON a transport uploads. Nothing in the runtime path parses JSON.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// The report schema version. Grow-only: fields are added, never removed or repurposed, so an
/// ingest server can key off `schema` and stay backward-compatible.
pub const SCHEMA: u32 = 1;

/// What ended the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A Rust panic that reached the process boundary (the app died).
    Panic,
    /// A POSIX signal (SIGSEGV/SIGABRT/…) — a native fault or abort.
    Signal,
    /// An uncaught Java/JVM exception on Android.
    Java,
    /// A panic day-core CONTAINED at a trampoline boundary — the app kept running. Non-fatal;
    /// recorded for diagnostics, distinguished from a real crash.
    Contained,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Panic => "panic",
            Kind::Signal => "signal",
            Kind::Java => "java",
            Kind::Contained => "contained",
        }
    }

    pub fn from_tag(s: &str) -> Option<Kind> {
        match s {
            "panic" => Some(Kind::Panic),
            "signal" => Some(Kind::Signal),
            "java" => Some(Kind::Java),
            "contained" => Some(Kind::Contained),
            _ => None,
        }
    }
}

/// Signal-specific fields (present only when `kind == Signal`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignalInfo {
    pub signo: i64,
    pub name: String,
    pub code: i64,
    /// Faulting address (`si_addr`), hex.
    pub addr: usize,
    /// Program counter at the fault (from the ucontext), hex; `0` when the arch is unknown.
    pub pc: usize,
    /// ASLR load slide captured at init; `pc - slide` symbolizes offline against the shipped binary.
    pub slide: usize,
}

/// A finalized crash report — the unit the consent UI displays and a [`crate::Reporter`] uploads.
/// Every field is best-effort; unknowns are empty or `"unknown"`, never a failure.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub kind_str: String,
    pub fatal: bool,
    pub app_id: String,
    pub app_version: String,
    pub app_build: String,
    pub day_version: String,
    pub backend: String,
    pub os_name: String,
    pub os_version: String,
    pub device_model: String,
    pub simulator: bool,
    pub locale: String,
    pub session_id: String,
    pub started_at_ms: u64,
    pub uptime_ms: u64,
    pub message: String,
    pub location: String,
    pub thread: String,
    pub main_thread: bool,
    pub signal: Option<SignalInfo>,
    pub backtrace_text: String,
}

impl Report {
    pub fn kind(&self) -> Option<Kind> {
        Kind::from_tag(&self.kind_str)
    }

    /// The human-readable report text shown to the user on the disclosure surface. This is exactly
    /// what an upload transmits (the JSON is a machine mirror of the same facts) — no hidden fields.
    pub fn display_text(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "Day crash report");
        let _ = writeln!(
            s,
            "  kind:     {}{}",
            self.kind_str,
            if self.fatal { "" } else { " (non-fatal)" }
        );
        if !self.message.is_empty() {
            let _ = writeln!(s, "  message:  {}", self.message);
        }
        if !self.location.is_empty() {
            let _ = writeln!(s, "  location: {}", self.location);
        }
        let _ = writeln!(
            s,
            "  app:      {} {} ({})",
            self.app_id, self.app_version, self.app_build
        );
        let _ = writeln!(s, "  day:      {} · {}", self.day_version, self.backend);
        let _ = writeln!(
            s,
            "  os:       {} {}{}",
            self.os_name,
            self.os_version,
            if self.simulator { " (simulator)" } else { "" }
        );
        if !self.device_model.is_empty() {
            let _ = writeln!(s, "  device:   {}", self.device_model);
        }
        let _ = writeln!(s, "  locale:   {}", self.locale);
        let _ = writeln!(
            s,
            "  thread:   {}{}",
            self.thread,
            if self.main_thread { " (main)" } else { "" }
        );
        let _ = writeln!(s, "  uptime:   {} ms", self.uptime_ms);
        if let Some(sig) = &self.signal {
            let _ = writeln!(
                s,
                "  signal:   {} ({}) code={} addr={:#x} pc={:#x} slide={:#x}",
                sig.signo, sig.name, sig.code, sig.addr, sig.pc, sig.slide
            );
        }
        if !self.backtrace_text.is_empty() {
            let _ = writeln!(s, "\nBacktrace:\n{}", self.backtrace_text);
        }
        s
    }

    /// The schema-versioned JSON, hand-rolled (the runtime crates avoid serde). Field order is
    /// stable; a transport parses this with whatever JSON library it already has.
    pub fn to_json(&self) -> String {
        let mut s = String::with_capacity(1024);
        s.push('{');
        json_num(&mut s, "schema", SCHEMA as u64, true);
        json_str(&mut s, "kind", &self.kind_str, false);
        json_bool(&mut s, "fatal", self.fatal, false);
        // app { }
        s.push_str(",\"app\":{");
        json_str(&mut s, "id", &self.app_id, true);
        json_str(&mut s, "version", &self.app_version, false);
        json_str(&mut s, "build", &self.app_build, false);
        s.push('}');
        // day { }
        s.push_str(",\"day\":{");
        json_str(&mut s, "version", &self.day_version, true);
        json_str(&mut s, "backend", &self.backend, false);
        s.push('}');
        // os { }
        s.push_str(",\"os\":{");
        json_str(&mut s, "name", &self.os_name, true);
        json_str(&mut s, "version", &self.os_version, false);
        s.push('}');
        // device { }
        s.push_str(",\"device\":{");
        json_str(&mut s, "model", &self.device_model, true);
        json_bool(&mut s, "simulator", self.simulator, false);
        s.push('}');
        json_str(&mut s, "locale", &self.locale, false);
        // session { }
        s.push_str(",\"session\":{");
        json_str(&mut s, "id", &self.session_id, true);
        json_num(&mut s, "started_at_ms", self.started_at_ms, false);
        json_num(&mut s, "uptime_ms", self.uptime_ms, false);
        s.push('}');
        json_str(&mut s, "message", &self.message, false);
        json_str(&mut s, "location", &self.location, false);
        // thread { }
        s.push_str(",\"thread\":{");
        json_str(&mut s, "name", &self.thread, true);
        json_bool(&mut s, "main", self.main_thread, false);
        s.push('}');
        if let Some(sig) = &self.signal {
            s.push_str(",\"signal\":{");
            json_num(&mut s, "signo", sig.signo as u64, true);
            json_str(&mut s, "name", &sig.name, false);
            json_num(&mut s, "code", sig.code as u64, false);
            json_num(&mut s, "addr", sig.addr as u64, false);
            json_num(&mut s, "pc", sig.pc as u64, false);
            json_num(&mut s, "slide", sig.slide as u64, false);
            s.push('}');
        }
        json_str(&mut s, "backtrace_text", &self.backtrace_text, false);
        s.push('}');
        s
    }
}

/// Reconstruct a [`Report`] from the JSON we wrote — so the consent UI and transports can display
/// and re-serialize a finalized report without a sidecar. Returns `None` if the JSON is unparseable.
pub fn parse_json(s: &str) -> Option<Report> {
    let flat = json_flatten(s)?;
    let g = |k: &str| flat.get(k).cloned().unwrap_or_default();
    let gn = |k: &str| flat.get(k).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let gb = |k: &str| flat.get(k).map(|v| v == "true").unwrap_or(false);

    let signal = if flat.contains_key("signal.signo") {
        Some(SignalInfo {
            signo: gn("signal.signo") as i64,
            name: g("signal.name"),
            code: gn("signal.code") as i64,
            addr: gn("signal.addr") as usize,
            pc: gn("signal.pc") as usize,
            slide: gn("signal.slide") as usize,
        })
    } else {
        None
    };

    Some(Report {
        kind_str: g("kind"),
        fatal: gb("fatal"),
        app_id: g("app.id"),
        app_version: g("app.version"),
        app_build: g("app.build"),
        day_version: g("day.version"),
        backend: g("day.backend"),
        os_name: g("os.name"),
        os_version: g("os.version"),
        device_model: g("device.model"),
        simulator: gb("device.simulator"),
        locale: g("locale"),
        session_id: g("session.id"),
        started_at_ms: gn("session.started_at_ms"),
        uptime_ms: gn("session.uptime_ms"),
        message: g("message"),
        location: g("location"),
        thread: g("thread.name"),
        main_thread: gb("thread.main"),
        signal,
        backtrace_text: g("backtrace_text"),
    })
}

/// A minimal JSON object reader specialized to our own emitter's output: an object of scalars
/// (string/number/bool) and one level of nested objects. Nested keys are flattened with a dot
/// (`app.id`). Not a general JSON parser — it round-trips [`Report::to_json`] and nothing else.
fn json_flatten(s: &str) -> Option<BTreeMap<String, String>> {
    let b = s.as_bytes();
    let mut i = 0usize;
    let mut out = BTreeMap::new();
    read_object(b, &mut i, "", &mut out)?;
    Some(out)
}

fn read_object(
    b: &[u8],
    i: &mut usize,
    prefix: &str,
    out: &mut BTreeMap<String, String>,
) -> Option<()> {
    skip_ws(b, i);
    if *b.get(*i)? != b'{' {
        return None;
    }
    *i += 1;
    loop {
        skip_ws(b, i);
        match b.get(*i)? {
            b'}' => {
                *i += 1;
                return Some(());
            }
            b',' => {
                *i += 1;
                continue;
            }
            b'"' => {
                let key = read_string(b, i)?;
                skip_ws(b, i);
                if *b.get(*i)? != b':' {
                    return None;
                }
                *i += 1;
                skip_ws(b, i);
                let full = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                match b.get(*i)? {
                    b'{' => read_object(b, i, &full, out)?,
                    b'"' => {
                        let v = read_string(b, i)?;
                        out.insert(full, v);
                    }
                    _ => {
                        let v = read_scalar(b, i);
                        out.insert(full, v);
                    }
                }
            }
            _ => return None,
        }
    }
}

fn skip_ws(b: &[u8], i: &mut usize) {
    while let Some(c) = b.get(*i) {
        if c.is_ascii_whitespace() {
            *i += 1;
        } else {
            break;
        }
    }
}

fn read_string(b: &[u8], i: &mut usize) -> Option<String> {
    if *b.get(*i)? != b'"' {
        return None;
    }
    *i += 1;
    // Accumulate raw bytes so multi-byte UTF-8 survives; decode once at the end.
    let mut out: Vec<u8> = Vec::new();
    while let Some(&c) = b.get(*i) {
        *i += 1;
        match c {
            b'"' => return Some(String::from_utf8_lossy(&out).into_owned()),
            b'\\' => {
                let e = *b.get(*i)?;
                *i += 1;
                match e {
                    b'"' => out.push(b'"'),
                    b'\\' => out.push(b'\\'),
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'u' => {
                        // \uXXXX — read 4 hex digits, encode the code point as UTF-8.
                        let hex = std::str::from_utf8(b.get(*i..*i + 4)?).ok()?;
                        let cp = u32::from_str_radix(hex, 16).ok()?;
                        *i += 4;
                        let ch = char::from_u32(cp).unwrap_or('\u{fffd}');
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                    }
                    other => out.push(other),
                }
            }
            _ => out.push(c),
        }
    }
    None
}

fn read_scalar(b: &[u8], i: &mut usize) -> String {
    let start = *i;
    while let Some(&c) = b.get(*i) {
        if c == b',' || c == b'}' || c.is_ascii_whitespace() {
            break;
        }
        *i += 1;
    }
    String::from_utf8_lossy(&b[start..*i]).to_string()
}

// ---- kv codec ------------------------------------------------------------------------------

/// Encode a value for a kv line: escape `\` and newlines so a value is always single-line. The
/// signal handler writes raw ASCII directly (it can't call this), but every non-signal writer and
/// the Java shim's format agree with it.
pub fn kv_escape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

fn kv_unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A parsed kv artifact: last value wins on a duplicate key.
pub type Fields = BTreeMap<String, String>;

/// Parse a kv artifact (tolerant: blank and malformed lines are skipped, so a partially-written
/// signal file still yields whatever complete lines it has).
pub fn parse_kv(text: &str) -> Fields {
    let mut map = Fields::new();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            if !k.is_empty() {
                map.insert(k.to_string(), kv_unescape(v));
            }
        }
    }
    map
}

/// Build a kv document from ordered pairs (values escaped).
pub fn write_kv(pairs: &[(&str, String)]) -> String {
    let mut s = String::new();
    for (k, v) in pairs {
        let _ = writeln!(s, "{k}={}", kv_escape(v));
    }
    s
}

// ---- JSON helpers --------------------------------------------------------------------------

fn json_str(s: &mut String, key: &str, val: &str, first: bool) {
    if !first {
        s.push(',');
    }
    s.push('"');
    s.push_str(key);
    s.push_str("\":");
    json_escape_into(s, val);
}

fn json_num(s: &mut String, key: &str, val: u64, first: bool) {
    if !first {
        s.push(',');
    }
    let _ = write!(s, "\"{key}\":{val}");
}

fn json_bool(s: &mut String, key: &str, val: bool, first: bool) {
    if !first {
        s.push(',');
    }
    let _ = write!(s, "\"{key}\":{val}");
}

fn json_escape_into(s: &mut String, val: &str) {
    s.push('"');
    for c in val.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(s, "\\u{:04x}", c as u32);
            }
            c => s.push(c),
        }
    }
    s.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kv_roundtrip_escapes_newlines_and_backslashes() {
        let pairs = [
            ("message", "boom\nsecond line\\path".to_string()),
            ("loc", "src/x.rs:10:4".to_string()),
        ];
        let doc = write_kv(&pairs);
        // Every record stays on one physical line.
        assert_eq!(doc.lines().count(), 2);
        let parsed = parse_kv(&doc);
        assert_eq!(parsed["message"], "boom\nsecond line\\path");
        assert_eq!(parsed["loc"], "src/x.rs:10:4");
    }

    #[test]
    fn parse_kv_skips_blank_and_malformed_lines() {
        let parsed = parse_kv("a=1\n\ngarbage-no-eq\n=novalue-key\nb=2\n");
        assert_eq!(parsed.get("a").map(String::as_str), Some("1"));
        assert_eq!(parsed.get("b").map(String::as_str), Some("2"));
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn json_escapes_control_and_quotes() {
        let r = Report {
            kind_str: "panic".into(),
            message: "he said \"hi\"\nthen \t left".into(),
            ..Default::default()
        };
        let j = r.to_json();
        assert!(j.contains(r#""kind":"panic""#));
        assert!(j.contains(r#"\"hi\""#));
        assert!(j.contains(r"\n"));
        assert!(j.contains(r"\t"));
        assert!(j.starts_with(r#"{"schema":1"#));
    }

    #[test]
    fn json_round_trips_including_unicode_and_signal() {
        let original = Report {
            kind_str: "signal".into(),
            fatal: true,
            app_id: "dev.example.café".into(),
            app_version: "1.0".into(),
            app_build: "9".into(),
            day_version: "0.0.14".into(),
            backend: "ios-uikit".into(),
            os_name: "iOS".into(),
            os_version: "18.0".into(),
            device_model: "iPhone".into(),
            simulator: true,
            locale: "fr".into(),
            session_id: "abc-123".into(),
            started_at_ms: 1700,
            uptime_ms: 42,
            message: "boom: état \"invalide\"\nline two".into(),
            location: "src/x.rs:1:2".into(),
            thread: "main".into(),
            main_thread: true,
            signal: Some(SignalInfo {
                signo: 11,
                name: "SIGSEGV".into(),
                code: 1,
                addr: 0,
                pc: 4096,
                slide: 256,
            }),
            backtrace_text: "frame0\nframe1".into(),
        };
        let json = original.to_json();
        let parsed = parse_json(&json).expect("parse own json");
        assert_eq!(parsed, original);
    }

    #[test]
    fn parse_json_rejects_garbage() {
        assert!(parse_json("not json").is_none());
        assert!(parse_json("").is_none());
    }

    #[test]
    fn signal_block_only_when_present() {
        let mut r = Report {
            kind_str: "signal".into(),
            ..Default::default()
        };
        // No signal BLOCK when `signal` is None (the "kind":"signal" value is a separate thing).
        assert!(!r.to_json().contains("\"signal\":{"));
        r.signal = Some(SignalInfo {
            signo: 11,
            name: "SIGSEGV".into(),
            pc: 0x1000,
            ..Default::default()
        });
        let j = r.to_json();
        assert!(j.contains("\"signal\":{"));
        assert!(j.contains("\"signo\":11"));
    }
}
