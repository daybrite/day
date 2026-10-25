<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-part-wakelock

Keep the display awake while your app presents an activity that needs it.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

Locks are scoped handles: releasing the last handle allows normal display sleep again. Acquire and
release them on the UI thread. Linux currently has no implementation; browser support depends on the
Screen Wake Lock API and page visibility.

## Use

Add this entry to your app’s `Cargo.toml`, using the same Day source as the rest of the app:

```toml
[dependencies]
day-part-wakelock = { git = "https://github.com/daybrite/day.git" }
```

## Documentation

- [Usage and platform limits](https://daybrite.dev/docs/reference/wakelock/)
- [Platform capabilities](https://daybrite.dev/docs/reference/platforms/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://daybrite.dev/api/day_part_wakelock/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-part-wakelock) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
