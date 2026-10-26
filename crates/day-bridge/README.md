<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-bridge

Connect a Rust API to native-language implementations in a Day extension.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

This is the runtime support for daybridge, Day’s native-language binding system. It provides
declarations, shared error and support types, and asynchronous completion handling.
[`day-build`](https://github.com/daybrite/day/tree/main/crates/day-build) generates the adapters at
build time.

Use this crate when writing a platform service or extension. Applications usually call the finished
service crate instead of defining a bridge.

## Documentation

- [Native-language bindings](https://github.com/daybrite/day/blob/main/docs/bridge.md/)
- [Build a platform service](https://daybrite.dev/contributing/tutorials/part/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://daybrite.dev/api/day_bridge/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-bridge) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
