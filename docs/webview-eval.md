---
title: "Web view JavaScript evaluation"
description: "Run JavaScript in an embedded page and await its JSON result."
---

# Web view JavaScript evaluation

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
`0␟<error name>␟<message>` (`␟` denotes U+001F). Backends remove their native string/JSON wrapper
before delivering this envelope. Replies are separate from URL changes and external-link events.
Disposing the Day node invalidates its handle; callers must handle errors when navigating away.

| Target | Implementation |
|---|---|
| macos-appkit, macos-gtk, ios-uikit | WKWebView `evaluateJavaScript` |
| linux-gtk | WebKitGTK `evaluate_javascript` |
| macos-qt, linux-qt | QWebEngine `runJavaScript` |
| windows-winui | WinUI's `WebView2` control, driving its `ICoreWebView2` (the deprecated windows-xaml hosts a WebView2 composition controller) |
| windows-gtk, windows-qt without Qt WebEngine | WebView2 child window through Wry |
| android-mdc | Android WebView `evaluateJavascript` |
| harmony-arkui | ArkWeb `runJavaScript` |
| web-dom | Same-origin iframe `contentWindow.eval` |

`eval_support()` reports whether an evaluation implementation exists. Browser same-origin rules
still apply: a bundled site works; an inaccessible cross-origin frame returns an error. Remote
sites can also refuse iframe embedding. This API does not bypass those restrictions.

The Windows GTK/Qt child hosts are not runtime-verified on Windows. They require the WebView2
Runtime. The common Rust host cross-checks against `x86_64-pc-windows-msvc`; the Qt hosting code
has a C++ syntax check. Linux GTK's API is checked against the installed `webkit6` crate source.
Harmony requires a compatible ArkWeb engine; a successful Rust build alone does not verify it.

## Testing

In the `day-piece-webview` repository, `cargo test --lib` covers encoding, error replies and native path handling.
`node --test tests/browser.mjs` checks browser link routing and same-origin evaluation/error replies.
The demo's `dayscript/webview.yaml` checks the actual bundled page and return path:

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
