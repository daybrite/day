<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-tweak-tree-style

Apply native sidebar and alternating-row treatments to Day trees.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

AppKit supports sidebar styling and alternating rows. GTK supports sidebar styling; other toolkit
combinations leave these options unchanged. These options affect native presentation rather than the
tree’s data or selection.

## Use

Add this crate to the app’s dependencies and forward the matching toolkit feature as described in
the tweak guide:

```sh
cargo add day-tweak-tree-style
```

## Documentation

- [Use and configure tweaks](https://daybrite.dev/docs/how-to/tweaks/)
- [Tweak contracts and examples](https://daybrite.dev/docs/reference/tweaks/)
- [Trees](https://daybrite.dev/docs/reference/tree/)
- [Rust API source](https://github.com/daybrite/day/blob/main/crates/day-tweak-tree-style/src/lib.rs)

[Source](https://github.com/daybrite/day/tree/main/crates/day-tweak-tree-style) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
