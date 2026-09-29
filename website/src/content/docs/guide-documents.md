---
title: Opening documents
description: "Register file types, import files, and handle Finder, file-manager and installed web app activations."
order: 34
section: Guides
---

Declare supported files in `Day.toml`:

```toml
[[file_types]]
extensions = ["epub"]
mime_types = ["application/epub+zip"]
apple_uti = "org.idpf.epub-container"
```

Day generates the native file association. This enables **Open With** and, on macOS,
dropping a file onto the app's Finder or Dock icon. It does not make the app the default handler.
Use exact MIME types and extensions without leading dots. Use the established Apple UTI when
one exists.

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
the desktop File menu with `Shortcut::new("o")`, and in a mobile toolbar. See
[Reusable commands](/docs/guide-commands/).

The app decides what an open request does. A reader can import into its private library,
deduplicate by content hash, and call `day::open_window` with a stable book key on desktop.
On mobile, it can navigate to the reader in the current window. File registration does not
create windows or overwrite documents automatically.

## Platform differences

- Apple bundles carry document types and imported UTIs. AppKit receives open-document events;
  UIKit receives app/scene URL contexts and imports copies. Native reads retain security-scoped
  access for the duration of the read.
- Android and HarmonyOS copy granted provider files asynchronously into temporary app cache.
  Persist accepted content in your own storage. Staging is limited to 512 MiB.
- Linux packages include MIME associations and file arguments in their desktop entry.
- Windows MSIX and NSIS packages register Open With handlers. Windows GTK/Qt development
  builds emit an opt-in `register.reg`; their distribution packaging still requires a custom
  installer. Neither build nor install replaces the user's default association.
- macOS GTK/Qt builds with declared file types produce development app bundles for Finder/Dock
  testing. These reference local resources and installed toolkit libraries; they are not
  redistributable packages.
- Installed web apps use the browser's File Handling API when available. Unsupported browsers
  and ordinary tabs still use the file picker.

Some desktop activations start a new process rather than contacting an existing one. Document
edit/write-back roles and persistent mobile provider grants are not part of this API.

The [document reference](/docs/internal/documents) covers the full platform mapping, cache
lifetime, command-line delivery, manifest fields, testing, and native API references.
