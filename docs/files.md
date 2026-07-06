# Files: native open & save pickers

Day opens and saves files through each platform's **native file-interaction UI** — the same
imperative request→response model as [dialogs](./dialogs.md), so an action opens a picker and
`.await`s the result:

```rust
button(tr("open")).action(|| day::task(async move {
    if let Some(file) = open_file().filter("Text", &["txt", "md"]).await {
        let text = file.read_to_string()?;   // FileUrl::read_to_string
        editor.set(text);
    }
}));

button(tr("save")).action(|| day::task(async move {
    let saved: Option<FileUrl> = save_file(editor.get_untracked().into_bytes())
        .suggested_name("notes.txt")
        .filter("Text", &["txt"])
        .await;
}));
```

## The path type: `FileUrl`

A file location crosses back as a **`FileUrl`** — a bespoke newtype wrapping a single *locator
string*. This is a deliberate choice over the obvious alternatives:

- **Not `std::path::PathBuf`.** On Android the Storage Access Framework returns a `content://`
  URI, not a filesystem path — a `PathBuf` literally cannot represent it and `std::fs` cannot
  open it.
- **Not a bare `String`.** No type-safety, and every call site would re-implement the same
  parsing.
- **Not `url::Url`.** Its parsing normalizes/validates in ways that mangle `content://`
  authorities, and it pulls in a heavy dependency for no benefit.

`FileUrl` is the lossless union — a filesystem path on desktop/iOS, a `content://` URI on Android
— with ergonomic accessors:

| method | result |
|---|---|
| `as_str()` | the raw locator |
| `local_path() -> Option<PathBuf>` | `Some` for filesystem paths (and `file://`), `None` for `content://` |
| `file_name() -> Option<String>` | the last path component, for display |
| `read() / read_to_string()` | the bytes / UTF-8 text (local paths; `content://` errors) |

**Opened files are always readable.** Where a platform doesn't hand back a usable path, the
backend materializes one first — Android copies the picked document into the app cache, iOS
imports it into the app sandbox — so `open_file().await?.read_to_string()` "just works" on every
target.

## The builders (`day-pieces`, in the prelude)

- `open_file()` → `OpenFile`: `.title(..)`, `.filter(name, &["ext", …])`, `.await → Option<FileUrl>`.
- `save_file(data)` → `SaveFile`: `.title(..)`, `.suggested_name(..)`, `.filter(..)`,
  `.await → Option<FileUrl>`. The bytes are staged to an app-writable temp file that the backend
  hands to the native save UI; the pieces layer delivers them to a chosen local destination and
  cleans up.

## Per-toolkit native mapping

| Toolkit | Open | Save |
|---|---|---|
| appkit | `NSOpenPanel` (sheet) | `NSSavePanel` (sheet) |
| uikit  | `UIDocumentPickerViewController` (`.import`) | `UIDocumentPickerViewController` (export) |
| gtk    | `GtkFileDialog.open` (GTK 4.10+) | `GtkFileDialog.save` |
| qt     | `QFileDialog` (`ExistingFile`) via the C++ shim | `QFileDialog` (`AnyFile`/`AcceptSave`) |
| android | `ACTION_OPEN_DOCUMENT` + `ContentResolver` (copy → cache) | `ACTION_CREATE_DOCUMENT` + `ContentResolver` |
| mock   | records the spec; resolved programmatically | same |
| winui  | not yet implemented (like its alert dialogs) | — |

All backends present the picker **non-blocking** (sheet / `open()` / delegate / Activity result),
so the main loop keeps running — and dayscript stays live — while a picker is up.

## Plumbing

Files ride the existing `present` seam (docs/dialogs.md) rather than adding new `Toolkit` methods:

- `day_spec::present::PresentSpec::{OpenFile, SaveFile}` + `FileFilter { name, extensions }`.
- `PresentResult::Files(Vec<String>)` — the chosen locators, crossing the C ABI (Qt shim /
  Android JNI) as tag `3` with the paths joined by the unit separator.
- `Cap::FileDialogs` advertises native support.
- `day_spec::present::app_temp_dir()` — the app-writable staging dir; Android sets it to
  `getCacheDir()` (the OS temp dir isn't app-writable there).

## dayscript

A file picker is a presentation, so a script answers it with a path (docs/dialogs.md):

```yaml
- tap: { id: btn-save-file }
- assert_presented: {}
- respond: { path: "notes.txt" }        # relative → the app temp dir (writable on every target)
- tap: { id: btn-open-file }
- assert_presented: {}
- respond: { path: "notes.txt" }         # reads the file just written — a real round-trip
```

This makes open/save flows headless-testable and screenshot-able on every backend without touching
the machine's real filesystem. See `apps/showcase` (the **Files** playground) and
`apps/showcase/scripts/files.yaml`.
