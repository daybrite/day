<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-part-downloads

Manage file downloads with progress, pause, resume, and integrity checks.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

It builds on [`day-part-http`](https://github.com/daybrite/day/tree/main/crates/day-part-http) for
transfers and offers platform download-service integration where available. Resume behavior depends
on the server, and background transfer support depends on the operating system.

## Use

Add this entry to your app’s `Cargo.toml`, using the same Day source as the rest of the app:

```toml
[dependencies]
day-part-downloads = { git = "https://github.com/daybrite/day.git" }
```

## Documentation

- [Usage and platform limits](https://daybrite.dev/docs/reference/downloads/)
- [HTTP networking](https://daybrite.dev/docs/reference/http/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://daybrite.dev/api/day_part_downloads/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-part-downloads) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
