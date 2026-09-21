---
title: "day-piece-break: crash reporting"
description: "The external crash capture, report review, and submission Piece."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-piece-break: crash reporting

Crash reporting lives in the standalone [day-piece-break repository](https://github.com/daybrite/day-piece-break).
The [package reference](https://github.com/daybrite/day-piece-break/blob/main/docs/break.md)
covers capture, the report schema, transports, consent UI, platform limitations, and tests.

```toml
[dependencies]
day-piece-break = { git = "https://github.com/daybrite/day-piece-break" }
```

Initialize `day_piece_break::Config` before launching the app. Its default `ui` feature provides
`consent_banner()` for reviewing, sending, and discarding pending reports. Set
`default-features = false` to use capture, storage, and transports with your own interface.
The UI composes existing Day pieces and needs no per-backend feature flags.

## Migrating from day-break

Replace the old dependency on `day.git` with the line above and rename Rust references from
`day_break::` to `day_piece_break::`. To preserve existing Rust source temporarily, use a Cargo alias:

```toml
day-break = { package = "day-piece-break", git = "https://github.com/daybrite/day-piece-break" }
```

Existing reports remain readable: the `day-break` storage directory, schema-1 report format,
`DAY_BREAK_*` environment variables, Android Java/JNI names, and `dbreak-*` UI IDs are unchanged.
`day diagnose` continues reading the same files. `day.version` now reads the linked framework's
`day_core::VERSION`, independently of the reporter's release version.
