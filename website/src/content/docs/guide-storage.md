---
navTitle: "Storage"
title: Local storage
description: "Persistent settings and app-private files on native and web targets."
order: 29
section: How-to guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Persist small settings with `day::prefs`; store user documents and caches with [`day-part-filesystem`](/docs/reference/fs).
Start with a working app. The facade's default `prefs` feature supplies preferences; add
[`day-part-filesystem`](/docs/reference/fs) from the same revision for file operations. Neither store encrypts its contents.

## Persist a setting

Create and bind the signal in the scope that should own persistence. This complete module also
shows explicit writes when the app needs a success flag.

Complete module for `src/storage.rs` in the [technical recipe harness](https://github.com/daybrite/day/blob/main/docs/snippet-catalog.md#technical-recipes).
The harness compiles this exact block; it is not a standalone app. Add the catalog entries below
to `resource/locales/en/app.ftl` and declare the module in `src/lib.rs`.

<!-- technical-example: storage -->
```rust
use crate::res;
use day::prelude::*;

pub fn saved_toggle() -> impl Piece {
    let enabled = Signal::new(false);
    day::prefs::bind("settings.reminders", enabled);
    labeled(
        res::str::storage_reminders(),
        toggle(enabled).id(id!(storage_reminders)),
    )
}

pub fn save_setting(value: bool) -> bool {
    day::prefs::set("settings.reminders", &value.to_string())
}

pub async fn save_document(bytes: Vec<u8>) -> Result<(), day_part_filesystem::FsError> {
    day_part_filesystem::write_future("notes/current.txt", bytes).await
}
```

<!-- technical-catalog: storage -->
```ftl
storage_reminders = Reminders
```

Preference keys and downloaded-file paths are application identifiers, not bundled resources.
The bytes passed to `save_document` are user data. Call the async function inside [`day::task`](https://daybrite.dev/api/day/fn.task.html) and
handle its result there; use a generated localized message for any failure shown to the user.

[`bind`](https://daybrite.dev/api/day_part_preferences/fn.bind.html) reads a stored value immediately and uses it only if `FromStr` succeeds. It watches later
changes and serializes them with `ToString`; it does not save the initial default merely because
you created the binding. Its watch ends with the current scope, and it discards write failures.
Use `set` directly when a failed save must be reported. `get` returns `None` for absence or an
unavailable store; an empty stored value is `Some("")`. [`remove`](https://daybrite.dev/api/day_part_filesystem/fn.remove.html) returns false for absence or a
failed removal, so it cannot distinguish those cases.

With the optional `model` feature, [`day::prefs::bind_json`](https://daybrite.dev/api/day/prefs/fn.bind_json.html) persists a model store. Navigation's
[`.restore(key)`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.restore) uses the installed nav store. See [model state](/docs/explanation/model) and
[navigation](/docs/how-to/navigation) for these separate contracts.

## Read, list, and remove files

Each operation has blocking, callback (`*_async`), and future (`*_future`) forms. Use futures
under [`day::task`](https://daybrite.dev/api/day/fn.task.html) to resume on the UI thread; callbacks require `Send` and must deliver UI data
through a `Setter`. Blocking forms exist on web but return `FsError::Unsupported`.

| Operation | Contract |
| --- | --- |
| [`read_future(path)`](https://daybrite.dev/api/day_part_filesystem/fn.read_future.html) | Returns the whole file as `Vec<u8>` or an error |
| [`write_future(path, bytes)`](https://daybrite.dev/api/day_part_filesystem/fn.write_future.html) | Creates parents; creates or truncates the file; no atomic-replacement guarantee |
| [`remove_future(path)`](https://daybrite.dev/api/day_part_filesystem/fn.remove_future.html) | Removes a file or empty directory; missing paths return `NotFound` |
| [`list_future(dir)`](https://daybrite.dev/api/day_part_filesystem/fn.list_future.html) | Sorted immediate names, directories suffixed `/`; `""` means root; a missing directory is empty |
| [`open_read(path)`](https://daybrite.dev/api/day_part_filesystem/fn.open_read.html) | Native-only, blocking, seekable `std::fs::File` for readers that must avoid whole-file buffers |

Paths must be relative, with no colon, leading separator, empty segment, `.` or `..` segment.
The root exception applies only to listing. These lexical checks are not an OS sandbox or
protection against symlinks planted by another actor. `FsError` is non-exhaustive: handle
`NotFound`, `BadPath`, `Io`, `Unsupported`, and a fallback arm when matching variants.

Dropping an fs future stops listening; native jobs are detached blocking-pool work and may still
write the file. Do not describe cancellation as undoing an accepted save.

## Resolve storage locations

On native targets, `DAY_DATA_DIR` overrides the app-data root. Without it, the implementation
uses the platform data directory plus `day`; file operations add `day-fs/`. This fallback is
shared naming, not proof of isolation between unrelated apps. A host must provide an app-specific
root when isolation is required. Mobile hosts supply their private files directory.

| Target | Fallback before `day/day-fs/` |
| --- | --- |
| macOS / iOS | `~/Library/Application Support/` within the process's home/container |
| Linux | `$XDG_DATA_HOME/`, otherwise `~/.local/share/` |
| Windows | `%APPDATA%/` |
| Web | Origin Private File System; no native path |

Web preferences use localStorage; files use OPFS. Browser storage can be unavailable or fail,
and this API provides no durability promise against browser eviction. Unknown native hosts can
return `Unsupported`. Keep credentials in a separately integrated secure store.

## Verify persistence

Compile the module with the [recipe harness](https://github.com/daybrite/day/blob/main/docs/snippet-catalog.md#technical-recipes).
In the app, change the toggle, restart, and verify restoration. Exercise unavailable storage and
a failed write separately. For isolated tests, enable the prefs `memory` feature and use a
repository-local `DAY_DATA_DIR`; these controls avoid touching real user settings.

Source evidence: [prefs API and bind implementation](https://github.com/daybrite/day/blob/main/crates/day-part-preferences/src/lib.rs),
[fs API](https://github.com/daybrite/day/blob/main/crates/day-part-filesystem/src/lib.rs), and
[native root and IO behavior](https://github.com/daybrite/day/blob/main/crates/day-part-filesystem/src/native.rs).
Detailed lookup: [prefs](/docs/reference/prefs), [files](/docs/reference/fs), and
[async lifetime](/docs/explanation/async).
