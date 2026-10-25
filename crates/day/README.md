<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day

Build apps in Rust with native controls on desktop and mobile, and HTML controls on the web.

This is the main application crate for [Day](https://daybrite.dev/docs/). It brings together UI
components, reactive state, layout, and localization. Your Rust code describes the interface; the
selected toolkit creates its controls and applies state changes.

Start with [`day-cli`](https://github.com/daybrite/day/tree/main/crates/day-cli) to create a project
with its platform hosts and generated resources. Build for one toolkit at a time. Optional Cargo
features add facilities such as file storage, permissions, settings, and rich-text editing; the
guides explain their setup.

## Documentation

- [Start with a Day app](https://daybrite.dev/docs/tutorials/first-app/)
- [UI components and composition](https://daybrite.dev/docs/explanation/pieces/)
- [Supported targets](https://daybrite.dev/docs/reference/platforms/)
- [Rust API](https://daybrite.dev/api/day/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
