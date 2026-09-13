---
title: "HTTP"
description: "HTTP through each platform's network stack via day-part-http, inheriting system proxies and TLS."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# HTTP through the platform stack (headless capability crate)

> **Status: implemented** as `day-part-http` (in `parts/`), a headless day-ecosystem crate with no
> UI Piece: request/response HTTP (plus streaming downloads) through each platform's networking
> stack: NSURLSession on macOS/iOS, OkHttp on Android (the platform's frozen engine,
> current; see the engine note below), WinHTTP on Windows, the browser's `fetch()` on the web
> (`web-dom`, async entry points only; see the web tier below), with a
> bundled ureq + rustls fallback on Linux and HarmonyOS. Verified end-to-end with a local-server
> test suite on the real Apple half (macOS) and the real fallback half (Linux), and live on
> macOS/iOS-sim/Android-emulator/browser via the showcase walkthrough and Day Skies' Open-Meteo
> fetch.

Day uses the platform stack because the OS already knows things an app cannot easily discover
(system proxies and PAC scripts, per-network VPN routing, Low Data Mode, enterprise/MDM
certificate stores, user-installed CAs). Apps that fetch through the platform inherit all of it,
and the native targets use the platform's TLS (rustls compiles only into the cfg-gated Linux/OHOS
fallback).

## Authoring

```rust
use day_part_http::{Request, fetch};

// Blocking — call it off the UI thread (a worker thread of your own).
let resp = fetch(&Request::get("https://api.example.com/data.json"))?;
if (200..300).contains(&resp.status) {
    let body: MyData = serde_json::from_slice(&resp.body)?;
}
```

`Request` is a builder: `get/post/put/delete/patch/head(url)`, `.header(k, v)` (duplicates
allowed), `.body(Vec<u8>)`, `.timeout(Duration)`, `.allow_expensive(bool)` /
`.allow_constrained(bool)`. `Response { status, headers, body }` adds `text()` (lossy UTF-8) and a
case-insensitive `header(name)`.

Two contract points differ from ureq-style clients:

- **4xx/5xx are `Ok`.** An HTTP error status is a *response* (`resp.status == 404`), not an
  `HttpError`. Errors are transport-level only: `BadUrl`, `Timeout`, `Dns`, `Connect`, `Tls`,
  `Io`, `Unsupported` (the enum is `#[non_exhaustive]`).
- **`timeout` bounds progress, not the transfer.** It covers connecting, awaiting the response
  head, and idle gaps; a multi-minute download that keeps moving is never cut off. Default 30 s.

### Async + the Setter idiom

```rust
let status: Signal<String> = Signal::new(String::new());
let done = status.setter(); // Copy + Send; hops to the UI thread itself
day_part_http::fetch_async(Request::get(url), move |result| {
    // Runs on an UNSPECIFIED BACKGROUND thread (URLSession's delegate queue on Apple,
    // OkHttp's dispatcher on Android, a spawned thread on Windows and the Rust fallback).
    // Never touch UI state directly here.
    if let Ok(resp) = result {
        done.set(resp.text()); // no-ops harmlessly if the page was disposed meanwhile
    }
});
```

`fetch_async(req, on_done)` completes on a background thread, because the crate never calls
`day_reactive::on_main` (which requires an installed backend poster and would break plain-`main`
programs and `cargo test`). Capturing a `Setter` in `on_done` is the standard delivery idiom
(DESIGN §4.5); it marshals to the UI thread itself and absorbs late deliveries after disposal.
The showcase's Network & HTTP page demonstrates it twice: a deterministic local fetch (a
one-shot loopback server natively; on web-dom, where a tab can host no listener, the dev
server's same-origin `/day-http-ok` echo endpoint with identical bodies), and
a URL checker (type any http(s) URL, tap Check) that prints the response headers and body size.
`resp.headers` is the full header list, `resp.header(name)` the case-insensitive lookup.

### Feeding remote-image

`day-piece-remote-image` stays fetch-agnostic (the app owns the bytes signal), but gains the
one-liner for the common case:

```rust
remote_image_url("https://example.com/logo.png").rounded(8.0)
```

`remote_image_url` fetches once through `day-part-http` and pushes 2xx bytes into the piece's own
signal via a `Setter`; failures leave the placeholder color showing.

### Downloads and streaming

