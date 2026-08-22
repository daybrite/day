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

## Color

On a terminal the level column is colored — red `ERROR`, yellow `WARN`, green `INFO`, blue
`DEBUG`, cyan `TRACE`. That is `env_logger`'s palette rather than a new one, so the colors carry
the meaning they already have elsewhere in Rust. The message stays plain: color across a whole
line is noise at `INFO`, and an uncolored message stays greppable.

The escapes are written only where they render. Day checks whether the destination is a color
terminal instead of assuming, so color drops out on its own when output is redirected to a file,
when `NO_COLOR` or `TERM=dumb` is set, and on the targets where no terminal is involved — logcat,
Xcode's console, the browser. On Windows it enables the console's VT processing first, so older
consoles show color rather than raw escapes.

Under `day launch` the app's stderr is a pipe, so the app itself writes none. **`day-cli` colors
the lines as it re-emits them**, tagged with the target they came from:

```
[macos-appkit] ERROR my_app: the database is unreadable
[macos-appkit] INFO  my_app: importing 412 rows
[macos-appkit] DEBUG day_core::nav: restoring app.section
```

The prefix takes the level's color too, so scanning the left column finds the errors.

The VS Code extension's **Run** button shows exactly this, and needed no change of its own to get
it: it runs `day launch` as a VS Code task in an integrated terminal, which is a terminal like any
other. Its **debug** path (F5) is different — it spawns `day launch` with pipes and forwards the
text to the Debug Console, so the color is stripped there and the output is plain.

Note that `day.extraEnv` will not change that. Those entries are passed through as `day launch
--env`, which sets the environment of **the app**, not of the CLI doing the coloring.

A forwarded line with no level keeps the older per-stream color, blue for stdout and yellow for
stderr: a stray `println!`, a Qt warning, a raw logcat line. Colored output is a presentation
choice, so it never changes routing — an `ERROR` an app wrote to stdout is still forwarded to
stdout.

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
    day::launch(day::WindowOptions::default(), my_app::root);
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

### `env_logger`, worked

`env_logger` is the one to reach for on a desktop-focused app, and it is a two-line change:

```toml
# Cargo.toml
[dependencies]
env_logger = "0.11"
```

```rust
fn main() {
    // Before `day::launch`, or Day's logger wins the race and this call does nothing.
    env_logger::init();
    day::launch(day::WindowOptions::default(), my_app::root);
}
```

The per-target filter is the real reason to switch — one backend turned up without the rest of the
app coming with it:

```sh
RUST_LOG=warn,day_gtk=debug,my_app=trace day launch -p linux-gtk
```

`RUST_LOG` replaces `DAY_LOG` once you do this. `DAY_LOG` is read by Day's own logger, and that
logger is no longer installed.

Day does **not** adopt `env_logger` as its default, because it writes ANSI
text to stderr on every target. On Android that means every line lands in logcat at **ERROR**,
because day-android maps fd 2 to that level; on web-dom it means the lines are discarded, since
std's stdio on wasm accepts bytes and drops them; on iOS it means escape codes in the Xcode
console. Day's default exists to route per platform. `env_logger` is the right choice when your
app targets desktops, and the wrong one to impose on the other six targets.

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
