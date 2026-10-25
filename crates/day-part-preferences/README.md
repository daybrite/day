<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-part-preferences

Keep small string values between app launches.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

It stores settings such as a selected theme or recent choice. Use secure storage for secrets and
file storage for larger data. The `memory` feature selects a nonpersistent store for tests.

## Use

Day apps normally use the default-enabled `prefs` feature of
[`day`](https://github.com/daybrite/day/tree/main/crates/day); a separate dependency is optional.
For direct use:

```toml
[dependencies]
day-part-preferences = "0.6"
```

## Documentation

- [Usage and platform limits](https://daybrite.dev/docs/reference/prefs/)
- [Choose app storage](https://daybrite.dev/docs/how-to/guide-storage/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://daybrite.dev/api/day_part_preferences/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-part-preferences) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
