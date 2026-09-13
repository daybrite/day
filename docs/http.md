---
title: "HTTP"
description: "HTTP through each platform's network stack via day-part-http: streaming, uploads, redirects, authentication, trust, cookies, caching and WebSockets."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# HTTP through the platform stack (headless capability crate)

> **Status: implemented** as `day-part-http` (in `parts/`), a headless crate with no UI piece.
> Requests run through each platform's own networking stack: URLSession on macOS and iOS, OkHttp
> on Android, WinHTTP on Windows, the system's libcurl on Linux, the Network Kit on HarmonyOS, and
> the browser's `fetch` and `WebSocket` on the web. The crate's tests run against its local test
> server over URLSession and libcurl on macOS, and the showcase's Network & HTTP walkthrough
> (`dayscript/network.yaml`) runs every client feature on macOS, iOS and Android.

The OS already knows things an app cannot easily discover: system proxies and PAC scripts,
per-network VPN routing, Low Data Mode, and enterprise and user-installed certificate stores.
Requests sent through the platform stack inherit all of it, and the app ships no TLS library of
its own.

The crate has two entry points. The functions at the crate root send one request with no state
kept between requests. A [`Client`](#the-client) carries policy across requests and adds the
rest of a modern HTTP API: streamed bodies, uploads, redirect and challenge callbacks, trust
decisions, cookies, caching, metrics and WebSockets.

## Sending a request

```rust
use day_part_http::{Request, fetch};

// Blocking: call it off the UI thread.
let resp = fetch(&Request::get("https://api.example.com/data.json"))?;
if (200..300).contains(&resp.status) {
    let body: MyData = serde_json::from_slice(&resp.body)?;
}
```

`Request` is a builder: `get`, `post`, `put`, `delete`, `patch` and `head` start one, then
`.header(name, value)` (duplicates allowed), `.bearer(token)`, `.basic_auth(user, password)`,
`.body(bytes)`, `.timeout(d)`, `.timeout_total(d)`, `.allow_expensive(bool)`,
`.allow_constrained(bool)`, `.cache(policy)` and `.priority(p)`. The upload builders are under
[Uploads](#uploads). `Response { status, headers, body }` adds `text()` (lossy UTF-8) and a
case-insensitive `header(name)`.

Two contract points apply everywhere:

- **4xx and 5xx are responses.** An HTTP error status arrives as `resp.status == 404`. An
  `HttpError` is a transport failure: `BadUrl`, `Timeout`, `Dns`, `Connect`, `Tls`, `Io`,
  `Cancelled`, `TooManyRedirects`, `Status` or `Unsupported` (the enum is `#[non_exhaustive]`).
- **`timeout` bounds progress.** It covers connecting, waiting for the response head, and idle
  gaps in the body, so a long download that keeps moving runs to the end. The default is 30 s.
  `timeout_total` bounds the whole request, redirects and challenges included.

The root functions are:

| Function | Shape |
|---|---|
| `fetch(&req)` | blocks; returns `Response` |
| `fetch_async(req, on_done)` | `on_done` runs on a background thread |
| `fetch_future(req)` | a future; dropping it cancels the request |
| `fetch_to_file(&req, &dest)`, `fetch_to_file_async` | writes the body to `dest` as it arrives |
| `fetch_streamed(&req, &mut sink)` | hands the head and each chunk to a `StreamSink`, which can stop the transfer |

They share one client that keeps no cookies and no cache, so every call starts fresh.

### Async and the Setter idiom

```rust
// Await-style (docs/async.md): under day::task the continuation runs on the UI thread, so the
// readout is a plain signal write.
day::task(async move {
    match day_part_http::fetch_future(req).await {
        Ok(resp) => status.set(format!("{} · {} bytes", resp.status, resp.body.len())),
        Err(e) => status.set(format!("error: {e}")),
    }
});

// Callback-style: the completion runs on the transport's thread, so hand results to the UI
// through a Setter, which hops threads itself and ignores late deliveries.
let done = status.setter();
day_part_http::fetch_async(Request::get(url), move |result| {
    if let Ok(resp) = result {
        done.set(resp.text().into_owned());
    }
});
```

The crate never calls `day_reactive::on_main` itself, which keeps it usable from plain `main`
programs and `cargo test`. Aborting a `day::task` drops its future, which cancels the request.

## The client

```rust
use std::time::Duration;
use day_part_http::{Client, Redirects, Request};

let client = Client::builder()
    .timeout_idle(Duration::from_secs(20))
    .timeout_total(Duration::from_secs(120))
    .redirects(Redirects::Follow(5))
    .user_agent("Skies/1.0")
    .on_challenge(|challenge, reply| {
        if challenge.host == "api.example.com" {
            reply.credential("day", "sunrise");
        }
    })
    .build();

day::task(async move {
    match client.fetch_future(Request::get("https://api.example.com/forecast")).await {
        Ok(resp) => status.set(format!("{} · {} bytes", resp.status, resp.body.len())),
        Err(e) => status.set(format!("error: {e}")),
    }
});
```

A `Client` is cheap to clone, and clones share its policy, cookie store and platform session.
Everything asynchronous has a callback form (`*_async`) and a future form (`*_future`). Callbacks
run on the thread the platform reports on, and dropping a future, a `Body` or a `WebSocket`
cancels what it stands for.

| Builder option | Default |
|---|---|
| `timeout_idle(d)` | 30 s, for requests that set no `timeout` |
| `timeout_total(d)` | none |
| `redirects(Redirects::Follow(n) \| Redirects::Never)` | `Follow(10)` |
| `on_redirect`, `on_challenge`, `on_server_trust` | none |
| `trust(Trust)`, `identity(Identity)` | the platform's evaluation, no identity |
| `cookies(Cookies)` | the platform store on Apple and the web, an in-memory jar elsewhere |
| `cache(Cache)` | the platform cache at 8 MiB in memory and 64 MiB on disk |
| `header(name, value)`, `user_agent(agent)` | none |
| `max_per_host(n)` | the platform's |
| `wait_for_connectivity(bool)` | `false` |
| `question_timeout(d)` | two minutes |
| `transport(t)` | the platform's transport |

A handler receives a reply object (`RedirectReply`, `ChallengeReply`, `TrustReply`). It may answer
at once, or keep the reply and answer later from any thread while the exchange waits, up to the
question timeout. Dropping a reply unanswered takes its default.

`client.capabilities()` (or the root `capabilities()`) reports what the platform's transport
offers, and the [capability table](#capabilities) lists them per platform. An option the
platform cannot honor fails the request with `Unsupported`, so a pin or a redirect handler never
silently does nothing.

## Streaming responses

```rust
day::task(async move {
    let streaming = match client.send_future(Request::get(url)).await {
        Ok(streaming) => streaming,
        Err(e) => return status.set(format!("error: {e}")),
    };
    let total = streaming.expected_length();
    let mut body = streaming.into_body();
    let mut received = 0;
    while let Some(chunk) = body.next().await {
        match chunk {
            Ok(bytes) => {
                received += bytes.len() as u64;
                progress.set((received, total));
            }
            Err(e) => return status.set(format!("error: {e}")),
        }
    }
});
```

`send_future` resolves with the head: `status()`, `headers()`, `header(name)`, the final `url()`
and `expected_length()`. The body is then pulled chunk by chunk as the platform delivers it:

- **Backpressure.** A body holds at most four chunks, queued or granted, before it stops asking
  the transport for more, so a slow reader slows the socket. `pause()` and `resume()` stop and
  restart the flow explicitly.
- **Other ways to read.** `next_blocking()` waits on the calling thread, `read_async(callback)`
  delivers each chunk to a callback that returns whether it wants the next, and
  `streaming.collect_future()` gathers the whole body into a `Response`.
- **Cancellation.** Dropping the `Body` cancels the transfer. `body.in_flight()` returns an
  `InFlight` handle that cancels it from anywhere, which is what [day-part-downloads](downloads.md)
  keeps for each download.
- **Progress.** `received()` counts bytes so far, and `metrics()` reports the finished transfer.

## Uploads

| Builder | Body |
|---|---|
| `.body(bytes)` | bytes in memory |
| `.body_file(path)` | a file, read as it is sent |
| `.body_stream(reader, len)` | any `Read + Send`, with its length when known |
| `.form(Form)` | `multipart/form-data`, with text fields, in-memory files and files on disk |

```rust
use day_part_http::{Form, Request};

let form = Form::new()
    .text("title", "Sunrise")
    .file("photo", &photo_path, "image/jpeg");
let progress = sent.setter();
let request = Request::post(url, Vec::new())
    .form(form)
    .upload_progress(move |bytes, total| progress.set((bytes, total)));
```

`upload_progress` reports bytes sent and the total where the length is known. A stream of
unknown length goes out with chunked transfer encoding. Where the platform cannot send a body
from a reader (`upload_streaming` is false: HarmonyOS and the web), file and stream bodies are
read into memory first.

## Redirects

The client follows redirects itself on every platform that hands redirect responses back
(`manual_redirects`), so the rules are the same everywhere:

- A `303` turns any method but `HEAD` into `GET`, and a `301` or `302` turns a `POST` into a `GET`,
  dropping the body.
- `Authorization`, `Proxy-Authorization` and `Cookie` headers set on the request are dropped when a
  redirect leaves the origin. Cookies from the client's store are chosen again for the new URL.
- `Redirects::Follow(n)` fails with `TooManyRedirects` after `n` hops; `Redirects::Never`
  delivers the redirect response itself.

```rust
let client = Client::builder()
    .on_redirect(|hop, reply| {
        if hop.to.starts_with("https://") {
            reply.follow();
        } else {
            reply.stop();
        }
    })
    .build();
```

`Hop` carries the `status`, `from` and `to` URLs, the `method` the next request uses, and how many
redirects were `followed` before it. `stop()` makes the redirect response the response, and
`cancel()` fails the request with `Cancelled`. HarmonyOS and the web follow redirects inside the
platform, so there `on_redirect` fails requests with `Unsupported`, while `Redirects::Follow` and
`Redirects::Never` still apply to what the platform returns.

## Authentication

A `401` or `407` without a handler is delivered as the response. With `on_challenge`, the client
parses the `WWW-Authenticate` (or `Proxy-Authenticate`) header, asks the handler, and retries:

- `reply.credential(user, password)` answers Basic or Digest (MD5 or SHA-256, `qop=auth`), and
  the client writes whichever the server offered, preferring Digest.
- `reply.bearer(token)` sends `Authorization: Bearer`.
- `reply.default_handling()` delivers the challenge response; `reply.cancel()` fails the request.

`Challenge` names the `url`, `host`, `port`, `realm`, `scheme`, whether a `proxy` asked, and the
`previous_failures` for this request. After three refused answers the `401` is delivered.
Credentials that succeed are reused for later requests to the same origin and realm without
asking again.

URLSession raises Basic, Digest, NTLM and Negotiate challenges itself (`auth_questions`,
`native_auth_schemes`), and the client routes them to the same handler, so NTLM and Negotiate
work on Apple. `Request::basic_auth` and `Request::bearer` set the header up front with no
challenge round trip.

## Server trust and client certificates

```rust
use day_part_http::{Client, Identity, Trust};

let client = Client::builder()
    .trust(Trust::system().pin("api.example.com", "sha256/Y9mvm0exBk1JoQ57f9Vm28jKo5lFm/woKcVxrYxu80o="))
    .identity(Identity::pkcs12(p12_bytes, "password"))
    .on_server_trust(|trust, reply| {
        if trust.system_trusted {
            reply.default_handling();
        } else {
            reply.reject();
        }
    })
    .build();
```

- **Pins.** A pin is the SHA-256 of a certificate's public key, written `sha256/<base64>`. A host
  (or `*.example.com`) with pins must present a chain holding one of them, on top of the
  platform's own evaluation; several pins for one host accept any, which is how a key rotation
  ships. `ServerTrust::pins()` prints the pins of a chain the platform reported. Pins need the
  platform to report the chain (`server_trust`). Where it cannot, a pinned request fails with
  `Unsupported` rather than connecting unpinned, and so does a WebSocket to a pinned host.
- **Trust decisions.** `on_server_trust` sees the host, the DER chain, the platform's verdict
  and its error. `accept()` trusts a server the platform refused (a self-signed development
  server), `reject()` fails with `Tls`, and `default_handling()` keeps the verdict.
- **Client certificates.** `Identity::pkcs12(der, password)` is presented when a server asks for
  one, where `client_identity` is true.

## Cookies

`Cookies` chooses where a client keeps them:

| Variant | Store |
|---|---|
| `Cookies::Platform` | `HTTPCookieStorage` on Apple and the browser's jar on the web; an in-memory jar where the platform offers none to share |
| `Cookies::jar()` | an in-memory jar the client reads and writes |
| `Cookies::jar_at(file)` | the same jar, saved to `file`, so cookies with an expiry survive a relaunch |
| `Cookies::Off` | none sent, none kept |

The jar follows RFC 6265: domain and path matching, `Secure`, `HttpOnly`, `Max-Age` and
`Expires`, and the public suffix list, which keeps a site from setting a cookie for
a whole registry such as `co.uk`. Cookies set on a redirect response apply to the next hop.
`client.cookies()` lists the store and `client.clear_cookies()` empties it. A `CookieJar` also
works on its own, through `store(url, set_cookie_values)` and `header(url)`.

## Caching

`Cache::platform()` (the default) or `Cache::Platform { memory, disk }` uses the platform's HTTP
cache where it has one (`platform_cache`), and `Cache::Off` disables it. A request chooses how to
use it with `Request::cache`:

| `CachePolicy` | Behavior |
|---|---|
| `Default` | the response's own cache headers decide |
| `Reload` | always from the network |
| `PreferCache` | cached data when present, however old |

`Metrics::from_cache` says whether a response came from the cache, and `client.clear_cache()`
empties it. The root functions use no cache.

## Metrics

`body.metrics()` reports the finished transfer where the platform measures it (`metrics`): DNS,
connect, TLS, time to first byte and total durations, the protocol (`http/1.1`, `h2`, `h3`),
whether the connection was reused, the remote address, the TLS version, whether the response came
from the cache, bytes sent and received, and the redirects followed. A field the platform does not
report is `None`.

## WebSockets

```rust
use day_part_http::{Message, Request};

day::task(async move {
    let request = Request::get("wss://chat.example.com/socket").protocols(["chat"]);
    let mut socket = match client.websocket_future(request).await {
        Ok(socket) => socket,
        Err(e) => return state.set(format!("error: {e}")),
    };
    socket.sender().send_async(Message::Text("hello".into()), |_| {});
    while let Some(message) = socket.next().await {
        match message {
            Ok(Message::Text(text)) => last.set(text),
            Ok(Message::Binary(_)) => {}
            Ok(Message::Close { code, reason }) => state.set(format!("closed · {code} {reason}")),
            Err(e) => return state.set(format!("error: {e}")),
        }
    }
});
```

`websocket_future` resolves once the handshake completes, and `socket.protocol()` names the
subprotocol the server chose. `socket.sender()` returns a `WsSender` that any thread may keep, with
`send_async`/`send_future`, `ping_async`/`ping_future` where `websocket_ping` is true, and
`close(code, reason)`. Messages arrive in order through `next()` or `read_async`, with the same
four-message backpressure window as a body. The last message is `Close`. The handshake carries the
client's headers and cookies where `websocket_headers` is true. Dropping the `WebSocket` closes it.

## Capabilities

| Capability | macOS, iOS | Android | Linux | Windows | HarmonyOS | Web |
|---|---|---|---|---|---|---|
| `streaming` | yes | yes | yes | yes | yes | yes |
| `upload_streaming` | yes | yes | yes | yes | no | no |
| `upload_progress` | yes | yes | yes | yes | yes | no |
| `manual_redirects` | yes | yes | yes | yes | no | no |
| `auth_questions` | yes | no | no | no | no | no |
| `native_auth_schemes` | yes | no | no | no | no | no |
| `server_trust` | yes | yes | no | no | no | no |
| `client_identity` | yes | yes | libcurl 7.71.0 and later | yes | no | no |
| `platform_cookies` | yes | no | no | no | no | yes |
| `platform_cache` | yes | yes | no | no | yes | yes |
| `metrics` | yes | yes | yes | yes | no | no |
| `websockets` | yes | yes | libcurl 8.11.0 and later, built with `ws` | yes | yes | yes |
| `websocket_ping` | yes | no | no | no | no | no |
| `websocket_headers` | yes | yes | with `websockets` | yes | yes | no |
| `wait_for_connectivity` | yes | no | no | no | no | no |

The client supplies what a transport lacks where it can: redirects, challenge answers, the cookie
jar and total-time limits run in Rust on every platform. Server trust questions and client
certificates need the stack's own TLS handshake, so they follow the table. On HarmonyOS the
Network Kit delivers a body as fast as it arrives, with no way to hold it back, so unread chunks
queue in the client.

## Transports

Each platform implements the crate's `Transport` trait, which performs one exchange at a time and
reports it as events: the head, body chunks, upload progress, trust or authentication questions,
metrics, then the end or a failure. The client asks for body chunks with `demand`, answers
questions with `answer`, and stops an exchange with `cancel`.

| Platform | Transport |
|---|---|
| macOS, iOS | one `URLSession` per client with a delegate (`src/apple.rs`): the delegate declines redirects so the client decides, raises challenges and server trust as questions, suspends a task while its body has no demand, and sends file and stream uploads from the file or a bound stream pair |
| Android | OkHttp through a Java bridge arm (`src/bridge.rs`), with redirects and cookies left to the client; a reader pool reads each body only as far as demand allows, and an `X509ExtendedTrustManager` raises server trust questions |
| Linux | the system's libcurl, opened with `dlopen` at run time (`libcurl.so.4`, then `libcurl-gnutls.so.4`) (`src/linux.rs`): one driver thread runs a multi handle, pauses a transfer's writes when demand runs out, and opens WebSockets in libcurl's connect-only mode |
| Windows | WinHTTP in asynchronous mode (`src/windows.rs`): automatic proxy detection, redirects and cookies turned off in WinHTTP so the client handles them, reads issued only against demand, uploads written as the body is read, and WebSockets through `WinHttpWebSocket*` |
| HarmonyOS | the Network Kit (`@ohos.net.http` `requestInStream`, `@ohos.net.webSocket`) through an ArkTS bridge arm |
| Web | `fetch` with a `ReadableStream` reader that pulls under demand, an `AbortController` for cancellation and the idle bound, and the browser's `WebSocket`, through a JavaScript bridge arm |

Android, HarmonyOS and the web share one Rust transport, `src/bridged.rs`, over the bridge's
stream tier ([docs/bridge.md](bridge.md) "Streams"). Each exchange is one `Emit<Vec<u8>>` stream
of frames, and every frame starts with a tag byte and the stream's token, big-endian:

| Tag | Frame | Fields |
|---|---|---|
| 1 | head | status, final URL, header block, expected length (−1 when unknown) |
| 2 | chunk | body bytes |
| 3 | sent | bytes sent, total |
| 4 | question | question id, kind, host, platform verdict, error, DER chain |
| 5 | metrics | durations in microseconds, protocol, reuse, remote address, TLS version, cache flag, byte counts |
| 6 | end | |
| 7 | failed | sentinel, message |
| 8 | need body | bytes wanted for a streamed upload |
| 16–20 | WebSocket open, text, binary, closed, failed | protocol; message; bytes; code and reason; sentinel and message |

Strings are a 32-bit length and UTF-8, and a header block is `name\nvalue\n` repeated. The
sentinels are −1 timeout, −2 DNS, −3 TLS, −4 connect, −5 I/O, −6 bad URL and −7 cancelled. Plain
calls go the other way: `demand_native`, `answer_native`, `body_native` (an empty chunk ends an
upload), `exchange_cancel_native`, `ws_send_native` and `ws_close_native`.

`ClientBuilder::transport` swaps in any other `Transport`: a scripted double in a unit test, or an
app's own stack.

`tier()` reports `NativeStack` wherever a transport is present. It reports `Unavailable` on Linux
when no libcurl loads, on a HarmonyOS build made with bare cargo instead of `day build` (which
stages the ArkTS arm), and on any other target; there every call fails with `Unsupported`.

## Threads and blocking calls

`fetch`, `fetch_to_file`, `fetch_streamed` and `Body::next_blocking` wait on the calling thread,
so keep them off the UI thread. Callbacks and wakeups come from the transport's own thread:
URLSession's delegate queue, OkHttp's reader pool, the libcurl driver thread, WinHTTP's callback
threads, the HarmonyOS JS thread, or the browser's only thread. The one thread the crate starts
for a request is the writer that feeds a streamed upload into URLSession's bound stream pair.

Two platforms restrict the blocking calls:

- **The web** has one thread, and waiting there would starve the event loop the answer needs
  ([docs/web.md](web.md)), so the blocking calls return `Unsupported` and the `*_async` and
  `*_future` forms work in full. CORS governs cross-origin requests and which response headers
  are visible, and the browser controls headers such as `Host`, `Cookie` and `Origin`.
- **HarmonyOS** runs the arm on the JS thread, which is Day's UI thread there. The blocking calls
  return `Unsupported` when called on that thread and work from any other.

## Error mapping

| `HttpError` | Apple (`NSURLErrorDomain`) | Android (OkHttp) | Linux (`CURLE_*`) | Windows (`ERROR_WINHTTP_*`) | Web |
|---|---|---|---|---|---|
| `Timeout` | −1001 | `SocketTimeoutException` | `OPERATION_TIMEDOUT` | 12002 | the idle timer's abort |
| `Dns` | −1003, −1006 | `UnknownHostException` | `COULDNT_RESOLVE_HOST`, `COULDNT_RESOLVE_PROXY` | 12007 | |
| `Connect` | −1004, −1009 | `ConnectException` | `COULDNT_CONNECT` | 12029, 12030 | |
| `Tls(msg)` | −1200 to −1206 | `SSLException` | the certificate and handshake codes | 12157, 12175 and the other secure failures | |
| `BadUrl` | −1000, −1002 | a URL OkHttp rejects | `URL_MALFORMAT` | 12005, 12006 | `new URL` rejects it |
| `Cancelled` | −999 | a cancelled call | an aborted transfer | 12017 | an `AbortError` from cancellation |
| `Io(msg)` | anything else | anything else | anything else | anything else | anything else |

Browsers report DNS, connection, TLS and CORS failures as one `TypeError` without detail, so on
the web every network failure is `Io` with the browser's message.

## App Transport Security and Android cleartext

Both mobile platforms restrict plain `http://`, and the platform stack enforces the app's policy:

- **App Transport Security** (Apple) refuses non-HTTPS URLs unless the app's `Info.plist` carries
  an exception. Requests to IP addresses such as `http://127.0.0.1:…` are exempt, so the
  showcase's local test server needs no plist change. For a real cleartext host, add a scoped
  `NSExceptionDomains` entry rather than `NSAllowsArbitraryLoads`.
- **Android cleartext** is blocked app-wide since targetSdk 28, loopback included. The scaffold
  ships a `network_security_config.xml` that permits cleartext to `127.0.0.1` only, and a real
  exception belongs in the same file.

## Testing

`day_part_http::testing::Server` is a local HTTP/1.1 and WebSocket server for tests and
demonstrations, on every target but the web. It binds a loopback port and answers each
connection on its own thread:

```rust
use day_part_http::{Request, fetch, testing::Server};

#[test]
fn follows_a_redirect_chain() {
    let server = Server::start().expect("test server");
    let resp = fetch(&Request::get(server.url("/redirect/3"))).expect("fetch");
    assert_eq!(resp.text(), "redirected");
}
```

Its routes cover the whole client: redirect chains, Basic, Digest and Bearer challenges, cookies
set and deleted across redirects, `max-age` responses that count real hits, drip-fed chunked
bodies, deterministic bytes with `Range`, `If-Range` and a rate limit, SHA-256 digests of those
bytes, an upload digest, and a WebSocket echo. The module's documentation lists each route. The
showcase's Network & HTTP page runs one inside the app on the platforms that allow a listening
socket.

## The Android engine

The Android transport runs on OkHttp 4.12, the engine AOSP's own `HttpURLConnection` has been a
fork of since Android 4.4. The system `ProxySelector`, VPN routing, the network security config
(OkHttp checks `NetworkSecurityPolicy` for cleartext), and the platform trust manager with the
user CA store all apply. OkHttp adds HTTP/2 over TLS and per-call cancellation. The okhttp, okio
and kotlin-stdlib Gradle dependencies add roughly 1.5 to 2.5 MB before R8, and well under 1 MB
after shrinking. The coordinate rides the part's own `[package.metadata.day.android]
gradle-dependencies`, the mechanism [day-piece-lottie](https://github.com/daybrite/day-piece-lottie)
also uses.

## Deferred

- Redirect approval, server trust questions and client certificates on HarmonyOS and the web,
  whose platform APIs follow redirects and evaluate trust without asking.
- Server trust questions on Linux and Windows.
- Uploads the OS runs while the app is away. Downloads have that tier in
  [day-part-downloads](downloads.md).

## What it shows about the extension system

This is a headless part: `cfg(target_os)` backends behind one `mod imp`, per-target dependencies,
and bridge arms in Java, ArkTS and JavaScript declared beside the Rust that reads their frames.
Its Android arm contributes `android.permission.INTERNET` and the OkHttp coordinate through
`[package.metadata.day.android]`. It was the first part on the bridge's stream tier, and
[day-part-downloads](downloads.md) builds on its client without platform code of its own for the
in-app tier.
