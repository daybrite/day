# Web view (external piece)

> **Status: implemented** as `day-piece-webview`, an external Day Piece (like `day-piece-combobox`)
> registered link-time into each backend's renderer slice without touching day. It wraps each
> toolkit's native web view and fills the space it's offered. It is a reference for pieces whose native
> backend is heavier than a control: a whole embedded browser, with commands in and URL events out.

## Authoring

```rust
use day_piece_webview::web_view;

let url = Signal::new("https://daybrite.dev".to_string());
let (go, back, fwd, stop, reload) = (Trigger::new(), Trigger::new(), Trigger::new(),
                                     Trigger::new(), Trigger::new());

// The URL bar is bound two-way: type + Go loads it; navigation reports the URL back so the field follows.
text_field(url).id("url");
button("Go").action(move || go.notify());
button("Back").action(move || back.notify());          // + Forward / Stop / Reload the same way

web_view(url).go(go).back(back).forward(fwd).stop(stop).reload(reload).id("web")
```

`web_view(url)` takes a `Signal<String>`. The initial value loads when the view is created; firing the
`.go()` trigger (re)loads whatever the signal currently holds. History is driven imperatively with `Copy`
`Trigger`s (`.back()/.forward()/.stop()/.reload()`), each `watch`ed to a command. Native navigation
reports the current URL back into the bound signal, so a bound `text_field` follows along. `WebView`
implements `Piece`, so `.id()`/`.a11y()`/`.frame()` chain via `Decorate`. It's a growing leaf
(`Flex { grow_w, grow_h }`), so put it last in a `column` and it fills the remaining space.

## Per-backend native realization

| | AppKit | UIKit | Qt | Android | GTK | WinUI |
|---|---|---|---|---|---|---|
| control | `WKWebView` | `WKWebView` | `QWebEngineView` | `android.webkit.WebView` | WebKitGTK `WebView` | UWP-XAML `WebView` |
| native code | objc2-web-kit | hand-rolled `extern_class!` + `msg_send!` | `src/lib-qt-shim.cpp` (+ links `Qt6WebEngineWidgets`) | `android/java/…/DayWebView.java` | `webkit6` crate | `src/lib-winui-shim.cpp` |
| URL-back event | `Custom("webview:url", …)` | `Custom("webview:url", …)` | `Custom("webview:url", …)` | `TextChanged` (kind 1) | `Custom("webview:url", …)` | `Custom("webview:url", …)` |

Rendering, two-way URL binding, and controls are verified on AppKit, Qt, UIKit (iOS sim), and Android.
GTK and WinUI are written blind (no WebKitGTK / Windows host on the reference machine) to build and run
in CI; the GTK `webkit6` API is verified against the crate source, and both are captured in the CI
gallery.

**Backend notes:**
- **GTK**: WebKitGTK 6 via the `webkit6` crate. **Linux/Windows only**: Homebrew's `webkitgtk` vends the
  GTK3 API and has no bottle, and WebKitGTK isn't viable on macOS-quartz, so `webkit6` is a non-macOS
  target dependency and `macos-gtk` falls back to a placeholder leaf. The CI Linux/Windows GTK jobs
  install `libwebkitgtk-6.0-dev` / `mingw-w64-x86_64-webkitgtk6`.
- **WinUI**: the UWP-XAML `Windows.UI.Xaml.Controls.WebView` (EdgeHTML), which is in the base Windows SDK
  cppwinrt projection day-winui already uses (no Windows App SDK / WebView2). Creation + navigation are
  wrapped in try/catch: EdgeHTML WebView can be unavailable in an unpackaged Win32 XAML host, so it
  degrades to a label rather than crashing.

## CI screenshots + gallery

The dayscript walkthrough (`apps/showcase/dayscript/walkthrough.yaml`) visits the web-view page last,
`pause`s (runner-side) for the page to load, and captures `webview.png`. Each combo uploads its
`screenshots-<combo>` artifact; `website/gallery.config.mjs` lists a `webview` shot, so the assembled
gallery on daybrite.dev shows the web view across every platform that produced it.

## What this piece taught the extension system

Building `day-piece-webview` as a fully self-contained piece surfaced (and fixed) three things; see
[extending.md](extending.md):

1. **Android manifest permissions.** A web view needs `INTERNET`, but a piece can't edit the app manifest.
   `[package.metadata.day.android]` gained a `permissions = [...]` key; `day build` writes them to a
   generated overlay manifest that AGP merges into the app manifest, so the app needs no edits.
2. **iOS framework loading.** objc2-web-kit only binds the macOS `WKWebView`, so the iOS class is
   hand-rolled, and WebKit.framework has to be loaded for its Objective-C class to register. A `#[link]`
   autolink hint is unreliable across the cargo-staticlib → xcode link boundary, so the piece `dlopen`s the
   (public) framework once at first use. Self-contained: no framework entry in the app's xcode project.
3. **Grow-leaf sizing on Android.** day-android's default `measure` (for `measure: None`) returns a view's
   *natural* size, which is ~0 for a `WebView`. A fill leaf must return the *proposal* from `measure`
   (as the built-in `list` does); AppKit/Qt/UIKit already do this in their `measure: None` default.

Two more findings handled within existing contracts: native→URL reporting uses `Custom("webview:url", …)`
on Apple/Qt but the public `TextChanged` kind on Android (its `Custom` kind is reserved for deep links);
and `text_field`'s `Submitted` event is currently a no-op, so loading is driven by a **Go** button.
