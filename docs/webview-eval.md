# Web view JavaScript evaluation

> [!IMPORTANT]
> **Status: partly implemented.** The front-end ships — `JsHandle::eval` returning a future, the
> JavaScript envelope, the request/reply codec, `eval_support()`, and the `num`-keyed split that
> keeps evaluation replies from clobbering the URL readback. **AppKit, UIKit, Qt and XAML** have
> working arms; `eval_support()` reports `Native` there.
>
> **GTK, Android and ArkUI** carry an inert `WebPatch::Eval` arm so their `match` stays exhaustive,
> and report `Unsupported`. The per-platform research below is what those arms need; the order in
> [Implementation order](#implementation-order) still holds, minus the three that are done.
> **web-dom can never do this** — `contentWindow.eval` throws across origins.
>
> Verified end to end on **macos-qt (21/21 script steps)** and **macos-appkit (20/21 — only the
> engine-specific `SyntaxError` wording differs)**: values, object serialization, thrown exceptions
> and syntax errors all round-trip. Eight unit tests pin the codec and the escaping.
>
> **windows-xaml verified on real hardware, 2026-08**, driving the showcase's JS console against
> `https://daybrite.dev`: `document.title` → the page's title, `1+1` → `2`,
> `({a:1,b:[2,3],c:'hi'})` → `{"a":1,"b":[2,3],"c":"hi"}`, `undefined` → `null`,
> `throw new Error('boom')` → `Error: boom`, a self-referential object →
> `TypeError: Converting circular structure to JSON`, and a string carrying an escaped quote and
> `é` round-tripping as `café`. `1 +` reports **`SyntaxError: Unexpected end of input`** —
> the case the JS envelope structurally cannot catch, delivered by `ExecuteScriptWithResult`'s
> engine-level error channel. See [webview.md](./webview.md) for the shipped piece.

Goal: `js.eval("document.title").await` returning a value from the embedded engine, on every backend
that has one.

## The short answer

Every backend with a real engine offers the same primitive — submit a script string, get one value
back asynchronously, on the UI thread. What they do *not* share is the error channel, the value
type, or any guarantee that the callback arrives at all. Three lowest common denominators fall out,
and they drive the whole design:

1. **The value is a JSON string.** Android, WebView2 and ArkUI only ever produce one.
2. **The error channel must be built in JavaScript**, because Qt and Android have none.
3. **Delivery is at-most-once**, because Android and WebView2 drop pending callbacks on teardown.

## Capability matrix

| | appkit | uikit | gtk | qt | android | xaml | arkui | dom |
|---|---|---|---|---|---|---|---|---|
| Async eval with result | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | **✗** |
| Native error channel | ✓ rich | ✓ rich | ✓ message | **✗** | **✗** | ✓ on `_21` | ~ `Ext` only | — |
| Reports *syntax* errors | ✓ | ✓ | ✓ | ✗ | ✗ | ✓ on `_21` | ? | — |
| Awaits promises | ✓ ¹ | ✓ ¹ | ✓ ² | ✗ | ✗ | ✗ | ✗ | — |
| Isolated world | ✓ ¹ | ✓ ¹ | ✓ | ✓ | ✗ | ✗ | ✗ | — |
| Cancel in flight | ✗ | ✗ | ✗ ³ | ✗ | ✗ | ✗ | ✗ | — |
| Callback guaranteed | exactly once | exactly once | exactly once | exactly once ⁴ | **at most once** | **at most once** | ? | — |
| `num` correlation slot | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | **✗** ⁵ | n/a |

¹ `callAsyncJavaScript` / `WKContentWorld`, macOS 11 / iOS 14.
² `call_async_javascript_function`.
³ The `GCancellable` converts the completion to `Err(CANCELLED)`; the script still runs.
⁴ Qt fires pending callbacks with an invalid `QVariant` during page destruction — unusually strong.
⁵ ArkUI's generic piece channel hardcodes `num = 0.0`. This is the one framework change required.

Note the shape of the table: the axes where backends differ are exactly the axes a naive API would
expose. Everything below is about collapsing them.

## Per-platform detail

### Apple — WKWebView (appkit, uikit)

Two methods. `evaluateJavaScript:completionHandler:` (macOS 10.10 / iOS 8) evaluates a *program* and
yields the completion value of the last expression. `callAsyncJavaScript:arguments:inFrame:inContentWorld:completionHandler:`
(macOS 11 / iOS 14) wraps the body in `(async function(…){…})`, so `await` works, promises are awaited,
thenables are resolved, and arguments pass structurally instead of by string interpolation. The async
form requires `return`; the plain form forbids it.

The error model is the best of any backend. Errors arrive as `NSError` in `WKErrorDomain`:

| Code | Meaning |
|---|---|
| 4 `JavaScriptExceptionOccurred` | the script threw, **or** a promise rejected |
| 5 `JavaScriptResultTypeIsUnsupported` | value can't cross IPC (function, DOM node, un-awaited Promise) |
| 12 `JavaScriptInvalidFrameTarget` | target frame vanished mid-navigation |
| 14 `JavaScriptAppBoundDomain` | blocked by app-bound domains |

`userInfo` carries `WKJavaScriptExceptionMessage`, `…LineNumber`, `…ColumnNumber`, `…SourceURL`. Read
those by literal string key — the `_WK…ErrorKey` symbols are SPI and must not be linked.

Value mapping: `undefined` → nil with no error; `null` → `NSNull`; numbers are **always** doubles;
`Date` → `NSDate`; `Map`/`Set`/`Error` → empty dictionaries. Cyclic objects **succeed** and produce a
genuinely self-referential `NSDictionary`, which will hang `NSJSONSerialization`. That alone argues
for stringifying in JS rather than marshaling `id` graphs.

Code 5 is overloaded: a dead web content process also reports it, with empty `userInfo`. Disambiguate
on the presence of `WKJavaScriptExceptionMessage`.

Both methods must be called on the main thread; the handler is documented to always run there, and
never re-entrantly. There is no cancellation of any kind.

Availability note: `objc2-web-kit` binds `WKWebView` for **macOS only**, so the UIKit arm must send
these by selector, exactly as it already hand-rolls the rest of its `WKWebView` usage.

### GTK — WebKitGTK 6

`WebViewExt::evaluate_javascript(script, world_name, source_uri, cancellable, callback)` and a
`…_future` variant. Errors are real: domain `WebKitJavascriptError`, code 699 `SCRIPT_FAILED` for a
thrown exception, with the message pre-formatted as `"<source_uri>:<line>:<col>: <message>"` — which is
why `source_uri` is worth passing even though it doesn't affect execution.

`JSCValue::to_json(indent)` returns `Option<GString>`, and `None` is ambiguous across `undefined`, a
function, and a cyclic object. Worse, the cyclic case leaves a **latent exception on the `JSCContext`**
that poisons the next operation unless cleared with `context.clear_exception()`.

Any non-`NULL` `world_name` creates a distinct isolated world. The binding asserts it is called on the
thread owning the default `MainContext`, and the closure is thread-guarded — dropping it on another
thread aborts. Cancellation is result-side only.

### Qt — QWebEngineView

`QWebEnginePage::runJavaScript(script[, worldId], callback)`, with `worldId` 0 = main, 1 =
`ApplicationWorld`. The overloads became templates in Qt 6.12, so the shim should pass a bare lambda
rather than an explicit `std::function` to compile on both sides of that change.

**There is no error channel.** A thrown exception, a syntax error, an unsupported return type, a
discarded page and a genuine `null` all arrive as the same default-constructed, invalid `QVariant`.
Qt reports these to stderr via `qWarning` instead. This is a design fact confirmed in
`web_contents_adapter.cpp`, not a bug awaiting a fix.

The compensating strength is lifetime: Qt guarantees the callback runs even during page destruction
(with an invalid value), so a boxed Rust closure is always reclaimed exactly once. The matching
hazard is that touching the `QWebEnginePage`/`QWebEngineView` inside that late callback is undefined
behaviour — the shim lambda must capture PODs only.

Values round-trip through Chromium's `base::Value`, so `Date` arrives as a string and `1` vs `1.0`
can differ in `QVariant` type. `QJsonDocument::fromVariant` is lossy for top-level scalars, which is
a second reason to stringify in JS.

### Android — android.webkit.WebView

`evaluateJavascript(String, ValueCallback<String>)`, API 19, so unconditionally available at Day's
`minSdk = 24`. Both the call and the callback are on the UI thread, and Chromium posts the callback
to a fresh looper turn rather than invoking it inline.

**No error channel.** A thrown exception and a returned `undefined` both produce the literal string
`"null"`. Two quirks beyond that: when JSON writing fails the callback receives the **empty string**,
which is not valid JSON and must be treated as an engine failure; and cyclic objects null only the
*repeating member* (`{"self":null}`) rather than throwing as `JSON.stringify` would.

Delivery is at most once. If the WebView is destroyed first, `AwContents` early-returns and the
callback **never fires**, so a pending-request map leaks without a timeout.

### Windows — WebView2 (xaml)

`ExecuteScriptAsync` has the same `"null"`-for-everything weakness as Android. But
**`ExecuteScriptWithResult` on `ICoreWebView2_21` fixes it properly**: `Succeeded`, `ResultAsJson`, and
an `ICoreWebView2ScriptException` carrying name, message, line and column. Because it works at the
engine level rather than in JS, it also catches **syntax errors** — the one case the JS envelope
below cannot.

It arrived in SDK 1.0.2277.86; the piece already pins 1.0.3179.45, so the header is present. The
*runtime* is not guaranteed, so probe with `try_query<ICoreWebView2_21>()` and fall back to
`ExecuteScript` plus the envelope. Note the documented trap on `get_Exception`: it returns `S_OK` even
when acquisition fails, so null-check the out-pointer rather than the `HRESULT`.

Callbacks run on the creating UI thread, serially, never re-entrantly. `Close()` releases pending
handlers, so delivery is at most once.

### HarmonyOS — ArkUI

`WebviewController.runJavaScript(script)` returns `Promise<string>` (API 9). `runJavaScriptExt`
(API 10) returns a `JsMessageExt` with typed getters including `getError()` and a `JsMessageType::ERROR`
variant, which appears to be the only way to recover a JS exception. Controller calls fail with
`17100001` unless the controller is attached to a live `Web`, so evaluation must be gated on
`onControllerAttached` or later.

Both facts marked ? in the matrix are unverified — official documentation endpoints were unreachable
during this research. Settle them against the installed DevEco SDK's `@ohos.web.webview.d.ts` before
implementing. Also note this arm cannot be tested on the x86_64 emulator at all, because `ArkWebCore.hap`
carries arm64-only native libraries.

### web-dom — iframe

Impossible cross-origin. `iframe.contentWindow.eval(...)` throws `SecurityError`, the same wall that
blocks history and URL readback. Same-origin content could be evaluated, but the piece cannot know the
origin ahead of time and the failure is silent, so this arm reports the capability as unsupported and
does not try.

## The design

### 1. Wrap every script, and `eval` it from a string literal

This single decision lifts Qt and Android to roughly the error fidelity of WebKit, and normalizes
value marshaling on all seven backends. The shipped wrapper passes the script to `eval` as an
escaped **string literal** rather than splicing it in as source — which, unlike splicing, also
catches syntax errors and accepts statements (`throw …`, `var a = 1; a + 1`), because `eval`
compiles at run time inside the `try`. The cost is that a page whose CSP omits `unsafe-eval`
refuses it, reported as a legible caught error. The original spliced form is kept below for
contrast:

```js
(function(){try{var v=(/*USER*/
);return JSON.stringify({ok:1,v:v===undefined?null:v,u:v===undefined})}
catch(e){return JSON.stringify({ok:0,n:(e&&e.name)||"",m:(e&&e.message)||String(e),s:(e&&e.stack)||""})}
})()
```

The result is therefore always a *string*, which every backend can carry, and which JSON-encodes to a
quoted string at the outer layer. Day parses twice. That double encoding is deliberate: the outer
parse now succeeds whenever the engine worked at all, so an empty string or an outer parse failure
becomes an unambiguous engine-level failure rather than being confused with a script-level one.

Six things this must account for:

- **A syntax error in the user script is not caught** — the wrapper and the script compile as one
  unit. Only WebView2's `ExecuteScriptWithResult` reports it natively. `new Function(...)` would move
  compilation inside the `try` but breaks under a CSP without `unsafe-eval`, so it is not the default.
- **Emit a newline before the closing braces**, or a trailing `//` comment in the user script swallows
  them.
- **`JSON.stringify` drops unserializable values**, so a missing `v` key means "returned something
  unserializable" and is distinct from `"v":null`.
- **Cycles now throw** and get reported, where the raw Android API silently nulled a member.
- **`BigInt` throws** in `JSON.stringify`.
- **Wrapping adds a function scope**, so top-level `var` and function declarations no longer leak to
  `globalThis`. Scripts that set up global state need the unwrapped path.

Two evaluation modes follow: `Expression` (the wrapper above) and `Statements` (the body becomes an
IIFE and the caller writes `return`).

### 2. Correlate on `Event::Custom`'s `num`

`Event::Custom { tag, num: f64, text }` is the documented open channel, and `num` already survives
every native boundary except one. Correlation ids start at 1; `f64` is exact to 2^53.

This keeps the whole mechanism inside the piece. The alternative — a dedicated `Event` variant with a
`req` field, intercepted in `pump_events_inner` before node dispatch, as `Event::PresentResult` does —
is the house pattern for correlated results, but it would require day-core and `BridgeKind` to learn
about an *external* piece's feature. That is the wrong direction for something that exists to be
implementable without touching day.

Two consequences:

- **The URL readback collides and must be fixed first.** `lib.rs` currently treats *any* `Custom` on
  the node as the URL, and its comment says so. URL reports keep `num == 0.0`; eval replies use
  `num >= 1.0`; the handler branches. Without this, an eval result overwrites the user's URL bar.
- **ArkUI needs a two-line framework fix.** `day-arkui-sys`'s `PieceEvent` passes a literal `0.0`, and
  the ArkTS-facing `pieceEvent(id, text)` has no `num` parameter. The Rust decode already forwards
  `num`, so widening the shim to `(id, num, text)` — defaulting to `0.0` when `argc < 3` — is the
  whole change. This also makes ArkUI match Android, which has carried `num` all along.

### 3. Mirror `day_core::present`, not `FetchFuture`

Eval completions all arrive on the UI thread, so the future is `Rc`/`RefCell` with a stored `Waker`,
like `day_core::present`, rather than `day-part-http`'s `Arc`/`Mutex` (which exists because HTTP
completes on a background thread). Register lazily in `poll` so nothing is dispatched until awaited,
resolve by looking the id up in a thread-local map, and follow `docs/async.md`'s rule that a piece
offers a callback *and* a future and never calls `on_main` itself.

