// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Subprocess crash harness: the real end-to-end path. A real crash needs a real process — you
//! cannot segfault the test runner and survive — so each crash runs in a CHILD spawned as
//! `current_exe()` filtered to the single [`child_entry`] test. The child reads `DAY_BREAK_TEST_MODE`,
//! arms day-break at `DAY_BREAK_TEST_DIR`, and crashes (or reconciles + prints). The parent asserts
//! the child died the expected way, then runs a `reconcile` child and asserts the finalized report.
//!
//! Note on the panic case: libtest wraps each test in `catch_unwind`, so a child `panic!` is caught
//! and the child exits 101 — but our panic HOOK still runs first and writes the pending artifact,
//! and the sentinel is never cleared (no lifecycle in a test), so reconcile still sees a fatal panic.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The single entry point the parent invokes in a child (`--exact child_entry`). In the parent's
/// own run `DAY_BREAK_TEST_MODE` is unset, so this returns immediately and passes trivially.
#[test]
fn child_entry() {
    let Ok(mode) = std::env::var("DAY_BREAK_TEST_MODE") else {
        return;
    };
    let dir = std::env::var("DAY_BREAK_TEST_DIR").expect("child needs DAY_BREAK_TEST_DIR");
    day_break::Config::new()
        .dir(&dir)
        .app_id("dev.test.break")
        .init()
        .expect("child init");

    match mode.as_str() {
        "panic" => panic!("intentional test panic"),
        "abort" => std::process::abort(),
        "segv" => unsafe {
            let p = std::hint::black_box(std::ptr::null_mut::<u8>());
            std::ptr::write_volatile(p, 1u8);
            unreachable!("segv did not fault");
        },
        "reconcile" => {
            let ls = day_break::last_session();
            let n = day_break::report_paths().len();
            println!("RECONCILE kind={} reports={n}", describe(&ls));
            let json = day_break::report_paths()
                .first()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_default();
            println!("JSON {}", json.replace('\n', " "));
            std::io::Write::flush(&mut std::io::stdout()).ok();
            std::process::exit(0);
        }
        other => panic!("unknown DAY_BREAK_TEST_MODE={other}"),
    }
}

fn describe(ls: &day_break::SessionEnd) -> String {
    match ls {
        day_break::SessionEnd::Clean => "clean".into(),
        day_break::SessionEnd::Unknown => "unknown".into(),
        day_break::SessionEnd::Crashed { kind, .. } => format!("crashed:{}", kind.as_str()),
    }
}

fn run_child(mode: &str, dir: &Path) -> Output {
    Command::new(std::env::current_exe().unwrap())
        .args(["child_entry", "--exact", "--nocapture", "--test-threads=1"])
        .env("DAY_BREAK_TEST_MODE", mode)
        .env("DAY_BREAK_TEST_DIR", dir)
        .env("RUST_BACKTRACE", "0")
        .output()
        .expect("spawn child")
}

fn reconcile(dir: &Path) -> (String, String) {
    let out = run_child("reconcile", dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // libtest's `--nocapture` streams the test's println AFTER "test child_entry ... " on the same
    // line (no newline), so the marker lands mid-line — match it anywhere, not just at line start.
    let after = |marker: &str| -> String {
        stdout
            .lines()
            .find_map(|l| l.find(marker).map(|i| l[i + marker.len()..].to_string()))
            .unwrap_or_default()
    };
    (after("RECONCILE "), after("JSON "))
}

struct Scratch(PathBuf);
impl Scratch {
    fn new(tag: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!("day-break-crash-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn panic_child_produces_fatal_panic_report() {
    let d = Scratch::new("panic");
    let out = run_child("panic", &d.0);
    assert!(!out.status.success(), "panic child should exit non-zero");
    let (kind, json) = reconcile(&d.0);
    assert!(kind.contains("crashed:panic"), "last_session: {kind}");
    assert!(json.contains(r#""kind":"panic""#), "json: {json}");
    assert!(json.contains(r#""fatal":true"#), "json: {json}");
    assert!(
        json.contains("intentional test panic"),
        "message missing: {json}"
    );
}

#[test]
#[cfg(unix)]
fn abort_child_produces_fatal_signal_report() {
    use std::os::unix::process::ExitStatusExt;
    let d = Scratch::new("abort");
    let out = run_child("abort", &d.0);
    assert_eq!(out.status.signal(), Some(libc::SIGABRT), "expected SIGABRT");
    let (kind, json) = reconcile(&d.0);
    assert!(kind.contains("crashed:signal"), "last_session: {kind}");
    assert!(json.contains(r#""kind":"signal""#), "json: {json}");
    assert!(
        json.contains(r#""signo":6"#),
        "expected SIGABRT signo=6: {json}"
    );
}

#[test]
#[cfg(unix)]
fn segv_child_produces_fatal_signal_report() {
    use std::os::unix::process::ExitStatusExt;
    let d = Scratch::new("segv");
    let out = run_child("segv", &d.0);
    let sig = out.status.signal();
    assert!(
        matches!(sig, Some(libc::SIGSEGV) | Some(libc::SIGBUS)),
        "expected SIGSEGV/SIGBUS, got {sig:?}"
    );
    let (kind, json) = reconcile(&d.0);
    assert!(kind.contains("crashed:signal"), "last_session: {kind}");
    assert!(json.contains(r#""kind":"signal""#), "json: {json}");
}
