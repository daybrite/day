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

`day_piece_webview::support()` reports what the running backend realizes. `Native` is an embedded
browser engine with the full command set. `Emulated` loads pages but cannot drive history or report
navigation back — web-dom's `<iframe>`, see the note below. `Unsupported` renders day's placeholder
leaf (macos-gtk and windows-gtk, which have no WebKitGTK). Gate history controls on it:

```rust
let history = support() == Support::Native;
button("Back").enabled(move || history).action(move || back.notify());
```

`.back()`, `.forward()` and `.stop()` are no-ops below `Native`, so a button left enabled there is one
that does nothing when pressed.

Evaluating JavaScript and reading a value back ships on **AppKit, UIKit, Qt and XAML**:
`JsHandle::eval(script).await` returns the value as JSON, or the error the script threw. Ask
`eval_support()` before offering it — GTK, Android and ArkUI have an engine but no arm yet, and
web-dom can never have one. See [webview-eval.md](./webview-eval.md) for the per-platform research,
the JavaScript envelope, and what each remaining arm needs.

## Per-backend native realization

| | AppKit | UIKit | Qt | Android | GTK | XAML |
|---|---|---|---|---|---|---|
| control | `WKWebView` | `WKWebView` | `QWebEngineView` | `android.webkit.WebView` | WebKitGTK `WebView` | UWP-XAML `WebView` |
| native code | objc2-web-kit | hand-rolled `extern_class!` + `msg_send!` | `src/lib-qt-shim.cpp` (+ links `Qt6WebEngineWidgets`) | `android/java/…/DayWebView.java` | `webkit6` crate | `src/lib-xaml-shim.cpp` |
| URL-back event | `Custom("webview:url", …)` | `Custom("webview:url", …)` | `Custom("webview:url", …)` | `TextChanged` (kind 1) | `Custom("webview:url", …)` | `Custom("webview:url", …)` |

Rendering, two-way URL binding, and controls are verified on AppKit, Qt, UIKit (iOS sim), and Android.
GTK and XAML are written blind (no WebKitGTK / Windows host on the reference machine) to build and run
in CI; the GTK `webkit6` API is verified against the crate source, and both are captured in the CI
gallery.

**Backend notes:**
- **GTK**: WebKitGTK 6 via the `webkit6` crate. **Linux/Windows only**: Homebrew's `webkitgtk` vends the
  GTK3 API and has no bottle, and WebKitGTK isn't viable on macOS-quartz, so `webkit6` is a non-macOS
  target dependency and `macos-gtk` falls back to a placeholder leaf. The CI Linux/Windows GTK jobs
  install `libwebkitgtk-6.0-dev` / `mingw-w64-x86_64-webkitgtk6`.
- **ArkUI (HarmonyOS)**: the ArkTS `Web` component. The ArkUI **C** node API has no Web node kind, so
  this is the first piece whose native half is ArkTS: the crate ships `ohos/ets/Index.ets`, `day build`
  stages it into the app's hvigor project (`[package.metadata.day.ohos]`), and day-arkui's generic piece
  bridge builds it in a `BuilderNode` and mounts its FrameNode in the Day tree. Commands go out through
  `webview.WebviewController`; `onPageEnd` reports each committed URL back. **The x86_64 emulator cannot
  run it**: its `ArkWebCore.hap` carries arm64-only native libs (`bm install` answers "the Abi type
  supported by the device does not match"), so the engine loads as null and the component's surface
  wedges the window's compositor; the walkthrough skips this page there (docs/harmonyos.md).
- **web-dom**: an `<iframe>` — the one backend with no engine to embed, because the host page already
  is one. `Load` and `Reload` work. `Back`, `Forward` and `Stop` are no-ops, and navigation does not
  report back into the bound signal, because the same-origin policy forbids a parent document from
  reading or driving a cross-origin child: `contentWindow.history.back()` and
  `contentWindow.location.href` both throw `SecurityError`. Driving the **top-level** history instead
  would be worse than doing nothing — day's web router owns that stack (`pushState` on hash routes), so
  a back press would navigate the app off the page hosting the frame.

  Same-origin content has none of these limits, but a piece cannot know the origin before loading and
  the failure is silent when it guesses wrong, so the arm reports `Support::Emulated` and behaves
  identically either way rather than working only sometimes. `Reload` re-assigns the last URL day set,
  not wherever the frame has since navigated to, and the arm keeps that URL itself because day-dom is
  write-only (`set_attr` with no getter). No `sandbox` attribute is set: present-but-empty is
  deny-everything, which breaks scripts and forms on essentially every real site. A site that refuses
  embedding (`X-Frame-Options`, CSP `frame-ancestors`) renders blank and the parent cannot detect it —
  the load event fires either way, so no arm of this piece can report it.
- **XAML**: **WebView2**, hosted windowless. The obvious choice — the UWP-XAML
  `Windows.UI.Xaml.Controls.WebView` (EdgeHTML), already in the base SDK projection day-xaml uses —
  does not work in Day's Win32 XAML-Islands host: it renders blank, never raises
  `NavigationCompleted`, and crashes on navigation. A plain child HWND over the island does not work
  either, because the island's `ContentIsland` InputSite owns pointer input for the whole surface, so
  the web view never sees a click.

  So the shim (`src/lib-xaml-shim.cpp`) boxes a transparent XAML `Border` as the day handle, renders
  the page into a `Windows.UI.Composition` visual through a `CoreWebView2CompositionController`, and
  splices that visual in with `ElementCompositionPreview::SetElementChildVisual`. The web view is
  then a real node in the XAML visual tree — correct z-order, clipping, DPI, and layout, with no
  second window to track and no airspace problem — and pointer events arrive at the Border and are
  forwarded to `SendMouseInput`. This is the same technique the official XAML WebView2 controls use
  internally.

  `WebView2LoaderStatic.lib` is linked statically (from the Microsoft.Web.WebView2 NuGet package, not
  the base SDK), so there is no DLL to bundle; the WebView2 Runtime itself is a system-wide install,
  present on Windows 11 and on the CI runners. When it is absent, controller creation fails and the
  Border's URL label stays as the fallback, so the page degrades rather than crashing.

## CI screenshots + gallery

The dayscript walkthrough (`Day-Showcase/dayscript/walkthrough.yaml`) visits the web-view page last,
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

4. **ArkTS-built components on HarmonyOS.** The ArkUI C API can't construct a `Web` at all, so a piece
   needed a way to ship ArkTS and have it mounted in the native tree. `[package.metadata.day.ohos] ets =
   [...]` stages a piece's `.ets` into the hvigor project, `day build` generates the `DayPieces.ets`
   aggregator the host page registers, and the shim's `registerPiece`/`pieceEvent` pair carries
   make/update/dispose and events across generically, so `map`/`lottie` need no new bridge.

Two more findings handled within existing contracts: native→URL reporting uses `Custom("webview:url", …)`
on Apple/Qt but the public `TextChanged` kind on Android (its `Custom` kind is reserved for deep links);
and `text_field`'s `Submitted` event is currently a no-op, so loading is driven by a **Go** button.
