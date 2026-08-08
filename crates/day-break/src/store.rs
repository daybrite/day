// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! On-disk layout, the session sentinel, and next-launch reconciliation.
//!
//! Everything a session might leave behind lives in one directory, filenames keyed by a per-launch
//! session id `sid` so [`reconcile`] can group artifacts without opening them:
//!
//! ```text
//! session-<sid>.kv        sentinel: static context, written at init, DELETED on WillTerminate
//! sig-<sid>.sig           raw signal record: fd pre-opened at init, stays empty unless a signal fires
//! pending-<sid>-<seq>.kv  a panic (normal context, written by the panic hook)
//! contained-<sid>-<seq>.kv a panic day-core contained (renamed from pending by the observer)
//! java-<sid>.kv           an Android uncaught Java exception (written by the Java shim)
//! reports/report-<ms>.json finalized reports, rotated to `max_reports`
//! ```
//!
//! A crash means WillTerminate never ran, so the sentinel survives — its presence alongside a
//! handler-written artifact is what distinguishes a crash from a clean exit. A sentinel with NO
//! artifact means the session vanished without any handler firing (an OS kill / battery pull); that
//! is reported as an *unknown* end, never a crash.

// On wasm32 the capture machinery is never armed (`Config::init` is a graceful no-op there), so
// only the report queries and `SessionEnd` stay reachable — the rest is intentionally uncalled.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use std::path::{Path, PathBuf};

use crate::report::{Fields, Kind, Report, SignalInfo, parse_kv};

/// The resolved crash-report directory for `app_id`, honoring an explicit override. Mobile sandbox
/// roots are already per-app; desktop namespaces under the app id so two apps don't share a dir.
/// The same layout a day-lite superapp host uses for its package store.
pub fn store_dir(app_id: &str, override_dir: Option<&Path>) -> PathBuf {
    if let Some(d) = override_dir {
        return d.to_path_buf();
    }

    #[cfg(target_os = "android")]
    if let Some(base) = crate::java_android::files_dir() {
        return base.join("day-break");
    }

    #[cfg(all(target_os = "linux", target_env = "ohos"))]
    {
        // The OHOS sandbox is already per-app, so this path doesn't namespace by app id (the
        // desktop block below, which does, is unreachable here).
        let _ = app_id;
        for var in ["OHOS_APP_FILES_DIR", "HOME", "TMPDIR"] {
            if let Some(v) = std::env::var_os(var)
                && !v.is_empty()
            {
                return PathBuf::from(v).join("day-break");
            }
        }
        return PathBuf::from("/data/storage/el2/base/haps/entry/files/day-break");
    }

    // Desktop (and iOS/macOS): namespace under the app id. Computed here, AFTER the mobile
    // early-returns, so it isn't a dead binding on the sandboxed targets (ohos returns above).
    #[allow(unreachable_code)]
    {
        let slug = slug(app_id);
        if let Some(home) = std::env::var_os("HOME") {
            let base = PathBuf::from(home);
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            return base
                .join("Library/Application Support")
                .join(&slug)
                .join("day-break");
            #[allow(unreachable_code)]
            base.join(format!(".{slug}")).join("day-break")
        } else {
            std::env::temp_dir().join(slug).join("day-break")
        }
    }
}

/// Reduce an app id to a filesystem-safe directory name.
fn slug(app_id: &str) -> String {
    let mut s: String = app_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if s.is_empty() {
        s.push_str("day-break");
    }
    s
}

pub fn reports_subdir(dir: &Path) -> PathBuf {
    dir.join("reports")
}

fn sentinel_path(dir: &Path, sid: &str) -> PathBuf {
    dir.join(format!("session-{sid}.kv"))
}

pub fn sig_path(dir: &Path, sid: &str) -> PathBuf {
    dir.join(format!("sig-{sid}.sig"))
}

/// Write the session sentinel (static context) and pre-create the reports subdir. Called at init.
pub fn write_sentinel(dir: &Path, sid: &str, ctx: &[(&str, String)]) -> std::io::Result<()> {
    std::fs::create_dir_all(reports_subdir(dir))?;
    std::fs::write(sentinel_path(dir, sid), crate::report::write_kv(ctx))
}

