<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-core

The runtime that turns Day’s UI descriptions into a mounted interface.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day`](https://github.com/daybrite/day/tree/main/crates/day) and
> [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces) use it as part of
> their implementation.

It manages component lifetimes, layout, event routing, and the bindings that update native controls
when state changes. Extension authors use it to implement components and connect them to the
rendering tree.

## Documentation

- [How pieces work](https://daybrite.dev/docs/explanation/pieces/)
- [Layout](https://daybrite.dev/docs/explanation/layout/)
- [Framework architecture](https://daybrite.dev/contributing/architecture/)
- [Rust API](https://docs.rs/day-core/latest/day_core/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-core) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
