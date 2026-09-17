---
title: "App-local file storage"
description: "Reading and writing app-local files via day-part-fs, including the OPFS-only web arm."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# App-local file storage

`day-part-fs` reads and writes files beneath an app-data directory. Use it for documents,
cached responses, and other data that is too large for [preferences](prefs.md).
Native targets use filesystem storage; web uses the browser’s Origin Private File System
(OPFS). Files persist across launches, subject to the platform’s storage policies.

This reference covers paths, errors, and backend behavior. The
[local storage guide](https://daybrite.dev/docs/guide-storage) shows how to use files from an app.

## Authoring

```rust
// Blocking — Unsupported on web; keep large files off the UI thread.
day_part_fs::write("notes/today.txt", b"rain later")?;
let bytes = day_part_fs::read("notes/today.txt")?;
let names = day_part_fs::list("notes")?;   // ["today.txt"], dirs get a trailing '/'
day_part_fs::remove("notes/today.txt")?;

// Async — works on EVERY target, including web. Await under day::task (docs/async.md).
// `day::task` takes a `Future<Output = ()>`, so each result is handled in the block
// instead of propagated with `?`:
day::task(async move {
    match day_part_fs::write_future("notes/today.txt", text.into_bytes()).await {
        Ok(()) => status.set("saved".into()),
        Err(e) => status.set(format!("error: {e}")),
    }
    if let Ok(names) = day_part_fs::list_future("notes").await {
        files.set(names.join(", "));
    }
});
```

The contract points:

- **Paths are relative and sandboxed.** The root is private to the app; an absolute path or a
  `.`/`..`/empty segment is `FsError::BadPath` before any backend runs. `write` creates missing
  parent directories.
- **`list("")` is the root**, entries sorted, directories suffixed `/`. A never-written
  directory lists as empty, the ordinary first-run state, not an error.
- **The blocking calls follow the day-part-http rule**: real on every native target, and
  `FsError::Unsupported` on web, where the single browser thread cannot wait. The `*_async`
  twins and `*_future` forms are the portable surface.

## Where files live

| Target | Root |
|---|---|
| Android / HarmonyOS | the host-provided `DAY_DATA_DIR` (the app's private files dir) + `day-fs/` |
| macOS / iOS | `~/Library/Application Support/day/day-fs/` (the iOS sandbox `HOME` makes this the app container) |
| Linux | `$XDG_DATA_HOME/day/day-fs/` (else `~/.local/share/day/day-fs/`) |
| Windows | `%APPDATA%\day\day-fs\` |
| web-dom | the origin's OPFS via the day-dom shim (`day_dom_fs_start` + the request-id completion exports). OPFS is the only store: a context without it (a pre-OPFS browser, or a private-browsing/ephemeral session, which WebKit gives no storage backing) answers `Unsupported` (no `getDirectory` at all) or `Io` (present but broken), never a silent alternate store |
| anything else | `FsError::Unsupported` |

`DAY_DATA_DIR` wins everywhere when set; the mobile hosts export it (DayActivity in day-android,
EntryAbility in day-arkui's staged ArkTS host), and tests set it to a scratch directory.

`day_part_fs::data_dir()` returns that directory without the `day-fs/` leaf on native targets,
for code that keeps files of its own beside the part's: a download manager's journal and partial
files, for instance ([docs/downloads.md](downloads.md)). The web has no such directory, so the
function exists only off wasm32.

## Error taxonomy

`NotFound`, `BadPath`, `Io(message)`, `Unsupported`. The web tier collapses provider detail
into `Io`, except `NotFoundError` → `NotFound` and a context without OPFS → `Unsupported`.

## What it shows about the extension system

This is the third part to ride the day-dom shim (after prefs and http), and the second to
complete back into wasm with the request-id pattern. Native needs no platform code at all: one
`std::fs` backend over an env-resolved root covers six targets, with the mobile hosts
contributing a single `DAY_DATA_DIR` line each.

## v2 notes (out of scope)

Streaming reads/writes (today a file is one buffer; see the memory-efficiency rule before
storing anything huge), append, rename, recursive remove, file metadata (size/mtime), and
cancellation for in-flight web operations.

For macOS container storage, file permissions, migration, and SQLite sidecar restrictions,
see [macOS App Sandbox](sandbox.md).
