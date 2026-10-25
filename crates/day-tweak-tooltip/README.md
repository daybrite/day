<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-tweak-tooltip

Attach native help text to Day components.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

The modifier uses AppKit and GTK hover tooltips and Material long-press tooltips. Other toolkits
leave the component unchanged. Supply generated localized text, and keep essential instructions
visible without relying on a tooltip.

## Use

Add this crate to the app’s dependencies and forward the matching toolkit feature as described in
the tweak guide:

```toml
[dependencies]
day-tweak-tooltip = "0.6"
```

## Documentation

- [Use and configure tweaks](https://daybrite.dev/docs/how-to/tweaks/)
- [Tweak contracts and examples](https://daybrite.dev/docs/reference/tweaks/)
- [Accessibility](https://daybrite.dev/docs/reference/accessibility/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-tweak-tooltip/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-tweak-tooltip) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
