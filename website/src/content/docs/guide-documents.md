---
navTitle: "Documents"
title: Opening documents
description: "Register file types, import files, and handle Finder, file-manager and installed web app activations."
order: 34
section: How-to guides
---

Declare supported files in `Day.toml`:

```toml
[[file_types]]
extensions = ["epub"]
mime_types = ["application/epub+zip"]
apple_uti = "org.idpf.epub-container"
```

Day generates the native file association. This enables **Open With** and, on macOS,
dropping a file onto the app's Finder or Dock icon. Use exact MIME types and extensions without
leading dots. Use the established Apple UTI when one exists.

For a format the app itself defines, say so, and say how the app uses it:

```toml
[[file_types]]
extensions = ["daynote"]
mime_types = ["application/x-daynote"]
name = "file-type-daynote"   # a Fluent message: the type's name in Finder and Explorer
role = "editor"               # viewer (default) | editor | none
rank = "owner"                # alternate (default) | default | owner | none
exported = true               # the app defines this type
```

An exported type becomes the app's own on Apple platforms and gets a MIME type on Linux. With
`rank = "owner"`, the Windows installer also makes the app the default for those files.

Register a receiver in the app root:

```rust
day::on_open_files(move |files| {
    day::task(async move {
        for locator in files {
            let file = FileUrl::new(locator);
            match file.read_limited(64 * 1024 * 1024).await {
                Ok(bytes) => import_book(bytes).await,
                Err(error) => show_import_error(error),
            }
        }
    });
});
```

`import_book` and `show_import_error` are application functions. Validate the contents and use
localized error messages. The callback runs on the UI thread; native file reads run on a worker.
Requests received before the root mounts are queued. Registration ends when its reactive scope
is disposed.

Use the same import function for the native file picker. A reusable command can expose it in
the desktop File menu with [`Shortcut::primary("o")`](https://docs.rs/day-spec/latest/day_spec/struct.Shortcut.html#method.primary), and in a mobile toolbar. See
[Reusable commands](/docs/how-to/guide-commands/).

The app decides what an open request does. A reader can import into its private library,
deduplicate by content hash, and call [`day::open_window`](https://docs.rs/day-app/latest/day/fn.open_window.html) with a stable book key on desktop.
On mobile, it can navigate to the reader in the current window. File registration does not
create windows or overwrite documents automatically.

## Compile a bounded file reader

`FileUrl` belongs to the facade prelude. This function returns bytes to an app-owned importer;
it does not infer a file format or display raw error text. Use it from an [`on_open_files`](https://docs.rs/day-app/latest/day/fn.on_open_files.html)
callback through [`day::task`](https://docs.rs/day-app/latest/day/fn.task.html). The other Rust block above is an integration fragment.

Complete module for `src/documents.rs` in the [technical recipe harness](https://github.com/daybrite/day/blob/main/docs/snippet-catalog.md#technical-recipes).
The harness compiles this exact block; it is not a standalone app. Add the catalog entries below
to `resource/locales/en/app.ftl` and declare the module in `src/lib.rs`.

<!-- technical-example: documents -->
```rust
use day::prelude::*;

pub async fn read_import(locator: String) -> std::io::Result<Vec<u8>> {
    FileUrl::new(locator).read_limited(64 * 1024 * 1024).await
}
```

<!-- technical-catalog: documents -->
```ftl
import_failed = The document could not be opened.
```

The 64 MiB bound is a recipe policy; choose a limit for your format. Map IO errors to localized
messages such as `res::str::import_failed()` on the UI thread. Compile validation does not test
file associations, provider grants, or Finder activation.

## Platform differences

- Apple bundles carry document types with their role and rank, and exported or imported UTIs. AppKit receives open-document events;
  UIKit receives app/scene URL contexts and imports copies. Native reads retain security-scoped
  access for the duration of the read.
- Android and HarmonyOS copy granted provider files asynchronously into temporary app cache.
  Persist accepted content in your own storage. Staging is limited to 512 MiB.
- Linux packages include MIME associations and file arguments in their desktop entry, and a
  shared-mime-info package for the app's own types.
- Windows MSIX and NSIS packages register Open With handlers. Windows GTK/Qt development
  builds emit an opt-in `register.reg`; their distribution packaging still requires a custom
  installer. The NSIS installer makes the app the default only for `rank = "owner"` types, and
  restores the previous default on uninstall.
- macOS GTK/Qt builds with declared file types produce development app bundles for Finder/Dock
  testing. These reference local resources and installed toolkit libraries; they are not
  redistributable packages.
- Installed web apps use the browser's File Handling API when available. Unsupported browsers
  and ordinary tabs still use the file picker.

Some desktop activations start a new process rather than contacting an existing one. The
`editor` role tells the system the app edits these files; the app still receives a copy to read,
and persistent mobile provider grants are not part of this API.

The [document reference](/docs/reference/documents) covers the full platform mapping, cache
lifetime, command-line delivery, manifest fields, testing, and native API references.
