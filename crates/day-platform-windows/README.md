<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-platform-windows

Windows operating-system services shared across Day’s UI toolkits.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day-toolkit-winui`](https://github.com/daybrite/day/tree/main/crates/day-toolkit-winui),
> [`day-toolkit-gtk`](https://github.com/daybrite/day/tree/main/crates/day-toolkit-gtk), and
> [`day-toolkit-qt`](https://github.com/daybrite/day/tree/main/crates/day-toolkit-qt) use it as part
> of their implementation.

The current implementation provides the native Windows share UI. It is independent of the WinUI
control implementation, so GTK and Qt can use the same service. Calls that present UI run on the
calling UI thread.

## Documentation

- [Windows setup](https://daybrite.dev/docs/reference/platforms/windows-winui/)
- [Sharing](https://daybrite.dev/docs/reference/sharing/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-platform-windows/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-platform-windows) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
