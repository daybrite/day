# day-piece-webview

An embedded native web view for Day apps: `WKWebView` on macOS/iOS, `QWebEngineView` on
Qt, WebKitGTK on Linux, `android.webkit.WebView` on Android — one Rust API for URL loading
and navigation events.

Where a platform combination has no viable engine, the piece degrades explicitly rather
than bundling a browser.

Pieces are Day's extension unit: a crate with one Rust API and per-toolkit native
renderers, enabled per backend by cargo features (`day build` wires them automatically).

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
