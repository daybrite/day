# Clipboard (headless capability crate)

> **Status: implemented** as `day-part-clipboard` (in `parts/`, the headless counterpart of
> `pieces/`). It's a **headless** day-ecosystem crate (no UI Piece): a shared cross-platform API for
> the system's plain-text clipboard through each platform's native API. Any Rust code can depend on
> it and call `day_part_clipboard::{set_text, get_text, has_text}`. Verified on macOS (roundtrip
> checked against `pbpaste`); iOS and Android pass clippy for their targets; HarmonyOS cross-compiles
> + links against the native `libpasteboard.so`/`libudmf.so`.

## Authoring

```rust
day_part_clipboard::set_text("hello");
if day_part_clipboard::has_text() {
    println!("clipboard: {}", day_part_clipboard::get_text().unwrap_or_default());
}
```

`set_text(&str) -> bool` replaces the clipboard with plain text (`true` on success). `get_text() ->
Option<String>` reads the current text; it returns `None` when the clipboard is empty, holds no text
representation, or the platform denies access. `has_text() -> bool` checks for text, using a cheap
native probe where one exists (`UIPasteboard.hasStrings`, Win32 `IsClipboardFormatAvailable`, …).

There are no cargo features; platform selection is purely `#[cfg(target_os)]`, because the clipboard
is an OS concern rather than a toolkit one. `parts/day-part-clipboard/examples/clipboard.rs` is a
plain `main` that uses it with no Day framework at all
(`cargo run -p day-part-clipboard --example clipboard "hi"`).

## Per-platform native realization

| OS | API | dependency |
|---|---|---|
| macOS | `NSPasteboard` (clearContents + setString:forType: / stringForType:) | `objc2-app-kit` |
| iOS | `UIPasteboard.generalPasteboard` (string / setString: / hasStrings) | `objc2-ui-kit` |
| Windows | Win32 clipboard, `CF_UNICODETEXT` (OpenClipboard/Set/GetClipboardData) | raw FFI (user32/kernel32) |
| Linux | `wl-copy`/`wl-paste` (Wayland), `xclip` (X11) via `std::process` | std only |
| HarmonyOS | native `OH_Pasteboard_*` + UDMF plain-text record (`libpasteboard.so`, `libudmf.so`) | raw FFI (BasicServicesKit) |
| Android | `ClipboardManager` via a Java shim | `day-android` + `[package.metadata.day.android]` |

macOS is toolkit-independent: the general pasteboard needs no NSApplication, run loop, or window, so it
works identically in day-appkit and day-qt binaries (and plain `cargo test` processes). NSPasteboard is
**not thread-safe** (concurrent access from two threads can segfault inside AppKit), so the crate
serializes its own accesses behind a process-wide mutex.

Desktop Linux has no toolkit-independent native clipboard API (the clipboard lives in the display
server, and GDK's accessor needs GTK initialized, which would break day-qt binaries), so the crate
shells out to the session's standard tools: `wl-copy`/`wl-paste` from `wl-clipboard` on Wayland,
`xclip` on X11 (the session type picks which to try first; the other is the fallback). One of them
must be installed; both are common distro packages.

HarmonyOS is `target_os = "linux"` but has no such tools, so it's gated on `target_env = "ohos"` and
uses the native Pasteboard C API (API 13+) instead. That's pure FFI: it needs neither a permission
nor the Day runtime (unlike Android). Content is typed through UDMF: a write wraps an
`OH_UdsPlainText` in a record in an `OH_UdmfData`; a read uses `OH_UdmfData_GetPrimaryPlainText`.

Platform access rules worth knowing:

- **Android 10+** only lets an app *read* the clipboard while it holds input focus: `get_text()` /
  `has_text()` return `None`/`false` in the background. Writing is always allowed. No manifest permission
  is involved either way.
- **iOS 14+** shows the system "app pasted from …" banner when `get_text()` reads the pasteboard;
  `has_text()` uses `hasStrings`, the pre-check Apple provides that does not trigger the banner.

## What it shows about the extension system

Like `day-part-battery` (see [battery.md](battery.md) and [extending.md](extending.md)), this is a
headless external crate: it has no UI Piece and registers nothing into any backend's `RENDERERS`
slice. Its Android side contributes its own `DayClipboard.java` through
`[package.metadata.day.android]` exactly like the UI pieces but registers no renderer; on every
other platform the crate is fully day-independent.
