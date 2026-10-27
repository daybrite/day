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

Add it to your app:

```sh
cargo add day-part-downloads
```

## Documentation

- [Usage and platform limits](https://daybrite.dev/docs/reference/downloads/)
- [HTTP networking](https://daybrite.dev/docs/reference/http/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://docs.rs/day-part-downloads/latest/day_part_downloads/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-part-downloads) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
