<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-pieces

Day’s built-in UI components and their builder APIs.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day`](https://github.com/daybrite/day/tree/main/crates/day) and
> [`day-piece-settings`](https://github.com/daybrite/day/tree/main/crates/day-piece-settings) use it
> as part of their implementation.

It provides controls and containers for interfaces such as forms, lists, navigation, and dialogs. A
piece is a description of UI that Day mounts through the selected toolkit. App code normally imports
these components through [`day`](https://github.com/daybrite/day/tree/main/crates/day); reusable
components may depend on this crate directly.

## Documentation

- [Compose an interface](https://daybrite.dev/docs/explanation/pieces/)
- [Component reference](https://daybrite.dev/docs/reference/)
- [Writing Day extensions](https://daybrite.dev/contributing/extending/)
- [Rust API](https://daybrite.dev/api/day_pieces/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-pieces) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
