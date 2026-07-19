# HTTP through the platform stack (headless capability crate)

> **Status: implemented** as `day-part-http` (in `parts/`), a headless day-ecosystem crate with no
> UI Piece: request/response HTTP (plus streaming downloads) through each platform's own networking
> stack — NSURLSession on macOS/iOS, `HttpURLConnection` on Android, WinHTTP on Windows — with a
> bundled ureq + rustls fallback on Linux and HarmonyOS. Verified end-to-end with a local-server
> test suite on the real Apple half (macOS) and the real fallback half (Linux), and live on
> macOS/iOS-sim/Android-emulator via the showcase walkthrough and Day Skies' Open-Meteo fetch.

Why the platform stack instead of a Rust HTTP crate: the OS already knows the things an app can't
easily discover — system proxies and PAC scripts, per-network VPN routing, Low Data Mode,
enterprise/MDM certificate stores, user-installed CAs. Apps that fetch through the platform inherit
all of it, and the native targets bundle **no TLS code at all** (rustls compiles only into the
cfg-gated Linux/OHOS fallback).

## Authoring

```rust
use day_part_http::{Request, fetch};

// Blocking — call it off the UI thread (a worker thread, or day::task's pool).
let resp = fetch(&Request::get("https://api.example.com/data.json"))?;
if (200..300).contains(&resp.status) {
    let body: MyData = serde_json::from_slice(&resp.body)?;
}
```

`Request` is a builder: `get/post/put/delete/patch/head(url)`, `.header(k, v)` (duplicates
allowed), `.body(Vec<u8>)`, `.timeout(Duration)`, `.allow_expensive(bool)` /
`.allow_constrained(bool)`. `Response { status, headers, body }` adds `text()` (lossy UTF-8) and a
case-insensitive `header(name)`.

Two contract points that differ from ureq-style clients:

- **4xx/5xx are `Ok`.** An HTTP error status is a *response* (`resp.status == 404`), not an
  `HttpError`. Errors are transport-level only: `BadUrl`, `Timeout`, `Dns`, `Connect`, `Tls`,
  `Io`, `Unsupported` (the enum is `#[non_exhaustive]`).
- **`timeout` bounds progress, not the transfer.** It covers connecting, awaiting the response
  head, and idle gaps — a multi-minute download that keeps moving is never cut off. Default 30 s.

### Async + the Setter idiom

```rust
let status: Signal<String> = Signal::new(String::new());
let done = status.setter(); // Copy + Send; hops to the UI thread itself
day_part_http::fetch_async(Request::get(url), move |result| {
    // Runs on an UNSPECIFIED BACKGROUND thread (URLSession's delegate queue on Apple,
    // a spawned thread elsewhere). Never touch UI state directly here.
    if let Ok(resp) = result {
        done.set(resp.text()); // no-ops harmlessly if the page was disposed meanwhile
    }
});
```

`fetch_async(req, on_done)` completes on a background thread by design: the crate never calls
`day_reactive::on_main` (which requires an installed backend poster and would break plain-`main`
programs and `cargo test`). Capturing a `Setter` in `on_done` is the standard delivery idiom
(DESIGN §4.5) — it marshals to the UI thread itself and absorbs late deliveries after disposal.
The showcase's Platform services page demonstrates it twice: a deterministic loopback fetch, and
a URL checker (type any http(s) URL, tap Check) that prints the response headers and body size —
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
| Android | `HttpURLConnection` via the part-owned `DayHttp.java` shim; one `byte[]` envelope per call | `day-android` + `[package.metadata.day.android]` |
| Windows | WinHTTP (winhttp.dll, resolved dynamically; `WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY`) | raw FFI (runtime lookup) |
| Linux | ureq 3 + rustls (the only tier that bundles TLS) | ureq, `fallback.rs` |
| HarmonyOS | ureq 3 + rustls — the OSS 5.1 NDK has no HTTP C API (`HMS_Rcp_*` is HarmonyOS-NEXT-SDK-only) | ureq, same `fallback.rs` |
| unknown/mock | catch-all: every call returns `HttpError::Unsupported` | — |

`tier()` reports which of the three tiers the compiled target uses — `NativeStack`,
`RustFallback`, or `Unavailable` — so an app (or a doc table) never has to guess:

