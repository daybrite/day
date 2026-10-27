<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-localization

The Fluent-based localization engine used by Day apps and framework components.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day-core`](https://github.com/daybrite/day/tree/main/crates/day-core) and
> [`day-localization-ui`](https://github.com/daybrite/day/tree/main/crates/day-localization-ui) use
> it as part of their implementation.

It manages locale selection and message catalogs, including separate catalogs for reusable
extensions. App code should use generated string accessors;
[`day-localization-ui`](https://github.com/daybrite/day/tree/main/crates/day-localization-ui)
connects localized text to UI updates.

## Documentation

- [Localize an app](https://daybrite.dev/docs/how-to/localization/)
- [Localization reference](https://daybrite.dev/docs/reference/localization/)
- [Rust API](https://docs.rs/day-localization/latest/day_localization/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-localization) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
