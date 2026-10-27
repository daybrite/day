---
title: "Crash reporting"
description: "Capture crashes, review pending reports, and send them with the user’s consent."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Crash reporting

A crash report records information about an unexpected app failure so developers can investigate it.
A review-and-send step lets people decide whether to share that diagnostic information.

`day-piece-crash-reporter` records crashes locally and provides a review-and-send interface for the
next launch. Your app chooses the destination and when to request consent.

## Add to your app

Add [`day-piece-crash-reporter`](https://crates.io/crates/day-piece-crash-reporter) to your app:

```sh
cargo add day-piece-crash-reporter
```

[Crate](https://crates.io/crates/day-piece-crash-reporter) ·
[API reference](https://docs.rs/day-piece-crash-reporter/latest/day_piece_crash_reporter/) ·
[Source](https://github.com/daybrite/day-piece-break).
The default `ui` feature includes the consent interface. Use `default-features = false` for
capture and transports with your own interface. No per-toolkit feature is required.

## Initialize capture

Initialize once before launching the app. This API excerpt uses the app identity supplied by
Day's build and retains at most five finalized reports, the default limit.

```rust
use day_piece_crash_reporter::{Config, InitError};

pub fn initialize_reports() -> Result<(), InitError> {
    Config::new().max_reports(5).init()
}
```

[`Config::init()`](https://docs.rs/day-piece-crash-reporter/latest/day_piece_crash_reporter/struct.Config.html#method.init)
reconciles the previous session and installs capture hooks. A second initialization returns
an error. The report directory defaults to the app's data directory. Keep reports out of shared
or public storage and set a retention limit appropriate to your app.

[`last_session()`](https://docs.rs/day-piece-crash-reporter/latest/day_piece_crash_reporter/fn.last_session.html)
distinguishes known crashes from an unclean exit. An unclean exit can also result from an OS kill
or power loss; do not describe every such exit as a captured crash.

## Review and send reports

Add [`consent_banner()`](https://docs.rs/day-piece-crash-reporter/latest/day_piece_crash_reporter/fn.consent_banner.html)
to the app's interface after initialization. It lets the person inspect, send, or discard pending
reports. Configure a transport with
[`Config::reporter()`](https://docs.rs/day-piece-crash-reporter/latest/day_piece_crash_reporter/struct.Config.html#method.reporter)
before initialization; no upload destination is configured by default.

| Transport | Intended destination |
| --- | --- |
| [`RestReporter`](https://docs.rs/day-piece-crash-reporter/latest/day_piece_crash_reporter/struct.RestReporter.html) | An app-owned HTTP endpoint. |
| [`GithubIssueReporter`](https://docs.rs/day-piece-crash-reporter/latest/day_piece_crash_reporter/struct.GithubIssueReporter.html) | A GitHub issue submission flow. |
| [`EmailReporter`](https://docs.rs/day-piece-crash-reporter/latest/day_piece_crash_reporter/struct.EmailReporter.html) | The person's email client. |
| [`Reporter`](https://docs.rs/day-piece-crash-reporter/latest/day_piece_crash_reporter/trait.Reporter.html) | Your own submission implementation. |

For a custom interface,
[`pending()`](https://docs.rs/day-piece-crash-reporter/latest/day_piece_crash_reporter/fn.pending.html)
provides a signal of report metadata.
[`report_text()`](https://docs.rs/day-piece-crash-reporter/latest/day_piece_crash_reporter/fn.report_text.html)
loads a selected report, while
[`send()`](https://docs.rs/day-piece-crash-reporter/latest/day_piece_crash_reporter/fn.send.html)
and [`discard()`](https://docs.rs/day-piece-crash-reporter/latest/day_piece_crash_reporter/fn.discard.html)
operate on that selection. Sending completes through a `Send` callback: pass results through a
[Setter](/docs/explanation/async) before updating UI state. Handle submission errors and retain
the person's ability to discard a report.

## Privacy and capture limits

Reports can contain panic messages, paths, and app context. Use
[`Config::redact()`](https://docs.rs/day-piece-crash-reporter/latest/day_piece_crash_reporter/struct.Config.html#method.redact)
to scrub panic messages before persistence. This hook is not a general guarantee that every
report field is sanitized; inspect the full report and your custom metadata before enabling
submission. Do not embed a privileged service credential in the app.

| Platform | Capture |
| --- | --- |
| macOS / iOS / Linux / HarmonyOS | Rust panics and native Unix faults. Native fault capture does not guarantee a complete stack trace. |
| Android | Rust panics, Unix faults, and uncaught Java exceptions. |
| Windows | Rust panics. Native Windows exception capture is not implemented. |
| Web | Initialization is a no-op and queries return empty state. |

A fatal-fault handler has strict execution limits. Keep app crash hooks minimal; present UI and
perform network requests on a later launch, not inside a crash handler.

## Migrating from day-break

Replace the old dependency with `day-piece-crash-reporter` and rename Rust references from
`day_break::` to `day_piece_crash_reporter::`. Existing schema-1 reports and the `day-break`
storage directory remain readable. `day diagnose` continues reading those files.
