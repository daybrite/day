<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-toolkit-qt-sys

Low-level Rust bindings and a C++ bridge for Day’s Qt 6 Widgets implementation.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day-toolkit-qt`](https://github.com/daybrite/day/tree/main/crates/day-toolkit-qt) uses it as
> part of its implementation.

It compiles the native bridge and exposes C-compatible operations over Qt widget handles. Native
ownership and callback rules are part of the toolkit implementation. Build it with the Qt SDK and
C++ toolchain required by the selected Day target.

## Documentation

- [Qt build requirements](https://daybrite.dev/docs/reference/platforms/linux-qt/)
- [Framework architecture](https://daybrite.dev/contributing/architecture/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-toolkit-qt-sys/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-toolkit-qt-sys) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