/// Remove this session's sentinel + its empty raw-signal file (a clean exit — WillTerminate).
pub fn clear_session(dir: &Path, sid: &str) {
    let _ = std::fs::remove_file(sentinel_path(dir, sid));
    // The sig file is only meaningful when non-empty; on a clean exit it never got written.
    let p = sig_path(dir, sid);
    if std::fs::metadata(&p).map(|m| m.len() == 0).unwrap_or(false) {
        let _ = std::fs::remove_file(p);
    }
}

/// Write a `pending-<sid>-<seq>.kv` panic artifact and return its path (for a later downgrade).
pub fn write_pending(
    dir: &Path,
    sid: &str,
    seq: u64,
    fields: &[(&str, String)],
) -> std::io::Result<PathBuf> {
    let p = dir.join(format!("pending-{sid}-{seq}.kv"));
    std::fs::write(&p, crate::report::write_kv(fields))?;
    Ok(p)
}

/// Rename a pending panic artifact to `contained-*` (day-core caught it; the app lives).
pub fn downgrade_to_contained(pending: &Path) {
    if let Some(name) = pending.file_name().and_then(|n| n.to_str())
        && let Some(rest) = name.strip_prefix("pending-")
    {
        let dst = pending.with_file_name(format!("contained-{rest}"));
        let _ = std::fs::rename(pending, dst);
    }
}

// ---- reconcile -----------------------------------------------------------------------------

/// One session's grouped artifacts (paths only; content read lazily during finalize).
#[derive(Default)]
struct Group {
    sentinel: Option<PathBuf>,
    sig: Option<PathBuf>,
    pendings: Vec<PathBuf>,
    contained: Vec<PathBuf>,
    java: Option<PathBuf>,
}

/// Split a `kind-<sid>[-seq].ext` filename into (kind, sid). `sid` is `<pidhex>-<nanoshex>`, which
/// itself contains a dash, so we rejoin everything between the kind prefix and any trailing `-<seq>`.
fn parse_name(name: &str) -> Option<(&'static str, String)> {
    let (kind, rest, has_seq): (&'static str, &str, bool) = if let Some(r) = name
        .strip_prefix("session-")
        .and_then(|r| r.strip_suffix(".kv"))
    {
        ("session", r, false)
    } else if let Some(r) = name
        .strip_prefix("sig-")
        .and_then(|r| r.strip_suffix(".sig"))
    {
        ("sig", r, false)
    } else if let Some(r) = name
        .strip_prefix("pending-")
        .and_then(|r| r.strip_suffix(".kv"))
    {
        ("pending", r, true)
    } else if let Some(r) = name
        .strip_prefix("contained-")
        .and_then(|r| r.strip_suffix(".kv"))
    {
        ("contained", r, true)
    } else {
        let r = name
            .strip_prefix("java-")
            .and_then(|r| r.strip_suffix(".kv"))?;
        ("java", r, false)
    };
    let sid = if has_seq {
        // strip the trailing `-<seq>`
        rest.rsplit_once('-').map(|(head, _)| head).unwrap_or(rest)
    } else {
        rest
    };
    Some((kind, sid.to_string()))
}

/// Is `pid` a live process? Best-effort; used to skip a concurrently-running instance's sentinel.
fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // signal 0 probes existence without delivering; ESRCH ⇒ gone. EPERM ⇒ alive (not ours).
        let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if r == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// The end-state of the previous session, surfaced to the app via [`crate::last_session`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEnd {
    /// The previous session exited cleanly (WillTerminate ran), or there was no previous session.
    Clean,
    /// The previous session crashed; a finalized report is available.
    Crashed { kind: Kind, message: String },
    /// The previous session vanished with no handler firing — an OS kill / power loss. Not a crash.
    Unknown,
}

