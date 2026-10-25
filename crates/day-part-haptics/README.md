<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-part-haptics

Play tactile feedback for selections, impacts, and action outcomes.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

The platform chooses the available vibration or haptic effect. Check capability before depending on
feedback: unsupported devices leave calls without an effect.

## Use

Add this entry to your app’s `Cargo.toml`, using the same Day source as the rest of the app:

```toml
[dependencies]
day-part-haptics = { git = "https://github.com/daybrite/day.git" }
```

## Documentation

- [Usage and platform limits](https://daybrite.dev/docs/reference/haptics/)
- [Pair feedback with sound](https://daybrite.dev/docs/reference/sound/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://daybrite.dev/api/day_part_haptics/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-part-haptics) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
