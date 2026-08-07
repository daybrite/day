---
title: Call an HTTP API
description: "Fetch over each platform's own networking stack with day-part-http, land the result in signals, and cancel in-flight requests by dropping their future."
order: 30
section: Guides
---

Day apps fetch through each platform's own networking stack: `NSURLSession` on macOS and iOS,
OkHttp on Android, WinHTTP on Windows, the browser's `fetch()` on the web, and a bundled
ureq + rustls client on Linux and HarmonyOS. The request inherits what the OS already knows
(system proxies and PAC scripts, VPN routing, Low Data Mode, certificate stores), and the
native targets bundle no TLS code at all. The whole call site is:

```rust
day::task(async move {
    match day_part_http::fetch_future(Request::get(url)).await {
        Ok(resp) => body.set(resp.text().into_owned()),
        Err(e) => body.set(format!("error: {e}")),
    }
});
```

**Works on:** macOS, iOS, Android, Windows, and web via the platform stack; Linux and
HarmonyOS via the Rust fallback (correct HTTPS, but proxy awareness limited to the
`http_proxy` environment variables); on an unknown target every call returns `Unsupported`.
On web the blocking entry points return `Unsupported`; the async forms work in full.
`day_part_http::tier()` reports the compiled target's tier.

## 1. Fetch into UI state

Add the crate, then hold the request's outcome in signals and let the UI bind them:

```toml
[dependencies]
day-part-http = { git = "https://github.com/daybrite/day.git" }
```

The `match` goes inside the task: `day::task` takes a future with `Output = ()`, so an async
block that returns a `Result` doesn't compile there; handle both arms and write signals.
Under `day::task` the future resumes on the UI thread, so those writes are plain `set` calls:

```rust
use day::prelude::*;
use day_part_http::{Request, fetch_future};

fn forecast_row() -> impl Piece {
    let readout = Signal::new(String::new());
    let loading = Signal::new(false);
    column((
        button("Fetch").action(move || {
            loading.set(true);
            day::task(async move {
                match fetch_future(Request::get("https://api.example.com/data.json")).await {
                    Ok(resp) if (200..300).contains(&resp.status) => {
                        readout.set(resp.text().into_owned());
                    }
                    Ok(resp) => readout.set(format!("HTTP {}", resp.status)),
                    Err(e) => readout.set(format!("error: {e}")),
                }
                loading.set(false);
            });
        }),
        label(move || if loading.get() { "loading…".to_string() } else { readout.get() }),
    ))
    .spacing(8.0)
}
```

Two contract points that differ from ureq-style clients:

- **4xx/5xx are `Ok`.** An HTTP error status is a response (`resp.status == 404`), not an
  `HttpError`, which is why the sample checks the status range. Errors are transport-level
  only: `BadUrl`, `Timeout`, `Dns`, `Connect`, `Tls`, `Io`, `Cancelled`, `Unsupported`.
- **`timeout` bounds progress, not the transfer.** It covers connecting, awaiting the response
  head, and idle gaps; a long download that keeps moving is never cut off. Default 30 s.

`Response` is `{ status, headers, body }` plus `text()` (lossy UTF-8) and a case-insensitive
`header(name)` lookup. There is no built-in JSON layer; parse `resp.body` with `serde_json`,
an ordinary app dependency and what the crate's own docs use:

```rust
#[derive(serde::Deserialize)]
struct Forecast { temperature: f64 }

let data: Result<Forecast, _> = serde_json::from_slice(&resp.body);
```

## 2. Cancel an in-flight request

Tasks are not owned by the scope that spawned them: leaving the page does not stop a running
fetch. To cancel, keep the `TaskHandle` that `day::task` returns and call `.abort()`. Aborting
drops the task's future, and **dropping the `FetchFuture` is what cancels the platform
request**: `NSURLSessionTask.cancel` on Apple, OkHttp `Call.cancel` on Android, the fetch's
`AbortController` on web. Windows and the Rust fallback can't cancel mid-flight; they run the
request out on a worker thread and discard the result. Re-tapping below supersedes the
previous request:

