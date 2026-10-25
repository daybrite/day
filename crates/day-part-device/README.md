<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-part-device

Read device information and battery state through a shared Rust API.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

Device identity and battery information are separate snapshots. The fields available depend on the
operating system and hardware; callers must handle unavailable information.

## Use

Add this entry to your app’s `Cargo.toml`, using the same Day source as the rest of the app:

```toml
[dependencies]
day-part-device = { git = "https://github.com/daybrite/day.git" }
```

## Documentation

- [Usage and platform limits](https://daybrite.dev/docs/reference/deviceinfo/)
- [Battery information](https://daybrite.dev/docs/reference/battery/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://daybrite.dev/api/day_part_device/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-part-device) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
