<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-tweak-button-bezel

Apply native AppKit bezel styles to Day buttons.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

This modifier works with AppKit. On other toolkits it leaves the component unchanged. Use it when a
macOS-specific button treatment is needed.

## Use

Add this crate to the app’s dependencies and forward the matching toolkit feature as described in
the tweak guide:

```toml
[dependencies]
day-tweak-button-bezel = "0.6"
```

## Documentation

- [Use and configure tweaks](https://daybrite.dev/docs/how-to/tweaks/)
- [Tweak contracts and examples](https://daybrite.dev/docs/reference/tweaks/)
- [Buttons](https://daybrite.dev/docs/reference/buttons/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-tweak-button-bezel/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-tweak-button-bezel) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
