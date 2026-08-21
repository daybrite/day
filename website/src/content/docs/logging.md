---
title: Logging
description: "Day emits through the `log` facade — levels, per-platform sinks, DAY_LOG, and how to install env_logger, tracing, or your own logger."
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

On native targets it has a sharper edge — `println!` **panics** if the write fails, and a closed
pipe is ordinary when the launcher shuts an app down. Raised inside a platform callback, that
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

## Levels

`error!` when a feature is gone. `warn!` when Day recovered but not as intended — a missing font, a
piece with no renderer on this backend. `info!` for milestones worth having in a report. `debug!`
for the trace you want while working on something. `trace!` for a per-frame firehose.

Debug builds show `debug!` and above; release builds show `info!` and above. Turn it up with
`DAY_LOG`:

```sh
DAY_LOG=debug day launch -p macos-appkit
day launch -p web-dom --env DAY_LOG=debug     # the web has no environment; this rides the URL
```

## Bring your own logger

Day's logger is a default, not a policy. `log` allows one logger per process and the first one
wins, so install yours before `day::launch`:

```rust
fn main() {
    env_logger::init();          // or tracing_subscriber, or your own log::Log
    day::launch(day::WindowOptions { .. }, my_app::root);
}
```

That is the entire opt-out: no feature to set, and every `info!` you have already written keeps
working, because those are `log`'s macros rather than Day's. You gain whatever that logger offers —
`env_logger`'s `RUST_LOG=warn,day_uikit=debug` per-target filtering, for instance, or `tracing`'s
spans via `tracing-log`.

Full reference: [docs/logging.md](https://github.com/daybrite/day/blob/main/docs/logging.md).
