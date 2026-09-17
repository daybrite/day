---
title: "Async without a runtime"
description: "day::task futures on the main thread with no tokio: how background work returns to the UI through Setter and on_main."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Async tasks and background work

Use `day::task` to await work without blocking the interface. Its futures run on Day’s main
loop, so they can read and write signals before and after an `.await`. Day provides this
executor itself; apps do not need Tokio to use it.

Use a worker thread for CPU-heavy or blocking work. Return its results to the UI through
`Setter` or `on_main`, because signals cannot move between threads.

For app examples, see [HTTP requests](https://daybrite.dev/docs/guide-http) and
[Local storage](https://daybrite.dev/docs/guide-storage). This reference covers task lifetimes,
cancellation, and the rules for implementing asynchronous parts.

## `day::task` and `TaskHandle`

```rust
button("Save").action(move || {
    day::task(async move {
        if confirm("Overwrite?").await {           // native modal (docs/dialogs.md)
            let resp = day_part_http::fetch_future(req).await;
            status.set(render(resp));              // UI thread — a plain signal write
        }
    });
});
```

`task(fut)` polls the future once before returning and hands back a `TaskHandle` (`Copy`,
`!Send`, freely discardable). `handle.abort()` removes and drops the task's future; dropping
the future invokes its cleanup. For `fetch_future`, that requests cancellation where the
platform supports it; other implementations finish in the background and discard the result.
See the [HTTP cancellation contract](http.md). Aborting a finished task is a no-op; `is_finished()` reports
completed-or-aborted. Task ids are never reused, so stale handles are harmless.

A task is not owned by the scope that starts it. Leaving a page does not abort its tasks.
Keep the handle and abort it explicitly, or use `Resource` for work that should end with a scope.

## `Resource` and `Load` (day::reactive)

`Resource` is the declarative layer: a tracked `source` whose value feeds an async `fetcher`,
and the result is stored in a `Signal<Load<T>>`.

```rust
use day::reactive::{Load, Resource};

let stations = Resource::new(
    move || region.get(),                                   // tracked — refetch on change
    |region| async move { fetch_stations(region).await },   // Result<T, E: Error + Send + Sync>
);
when(move || stations.ready(), move || station_list(stations));
stations.refetch();                                         // force, even if region is unchanged
```

- `Load<T>` is `Loading | Ready(T) | Failed(Arc<dyn Error + Send + Sync>)`, `Clone`, with
  `ready()/is_loading()/is_ready()/error()` accessors. `Resource` is a `Copy` handle:
  `signal()`, `get()`, `with()`, `loading()`, `ready()` (all tracked), `refetch()`.
- **Latest wins.** A source change supersedes the in-flight fetch: its task is aborted (the
  drop requests cancellation where supported) and a completion that slips through writes
  nothing. `refetch()` always fetches; a rerun with an unchanged source value fetches nothing.
- **Scope cleanup cancels the fetch.** When the owning scope is disposed, its task is
  aborted. A late write to the disposed signal has no effect.
- The fetcher runs on the main-loop executor, so it may read and write signals after its
  awaits, and its source value needs no `Send` bound. §4.5's `MaybeSend` bound was removed for
  this reason. See the DESIGN status note.
- Namespacing: the prelude's `Resource` is the asset handle ([docs/resources.md](resources.md)), which
  predates this type; the async one lives at `day::reactive::Resource`, or depend on
  `day-reactive` directly.

`day-part-http` pairs with it for the common case (see the showcase's Platform-services page:
the loopback `Resource` demo, the PATCH `fetch_future` demo, and the URL checker that aborts
its previous in-flight task on re-tap).

## The policy

These rules keep UI state on the main thread and let parts work without depending on a
particular executor:

1. **Piece builders and event handlers are synchronous.** No `async fn` in `Piece::build`, actions,
   or event handlers. `day::task(async { … })` is the one explicit bridge from a sync action
   into a sequential flow.
2. **`day::task` is the only executor for signal-touching futures.** Its futures run on the UI
   thread, so after an `.await` they read and write signals directly, without a `Setter` or
   marshaling. Futures that never touch signals may run anywhere.
3. **Parts expose a callback and a future, never a runtime-bound API.** `fetch_async(req, cb)`
   plus `fetch_future(req)`; both must work in a plain-`main` binary and under `cargo test`
   (so a part never calls `on_main` itself, per [docs/http.md](http.md)'s contract).
4. **Keep other async runtimes in app-specific crates.** A dependency that demands tokio
   (matrix-rust-sdk) gets a headless core crate owning that runtime on background threads;
   results cross back only through `Setter`/`on_main`, and `!Send` handles never leave the
   main thread. The Day-Matrix app's `matrix-core` crate (a standalone Day app) is the
   reference; its bridge rule is documented at the top of its lib.rs. No `day-*` crate
   depends on an async runtime.
5. **Return to the UI thread through `Setter` or `on_main`** (DESIGN §3.3). Completion
   callbacks that run on background threads (e.g. `fetch_async`) deliver through them;
   futures on `day::task` don't need them.

## Under the hood

- The executor (`crates/day-core/src/present.rs`) stores boxed futures in a thread-local map;
  waking posts a re-poll through `day_reactive::on_main`. It is std-only, ~100 lines.
- day-reactive reaches the executor through an installed hook (`install_spawner`, the
  poster/scheduler pattern) because day-core depends on day-reactive, not the reverse.
  `day_core::launch_with` wires it on every backend (including mock). The spawner returns an
  abort closure that must be a no-op after completion: the spawner polls eagerly, so a
  synchronously-ready fetcher finishes before `Resource` can store the abort.
- `FetchFuture` ([docs/http.md](http.md)) is oneshot plumbing over `fetch_async`'s completion callback;
  its `Drop` runs the platform cancel. It has no executor dependency; any executor can await
  it, including a test's `block_on`.
- `day-async` (2026-09) holds that plumbing once, std-only: `oneshot()` — a `Deliver<T>` any
  thread may send from and a `Oneshot<T>` any executor can await, resolving to `Dropped` rather
  than pending forever when the sender goes away — and `TokenRegistry<T>`, the
  register-before-call, remove-by-token map behind every platform completion that crosses an
  FFI boundary as a number. The bridge's callback tier ([docs/bridge.md](bridge.md) "Callbacks")
  is its first user; the parts' hand-rolled copies migrate onto it.
- `day-async` also owns the process's one timer thread: `schedule(delay, job)` runs a job after a
  delay and `unschedule(id)` forgets it, so a part's time limits and backoff wait there instead of
  parking a thread each. day-part-http's total-time limits and question timeouts, and
  day-part-downloads' retry backoff, use it. The web has no threads, so a timer never fires there.

## Test hooks

- **day-core executor tests**: install an inline poster once
  (`day_reactive::install_main_poster(|f| f())`) and every wake re-polls synchronously on the
  test thread (`present.rs`'s `task_tests`).
- **day-reactive Resource tests**: `install_spawner` a miniature executor (poll-once at spawn +
  an explicit `pump()`), and resolve hand-rolled manual futures (`resource_tests`).
- **day-part-http future tests**: a ~25-line park/unpark `block_on` (tests/http.rs). The
  completion's wake from the delegate queue is exactly the cross-thread path production uses.
- Missing installs fail loudly: `on_main` and the spawner panic with "backend not started"
  rather than dropping work.
