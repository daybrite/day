<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-part-keychain

Store small secrets in the operating system’s secure storage.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

Use it for tokens, passwords, and keys that should not live in ordinary preferences. Protection
levels and authentication requirements vary by platform; handle unsupported policies and
authentication failures explicitly.

## Use

Add this entry to your app’s `Cargo.toml`, at the same Day version as the rest of the app:

```toml
[dependencies]
day-part-keychain = "0.6"
```

## Documentation

- [Usage and platform limits](https://daybrite.dev/docs/reference/keychain/)
- [Local authentication](https://daybrite.dev/docs/reference/biometrics/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://daybrite.dev/api/day_part_keychain/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-part-keychain) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
