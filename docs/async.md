---
navTitle: "Async"
title: "Async without a runtime"
description: "day::task futures on the main thread with no tokio: spawn_blocking for work that blocks, sleep on the toolkit's timer, and how a thread's results return to the UI through Setter and on_main."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Async tasks and background work

Use [`day::task`](https://docs.rs/day-app/latest/day/fn.task.html) to await work without blocking the interface. Its futures run on Day’s main
loop, so they can read and write signals before and after an `.await`. Day provides this
executor itself; apps do not need Tokio to use it.

Use [`day::spawn_blocking`](https://docs.rs/day-app/latest/day/fn.spawn_blocking.html) for CPU-heavy or blocking work: it runs the job on a pool thread and
resolves back in the task, where the result goes into a signal like any other value. A worker
thread of your own returns its results through `Setter` or [`on_main`](https://docs.rs/day-reactive/latest/day_reactive/fn.on_main.html), because signals cannot
move between threads.

For app examples, see [HTTP requests](/docs/how-to/guide-http) and
[Local storage](/docs/how-to/guide-storage). This reference covers task lifetimes,
cancellation, and the rules for implementing asynchronous parts.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## [`day::task`](https://docs.rs/day-app/latest/day/fn.task.html) and `TaskHandle`

```rust
button(res::str::save()).action(move || {
    let req = req.clone();
    day::task(async move {
        if confirm(res::str::overwrite()).await {
            // native modal (docs/dialogs.md)
            let resp = day_part_http::fetch_future(req).await;
            status.set(render(resp)); // UI thread — a plain signal write
        }
    });
});
```

[`task(fut)`](https://docs.rs/day-core/latest/day_core/fn.task.html) polls the future once before returning and hands back a `TaskHandle` (`Copy`,
`!Send`, freely discardable). [`handle.abort()`](https://docs.rs/day-core/latest/day_core/struct.TaskHandle.html#method.abort) removes and drops the task's future; dropping
the future invokes its cleanup. For `fetch_future`, that requests cancellation where the
platform supports it; other implementations finish in the background and discard the result.
See the [HTTP cancellation contract](http.md). Aborting a finished task is a no-op; [`is_finished()`](https://docs.rs/day-core/latest/day_core/struct.TaskHandle.html#method.is_finished) reports
completed-or-aborted. Task ids are never reused, so stale handles are harmless.

A task is not owned by the scope that starts it. Leaving a page does not abort its tasks.
Keep the handle and abort it explicitly, or use `AsyncData` for work that should end with a scope.

On `WillTerminate`, Day stops accepting async work and drops pending tasks before calling app
lifecycle handlers. Their destructors can still update signals; handlers can then synchronously
close databases and other dependencies. `DidExit` performs the same cleanup for backends whose
loop returns without `WillTerminate`. New tasks submitted during shutdown are dropped without
polling, and late wakes are ignored. This does not wait for detached worker threads or cancel
already accepted database writes; the owning service's shutdown must drain those writes.
OS process kills without a lifecycle notification cannot run this cleanup.

## [`spawn_blocking`](https://docs.rs/day-async/latest/day_async/fn.spawn_blocking.html)

```rust
day::task(async move {
    let parsed = day::spawn_blocking(move || parse_feed(&bytes)).await; // a pool thread
    match parsed {                                                     // the UI thread again
        Ok(Ok(feed)) => items.set(feed.items),
        Ok(Err(_)) => status.set(res::str::request_failed().format()),
        Err(_) => status.set(res::str::operation_failed(res::str::request_failed().format()).format()),                           // panicked, or never ran
    }
});
```

[`spawn_blocking(job)`](https://docs.rs/day-async/latest/day_async/fn.spawn_blocking.html) takes a `Send` closure and returns a future for its value. The job runs on
Day's blocking pool: process-wide, started by the first job, at most [`day::blocking_limit()`](https://docs.rs/day-app/latest/day/fn.blocking_limit.html)
threads (the machine's parallelism, between four and thirty-two), idle threads leaving after a
few seconds. Jobs beyond the limit queue in order. The future resolves to
`Err(BlockingError::Panicked)` when the job panicked and `Err(BlockingError::Closed)` when no
pool thread could ever run it.

Dropping the future before the job starts cancels it: the job never runs, which is what a
timeout racing a slow read wants. Dropping it while the job runs lets the job finish and
discards the value, since a blocking call cannot be interrupted. [`Blocking::detach`](https://docs.rs/day-core/latest/day_core/struct.Blocking.html#method.detach) keeps the
job without the value, for work whose effect is its own completion callback.

Use it for a file read, a parse, a hash, a database open, or a `block_on` into a runtime some
dependency insists on. It is not a replacement for `DatabaseWorker`, which keeps one
connection with ordered writes and reactive reads ([docs/persistence](persistence.md)), nor
for work that reports progress partway: that wants a channel, not polling. A part that
exposes a future should await [`spawn_blocking`](https://docs.rs/day-async/latest/day_async/fn.spawn_blocking.html) rather than start a thread, so its test's
`block_on` and an app's [`day::task`](https://docs.rs/day-app/latest/day/fn.task.html) get the same behavior.

On the web there is no thread, so the job runs inline at the future's first poll. Keep web jobs
short, or branch on `cfg!(target_arch = "wasm32")`. The mock toolkit runs jobs inline as well,
because its posts run on the test thread ([`day_async::set_blocking_inline`](https://docs.rs/day-async/latest/day_async/fn.set_blocking_inline.html)).

## [`sleep`](https://docs.rs/day-core/latest/day_core/fn.sleep.html)

[`day::sleep(ms)`](https://docs.rs/day-app/latest/day/fn.sleep.html) resolves after at least `ms` milliseconds, on the UI thread. It is the
toolkit's own timer, not a parked thread: `dispatch_after` on Apple, `Handler.postDelayed` on
Android, `g_timeout_add` on GTK, `QTimer::singleShot` on Qt, a Win32 `SetTimer` on Windows, and
`setTimeout` on ArkUI and the web. A loop that ticks every second, a debounce, or a timeout raced
against a fetch costs nothing but the timer. Dropping the sleep (aborting its task) lets the
native timer fire into nothing. The platform may round a short delay up: Windows timers tick
every ten milliseconds or so. Both Windows backends check a monotonic deadline when a timer signals
and rearm an early signal for the remaining time, so clock granularity cannot shorten a sleep.

## Async data

Asynchronous data has a lifecycle separate from the view that displays it: it can be loading,
available, refreshing or failed. `AsyncData<T, E>` owns that lifecycle for an HTTP request,
a database read or another future. Bundled images, files and translations are the separate
[app resources](resources.md).

```rust
let stations = AsyncData::new(
    move || region.get(),
    |region| async move { fetch_stations(region).await },
);
load(stations, |data| {
    // Read in bindings so new results update the mounted content.
    label(move || data.with(|stations| stations.len().to_string()))
})
.loading(spinner)
.failed(|_| label(res::str::request_failed()).secondary());
stations.refetch();
```

### Creation and source identity

[`AsyncData::new`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.new)
starts a load immediately. Reads in the source closure are tracked; reads in the fetcher are
not. An unchanged source starts no request. The fetcher returns `Result<T, E>`; neither data
nor error requires `Clone`, `Error`, `Send` or `Sync`. Its future runs on the main loop.
[`AsyncData::infallible`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.infallible)
accepts a future returning `T` directly and uses `Infallible` as its error type.

[`AsyncData::optional`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.optional)
takes a source returning `Option<S>`. `None` cancels work, clears data and errors, and becomes
idle. It does not invoke the fetcher, even when asked to refetch.

An explicit refetch retains the value. A changed source clears it: switching cities must not
show the old city's forecast under the new heading. To retain previous results across source
changes, use [`AsyncData::with_options`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.with_options)
or [`AsyncData::optional_with_options`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.optional_with_options)
with `AsyncDataOptions { keep_previous: true }`. The
[`stale()`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.stale)
read then tells the view that the value belongs to another source. Switching back to the
value's source clears that flag. A same-source refresh is not stale in this sense; the flag
is not an age or expiration policy.

### State and reads

[`state()`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.state)
returns a read-only `LoadState<T, E>` snapshot. Values and errors are held in `Rc`, so copying
a snapshot does not copy their contents. No writable state signal is exposed.

| State | Meaning |
| --- | --- |
| `Idle` | No source, or an initial request was cancelled |
| `Loading` | A request with no previous value |
| `Ready(value)` | The last request succeeded |
| `Refreshing(value)` | A request with a retained value |
| `Failed { error, previous }` | An initial failure, or a refresh failure with retained data |

The error clears when another request starts. A failed refresh keeps the previous value.
All reads below are tracked; wrap them in `untrack` when no subscription is wanted.

| Read | Result |
| --- | --- |
| [`value()`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.value) | An owned `Option<T>`; requires `T: Clone` |
| [`with_value()`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.with_value) | A borrowed `Option<&T>`; no clone bound |
| [`error()`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.error) | An owned `Option<E>`; requires `E: Clone` |
| [`with_error()`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.with_error) | A borrowed `Option<&E>`; no clone bound |
| [`with_state()`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.with_state) | A borrowed complete `LoadState` |
| [`has_value()`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.has_value) | Whether data is available, including during refresh or refresh failure |
| [`loading()`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.loading) | Whether any request is running |
| [`refreshing()`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.refreshing) | Whether a request is running with retained data |

`LoadState` also exposes
[`value`](https://docs.rs/day-reactive/latest/day_reactive/enum.LoadState.html#method.value),
[`shared_value`](https://docs.rs/day-reactive/latest/day_reactive/enum.LoadState.html#method.shared_value),
[`error`](https://docs.rs/day-reactive/latest/day_reactive/enum.LoadState.html#method.error),
[`is_loading`](https://docs.rs/day-reactive/latest/day_reactive/enum.LoadState.html#method.is_loading) and
[`is_refreshing`](https://docs.rs/day-reactive/latest/day_reactive/enum.LoadState.html#method.is_refreshing).
`shared_value` borrows the `Rc<T>` when a consumer needs to retain it cheaply.

### Rendering and refresh

[`load(data, ready)`](https://docs.rs/day-pieces/latest/day_pieces/fn.load.html) accepts an
`AsyncData`, a `Signal<LoadState<T, E>>`, or a closure returning `LoadState<T, E>`.
Its content stays mounted whenever data is available, preserving focus, scroll position and
local state during refresh and refresh failure. Idle shows nothing. The
[`.loading(...)`](https://docs.rs/day-pieces/latest/day_pieces/struct.LoadPiece.html#method.loading)
and [`.failed(...)`](https://docs.rs/day-pieces/latest/day_pieces/struct.LoadPiece.html#method.failed)
placeholders apply when no value is available; the failed callback receives `&E`.
A different failure updates the error placeholder even if the state remains failed.

The content builder receives `Loaded<T>`. Clone this handle into bindings and read
[`get()`](https://docs.rs/day-pieces/latest/day_pieces/struct.Loaded.html#method.get) or
[`with(...)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Loaded.html#method.with)
inside them. `with` borrows without cloning `T`. Reading once while building captures a
snapshot that will not update the view after a refresh.

Present refresh progress with
[`refreshing()`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.refreshing)
and refresh errors with
[`with_error()`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.with_error)
in adjacent pieces. To deliberately replace content during refresh, opt into
[`.replace_on_refresh()`](https://docs.rs/day-pieces/latest/day_pieces/struct.LoadPiece.html#method.replace_on_refresh).

### Cancellation and ownership

[`refetch()`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.refetch)
forces a request with the current source. Calls in a batch coalesce. A newer request
invalidates and aborts the earlier one; late completions cannot overwrite the current state.

[`cancel()`](https://docs.rs/day-reactive/latest/day_reactive/struct.AsyncData.html#method.cancel)
cancels the running request and restores its previous settled state, including a previous
refresh error. Cancelling an initial request returns to idle. Cancellation creates no error
and does nothing after scope disposal. A later refetch starts another request.

An async data handle is `Copy`, so dropping a copy does not cancel it. Its creation scope owns
its reaction and task. Disposing that scope invalidates and aborts pending work. A cache
entry should own a child scope and dispose it on eviction, outside any cache borrow; a
root-scoped handle otherwise lives until application shutdown. Physical cancellation depends
on the future: [HTTP cancellation](http.md) may stop the request or discard its result.

The API provides no automatic retry, shared cache, persistence or offline synchronization.

## Threading rules

These rules keep UI state on the main thread and let parts work without depending on a
particular executor:

1. **Piece builders and event handlers are synchronous.** No `async fn` in [`Piece::build`](https://docs.rs/day-core/latest/day_core/trait.Piece.html#tymethod.build), actions,
   or event handlers. [`day::task(async { … })`](https://docs.rs/day-app/latest/day/fn.task.html) is the one explicit bridge from a sync action
   into a sequential flow.
2. **[`day::task`](https://docs.rs/day-app/latest/day/fn.task.html) is the only executor for signal-touching futures.** Its futures run on the UI
   thread, so after an `.await` they read and write signals directly, without a `Setter` or
   marshaling. Futures that never touch signals may run anywhere.
   **Blocking work inside such a future goes through [`day::spawn_blocking`](https://docs.rs/day-app/latest/day/fn.spawn_blocking.html)**, never a
   [`std::thread::spawn`](https://doc.rust-lang.org/std/thread/fn.spawn.html) plus a oneshot of your own, and never inline: a parse or a file read in
   a [`day::task`](https://docs.rs/day-app/latest/day/fn.task.html) body stalls the interface for its whole duration.
3. **Parts expose a callback and a future, never a runtime-bound API.** [`fetch_async(req, cb)`](https://docs.rs/day-part-http/latest/day_part_http/fn.fetch_async.html)
   plus [`fetch_future(req)`](https://docs.rs/day-part-http/latest/day_part_http/fn.fetch_future.html); both must work in a plain-`main` binary and under `cargo test`
   (so a part never calls [`on_main`](https://docs.rs/day-reactive/latest/day_reactive/fn.on_main.html) itself, per [docs/http](http.md)'s contract).
4. **Keep other async runtimes in app-specific crates.** A dependency that demands tokio
   (matrix-rust-sdk) gets a headless core crate owning that runtime on background threads;
   results cross back only through `Setter`/[`on_main`](https://docs.rs/day-reactive/latest/day_reactive/fn.on_main.html), and `!Send` handles never leave the
   main thread. The Day-Matrix app's `matrix-core` crate (a standalone Day app) is the
   reference; its bridge rule is documented at the top of its lib.rs. No `day-*` crate
   depends on an async runtime.
5. **Return to the UI thread through `Setter` or [`on_main`](https://docs.rs/day-reactive/latest/day_reactive/fn.on_main.html)**. Completion
   callbacks that run on background threads (e.g. `fetch_async`) deliver through them;
   futures on [`day::task`](https://docs.rs/day-app/latest/day/fn.task.html) don't need them.

## Example setup and task lifetime

The Rust blocks above are integration fragments with app-defined values and functions; they
are not standalone apps. Use the [compiled HTTP recipe](/docs/how-to/guide-http)
for complete imports and catalog keys. `request_failed` is defined in
that recipe's catalog; other generated messages come from [the snippet catalog](/docs/reference/localization#reference-example-messages).

[`on_main`](https://docs.rs/day-reactive/latest/day_reactive/fn.on_main.html) takes a `Send + 'static` closure and requires an installed main poster. It cannot
capture a `Signal`; pass data through a `Setter` instead. [`day::task`](https://docs.rs/day-app/latest/day/fn.task.html) accepts a non-`Send`
`Future<Output = ()>` and eagerly polls it once. Keep CPU or blocking IO work out of that poll.
Dropping a task handle does not abort its task; call [`abort`](https://docs.rs/day-core/latest/day_core/struct.TaskHandle.html#method.abort) or use a scope-owned `AsyncData`.

See [executor and shutdown](../crates/day-core/src/present.rs),
[Async data and Setter](../crates/day-reactive/src/lib.rs), and
[blocking pool](../crates/day-async/src/lib.rs) for implementation details.
