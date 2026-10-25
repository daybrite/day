<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-async

Shared asynchronous helpers for Day’s runtime and platform services.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day-core`](https://github.com/daybrite/day/tree/main/crates/day-core) and
> [`day-bridge`](https://github.com/daybrite/day/tree/main/crates/day-bridge) use it as part of
> their implementation.

It supplies completion futures, callback-token registries, and a bounded blocking-worker pool. These
helpers connect native callbacks and background work to Day’s UI-thread executor. App code normally
uses the task APIs exposed by [`day`](https://github.com/daybrite/day/tree/main/crates/day).

## Documentation

- [Asynchronous work](https://daybrite.dev/docs/explanation/async/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://daybrite.dev/api/day_async/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-async) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
