---
title: Build flavors
description: Ship one source tree as several apps — a paid build, a white-label build, a demo — with Day-<name>.toml and day build --flavor.
order: 32
section: Build & ship
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

A flavor ships one source tree as more than one app: a paid build beside a free one, a
white-label build per customer, a demo build that goes to fewer platforms. Each flavor is a file
beside `Day.toml`, and it states only what differs.

```toml
# Day-custom.toml — `day build --flavor custom`

# Overlay directories go before the first table: TOML gives a bare key to the table above it.
resources = "resource-custom"
store = "store-custom"

[app]
id = "dev.example.notes.custom"
title = "Notes Custom"
scheme = "notescustom"
targets = ["macos-appkit", "ios-uikit", "android-mdc", "web-dom"]

[app.android]
id = "dev.example.notes_custom"

[cargo]
features = ["custom-branding"]

[env]
NOTES_EDITION = "Custom"
```

```bash
day build  --flavor custom -p ios-uikit
day launch --flavor custom -p macos-appkit
day pack   --flavor custom -p android-mdc
```

`--flavor` works on every command, so `day metadata`, `day lint`, `day store stage` and
`day screenshot` all describe the flavor you name. `DAY_FLAVOR=custom` is the same switch, which
is what a CI job sets for a matrix leg.

## What a flavor can change

- **Identity.** `id`, `title`, `artifact`, `scheme` and `build`, including the
  `[app.<platform>]` override tables [Day.toml](/docs/project-structure) already takes. A
  different `id` installs beside the base app rather than replacing it.
- **Targets.** `targets` replaces the base list, so a flavor can ship on fewer platforms or more.
- **Code.** `[cargo] features` adds cargo features, on top of the backend feature and the ones
  the app's [pieces](/docs/glossary#piece) need.
- **Values.** `[env]` reaches every build tool the run spawns, so `build.rs` and
  `option_env!("NOTES_EDITION")` see it.
- **Resources.** `resources` names a `resource/`-shaped directory that is merged over the app's
  by relative path: ship one icon and one locale file, inherit every other asset. A `.ftl` file
  is replaced whole, so an overlay of `locales/en/app.ftl` carries every key the app reads.
- **The store listing.** `store` names the `store/` directory `day store stage` reads, because a
  flavor with a different app id is a different store record.

The crate name, the `day` dependency and the manifest schema stay in `Day.toml`. Changing those
makes a different project, which is what `day new` is for.

## Reading a flavor from Rust

A cargo feature compiles code in or out; an `[env]` value is read at compile time.

```rust
#[cfg(feature = "custom-branding")]
pub fn edition_badge() -> String {
    // `option_env!` keeps a plain `day build` compiling, with no flavor in sight.
    match option_env!("NOTES_EDITION") {
        Some(edition) => format!("{edition} edition"),
        None => "Standard edition".to_string(),
    }
}
```

Text that differs per flavor is better placed in the resource overlay, where it stays
[translatable](/docs/localization).

## Where the output goes

A flavored build writes to `build/day/flavors/<name>/`, so flavors build incrementally against
each other and both apps exist on disk at once. Packaged artifacts take the flavor as a suffix —
`notes-custom.dmg` — unless `app.artifact` names something else, so one release directory can
hold them all.

## One dayscript, both apps

A flavor changes what the app says, so the [dayscript](/docs/dayscript) gates match it too:
`skip_on: [flavor:custom]` and `only_on: [flavor:custom]`, with `flavor:none` for the base app.

```yaml
- assert_text: { id: welcome-title, text: "Welcome to Notes", skip_on: [flavor:custom] }
- assert_text: { id: welcome-title, text: "Welcome to the Custom edition", only_on: [flavor:custom] }
```

## Checks

`day lint` reads every `Day-<name>.toml` in the project, including the ones this build is not
using, and reports a file that does not parse, a cargo feature `Cargo.toml` does not declare, and
two flavors that resolve to the same app id — a pair that would install as one app, each
overwriting the other.

`day metadata --json` reports the active flavor and every declared one, which is how a script
discovers what a project can build.

## In CI

The [Day app workflow](/docs/packaging) takes a `flavors:` input. Each name becomes a matrix leg
per target, alongside the base app:

```yaml
jobs:
  app:
    uses: daybrite/actions/.github/workflows/dayapp.yml@main
    with:
      targets: macos-appkit,ios-uikit,android-mdc,web-dom
      flavors: custom
```
