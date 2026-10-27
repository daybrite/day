---
title: "Web views"
description: "Embed pages and bundled sites, run JavaScript, and manage browser profiles and resources."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Web views

Embed a help site, a document reader, or a browser pane with `day-piece-webview`. The piece
uses the selected toolkit's web engine and exposes navigation controls, JavaScript evaluation,
browser profiles, and application-provided resources through one Rust interface.

For account sign-in, use [authentication](/docs/reference/auth), which opens the platform's
sign-in surface. An embedded browser is not a replacement for that flow.

On this page: [setup](#add-to-your-app), [platforms](#platforms-and-web-engines),
[navigation](#navigation-and-lifetime), [bundled sites](#bundle-a-site),
[application resources](#application-resources), [JavaScript](#javascript-interaction),
and [browser profiles](#browser-profiles).

## Add to your app

Add [`day-piece-webview`](https://crates.io/crates/day-piece-webview) to your app:

```sh
cargo add day-piece-webview
```

[Crate](https://crates.io/crates/day-piece-webview) ·
[API reference](https://docs.rs/day-piece-webview/latest/day_piece_webview/) ·
[Source](https://github.com/daybrite/day-piece-webview).
The [Day CLI](/docs/reference/cli) enables the matching toolkit feature and packages native
contributions. No dependency entry is needed in `Day.toml`.

## Platforms and web engines

| Platform / toolkit | Engine and prerequisites |
| --- | --- |
| AppKit / UIKit | [WKWebView](https://developer.apple.com/documentation/webkit/wkwebview), provided by the OS. |
| macOS / GTK | WKWebView hosted within the GTK layout. Native-child clipping and overlap need attention in nested scrolling or overlays. |
| Linux / GTK | [WebKitGTK](https://webkitgtk.org/reference/webkit2gtk/stable/), using the WebKitGTK 6 libraries alongside GTK 4. |
| macOS / Qt, Linux / Qt | [Qt WebEngine](https://doc.qt.io/qt-6/qtwebengine-overview.html). A build without WebEngine uses a URL-label fallback. |
| Windows / WinUI | [WebView2](https://learn.microsoft.com/en-us/microsoft-edge/webview2/). The installed WebView2 Runtime is separate from its build-time SDK. |
| Android | [Android WebView](https://developer.android.com/develop/ui/views/layout/webapps/webview); Internet permission is contributed by the crate. |
| HarmonyOS / ArkUI | ArkWeb. A compatible installed runtime is required; custom-resource handling needs API 12+. |
| Web | An iframe in the host browser. Remote sites can refuse framing, and same-origin restrictions limit scripting and navigation observation. |

Most capability functions describe compiled integration, not successful engine startup. A
native support result does not prove a Qt WebEngine installation or a usable runtime.

## Navigation and lifetime

| Content | Entry point |
| --- | --- |
| Remote page or a URL selected at runtime | [`web_view()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/fn.web_view.html) with a string binding. |
| Site shipped with the app | [`web_view_inline()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/fn.web_view_inline.html) with a generated asset-directory constant. |
| Documents or binary data supplied by the app | [`web_view_resources()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/fn.web_view_resources.html) with a [resource provider](#application-resources). |

This component belongs in an app with a generated catalog containing `browser_back` and
`browser_reload`. The URL is provided by the caller:

```rust
use day::prelude::*;
use day_piece_webview::{support, web_view};

pub fn browser_panel(url: Signal<String>) -> impl Piece {
    let back = Trigger::new();
    let reload = Trigger::new();
    column((
        row((
            button(res::str::browser_back())
                .enabled(support() == Support::Native)
                .action(move || back.notify()),
            button(res::str::browser_reload()).action(move || reload.notify()),
        )),
        web_view(url).back(back).reload(reload).grow(),
    ))
}
```

```ftl
browser_back = Back
browser_reload = Reload
```

The initial URL loads when the view is built. Editing the URL binding alone does not navigate;
attach a [`go()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/struct.WebView.html#method.go)
trigger and fire it after the edit. Native navigation writes the observed URL back to the binding.
The browser iframe cannot report navigation from arbitrary remote origins.

The [`WebView`](https://docs.rs/day-piece-webview/latest/day_piece_webview/struct.WebView.html)
builder also supplies forward, stop, reload, and force-reload triggers. Check
[`support()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/fn.support.html)
when enabling history controls and
[`force_reload_support()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/fn.force_reload_support.html)
before promising a cache-bypassing reload.

Removing an ordinary view releases its browser. Use a
[`WebSession`](https://docs.rs/day-piece-webview/latest/day_piece_webview/struct.WebSession.html)
when the app needs a retained browser across reconstruction. Session retention and browser
storage are different concerns: [browser profiles](#browser-profiles) control
cookies and website data, while a session controls the retained view.

Use the external-link callback to apply your app's navigation policy. Treat remote content,
redirect destinations, and scripts as untrusted unless your app explicitly controls them.

## Bundle a site

Put `index.html` and its related files under `resource/assets/site/`. Open the generated
directory constant:

```rust
use day::prelude::*;
use day_piece_webview::web_view_inline;

pub fn bundled_help() -> impl Piece {
    web_view_inline(res::assets::site).grow()
}
```

Use document-relative links inside the site, such as `images/diagram.svg` or `guide.html`.
The toolkit chooses its local-content URL; applications should not construct a `file:`,
`qrc:`, or engine-specific asset URL. App-authored text in a bundled interface must use the
app's generated translations. See [resources](/docs/reference/resources).

## Application resources

Use a resource provider when an embedded page comes from a database, archive, generated report,
or another application-owned source. The web engine requests resources by path and the
provider returns bytes with their MIME type. Ordinary relative links in the document continue
to resolve within that provider.

### Provide a document

This helper accepts document content from its caller. It serves one HTML document and reports
other paths as missing. App-generated UI text must be translated before it reaches the helper;
user-authored documents can retain their own text.

```rust
use std::sync::Arc;
use day_piece_webview::{ResourceProvider, ResourceResponse};

pub fn document_provider(html: Vec<u8>) -> ResourceProvider {
    let body: Arc<[u8]> = html.into();
    ResourceProvider::new(move |request| {
        if request.path == "index.html" {
            ResourceResponse::new("text/html; charset=utf-8", body.clone())
        } else {
            ResourceResponse::not_found()
        }
    })
}
```

Pass the provider and the document-relative entry path to
[`web_view_resources()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/fn.web_view_resources.html).
The engine chooses the actual origin; treat provider URLs as opaque.

[`ResourceProvider::new()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/struct.ResourceProvider.html#method.new)
requires a thread-safe callback. Native engines may call it on workers; the browser integration
calls it on the app thread. Do not read Day signals or touch toolkit objects inside the handler.
Prepare immutable data or use a thread-safe store, and keep browser callbacks short.

### Paths, responses, and lifetime

[`ResourceRequest`](https://docs.rs/day-piece-webview/latest/day_piece_webview/struct.ResourceRequest.html)
contains the path, query, and method. Use a bounded namespace of allowed resources. Do not join
an untrusted request path directly to an unrestricted filesystem root.
[`ResourceResponse`](https://docs.rs/day-piece-webview/latest/day_piece_webview/struct.ResourceResponse.html)
contains a status, MIME type, shared byte body, and optional headers. Each response is buffered;
this is not a streaming server. Native adapters handle byte ranges and HEAD requests, with
engine-specific restrictions.

Views retain their provider. Releasing a view cancels delivery to that view; keep a provider
handle if other views will continue using its namespace. Check callback ownership before
capturing large document caches.

For a static bundled site, prefer
[`web_view_inline()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/fn.web_view_inline.html)
with the generated asset-directory constant. It needs no custom handler. If your HTML
references bundled images or styles, preserve document-relative paths and use the provider's
[bundled-site mounting API](https://docs.rs/day-piece-webview/latest/day_piece_webview/struct.ResourceProvider.html#method.with_site)
when combining app resources with dynamic content.

### Platform limits

| Platform / toolkit | Constraint |
| --- | --- |
| AppKit / UIKit / macOS GTK | WebKit custom-scheme handling is installed before the browser is created. |
| Linux GTK | Uses a WebKit URI-scheme handler; test media/range behavior for the document types you serve. |
| Qt WebEngine | Requires a registered custom scheme. Arbitrary HTTP status behavior differs from an HTTP server; additional response headers require Qt 6.6+. |
| Android / WinUI | Uses reserved origins and request interception. Return the correct MIME type for binary resources. |
| HarmonyOS | Requires ArkWeb's custom-resource handler support, API 12+. |
| Web | Requires HTTPS or localhost and service workers. The owning app tab must stay alive; disabling service workers prevents provider views from loading. |

The browser worker is scoped to this crate's asset namespace. It is distinct from ordinary
inline content and does not persist document bodies. Use [browser profiles](#browser-profiles)
for website storage and [web view scripting](#javascript-interaction) for interaction with
a loaded document.

## JavaScript interaction

Run JavaScript in a loaded page and receive its JSON result in Rust.

Bind a handle to the web view, then evaluate after its page loads. This is an integration
fragment; add the generated `site` asset directory to your app:

```rust
use day_piece_webview::{JsHandle, web_view_inline};

let js = JsHandle::new();
let view = web_view_inline(res::assets::site).js(js);
// In an event handler:
day::task(async move {
    match js.eval("document.title").await {
        Ok(json) => { /* JSON string, including its quotes */ }
        Err(error) => { /* unavailable view, script error, or engine failure */ }
    }
});
```

[`JsHandle::eval()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/struct.JsHandle.html#method.eval)
returns a future. The caller supplies trusted code. Do not concatenate untrusted strings into scripts; serialize
values with a JSON encoder. Returned values are serialized as JSON. `undefined` becomes `null`;
exceptions and values that cannot be serialized become errors. This is synchronous JavaScript
evaluation, not an API for awaiting JavaScript promises.

Disposing the Day node invalidates its handle; callers must handle errors when navigating away.
Use [browser profiles](#browser-profiles) to manage persistent storage.

### Evaluation support

[`eval_support()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/fn.eval_support.html) reports whether an evaluation implementation exists. Browser same-origin rules
still apply: a bundled site works; an inaccessible cross-origin frame returns an error. Remote
sites can also refuse iframe embedding. This API does not bypass those restrictions.

> [!WARNING]
> **Windows / GTK / Qt: web engines.**
> Child-window hosting requires the WebView2 Runtime and remains unverified on Windows.

> [!WARNING]
> **HarmonyOS / ArkUI: web engine.**
> Evaluation requires a compatible ArkWeb engine; device behavior remains unverified.

## Browser profiles

Use a browser profile to separate website accounts or retain a login across app launches.
A profile controls cookies and engine-managed website data. It is independent of a retained
browser session, which controls the lifetime of the view itself.

### Choose a profile before building a view

```rust
use day::prelude::*;
use day_piece_webview::{WebProfile, web_view};

pub fn account_browser(url: Signal<String>, account_key: String) -> impl Piece {
    web_view(url).profile(WebProfile::persistent(account_key))
}
```

[`WebProfile::persistent()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/struct.WebProfile.html#method.persistent)
shares storage between views using the same identity and retains it across launches where the
engine supports named profiles. The key is an app-defined identifier; do not put credentials
or other secrets in it. Changing a profile requires rebuilding the view.

[`WebProfile::private()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/struct.WebProfile.html#method.private)
requests an in-memory store. It does not erase existing persistent storage or make
app-managed downloads and databases private. Unsupported private views are blocked rather
than silently opening with persistent storage.

### Check capabilities

| Engine | Profile behavior |
| --- | --- |
| WKWebView | Named persistent stores require macOS 14 / iOS 17+. Older systems isolate explicit named profiles in memory. Private views use a nonpersistent store. |
| Qt WebEngine / WebKitGTK | Named disk profiles and ephemeral private profiles; app-selected storage roots are supported. |
| WebView2 | Named profiles and InPrivate mode depend on the installed runtime's capabilities. |
| Android WebView | Named profiles require AndroidX `MULTI_PROFILE`; complete deletion requires `DELETE_BROWSING_DATA`. Stock WebView has no private mode through this API. |
| ArkWeb | Named profiles are emulated over app-wide, origin-separated storage. Private mode is available; complete profile clearing is unsupported. |
| Web iframe | The host browser owns storage. Named profiles, private mode, and complete deletion are unsupported. |

Check [`profile_support()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/fn.profile_support.html),
[`private_browsing_support()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/fn.private_browsing_support.html),
and [`clear_data_support()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/fn.clear_data_support.html)
before offering the corresponding controls.

### Cookies and clearing data

Attach a [`JsHandle`](https://docs.rs/day-piece-webview/latest/day_piece_webview/struct.JsHandle.html)
to address a realized view. Its
[`cookie_header()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/struct.JsHandle.html#method.cookie_header)
operation returns eligible native cookies, including HttpOnly cookies, for a URL. Treat the
result as a credential. For a redirected native HTTP request, obtain cookies for the new
destination rather than forwarding the old header to another origin.

[`set_cookie()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/struct.JsHandle.html#method.set_cookie)
imports a response cookie into the same jar.
[`clear_data()`](https://docs.rs/day-piece-webview/latest/day_piece_webview/struct.JsHandle.html#method.clear_data)
clears the entire profile, including cookies, cache, and persistent website storage. Await its
result before making new requests. Give each site a dedicated profile when a Forget Site
action must not erase unrelated accounts. Qt resets affected pages while clearing the profile.

Use [authentication](/docs/reference/auth) for provider sign-in and
[keychain storage](/docs/reference/keychain) for app-owned tokens. Browser profile storage is
not an app token vault.

## Related documentation

- [Lottie animations](/docs/reference/lottie) for animation playback in a web view.
- [Apple: WebKit](https://developer.apple.com/documentation/webkit)
- [Android: WebView](https://developer.android.com/develop/ui/views/layout/webapps/webview)
