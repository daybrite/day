<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-script

The embedded automation engine for Day’s scripted UI tests.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day`](https://github.com/daybrite/day/tree/main/crates/day) uses it as part of its
> implementation.

It executes dayscript steps against a running app, including input, assertions, and screenshots. The
[`day-cli`](https://github.com/daybrite/day/tree/main/crates/day-cli) tool starts test sessions and
communicates with the engine. For application tests, write dayscript files and use the CLI rather
than integrating this engine yourself.

## Documentation

- [Test an app with dayscript](https://daybrite.dev/docs/how-to/dayscript/)
- [Automation and agent tools](https://daybrite.dev/docs/reference/for-agents/)
- [Rust API](https://daybrite.dev/api/day_script/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-script) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
