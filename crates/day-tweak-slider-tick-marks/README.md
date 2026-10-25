<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-tweak-slider-tick-marks

Add native tick marks and optional snapping to Day sliders.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

Count, position, and snapping behavior vary by toolkit. AppKit, GTK, Material, Qt, WinUI, and ArkUI
provide different subsets; UIKit and Web leave the component unchanged. Consult the compatibility
table before relying on a particular treatment.

## Use

Add this crate to the app’s dependencies and forward the matching toolkit feature as described in
the tweak guide:

```toml
[dependencies]
day-tweak-slider-tick-marks = "0.6"
```

## Documentation

- [Use and configure tweaks](https://daybrite.dev/docs/how-to/tweaks/)
- [Tweak contracts and examples](https://daybrite.dev/docs/reference/tweaks/)
- [Control examples](https://daybrite.dev/docs/reference/api-tour/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-tweak-slider-tick-marks/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-tweak-slider-tick-marks) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