/// Result of a reconcile pass.
pub struct Reconciled {
    /// Paths written this pass — asserted by the store tests; the runtime reads only `last_session`
    /// (finalized reports are enumerated later via [`report_paths`]).
    #[allow(dead_code)]
    pub finalized: Vec<PathBuf>,
    pub last_session: SessionEnd,
}

/// Scan `dir`, finalize every stale session's artifacts into `reports/report-*.json`, delete the
/// intermediates, rotate to `max_reports`, and report the previous session's end-state. Skips
/// `own_sid` (the session just started) and any session whose sentinel names a live pid.
pub fn reconcile(
    dir: &Path,
    own_sid: &str,
    own_pid: u32,
    ctx: &StaticCtx,
    max_reports: usize,
    keep_contained: bool,
) -> Reconciled {
    let mut groups: std::collections::BTreeMap<String, Group> = std::collections::BTreeMap::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => {
            return Reconciled {
                finalized: vec![],
                last_session: SessionEnd::Clean,
            };
        }
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some((kind, sid)) = parse_name(name) else {
            continue;
        };
        if sid == own_sid {
            continue;
        }
        let g = groups.entry(sid).or_default();
        let path = entry.path();
        match kind {
            "session" => g.sentinel = Some(path),
            "sig" => g.sig = Some(path),
            "pending" => g.pendings.push(path),
            "contained" => g.contained.push(path),
            "java" => g.java = Some(path),
            _ => {}
        }
    }

    let mut finalized = Vec::new();
    // Track the newest previous session's end for last_session (by started_at from the sentinel).
    let mut newest_ms = 0u64;
    let mut last_end = SessionEnd::Clean;

    for (sid, g) in &groups {
        let sentinel_fields = g
            .sentinel
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| parse_kv(&t));

        // A live concurrent instance owns this sentinel — leave it alone.
        if let Some(f) = &sentinel_fields
            && let Some(pid) = f.get("pid").and_then(|s| s.parse::<u32>().ok())
            && pid != own_pid
            && pid_alive(pid)
        {
            continue;
        }

        let started_ms = sentinel_fields
            .as_ref()
            .and_then(|f| f.get("started_at_ms"))
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        let outcome = finalize_group(dir, sid, g, sentinel_fields.as_ref(), ctx, keep_contained);

        // Clean up this session's intermediates regardless of whether a report was produced.
        for p in g.pendings.iter().chain(g.contained.iter()) {
            let _ = std::fs::remove_file(p);
        }
        if let Some(p) = &g.sig {
            let _ = std::fs::remove_file(p);
        }
        if let Some(p) = &g.java {
            let _ = std::fs::remove_file(p);
        }
        if let Some(p) = &g.sentinel {
            let _ = std::fs::remove_file(p);
        }

        // Record end-state for the newest previous session.
        if started_ms >= newest_ms {
            newest_ms = started_ms;
            last_end = match &outcome {
                Some(report) if report.fatal => SessionEnd::Crashed {
                    kind: report.kind().unwrap_or(Kind::Panic),
                    message: report.message.clone(),
                },
                Some(_) => last_end.clone(), // non-fatal leftover from a clean session
                None if g.sentinel.is_some() => SessionEnd::Unknown, // sentinel alone → OS kill
                None => SessionEnd::Clean,
            };
        }

        if let Some(report) = outcome
            && let Ok(path) = write_report(dir, started_ms, &report)
        {
            finalized.push(path);
        }
    }

    rotate(dir, max_reports);
    Reconciled {
        finalized,
        last_session: last_end,
    }
}

