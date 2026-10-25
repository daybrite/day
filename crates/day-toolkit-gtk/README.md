<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-toolkit-gtk

The GTK 4 implementation of Day’s UI on desktop systems.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day`](https://github.com/daybrite/day/tree/main/crates/day) uses it as part of its
> implementation.

It creates GTK 4 widgets and connects their signals and measurements to Day. Install the GTK
development libraries for the selected host; the Linux platform guide covers the standard
`linux-gtk` target.

## Documentation

- [Platform setup and limits](https://daybrite.dev/docs/reference/platforms/linux-gtk/)
- [Choose a target](https://daybrite.dev/docs/reference/platforms/)
- [Writing Day extensions](https://daybrite.dev/contributing/extending/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-toolkit-gtk/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-toolkit-gtk) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
