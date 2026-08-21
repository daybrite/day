---
title: "Logging"
description: "Day emits through the `log` facade: levels, per-platform sinks, DAY_LOG, and how an app installs env_logger, tracing, or its own log::Log."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Logging

Day emits through [`log`](https://docs.rs/log), the ecosystem's logging facade. Write a line with
the macros the prelude already gives you, and it comes out on every platform with no setup:

```rust
use day::prelude::*;

info!("importing {} rows", rows.len());
warn!("font {name:?} is missing — falling back to the system font");
error!("the database is unreadable: {e}");
debug!("hit test at {x},{y} -> {hit:?}");
trace!("frame {n}");
```

There is nothing to initialize. `day::launch` installs a logger before the first line can be
emitted, so the framework's own diagnostics and yours land in the same place, in the same format.

## Why not `println!`

Because it does not work everywhere, and where it fails it fails **silently**.

On `wasm32-unknown-unknown` — the target `web-dom` builds — Rust's standard library has no
stdout to write to. Its implementation accepts your bytes and drops them:

```rust
// library/std/src/sys/stdio/unsupported.rs
fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
    Ok(buf.len())          // accepted, then discarded
}
```

No error, no panic, no output. An app whose diagnostics are `println!` is simply mute in a
browser, and there is no fd to redirect the way Android redirects fd 1 and 2 into logcat.

`println!` has a second problem even on native targets: it **panics** when the write fails, and a
closed stderr pipe is routine when `day launch` tears an app down. A panic raised inside a native
trampoline — an event callback, a GCD or glib block — unwinds into non-Rust frames and aborts the
process, turning a clean exit into a spurious crash. Day's logger never panics on a failed write.

## Where the lines go

| Target | Sink |
|---|---|
| macOS, Linux, Windows | the process's stderr — your terminal |
| iOS | stderr, which is the Xcode console |
| Android | stderr, which day-android redirects into **logcat** (stdout at INFO, stderr at ERROR) |
| **web-dom** | the browser's **JavaScript console** |

On the web each level maps to the matching console method — `error!` to `console.error`, `warn!`
to `console.warn`, `info!` to `console.info`, `debug!` and `trace!` to `console.debug` — so
devtools' own level filter applies to Day's output the way it does to the page's.

Lines are formatted `LEVEL target: message`:

```
INFO  my_app: importing 412 rows
WARN  day_core::nav: .restore("app.section") has no NavStore installed — …
```

The target names the crate and module that emitted the line, which is what tells you whether a
complaint is yours, a piece's, or a backend's.

## Choosing a level

| Level | Use it for | On by default |
|---|---|---|
| `error!` | a feature is gone: the window could not be created, the engine never bound | yes |
| `warn!` | Day recovered, but not as intended: a missing font, a piece with no renderer on this backend, a placeholder substituted | yes |
| `info!` | milestones a user or operator would want in a report: startup, a completed import | yes |
| `debug!` | the trace you want while working on something — a malformed field, a layout decision | debug builds |
| `trace!` | per-frame or per-event firehose | no |

The default maximum level is `Debug` in a debug build and `Info` in a release build.

## Turning levels up

`DAY_LOG` takes a level name — `off`, `error`, `warn`, `info`, `debug`, `trace`:

```sh
DAY_LOG=debug day launch -p macos-appkit
```

The web has no process environment, so the launch server forwards it as a query parameter on the
page URL; `day launch -p web-dom --env DAY_LOG=debug` is the same thing. An app can also move the
level at runtime with `day::set_log_level(log::LevelFilter::Trace)`.

## Using a different logger

Day's logger is a default, not a policy. `log` allows exactly one logger per process and the first
registration wins, so **install yours before `day::launch`** and Day will step aside:

```rust
fn main() {
    env_logger::init();                       // or tracing_subscriber, or your own log::Log
    day::launch(day::WindowOptions { .. }, my_app::root);
}
```

That is the whole customization story. Day calls `log::set_logger` and ignores the `Err` that says
someone got there first — no feature flag to set, no call to opt out of, and every `info!` already
written keeps working because they are `log`'s macros, not Day's.

Two consequences worth knowing:

- **Per-target filtering comes with the logger you choose.** Day's default has a single global
  level; `env_logger` gives you `RUST_LOG=warn,day_uikit=debug` and the ability to silence one
  noisy backend.
- **`tracing` users** can bridge with `tracing-log` and receive Day's output as tracing events.

## Writing a logger

The trait is `log::Log`. Day's own web-dom implementation is the whole shape:

```rust
pub fn console_sink(level: log::Level, line: &str) {
    unsafe { day_dom_log(level as u32, line.as_ptr(), line.len()) };
}
```

A backend that needs its own destination installs a **sink** — a `fn(log::Level, &str)` that
receives the already-formatted line — through `day_core::set_log_sink`, rather than replacing the
logger. That keeps the format and the level filtering in one place. The facade wires web-dom's in
`day::web::start`.

Day does not ship `console_log` or `wasm-logger` for the browser, though both exist: each requires
`web-sys` (and `wasm-logger` `wasm-bindgen`), which the web-dom backend deliberately does without
— its whole shim is numeric ids across `extern "C"`, with no bundler and no npm
([docs/web.md](web.md)). Routing a formatted line to `console.*` is one call, so the dependency
would buy nothing and cost the toolchain.

## What Day itself logs

Every framework diagnostic goes through the same macros, so `DAY_LOG=debug` shows you Day's
reasoning alongside your own: a piece with no renderer on the current backend, a route that did
not match, a bundled font the toolkit refused, a nav stack whose bar actions were dropped because
it merged into an enclosing one. Those were `eprintln!` before and are `warn!` now, which means
they are filterable, they carry the emitting module, and they reach the browser console.
