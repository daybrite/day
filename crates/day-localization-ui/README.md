<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-localization-ui

Reactive localized text for Day’s UI components.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day`](https://github.com/daybrite/day/tree/main/crates/day) and
> [`day-piece-settings`](https://github.com/daybrite/day/tree/main/crates/day-piece-settings) use it
> as part of their implementation.

It connects the message engine in
[`day-localization`](https://github.com/daybrite/day/tree/main/crates/day-localization) to UI
bindings, so changes to the locale or message arguments update visible text. App code uses the
localization support exposed by [`day`](https://github.com/daybrite/day/tree/main/crates/day) and
generated string accessors.

## Documentation

- [Localize an app](https://daybrite.dev/docs/how-to/localization/)
- [Localization reference](https://daybrite.dev/docs/reference/localization/)
- [Rust API](https://daybrite.dev/api/day_localization_ui/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-localization-ui) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
