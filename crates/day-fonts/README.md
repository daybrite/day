<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-fonts

Font metadata and bundled-font lookup shared by Day’s build tools and runtime.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day-build`](https://github.com/daybrite/day/tree/main/crates/day-build) and
> [`day-spec`](https://github.com/daybrite/day/tree/main/crates/day-spec) use it as part of their
> implementation.

It reads family and PostScript names from font files and supplies consistent identifiers for staging
and loading them. In an app, bundle fonts through the resource system and use the generated font
accessors.

## Documentation

- [Custom fonts](https://daybrite.dev/docs/reference/fonts/)
- [Bundled resources](https://daybrite.dev/docs/how-to/resources/)
- [Rust API](https://daybrite.dev/api/day_fonts/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-fonts) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