```rust
// Straight to disk — the body never sits in memory.
let dl = fetch_to_file(&Request::get(apk_url), &dest)?;   // Download { status, headers, bytes_written }

// Full control — progress, cancellation, incremental hashing:
struct MySink { /* progress handle, hasher, file … */ }
impl StreamSink for MySink {
    fn head(&mut self, status: u16, headers: &[(String, String)]) -> bool {
        status == 200 // returning false aborts before any chunk
    }
    fn chunk(&mut self, data: &[u8]) -> Result<(), HttpError> {
        /* hash + write + report; return Err to cancel mid-body */ Ok(())
    }
}
let dl = fetch_streamed(&Request::get(url), &mut MySink { .. })?;
```

`fetch_to_file` has an async twin (`fetch_to_file_async`). App Fair's downloader is the shipped
reference: a `StreamSink` that hashes as it writes, reports progress, honors a cancel flag, and
implements HTTP `Range` resume by deciding append-vs-restart in `head()`.

## Per-platform native realization

| OS | API | dependency |
|---|---|---|
| macOS + iOS | `NSURLSession` (shared ephemeral session; per-request delegate session for streaming) | objc2-foundation, shared `apple.rs` |
| Android | OkHttp 4.12: the asynchronous forms through a daybridge Java arm (`src/bridge.rs`, a `Done<Vec<u8>>` completed from OkHttp's dispatcher — [docs/bridge.md](bridge.md) "Callbacks"), the blocking forms through the part-owned `DayHttp.java` shim; one `byte[]` envelope per call either way | `day-bridge`, `day-android` + `[package.metadata.day.android]` (staged Java + the okhttp Gradle coordinate) |
| Windows | WinHTTP (winhttp.dll, resolved dynamically; `WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY`): a synchronous session for the blocking forms, a `WINHTTP_FLAG_ASYNC` session with a status callback for `fetch_async`/`fetch_future` | raw FFI (runtime lookup) |
| Web (`web-dom`) | the browser's `fetch()` via the day-dom shim's `day_dom_http_*` imports (request-id + AbortController); **async entry points only** — `fetch`/`fetch_to_file`/`fetch_streamed` return `Unsupported` | `web.rs` (wasm32; requires the day-dom host page, the day-part-prefs pattern) |
| Linux | ureq 3 + rustls (the only tier that bundles TLS) | ureq, `fallback.rs` |
| HarmonyOS | the ArkTS Network Kit (`@ohos.net.http`) through a daybridge ArkTS arm (`src/bridge.rs`): the OSS NDK has no HTTP C API, so the request runs on the JS thread and completes the bridge token; the blocking forms wait for that completion and answer `Unsupported` on the JS thread itself | `day-bridge`, `ohos.rs` (native only when `day build` staged the arm; a bare cargo build reports `Unavailable`) |
| unknown/mock | catch-all: every call returns `HttpError::Unsupported` | — |

`tier()` reports which of the three tiers the compiled target uses (`NativeStack`,
`RustFallback`, or `Unavailable`), so an app (or a doc table) never has to guess:

- **NativeStack**: system proxy + PAC, VPN routing, platform TLS + certificate stores all apply.
  The web is this tier: the browser is the platform stack (proxies, TLS, certificate store,
  HTTP/2/3 all come from it). But it is async-only: on the single browser thread a blocking wait
  would starve the event loop the completion needs ([docs/web.md](web.md)), so the blocking entry points
  return `Unsupported` while `fetch_async`/`fetch_future` work in full. Two web-only rules
  apply: CORS governs cross-origin requests (and limits which response headers are visible),
  and browser-controlled headers (`Host`, `Cookie`, `Origin`, …) cannot be set from a request.
- **RustFallback**: correct HTTP(S) via rustls + webpki roots, but system awareness is limited to
  the `http_proxy`/`https_proxy`/`no_proxy` environment variables (no PAC, no desktop proxy
  settings).
- **Unavailable**: every call fails with `Unsupported` (the mock/unknown-target posture).

## Error mapping

| `HttpError` | Apple (`NSURLErrorDomain`) | Android (exception) | Windows (`ERROR_WINHTTP_*`) | web (fetch rejection) | fallback (ureq) |
|---|---|---|---|---|---|
| `Timeout` | −1001 | `SocketTimeoutException` | 12002 | `AbortError` from the timeout timer | `Timeout` |
| `Dns` | −1003, −1006 | `UnknownHostException` | 12007 | — (see below) | `HostNotFound` |
| `Connect` | −1004, −1009 | `ConnectException` | 12029, 12030 | — (see below) | `ConnectionFailed` |
| `Tls(msg)` | −1200…−1206 | `SSLException` | secure-failure set (12157, 12175, …) | — (see below) | `Tls` |
| `BadUrl` | −1000, −1002 | `IllegalArgumentException` (URL rejected) | 12005, 12006 | `new URL(...)` rejects | `BadUri` |
| `Cancelled` | −999 | `Call.isCanceled()` (sentinel −7) | — (discard tier) | `AbortError` from `day_dom_http_abort` | — (discard tier) |
| `Io(msg)` | anything else | anything else | anything else | anything else (see below) | anything else |

The web column is coarse because browsers collapse DNS, connect, TLS, and CORS failures
into one opaque `TypeError` (an anti-fingerprinting measure), so every network-level failure
surfaces as `Io` with the browser's message; `Dns`/`Connect`/`Tls` never occur on this tier.

## Options: applied vs accepted

Options that only some platforms can realize are documented per platform:

| option | Apple | Android | Windows | web | fallback |
|---|---|---|---|---|---|
| `.timeout` | `timeoutInterval` (idle timer) | OkHttp connect/read/write per-phase bounds (no callTimeout) | per-operation `WinHttpSetTimeouts` | an abort timer over connect + response head (body phase uncapped — fallback parity; fetch has no native timeout) | resolve/connect/send/response-head timeouts (body phase uncapped) |
| `.allow_expensive` / `.allow_constrained` | native (`allowsExpensiveNetworkAccess` / `allowsConstrainedNetworkAccess`, Low Data Mode) | advisory only | advisory only | advisory only | advisory only |
| `.header` | as given | as given | as given | browser-controlled names (`Host`, `Cookie`, `Origin`, …) are ignored per the fetch spec | as given |
| redirects | followed (no opt-out in v1) | followed | followed | followed | followed |

## App Transport Security (iOS/macOS) and Android cleartext

Both mobile platforms restrict plain `http://` by default; the platform stack enforces the
platform's policy, and two notes apply:

- **ATS** (Apple): `NSURLSession` refuses non-HTTPS URLs unless the app's Info.plist carries an
  exception (`NSAppTransportSecurity`). Loopback IP fetches (`http://127.0.0.1:…`) are exempt;
  the showcase's local demo needs no plist changes. For a real cleartext host, add a scoped
  `NSExceptionDomains` entry; don't reach for `NSAllowsArbitraryLoads`.
- **Android cleartext**: blocked app-wide since targetSdk 28, including loopback. The showcase
  scaffold ships a `network_security_config.xml` permitting cleartext to `127.0.0.1` only (plus
  the `android:networkSecurityConfig` manifest attribute); scope any real exception the same way.

The fallback tier performs no such policy enforcement (ureq fetches `http://` without
restriction), another reason `tier()` exists.

## Threading

`fetch`/`fetch_to_file`/`fetch_streamed` block the calling thread and must run off the UI thread
(spawn, or `day::task`). On Android the calling thread is attached to the JVM via
`day_android::with_env`; class resolution works from any Rust-spawned thread because day-android's
`dfind`/`dcall_static` fall back to the app `ClassLoader` cached at init (a bare JNI `FindClass`
on a native thread sees only the system loader). `fetch_async`/`fetch_to_file_async` are
fire-and-forget wrappers that deliver on a background thread; see the Setter idiom above.

Which thread that is differs by tier. Apple, Android, Windows and HarmonyOS are natively
asynchronous: no Rust thread exists behind `fetch_async` or `fetch_future`. URLSession completes
on its delegate queue; on Android the bridge arm hands the call to OkHttp's own dispatcher and
its `Callback` completes the bridge token from there; on Windows a `WINHTTP_FLAG_ASYNC` session
drives the request on WinHTTP's threads and its status callback delivers; on HarmonyOS the
ArkTS arm runs the Network Kit's promise on the JS thread and settles the token (all 2026-09;
before that a Rust thread per request parked inside the synchronous arm). Only the Rust
fallback on desktop Linux still spawns a worker thread per asynchronous request.

