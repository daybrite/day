---
title: "File pickers"
description: "Native open and save dialogs, scoped access on sandboxed platforms, and the piece-level API."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Files: native open & save pickers

Day opens and saves files through each platform's native file-interaction UI, using the same
imperative request→response model as [dialogs](./dialogs.md): an action opens a picker and
`.await`s the result:

```rust
button(res::str::open()).action(move || {
    day::task(async move {
        if let Some(file) = open_file()
            .filter(res::str::text_files().format(), &["txt", "md"])
            .await
        {
            // `day::task` takes a `Future<Output = ()>`, so handle the error here
            // rather than reaching for `?`.
            match file.read_to_string() {
                // FileUrl::read_to_string
                Ok(text) => editor.set(text),
                Err(e) => status.set(res::str::operation_failed(e.to_string()).format()),
            }
        }
    });
});

button(res::str::save()).action(move || {
    day::task(async move {
        let saved: Option<FileUrl> = save_file(editor.get_untracked().into_bytes())
            .suggested_name("notes.txt")
            .filter(res::str::text_files().format(), &["txt"])
            .await;
    });
});
```

For OS file associations, Finder/Dock drops, and cold/warm document delivery, see
[Document types and file activation](./documents.md). Use [`FileUrl::read_limited(limit).await`](https://daybrite.dev/api/day_pieces/struct.FileUrl.html#method.read_limited)
for bounded reads that do not block the native UI thread.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](/docs/how-to/localization).

## The path type: `FileUrl`

A file location crosses back as a **`FileUrl`**, a newtype wrapping a single *locator string*.
Each platform hands back a different kind of locator, and the type stores whichever arrives:

- **A string, because of Android:** the Storage Access Framework returns a `content://` URI
  rather than a filesystem path; a `std::path::PathBuf` cannot represent it and `std::fs`
  cannot open it.
- **A newtype, for one parser:** the accessors below live in one place instead of at every
  call site.
- **Stored verbatim:** `url::Url` normalizes and validates in ways that mangle `content://`
  authorities, and it would add a large dependency, so the locator is kept as received.

`FileUrl` is the lossless union (a filesystem path on desktop/iOS, a `content://` URI on Android)
with accessors:

