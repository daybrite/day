# day-break: consent-first crash reporting (normative)

> **Status: implemented** (`crates/day-break`). The panic hook, POSIX signal handlers, session
> sentinel, next-launch reconciliation, the report schema + JSON round-trip, the app-identity
> plumbing, the pluggable transports, and the `ui` consent surface (localized en/fr/ar/zh-CN) are
> built and tested (unit + a subprocess crash harness + a mock-backend banner e2e, on macOS/Linux
> hosts). The Android uncaught-exception layer and on-device verification track in DESIGN.md §8.5.
> This file is the normative reference for the whole design.

day-break is Day's answer to Crashlytics / Sentry / Bugsnag / Backtrace, built on one principle:
**the user is fully informed and nothing leaves the device without an explicit action.** It
registers standard crash handlers, writes a report when the app dies abnormally, and on the *next*
launch lets the app show the user exactly what would be sent and ask whether to send it, through
a transport the app chooses (a REST endpoint, a GitHub-issue flow, or a `mailto:` the user sends).

It is an **optional** crate: an app that doesn't depend on it is unaffected. It is a framework
crate (`crates/day-break`), not a `parts/day-part-*`, because its consent surface is a Day piece
and parts may not depend on `day-pieces`.

## Using it

Arm capture as early as possible (the first thing in the app entry, before `day::launch`):

```rust
day_break::Config::new()
    .max_reports(5)                    // rotation (default 5)
    .redact(|msg| { /* scrub secrets from panic messages */ })
    .init()
    .ok();

day::launch(WindowOptions::default(), app_root);
```

On the next launch, surface any pending report and let the user decide:

```rust
match day_break::last_session() {
    day_break::SessionEnd::Crashed { .. } => show_crash_prompt(),  // your UI, or consent_banner()
    day_break::SessionEnd::Unknown => { /* an OS kill — not a crash; usually ignore */ }
    day_break::SessionEnd::Clean => {}
}
```

`report_paths()` returns the finalized reports newest-first; each is the schema-versioned JSON
below. `latest_report_text()` returns the newest report's content for display. `discard(path)`
removes one. The `send()` / consent-surface API (the only network path) is documented with the
transports section.

### The consent rule

There is **no auto-upload mode** in v1. Upload happens only through a transport's `send`, which is
called by app code. The intended trigger is a user action ("Send report") on a disclosure surface
that has shown the full report text. The report the user reads is byte-for-byte what is uploaded;
there are no hidden fields. A conforming app does not call `send` from a non-interactive path.

## What is captured

| Crash class | Mechanism | Platforms (v1) |
|---|---|---|
| Rust panic (fatal or day-core-contained) | chained `std::panic::set_hook` + `std::backtrace` | all |
| Native fault / abort (SIGSEGV/SIGBUS/SIGILL/SIGFPE/SIGABRT/SIGTRAP) | `sigaction` handlers | all Unix (macOS, iOS, Linux, Android, HarmonyOS) |
| Uncaught Java exception | `Thread.setDefaultUncaughtExceptionHandler` (Android shim) | Android |

**Deferred to a later version** (listed so the limits are on record): Windows
`SetUnhandledExceptionFilter` (the panic hook still covers Rust panics, the dominant class); iOS
`NSSetUncaughtExceptionHandler` (an uncaught ObjC exception ends in `abort()`, which the SIGABRT
handler already records); HarmonyOS `errorManager` (ArkTS-only, no NDK C entry; native crashes
are covered by the signal handlers).

### Panic vs. contained panic

day-core contains panics at its trampoline boundaries (the event pump, posted main-thread tasks)
so the app survives (DESIGN.md §8.5, §8). day-break's hook fires for *every* panic, then day-core
notifies it (`day_core::set_contained_panic_observer`) when it catches one, so a contained panic
is recorded as a **non-fatal** `contained` report, distinct from a crash. Set
`Config::keep_contained(false)` to drop them.

### Signal-handler discipline

A signal handler is async-signal-safe: no allocation, no locks. Everything risky happens at
`init` (open the report fd, allocate the alternate stack, capture the ASLR slide and monotonic
epoch, save previous dispositions). The handler only formats integers into a fixed stack buffer,
`write(2)`s them, and **chains**: it restores the previous disposition and either re-raises
(abort/trap) or returns so the faulting instruction re-executes and the OS crash reporter still
runs. This etiquette preserves Android ART's `libsigchain` and HarmonyOS FaultLoggerd, so the
platform's own tombstone/faultlog is still produced alongside day-break's report.

