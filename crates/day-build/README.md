<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-build

Generate typed resource and localization accessors for a Day app’s build script.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

It scans the app’s resource catalogs and produces symbolic accessors for strings, images, vectors,
fonts, and assets. It also generates native-language bindings used by Day extensions. Projects
created by [`day-cli`](https://github.com/daybrite/day/tree/main/crates/day-cli) already include the
build-script setup.

## Use

Use it as a build dependency in `Cargo.toml`:

```toml
[build-dependencies]
day-build = "0.6"
```

A `build.rs` entry point:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    day_build::prebuild_project()?;
    Ok(())
}
```

## Documentation

- [Bundled resources](https://daybrite.dev/docs/how-to/resources/)
- [Localization](https://daybrite.dev/docs/how-to/localization/)
- [Project structure](https://daybrite.dev/docs/reference/project-structure/)
- [Rust API](https://daybrite.dev/api/day_build/index.html)

[Source](https://github.com/daybrite/day/tree/main/crates/day-build) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