| method | result |
|---|---|
| [`as_str()`](https://daybrite.dev/api/day_pieces/struct.FileUrl.html#method.as_str) | the raw locator |
| [`local_path() -> Option<PathBuf>`](https://daybrite.dev/api/day_pieces/struct.FileUrl.html#method.local_path) | `Some` for filesystem paths (and `file://`), `None` for `content://` |
| [`file_name() -> Option<String>`](https://daybrite.dev/api/day_pieces/struct.FileUrl.html#method.file_name) | the last path component, for display |
| [`read`](https://daybrite.dev/api/day_pieces/struct.FileUrl.html#method.read)` () /  `[`read_to_string`](https://daybrite.dev/api/day_pieces/struct.FileUrl.html#method.read_to_string)`()` | the bytes / UTF-8 text (local paths; `content://` errors) |

**Opened files are always readable.** Where a platform doesn't hand back a usable path, the
toolkit materializes one first: Android copies the picked document into the app cache, and iOS
imports it into the app sandbox. So [`open_file`](https://daybrite.dev/api/day_pieces/fn.open_file.html)`().await?`[`.read_to_string`](https://daybrite.dev/api/day_pieces/struct.FileUrl.html#method.read_to_string)`()` works on every
target.

## The builders (`day-pieces`, in the prelude)

- [`open_file()`](https://daybrite.dev/api/day_pieces/fn.open_file.html) → `OpenFile`: [`.title(..)`](https://daybrite.dev/api/day_pieces/struct.OpenFile.html#method.title), [`.filter(name, &["ext", …])`](https://daybrite.dev/api/day_pieces/struct.OpenFile.html#method.filter), `.await → Option<FileUrl>`.
- [`open_files()`](https://daybrite.dev/api/day_pieces/fn.open_files.html) → `OpenFiles`: the same [`.title(..)`](https://daybrite.dev/api/day_pieces/struct.OpenFiles.html#method.title) and [`.filter(..)`](https://daybrite.dev/api/day_pieces/struct.OpenFiles.html#method.filter),
  `.await → Vec<FileUrl>`: every file the user picked, in the picker's order, and an empty
  `Vec` on cancel. The picker allows a multiple selection; a toolkit without one still answers
  with the single pick.
- [`open_folder()`](https://daybrite.dev/api/day_pieces/fn.open_folder.html) → `OpenFolder`: [`.title(..)`](https://daybrite.dev/api/day_pieces/struct.OpenFolder.html#method.title), `.await → Option<FileUrl>`: the chosen
  directory, or `None` on cancel. Only toolkits that answer `Cap::FolderDialog` as `Native` show
  a chooser; the others resolve `None` at once, so an app checks the capability before offering
  an "Add Folder…" command (the phones answer `Unsupported`).
- [`save_file(data)`](https://daybrite.dev/api/day_pieces/fn.save_file.html) → `SaveFile`: [`.title(..)`](https://daybrite.dev/api/day_pieces/struct.OpenFile.html#method.title), [`.suggested_name(..)`](https://daybrite.dev/api/day_pieces/struct.SaveFile.html#method.suggested_name), [`.filter(..)`](https://daybrite.dev/api/day_pieces/struct.OpenFile.html#method.filter),
  `.await → Option<FileUrl>`. The bytes are staged to an app-writable temp file that the toolkit
  hands to the native save UI; the pieces layer delivers them to a chosen local destination and
  cleans up.

```rust
// A music library's "Add Files…" and "Add Folder…" commands.
let tracks: Vec<FileUrl> = open_files()
    .filter(res::str::audio_files().format(), &["mp3", "flac", "m4a"])
    .await;

// Ask `capability(Cap::FolderDialog)` before offering the command (docs/cover.md shows the
// same probe for the status bar); phones answer `Unsupported`.
if capability(Cap::FolderDialog) == Support::Native
    && let Some(folder) = open_folder().title(res::str::add_folder().format()).await
{
    scan(folder.local_path());
}
```

Both probes and the enums are in the prelude ([`capability`](https://daybrite.dev/api/day_core/fn.capability.html), `Cap`, `Support`).

## Per-toolkit native mapping

Platform guidance: [Apple: file management](https://developer.apple.com/design/human-interface-guidelines/file-management); [Android: accessing shared documents](https://developer.android.com/training/data-storage/shared/documents-files).

| Toolkit | Open (one or several) | Folder | Save |
|---|---|---|---|
| appkit | [`NSOpenPanel`](https://developer.apple.com/documentation/appkit/nsopenpanel) (sheet; `allowsMultipleSelection`, every `URLs` entry reported) | `NSOpenPanel` with `canChooseDirectories` | [`NSSavePanel`](https://developer.apple.com/documentation/appkit/nssavepanel) (sheet) |
| uikit  | [`UIDocumentPickerViewController`](https://developer.apple.com/documentation/uikit/uidocumentpickerviewcontroller) (`.import`; `allowsMultipleSelection`, each pick copied into the sandbox) | none: resolves `None` | [`UIDocumentPickerViewController`](https://developer.apple.com/documentation/uikit/uidocumentpickerviewcontroller) (export) |
| gtk    | [`GtkFileDialog.open` / `open_multiple`](https://docs.gtk.org/gtk4/class.FileDialog.html) (GTK 4.10+) | `GtkFileDialog.select_folder` | [`GtkFileDialog.save`](https://docs.gtk.org/gtk4/class.FileDialog.html) |
| qt     | [`QFileDialog`](https://doc.qt.io/qt-6/qfiledialog.html) (`ExistingFile` / `ExistingFiles`) via the C++ shim | `QFileDialog` (`Directory` + `ShowDirsOnly`) | [`QFileDialog`](https://doc.qt.io/qt-6/qfiledialog.html) (`AnyFile`/`AcceptSave`) |
| android | `ACTION_OPEN_DOCUMENT` (+ `EXTRA_ALLOW_MULTIPLE`, every `ClipData` item) + [`ContentResolver`](https://developer.android.com/reference/android/content/ContentResolver) (copy → cache) | none: resolves `None` | `ACTION_CREATE_DOCUMENT` + [`ContentResolver`](https://developer.android.com/reference/android/content/ContentResolver) |
| arkui (HarmonyOS) | ArkTS `DocumentViewPicker.select` (`maxSelectNumber` for several) + `@ohos.file.fs` (copy → cache) | none: resolves `None` | `DocumentViewPicker.save` + `@ohos.file.fs` |
| dom (web) | `<input type=file>` (`multiple` for several; the browser's picker) | none: resolves `None` | a Blob download |
| mock   | records the spec; resolved programmatically | same | same |
| WinUI  | WinRT `FileOpenPicker` (`PickSingleFileAsync` / `PickMultipleFilesAsync`) | WinRT `FolderPicker` | WinRT `FileSavePicker` |

`Cap::FolderDialog` is `Native` exactly where the Folder column names a chooser: the four
desktops. The phone and web toolkits answer `Unsupported` and resolve an [`open_folder()`](https://daybrite.dev/api/day_pieces/fn.open_folder.html) request
as cancelled at once, so the await never hangs; an app that offers "Add Folder…" asks the
capability first and hides the command elsewhere.

On HarmonyOS the picker lives in the ArkTS `@kit.CoreFileKit` layer, not the native NodeAPI, so
the `day-toolkit-arkui` toolkit calls up into its ArkTS host over NAPI (safe: Day's loop runs on the
JS thread); the host drives `DocumentViewPicker` and answers via a registered `onFileResult`
callback, wired in the framework's ArkTS host page (`crates/day-toolkit-arkui/platform/harmony/ets/Index.ets`,
staged into every app's hvigor project by `day build` — [docs/harmonyos](harmonyos.md)).

A browser has no filesystem, so on Web the bytes ride a per-page store instead of paths:
an opened file's content lands under a virtual `/day-web/<name>` path that [`FileUrl::read`](https://daybrite.dev/api/day_pieces/struct.FileUrl.html#method.read)
resolves, and a save's staged bytes leave as a download named by [`suggested_name`](https://daybrite.dev/api/day_pieces/struct.SaveFile.html#method.suggested_name). The app uses
the same builders and the same `FileUrl` surface on web as on every other target.

All toolkits present the picker non-blocking (sheet / `open()` / delegate / Activity result),
so the main loop keeps running and dayscript stays live while a picker is up.

## Plumbing

Files go through the existing [`present`](https://daybrite.dev/api/day_core/fn.present.html) path ([docs/dialogs](dialogs.md)) and add no `Toolkit` methods:

- `day_spec::present::PresentSpec::{OpenFile, OpenFolder, SaveFile}` + `FileFilter { name, extensions }`.
  `OpenFile { multiple }` is the one spec behind both [`open_file()`](https://daybrite.dev/api/day_pieces/fn.open_file.html) (`false`) and
  [`open_files()`](https://daybrite.dev/api/day_pieces/fn.open_files.html) (`true`); `OpenFolder { title }` carries the folder chooser.
- `PresentResult::Files(Vec<String>)`: the chosen locators, crossing the C ABI (Qt shim /
  Android JNI / WinUI shim / ArkTS host) as tag `3` with the paths joined by the unit separator.
  A multiple-selection pick reports every path, in the picker's order.
- `Cap::FileDialogs` advertises native open/save pickers; `Cap::FolderDialog` the folder chooser.
- [`day_spec::present::app_temp_dir()`](https://daybrite.dev/api/day_spec/present/fn.app_temp_dir.html): the app-writable staging dir; Android sets it to
  `getCacheDir()` (the OS temp dir isn't app-writable there).

## dayscript

A file picker is a presentation, so a script answers it with a path ([docs/dialogs](dialogs.md)):

```yaml
- tap: { id: btn_save_file }
- assert_presented: {}
- respond: { path: "notes.txt" }        # relative → the app temp dir (writable on every target)
- tap: { id: btn_open_file }
- assert_presented: {}
- respond: { path: "notes.txt" }         # reads the file just written: a real round-trip
```

This makes open/save flows headless-testable and screenshot-able on every toolkit without touching
the machine's real filesystem. See `Day-Showcase` (the **Files & storage** page's Files section) and
`Day-Showcase/dayscript/files.yaml`. The same `respond: { path }` answers an [`open_files()`](https://daybrite.dev/api/day_pieces/fn.open_files.html) pick
(with that one file) and an [`open_folder()`](https://daybrite.dev/api/day_pieces/fn.open_folder.html) request (with that path as the directory), on every
toolkit, including the ones without a native folder chooser.

## Windows picker behavior

The WinUI toolkit advertises `Cap::FileDialogs = Native` and `Cap::FolderDialog = Native`. It
uses the system WinRT pickers (`FileOpenPicker`, `FolderPicker`, `FileSavePicker`), initialized
with the host HWND through [`IInitializeWithWindow`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-iinitializewithwindow) (the
[desktop picker interop contract](https://devblogs.microsoft.com/oldnewthing/20190412-00/?p=102413)).
Open filters become `.ext` entries, with `*` when no filters are supplied (the folder picker
always carries `*`, which the desktop requires); save filters retain their names. The WinRT API
does not expose a custom dialog title. A several-files pick goes through
`PickMultipleFilesAsync` and reports every chosen path.
The selected local path becomes a `FileUrl`, so [`read()`](https://daybrite.dev/api/day_pieces/struct.FileUrl.html#method.read) returns the original bytes.

Registration precedes native setup. Missing owners, activation/filter/initialization
failures, canceled operations, and errors retrieving the result or path resolve as
`Dismissed`, rather than leaving the calling future pending. Completion removes the
registration before notifying Rust; duplicate or late replies are ignored.

Validate on a Windows host with `Day-Showcase/dayscript/files.yaml`, then manually
open and cancel the native picker and select a file with a Unicode path. In Day-Sketch,
insert an image through the **+** menu, edit its geometry/opacity/rotation, save the
drawing, and reopen it. Scripted `respond` validates the portable presentation flow;
it does not select a file through the real Windows picker UI.

## macOS sandboxing

See [macOS App Sandbox](sandbox.md) for Day.toml configuration and persistent file access.
[`FileUrl::bookmark(read_only)`](https://daybrite.dev/api/day_pieces/struct.FileUrl.html#method.bookmark) returns opaque bookmark bytes;
[`FileUrl::resolve_bookmark(bytes)`](https://daybrite.dev/api/day_pieces/struct.FileUrl.html#method.resolve_bookmark) returns a `FileAccess` guard. Keep it alive through I/O and
renew stale bookmarks while the guard is held. A saved path alone does not preserve permission.
