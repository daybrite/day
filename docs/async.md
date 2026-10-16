---
title: "Async without a runtime"
description: "day::task futures on the main thread with no tokio: spawn_blocking for work that blocks, sleep on the toolkit's timer, and how a thread's results return to the UI through Setter and on_main."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Async tasks and background work

Use `day::task` to await work without blocking the interface. Its futures run on Day’s main
loop, so they can read and write signals before and after an `.await`. Day provides this
executor itself; apps do not need Tokio to use it.

Use `day::spawn_blocking` for CPU-heavy or blocking work: it runs the job on a pool thread and
resolves back in the task, where the result goes into a signal like any other value. A worker
thread of your own returns its results through `Setter` or `on_main`, because signals cannot
move between threads.

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

On `WillTerminate`, Day stops accepting async work and drops pending tasks before calling app
lifecycle handlers. Their destructors can still update signals; handlers can then synchronously
close databases and other dependencies. `DidExit` performs the same cleanup for backends whose
loop returns without `WillTerminate`. New tasks submitted during shutdown are dropped without
polling, and late wakes are ignored. This does not wait for detached worker threads or cancel
already accepted database writes; the owning service's shutdown must drain those writes.
OS process kills without a lifecycle notification cannot run this cleanup.

## `spawn_blocking`

```rust
day::task(async move {
    let parsed = day::spawn_blocking(move || parse_feed(&bytes)).await; // a pool thread
    match parsed {                                                     // the UI thread again
        Ok(Ok(feed)) => items.set(feed.items),
        Ok(Err(e)) => status.set(e.to_string()),
        Err(e) => status.set(e.to_string()),                           // panicked, or never ran
    }
});
```

`spawn_blocking(job)` takes a `Send` closure and returns a future for its value. The job runs on
Day's blocking pool: process-wide, started by the first job, at most `day::blocking_limit()`
threads (the machine's parallelism, between four and thirty-two), idle threads leaving after a
few seconds. Jobs beyond the limit queue in order. The future resolves to
`Err(BlockingError::Panicked)` when the job panicked and `Err(BlockingError::Closed)` when no
pool thread could ever run it.

Dropping the future before the job starts cancels it: the job never runs, which is what a
timeout racing a slow read wants. Dropping it while the job runs lets the job finish and
discards the value, since a blocking call cannot be interrupted. `Blocking::detach` keeps the
job without the value, for work whose effect is its own completion callback.

Use it for a file read, a parse, a hash, a database open, or a `block_on` into a runtime some
dependency insists on. It is not a replacement for `DatabaseWorker`, which keeps one
connection with ordered writes and reactive reads ([docs/persistence.md](persistence.md)), nor
for work that reports progress partway: that wants a channel, not polling. A part that
exposes a future should await `spawn_blocking` rather than start a thread, so its test's
`block_on` and an app's `day::task` get the same behavior.

On the web there is no thread, so the job runs inline at the future's first poll. Keep web jobs
short, or branch on `cfg!(target_arch = "wasm32")`. The mock toolkit runs jobs inline as well,
because its posts run on the test thread (`day_async::set_blocking_inline`).

## `sleep`

`day::sleep(ms)` resolves after at least `ms` milliseconds, on the UI thread. It is the
toolkit's own timer, not a parked thread: `dispatch_after` on Apple, `Handler.postDelayed` on
Android, `g_timeout_add` on GTK, `QTimer::singleShot` on Qt, a Win32 `SetTimer` on XAML, and
`setTimeout` on ArkUI and the web. A loop that ticks every second, a debounce, or a timeout raced
against a fetch costs nothing but the timer. Dropping the sleep (aborting its task) lets the
native timer fire into nothing. The platform may round a short delay up: Windows timers tick
every ten milliseconds or so. XAML and WinUI check a monotonic deadline when a timer signals
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
   **Blocking work inside such a future goes through `day::spawn_blocking`**, never a
   `std::thread::spawn` plus a oneshot of your own, and never inline: a parse or a file read in
   a `day::task` body stalls the interface for its whole duration.
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
  It stays separate from `day::sleep`'s native timer because a part must work in a plain binary
  and under `cargo test`, where no backend has installed a timer door.
- `day-async` holds the blocking pool behind `spawn_blocking` (2026-10): a `VecDeque` of jobs, a
  `Condvar`, and threads that start on demand up to `blocking_limit()` and exit after two idle
  seconds. A job is the user's closure under `catch_unwind`, delivering through a `oneshot`; a
  cancelled flag, set by the future's `Drop`, skips a job that has not started. day-part-fs's
  async operations run on it, detached. `blocking_threads()` reports the live count.
- `day::sleep` rides `Platform::post_delayed`, which every backend implements with the loop's
  own timer; the trait default (a thread that sleeps and posts) logs once when a backend without
  one falls back to it. The conformance app's headless cases `sleep-*`,
  `an-aborted-task-never-wakes-from-its-sleep` and `*-blocking-*` run on every toolkit in CI
  ([docs/testing.md](testing.md)) and prove the timer resolves after its delay, in delay order,
  yields on zero, and that blocking jobs leave the UI thread, resolve on it, report panics and
  honor cancellation.

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
