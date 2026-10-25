<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-part-connectivity

Query the device’s current network connection.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

It reports a connectivity snapshot, including connection type when the platform exposes it. A
connected network does not guarantee that a particular server is reachable; handle request failures
in your networking code.

## Use

Add this entry to your app’s `Cargo.toml`, at the same Day version as the rest of the app:

```toml
[dependencies]
day-part-connectivity = "0.6"
```

## Documentation

- [Usage and platform limits](https://daybrite.dev/docs/reference/network/)
- [HTTP requests](https://daybrite.dev/docs/reference/http/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://daybrite.dev/api/day_part_connectivity/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-part-connectivity) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
