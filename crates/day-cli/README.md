<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: MPL-2.0
-->

# day-cli

Create, build, run, test, and package Day apps from the command line.

[Day](https://daybrite.dev/docs/) is a Rust framework for building apps with native desktop and
mobile controls, plus HTML controls on the web.

The executable is named `day`. It coordinates Cargo and the selected platform’s build tools,
prepares resources and translations, and runs scripted UI tests. Install the SDKs for your target
before building.

Use `android-material` for Android builds. The tooling still accepts `android-mdc` as a temporary
deprecated alias.

## Use

Install the CLI from the current source, then create a project:

```bash
cargo install --locked --git https://github.com/daybrite/day.git day-cli
day new app hello
cd hello
day doctor
```

Follow the first-app guide to build and launch on your target.

## Documentation

- [Start with a Day app](https://daybrite.dev/docs/tutorials/first-app/)
- [CLI commands](https://daybrite.dev/docs/reference/cli/)
- [Build requirements](https://daybrite.dev/docs/reference/system-requirements/)

[Source](https://github.com/daybrite/day/tree/main/crates/day-cli) ·
[Contributing](https://daybrite.dev/contributing/setup/) · [MPL-2.0
license](https://github.com/daybrite/day/blob/main/LICENSE)
