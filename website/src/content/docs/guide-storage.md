---
title: Store data on device
description: "Persist settings with day::prefs and files with day-part-fs — both land in each platform's conventional storage location, from NSUserDefaults to OPFS."
order: 29
section: Guides
---

Apps remember two kinds of things: small settings (a theme choice, a volume, the last-open tab)
and real data (documents, exports, caches). Day splits them across two parts. `day::prefs` is a
string key/value store backed by each platform's own preferences facility; `day-part-fs` is
private per-app file storage. One call each:

```rust
day::prefs::set("theme", "dark");                       // NSUserDefaults, SharedPreferences, …
day_part_fs::write("notes/today.txt", b"rain later")?;  // a real file under the app-data root
```

Both persist per app and survive restarts, in the place each platform expects: prefs go to
`NSUserDefaults` on macOS and iOS, `SharedPreferences` on Android, a file under the config
directory on Linux and Windows, and `localStorage` on the web; files go under an app-private
data root natively and into the browser's Origin Private File System (OPFS) on the web. The
dividing line is size and shape: prefs is a small string store for settings, not a database,
and anything file-shaped belongs in `day-part-fs`.

**Works on:** both parts cover macOS, iOS, Android, Linux, Windows, HarmonyOS, and `web-dom`.
On any other target prefs is a no-op store (`get` returns `None`, `set` returns `false`) and
every fs call returns `FsError::Unsupported`. On the web, fs is async-only; see step 3.

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

On the web this matters most, because a reload is part of normal life there. The showcase's
Controls page binds its counter, name field, volume, and toggle on wasm only, so a reload keeps
them while native launches start fresh on purpose:

```rust
#[cfg(target_arch = "wasm32")]
day::prefs::bind("controls.count", count);
```

The same store also backs navigation persistence: call `day::prefs::install_nav_store()` once in
`main` and a `selector` or `stack` marked `.restore(key)` remembers its state across launches.
See [navigation](/docs/navigation).

## 3. Write and read files

`day-part-fs` is a separate dependency:

```toml
[dependencies]
day-part-fs = { git = "https://github.com/daybrite/day.git" }
```

Paths are relative and sandboxed inside a private per-app root: an absolute path or a `.`/`..`
segment is `FsError::BadPath` before any platform code runs. `write` creates missing parent
directories. Each operation comes in three forms: blocking (`read`, `write`, `remove`, `list`),
callback (`read_async`, …), and future (`read_future`, …). The blocking calls are real on every
native target and return `FsError::Unsupported` on the web, where the single browser thread
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
first-run state, not an error. Removing a missing path is `FsError::NotFound`.

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

- **Prefs is a small string store, not a database.** Keep values modest; large blobs belong in
  a file. On the web, `localStorage` can throw (private browsing, storage pressure); failures
  report as uncommitted writes or absent reads, never a panic.
- **`bind`'s write-back stops with its scope.** Bind in the scope that owns the signal, right
  after creating it. A signal bound inside a page keeps persisting only while that page's scope
  is alive.
- **The blocking fs calls don't exist on web.** They return `FsError::Unsupported`; the
  `*_async` and `*_future` forms are the portable surface. Even natively, keep large files off
  the UI thread.
- **A file is one buffer.** v1 has no streaming: `read` and `write` move the whole body through
  memory, so don't store anything huge this way.
- **OPFS is the only web store.** A pre-OPFS browser, or a private-browsing session (WebKit
  gives ephemeral sessions no storage backing), answers `Unsupported` or `Io`; there is no
  silent fallback store.
- **Launch overrides beat stored settings.** The settings pieces apply persisted theme/language
  with an env-wins rule: when `DAY_THEME` or `DAY_LOCALE` is set (a `day launch --env` run, CI
  variants), the persisted value is not re-applied at boot.
- **Testing persistence by hand on macOS:** an unbundled binary stores under the process-name
  defaults domain; `defaults delete <name>` clears it (deleting the plist alone won't;
  `cfprefsd` caches).

## Reference

[prefs](/docs/internal/prefs) — the full `day::prefs` contract and each platform's store.
[fs](/docs/internal/fs) — the path rules, error taxonomy, and the OPFS web tier.
[async](/docs/internal/async) — `day::task` and the rules the file sample leans on.
