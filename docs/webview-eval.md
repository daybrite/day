---
title: "Web view JavaScript evaluation"
description: "Run JavaScript in an embedded page and await its JSON result."
---

# Web view JavaScript evaluation

Run JavaScript in an embedded web view and receive its result in Rust. Use a `JsHandle` to
address the view after its page has loaded.

## Add to your app

Defined in [`day-piece-webview`](https://github.com/daybrite/day-piece-webview).
Add the dependency to your app’s `Cargo.toml`:

```toml
[dependencies]
day-piece-webview = { git = "https://github.com/daybrite/day-piece-webview.git" }
```

`day build -t <target>` enables the matching toolkit feature and stages native contributions.
No dependency entry is needed in `Day.toml`.

See [platform support](#platform-support) for required web engines and evaluation restrictions.

## Evaluating JavaScript

Bind a `JsHandle` to the web view, then evaluate after its page loads:

```rust
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

The caller supplies trusted code. Do not concatenate untrusted strings into scripts; serialize
values with a JSON encoder. Returned values are serialized as JSON. `undefined` becomes `null`;
exceptions and values that cannot be serialized become errors. This is synchronous JavaScript
evaluation, not an API for awaiting JavaScript promises.

Each request has its own numeric ID. A reply contains either `1␟<JSON>` or
`0␟<error name>␟<message>` (`␟` denotes U+001F). Toolkits remove their native string/JSON wrapper
before delivering this envelope. Replies are separate from URL changes and external-link events.
Disposing the Day node invalidates its handle; callers must handle errors when navigating away.

## Platform support

| Target | Implementation |
|---|---|
| macos-appkit, macos-gtk, ios-uikit | [WKWebView](https://developer.apple.com/documentation/webkit/wkwebview) `evaluateJavaScript` |
| linux-gtk | WebKitGTK `evaluate_javascript` |
| macos-qt, linux-qt | QWebEngine `runJavaScript` |
| windows-winui | WinUI's `WebView2` control, driving its [`ICoreWebView2`](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2) |
| windows-gtk, windows-qt without Qt WebEngine | WebView2 child window through Wry |
| android-mdc | Android WebView `evaluateJavascript` |
| harmony-arkui | ArkWeb `runJavaScript` |
| web-dom | Same-origin iframe `contentWindow.eval` |

`eval_support()` reports whether an evaluation implementation exists. Browser same-origin rules
still apply: a bundled site works; an inaccessible cross-origin frame returns an error. Remote
sites can also refuse iframe embedding. This API does not bypass those restrictions.

> [!WARNING]
> **Windows / GTK / Qt: web engines.**
> Child-window hosting requires the WebView2 Runtime and remains unverified on Windows.

> [!WARNING]
> **HarmonyOS / ArkUI: web engine.**
> Evaluation requires a compatible ArkWeb engine; device behavior remains unverified.

## Testing embedded content

Use `web_eval` to check the bundled page and its return path:

```yaml
- web_eval:
    id: webview
    script: "document.getElementById('script-status').textContent"
    text: "The bundled script ran."
    timeout_secs: 30
```

Use `text` for an exact result or `contains` for a substring. An omitted assertion only checks
that evaluation succeeds. For rendered-content tests, also assert dimensions and inspect a
screenshot: a page can execute JavaScript while its content has no visible layout space.

## Platform guidance

- [Apple: WebKit](https://developer.apple.com/documentation/webkit)
- [Android: WebView](https://developer.android.com/develop/ui/views/layout/webapps/webview)
