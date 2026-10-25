<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-part-clipboard

Copy and paste text and encoded data through the system clipboard.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

It supports multiple representations of an item, such as text and image bytes. Clipboard access
follows platform privacy and focus rules; check the result of a read rather than assuming another
app’s data is available.

## Use

Add this entry to your app’s `Cargo.toml`, at the same Day version as the rest of the app:

```toml
[dependencies]
day-part-clipboard = "0.6"
```

## Documentation

- [Usage and platform limits](https://daybrite.dev/docs/reference/clipboard/)
- [Platform capabilities](https://daybrite.dev/docs/reference/platforms/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://daybrite.dev/api/day_part_clipboard/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-part-clipboard) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
