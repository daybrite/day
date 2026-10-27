<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-part-permissions

Check and request permission to use protected platform capabilities.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

Runtime requests work alongside the capability declarations and localized explanations in
`Day.toml`. A declaration alone does not grant access; handle denied and unavailable results in the
app.

## Use

Add it to your app:

```sh
cargo add day-part-permissions
```

Day apps can also enable the `permissions` feature of
[`day`](https://github.com/daybrite/day/tree/main/crates/day) and use its re-export.

## Documentation

- [Usage and platform limits](https://daybrite.dev/docs/reference/permissions/)
- [Configure permissions](https://daybrite.dev/docs/how-to/guide-permissions/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://docs.rs/day-part-permissions/latest/day_part_permissions/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-part-permissions) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