## The report (schema 1)

A finalized report is JSON, grow-only (fields are added, never removed or repurposed; key off
`schema`). Intermediate on-disk artifacts use a line-oriented `key=value` format instead. The
signal handler can only emit ASCII from its constrained context, so nothing in the runtime path
parses JSON; reconciliation on the next launch turns the kv artifacts into the JSON below.

```json
{
  "schema": 1,
  "kind": "panic | signal | java | contained",
  "fatal": true,
  "app": { "id": "dev.example.app", "version": "1.2.0", "build": "42" },
  "day": { "version": "0.0.14", "backend": "ios-uikit" },
  "os": { "name": "iOS", "version": "18.0" },
  "device": { "model": "iPhone", "simulator": true },
  "locale": "en",
  "session": { "id": "1a2b-...", "started_at_ms": 0, "uptime_ms": 0 },
  "message": "…", "location": "src/x.rs:10:4",
  "thread": { "name": "main", "main": true },
  "signal": { "signo": 11, "name": "SIGSEGV", "code": 1, "addr": 0, "pc": 0, "slide": 0 },
  "backtrace_text": "…"
}
```

No user data beyond the fields listed. `signal` is present only for `kind: "signal"`.

### App identity and symbolication

Day.toml's `[app]` id/version/build are not otherwise in the binary, so `day build`/`day launch`
export them (`DAY_APP_ID`/`DAY_APP_VERSION`/`DAY_APP_BUILD`) and day-break's `build.rs` bakes them
in; a bare `cargo` build without the CLI falls back to a runtime env lookup, then `"unknown"`.
`day.version` is day-break's own crate version; `day.backend` is `day_core::backend_name()`.

For a native fault, `signal.pc - signal.slide` is the module-relative address to symbolize offline
against the shipped binary. The release profile ships no debug info by default, so backtraces
carry symbol names but not `file:line`; an app that wants line tables in release adds, in **its
own** workspace, `[profile.release] debug = "line-tables-only"` (day-break does not change the
global profile).

## Session end and the sentinel

At `init` day-break writes a session sentinel and removes it on `Lifecycle::WillTerminate` (a
clean exit). WillTerminate is reliable on desktop but best-effort on mobile (it does not fire on a
crash or an OS kill), so the reconciler treats a leftover sentinel **with no handler-written
artifact** as `SessionEnd::Unknown` (an OS kill or power loss), never a crash. Only a
handler-written artifact produces a crash report.

## Transports (`Reporter`)

Upload is pluggable. A transport implements:

```rust
pub trait Reporter {
    fn name(&self) -> &str;          // shown on the consent surface
    fn describe(&self) -> String;    // one-line disclosure: where the report goes
    fn send(&self, report: &Report, done: Box<dyn FnOnce(Result<(), SendError>)>);
}
```

Built-ins:

- **`RestReporter`** — POSTs the report JSON to a URL via the native HTTP stack
  (`day-part-http`), off the UI thread. This is also the shape a **GitHub-issue proxy** takes: run
  a small server that accepts the report JSON and opens an issue with your repo token held
  server-side (never on the device); the device just POSTs to it.
- **`GithubIssueReporter`** — zero-infrastructure: opens
  `https://github.com/<owner>/<repo>/issues/new?title=…&body=…` (truncated to URL limits) in the
  browser via the `open_url` toolkit duty; the user reviews and submits the issue themselves.
- **`EmailReporter`** — opens `mailto:dev@example.com?subject=…&body=…` (also `open_url`); the
  user sends the mail. Body is truncated; the full report is attached/inlined per app choice.

All three keep the human in the loop; `GithubIssueReporter` and `EmailReporter` require zero
backend and hand the final submit to the user.

## Testing

`cargo test -p day-break` covers the kv/JSON codecs, the reconcile matrix, rotation, and panic-hook
chaining in-process, plus a **subprocess crash harness** (`tests/crash.rs`) that re-executes the
test binary to actually panic / abort / segfault a child and asserts the finalized report (run on
macOS and Linux CI hosts). On-device capture (Android UEH, iOS/HarmonyOS signals) is verified
through the showcase "Crash Reporting" page and its dayscript (`docs/agent.md`), which uses the
`expect_exit` step to tolerate the intentional crash.
