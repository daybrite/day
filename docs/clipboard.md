---
title: "Clipboard"
description: "Reading and writing the system clipboard via day-part-clipboard, with the platform quirks that complicate round-trips."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Clipboard

Let people copy and paste text, images, and app-specific data. `day-part-clipboard` reads and
writes the system clipboard, including multiple representations of the same item. Access depends
on the operating system’s privacy and focus rules.

## Add to your app

Add [`day-part-clipboard`](https://github.com/daybrite/day/tree/main/crates/day-part-clipboard) to your app’s Cargo dependencies, using the same Day version as the rest of your app:

```toml
[dependencies]
day-part-clipboard = "0.6"
```

The implementation is selected by the compilation target; no toolkit feature flags
or dependency entry in `Day.toml` are needed.

| Platforms / toolkits | Availability |
| --- | --- |
| macOS, iOS, Android, Windows, Linux (GTK / Qt), HarmonyOS | Text and binary clipboard APIs; Linux needs session clipboard tools. |
| Web | Permission- and browser-dependent. Binary format support differs by engine. |

## Authoring

```rust
day_part_clipboard::set_text("hello");
if day_part_clipboard::has_text() {
    println!("clipboard: {}", day_part_clipboard::get_text().unwrap_or_default());
}
```

[`set_text(&str) -> bool`](https://daybrite.dev/api/day_part_clipboard/fn.set_text.html) replaces the clipboard with plain text (`true` on success).
[`get_text() -> Option<String>`](https://daybrite.dev/api/day_part_clipboard/fn.get_text.html) reads the current text; it returns `None` when the clipboard is empty, holds no text
representation, or the platform denies access. [`has_text() -> bool`](https://daybrite.dev/api/day_part_clipboard/fn.has_text.html) checks for text, using a cheap
native probe where one exists ([`UIPasteboard.hasStrings`](https://developer.apple.com/documentation/uikit/uipasteboard), Win32 `IsClipboardFormatAvailable`, …).

There are no cargo features; platform selection is purely `#[cfg(target_os)]`, because the clipboard
is an OS concern rather than a toolkit one. `crates/day-part-clipboard/examples/clipboard.rs` is a
plain `main` that uses it with no Day framework at all
(`cargo run -p day-part-clipboard --example clipboard "hi"`).

## Platform implementations

Platform guidance: [Android: copying and pasting](https://developer.android.com/develop/ui/views/touch-and-input/copy-paste).

| OS | API | dependency |
|---|---|---|
| macOS | [`NSPasteboard`](https://developer.apple.com/documentation/appkit/nspasteboard) (clearContents + setString:forType: / stringForType:) | `objc2-app-kit` |
| iOS | [`UIPasteboard.generalPasteboard`](https://developer.apple.com/documentation/uikit/uipasteboard) (string / setString: / hasStrings) | `objc2-ui-kit` |
| Windows | [Win32 clipboard](https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard), [`CF_UNICODETEXT`](https://learn.microsoft.com/en-us/windows/win32/dataxchg/standard-clipboard-formats) (OpenClipboard/Set/GetClipboardData) | raw FFI (user32/kernel32) |
| Linux | `wl-copy`/`wl-paste` (Wayland), `xclip` (X11) via `std::process` | std only |
| HarmonyOS | native [`OH_Pasteboard_*`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-basic-services-kit/capi-pasteboard.md) + UDMF plain-text record (`libpasteboard.so`, `libudmf.so`) | raw FFI (BasicServicesKit) |
| Android | [`ClipboardManager`](https://developer.android.com/reference/android/content/ClipboardManager) via a Java shim | `day-toolkit-material` + `[package.metadata.day.android]` |

macOS is toolkit-independent: the general pasteboard needs no [NSApplication](https://developer.apple.com/documentation/appkit/nsapplication), run loop, or window, so it
works identically in day-toolkit-appkit and day-toolkit-qt binaries (and plain `cargo test` processes). [NSPasteboard](https://developer.apple.com/documentation/appkit/nspasteboard) is
**not thread-safe** (concurrent access from two threads can segfault inside AppKit), so the crate
serializes its own accesses behind a process-wide mutex.

Desktop Linux has no toolkit-independent native clipboard API (the clipboard lives in the display
server, and GDK's accessor needs GTK initialized, which would break day-toolkit-qt binaries), so the crate
shells out to the session's standard tools: `wl-copy`/`wl-paste` from `wl-clipboard` on Wayland,
`xclip` on X11 (the session type picks which to try first; the other is the fallback). One of them
must be installed; both are common distro packages.

HarmonyOS is `target_os = "linux"` but has no such tools, so it's gated on `target_env = "ohos"` and
uses the native Pasteboard C API (API 13+) instead. That's pure FFI: it needs neither a permission
nor the Day runtime (unlike Android). Content is typed through UDMF: a write wraps an
`OH_UdsPlainText` in a record in an `OH_UdmfData`; a read uses `OH_UdmfData_GetPrimaryPlainText`.

Platform access rules:

- **Android 10+** only lets an app *read* the clipboard while it holds input focus: [`get_text()`](https://daybrite.dev/api/day_part_clipboard/fn.get_text.html) /
  [`has_text()`](https://daybrite.dev/api/day_part_clipboard/fn.has_text.html) return `None`/`false` in the background. Writing is always allowed. No manifest permission
  is involved either way. Writing raises a system overlay of its own on recent versions, and it can
  hold that focus for a moment — so a read taken right after a write of the app's own is one of the
  cases that comes back empty. [`day::install_edit_commands`](https://daybrite.dev/api/day/fn.install_edit_commands.html) covers it by remembering the payload it
  placed (see [menus](menus.md)); an app calling this crate directly gets the platform's answer
  verbatim.
- **iOS 14+** shows the system "app pasted from …" banner when [`get_text()`](https://daybrite.dev/api/day_part_clipboard/fn.get_text.html) reads the pasteboard;
  [`has_text()`](https://daybrite.dev/api/day_part_clipboard/fn.has_text.html) uses `hasStrings`, the pre-check Apple provides that does not trigger the banner.
  Prefer the asynchronous typed [`read`](https://daybrite.dev/api/day_part_clipboard/fn.read.html) API in UI actions: it performs the native read on a
  worker so a delayed provider or paste authorization cannot block the main thread. The legacy
  [`get_text()`](https://daybrite.dev/api/day_part_clipboard/fn.get_text.html) helper remains synchronous.

## Extension integration

Like [`day-part-device`](/docs/reference/battery) (see [battery](battery.md) and [extending](extending.md)), this is a
headless external crate: it has no UI Piece and registers nothing into any toolkit's `RENDERERS`
slice. Its Android side contributes its own `DayClipboard.java` through
`[package.metadata.day.android]` exactly like the UI pieces but registers no renderer; on every
other platform the crate is fully day-independent.

## Typed binary content

`day::clipboard` re-exports [`day-part-clipboard`](https://daybrite.dev/api/day_part_clipboard/index.html). The original text functions remain
compatible. The typed API describes **one item with alternate representations**, not
several unrelated clipboard items:

```rust,ignore
use day::clipboard::{Content, Representation};
let content = Content(vec![
    Representation::new("image/png", png_bytes),
    Representation::new("application/x-my-app-document", document_bytes),
]);
let written_types = day::clipboard::write(content).await?;
let item = day::clipboard::read(&["application/x-my-app-document", "image/png"]).await?;
```

[`Representation`](https://daybrite.dev/api/day_part_clipboard/struct.Representation.html) holds a MIME string and shared, owned `Arc<Vec<u8>>` bytes. No paths,
image handles, UTF-8 conversions, or base64 encoding are imposed on application data.
[`Content::validate`](https://daybrite.dev/api/day_part_clipboard/struct.Content.html#method.validate) rejects empty offers, duplicate or invalid MIME names, more than
32 representations, and more than 64 MiB total. Reads apply the same byte limit where
native length information is available. Use bare MIME names without parameters.

[`write(Content)`](https://daybrite.dev/api/day_part_clipboard/fn.write.html) starts the request immediately and returns a [`ClipboardFuture`](https://daybrite.dev/api/day_part_clipboard/type.ClipboardFuture.html) whose
result lists the representations actually accepted. A platform may accept only a subset;
zero accepted representations is an error. [`read(&[mime, ...])`](https://daybrite.dev/api/day_part_clipboard/fn.read.html) starts immediately and
returns the first available representation in caller preference order, `Ok(None)` for
no matching content, or an error. Permission/access failures are not replaced with an
old in-process copy. An application implementing Cut must remove its objects only after
write succeeds. [`Error`](https://daybrite.dev/api/day_part_clipboard/enum.Error.html) distinguishes invalid data, oversized content, unsupported APIs,
and unavailable access (including permission denial).

Some native APIs hide denied access as an empty clipboard (notably mobile privacy rules),
and Linux session tools do not reliably distinguish a missing type from denied access.
Those cases return `Ok(None)`; errors are distinguished when the platform reports them.
Neither case reads a cached previous copy.

The future is local to the calling UI thread. Native operations currently complete
synchronously; web operations await browser promises. Call from the initiating user
action before spawning/awaiting unrelated work: the web adapter snapshots the live paste
event's strings and `File` objects immediately. Canceled web futures remove their request
registrations, and late callbacks still free transferred buffers. A call outside a paste
event uses [`navigator.clipboard.read`](https://www.w3.org/TR/clipboard-apis/#dom-clipboard-read), subject to secure-context, permission, and browser
user-activation rules. Copy offers supported [`ClipboardItem`](https://www.w3.org/TR/clipboard-apis/#clipboarditem) representations (notably PNG,
plain text, and HTML), and copy/cut events also receive textual custom representations.
The result reports whichever system/event write actually succeeded. Arbitrary MIME types
are not universally writable by browsers.

Outside a native paste event, a byte read waits for all byte writes already issued by this
page to settle before asking the browser for clipboard contents. This preserves Copy → Paste
ordering when a write is asynchronous. Failed writes still report their own errors and do not
block a subsequent read; native paste events use their captured payload immediately.

### Native representation mappings

| Platform | Binary transport | Limits / interoperability |
|---|---|---|
| macOS (AppKit, GTK, Qt) | [`NSPasteboard.setData:forType:`](https://developer.apple.com/documentation/appkit/nspasteboard) / `dataForType:` | PNG/JPEG/TIFF/GIF/BMP/SVG/WebP MIME names map to Apple pasteboard identifiers; custom names remain exact. Same process-wide lock as text operations. |
| iOS UIKit | One [`UIPasteboard.items`](https://developer.apple.com/documentation/uikit/uipasteboard) dictionary with multiple [`NSData`](https://developer.apple.com/documentation/foundation/nsdata) values | Same Apple identifiers. Paste menu validation accepts non-text items. System paste authorization still applies. |
| Android Material Components | [`ClipData`](https://developer.android.com/reference/android/content/ClipData) content URI + read-only `BinaryClipboardProvider` | The crate contributes its provider and manifest through Day metadata. Binary data is stored in app cache files, not Binder parcels. URI grants allow receiving apps to open typed representations. Provider paths are UUID-only; writes through the provider are rejected. Cache eviction can make old URI clips unavailable. The current generated Java bridge transports the internal packet as base64 because it does not yet marshal byte-vector returns; system clipboard content remains binary. |
| Windows (OS-level, any toolkit) | Registered formats, including `PNG`; [`CF_UNICODETEXT`](https://learn.microsoft.com/en-us/windows/win32/dataxchg/standard-clipboard-formats) for text | A private length-framed companion preserves exact byte lengths for Day-to-Day arbitrary data, independent of global-allocation padding. Packed [`CF_DIB`](https://learn.microsoft.com/en-us/windows/win32/dataxchg/standard-clipboard-formats) screenshots are exposed as encoded BMP, including palette/mask offsets. |
| Linux GTK/Qt | `wl-copy`/`wl-paste --type`, or `xclip -target` | Session tools currently publish one representation: PNG, then SVG, then text, then the first offer. The returned accepted-type list exposes that restriction. Matching tools and a display session are required. |
| HarmonyOS ArkUI | Pasteboard + UDMF general byte entries; plain text uses its native UDS record | Encoded image types map to UTD identifiers. PixelMap-only or URI-only foreign image records are not converted. Custom UTD acceptance is platform-dependent and reported through the accepted-type result. |

The OS-based implementation deliberately remains usable without initializing GTK or Qt.
The desktop toolkit edit routes remain responsible for allowing a focused text editor to
handle its own Copy/Paste first. Apps that need custom async behavior can use
[`day::install_edit_bridge`](https://daybrite.dev/api/day/fn.install_edit_bridge.html) with `day::EditState` and the typed clipboard functions.

### Interoperability

Offer an app-specific representation alongside a standard format when other applications
should be able to paste the data. For images, PNG is a useful interchange format; retain the
original encoded data separately if the app needs it for lossless editing.

For Cut, delete the captured selection only after the write succeeds and only if that
selection is still current when the asynchronous operation completes.

> [!WARNING]
> **Windows, Linux, and HarmonyOS: clipboard interoperability.**
> Native binary clipboard behavior remains unverified on these platforms. Test transfers
> with the other applications and data formats your app needs.

> [!NOTE]
> **Web / DOM: clipboard permissions.**
> Browser permissions and clipboard formats differ by engine. Test permission denial and
> paste from other applications in the browsers your app supports.