Because no backend can cancel a running script, `Drop` deregisters the pending id so a late reply is
a silent no-op. The script keeps running; the result is discarded. Say this plainly in the API docs.

Because delivery is at most once on Android and WebView2, **every pending request needs a timeout**,
and the map must be drained when the node is disposed. Note also that `pump_events` clears the whole
in-flight batch if any handler panics, which would strand sibling requests — another reason the
timeout is mandatory rather than a nicety.

### 4. Surface, sketched

```rust
let js = JsHandle::new();
web_view(url).js(js);

day::task(async move {
    match js.eval("document.title").await {
        Ok(json) => …,                        // "\"Daybrite\""
        Err(EvalError::Threw { message, .. }) => …,
        Err(EvalError::Unsupported) => …,     // web-dom
    }
});
```

```rust
pub enum EvalError {
    Unsupported,                                   // no engine, or web-dom
    Threw { name: String, message: String, stack: String },
    Unserializable,                                // ran, value could not cross
    Timeout,
    ViewGone,
    Engine(String),                                // could not run or decode
}
pub enum EvalWorld { Isolated, Page }              // honoured on apple/gtk/qt, ignored elsewhere
pub enum EvalMode  { Expression, Statements }
```

`EvalWorld` is a hint, not a guarantee — Android, WebView2 and ArkUI have no world concept, and an
isolated world cannot read page globals on the backends that do. Default to `Isolated` and let callers
opt into `Page`.

