---
navTitle: "Preferences"
title: "Preferences storage"
description: "Persistent key-value preferences via day-part-prefs, stored in each platform's native store."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Persistent preferences

`day::prefs` stores small string values between launches, such as a selected theme or volume.
It is provided by [`day-part-prefs`](https://daybrite.dev/api/day_part_prefs/index.html) and re-exported by `day` through the default-enabled `prefs`
feature. Apps using the facade do not need a separate dependency.

## Add to your app

The default-enabled `day::prefs` re-export needs no additional dependency.
For direct use, the API is defined in [`day-part-prefs`](https://github.com/daybrite/day/tree/main/parts/day-part-prefs). Add this optional dependency to your app’s `Cargo.toml`
`[dependencies]` table, using the same Day Git source as the rest of your app:

```toml
[dependencies]
day-part-prefs = { git = "https://github.com/daybrite/day.git" }
```

The implementation is selected by the compilation target; no toolkit feature flags
or dependency entry in `Day.toml` are needed.

| Platforms / toolkits | Availability |
| --- | --- |
| macOS, iOS, Android, Windows, Linux (GTK / Qt), HarmonyOS, Web | Small string values; native stores or files on native targets, localStorage on Web. This is not encrypted secret storage. |

Use [`bind`](https://daybrite.dev/api/day_part_prefs/fn.bind.html) to restore a signal’s saved value and persist later changes. For files, use
[app-local file storage](fs.md). Neither API is an encrypted store for passwords or tokens.

A direct [`day-part-prefs`](https://daybrite.dev/api/day_part_prefs/index.html) dependency remains available. The `day::prefs` and `day_part_prefs`
paths expose the same API. Disabling the facade’s default features removes its preferences
dependency; enable any other default features your app still needs explicitly.

## Authoring

```rust
day::prefs::set("greeting", "hello");                // persist a value
assert_eq!(day::prefs::get("greeting").as_deref(), Some("hello"));
assert!(day::prefs::contains("greeting"));
day::prefs::remove("greeting");                      // delete it
```

| Function | Behavior |
|---|---|
| [`set(key, value) -> bool`](https://daybrite.dev/api/day_part_prefs/fn.set.html) | Persist `value` under `key`, overwriting. `true` when the write committed. |
| [`get(key) -> Option<String>`](https://daybrite.dev/api/day_part_prefs/fn.get.html) | The stored string, or `None` if absent. A stored `""` is `Some("")`. |
| [`remove(key) -> bool`](https://daybrite.dev/api/day_part_prefs/fn.remove.html) | Delete the value; `true` only if it existed and was removed. |
| [`contains(key) -> bool`](https://daybrite.dev/api/day_part_prefs/fn.contains.html) | Whether a value is currently stored under `key`. |
| [`bind(key, signal)`](https://daybrite.dev/api/day_part_prefs/fn.bind.html) | Two-way-bind a `Signal<T>` to a stored value: seed from the store now, persist every later change. `T` round-trips through `FromStr`/`ToString`. Call it right after creating the signal; the write-back stops with the creating scope. |
| [`bind_json(key, store)`](https://daybrite.dev/api/day/prefs/fn.bind_json.html) | (`day::prefs`, with the `model` feature.) Keep a keyed [day-model](/docs/explanation/model) store (`Store<Keyed<T>>`, `T: Serialize + DeserializeOwned`) under `key` as one JSON array: seed it from the saved rows now, then save every later change, a field edit as well as an insert, removal or move. An unreadable value leaves the store as it is and logs a warning. Suits a small list; larger documents belong in [day-persistence](persistence.md). |
| [`install_nav_store()`](https://daybrite.dev/api/day_part_prefs/fn.install_nav_store.html) | Install this store as day-core's navigation-persistence sink, so a [`nav`](https://daybrite.dev/api/day_pieces/fn.nav.html)/[`nav_stack`](https://daybrite.dev/api/day_pieces/fn.nav_stack.html) marked [`.restore(key)`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.restore) remembers its state across launches (and an Android process death). [`day::launch`](https://daybrite.dev/api/day/fn.launch.html) calls it when the app has installed no store of its own, so an app calls it only when it boots without the facade. Nav keys are namespaced under `day.nav.`. See [navigation](navigation.md). |

Values persist across launches. Platform selection is purely `#[cfg(target_os)]` (plus
`#[cfg(target_arch = "wasm32")]` for the web), since persistence depends on the OS, not on which
widget toolkit is in use. The one cargo feature, `memory`, swaps every platform store for an
in-process map per thread; `day`'s headless `mock` toolkit turns it on, so an app's `cargo test`
starts with empty preferences and never touches the user's real ones. A test that restarts its
own process to check persistence gives the children `DAY_PREFS_DIR=<dir>`: the file store in that
directory then serves in place of the map, private to that test. The file store (Linux, Windows,
HarmonyOS) honors `DAY_PREFS_DIR` in every build. `parts/day-part-prefs/examples/prefs.rs` is a plain `main` that uses it with no
Day framework at all (run it twice to watch a value survive the process).

This is a small string store for user settings and lightweight app state, not a database. Keep
values modest; large blobs belong in a file.

## Per-platform native realization

Platform guidance: [Apple: settings](https://developer.apple.com/design/human-interface-guidelines/settings); [Android: saving key-value data](https://developer.android.com/training/data-storage/shared-preferences).

| OS | API | dependency |
|---|---|---|
| macOS | [`NSUserDefaults.standard`](https://developer.apple.com/documentation/foundation/nsuserdefaults) | `objc2` + `objc2-foundation`, shared `apple.rs` |
| iOS | [`NSUserDefaults.standard`](https://developer.apple.com/documentation/foundation/nsuserdefaults) | `objc2` + `objc2-foundation`, shared `apple.rs` |
| Android | [`SharedPreferences`](https://developer.android.com/reference/android/content/SharedPreferences) (`MODE_PRIVATE`) via a Java shim | `day-android` + `[package.metadata.day.android]` |
| Linux | per-app file store under [`$XDG_CONFIG_HOME/day`](https://specifications.freedesktop.org/basedir/latest/) (or `~/.config/day`) | std only, shared `file.rs` |
| Windows | per-app file store under `%APPDATA%\day` | std only, shared `file.rs` |
| HarmonyOS | file store, best-effort in the app sandbox (`target_env = "ohos"`) | std only, shared `file.rs` |
| Web | `localStorage` (per origin, `day.pref.` namespace) via the day-dom shim | the `web-dom` host page ([docs/web](web.md)), `web.rs` |

## What each platform does

- **macOS / iOS**: [`NSUserDefaults.standard`](https://developer.apple.com/documentation/foundation/nsuserdefaults) is the system's per-application preferences store (a
  plist under `~/Library/Preferences` on macOS, the app container on iOS). It is toolkit-independent
  (no [`NSApplication`](https://developer.apple.com/documentation/appkit/nsapplication)/[`UIApplication`](https://developer.apple.com/documentation/uikit/uiapplication), run loop, or window), so the crate works in `day-qt` binaries
  and plain `cargo test` processes as well as under `day-appkit`/`day-uikit`. `setObject:forKey:` is
  the only `unsafe` objc2 call (the value must be a property-list type, and we always pass a real
  [`NSString`](https://developer.apple.com/documentation/foundation/nsstring)); writes are immediately readable and flushed to disk by the system. [`set`](https://daybrite.dev/api/day_part_prefs/fn.set.html) always
  returns `true`.
- **Android**: an app-private [`SharedPreferences`](https://developer.android.com/reference/android/content/SharedPreferences) file named `day_part_prefs`, opened with
  `Context.MODE_PRIVATE` from `day-android`'s cached `Context` (`DayBridge.ctx`). Writes use
  `Editor.commit()` (synchronous), so [`set`](https://daybrite.dev/api/day_part_prefs/fn.set.html)/[`remove`](https://daybrite.dev/api/day_part_prefs/fn.remove.html) return the true commit result. No manifest
  permission is required; [`SharedPreferences`](https://developer.android.com/reference/android/content/SharedPreferences) is private storage. Like the UI pieces, the crate
  stages its own Java shim through `[package.metadata.day.android]` and rides the Day runtime (it
  needs the app's JVM + `Context`).
- **Linux / Windows / HarmonyOS**: a file-backed store, one flat `String -> String` map serialized
  under `<config-dir>/day/<app-id>.store`, where the app id is `Day.toml`'s `[app].id` as
  `day build`/`day launch` export it (`DAY_APP_ID`; a binary built outside the CLI shares
  `day-part-prefs.store`). The file is new per app: nothing is read from the
  shared file earlier builds wrote, by design (apps started over when the store became
  per-app, 2026-10). `config-dir` is `$XDG_CONFIG_HOME` (else `~/.config`)
  on Linux, `%APPDATA%` (else `%USERPROFILE%\AppData\Roaming`) on Windows, and a best-effort app files
  dir on HarmonyOS. Each entry is a line `escaped_key=escaped_value`; the escaper removes every raw
  `=`, newline, and carriage return (`\` → `\\`, newline → `\n`, CR → `\r`, `=` → `\e`), so the first
  raw `=` on a line is unambiguously the separator and a value may contain anything. Writes are
  best-effort atomic (write a sibling temp file, then rename over the target); a process-wide mutex
  serializes load-modify-save cycles. Every read tolerates a missing, unreadable, or corrupt file
  by treating the store as empty, so a partial write or a hand-edit can never panic a caller. [`set`](https://daybrite.dev/api/day_part_prefs/fn.set.html)
  returns `false` only when the store file could not be written (e.g. a read-only home). It depends
  on `std` only.
- **Web**: `localStorage`, reached through the day-dom host shim's `day_dom_pref_*`
  imports under a `day.pref.` key namespace. Values survive reloads and browser restarts,
  scoped per origin. `localStorage` can throw (private browsing, storage pressure); failures
  report as uncommitted/absent, matching the contract everywhere else. The showcase's Controls
  page binds its state through this store, so a reload keeps the counter ([docs/web](web.md)). On
  wasm outside a day-dom host page the imports are unresolved and instantiation fails.
- **Any other platform**: a no-op store: [`set`](https://daybrite.dev/api/day_part_prefs/fn.set.html)/[`remove`](https://daybrite.dev/api/day_part_prefs/fn.remove.html)/[`contains`](https://daybrite.dev/api/day_part_prefs/fn.contains.html) return `false`, [`get`](https://daybrite.dev/api/day_part_prefs/fn.get.html) returns
  `None`.

## What it shows about the extension system

Like [`day-part-device`](/docs/reference/battery) and [`day-part-connectivity`](/docs/reference/network), this is a headless external crate: it has no UI Piece
and registers nothing in any toolkit's `RENDERERS` slice. On Android it stages its own Java shim
through `[package.metadata.day.android]` (with no permission this time, since private storage needs
none), which `day build` folds into the app's Gradle build without touching any core day crate. On
every other platform it is fully day-independent (pure FFI on Apple, pure `std` file I/O on desktop).
See [docs/extending](extending.md).

## Settings and About

`pieces/day-piece-settings` ([docs/settings](settings.md)) is the typed, declarative layer
over this store: `Pref<T>` statics, groups and rows, the Settings and About surfaces, and the
env-wins rule for `DAY_THEME`/`DAY_LOCALE`. Its default `Store` is this crate. When testing
persistence by hand on macOS, unbundled binaries store under the process-name defaults domain
(`defaults delete <name>` clears it; a plist delete alone won't, because cfprefsd caches).

## Binding failures

`bind<T>` requires `FromStr + ToString + Clone + 'static`. Invalid saved text leaves the initial
signal value unchanged. It persists later watched changes, not the initial default. Its callback
ignores [`set`](https://daybrite.dev/api/day_part_prefs/fn.set.html)'s boolean result; use explicit writes when the UI must report a failure. Keep UI
messages in generated accessors and treat stored strings as data.

The example values above are synthetic fixtures, not UI translations. Compile-designated code
is in the [storage recipe](/docs/how-to/guide-storage). See the
[preferences API](../parts/day-part-prefs/src/lib.rs), including the memory feature and its
`DAY_PREFS_DIR` test override.

## Typed preferences without a settings surface

[`Pref<T>`](https://daybrite.dev/api/day_part_prefs/struct.Pref.html), `PrefValue`, `Store`,
`NativeStore`, `MemoryStore`, `install_store`, `persistence_failures`, and `pref_enum!` belong
to `day-part-prefs`. The default facade exposes them through `day::prefs`. These APIs depend
on the reactive runtime, but do not pull in the settings UI or a widget toolkit.

Declare a preference once, with a stable key and a default function:

```rust
static SHOW_GRID: day::prefs::Pref<bool> = day::prefs::Pref::new("editor.grid", || true);
```

The first read seeds one process-lived signal per key. Writes update that signal and persist
through the installed store; writing the default removes the stored override. Store failures
leave the in-memory value intact and appear in [`persistence_failures()`](https://daybrite.dev/api/day_part_prefs/fn.persistence_failures.html). Install a custom
store before reading preferences. Replacing the store does not reseed existing signals.

The optional [settings surface](/docs/reference/settings) re-exports these types and adds
rows, groups, providers, and presentation. Both entry points use the same store and signals.
