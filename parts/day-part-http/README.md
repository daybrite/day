<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-part-http

Fetch like the platform, not around it.

This crate does HTTP through each platform's own networking stack: URLSession on macOS and iOS,
OkHttp on Android, WinHTTP on Windows, the system's libcurl on Linux, the Network Kit on
HarmonyOS, and the browser's `fetch` and `WebSocket` on the web. Requests pick up what the OS
already knows: system proxies and PAC scripts, VPN routing, Low Data Mode, and enterprise
certificate stores.

The crate-root functions send one request at a time: blocking `fetch`, callback `fetch_async`,
awaitable `fetch_future` (dropping it cancels the request), and `fetch_to_file` and
`fetch_streamed` for bodies that belong on disk. A `Client` adds the rest of a modern HTTP API on
the same stacks: response bodies streamed with backpressure, uploads from files, readers and
multipart forms, redirect and authentication callbacks, public-key pins and server trust
decisions, client certificates, cookies, caching, transfer metrics, and WebSockets.
`capabilities()` reports what the platform offers. HTTP error statuses are responses rather than
errors, and the request timeout bounds progress, so a long download that keeps moving runs to
the end. The crate also ships the local test server its tests and the showcase use.

Parts are Day's small capability crates: a plain Rust API over something the platform already
provides. This one works in any Rust program, with or without a Day app around it.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps out of
each platform's own widgets — AppKit, UIKit, Android's Material widgets, GTK 4, Qt 6, XAML, and
ArkUI — from one codebase. When you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button. The framework also ships the tooling around the app: the `day` CLI, a VS
Code extension, GitHub CI workflows, localization, accessibility, and dayscript automation.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).

## Sessions and runtime providers

`Session` owns provider selection, optional interception, and payload statistics. Clones share
that scope. `Client::builder().session(session.clone()).build()` creates a client in it. A new
session starts with the native provider and no interception; it is isolated from global settings.
Clients without an explicit session, including the crate-root `fetch_*` functions, follow
`Session::global()`. Changes apply to subsequent logical requests and WebSocket connections,
even on clients created before the change. Active operations retain their selected provider.

```rust,no_run
use day_part_http::{Client, Provider, Session};
let session = Session::new();
let client = Client::builder().session(session.clone()).build();
session.set_provider(Provider::Native);
// A custom factory receives each client's TransportConfig and returns Arc<dyn Transport>:
// session.set_provider(Provider::custom(|config| Arc::new(MyTransport::new(config))));
// Session::global().set_provider(...) changes the default for ordinary app clients.
```

An explicit `ClientBuilder::transport(...)` remains a fixed transport and bypasses provider
selection/interception. Its traffic still contributes to global statistics.

### Optional reqwest provider

Enable `day-part-http`'s **`reqwest` feature** (off by default), then select
`session.set_provider(Provider::Reqwest)`. It uses reqwest for HTTP, tokio-tungstenite for
WebSockets, a shared two-worker Tokio runtime, and rustls with WebPKI roots. The native stack
remains the default even when the feature is compiled. The provider is available on native
macOS, iOS, Android, Windows and desktop Linux; its dependencies and enum variant are excluded
on wasm and HarmonyOS. Those targets retain the native provider and the portable simulator.

It supports streaming, upload progress, cancellation, redirects, portable cookie jars and
HTTP authentication handled by `Client`. It does **not** claim native PAC/VPN policy parity,
platform caches/cookie stores, native authentication schemes, client identities or custom
server-trust callbacks. Pins and client identities fail with `Unsupported` rather than being
silently ignored. Inspect `client.capabilities()` after changing providers. Cached connections
are retained per client/provider generation; existing transfers finish on the previous provider.

## Programmatic interception

Handlers receive the real prepared request, after headers and portable cookies have been
applied. Responses then pass through the ordinary redirect/authentication, streaming, size-limit,
parsing and application logic. No listening port or native network exchange is needed
for the simulated response. This works in native apps and browsers.

```rust,no_run
use day_part_http::{Client, Request, Session, simulation::{Simulation, Reply}};
let simulation = Simulation::new();
simulation.route("GET", "https://api.example/items", |request| {
    assert_eq!(request.method, "GET");
    Ok(Reply::new(200, br#"{"items":[]}"#.to_vec())
        .header("Content-Type", "application/json"))
});
let session = Session::new();
session.set_simulation(Some(simulation.clone()));
let client = Client::builder().session(session.clone()).build();
// client.fetch_future(Request::get("https://api.example/items")).await?;
// session.set_simulation(None); // subsequent requests use the selected real provider
```

