<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-toolkit-dom

The HTML DOM implementation of Day’s UI on the web.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day`](https://github.com/daybrite/day/tree/main/crates/day) uses it as part of its
> implementation.

It creates HTML elements and connects browser events to Day’s Rust code compiled to WebAssembly. A
JavaScript bridge handles DOM operations. Browser permissions and security policies apply to
platform services.

## Documentation

- [Platform setup and limits](https://daybrite.dev/docs/reference/platforms/web-dom/)
- [Choose a target](https://daybrite.dev/docs/reference/platforms/)
- [Writing Day extensions](https://daybrite.dev/contributing/extending/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-toolkit-dom/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-toolkit-dom) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
