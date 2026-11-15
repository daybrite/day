<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-process

What a Day process has before, and without, a user interface.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

> [!NOTE]
> **Usually an indirect dependency.** App code reaches these through
> [`day`](https://github.com/daybrite/day/tree/main/crates/day). A part that runs without a window,
> such as a background worker or a push handler, depends on this crate directly.

It supplies the app's id and its data, cache and configuration directories; the registry of services
that start with the process; native notification delivery; and the guard that keeps a Rust panic
from unwinding into foreign code. It depends on no toolkit, widget or reactive code.

## Documentation

- [Where files live](https://daybrite.dev/docs/how-to/guide-storage/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://docs.rs/day-process/latest/day_process/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-process) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0](https://github.com/daybrite/day/blob/main/LICENSE)
