<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-spec

Shared contracts between Day’s runtime and its toolkit implementations.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day-core`](https://github.com/daybrite/day/tree/main/crates/day-core) and
> [`day-toolkit-appkit`](https://github.com/daybrite/day/tree/main/crates/day-toolkit-appkit) use it
> as part of their implementation.

It defines toolkit operations, native events, capability reporting, and the value types passed
between the runtime and controls. Direct use is intended for toolkit and extension authors;
applications use the APIs exposed by [`day`](https://github.com/daybrite/day/tree/main/crates/day).

## Documentation

- [Platform capabilities](https://daybrite.dev/docs/reference/platforms/)
- [Framework architecture](https://daybrite.dev/contributing/architecture/)
- [Writing Day extensions](https://daybrite.dev/contributing/extending/)
- [Rust API](https://docs.rs/day-spec/latest/day_spec/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-spec) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
