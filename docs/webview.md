---
title: "Web view"
description: "The web view piece: remote pages and bundled inline sites, sessions, link policy, and per-platform engines."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

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

Evaluating JavaScript and reading a value back is its own story — [docs/webview-eval.md](webview-eval.md) keeps the per-platform support list current:
`JsHandle::eval(script).await` returns the value as JSON, or the error the script threw. Ask
`eval_support()` before offering it — GTK, Android and ArkUI have an engine but no arm yet, and
web-dom can never have one. See [webview-eval.md](./webview-eval.md) for the per-platform research,
the JavaScript envelope, and what each remaining arm needs.

### Sessions (surviving navigation)

Day rebuilds a destination's whole subtree on every navigation, so a plain `web_view` gets a fresh
native view each visit and reloads from scratch. `.session(…)` moves the engine out of that lifetime:

```rust
web_view(url).session(WebSession::global("myapp.browser"))
```

The piece keeps the native web view alive against that id, and the next `web_view` bound to the same
session re-attaches it with its page, scroll position, history and **JavaScript context** intact.
This is the shape Apple settled on for the same problem — `WebPage` holds the session, `WebView`
renders it — and it works for the same reason: a web view's content lives in the object and its
content process, not in its attachment to a parent view.

Sessions are keyed by a `&'static str` so `WebSession::global` is idempotent, which matters because
the page function runs again on every navigation. There is deliberately no anonymous constructor: an
id that changed per build would retain a new engine each visit and leak them all. A retained view is
never freed — **one session is one live web view for the process's lifetime**, so use them for pages
a user returns to, not per list item.

Reactive state is a separate problem with a separate answer: signals declared in a page function are
minted fresh per visit too, so hoist them with `Signal::global` behind a `OnceCell` (the showcase's
`pages/webview.rs` and `pages/scripting.rs` both do this).

**Per-backend.** AppKit and UIKit release a handle by *detaching* it (`removeFromSuperview`), so the
piece holding its own reference is all it takes. Qt is the exception — its `release` calls
`deleteLater()` on the handle — so what the shim retains is the `QWebEngineView` *inside* the
container, and `~DayWebView` re-parents it out before `~QWidget` deletes its children. All three
also have to keep the node the retained view reports to up to date: the view outlives the node that
first realized it, so `make` re-points it at whichever node is showing it now, and `release` leaves
a retained view's bookkeeping alone.

Verified on macos-appkit, ios-uikit and macos-qt by setting `window.__dayMarker` in the live page,
navigating away and back, and reading it again — the showcase walkthrough asserts exactly that, so a
backend that starts rebuilding instead of retaining fails CI rather than degrading quietly.

GTK, Android, XAML, ArkUI and web-dom ignore `session` and rebuild as before. GTK's `release` also
only detaches, so it is the next one that could carry this; the others would each need the same work
Qt needed.

## Inline sites: `web_view_inline` (app-embedded content)

A directory under `resource/assets/` can ship a whole site — pages, stylesheets, scripts,
images, structure preserved ([docs/resources.md](resources.md)'s asset tree) — and the view serves it from
inside the app, no network:

```rust
// resource/assets/web/minisite/{index.html, css/, js/, img/, pages/}
web_view_inline(res::assets::web::minisite)         // lazy: loads index.html directly
    .session(WebSession::global("app.embedded"))
    .start_page("pages/intro.html")                  // optional, instead of index.html
    .on_external_link(|url| LinkPolicy::OpenSystem)  // optional; this IS the default

// the checked route: index.html presence validated before the view exists
let site = res::assets::web::minisite.prepare_site().await?;   // InlineSite
web_view_inline(site)
```

Two rules define the mode. **Relative references resolve natively**: the arm loads the site
through the platform's own local-content channel, so the engine itself resolves `css/style.css`
or `../index.html` — no interception layer rewrites anything. **Navigations that leave the site
are cancelled in-view** and dispatched per `LinkPolicy`: `OpenSystem` (the default — the OS
browser for `https://`, the mail client for `mailto:`, whatever the scheme maps to), `InView`
(allow it after all), or `Ignore` (the `on_external_link` closure already did the in-app work —
the showcase intercepts `day-showcase://<route>` links and navigates the app itself). The
decision runs in Rust: events are enqueue-only (§8.3), so the native side always cancels and
reports (`num = -1` on the shared Custom channel), and the policy's verdict follows as a
command. `prepare_site()` is a future because backends whose engine cannot read embedded stores
in place will extract to the platform cache dir here; on every v1 backend it resolves on first
poll.

Per backend — gate on `inline_support()`:

| Backend | Channel | Policy hook |
|---|---|---|
| AppKit / UIKit | `loadFileURL:allowingReadAccessToURL:` into the bundle's assets tree (canonicalized — WebKit reports standardized URLs, so the policed base must match) | `decidePolicyForNavigationAction` |
| Android | `file:///android_asset/<dir>/…` (the assets tree IS the APK `assets/` root; the URL family is exempt from the API-30 file-access default) | `shouldOverrideUrlLoading` |
| Qt | `qrc:/day/assets/<dir>/…` — QWebEngine reads the qrc-staged tree natively; policed by (scheme, path-prefix), since Chromium normalizes qrc spellings | `acceptNavigationRequest` (the shim's `DayWebPage`) |
| XAML | `SetVirtualHostNameToFolderMapping` maps the exe-relative assets dir under `day-assets.example`; `NewWindowRequested` is swallowed and reported as external | `NavigationStarting` |
| GTK (linux) | extract-to-cache — WebKitGTK cannot browse a GResource, so `prepare_site()` (or realize, on the lazy path) copies the tree to the user cache once per process and the view loads the canonical `file://` URL | `decide-policy` |
| ArkWeb | `resource://rawfile/day/<dir>/…` over the rawfile staging; the inline marker crosses in the piece's props string and the ArkTS side composes and polices the URL | `onLoadIntercept` |
| web-dom | the deployed `assets/data/<dir>/…` URL — same origin as the host page, so the browser resolves the site AND the shim's capture-phase click hook (armed by the `data-day-inline-base` attribute) polices leaving links | in-frame click hook → `day_dom_piece_event` |

Every backend with a web engine reports `Native`; `Unsupported` remains only where there is no
engine at all (macos-gtk / windows-gtk, which have no WebKitGTK build). The Qt, XAML, GTK and
ArkWeb arms are compile-verified from this host and behavior-verified by their CI legs; Qt was
additionally exercised live on macos-qt.

The showcase's Web View page shows both modes as tabs: **Remote** (the browsing demo above) and
**Embedded** (`resource/assets/web/minisite/`, with all three link dispositions live).

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
  wedges the window's compositor; the walkthrough skips this page there ([docs/harmonyos.md](harmonyos.md)).
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
