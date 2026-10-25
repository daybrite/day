<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-platform-linux

Linux desktop integration shared by Day’s GTK and Qt toolkits.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day-toolkit-gtk`](https://github.com/daybrite/day/tree/main/crates/day-toolkit-gtk) and
> [`day-toolkit-qt`](https://github.com/daybrite/day/tree/main/crates/day-toolkit-qt) use it as part
> of their implementation.

Its current implementation covers D-Bus session services for status items, launcher integration, and
single-instance coordination. The available services depend on the desktop session. App code uses
the corresponding high-level Day APIs.

## Documentation

- [Linux with GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk/)
- [Linux with Qt](https://daybrite.dev/docs/reference/platforms/linux-qt/)
- [Desktop app features](https://daybrite.dev/docs/how-to/guide-desktop/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-platform-linux/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-platform-linux) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
