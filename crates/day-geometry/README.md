<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-geometry

Geometry value types shared by Day’s layout, drawing, and toolkit APIs.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day-core`](https://github.com/daybrite/day/tree/main/crates/day-core) and
> [`day-spec`](https://github.com/daybrite/day/tree/main/crates/day-spec) use it as part of their
> implementation.

Points, sizes, rectangles, and related values use density-independent coordinates. Toolkits convert
these values to device pixels. App code normally obtains the shared types through
[`day`](https://github.com/daybrite/day/tree/main/crates/day).

## Documentation

- [Layout and sizing](https://daybrite.dev/docs/explanation/layout/)
- [Canvas drawing](https://daybrite.dev/docs/reference/canvas/)
- [Rust API](https://docs.rs/day-geometry/latest/day_geometry/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-geometry) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
