---
navTitle: "HTTP"
title: HTTP requests
description: "Asynchronous HTTP requests, response handling, and cancellation."
order: 30
section: How-to guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Fetch a response with [`day-part-http`](/docs/reference/http) and handle transport failures separately from HTTP status
codes. Start with a working Day app and add [`day-part-http`](/docs/reference/http) from the same checkout or pinned
revision as `day`. Use the [recipe harness](/docs/reference/examples/catalog#technical-recipes)
to compile the example without making a network request.

## Fetch when the input changes

Keep the endpoint in a signal. Use `day::reactive::Resource` for a request owned by the current
page scope: it starts on construction, supersedes the previous request when the endpoint changes,
and aborts its task when the scope is disposed. This differs from an ordinary [`day::task`](https://daybrite.dev/api/day/fn.task.html), which
continues after the page closes unless you abort it.

Complete module for `src/http.rs` in the [technical recipe harness](/docs/reference/examples/catalog#technical-recipes).
The harness compiles this exact block; it is not a standalone app. Add the catalog entries below
to `resource/locales/en/app.ftl` and declare the module in `src/lib.rs`.

<!-- technical-example: http -->
```rust
use crate::res;
use day::prelude::*;
use day::reactive::Resource as AsyncResource;
use day_part_http::{Client, HttpError, Request};
use std::time::Duration;

pub fn response_panel(endpoint: Signal<String>) -> impl Piece {
    let response = AsyncResource::new(
        move || endpoint.get(),
        |url| async move {
            let response = Client::new()
                .fetch_limited_future(
                    Request::get(url)
                        .timeout(Duration::from_secs(15))
                        .timeout_total(Duration::from_secs(60)),
                    1024 * 1024,
                )
                .await?;
            if !(200..300).contains(&response.status) {
                return Err(HttpError::Status(response.status));
            }
            Ok(response.text().into_owned())
        },
    );
    load(response, |body| {
        label(move || body.get()).id(id!(response_body))
    })
    .loading(|| label(res::str::request_loading()).id(id!(request_loading)))
    .failed(|_| label(res::str::request_failed()).id(id!(request_failed)))
}
```

<!-- technical-catalog: http -->
```ftl
request_loading = Loading response…
request_failed = The request could not be completed.
```

The endpoint and response body are user/server data, not bundled resources or app-owned copy.
Validate an endpoint before enabling this panel. This recipe buffers at most 1 MiB of decoded
response bytes and returns `HttpError::BodyTooLarge` if the advertised or received body exceeds
that limit. Use streaming for larger responses. The body limit and timeouts are example policy
choices, not Day defaults. Keep raw transport diagnostics in logs and map
errors to localized UI messages.

## Choose request policy

| Contract | Behavior in the checked-out source |
| --- | --- |
| [`fetch_future(Request) -> FetchFuture`](https://daybrite.dev/api/day_part_http/fn.fetch_future.html) | Starts immediately; awaiting yields `Result<Response, HttpError>` |
| HTTP 4xx/5xx | Arrive as a response; the example explicitly converts rejected statuses to `HttpError::Status` |
| [`Request::timeout(Duration)`](https://daybrite.dev/api/day_part_http/struct.Request.html#method.timeout) | Overrides the client's idle timeout, normally 30 seconds |
| [`Request::timeout_total(Duration)`](https://daybrite.dev/api/day_part_http/struct.Request.html#method.timeout_total) | Bounds the whole request; the default client has no total limit |
| Redirects | The default client follows up to 10; use `ClientBuilder` for another policy |
| [`Response::text()`](https://daybrite.dev/api/day_part_http/struct.Response.html#method.text) | Lossy UTF-8; inspect `body` directly for binary data or strict decoding |
| [`Response::header(name)`](https://daybrite.dev/api/day_part_http/struct.Response.html#method.header) | First matching header, ASCII case-insensitive; duplicates remain in `headers` |
| Request defaults | Empty headers/body; expensive and constrained networking allowed; default cache policy; no explicit priority |

[`Request::get`](https://daybrite.dev/api/day_part_http/struct.Request.html#method.get), [`head`](https://daybrite.dev/api/day_part_http/struct.Request.html#method.head), and [`delete`](https://daybrite.dev/api/day_part_http/struct.Request.html#method.delete) accept a URL. [`post`](https://daybrite.dev/api/day_part_http/struct.Request.html#method.post), [`put`](https://daybrite.dev/api/day_part_http/struct.Request.html#method.put), and [`patch`](https://daybrite.dev/api/day_part_http/struct.Request.html#method.patch) also accept body
bytes. [`.header(name, value)`](https://daybrite.dev/api/day_part_http/struct.Request.html#method.header) appends a header; [`.body_file`](https://daybrite.dev/api/day_part_http/struct.Request.html#method.body_file), [`.body_stream`](https://daybrite.dev/api/day_part_http/struct.Request.html#method.body_stream), and [`.form`](https://daybrite.dev/api/day_part_http/struct.Request.html#method.form) offer
other payload sources. JSON parsing is an app dependency, not a built-in HTTP response method.
See the [HTTP reference](/docs/reference/http) for clients, cookies, redirects, credentials,
streaming, body limits, and the complete non-exhaustive error enum.

## Cancel and handle callbacks

Dropping a `FetchFuture` stops delivery and invokes the platform cancellation hook when available.
Aborting its `TaskHandle` drops that future. Cancellation is not a rollback of a request the
server has already accepted. Callback APIs take `Send + 'static` closures and may complete on an
unspecified thread; capture a `Setter`, not a `Signal`. Awaiting inside [`day::task`](https://daybrite.dev/api/day/fn.task.html) resumes on
the UI thread. Format generated messages there or in a reactive label closure.

## Account for the target

The source selects NSURLSession on Apple, OkHttp on Android, WinHTTP on Windows, the browser
transport on wasm32, and an ArkTS bridge on OpenHarmony. Linux uses system libcurl loaded at runtime. Unknown targets have no transport. [`tier()`](https://daybrite.dev/api/day_part_http/fn.tier.html) describes the HTTP implementation, not the website's maintenance tier;
[`capabilities()`](https://daybrite.dev/api/day_part_http/fn.capabilities.html) describes available options.

Blocking [`fetch`](https://daybrite.dev/api/day_part_http/fn.fetch.html), [`fetch_to_file`](https://daybrite.dev/api/day_part_http/fn.fetch_to_file.html), and [`fetch_streamed`](https://daybrite.dev/api/day_part_http/fn.fetch_streamed.html) must stay off the UI thread. Browser
blocking calls return `Unsupported`; OpenHarmony also rejects blocking waits on its JS thread.
Browser requests remain subject to CORS and restricted headers. TLS, cleartext, proxy, and
credential behavior depend on the selected transport and host configuration.

## Verify the result

Run the exact-code check from the [recipe setup](/docs/reference/examples/catalog#technical-recipes).
In your app, test a successful response, HTTP 404, timeout, replacement of an in-flight request,
and leaving the page on each target you ship. API details: [request and future contracts](https://github.com/daybrite/day/blob/main/crates/day-part-http/src/lib.rs)
and [client defaults](https://github.com/daybrite/day/blob/main/crates/day-part-http/src/client.rs).
