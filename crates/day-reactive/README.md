<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-reactive

Reactive state and scoped updates for Day interfaces.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

Signals hold changing values, computed values track their inputs, and effects respond to changes.
Scopes dispose of subscriptions with the UI that owns them. State handles belong to the UI thread; a
setter provides the supported path for sending updates from other threads.

Day applications normally use the re-exports in
[`day`](https://github.com/daybrite/day/tree/main/crates/day). Reusable state libraries can depend
on this crate directly.

## Documentation

- [Reactive state](https://daybrite.dev/docs/explanation/reactivity/)
- [State reference](https://daybrite.dev/docs/explanation/state/)
- [Background work](https://daybrite.dev/docs/explanation/async/)
- [Rust API](https://docs.rs/day-reactive/latest/day_reactive/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-reactive) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
