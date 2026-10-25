<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-toolkit-material

The Material Components implementation of Day’s UI on Android.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day`](https://github.com/daybrite/day/tree/main/crates/day) uses it as part of its
> implementation.

It creates Android View-based Material controls and connects their events to Day through JNI and
Java support code. Shared JVM and context services come from
[`day-platform-android`](https://github.com/daybrite/day/tree/main/crates/day-platform-android). The
CLI accepts `android-mdc` only as a temporary deprecated target alias.

## Documentation

- [Platform setup and limits](https://daybrite.dev/docs/reference/platforms/android-material/)
- [Choose a target](https://daybrite.dev/docs/reference/platforms/)
- [Writing Day extensions](https://daybrite.dev/contributing/extending/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-toolkit-material/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-toolkit-material) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
