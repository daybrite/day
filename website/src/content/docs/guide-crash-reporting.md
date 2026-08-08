---
title: Report crashes
description: "Capture panics and native faults with day-break, show the user the exact report on the next launch, and upload it only when they choose to send."
order: 34
section: Guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

When a Day app dies (a Rust panic, a segfault, an abort), day-break writes a report, and on
the next launch your app shows the user what was recorded and asks whether to send it. It
registers a chained panic hook and native signal handlers, and uploads through a transport you
choose: a REST endpoint, a prefilled GitHub issue, or an email the user sends. There is no
auto-upload mode; the only network path is `send`, called by your code from a user action,
after the user has read the report. Arming it is one call before the UI mounts:

```rust
day_break::Config::new()
    .reporter(day_break::EmailReporter::new("crashes@example.dev"))
    .init()
    .ok();
```

**Works on:** Rust panics are captured on every native target. Native faults (SIGSEGV, SIGBUS,
SIGILL, SIGFPE, SIGABRT, SIGTRAP) are caught on the Unix targets (macOS, iOS, Linux, Android,
and HarmonyOS), and Android also records uncaught Java exceptions. Windows records panics but
not native faults yet, and on the web `init` is a graceful no-op. The full matrix is in
[the break reference](/docs/internal/break).

## 1. Arm capture before the UI mounts

Add the crate and call `init` as early as possible: before `day::launch`, so a crash during
startup is still recorded:

```toml
[dependencies]
day-break = { git = "https://github.com/daybrite/day.git" }
```

The showcase wraps it in a helper called from every entry point:

```rust
/// Arm crash reporting. Idempotent (day-break's `init` is single-shot); safe to call from
/// every entry point.
pub fn install_crash_reporting() {
    let _ = day_break::Config::new()
        // "Send report" opens a prefilled email to the developer (no server needed).
        .reporter(day_break::EmailReporter::new("crashdemo@daybrite.dev"))
        .init();
}
```

Crash capture is process-global, so `init` is single-shot: a second call returns
`InitError::AlreadyInitialized`. That's why the helper ignores the result: calling it from
both `main` and a mobile entry point is safe.

The builder has a few more knobs, with the defaults in parentheses: `.max_reports(n)` caps the
report rotation (5), `.keep_contained(false)` drops reports for panics day-core contained
(kept by default), `.signals(false)` turns off the native signal handlers (on by default), and
`.redact(|msg| …)` scrubs secrets from panic messages before they are persisted, displayed, or
uploaded. App identity (id, version, build) is baked in by `day build` from `Day.toml`;
`.app_id(…)`, `.app_version(…)`, and `.app_build(…)` override it.

## 2. What gets captured

Three crash classes produce reports: a Rust panic (the panic hook), a native fault or abort
(the signal handlers), and, on Android, an uncaught Java exception. A panic that day-core
contains at its trampoline boundaries (the app survives) is recorded too, as a distinct
non-fatal report, so you also see the almost-crashes.

A report is versioned JSON: app id, version, and build; the day version and backend; OS,
device model, and locale; the session id and uptime; the panic message and source location, or
the signal's number and addresses; and a backtrace. Nothing else: the schema in
[the reference](/docs/internal/break) lists every field, and there is no user data beyond
them. The signal handlers chain to the previous disposition, so the platform's own crash
reporter (Android tombstones, HarmonyOS faultlogs) still runs alongside.

## 3. Show the report on the next launch

`init` reconciles the previous session before your UI mounts, so by the time it builds you can
ask what happened:

```rust
match day_break::last_session() {
    day_break::SessionEnd::Crashed { .. } => show_crash_prompt(), // your UI, or consent_banner()
    day_break::SessionEnd::Unknown => {} // an OS kill — not a crash; usually ignore
    day_break::SessionEnd::Clean => {}
}
```

The ready-made surface is `day_break::consent_banner()` from the `ui` feature (on by
default): a piece that appears while reports are pending, shows the full report text, and
offers send and discard. To build your own (the showcase's Crash Reporting page does),
compose the queries: `pending()` is a reactive `Signal<Vec<ReportMeta>>`, newest first;
`report_text(&meta)` is the full text, which is what the transport sends;
`reporter_description()` is the transport's one-line disclosure; `send(&meta, |result| …)`
uploads; `discard(&meta)` deletes. The showcase keeps its viewer current with one effect:

```rust
let report = Signal::new(String::new());
let pending = day_break::pending();
Effect::new(move || {
    pending.get(); // track
    report.set(day_break::latest_report_text().unwrap_or_default());
});
```

Whatever surface you build, the consent rule holds: show the user the report before offering
"Send", and call `send` only from that action.

## 4. Pick a reporter

Three transports ship with the crate:

- **`RestReporter::new(url)`** POSTs the report JSON via `day-part-http`, off the UI thread;
  `.named("our crash server")` sets the name shown to the user. It's also the shape a
  GitHub-issue proxy takes: a small server accepts the JSON and opens the issue with your repo
  token held server-side, never on the device.
- **`GithubIssueReporter::new(owner, repo)`** opens a prefilled new-issue page in the browser;
  the user reviews and submits it themselves. No server needed.
- **`EmailReporter::new(to)`** opens a prefilled `mailto:` compose (`.subject_prefix(…)` tags
  the subject); the user sends the mail.

Or implement the trait yourself:

```rust
pub trait Reporter: Send + Sync {
    fn name(&self) -> &str;          // shown on the consent surface
    fn describe(&self) -> String;    // one-line disclosure: where the report goes
    fn send(&self, report: &Report, done: Box<dyn FnOnce(Result<(), SendError>) + Send>);
}
```

The browser and email transports finish with `SendError::HandedOff` — they handed the report
to the platform and can't confirm delivery. It's reported so your UI can say so, not a
failure.

## Pitfalls

- **Arm first.** The hook can't record a crash that happens before `init` runs, and `init` is
  also what reconciles the previous session — call it at the top of the app entry, before
  `day::launch`.
- **Release backtraces carry symbols, not lines.** The release profile ships no debug info by
  default. For `file:line` in release reports, add `[profile.release] debug =
  "line-tables-only"` in your own workspace; day-break doesn't change the global profile. For
  native faults, `signal.pc - signal.slide` is the module-relative address to symbolize
  offline.
- **`Unknown` is not a crash.** A leftover session with no crash artifact — an OS kill, power
  loss — reconciles as `SessionEnd::Unknown`, never `Crashed`. Don't show crash UI for it.
- **Not every crash class is caught everywhere.** Windows native faults, the iOS
  Objective-C exception handler, and HarmonyOS `errorManager` are deferred to a later version;
  on iOS an uncaught ObjC exception ends in `abort()`, which the SIGABRT handler does record.

## Reference

[break](/docs/internal/break) — the whole design: the report schema, signal-handler
discipline, the session sentinel, app identity and symbolication, transports, and how it's
tested.