/// Decide a session's kind/fatality from its artifacts and compose a [`Report`]. Returns `None`
/// when there is nothing to report (a clean session, or a sentinel-alone OS kill).
fn finalize_group(
    dir: &Path,
    sid: &str,
    g: &Group,
    sentinel: Option<&Fields>,
    ctx: &StaticCtx,
    keep_contained: bool,
) -> Option<Report> {
    let _ = dir;
    let sentinel_present = g.sentinel.is_some();

    // Precedence: signal > java > panic > contained. A non-empty sig file is a real fault.
    let sig_nonempty = g
        .sig
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len() > 0)
        .unwrap_or(false);

    let (kind, fatal, artifact): (Kind, bool, Option<Fields>) = if sig_nonempty {
        let f = g
            .sig
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| parse_kv(&t));
        (Kind::Signal, true, f)
    } else if let Some(jp) = &g.java {
        let f = std::fs::read_to_string(jp).ok().map(|t| parse_kv(&t));
        (Kind::Java, true, f)
    } else if let Some(pp) = newest(&g.pendings) {
        let f = std::fs::read_to_string(pp).ok().map(|t| parse_kv(&t));
        // Sentinel present ⇒ no clean exit ⇒ the panic killed the process. Absent ⇒ clean exit,
        // a leftover from a caught/background panic ⇒ non-fatal.
        (Kind::Panic, sentinel_present, f)
    } else {
        let cp = newest(&g.contained)?;
        if !keep_contained {
            return None;
        }
        let f = std::fs::read_to_string(cp).ok().map(|t| parse_kv(&t));
        (Kind::Contained, false, f)
    };

    Some(compose(sid, kind, fatal, sentinel, artifact.as_ref(), ctx))
}

/// The newest file in a list by modified-time (stable enough; filenames also carry a seq/ts).
fn newest(paths: &[PathBuf]) -> Option<&PathBuf> {
    paths
        .iter()
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

/// Static context that a report falls back to when a stale sentinel is missing fields (e.g. a very
/// old artifact). Mostly the CURRENT process's identity — good enough for the constant fields.
pub struct StaticCtx {
    pub app_id: String,
    pub app_version: String,
    pub app_build: String,
    pub day_version: String,
}

fn compose(
    sid: &str,
    kind: Kind,
    fatal: bool,
    sentinel: Option<&Fields>,
    artifact: Option<&Fields>,
    ctx: &StaticCtx,
) -> Report {
    let s = |key: &str, fallback: &str| -> String {
        sentinel
            .and_then(|f| f.get(key))
            .cloned()
            .unwrap_or_else(|| fallback.to_string())
    };
    let a = |key: &str| -> String {
        artifact
            .and_then(|f| f.get(key))
            .cloned()
            .unwrap_or_default()
    };
    let anum = |key: &str| -> u64 {
        artifact
            .and_then(|f| f.get(key))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };

    let signal = if kind == Kind::Signal {
        Some(SignalInfo {
            signo: anum("sig") as i64,
            name: signal_name(anum("sig") as i32),
            code: anum("code") as i64,
            addr: anum("addr") as usize,
            pc: anum("pc") as usize,
            slide: anum("slide").max(
                sentinel
                    .and_then(|f| f.get("slide"))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
            ) as usize,
        })
    } else {
        None
    };

    // uptime: signal handler records `up_ms`; panic hook records `uptime_ms`; else derive from ts.
    let uptime = {
        let from_artifact = artifact
            .and_then(|f| f.get("up_ms").or_else(|| f.get("uptime_ms")))
            .and_then(|v| v.parse::<u64>().ok());
        from_artifact.unwrap_or(0)
    };
    // A signal record marks the main thread via `main=1`; the panic hook records the thread name.
    let main_thread = a("main") == "1" || a("thread") == "main";

    Report {
        kind_str: kind.as_str().to_string(),
        fatal,
        app_id: s("app_id", &ctx.app_id),
        app_version: s("app_version", &ctx.app_version),
        app_build: s("app_build", &ctx.app_build),
        day_version: s("day_version", &ctx.day_version),
        backend: s("backend", ""),
        os_name: s("os_name", ""),
        os_version: s("os_version", ""),
        device_model: s("device_model", ""),
        simulator: s("simulator", "0") == "1",
        locale: s("locale", ""),
        session_id: sid.to_string(),
        started_at_ms: s("started_at_ms", "0").parse().unwrap_or(0),
        uptime_ms: uptime,
        message: a("message"),
        location: a("location"),
        thread: if a("thread").is_empty() {
            a("tid")
        } else {
            a("thread")
        },
        main_thread,
        signal,
        backtrace_text: a("backtrace"),
    }
}

fn write_report(dir: &Path, started_ms: u64, report: &Report) -> std::io::Result<PathBuf> {
    let reports = reports_subdir(dir);
    std::fs::create_dir_all(&reports)?;
    // Prefer the session start ts for a stable, sortable name; fall back to a monotonic-ish counter.
    let stamp = if started_ms > 0 {
        started_ms
    } else {
        report.uptime_ms
    };
    let mut path = reports.join(format!("report-{stamp}.json"));
    // Avoid clobbering if two sessions share a start ms (rare) — append the session id.
    if path.exists() {
        path = reports.join(format!("report-{stamp}-{}.json", report.session_id));
    }
    std::fs::write(&path, report.to_json())?;
    Ok(path)
}

/// Keep the newest `max` finalized reports (by filename ts), delete the rest.
pub fn rotate(dir: &Path, max: usize) {
    let reports = reports_subdir(dir);
    let mut files: Vec<PathBuf> = match std::fs::read_dir(&reports) {
        Ok(e) => e
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
            .collect(),
        Err(_) => return,
    };
    if files.len() <= max {
        return;
    }
    // Sort oldest-first by modified time, drop the excess from the front.
    files.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    let excess = files.len() - max;
    for p in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(p);
    }
}

