<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-piece-text-editor

Edit styled text with the platform’s native editor inside a Day app.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

It binds the document and selection to reactive state. Formatting support differs by toolkit; the
reference lists those differences. Use Day’s text-area component for plain text.

## Use

Enable `text-editor` on your existing [`day`](https://github.com/daybrite/day/tree/main/crates/day)
dependency. Day forwards the selected toolkit feature to this component. The setup guide also covers
direct use by extension crates.

## Documentation

- [Setup and component reference](https://daybrite.dev/docs/reference/texteditor/)
- [Plain-text editing](https://daybrite.dev/docs/reference/textarea/)
- [Compose an interface](https://daybrite.dev/docs/explanation/pieces/)
- [Rust API](https://docs.rs/day-piece-text-editor/latest/day_piece_text_editor/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-piece-text-editor) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
