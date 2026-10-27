<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-piece-settings

Build an app’s Settings and About screens from shared declarations.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

It connects typed preferences to stored values and organizes controls into groups and sections. The
selected toolkit determines how the settings surface is presented. Use generated localized text for
titles, labels, and explanations.

## Use

Enable `settings` on your existing [`day`](https://github.com/daybrite/day/tree/main/crates/day)
dependency. Day forwards the selected toolkit feature to this component. The setup guide also covers
direct use by extension crates.

## Documentation

- [Setup and component reference](https://daybrite.dev/docs/reference/settings/)
- [Persistent preferences](https://daybrite.dev/docs/reference/prefs/)
- [Compose an interface](https://daybrite.dev/docs/explanation/pieces/)
- [Rust API](https://docs.rs/day-piece-settings/latest/day_piece_settings/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-piece-settings) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