/// Paths of finalized reports, newest-first.
pub fn report_paths(dir: &Path) -> Vec<PathBuf> {
    let reports = reports_subdir(dir);
    let mut files: Vec<PathBuf> = match std::fs::read_dir(&reports) {
        Ok(e) => e
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
            .collect(),
        Err(_) => return vec![],
    };
    files.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok()));
    files
}

/// The canonical POSIX signal name for a number (for report readability).
pub fn signal_name(signo: i32) -> String {
    let n = match signo {
        4 => "SIGILL",
        6 => "SIGABRT",
        7 => "SIGBUS",
        8 => "SIGFPE",
        11 => "SIGSEGV",
        5 => "SIGTRAP",
        _ => "SIG?",
    };
    n.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A positive pid that is (almost certainly) not a running process. NOT u32::MAX — that casts to
    // pid_t -1, and `kill(-1, 0)` targets every process and reports "alive".
    const DEAD_PID: u32 = 0x7FFF_FFFE;

    fn ctx() -> StaticCtx {
        StaticCtx {
            app_id: "dev.test.app".into(),
            app_version: "1.2.3".into(),
            app_build: "7".into(),
            day_version: "0.0.14".into(),
        }
    }

    fn sentinel(dir: &Path, sid: &str, pid: u32, started: u64) {
        write_sentinel(
            dir,
            sid,
            &[
                ("pid", pid.to_string()),
                ("started_at_ms", started.to_string()),
                ("app_id", "dev.test.app".into()),
                ("app_version", "1.2.3".into()),
                ("app_build", "7".into()),
                ("day_version", "0.0.14".into()),
                ("backend", "macos-appkit".into()),
                ("os_name", "macOS".into()),
                ("os_version", "15.0".into()),
                ("locale", "en".into()),
            ],
        )
        .unwrap();
    }

    #[test]
    fn parse_name_splits_kind_and_sid_with_seq() {
        assert_eq!(
            parse_name("session-1a2b-ff.kv"),
            Some(("session", "1a2b-ff".into()))
        );
        assert_eq!(
            parse_name("pending-1a2b-ff-3.kv"),
            Some(("pending", "1a2b-ff".into()))
        );
        assert_eq!(
            parse_name("sig-1a2b-ff.sig"),
            Some(("sig", "1a2b-ff".into()))
        );
        assert_eq!(
            parse_name("java-1a2b-ff.kv"),
            Some(("java", "1a2b-ff".into()))
        );
        assert_eq!(parse_name("unrelated.txt"), None);
    }

    #[test]
    fn sentinel_alone_is_unknown_not_a_crash() {
        let d = tempdir();
        // A dead pid (init is 1; use a very high pid unlikely to exist). Use pid 0 sentinel trick:
        // pid_alive(0) — kill(0,..) targets our group, so use u32::MAX which is never a real pid.
        sentinel(&d, "dead-1", DEAD_PID, 1000);
        let r = reconcile(&d, "ownsid", std::process::id(), &ctx(), 5, true);
        assert_eq!(r.last_session, SessionEnd::Unknown);
        assert!(r.finalized.is_empty());
        // The stale sentinel is cleaned up.
        assert!(!sentinel_path(&d, "dead-1").exists());
    }

    #[test]
    fn panic_with_sentinel_is_fatal() {
        let d = tempdir();
        sentinel(&d, "dead-2", DEAD_PID, 2000);
        write_pending(
            &d,
            "dead-2",
            1,
            &[
                ("message", "boom".into()),
                ("location", "x.rs:1:1".into()),
                ("thread", "main".into()),
                ("uptime_ms", "42".into()),
            ],
        )
        .unwrap();
        let r = reconcile(&d, "ownsid", std::process::id(), &ctx(), 5, true);
        match r.last_session {
            SessionEnd::Crashed { kind, message } => {
                assert_eq!(kind, Kind::Panic);
                assert_eq!(message, "boom");
            }
            other => panic!("expected Crashed, got {other:?}"),
        }
        assert_eq!(r.finalized.len(), 1);
        let json = std::fs::read_to_string(&r.finalized[0]).unwrap();
        assert!(json.contains(r#""kind":"panic""#));
        assert!(json.contains(r#""fatal":true"#));
        assert!(json.contains(r#""boom""#));
        assert!(json.contains(r#""backend":"macos-appkit""#));
    }

    #[test]
    fn pending_without_sentinel_is_non_fatal() {
        let d = tempdir();
        // No sentinel = clean exit; a leftover pending is a caught/background panic.
        write_pending(&d, "clean-1", 1, &[("message", "bg".into())]).unwrap();
        let r = reconcile(&d, "own", std::process::id(), &ctx(), 5, true);
        assert_eq!(r.finalized.len(), 1);
        let json = std::fs::read_to_string(&r.finalized[0]).unwrap();
        assert!(json.contains(r#""fatal":false"#));
    }

    #[test]
    fn signal_beats_pending_and_is_fatal() {
        let d = tempdir();
        sentinel(&d, "dead-3", DEAD_PID, 3000);
        write_pending(&d, "dead-3", 1, &[("message", "panic-first".into())]).unwrap();
        std::fs::write(
            sig_path(&d, "dead-3"),
            "sig=11\ncode=1\naddr=0\npc=4096\nmain=1\nup_ms=99\n",
        )
        .unwrap();
        let r = reconcile(&d, "own", std::process::id(), &ctx(), 5, true);
        assert_eq!(r.finalized.len(), 1);
        let json = std::fs::read_to_string(&r.finalized[0]).unwrap();
        assert!(json.contains(r#""kind":"signal""#));
        assert!(json.contains(r#""signo":11"#));
        assert!(json.contains(r#""pc":4096"#));
    }

    #[test]
    fn contained_dropped_when_keep_contained_false() {
        let d = tempdir();
        std::fs::write(d.join("contained-c1-1.kv"), "message=caught\n").unwrap();
        let r = reconcile(&d, "own", std::process::id(), &ctx(), 5, false);
        assert!(r.finalized.is_empty());
    }

    #[test]
    fn rotation_keeps_newest_n() {
        let d = tempdir();
        let reports = reports_subdir(&d);
        std::fs::create_dir_all(&reports).unwrap();
        for i in 0..8 {
            let p = reports.join(format!("report-{i}.json"));
            std::fs::write(&p, "{}").unwrap();
            // stagger mtimes
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        rotate(&d, 3);
        let remaining = report_paths(&d);
        assert_eq!(remaining.len(), 3);
    }

    // Minimal temp-dir helper (no tempfile dep): a unique dir under the OS temp dir, per test.
    fn tempdir() -> PathBuf {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("day-break-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
