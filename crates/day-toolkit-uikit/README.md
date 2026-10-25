<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-toolkit-uikit

The UIKit implementation of Day’s UI on iOS.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day`](https://github.com/daybrite/day/tree/main/crates/day) uses it as part of its
> implementation.

It creates UIKit views and connects touch input, navigation, and application lifecycle to Day.
Building and signing iOS apps requires the Apple tools described in the platform guide.

## Documentation

- [Platform setup and limits](https://daybrite.dev/docs/reference/platforms/ios-uikit/)
- [Choose a target](https://daybrite.dev/docs/reference/platforms/)
- [Writing Day extensions](https://daybrite.dev/contributing/extending/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-toolkit-uikit/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-toolkit-uikit) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
