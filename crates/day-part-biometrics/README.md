<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-part-biometrics

Ask the operating system to verify the person using your app.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

It presents the system authentication prompt and reports the outcome. Availability depends on device
hardware, enrollment, policy, and permissions. Local device authentication does not sign a user in
to an account service.

## Use

Add this entry to your app’s `Cargo.toml`, at the same Day version as the rest of the app:

```toml
[dependencies]
day-part-biometrics = "0.6"
```

## Documentation

- [Usage and platform limits](https://daybrite.dev/docs/reference/biometrics/)
- [Permission setup](https://daybrite.dev/docs/reference/permissions/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://daybrite.dev/api/day_part_biometrics/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-part-biometrics) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
