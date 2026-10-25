<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-part-sensors

Subscribe to motion and magnetic-field measurements.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

Readings arrive through callbacks until the subscription ends. Native callbacks run on a background
dispatcher, so send UI updates through a reactive setter. Slow consumers may receive coalesced
readings; this API is for live state rather than lossless recording. macOS and Linux currently
report unavailable.

## Use

Add this entry to your app’s `Cargo.toml`, at the same Day version as the rest of the app:

```toml
[dependencies]
day-part-sensors = "0.6"
```

## Documentation

- [Usage and platform limits](https://daybrite.dev/docs/reference/sensors/)
- [Background work and UI state](https://daybrite.dev/docs/explanation/async/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://daybrite.dev/api/day_part_sensors/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-part-sensors) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