A separate `eval_support()` is needed alongside `support()`: the two axes have diverged, since the dom
arm loads pages but cannot evaluate at all.

## Implementation order

1. **Front-end** — `WebPatch::Eval { req, script }`, the envelope builder, the pending map, the future,
   `eval_support()`, and the `num`-based fix to the URL handler. Testable against day-mock with a
   programmatic responder, mirroring `present`'s dayscript escape hatch.
2. **GTK** — cheapest real arm. No new dependencies; `webkit6` is already there and re-exports
   `javascriptcore`. Handle `to_json` returning `None` by checking and clearing the context exception.
3. **Apple** — add `block2` to the `appkit` and `uikit` features, plus the explicit `objc2-web-kit`
   features (`WKContentWorld`, `WKError`, `block2`) and `objc2-foundation` features (`NSError`,
   `NSDictionary`, `NSValue`, `NSNull`) that the lists currently omit. Factor the result marshaling into
   a shared file included by both arms, since only dispatch differs.
4. **Android** — `DayWebView.webEval(view, id, corr, script)` calling `evaluateJavascript` and reporting
   through `DayBridge.nativeOnEvent(id, 12, corr, value)`. No new Gradle dependency, no day-android edit.
5. **Qt** — two new C-ABI functions in *both* branches of the shim. The `#else` no-engine branch must
   define them too, or windows-qt fails to link; that is the most likely way this breaks CI.
