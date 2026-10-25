<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-macros

Compile-time macros used by Day’s framework and extension crates.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day`](https://github.com/daybrite/day/tree/main/crates/day) and
> [`day-core`](https://github.com/daybrite/day/tree/main/crates/day-core) use it as part of their
> implementation.

They generate builder forwarding, observable-model accessors, SVG path construction, and
native-language bindings. Use re-exports from the application or extension API when available;
depend on this crate directly when implementing an extension that needs its macros.

## Documentation

- [Observable models](https://daybrite.dev/docs/explanation/model/)
- [Writing Day extensions](https://daybrite.dev/contributing/extending/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-macros/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-macros) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
