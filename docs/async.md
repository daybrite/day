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
Keep the handle and abort it explicitly, or use `Resource` for work that should end with a scope.

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

## `Resource` and `Load` (day::reactive)

`Resource` is the declarative layer: a tracked `source` whose value feeds an async `fetcher`,
and the result is stored in a `Signal<Load<T>>`.

```rust
use day::reactive::{Load, Resource};

let stations = Resource::new(
    move || region.get(),                                   // tracked — refetch on change
    |region| async move { fetch_stations(region).await },   // Result<T, E: Error + Send + Sync>
);
load(stations, |s| station_list(s.get()))                  // the three states as subtrees
    .loading(spinner)
    .failed(|_| label(res::str::request_failed()).secondary());
stations.refetch();                                         // force, even if region is unchanged
```

[`load(state, ready)`](https://docs.rs/day-pieces/latest/day_pieces/fn.load.html) shows the three states as three subtrees: the ready arm, and optional
[`.loading(build)`](https://docs.rs/day-pieces/latest/day_pieces/struct.LoadPiece.html#method.loading) and [`.failed(|err| build)`](https://docs.rs/day-pieces/latest/day_pieces/struct.LoadPiece.html#method.failed) arms (a state without an arm builds nothing).
`state` is a `Resource<T>`, a `Signal<Load<T>>` or a closure returning a `Load<T>`. Arms rebuild
only when the variant changes; the ready arm is handed a `Loaded<T>`, whose [`get()`](https://docs.rs/day-pieces/latest/day_pieces/struct.Loaded.html#method.get) is a tracked
read that follows `Ready(a)` to `Ready(b)` in place, so a refetch updates the list without
rebuilding it. Whether a refetch passes through `Loading` is the `Resource`'s policy (below); an
app that wants its stale content shown during a refresh keeps the last value in its own state
and the ready arm reads it.

- `Load<T>` is `Loading | Ready(T) | Failed(Arc<dyn Error + Send + Sync>)`, `Clone`, with
  [`ready`](https://docs.rs/day-reactive/latest/day_reactive/enum.Load.html#method.ready)`()/`[`is_loading`](https://docs.rs/day-reactive/latest/day_reactive/enum.Load.html#method.is_loading)`()/`[`is_ready`](https://docs.rs/day-reactive/latest/day_reactive/enum.Load.html#method.is_ready)`()/`[`error`](https://docs.rs/day-reactive/latest/day_reactive/enum.Load.html#method.error)`()` accessors. `Resource` is a `Copy` handle:
  [`signal()`](https://docs.rs/day-reactive/latest/day_reactive/struct.Resource.html#method.signal), [`get()`](https://docs.rs/day-reactive/latest/day_reactive/struct.Resource.html#method.get), [`with()`](https://docs.rs/day-reactive/latest/day_reactive/struct.Resource.html#method.with), [`loading()`](https://docs.rs/day-reactive/latest/day_reactive/struct.Resource.html#method.loading), [`ready()`](https://docs.rs/day-reactive/latest/day_reactive/struct.Resource.html#method.ready) (all tracked), [`refetch()`](https://docs.rs/day-reactive/latest/day_reactive/struct.Resource.html#method.refetch).
- **Latest wins.** A source change supersedes the in-flight fetch: its task is aborted (the
  drop requests cancellation where supported) and a completion that slips through writes
  nothing. [`refetch()`](https://docs.rs/day-reactive/latest/day_reactive/struct.Resource.html#method.refetch) always fetches; a rerun with an unchanged source value fetches nothing.
- **Scope cleanup cancels the fetch.** When the owning scope is disposed, its task is
  aborted. A late write to the disposed signal has no effect.
- The fetcher runs on the main-loop executor, so it may read and write signals after its
  awaits, and its source value needs no `Send` bound.
- Namespacing: the prelude's `Resource` is the asset handle ([docs/resources](resources.md)), which
  predates this type; the async one lives at `day::reactive::Resource`, or depend on
  [`day-reactive`](/docs/explanation/state) directly.

[`day-part-http`](/docs/reference/http) pairs with it for the common case (see the showcase's Platform-services page:
the loopback `Resource` demo, the PATCH `fetch_future` demo, and the URL checker that aborts
its previous in-flight task on re-tap).

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
   `std::thread::spawn` plus a oneshot of your own, and never inline: a parse or a file read in
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
Dropping a task handle does not abort its task; call [`abort`](https://docs.rs/day-core/latest/day_core/struct.TaskHandle.html#method.abort) or use a scope-owned `Resource`.

See [executor and shutdown](../crates/day-core/src/present.rs),
[Resource and Setter](../crates/day-reactive/src/lib.rs), and
[blocking pool](../crates/day-async/src/lib.rs) for implementation details.
