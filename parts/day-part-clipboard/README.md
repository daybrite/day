# day-part-clipboard

Read and write the system clipboard's plain text.

`set_text`, `get_text`, and `has_text`, backed by whichever clipboard is native to the
running platform: `NSPasteboard` on macOS, `UIPasteboard` on iOS, the GTK and Qt
clipboards on Linux, `ClipboardManager` on Android, and the Win32 clipboard on Windows.

Parts are Day's small capability crates: no UI, just a plain Rust API over something the
platform already provides. This one works in any Rust program — you don't need a Day app
around it.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
