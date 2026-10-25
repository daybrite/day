<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-part-local-notify

Post or schedule local notifications through the operating system.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

It supplies notification content, channels, and triggers. Declare the required permissions and check
platform support before scheduling; notification delivery remains subject to operating-system
policy.

## Use

Add this entry to your app’s `Cargo.toml`, at the same Day version as the rest of the app:

```toml
[dependencies]
day-part-local-notify = "0.6"
```

## Documentation

- [Usage and platform limits](https://daybrite.dev/docs/reference/notify/)
- [Notification setup](https://daybrite.dev/docs/how-to/guide-notifications/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://daybrite.dev/api/day_part_local_notify/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-part-local-notify) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
