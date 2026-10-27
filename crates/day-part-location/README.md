<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-part-location

Read the device’s position once or subscribe to location updates.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

Accuracy choices let an app balance precision and power use. Request location permission before
starting, handle unavailable fixes, and release subscriptions when the information is no longer
needed.

## Use

Add it to your app:

```sh
cargo add day-part-location
```

## Documentation

- [Usage and platform limits](https://daybrite.dev/docs/reference/location/)
- [Request permissions](https://daybrite.dev/docs/reference/permissions/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://docs.rs/day-part-location/latest/day_part_location/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-part-location) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
