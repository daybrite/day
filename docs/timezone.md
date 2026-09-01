---
title: "Time zones"
description: "Wall-clock time and time-zone changes as reactive values via day-part-timezone."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Wall clock & time zones (headless capability crate)

> **Status: implemented** as `day-part-timezone` (in `parts/`, the headless counterpart of
> `pieces/`). A shared cross-platform API for the host's **wall clock** and **time-zone facts**:
> what time is it, what zone is this device in, and what is zone X's UTC offset at instant T
> (DST-correct). Offsets come from a bundled IANA database (jiff, `tzdb-bundle-always`), so every
> target answers from the same rules with no OS zoneinfo required.

Day's core is zoneless: day-l10n's `DATETIME` renders an epoch as **UTC civil time**,
and day-piece-datetime edits zoneless values ([docs/datepicker.md](datepicker.md)). That default suits
forms and timestamps; a world clock or an alarm needs real zone arithmetic, which this crate
supplies. The core's model is unchanged: the API stays in instants and offsets, and rendering
stays in Fluent.

## Authoring

```rust
let at = day_part_timezone::now();

// A world-clock row: Tokyo's current UTC offset, DST-correct.
let tokyo = day_part_timezone::offset_seconds("Asia/Tokyo", at);

// The device's own zone and offset.
let zone = day_part_timezone::local_zone();          // Some("Europe/Paris")
let local = day_part_timezone::local_offset_seconds(at);
```

| Function | Answers |
|---|---|
| `now() -> SystemTime` | the wall clock: `SystemTime::now()` everywhere except `wasm32`, where std has no clock and the day-dom shim answers `Date.now()` |
| `now_epoch_ms() -> u64` / `epoch_ms(SystemTime) -> u64` | the same instant as epoch milliseconds (the shape for stored anchors and Fluent arguments) |
| `local_zone() -> Option<String>` | the device's IANA zone id; `"UTC"` when the OS has a zone but no IANA name for it; `None` when the platform won't say |
| `is_zone(&str) -> bool` | membership in the bundled database (case-insensitive) |
| `offset_seconds(zone, at) -> Option<i32>` | `zone`'s UTC offset at `at`, east positive; `None` for unknown zones |
| `local_offset_seconds(at) -> Option<i32>` | the device zone's offset, correct even when the zone has no IANA name |

Everything is best-effort and non-panicking: unknown zones answer `None`, never an error, and
there is no `Result` in the API.

**Code that ships to web must call `day_part_timezone::now()`, not `SystemTime::now()`**,
because std's clock aborts on `wasm32-unknown-unknown`. On every other target the two are identical.

## Rendering zoned time

Day has no zoned formatting API. Fluent's `DATETIME` renders an epoch as UTC
civil time, so shifting the epoch by the zone's offset renders that zone's civil time with the
locale's own conventions (12/24-hour, digits, ordering):

```rust
// app.ftl:  clock-row-time = { DATETIME($when, timeStyle: "short") }
let off = day_part_timezone::offset_seconds(zone, at).unwrap_or(0) as i64;
let shifted_ms = day_part_timezone::epoch_ms(at) as i64 + off * 1000;
// pass `shifted_ms` as the $when argument
```

The shift is a **rendering** step: store and compare real instants, and shift only at the last
moment before Fluent.

## Per-platform realization

| Target | wall clock | local zone |
|---|---|---|
| macOS · iOS · Linux | `SystemTime::now()` | `/etc/localtime` (jiff `tz-system`) |
| Android | `SystemTime::now()` | `persist.sys.timezone` (jiff) |
| Windows | `SystemTime::now()` | registry → IANA mapping (jiff) |
| OpenHarmony | `SystemTime::now()` | `/etc/localtime`-style detection; falls back to `"UTC"` |
| `web-dom` (`wasm32`) | shim `day_dom_now_ms()` = `Date.now()` | shim `tz` env key = `Intl.DateTimeFormat().resolvedOptions().timeZone`; `?tz=` overrides for testing |

Offset lookups (`offset_seconds`, `is_zone`) never touch the OS: the IANA database is compiled in
(~200 KB), so a zone that resolves on one target resolves on all of them, including wasm.

There are no cargo features; platform selection is `#[cfg(target_arch = "wasm32")]` versus
everything else, because a clock is a host concern rather than a toolkit one.

## Boundaries

- **There is no civil-time type.** Do arithmetic on `SystemTime`/epoch values and offsets; render through
  Fluent. Apps that need calendar math (the day-of-week of "next Tuesday 06:30 local") apply the
  offset and work in shifted epoch seconds.
- **Zone changes are not signaled.** If the user changes the device zone while the app runs,
  `local_zone()` answers the new zone on the next call; there is no event. Re-query on
  `DidBecomeActive`/`WillEnterForeground`.
- **The database is frozen at build time.** Bundled tzdb rules are as current as the jiff release
  compiled in; a government moving its DST date reaches users through an app update, not an OS
  update. (This is the standard bundled-tzdb trade; OS databases have the mirror problem on
  unupdated devices.)