- **NativeStack**: system proxy + PAC, VPN routing, platform TLS + certificate stores all apply.
- **RustFallback**: correct HTTP(S) via rustls + webpki roots, but system awareness is limited to
  the `http_proxy`/`https_proxy`/`no_proxy` environment variables (no PAC, no desktop proxy
  settings).
- **Unavailable**: every call fails with `Unsupported` (the mock/unknown-target posture).

## Error mapping

| `HttpError` | Apple (`NSURLErrorDomain`) | Android (exception) | Windows (`ERROR_WINHTTP_*`) | fallback (ureq) |
|---|---|---|---|---|
| `Timeout` | −1001 | `SocketTimeoutException` | 12002 | `Timeout` |
| `Dns` | −1003, −1006 | `UnknownHostException` | 12007 | `HostNotFound` |
| `Connect` | −1004, −1009 | `ConnectException` | 12029, 12030 | `ConnectionFailed` |
| `Tls(msg)` | −1200…−1206 | `SSLException` | secure-failure set (12157, 12175, …) | `Tls` |
| `BadUrl` | −1000, −1002 | `MalformedURLException` | 12005, 12006 | `BadUri` |
| `Io(msg)` | anything else | anything else | anything else | anything else |

## Option honesty

Options that only some platforms can realize are documented, not silently dropped:

| option | Apple | Android | Windows | fallback |
|---|---|---|---|---|
| `.timeout` | `timeoutInterval` (idle timer) | connect + per-read timeouts | per-operation `WinHttpSetTimeouts` | resolve/connect/send/response-head timeouts (body phase uncapped) |
| `.allow_expensive` / `.allow_constrained` | native (`allowsExpensiveNetworkAccess` / `allowsConstrainedNetworkAccess`, Low Data Mode) | advisory only | advisory only | advisory only |
| redirects | followed (no opt-out in v1) | followed | followed | followed |

## App Transport Security (iOS/macOS) and Android cleartext

Both mobile platforms restrict plain `http://` by default; the platform stack enforces the
platform's policy — which is a feature, but needs two notes:

- **ATS** (Apple): `NSURLSession` refuses non-HTTPS URLs unless the app's Info.plist carries an
  exception (`NSAppTransportSecurity`). Loopback IP fetches (`http://127.0.0.1:…`) are exempt —
  the showcase's local demo needs no plist changes. For a real cleartext host, add a scoped
  `NSExceptionDomains` entry; don't reach for `NSAllowsArbitraryLoads`.
- **Android cleartext**: blocked app-wide since targetSdk 28 — including loopback. The showcase
  scaffold ships a `network_security_config.xml` permitting cleartext to `127.0.0.1` only (plus
  the `android:networkSecurityConfig` manifest attribute); scope any real exception the same way.

The fallback tier performs no such policy enforcement (ureq happily fetches `http://`), another
reason `tier()` exists.

## Threading

`fetch`/`fetch_to_file`/`fetch_streamed` block the calling thread and MUST run off the UI thread
(spawn, or `day::task`). On Android the calling thread is attached to the JVM via
`day_android::with_env`; class resolution works from any Rust-spawned thread because day-android's
`dfind`/`dcall_static` fall back to the app `ClassLoader` cached at init (a bare JNI `FindClass`
on a native thread sees only the system loader). `fetch_async`/`fetch_to_file_async` are
fire-and-forget wrappers that deliver on a background thread — see the Setter idiom above.

## v2 notes (deliberately out of scope)

Cookies, multipart, upload streaming, websockets, `no_redirect` (needs an Apple session delegate
to honor honestly), a `CancelHandle` for in-flight aborts (today: `timeout` bounds, `Setter`
absorbs abandonment, `StreamSink` cancels mid-body), a `Future` adapter / §4.5 `Resource`
integration, and a native HarmonyOS half via a framework-owned ArkTS `registerHttp` bridge (the
`registerOpenUrl` pattern) if the Remote Communication Kit's C API reaches the OSS SDK.

## What it shows about the extension system

Like `day-part-network`, a headless part: `cfg(target_os)` halves behind one `mod imp`, per-target
dependencies, part-owned Java staged via `[package.metadata.day.android]` (which also contributes
`android.permission.INTERNET`), no framework changes. It is the first part with an async surface
and background completion threads — the shape DESIGN §4.5 blesses — and the first whose Java runs
on Rust-spawned threads, which is what motivated the app-ClassLoader fallback in day-android's
`DayEnv` helpers.
