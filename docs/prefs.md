---
navTitle: "Preferences"
title: "Preferences storage"
description: "Persistent key-value preferences via day-part-preferences, stored in each platform's native store."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Persistent preferences

Preferences remember small choices, such as a theme or playback volume, between app sessions. They
let people return to the settings they chose instead of configuring the app each time.

`day::prefs` stores string values through
[`day-part-preferences`](https://docs.rs/day-part-preferences/latest/day_part_preferences/index.html),
re-exported by `day` through the default-enabled `prefs` feature. Apps using the facade do not need
a separate dependency.

## Add to your app

The default-enabled `day::prefs` re-export needs no additional dependency.
For direct use, the API is defined in [`day-part-preferences`](https://github.com/daybrite/day/tree/main/crates/day-part-preferences). Add it to your app
when you need it directly:

```sh
cargo add day-part-preferences
```

The implementation is selected by the compilation target; no toolkit feature flags
or dependency entry in `Day.toml` are needed.

| Platforms / toolkits | Availability |
| --- | --- |
| macOS, iOS, Android, Windows, Linux (GTK / Qt), HarmonyOS, Web | Small string values; native stores or files on native targets, localStorage on Web. This is not encrypted secret storage. |

Use [`bind`](https://docs.rs/day-part-preferences/latest/day_part_preferences/fn.bind.html) to restore a signal’s saved value and persist later changes. For files, use
[app-local file storage](fs.md). Neither API is an encrypted store for passwords or tokens.

A direct [`day-part-preferences`](https://docs.rs/day-part-preferences/latest/day_part_preferences/index.html) dependency remains available. The `day::prefs` and `day_part_preferences`
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
| [`set(key, value) -> bool`](https://docs.rs/day-part-preferences/latest/day_part_preferences/fn.set.html) | Persist `value` under `key`, overwriting. `true` when the write committed. |
| [`get(key) -> Option<String>`](https://docs.rs/day-part-preferences/latest/day_part_preferences/fn.get.html) | The stored string, or `None` if absent. A stored `""` is `Some("")`. |
| [`remove(key) -> bool`](https://docs.rs/day-part-preferences/latest/day_part_preferences/fn.remove.html) | Delete the value; `true` only if it existed and was removed. |
| [`contains(key) -> bool`](https://docs.rs/day-part-preferences/latest/day_part_preferences/fn.contains.html) | Whether a value is currently stored under `key`. |
| [`bind(key, signal)`](https://docs.rs/day-part-preferences/latest/day_part_preferences/fn.bind.html) | Two-way-bind a `Signal<T>` to a stored value: seed from the store now, persist every later change. `T` round-trips through `FromStr`/`ToString`. Call it right after creating the signal; the write-back stops with the creating scope. |
| [`bind_json(key, store)`](https://docs.rs/day-app/latest/day/prefs/fn.bind_json.html) | (`day::prefs`, with the `model` feature.) Keep a keyed [day-model](/docs/explanation/model) store (`Store<Keyed<T>>`, `T: Serialize + DeserializeOwned`) under `key` as one JSON array: seed it from the saved rows now, then save every later change, a field edit as well as an insert, removal or move. An unreadable value leaves the store as it is and logs a warning. Suits a small list; larger documents belong in [day-persistence](persistence.md). |
| [`install_nav_store()`](https://docs.rs/day-part-preferences/latest/day_part_preferences/fn.install_nav_store.html) | Install this store as day-core's navigation-persistence sink, so a [`nav`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav.html)/[`nav_stack`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav_stack.html) marked [`.restore(key)`](https://docs.rs/day-pieces/latest/day_pieces/trait.NavBuilder.html#tymethod.restore) remembers its state across launches (and an Android process death). [`day::launch`](https://docs.rs/day-app/latest/day/fn.launch.html) calls it when the app has installed no store of its own, so an app calls it only when it boots without the facade. Nav keys are namespaced under `day.nav.`. See [navigation](navigation.md). |

Values persist across launches. Platform selection is purely `#[cfg(target_os)]` (plus
`#[cfg(target_arch = "wasm32")]` for the web), since persistence depends on the OS, not on which
widget toolkit is in use. The one cargo feature, `memory`, swaps every platform store for an
in-process map per thread; `day`'s headless `mock` toolkit turns it on, so an app's `cargo test`
starts with empty preferences and never touches the user's real ones. A test that restarts its
own process to check persistence gives the children `DAY_PREFS_DIR=<dir>`: the file store in that
directory then serves in place of the map, private to that test. The file store (Linux, Windows,
HarmonyOS) honors `DAY_PREFS_DIR` in every build. `crates/day-part-preferences/examples/prefs.rs` is a plain `main` that uses it with no
Day framework at all (run it twice to watch a value survive the process).

This is a small string store for user settings and lightweight app state, not a database. Keep
values modest; large blobs belong in a file.

## Platform implementations

Platform guidance: [Apple: settings](https://developer.apple.com/design/human-interface-guidelines/settings); [Android: saving key-value data](https://developer.android.com/training/data-storage/shared-preferences).

| OS | API | dependency |
|---|---|---|
| macOS | [`NSUserDefaults.standard`](https://developer.apple.com/documentation/foundation/nsuserdefaults) | `objc2` + `objc2-foundation`, shared `apple.rs` |
| iOS | [`NSUserDefaults.standard`](https://developer.apple.com/documentation/foundation/nsuserdefaults) | `objc2` + `objc2-foundation`, shared `apple.rs` |
| Android | [`SharedPreferences`](https://developer.android.com/reference/android/content/SharedPreferences) (`MODE_PRIVATE`) via a Java shim | `day-toolkit-material` + `[package.metadata.day.android]` |
| Linux | file store in the app's [`$XDG_CONFIG_HOME/<app id>`](https://specifications.freedesktop.org/basedir/latest/) (or `~/.config/<app id>`) | std only, shared `file.rs` |
| Windows | file store in `%APPDATA%\<app id>` | std only, shared `file.rs` |
| HarmonyOS | file store, best-effort in the app sandbox (`target_env = "ohos"`) | std only, shared `file.rs` |
| Web | `localStorage` (per origin, `day.pref.` namespace) via the day-toolkit-dom shim | the `web-dom` host page ([docs/web](web.md)), `web.rs` |

## What each platform does

- **macOS / iOS**: [`NSUserDefaults.standard`](https://developer.apple.com/documentation/foundation/nsuserdefaults) is the system's per-application preferences store (a
  plist under `~/Library/Preferences` on macOS, the app container on iOS). It is toolkit-independent
  (no [`NSApplication`](https://developer.apple.com/documentation/appkit/nsapplication)/[`UIApplication`](https://developer.apple.com/documentation/uikit/uiapplication), run loop, or window), so the crate works in `day-toolkit-qt` binaries
  and plain `cargo test` processes as well as under `day-toolkit-appkit`/`day-toolkit-uikit`. `setObject:forKey:` is
  the only `unsafe` objc2 call (the value must be a property-list type, and we always pass a real
  [`NSString`](https://developer.apple.com/documentation/foundation/nsstring)); writes are immediately readable and flushed to disk by the system. [`set`](https://docs.rs/day-part-preferences/latest/day_part_preferences/fn.set.html) always
  returns `true`.
- **Android**: an app-private [`SharedPreferences`](https://developer.android.com/reference/android/content/SharedPreferences) file named `day_part_preferences`, opened with
  `Context.MODE_PRIVATE` from `day-toolkit-material`'s cached `Context` (`DayBridge.ctx`). Writes use
  `Editor.commit()` (synchronous), so [`set`](https://docs.rs/day-part-preferences/latest/day_part_preferences/fn.set.html)/[`remove`](https://docs.rs/day-part-preferences/latest/day_part_preferences/fn.remove.html) return the true commit result. No manifest
  permission is required; [`SharedPreferences`](https://developer.android.com/reference/android/content/SharedPreferences) is private storage. Like the UI pieces, the crate
  stages its own Java shim through `[package.metadata.day.android]` and rides the Day runtime (it
  needs the app's JVM + `Context`).
- **Linux / Windows / HarmonyOS:** settings use `preferences.store` in the app's
  [`configuration directory`](https://docs.rs/day-app/latest/day/storage/fn.config_dir.html).
  See [storage locations](fs.md#where-files-live) for defaults and overrides. A missing or
  unreadable file is treated as an empty store. Writes replace the file through a temporary
  sibling file; [`set`](https://docs.rs/day-part-preferences/latest/day_part_preferences/fn.set.html)
  returns `false` if saving fails.
- **Web**: `localStorage`, reached through the day-toolkit-dom host shim's `day_dom_pref_*`
  imports under a `day.pref.` key namespace. Values survive reloads and browser restarts,
  scoped per origin. `localStorage` can throw (private browsing, storage pressure); failures
  report as uncommitted/absent, matching the contract everywhere else. The showcase's Controls
  page binds its state through this store, so a reload keeps the counter ([docs/web](web.md)). On
  wasm outside a day-toolkit-dom host page the imports are unresolved and instantiation fails.
- **Any other platform**: a no-op store: [`set`](https://docs.rs/day-part-preferences/latest/day_part_preferences/fn.set.html)/[`remove`](https://docs.rs/day-part-preferences/latest/day_part_preferences/fn.remove.html)/[`contains`](https://docs.rs/day-part-preferences/latest/day_part_preferences/fn.contains.html) return `false`, [`get`](https://docs.rs/day-part-preferences/latest/day_part_preferences/fn.get.html) returns
  `None`.

> [!WARNING]
> **Linux / Windows / HarmonyOS: preference migration.**
> Preferences stored at the previous `<config-dir>/day/<app-id>.store` location are not migrated
> automatically. Apps upgrading from that layout must migrate existing settings to
> `preferences.store` in the app's configuration directory.

## Extension integration

Like [`day-part-device`](/docs/reference/battery) and [`day-part-connectivity`](/docs/reference/network), this is a headless external crate: it has no UI Piece
and registers nothing in any toolkit's `RENDERERS` slice. On Android it stages its own Java shim
through `[package.metadata.day.android]` (with no permission this time, since private storage needs
none), which `day build` folds into the app's Gradle build without touching any core day crate. On
every other platform it is fully day-independent (pure FFI on Apple, pure `std` file I/O on desktop).
See [docs/extending](extending.md).

## Settings and About

`crates/day-piece-settings` ([docs/settings](settings.md)) is the typed, declarative layer
over this store: `Pref<T>` statics, groups and rows, the Settings and About surfaces, and the
env-wins rule for `DAY_THEME`/`DAY_LOCALE`. Its default `Store` is this crate. When testing
persistence by hand on macOS, unbundled binaries store under the process-name defaults domain
(`defaults delete <name>` clears it; a plist delete alone won't, because cfprefsd caches).

## Binding failures

`bind<T>` requires `FromStr + ToString + Clone + 'static`. Invalid saved text leaves the initial
signal value unchanged. It persists later watched changes, not the initial default. Its callback
ignores [`set`](https://docs.rs/day-part-preferences/latest/day_part_preferences/fn.set.html)'s boolean result; use explicit writes when the UI must report a failure. Keep UI
messages in generated accessors and treat stored strings as data.

The example values above are synthetic fixtures, not UI translations. Compile-designated code
is in the [storage recipe](/docs/how-to/guide-storage). See the
[preferences API](../crates/day-part-preferences/src/lib.rs), including the memory feature and its
`DAY_PREFS_DIR` test override.

## Typed preferences without a settings surface

[`Pref<T>`](https://docs.rs/day-part-preferences/latest/day_part_preferences/struct.Pref.html), `PrefValue`, `Store`,
`NativeStore`, `MemoryStore`, `install_store`, `persistence_failures`, and `pref_enum!` belong
to `day-part-preferences`. The default facade exposes them through `day::prefs`. These APIs depend
on the reactive runtime, but do not pull in the settings UI or a widget toolkit.

Declare a preference once, with a stable key and a default function:

```rust
static SHOW_GRID: day::prefs::Pref<bool> = day::prefs::Pref::new("editor.grid", || true);
```

The first read seeds one process-lived signal per key. Writes update that signal and persist
through the installed store; writing the default removes the stored override. Store failures
leave the in-memory value intact and appear in [`persistence_failures()`](https://docs.rs/day-part-preferences/latest/day_part_preferences/fn.persistence_failures.html). Install a custom
store before reading preferences. Replacing the store does not reseed existing signals.

The optional [settings surface](/docs/reference/settings) re-exports these types and adds
rows, groups, providers, and presentation. Both entry points use the same store and signals.
