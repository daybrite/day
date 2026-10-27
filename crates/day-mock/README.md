<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-mock

Test Day components with a headless toolkit that records UI operations.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

It uses deterministic measurements and an operation log so tests can inspect layout and state
updates without opening a native window. Component and toolkit authors can use it directly;
applications can select the `mock` feature of
[`day`](https://github.com/daybrite/day/tree/main/crates/day).

Mock tests check Day behavior, not the appearance or input behavior of a native toolkit. Use
dayscript tests on the intended target for those checks.

## Documentation

- [Component testing contracts](https://daybrite.dev/docs/how-to/testing/)
- [Test a running app](https://daybrite.dev/docs/how-to/dayscript/)
- [Writing Day extensions](https://daybrite.dev/contributing/extending/)
- [Rust API](https://docs.rs/day-mock/latest/day_mock/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-mock) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
