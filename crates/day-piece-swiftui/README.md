<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-piece-swiftui

Embed custom SwiftUI views in a Day interface on macOS and iOS.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

It hosts a SwiftUI view inside AppKit or UIKit and connects its inputs and events to Rust. Add the
Swift package and generated bindings described in the guide. This component is available only with
the AppKit and UIKit toolkits.

## Use

Enable `swiftui` on your existing [`day`](https://github.com/daybrite/day/tree/main/crates/day)
dependency. Day forwards the selected toolkit feature to this component. The setup guide also covers
direct use by extension crates.

## Documentation

- [Setup and component reference](https://daybrite.dev/docs/reference/swiftui/)
- [SwiftUI integration guide](https://daybrite.dev/docs/how-to/guide-swiftui/)
- [Compose an interface](https://daybrite.dev/docs/explanation/pieces/)
- [Rust API](https://daybrite.dev/api/day_piece_swiftui/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-piece-swiftui) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
