<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-toolchain

SDK and host-tool discovery shared by Day’s CLI and native build scripts.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day-cli`](https://github.com/daybrite/day/tree/main/crates/day-cli) and
> [`day-toolkit-xaml-sys`](https://github.com/daybrite/day/tree/main/crates/day-toolkit-xaml-sys)
> use it as part of their implementation.

It locates compilers, platform SDKs, and related tools, with environment overrides for installations
outside the default locations. Application developers normally let the CLI configure their builds;
extension authors use this crate when compiling native code.

## Documentation

- [Build requirements](https://daybrite.dev/docs/reference/system-requirements/)
- [Environment configuration](https://daybrite.dev/docs/reference/environment/)
- [Writing Day extensions](https://daybrite.dev/contributing/extending/)
- [Rust API](https://daybrite.dev/api/day_toolchain/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-toolchain) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
