---
navTitle: "File storage"
title: "App-local file storage"
description: "Reading and writing app-local files via day-part-filesystem, including browser storage and platform directories."
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

Add [`day-part-filesystem`](https://github.com/daybrite/day/tree/main/crates/day-part-filesystem) to your app:

```sh
cargo add day-part-filesystem
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
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

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
  create an OS sandbox. Native apps with different IDs use separate default directories.
  Matching IDs or directory overrides can share storage. [`write`](https://docs.rs/day-part-filesystem/latest/day_part_filesystem/fn.write.html) creates missing parents.
- **[`list("")`](https://docs.rs/day-part-filesystem/latest/day_part_filesystem/fn.list.html) is the root**, entries sorted, directories suffixed `/`. A never-written
  directory lists as empty, the ordinary first-run state, not an error.
- **The blocking calls follow the [day-part-http](/docs/reference/http) rule**: real on every native target, and
  `FsError::Unsupported` on web, where the single browser thread cannot wait. The `*_async`
  twins and `*_future` forms are the portable surface.

## Where files live

Use [`day::storage`](https://docs.rs/day-app/latest/day/storage/index.html) to locate the app's native storage directories:

- [`data_dir()`](https://docs.rs/day-app/latest/day/storage/fn.data_dir.html) is for documents, databases, and other data the app must retain.
- [`cache_dir()`](https://docs.rs/day-app/latest/day/storage/fn.cache_dir.html) is for data the app can download or compute again.
- [`config_dir()`](https://docs.rs/day-app/latest/day/storage/fn.config_dir.html) is for small configuration files. Use
  [preferences](prefs.md) for individual settings.

These functions resolve paths without creating directories. They return `None` when a directory
cannot be resolved, including on Web and in mobile processes without host-provided paths.
When using the paths directly, create the required directories before writing files.

Desktop defaults include the app ID from `[app] id` in `Day.toml`. Builds with the same ID and
storage environment share directories across toolkits. A [build flavor](/docs/how-to/flavors)
with a different ID uses separate defaults. A binary without a configured app ID falls back to
its executable name.

| Platform | Data | Cache | Configuration |
| --- | --- | --- | --- |
| macOS | `~/Library/Application Support/<id>` | `~/Library/Caches/<id>` | Same as data |
| macOS, App Sandbox | `Library/Application Support/<id>` inside the app container | `Library/Caches/<id>` inside the container | Same as data |
| iOS | `Library/Application Support/<id>` inside the app container | `Library/Caches/<id>` inside the container | Same as data |
| Windows | `%LOCALAPPDATA%\<id>` | `%LOCALAPPDATA%\<id>\cache` | `%APPDATA%\<id>` |
| Linux, BSD | `$XDG_DATA_HOME/<id>`, otherwise `~/.local/share/<id>` | `$XDG_CACHE_HOME/<id>`, otherwise `~/.cache/<id>` | `$XDG_CONFIG_HOME/<id>`, otherwise `~/.config/<id>` |
| Android | The app's private files directory | The app's private cache directory | `config/` inside the files directory |
| HarmonyOS | The app's private files directory | The app's private cache directory | `config/` inside the files directory |
| Web | No native path; files use the origin's OPFS | No native path | No native path |

Day uses XDG variables on Linux and BSD; it ignores them on macOS and Windows.
The Android and HarmonyOS hosts supply their app-private directories.

### Directory overrides

`DAY_DATA_DIR` replaces the data directory. When set, configuration uses its `config/`
subdirectory and cache uses its `cache/` subdirectory. `DAY_CACHE_DIR` takes precedence for
cache only. These overrides must be absolute paths; relative and empty values are ignored.

Overrides change path resolution; they do not restrict filesystem access or redirect every
storage API. Native Apple and Android preferences use their system stores, and `DAY_PREFS_DIR`
can select a separate preferences file store. See
[isolating a storage test](/docs/how-to/guide-storage#isolate-a-storage-test).

### Files stored by Day

| Content | Default location |
| --- | --- |
| This part's files | `day-fs/` inside the data directory |
| [Persistence](persistence.md) databases opened with [`Sqlite::app_data(name)`](https://docs.rs/day-persistence/latest/day_persistence/struct.Sqlite.html#method.app_data) | The data directory |
| [Windows keychain](keychain.md) items | `day-keychain/` inside the data directory |
| [Preferences](prefs.md) file store on Linux, Windows, and HarmonyOS | `preferences.store` inside the configuration directory |
| [Remembered window frames](windows.md) | `day-windows.txt` inside the configuration directory |

[`day_part_filesystem::data_dir()`](https://docs.rs/day-part-filesystem/latest/day_part_filesystem/fn.data_dir.html)
returns the data directory without the `day-fs/` subdirectory. Use it for files managed directly
by the app, such as a [download journal](downloads.md).
[`platform_data_dir()`](https://docs.rs/day-part-filesystem/latest/day_part_filesystem/fn.platform_data_dir.html)
returns the shared platform directory: Application Support, roaming `%APPDATA%`, or
`$XDG_DATA_HOME`. `DAY_DATA_DIR` does not redirect it. Both functions are native-only.

### Platform storage policies

- **macOS / iOS:** sandboxed apps use paths inside their app container. When enabling the
  [macOS App Sandbox](sandbox.md), check whether existing data needs migration.
  [Apple's filesystem guide](https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/FileSystemProgrammingGuide/FileSystemOverview/FileSystemOverview.html)
  describes container locations and backup behavior. On iOS, Application Support is normally
  backed up; Caches is excluded and may be cleared by the system.
- **Android:** [Auto Backup](https://developer.android.com/identity/data/autobackup) normally
  includes private files and shared preferences, subject to backup settings and quotas. Cache
  files are excluded. Keep data the app needs to retain out of the cache directory.
- **Windows:** data and caches use local storage; configuration uses the roaming directory.
  [MSIX virtualization](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-behind-the-scenes)
  can redirect AppData writes, depending on the package's configuration. Review those rules
  before relying on data sharing or removal during uninstall.
- **Linux:** [Flatpak](https://docs.flatpak.org/en/latest/sandbox-permissions.html) redirects the
  XDG directories into its sandbox. Non-classic [Snap packages](https://snapcraft.io/docs/reference/development/environment-variables/)
  remap `HOME`. Query Day's storage paths inside the packaged app instead of assuming the
  unpackaged paths still apply.
- **Web:** files use the origin's Origin Private File System (OPFS), while
  [preferences](prefs.md) use `localStorage`. Browser policy can make storage unavailable or
  remove its contents. Unsupported OPFS returns `FsError::Unsupported`; other access failures
  return `Io`. This part provides no alternate browser file store.

## Error taxonomy

`NotFound`, `BadPath`, `Io(message)`, `Unsupported`. The web tier collapses provider detail
into `Io`, except `NotFoundError` → `NotFound` and a context without OPFS → `Unsupported`.

## Host integration

Custom hosts must supply the platform storage environment. See the contributor
[storage integration notes](/contributing/extending#storage-integration) for the mobile host
and browser bridge contracts.

## Streaming reads and limits

The portable buffered API has no append, rename, recursive-remove, or metadata method.
Native-only [`open_read(path)`](https://docs.rs/day-part-filesystem/latest/day_part_filesystem/fn.open_read.html) already returns a seekable `std::fs::File`; use it off the UI
thread for large reads. A portable streaming writer and cancellation of accepted native IO
are not provided by the portable API.

For macOS container storage, file permissions, migration, and SQLite sidecar restrictions,
see [macOS App Sandbox](sandbox.md).

Native archive readers can use [`open_read(path)`](https://docs.rs/day-part-filesystem/latest/day_part_filesystem/fn.open_read.html) to obtain a read-only `std::fs::File`
implementing `Read + Seek`. It applies the same relative-path checks and `day-fs/` storage root
as [`read`](https://docs.rs/day-part-filesystem/latest/day_part_filesystem/fn.read.html) and [`write`](https://docs.rs/day-part-filesystem/latest/day_part_filesystem/fn.write.html), avoiding whole-file buffers and application-specific path reconstruction.
Opening and reading are blocking: perform this work off the UI thread. Web does not expose this
native handle; use [`read_future`](https://docs.rs/day-part-filesystem/latest/day_part_filesystem/fn.read_future.html) there, which currently returns the whole file as an owned buffer.

## Async lifetime

Native async jobs run detached on the blocking pool. Dropping `FsFuture` discards delivery but
does not undo an accepted write. [`write`](https://docs.rs/day-part-filesystem/latest/day_part_filesystem/fn.write.html) creates/truncates and does not promise atomic replacement.
`FsError` is non-exhaustive; callers must allow new variants.
