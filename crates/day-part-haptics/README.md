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

Add it to your app:

```sh
cargo add day-part-haptics
```

## Documentation

- [Usage and platform limits](https://daybrite.dev/docs/reference/haptics/)
- [Pair feedback with sound](https://daybrite.dev/docs/reference/sound/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://docs.rs/day-part-haptics/latest/day_part_haptics/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-part-haptics) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