```rust
let inflight: Rc<Cell<Option<day::TaskHandle>>> = Rc::new(Cell::new(None));
button("Check").action(move || {
    if let Some(prev) = inflight.take() {
        prev.abort();                     // drops the FetchFuture → platform cancel
    }
    let slot = inflight.clone();
    let handle = day::task(async move {
        match fetch_future(Request::get(url.get_untracked())).await {
            Ok(resp) => readout.set(format!("HTTP {} · {} bytes", resp.status, resp.body.len())),
            Err(e) => readout.set(format!("error: {e}")),
        }
        slot.set(None);
    });
    inflight.set(Some(handle));
});
```

Aborting a finished task is a no-op, and a write to a signal whose scope has since been
disposed is a silent no-op, so a late completion can't crash a page the user already left.

## 3. Load on mount with Resource

For "fetch when this page appears, refetch when an input changes", `day::reactive::Resource`
is the scope-tied form: a tracked `source` feeds an async `fetcher`, and the result lands in a
`Signal<Load<T>>`. Unlike `day::task`, the fetcher returns a `Result`, and the error becomes
`Load::Failed`:

```rust
use day::reactive::{Load, Resource};

let forecast: Resource<String> = Resource::new(
    move || city.get(),                       // tracked — a city change refetches
    |city| async move {
        let url = format!("https://api.example.com/wx?city={city}");
        let resp = day_part_http::fetch_future(Request::get(url)).await?;
        Ok::<_, day_part_http::HttpError>(resp.text().into_owned())
    },
);
label(move || forecast.with(|l| match l {
    Load::Loading => "…".to_string(),
    Load::Ready(s) => s.clone(),
    Load::Failed(e) => format!("error: {e}"),
}))
```

Latest wins: a source change aborts the in-flight fetch (dropping its `FetchFuture`, the same
cancel rail as above) and a stale completion writes nothing. Scope disposal aborts it too, with no
handle bookkeeping, and `forecast.refetch()` forces a fresh fetch. Watch the import: the
prelude's `Resource` is the bundled-asset handle; the async one is `day::reactive::Resource`.

## 4. Other methods, headers, and bodies

`Request` builds every common shape: `get`/`delete`/`head(url)`, and `post`/`put`/`patch(url,
body)` taking the body bytes up front. `.header(name, value)` appends (duplicates allowed,
sent in order), and `.allow_expensive(bool)` / `.allow_constrained(bool)` gate cellular and
Low Data Mode use (native on Apple, advisory elsewhere):

```rust
let req = Request::post("https://api.example.com/notes", note_json_bytes)
    .header("Content-Type", "application/json")
    .timeout(std::time::Duration::from_secs(15));
```

For large downloads, `fetch_to_file(&req, &dest)` streams the body straight to disk, never
holding it in memory; `fetch_streamed` adds per-chunk control (progress, hashing, mid-body
cancel). To cache a response across launches, write `resp.body` with `day-part-fs`; see
[Store data on device](/docs/guide-storage).

## Pitfalls

- **Don't block the UI thread.** `fetch`, `fetch_to_file`, and `fetch_streamed` block their
  calling thread; run them on your own thread, or use the futures under `day::task`. On web
  the blocking calls return `Unsupported`; the single browser thread cannot wait.
- **`fetch_async` completes on a background thread.** Never touch signals directly in its
  callback; capture a `Setter` (`signal.setter()`), which hops to the UI thread itself. Under
  `day::task`, `fetch_future` resumes on the UI thread and needs none of this.
- **Cleartext `http://` is platform policy.** Apple's ATS refuses non-HTTPS URLs without a
  scoped Info.plist exception (loopback is exempt); Android blocks cleartext app-wide since
  targetSdk 28, loopback included; scope an exception in `network_security_config.xml`. The
  Rust fallback enforces no such policy, another reason `tier()` exists.
- **The browser adds CORS.** On web, cross-origin requests need the server's opt-in, and
  browser-controlled request headers (`Host`, `Cookie`, `Origin`) are ignored per the fetch
  spec. Network-level failures surface as `HttpError::Io`, since browsers hide DNS/TLS detail.

## Reference

[http](/docs/internal/http) — the full `Request`/`Response` contract, per-platform
realization, error mapping, and the cancel matrix.
[async](/docs/internal/async) — `day::task`, `TaskHandle`, `Resource`, and the rules that keep
async at the edges.
