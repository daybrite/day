---
title: Logging
description: "Day emits through the `log` facade: levels, per-platform sinks, DAY_LOG, and how to install env_logger, tracing, or your own logger."
order: 25
section: Guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Logging

Day logs through [`log`](https://docs.rs/log), the Rust ecosystem's logging facade. The macros come
from the prelude, and there is nothing to initialize:

```rust
use day::prelude::*;

info!("importing {} rows", rows.len());
warn!("font {name:?} is missing — falling back to the system font");
error!("the database is unreadable: {e}");
debug!("hit test at {x},{y} -> {hit:?}");
```

`day::launch` installs a logger before your first line can run, so your output and the framework's
appear together, in one format, on every platform.

## Don't use `println!`

It is silent on the web. Rust's standard library has no stdout on `wasm32-unknown-unknown`: its
implementation takes your bytes and discards them, with no error and no panic. An app that
diagnoses itself with `println!` says nothing at all in a browser.

On native targets `println!` **panics** if the write fails, and a closed pipe is ordinary when
the launcher shuts an app down. Raised inside a platform callback, that
panic aborts the process. Day's logger never panics on a failed write.

## Where output goes

| Platform | Sink |
|---|---|
| macOS, Linux, Windows | stderr — your terminal |
| iOS | stderr — the Xcode console |
| Android | logcat |
| Web | the browser's JavaScript console |

In a browser each level maps to the matching console method, so devtools' level filter works on
Day's output the way it does on your page's. Lines read `LEVEL target: message`, where the target
names the crate and module that emitted them:

```
INFO  my_app: importing 412 rows
WARN  day_core::nav: .restore("app.section") has no NavStore installed — …
```

## Color

On a terminal the level column is colored (red `ERROR`, yellow `WARN`, green `INFO`, blue
`DEBUG`, cyan `TRACE`), the same palette `env_logger` uses. The message stays plain, so it stays
greppable.

Day checks whether the destination is a color terminal, so the escapes drop out on their own when
you redirect to a file, when `NO_COLOR` is set, and on the targets with no terminal (logcat,
Xcode's console, the browser). Under `day launch` the app's stderr is a pipe, so
`day launch` does the coloring instead, tagging each line with the target it came from:

```
[macos-appkit] ERROR my_app: the database is unreadable
[macos-appkit] INFO  my_app: importing 412 rows
```

The VS Code extension's **Run** button shows exactly this, since it runs `day launch` as a task in
an integrated terminal. Its F5 debug path forwards the output to the Debug Console through a pipe
instead, where the color is stripped and the text arrives plain.

## Levels

`error!` when a feature is gone. `warn!` when Day recovered but not as intended (a missing font, a
piece with no renderer on this backend). `info!` for milestones worth having in a report. `debug!`
for the trace you want while working on something. `trace!` for a per-frame firehose.

Debug builds show `debug!` and above; release builds show `info!` and above. Turn it up with
`DAY_LOG`:

```sh
DAY_LOG=debug day launch -p macos-appkit
day launch -p web-dom --env DAY_LOG=debug     # the web has no environment; this rides the URL
```

## Bring your own logger

You can replace Day's logger. `log` allows one logger per process and the first one wins, so
install yours before `day::launch`:

```rust
fn main() {
    env_logger::init();          // or tracing_subscriber, or your own log::Log
    day::launch(day::WindowOptions::default(), my_app::root);
}
```

That is the entire opt-out. There is no feature to set, and every `info!` you have already
written keeps working, because those are `log`'s macros. You gain whatever that logger offers,
such as `env_logger`'s `RUST_LOG=warn,day_uikit=debug` per-target filtering or `tracing`'s spans
via `tracing-log`.

`env_logger` is a good choice for a desktop-focused app, and it is opt-in in Day because it writes
ANSI text to stderr on every target: stderr is logcat's **ERROR** level on Android, a discarded
buffer on the web, and the Xcode console on iOS. Day's own logger exists to route per platform.
`RUST_LOG` replaces `DAY_LOG` once you install it, because `DAY_LOG` belongs to the logger you just
displaced.

Full reference: [docs/logging.md](https://github.com/daybrite/day/blob/main/docs/logging.md).
