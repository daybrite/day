# Async: futures without a runtime

> **Status: implemented** (DESIGN.md §4.5, revised 2026-07). Day runs futures on its own
> main-loop executor — `day::task` — with no async runtime: no tokio, no reactor, no thread
> pool. The executor polls `!Send` futures on the UI thread; wakers re-poll through the same
> `on_main` poster everything else rides. On top of it sit `present().await` (docs/dialogs.md),
> `day_part_http::fetch_future` (docs/http.md), and `day::reactive::Resource` (below).

## The policy

Five rules keep async at the edges and the reactive core single-threaded. They are the
contract for every `day-*` crate and the recommended shape for apps:

1. **Async never appears in the authoring surface.** No `async fn` in `Piece::build`, actions,
   or event handlers. `day::task(async { … })` is the one explicit bridge from a sync action
   into a sequential flow.
2. **`day::task` is the only executor for signal-touching futures.** Its futures run on the UI
   thread, so after an `.await` they read and write signals directly — no `Setter`, no
   marshaling. Futures that never touch signals may run anywhere.
3. **Parts expose a callback and a future, never a runtime-bound API.** `fetch_async(req, cb)`
   plus `fetch_future(req)`; both must work in a plain-`main` binary and under `cargo test`
   (so a part never calls `on_main` itself — docs/http.md's contract).
4. **Foreign runtimes are quarantined in app-private crates.** A dependency that demands tokio
   (matrix-rust-sdk) gets a headless core crate owning that runtime on background threads;
   results cross back only through `Setter`/`on_main`, and `!Send` handles never leave the
   main thread. `apps/matrix/matrix-core` is the reference (its bridge rule is documented at
   the top of its lib.rs). No `day-*` crate depends on an async runtime.
5. **`Setter` and `on_main` remain the only cross-thread doors** (DESIGN §3.3). Completion
   callbacks that run on background threads (e.g. `fetch_async`) deliver through them;
   futures on `day::task` don't need them.

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
`!Send`, freely discardable). `handle.abort()` removes and drops the task's future — an
in-flight `.await` cancels via `Drop`, so aborting a task that awaits a `fetch_future` cancels
the platform request. Aborting a finished task is a no-op; `is_finished()` reports
completed-or-aborted. Task ids are never reused, so stale handles are harmless.

## `Resource` and `Load` (day::reactive)

The declarative layer: a tracked `source` whose value feeds an async `fetcher`; the result
lands in a `Signal<Load<T>>`.

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
  drop cancels any platform request inside) and a completion that slips through writes
  nothing. `refetch()` always fetches; a rerun with an unchanged source value fetches nothing.
- **Disposal is clean.** The owning scope's death aborts the in-flight fetch; a late write
  hits the disposed-signal no-op.
- The fetcher runs on the main-loop executor, so it may read and write signals after its
  awaits, and its source value needs no `Send` bound. §4.5's `MaybeSend` seam collapsed for
  this reason — see the DESIGN status note.
- Namespacing: the prelude's `Resource` is the ASSET handle (docs/resources.md), which
  predates this type — the async one lives at `day::reactive::Resource`, or depend on
  `day-reactive` directly.

`day-part-http` pairs with it for the common case (see the showcase's Platform-services page:
the loopback `Resource` demo, the PATCH `fetch_future` demo, and the URL checker that aborts
its previous in-flight task on re-tap).

## Under the hood

- The executor (`crates/day-core/src/present.rs`) stores boxed futures in a thread-local map;
  waking posts a re-poll through `day_reactive::on_main`. It is std-only, ~100 lines.
- day-reactive reaches the executor through an installed hook — `install_spawner`, the
  poster/scheduler pattern — because day-core depends on day-reactive, not the reverse.
  `day_core::launch_with` wires it on every backend (including mock). The spawner returns an
  abort closure that MUST be a no-op after completion: the spawner polls eagerly, so a
  synchronously-ready fetcher finishes before `Resource` can store the abort.
- `FetchFuture` (docs/http.md) is oneshot plumbing over `fetch_async`'s completion callback;
  its `Drop` runs the platform cancel. It has no executor dependency — any executor can await
  it, including a test's `block_on`.

## Testing seams

- **day-core executor tests**: install an inline poster once —
  `day_reactive::install_main_poster(|f| f())` — and every wake re-polls synchronously on the
  test thread (`present.rs`'s `task_tests`).
- **day-reactive Resource tests**: `install_spawner` a miniature executor (poll-once at spawn +
  an explicit `pump()`), and resolve hand-rolled manual futures (`resource_tests`).
- **day-part-http future tests**: a ~25-line park/unpark `block_on` (tests/http.rs) — the
  completion's wake from the delegate queue is exactly the cross-thread path production uses.
- Missing installs fail loudly: `on_main` and the spawner panic with "backend not started"
  rather than dropping work.
