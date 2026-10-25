<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-platform-macos

macOS window integration used by Day toolkits.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day-toolkit-qt`](https://github.com/daybrite/day/tree/main/crates/day-toolkit-qt) uses it as
> part of its implementation.

The current module manages native window tabs, including tab grouping and the standard window-tab
actions. [`day-toolkit-qt`](https://github.com/daybrite/day/tree/main/crates/day-toolkit-qt) uses it
to integrate Qt windows with macOS tab behavior. It is not a general collection of Apple platform
APIs.

## Documentation

- [Windows and desktop UI](https://daybrite.dev/docs/how-to/guide-desktop/)
- [macOS platform guide](https://daybrite.dev/docs/reference/platforms/macos-appkit/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-platform-macos/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-platform-macos) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