On HarmonyOS the blocking forms wait for that same completion, so they answer `Unsupported`
when called on the JS thread — Day's UI thread there — where waiting would starve the loop that
delivers it; and `fetch_streamed` delivers the whole body as one chunk, since the kit's
streaming form is not bridged yet.

On the web there is exactly one thread, and it must never wait: the blocking calls return
`Unsupported` there, and `fetch_async`'s completion arrives on that sole (UI) thread from the
browser event loop. Both delivery idioms work unchanged: a captured `Setter` detects it is
already on the UI thread, and `fetch_future` under `day::task` resumes there anyway.

## Async and cancellation

```rust
// Await-style (docs/async.md): starts immediately; resumes on the UI thread under day::task,
// so the readout is a plain signal write — no Setter.
day::task(async move {
    match day_part_http::fetch_future(req).await {
        Ok(resp) => status.set(format!("{} · {} bytes", resp.status, resp.body.len())),
        Err(e) => status.set(format!("error: {e}")),
    }
});
```

`fetch_future(req)` is oneshot plumbing over `fetch_async`'s completion: any executor can await
it (`day::task`, or a test's ~25-line `block_on` in tests/http.rs). **Dropping the future cancels
the request** where the platform can:

| tier | drop-cancel |
|---|---|
| Apple | native — `NSURLSessionTask.cancel()`; a completion that beats the observer maps `NSURLErrorCancelled` → `HttpError::Cancelled` |
| Android | native — OkHttp `Call.cancel()` through the bridge's `cancel_native(token)`, keyed by the `Done` token the arm registered BEFORE `enqueue` (sentinel −7 → `Cancelled`). No registration race remains: the token exists before the call is started, so a drop at any moment finds it |
| Web | native — the shim's per-request `AbortController.abort()` (`day_dom_http_abort`), rejecting the in-flight fetch (or its body read) with `AbortError` → `Cancelled` |
| Windows | native — `WinHttpCloseHandle` on the request from the dropping thread, WinHTTP's documented cancellation; the status callback then reports `ERROR_WINHTTP_OPERATION_CANCELLED` (12017) or closes straight to `HANDLE_CLOSING`, either of which delivers `Cancelled` exactly once |
| HarmonyOS | native — `HttpRequest.destroy()` through the bridge's `cancel_native(token)`; the promise then rejects and the arm answers with the cancelled sentinel |
| Rust fallback | discard-only — the request runs out on its worker thread under its `timeout` and the result is dropped |

Aborting a `day::task` that awaits a `fetch_future` (or superseding a `day::reactive::Resource`
fetch) drops the future and takes the same path. The showcase's URL checker aborts its previous
in-flight check on re-tap, a live demo of drop-cancel.

## The Android engine (OkHttp)

The Android half moved from `java.net.HttpURLConnection` to OkHttp 4.12 (2026-07). AOSP's own
`HttpURLConnection` has been a frozen OkHttp fork since Android 4.4, so this upgrade stays in the
same lineage: the system `ProxySelector`, VPN routing, network security config (OkHttp checks
`NetworkSecurityPolicy` for cleartext), and the platform `TrustManager`/user CA store all still
apply. The engine adds HTTP/2 (over TLS via ALPN), PATCH (the classic `HttpURLConnection` gap;
`Request::patch` now works on every platform), and thread-safe per-call cancellation. The costs
and behavior changes are that the okhttp + okio + kotlin-stdlib Gradle dependencies add roughly
1.5–2.5 MB pre-R8 (well under 1 MB after shrinking; OkHttp ships its own proguard rules), that
cross-protocol redirects (https→http) are now followed, matching the other platforms, and that
response headers now arrive in arrival order with duplicates preserved, where the old
`Map`-shaped API merged them. The
coordinate rides the part's own `[package.metadata.day.android] gradle-dependencies`, the
day-piece-lottie mechanism ([daybrite/day-piece-lottie](https://github.com/daybrite/day-piece-lottie)).

## v2 notes (out of scope)

Cookies, multipart, upload streaming, websockets, `no_redirect` (needs an Apple session delegate
to honor), cancellation for `fetch_to_file`/`fetch_streamed` futures (today `StreamSink`
cancels mid-body and covers the download cases), and a native HarmonyOS half via a
framework-owned ArkTS `registerHttp` bridge (the `registerOpenUrl` pattern) if the Remote
Communication Kit's C API reaches the OSS SDK.

## What it shows about the extension system

Like `day-part-network`, it is a headless part: `cfg(target_os)` halves behind one `mod imp`,
per-target dependencies, and part-owned Java staged via `[package.metadata.day.android]` (which
also contributes `android.permission.INTERNET`), with no framework changes. It is the first part
with an async surface and background completion threads (the shape DESIGN §4.5 blesses) and the
first whose Java runs on Rust-spawned threads, which motivated the app-ClassLoader fallback in
day-android's
`DayEnv` helpers. The web arm rides the day-part-prefs precedent (part-declared `extern "C"`
imports the day-dom shim implements), extended with the shim's request-id callback pattern for
its async completions; it is the first part to complete back into wasm.