`handle(predicate, handler)` supports arbitrary method/URL predicates, including parsed origin,
path and query matching. The first matching registration wins. Handlers may retain synchronized
state to implement pagination, ETags/304, authentication, or response sequences. They run
synchronously outside internal locks: keep them short and express waiting through `Reply::delay`
or conditions. Request bodies remain available through `transport::PreparedBody`; one-shot
stream bodies can be taken once. `Reply::chunks` defines streaming boundaries and
`fail_after_chunks` injects a mid-response failure.

**Unmatched requests fail by default.** `set_strict(false)` explicitly permits unmatched initial
requests to use the real provider. Once a request selects simulation, its redirects remain in
that simulator; an unmatched redirect fails rather than unexpectedly escaping to the internet.
`requests()` retains the latest 256 method/URL/ordinal/fault decisions, and `request_count()`
counts requests per method/URL. Bodies and authorization headers are not retained automatically.

For WebSockets, `websocket_echo(url)` installs an echo handler; `websocket(predicate, handler)`
can transform or reject messages. Delivery respects reader demand, pause/offline settings and
bandwidth limits. The pending message queue is bounded. Real provider sockets and simulated
sockets use the same `Client::websocket_*`, `WebSocket` and `WsSender` APIs.

### Deterministic adverse conditions

```rust,no_run
use day_part_http::simulation::{Conditions, Simulation};
use std::time::Duration;
let simulation = Simulation::new();
simulation.set_conditions(Conditions {
    latency: Duration::from_millis(200),
    bytes_per_second: Some(64 * 1024),
    failure_rate: 0.15,
    seed: 42,
    paused: false,
    offline: false,
});
```

Conditions apply to **simulated traffic**. HTTP upload size contributes to the initial delay;
response chunks are paced per transfer. Pausing suspends delivery, not timeout clocks. Offline
mode and injected failures produce transport errors. A response can separately prescribe delay
or fail after a specific chunk for an exact scenario. These are application-visible effects,
not TCP packet loss, shared radio bandwidth, or OS background-process suspension.

Failure decisions use the seed plus method, URL and occurrence number, so unrelated URLs do not
consume each other's random sequence. Concurrent identical requests still need explicit distinct
identities or a prescribed response sequence when their ordering matters. Replaying a seed does
not make thread scheduling deterministic.

`Simulation::with_clock(Arc::new(ManualClock::default()))` accepts an advanceable scheduler.
`clock.advance(duration)` runs due network events without sleeping; simulated idle and total
HTTP timeouts use that clock. Real providers and application timers retain their own clocks.
Use isolated sessions for parallel tests; install a global simulation before app startup only
when intentionally controlling an entire test process. Dayscripts can drive app controls or
select an app-owned startup scenario, as Showcase and News demonstrate.

## Statistics

```rust,no_run
use day_part_http::{Session, Statistics};
let session = Session::new();
let total = session.statistics();
println!("{} bytes, {:.1} B/s down, {:.1} B/s up", total.total_bytes(),
    total.download_bytes_per_second(), total.upload_bytes_per_second());
for transfer in session.transfers() {
    println!("{} {}: {:?}", transfer.id, transfer.url, transfer.statistics);
}
let app_total = Statistics::global(); // includes all sessions and explicit transports
```

> **Scope:** only operations passing through `day-part-http` are counted. These are not
> system-wide network counters: WebViews, media players, maps, reqwest clients created outside
> this provider, and other frameworks' networking are excluded.

Counters include started/active/completed/failed exchanges, upload/download payload bytes and
separate simulated byte totals. Session/global averages divide bytes by elapsed scope time,
including idle time; transfer averages use that exchange's lifetime. For a live chart, subtract
successive snapshots and divide by their elapsed-time difference. Showcase displays four animated
series (real/simulated × upload/download) with a bounded 30-second history.

An HTTP transfer is one exchange (redirect hops and authentication retries are separate); a
WebSocket transfer is one socket lifetime. These are **not physical TCP connection counts**:
pooling and HTTP multiplexing belong to the provider. Existing `Metrics` exposes provider facts
such as connection reuse when available. Payload counters exclude protocol overhead; upload
observations use provider payload-progress events and are not proof of remote receipt. Providers
without upload progress (browser fetch) count a known request body when response headers arrive;
partial uploads that fail before a response cannot be measured on those providers. Records retain at most 256 transfers per scope; lifetime aggregate counters survive history eviction.
Statistics never retain response bodies. Avoid credentials in URLs, which appear in history.

## Validation

`cargo test -p day-part-http --features reqwest` exercises isolated handlers, real client
redirects/cookies, deterministic faults, streaming limits, pause/cancellation, WebSocket echo,
and native/reqwest switching against the local test server. No external API is needed for these
interception/provider regression tests. Existing loopback tests remain necessary: simulation
cannot validate native DNS, TLS, proxy configuration or OS scheduling.
