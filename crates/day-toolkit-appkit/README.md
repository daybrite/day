<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-toolkit-appkit

The AppKit implementation of Day’s UI on macOS.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day`](https://github.com/daybrite/day/tree/main/crates/day) uses it as part of its
> implementation.

It creates AppKit controls and connects native events, layout, and window behavior to Day. Build on
macOS with the Apple tools described in the platform guide.

## Documentation

- [Platform setup and limits](https://daybrite.dev/docs/reference/platforms/macos-appkit/)
- [Choose a target](https://daybrite.dev/docs/reference/platforms/)
- [Writing Day extensions](https://daybrite.dev/contributing/extending/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-toolkit-appkit/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-toolkit-appkit) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
