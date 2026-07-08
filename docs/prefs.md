# Persistent preferences (headless capability crate)

> **Status: implemented** as `day-part-prefs` (in `parts/`, the headless counterpart of `pieces/`).
> It's a headless day-ecosystem crate (no UI Piece): a shared cross-platform API for a small persistent
> **string key/value store**, backed by each platform's native preferences facility. Any Rust code can
> depend on it and call `day_part_prefs::{set, get, remove, contains}`. Verified on macOS (real
> round-trip through `NSUserDefaults`); iOS-sim / Android (Rust side) / HarmonyOS / Linux all
> clippy-clean and cross-compile.

## Authoring

```rust
day_part_prefs::set("greeting", "hello");            // persist a value
assert_eq!(day_part_prefs::get("greeting").as_deref(), Some("hello"));
assert!(day_part_prefs::contains("greeting"));
day_part_prefs::remove("greeting");                  // delete it
```

| Function | Behavior |
|---|---|
| `set(key, value) -> bool` | Persist `value` under `key`, overwriting. `true` when the write committed. |
| `get(key) -> Option<String>` | The stored string, or `None` if absent. A stored `""` is `Some("")`. |
| `remove(key) -> bool` | Delete the value; `true` only if it existed and was removed. |
| `contains(key) -> bool` | Whether a value is currently stored under `key`. |

Values persist across launches; that's the point. The crate has no cargo features: platform
selection is purely `#[cfg(target_os)]`, since persistence depends on the OS, not on which widget
toolkit is in use. `parts/day-part-prefs/examples/prefs.rs` is a plain `main` that uses it with no
Day framework at all (run it twice to watch a value survive the process).

This is a small string store for user settings and lightweight app state, not a database. Keep
values modest; large blobs belong in a file.

## Per-platform native realization

| OS | API | dependency |
|---|---|---|
| macOS | `NSUserDefaults.standard` | `objc2` + `objc2-foundation`, shared `apple.rs` |
| iOS | `NSUserDefaults.standard` | `objc2` + `objc2-foundation`, shared `apple.rs` |
| Android | `SharedPreferences` (`MODE_PRIVATE`) via a Java shim | `day-android` + `[package.metadata.day.android]` |
| Linux | file store under `$XDG_CONFIG_HOME/day` (or `~/.config/day`) | std only, shared `file.rs` |
| Windows | file store under `%APPDATA%\day` | std only, shared `file.rs` |
| HarmonyOS | file store, best-effort in the app sandbox (`target_env = "ohos"`) | std only, shared `file.rs` |

## What each platform does

- **macOS / iOS**: `NSUserDefaults.standard` is the system's per-application preferences store (a
  plist under `~/Library/Preferences` on macOS, the app container on iOS). It is toolkit-independent
  (no `NSApplication`/`UIApplication`, run loop, or window), so the crate works in `day-qt` binaries
  and plain `cargo test` processes as well as under `day-appkit`/`day-uikit`. `setObject:forKey:` is
  the only `unsafe` objc2 call (the value must be a property-list type, and we always pass a real
  `NSString`); writes are immediately readable and flushed to disk by the system. `set` always
  returns `true`.
- **Android**: an app-private `SharedPreferences` file named `day_part_prefs`, opened with
  `Context.MODE_PRIVATE` from `day-android`'s cached `Context` (`DayBridge.ctx`). Writes use
  `Editor.commit()` (synchronous), so `set`/`remove` return the true commit result. No manifest
  permission is required; `SharedPreferences` is private storage. Like the UI pieces, the crate
  stages its own Java shim through `[package.metadata.day.android]` and rides the Day runtime (it
  needs the app's JVM + `Context`).
- **Linux / Windows / HarmonyOS**: a file-backed store, one flat `String -> String` map serialized
  under `<config-dir>/day/day-part-prefs.store`. `config-dir` is `$XDG_CONFIG_HOME` (else `~/.config`)
  on Linux, `%APPDATA%` (else `%USERPROFILE%\AppData\Roaming`) on Windows, and a best-effort app files
  dir on HarmonyOS. Each entry is a line `escaped_key=escaped_value`; the escaper removes every raw
  `=`, newline, and carriage return (`\` → `\\`, newline → `\n`, CR → `\r`, `=` → `\e`), so the first
  raw `=` on a line is unambiguously the separator and a value may contain anything. Writes are
  best-effort atomic (write a sibling temp file, then rename over the target); a process-wide mutex
  serializes load-modify-save cycles. Every read tolerates a missing, unreadable, or corrupt file
  by treating the store as empty, so a partial write or a hand-edit can never panic a caller. `set`
  returns `false` only when the store file could not be written (e.g. a read-only home). No extra
  dependencies beyond `std`.
- **Any other platform**: a no-op store: `set`/`remove`/`contains` return `false`, `get` returns
  `None`.

## What it shows about the extension system

Like `day-part-battery` and `day-part-network`, this is a headless external crate: it has no UI Piece
and registers nothing in any backend's `RENDERERS` slice. On Android it stages its own Java shim
through `[package.metadata.day.android]` (with no permission this time, since private storage needs
none), which `day build` folds into the app's Gradle build without touching any core day crate. On
every other platform it is fully day-independent (pure FFI on Apple, pure `std` file I/O on desktop).
See docs/extending.md.
