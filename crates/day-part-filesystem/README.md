<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-part-filesystem

Store files in an app-local data directory.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

It provides file reads, writes, listing, and removal. Native targets use a filesystem directory; the
web implementation uses the browser’s Origin Private File System. Browser storage remains subject to
origin permissions and eviction policies.

## Use

Add it to your app:

```sh
cargo add day-part-filesystem
```

Day apps can also enable the `fs` feature of
[`day`](https://github.com/daybrite/day/tree/main/crates/day) and use its re-export.

## Documentation

- [Usage and platform limits](https://daybrite.dev/docs/reference/fs/)
- [Choose app storage](https://daybrite.dev/docs/how-to/guide-storage/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://docs.rs/day-part-filesystem/latest/day_part_filesystem/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-part-filesystem) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
