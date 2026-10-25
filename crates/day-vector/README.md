<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-vector

SVG processing and platform icon generation used by Day’s tooling.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day-cli`](https://github.com/daybrite/day/tree/main/crates/day-cli) uses it as part of its
> implementation.

It parses and rasterizes vector artwork and writes formats such as Android VectorDrawable, Windows
icons, and macOS icons. App developers normally bundle vector assets and generate app icons through
the CLI. Direct use is useful for tools that need the underlying conversion functions.

## Documentation

- [Vector resources](https://daybrite.dev/docs/reference/vectors/)
- [Create app icons](https://daybrite.dev/docs/how-to/icons/)
- [Rust API](https://daybrite.dev/api/day_vector/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-vector) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
