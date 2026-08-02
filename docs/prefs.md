# Persistent preferences (headless capability crate)

> **Status: implemented** as `day-part-prefs` (in `parts/`, the headless counterpart of `pieces/`).
> It's a headless day-ecosystem crate (no UI Piece): a shared cross-platform API for a small persistent
> **string key/value store**, backed by each platform's native preferences facility. Verified on macOS
> (real round-trip through `NSUserDefaults`); iOS-sim / Android (Rust side) / HarmonyOS / Linux all
> clippy-clean and cross-compile.
>
> **Promoted into the facade (2026-08): reach for `day::prefs`.** Nearly every app wants settings, and
> this is the one part that lives in the reactive layer (`bind`) and backs a core framework feature
> (`.restore(key)` navigation), so `day` depends on it behind a **default-on `prefs` feature** and
> re-exports it as `day::prefs`. Apps need no separate dependency. It is still its own crate, so a
> direct `day-part-prefs` dependency and the `day_part_prefs::…` paths keep working unchanged — the two
> spellings are the same API. Decline it with `day = { …, default-features = false }`, which drops the
> Apple `NSUserDefaults` dependency and the Android `SharedPreferences` Java shim.

## Authoring

```rust
day::prefs::set("greeting", "hello");                // persist a value
assert_eq!(day::prefs::get("greeting").as_deref(), Some("hello"));
assert!(day::prefs::contains("greeting"));
day::prefs::remove("greeting");                      // delete it
```

| Function | Behavior |
|---|---|
| `set(key, value) -> bool` | Persist `value` under `key`, overwriting. `true` when the write committed. |
| `get(key) -> Option<String>` | The stored string, or `None` if absent. A stored `""` is `Some("")`. |
| `remove(key) -> bool` | Delete the value; `true` only if it existed and was removed. |
| `contains(key) -> bool` | Whether a value is currently stored under `key`. |
| `bind(key, signal)` | Two-way-bind a `Signal<T>` to a stored value: seed from the store now, persist every later change. `T` round-trips through `FromStr`/`ToString`. Call it right after creating the signal; the write-back stops with the creating scope. |
| `install_nav_store()` | Install this store as day-core's navigation-persistence sink, so a `selector`/`stack` marked `.restore(key)` remembers its state across launches (and an Android process death). Call once in `main`, before the UI mounts. Nav keys are namespaced under `day.nav.`. See [navigation](navigation.md). |

Values persist across launches; that's the point. The crate has no cargo features: platform
selection is purely `#[cfg(target_os)]` (plus `#[cfg(target_arch = "wasm32")]` for the web),
since persistence depends on the OS, not on which widget toolkit is in use. `parts/day-part-prefs/examples/prefs.rs` is a plain `main` that uses it with no
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
| Web | `localStorage` (per origin, `day.pref.` namespace) via the day-dom shim | the `web-dom` host page (docs/web.md), `web.rs` |

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
- **Web (web-dom)**: `localStorage`, reached through the day-dom host shim's `day_dom_pref_*`
  imports under a `day.pref.` key namespace — values survive reloads and browser restarts,
  scoped per origin. `localStorage` can throw (private browsing, storage pressure); failures
  report as uncommitted/absent, matching the contract everywhere else. The showcase's Controls
  page binds its state through this store, so a reload keeps the counter (docs/web.md). On
  wasm outside a day-dom host page the imports are unresolved and instantiation fails.
- **Any other platform**: a no-op store: `set`/`remove`/`contains` return `false`, `get` returns
  `None`.

## What it shows about the extension system

Like `day-part-battery` and `day-part-network`, this is a headless external crate: it has no UI Piece
and registers nothing in any backend's `RENDERERS` slice. On Android it stages its own Java shim
through `[package.metadata.day.android]` (with no permission this time, since private storage needs
none), which `day build` folds into the app's Gradle build without touching any core day crate. On
every other platform it is fully day-independent (pure FFI on Apple, pure `std` file I/O on desktop).
See docs/extending.md.

## Settings pieces + the env-wins rule (docs/windows.md)

`pieces/day-piece-settings` packages the theme/language settings rows every app was
hand-rolling — `appearance_picker`/`language_picker`/`settings_sections` persist through
this part and apply live. Its `apply_startup(theme_key, locale_key)` applies persisted
overrides at boot with the **env-wins rule**: when `DAY_THEME` or `DAY_LOCALE` is set (a
`day launch --env`/`--locale` run, every themed CI variant), the persisted value is NOT
re-applied — the launch override stays deterministic no matter what an earlier run stored.
Live picker changes still apply and persist after boot. Local-run hygiene when testing
persistence by hand: unbundled macOS binaries store under the process-name defaults domain
(`defaults delete <name>` clears it — a plist delete alone won't, cfprefsd caches).
