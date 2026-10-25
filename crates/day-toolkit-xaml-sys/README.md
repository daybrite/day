<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-toolkit-xaml-sys

Low-level Rust bindings and a C++/WinRT bridge for Day’s Windows UI implementation.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day-toolkit-winui`](https://github.com/daybrite/day/tree/main/crates/day-toolkit-winui),
> [`day-piece-text-editor`](https://github.com/daybrite/day/tree/main/crates/day-piece-text-editor),
> and [`day-piece-settings`](https://github.com/daybrite/day/tree/main/crates/day-piece-settings)
> use it as part of their implementation.

The `winui` feature selects the Windows App SDK’s WinUI controls. The bridge manages native handles,
callbacks, and control operations for the Rust toolkit and optional UI components. Use the
`windows-winui` target so Day selects the matching build configuration.

## Documentation

- [WinUI build requirements](https://daybrite.dev/docs/reference/platforms/windows-winui/)
- [Framework architecture](https://daybrite.dev/contributing/architecture/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-toolkit-xaml-sys/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-toolkit-xaml-sys) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
