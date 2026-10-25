<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-toolkit-arkui

The ArkUI implementation of Day’s UI on HarmonyOS.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day`](https://github.com/daybrite/day/tree/main/crates/day) uses it as part of its
> implementation.

It creates native ArkUI nodes and mounts them in the generated HarmonyOS host. The platform guide
covers the SDK, build tools, signing, and component limitations.

## Documentation

- [Platform setup and limits](https://daybrite.dev/docs/reference/platforms/harmony-arkui/)
- [Choose a target](https://daybrite.dev/docs/reference/platforms/)
- [Writing Day extensions](https://daybrite.dev/contributing/extending/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-toolkit-arkui/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-toolkit-arkui) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
