---
title: "Document types and file activation"
description: "Declare supported files once, receive OS open events, and import through the native picker."
---

# Document types and file activation

Declare the files an app can open in `Day.toml`. Day generates the platform registration;
application code handles the contents. Registration offers the app in **Open With**. It does
not replace the user's default app.

```toml
[[file_types]]
extensions = ["epub"]
mime_types = ["application/epub+zip"]
apple_uti = "org.idpf.epub-container"
```

`extensions` contains lowercase filename extensions without dots. `mime_types` contains exact
MIME types; wildcards are rejected. Both arrays are required. Supply the established Apple UTI
when one exists. Otherwise Day generates an app-specific imported identifier. Document roles
are currently read-only viewers; editor roles, document icons and exported custom type
conformance hierarchies are not exposed.

## Handle an open request

Register once in the app root. This also receives files dropped onto the app's Dock icon or
opened through a file manager's **Open With** command. The same callback handles cold and warm
activation wherever the platform delivers events to a running process.

```rust
use day::prelude::*;

day::on_open_files(move |locators| {
    day::task(async move {
        for locator in locators {
            let file = FileUrl::new(locator);
            match file.read_limited(64 * 1024 * 1024).await {
                Ok(bytes) => import_book(bytes).await,
                Err(error) => show_import_error(error),
            }
        }
    });
});
```

The example's `import_book` and `show_import_error` are app functions. Localize displayed errors
through generated resource accessors. Treat the contents as untrusted input: the association
and extension do not validate a file's format.

Day queues requests received before registration. Delivery runs on the UI thread, outside the
queue lock, after the root has mounted. A registration is removed with its reactive scope;
replacing it replaces the previous handler. Applications decide whether to queue imports,
reuse an existing document, open a window, or navigate to an existing reader. No window is
created automatically. Use a content hash or stable document identifier to avoid duplicate
imports; a filename alone is insufficient.

Use the same import function from a reusable **Open** command:

```rust
let open = Command {
    id: "open-document",
    label: res::str::open_document(),
    action: move || {
        day::task(async move {
            if let Some(file) = open_file()
                .filter(res::str::epub_files().format(), &["epub"])
                .await
            {
                import_file(file).await;
            }
        });
    },
}.build().icon(Symbol::Open).shortcut(Shortcut::new("o"));
```

Put `open.menu_item()` in a `MenuBarRole::File` submenu and `open.toolbar_item()` in mobile
navigation chrome. [Commands](./commands.md), [file pickers](./files.md), and
[windows](./windows.md) document those APIs.

## File access and lifetime

`FileUrl::read_limited(limit).await` reads on a native worker and returns `InvalidData` when the
limit is exceeded. It holds and releases Apple security-scoped access around the read. Web
files have already been read asynchronously into the browser's virtual file store.

Android and HarmonyOS copy provider-granted files into app cache before delivery, using
asynchronous I/O and a 512 MiB staging limit. Failed staging delivers an unreadable locator so
the app can display its normal import error. These paths are temporary: copy accepted content
into app-owned persistence before returning to the document later. Day does not promise a
persistent grant to the original document or write-back support. Cache and browser staging
entries may outlive an import; applications importing many large files should account for
that storage. Picker and activation requests are distinct from navigation/deep links.

## Platform mapping

| Target | Registration | Delivery and limits |
|---|---|---|
| macOS AppKit | `CFBundleDocumentTypes`, imported UTIs, Viewer role, Alternate handler rank in the built `.app` | `NSApplicationDelegate.application:openURLs:`; Finder, Open With and Dock drops, before or after launch. Requires the bundle, not its bare executable. |
| macOS GTK | Same bundle metadata | `GApplication` open signal. Day creates a development `.app` when file types are declared. |
| macOS Qt | Same bundle metadata | `QFileOpenEvent`; same development bundle policy as GTK. |
| iOS UIKit | Document types, imported UTIs; `LSSupportsOpeningDocumentsInPlace=false` | App delegate and scene URL contexts, including cold connection options. Files imports a copy; reading uses the current app window unless the app chooses otherwise. |
| Android MDC | Exported Activity `ACTION_VIEW`, `CATEGORY_DEFAULT`, exact MIME filters | Cold Intent and `onNewIntent`; `content://` and `file://` provider streams are copied before delivery. No broad external-storage permission is required. |
| Linux GTK / Qt | `.desktop` `MimeType`, `Exec … --day-open-files %U` in Flatpak/AppImage packaging | Explicit launch arguments; GTK also handles `GApplication` open events. Desktop integration must install the desktop entry. Qt activations may start another process. |
| Windows XAML | MSIX file associations and desktop open verb; NSIS per-user Open With ProgIDs | Explicit command-line activation. Another launch may create another process. The installer never replaces the user's default association. |
| Windows GTK / Qt | Generated `build/day/file-types/<target>/register.reg` | Import the file on the Windows development host to register that build. It names the current executable path. These toolkits remain build-only in Day's distribution packer; a custom installer must deploy the toolkit and equivalent registry entries. |
| HarmonyOS ArkUI | EntryAbility `viewData` skills with file/MIME URIs and `FileOpen` | Cold and warm Wants, then async copy using the granted URI. |
| Web DOM | Web manifest `file_handlers` | Feature-detected `launchQueue` consumer. Requires an installed PWA, a supporting browser, and user approval. Ordinary browser tabs and unsupported browsers retain the file picker. |

macOS GTK/Qt development bundles reference local resource roots and installed toolkit libraries.
They are for local Finder/Dock testing, not redistribution. AppKit's normal signed bundle is
unchanged. Installing or moving a bundle lets Launch Services discover its declaration; Day
never changes a user's default file handler as a side effect of building.

Desktop command-line activations accept `--day-open-file <path>` or
`--day-open-files <path>…`. Existing positional files also support shell drops directly onto
an executable. Arbitrary flags and navigation URLs are not treated as documents.

Adding MIME associations does not install a new system-wide MIME database for an unknown
format. Use an established MIME type where possible; custom formats may need additional
installer integration on Linux. OS file associations can be cached: rebuild and reinstall the
app after changing a declaration.

## Native references

- [Apple EPUB type](https://developer.apple.com/documentation/uniformtypeidentifiers/uttype-swift.struct/epub)
- [Apple open-in-place policy](https://developer.apple.com/documentation/bundleresources/information-property-list/lssupportsopeningdocumentsinplace)
- [Android intent filters](https://developer.android.com/guide/components/intents-filters)
- [Qt file-open events](https://doc.qt.io/qt-6/qfileopenevent.html)
- [Desktop entry specification](https://specifications.freedesktop.org/desktop-entry/latest-single/)
- [Windows desktop packaging extensions](https://learn.microsoft.com/windows/apps/desktop/modernize/desktop-to-uwp-extensions)
- [HarmonyOS file processing applications](https://developer.huawei.com/consumer/cn/doc/harmonyos-guides/file-processing-apps-startup)
- [Web app file handling](https://developer.chrome.com/docs/capabilities/web-apis/file-handling)