6. **XAML** — **done.** `ComPtr::As<ICoreWebView2_21>` with an `ExecuteScript` fallback, `find_ctx`
   guard, `CoTaskMemFree` on every out-string. Two details the sketch above did not anticipate:
   `TryGetResultAsString` returns the wrapper's string directly, so the success path needs no JSON
   walk at all (the same shape Qt's `QVariant` and WebKit's `NSString` hand back) — but the
   *fallback* path returns the result AS JSON, so it needs a real JSON string decoder including
   `\u`, because the protocol's own `` separator arrives escaped.
7. **ArkUI** — the `PieceEvent` widening first, then the arm. Verify on arm64 hardware.

## Open questions

- Whether a JS exception rejects HarmonyOS's `runJavaScript` promise or resolves it to `null`, and
  whether `runJavaScriptExt` is genuinely the only way to recover an error object.
- Whether WebKitGTK still invokes the callback when `WebKitSettings:enable-javascript` is false. The
  docs say the method "will do nothing"; do not assume the callback arrives.
- Any practical script-length ceiling on Android's Mojo transport. None is documented.
- Whether Apple's cycle-preservation and `Map`/`Set` → empty-dictionary behaviour hold on releases
  older than the 2025 `JavaScriptEvaluationResult` rewrite. The envelope makes this moot in practice.
