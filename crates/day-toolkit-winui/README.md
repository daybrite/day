<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-toolkit-winui

The WinUI 3 implementation of Day’s UI on Windows.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day`](https://github.com/daybrite/day/tree/main/crates/day) uses it as part of its
> implementation.

It hosts WinUI controls through the C++/WinRT bridge in
[`day-toolkit-xaml-sys`](https://github.com/daybrite/day/tree/main/crates/day-toolkit-xaml-sys).
Shared operating-system services come from
[`day-platform-windows`](https://github.com/daybrite/day/tree/main/crates/day-platform-windows). Use
the `windows-winui` target and its Windows App SDK requirements.

## Documentation

- [Platform setup and limits](https://daybrite.dev/docs/reference/platforms/windows-winui/)
- [Choose a target](https://daybrite.dev/docs/reference/platforms/)
- [Writing Day extensions](https://daybrite.dev/contributing/extending/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-toolkit-winui/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-toolkit-winui) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
