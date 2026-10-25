---
navTitle: "File storage"
title: "App-local file storage"
description: "Reading and writing app-local files via day-part-filesystem, including the OPFS-only web arm."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# App-local file storage

`day-part-filesystem` reads and writes files beneath an app-data directory. Use it for documents,
cached responses, and other data that is too large for [preferences](prefs.md).
Native targets use filesystem storage; web uses the browser’s Origin Private File System
(OPFS). Files persist across launches, subject to the platform’s storage policies.

## Add to your app

Defined in [`day-part-filesystem`](https://github.com/daybrite/day/tree/main/crates/day-part-filesystem). Add this to your app’s `Cargo.toml`
`[dependencies]` table, using the same Day Git source as the rest of your app:

```toml
[dependencies]
day-part-filesystem = { git = "https://github.com/daybrite/day.git" }
```

The implementation is selected by the compilation target; no toolkit feature flags
or dependency entry in `Day.toml` are needed.

| Platforms / toolkits | Availability |
| --- | --- |
| macOS, iOS, Android, Windows, Linux (GTK / Qt), HarmonyOS | Filesystem storage beneath the app-data root. |
| Web | Async APIs only, using OPFS; unavailable storage returns an error. |

This reference covers paths, errors, and storage behavior. The
[local storage guide](/docs/how-to/guide-storage) shows how to use files from an app.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](/docs/how-to/localization).

## Authoring

```rust
// Blocking — Unsupported on web; keep large files off the UI thread.
day_part_filesystem::write("notes/today.txt", b"rain later")?;
let bytes = day_part_filesystem::read("notes/today.txt")?;
let names = day_part_filesystem::list("notes")?; // ["today.txt"], dirs get a trailing '/'
day_part_filesystem::remove("notes/today.txt")?;

// Async — uses the target's storage implementation, which may be unavailable. Await under day::task (docs/async.md).
// `day::task` takes a `Future<Output = ()>`, so each result is handled in the block
// instead of propagated with `?`:
day::task(async move {
    match day_part_filesystem::write_future("notes/today.txt", text.into_bytes()).await {
        Ok(()) => status.set(res::str::saved().format()),
        Err(e) => status.set(res::str::operation_failed(e.to_string()).format()),
    }
    if let Ok(names) = day_part_filesystem::list_future("notes").await {
        files.set(names.join(", "));
    }
});
```

The contract points:

- **Paths are checked lexically.** Absolute paths, colons, and `.`/`..`/empty segments return
  `FsError::BadPath` before filesystem access. This does not reject symlinks within the root or
  create an OS sandbox. Native fallback directories use shared `day/day-fs` naming; provide
  an app-specific `DAY_DATA_DIR` when isolation is required. [`write`](https://daybrite.dev/api/day_part_filesystem/fn.write.html) creates missing parents.
- **[`list("")`](https://daybrite.dev/api/day_part_filesystem/fn.list.html) is the root**, entries sorted, directories suffixed `/`. A never-written
  directory lists as empty, the ordinary first-run state, not an error.
- **The blocking calls follow the [day-part-http](/docs/reference/http) rule**: real on every native target, and
  `FsError::Unsupported` on web, where the single browser thread cannot wait. The `*_async`
  twins and `*_future` forms are the portable surface.

## Where files live

| Target | Root |
|---|---|
| Android / HarmonyOS | the host-provided `DAY_DATA_DIR` (the app's private files dir) + `day-fs/` |
| macOS / iOS | `~/Library/Application Support/day/day-fs/` (the iOS sandbox `HOME` makes this the app container) |
| Linux | `$XDG_DATA_HOME/day/day-fs/` (else `~/.local/share/day/day-fs/`) |
| Windows | `%APPDATA%\day\day-fs\` |
| Web | the origin's OPFS via the day-toolkit-dom shim (`day_dom_fs_start` + the request-id completion exports). OPFS is the only store: a context without it (a pre-OPFS browser, or a private-browsing/ephemeral session, which WebKit gives no storage backing) answers `Unsupported` (no `getDirectory` at all) or `Io` (present but broken), never a silent alternate store |
| anything else | `FsError::Unsupported` |

`DAY_DATA_DIR` wins on native targets when set; the mobile hosts export it (DayActivity in day-toolkit-material,
EntryAbility in day-toolkit-arkui's staged ArkTS host), and tests set it to a scratch directory.

[`day_part_filesystem::data_dir()`](https://daybrite.dev/api/day_part_filesystem/fn.data_dir.html) returns that directory without the `day-fs/` leaf on native targets,
for code that keeps files of its own beside the part's: a download manager's journal and partial
files, for instance ([docs/downloads](downloads.md)). [`day_part_filesystem::platform_data_dir()`](https://daybrite.dev/api/day_part_filesystem/fn.platform_data_dir.html)
returns the platform directory above it, the one every application's data sits under, for an
app that opens a store another application keeps there (a Day client over a desktop app's own
database); `DAY_DATA_DIR` does not redirect that one. The web has no such directory, so both
functions exist only off wasm32.

## Error taxonomy

`NotFound`, `BadPath`, `Io(message)`, `Unsupported`. The web tier collapses provider detail
into `Io`, except `NotFoundError` → `NotFound` and a context without OPFS → `Unsupported`.

## What it shows about the extension system

This is the third part to ride the day-toolkit-dom shim (after prefs and http), and the second to
complete back into wasm with the request-id pattern. Native needs no platform code at all: one
`std::fs` implementation over an env-resolved root covers six targets, with the mobile hosts
contributing a single `DAY_DATA_DIR` line each.

## v2 notes (out of scope)

The portable buffered API has no append, rename, recursive-remove, or metadata method.
Native-only [`open_read(path)`](https://daybrite.dev/api/day_part_filesystem/fn.open_read.html) already returns a seekable `std::fs::File`; use it off the UI
thread for large reads. A portable streaming writer and cancellation of accepted native IO
are not supplied by these future wrappers.

For macOS container storage, file permissions, migration, and SQLite sidecar restrictions,
see [macOS App Sandbox](sandbox.md).

Native archive readers can use [`open_read(path)`](https://daybrite.dev/api/day_part_filesystem/fn.open_read.html) to obtain a read-only `std::fs::File`
implementing `Read + Seek`. It applies the same relative-path checks and `day-fs/` storage root
as [`read`](https://daybrite.dev/api/day_part_filesystem/fn.read.html) and [`write`](https://daybrite.dev/api/day_part_filesystem/fn.write.html), avoiding whole-file buffers and application-specific path reconstruction.
Opening and reading are blocking: perform this work off the UI thread. Web does not expose this
native handle; use [`read_future`](https://daybrite.dev/api/day_part_filesystem/fn.read_future.html) there, which currently returns the whole file as an owned buffer.

## Lifetime and evidence

Code above is an integration fragment; its literal bytes are synthetic sample document data.
Use the [compiled storage recipe](/docs/how-to/guide-storage) for full setup.
Native async jobs run detached on the blocking pool. Dropping `FsFuture` discards delivery but
does not undo an accepted write. [`write`](https://daybrite.dev/api/day_part_filesystem/fn.write.html) creates/truncates and does not promise atomic replacement.
`FsError` is non-exhaustive; callers must allow new variants.

Source: [public contracts](../crates/day-part-filesystem/src/lib.rs) and
[native implementation](../crates/day-part-filesystem/src/native.rs), reviewed 2026-10-22. Browser storage
and mobile host integration were not runtime-tested by the recipe compile check.
