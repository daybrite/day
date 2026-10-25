<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-toolkit-qt

The Qt 6 Widgets implementation of Day’s UI on desktop systems.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day`](https://github.com/daybrite/day/tree/main/crates/day) uses it as part of its
> implementation.

It creates Qt Widgets through the C++ bridge in
[`day-toolkit-qt-sys`](https://github.com/daybrite/day/tree/main/crates/day-toolkit-qt-sys). Qt
development libraries and a compatible C++ compiler are required. The platform guide covers setup
and control differences.

## Documentation

- [Platform setup and limits](https://daybrite.dev/docs/reference/platforms/linux-qt/)
- [Choose a target](https://daybrite.dev/docs/reference/platforms/)
- [Writing Day extensions](https://daybrite.dev/contributing/extending/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-toolkit-qt/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-toolkit-qt) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
