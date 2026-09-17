---
title: Local storage
description: "Persistent settings and app-private files on native and web targets."
order: 29
section: Guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day provides two forms of persistent storage: `day::prefs` for small settings and `day-part-fs`
for files. Settings are stored as strings; files hold documents, exports, or caches. Both use
platform storage locations assigned to the app.

Choose the store by the kind of data you need to keep:

| Data | API | Example |
|---|---|---|
| Small settings | `day::prefs` | Theme, volume, last selected tab |
| Files | `day-part-fs` | Notes, downloaded data, exported documents |

Both support macOS, iOS, Android, Linux, Windows, HarmonyOS, and web. On web, file operations
must be asynchronous. Neither API encrypts data; see [what not to store](#4-what-not-to-store)
before using them for sensitive information.

## 1. Persist a setting

`day::prefs` ships with the `day` crate (a default-on feature), so there is nothing to add to
`Cargo.toml`. The store takes and returns strings:

```rust
day::prefs::set("greeting", "hello");     // -> bool: did the write commit
day::prefs::get("greeting");              // -> Option<String>, Some("hello")
day::prefs::contains("greeting");         // -> bool
day::prefs::remove("greeting");           // -> bool: existed and was removed
```

Writes are synchronous and immediately readable. A stored empty string is `Some("")`, not
`None`. Keep values modest: a large blob belongs in a file (step 3).

## 2. Bind a signal so it survives relaunch

For state your UI already holds in a `Signal`, skip the manual get/set and bind it:

```rust
let volume = Signal::new(40.0f64);
day::prefs::bind("settings.volume", volume);
```

`bind(key, signal)` seeds the signal from the store now and persists every later change. Any
`Signal<T>` works when `T` round-trips through `FromStr`/`ToString`: numbers, bools, strings.
Call it right after creating the signal: the write-back is a reactive watch, and it stops when
the creating scope is disposed.

Preferences also back navigation persistence: call `day::prefs::install_nav_store()` once in
`main` and a `nav` or `nav_stack` marked `.restore(key)` remembers its state across launches.
See [navigation](/docs/navigation).

## 3. Write and read files

`day-part-fs` is a separate dependency. Match its revision to the Day version used by your app:

```toml
[dependencies]
day-part-fs = { git = "https://github.com/daybrite/day.git" }
```

Paths are relative and sandboxed inside a private per-app root: an absolute path or a `.`/`..`
segment is `FsError::BadPath` before any platform code runs. `write` creates missing parent
directories. Each operation comes in three forms: blocking (`read`, `write`, `remove`, `list`),
callback (`read_async`, …), and future (`read_future`, …). Blocking calls work on native targets and return `FsError::Unsupported` on the web, where the single browser thread
cannot wait; the `*_future` forms work everywhere, awaited under `day::task`:

```rust
let status = Signal::new(String::new());
day::task(async move {
    if let Err(e) = day_part_fs::write_future("notes/today.txt", text.into_bytes()).await {
        status.set(format!("error: {e}"));
        return;
    }
    match day_part_fs::read_future("notes/today.txt").await {
        Ok(bytes) => status.set(String::from_utf8_lossy(&bytes).into_owned()),
        Err(e) => status.set(format!("error: {e}")),
    }
});
```

The `match` stays inside the task because `day::task` takes a future with `Output = ()`; an
async block that returns a `Result` doesn't compile there. Handle both arms and write the
outcome into signals; the future resumes on the UI thread, so those are plain signal writes.

`list(dir)` returns the entry names directly under `dir`, sorted, with directories suffixed
`/`; `list("")` is the root, and a never-written directory lists as empty, the ordinary
first-run state, with no error. Removing a missing path is `FsError::NotFound`.

Where the files land:

| Target | Root |
|---|---|
| macOS / iOS | `~/Library/Application Support/day/day-fs/` (the iOS sandbox makes this the app container) |
| Android / HarmonyOS | the app's private files dir (host-provided `DAY_DATA_DIR`) + `day-fs/` |
| Linux | `$XDG_DATA_HOME/day/day-fs/`, else `~/.local/share/day/day-fs/` |
| Windows | `%APPDATA%\day\day-fs\` |
| web-dom | the origin's OPFS |

To cache a network response, fetch it with [day-part-http](/docs/guide-http) and write the body
with `day_part_fs::write_future`, so the next launch renders before the network answers.

## 4. What not to store

Neither store is for secrets. Prefs write to plain platform stores (a plist, a
`SharedPreferences` file, a flat config file, `localStorage`) and `day-part-fs` writes plain
files; neither encrypts. Day doesn't cover secret storage yet (there is no keychain or
keystore part), so keep tokens and passwords out of both until you wire the platform's secure
store yourself.

## Pitfalls

On unsupported targets, preferences return `None` for reads and `false` for writes. File
operations return `FsError::Unsupported`.

- Keep prefs values modest; large blobs belong in a file. On the web, `localStorage` can throw
  (private browsing, storage pressure); failures report as uncommitted writes or absent reads, never
  a panic.
- `bind`'s write-back stops with its scope. Bind in the scope that owns the signal, right
  after creating it. A signal bound inside a page keeps persisting only while that page's scope
  is alive.
- The blocking fs calls don't exist on web. They return `FsError::Unsupported`; use
  the `*_async` or `*_future` forms for code shared with web. Even natively, keep large files off
  the UI thread.
- `read` and `write` load the entire file into memory. Use a different approach for files
  too large to fit comfortably in memory; this API does not stream them.
- OPFS is the only web store. A pre-OPFS browser, or a private-browsing session (WebKit
  gives ephemeral sessions no storage backing), answers `Unsupported` or `Io`; there is no
  silent fallback store.
- Launch overrides beat stored settings. The settings pieces apply persisted theme/language
  with an env-wins rule: when `DAY_THEME` or `DAY_LOCALE` is set (a `day launch --env` run, CI
  variants), the persisted value is not re-applied at boot.
- To test persistence by hand on macOS: an unbundled binary stores under the process-name defaults
  domain; `defaults delete <name>` clears it (deleting the plist alone won't; `cfprefsd` caches).

## Reference

[prefs](/docs/internal/prefs) — the full `day::prefs` contract and each platform's store.
[fs](/docs/internal/fs) — the path rules, error taxonomy, and the OPFS web tier.
[async](/docs/internal/async) — `day::task` and main-thread updates and task lifetimes.
