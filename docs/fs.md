<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# App-local file storage (headless capability crate)

> **Status: implemented** as `day-part-fs` (in `parts/`), a headless day-ecosystem crate with no
> UI Piece: private per-app file storage with one API on every target. Native targets read and
> write real files under an app-data root; web-dom stores files in the browser's Origin
> Private File System (OPFS) through the day-dom shim. Exercised by the showcase's Platform
> services page and its walkthrough on every target.

Where [day-part-prefs](prefs.md) is a small key/value store for settings, this crate is for
*data*: documents, caches, exports (anything file-shaped). Both persist per app and survive
restarts; on the web, prefs ride `localStorage` while files ride OPFS, which is sized for real
data and holds a true directory hierarchy.

## Authoring

```rust
// Blocking — Unsupported on web; keep large files off the UI thread.
day_part_fs::write("notes/today.txt", b"rain later")?;
let bytes = day_part_fs::read("notes/today.txt")?;
let names = day_part_fs::list("notes")?;   // ["today.txt"], dirs get a trailing '/'
day_part_fs::remove("notes/today.txt")?;

// Async — works on EVERY target, including web. Await under day::task (docs/async.md):
day::task(async move {
    day_part_fs::write_future("notes/today.txt", text.into_bytes()).await?;
    let names = day_part_fs::list_future("notes").await?;
    Ok::<(), day_part_fs::FsError>(())
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
| web-dom | the origin's OPFS via the day-dom shim (`day_dom_fs_start` + the request-id completion exports). OPFS is the ONLY store: a context without it — a pre-OPFS browser, or a private-browsing/ephemeral session, which WebKit gives no storage backing — answers `Unsupported` (no `getDirectory` at all) or `Io` (present but broken), never a silent alternate store |
| anything else | `FsError::Unsupported` |

`DAY_DATA_DIR` wins everywhere when set; the mobile hosts export it (DayActivity on Android,
EntryAbility on OHOS), and tests set it to a scratch directory.

## Error taxonomy

`NotFound`, `BadPath`, `Io(message)`, `Unsupported`. The web tier collapses provider detail
into `Io`, except `NotFoundError` → `NotFound` and a context without OPFS → `Unsupported`.

## What it shows about the extension system

The third part to ride the day-dom shim (after prefs and http), and the second to complete back
into wasm with the request-id pattern. Native needs no platform code at all: one `std::fs`
backend over an env-resolved root covers six targets, with the mobile hosts contributing a
single `DAY_DATA_DIR` line each.

## v2 notes (deliberately out of scope)

Streaming reads/writes (today a file is one buffer; see the memory-efficiency rule before
storing anything huge), append, rename, recursive remove, file metadata (size/mtime), and
cancellation for in-flight web operations.
