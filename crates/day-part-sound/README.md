<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-part-sound

Play short bundled sound effects, once or in a loop.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

It uses platform audio facilities and supports volume control and preloading. Refer to bundled clips
through generated asset constants. Unsupported playback stays silent; this crate is intended for
sound effects rather than a media-player interface.

## Use

Add this entry to your app’s `Cargo.toml`, using the same Day source as the rest of the app:

```toml
[dependencies]
day-part-sound = { git = "https://github.com/daybrite/day.git" }
```

## Documentation

- [Usage and platform limits](https://daybrite.dev/docs/reference/sound/)
- [Bundle audio assets](https://daybrite.dev/docs/how-to/resources/)
- [Using platform services](https://daybrite.dev/docs/reference/parts/)
- [Rust API](https://daybrite.dev/api/day_part_sound/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-part-sound) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
