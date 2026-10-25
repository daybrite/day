<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-script-protocol

Shared request, response, and YAML document types for Day’s UI automation.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** You are unlikely to import this crate directly in an app.
> [`day-cli`](https://github.com/daybrite/day/tree/main/crates/day-cli) and
> [`day-script`](https://github.com/daybrite/day/tree/main/crates/day-script) use it as part of
> their implementation.

The CLI and embedded engine use these types to exchange dayscript steps and results. Direct use is
intended for tools that read, validate, or communicate using that protocol. For application testing,
start with the dayscript guide.

## Documentation

- [Dayscript testing](https://daybrite.dev/docs/how-to/dayscript/)
- [Automation tools](https://daybrite.dev/docs/reference/for-agents/)
- [Rust API](https://daybrite.dev/api/day_script_protocol/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-script-protocol) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
