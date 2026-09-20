---
title: "Build flavors"
description: "Day-<name>.toml: one source tree shipped as several apps, differing in id, name, icon, strings, features and targets."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Build flavors (§16.6)

A flavor ships one source tree as more than one app: a paid build beside a free one, a
white-label build per customer, a demo build with a smaller target list. It is a layer over
`Day.toml`, written in a `Day-<name>.toml` beside it, and it applies to one command at a time:

```sh
day build  --flavor custom -p ios-uikit
day launch --flavor custom -p macos-appkit
day pack   --flavor custom -p android-mdc
```

The flag is global, so every command takes it — `metadata`, `lint`, `store`, `screenshot`,
`icon`, `prepare`, `pack`. `DAY_FLAVOR=custom` does the same, which is how a CI job sets it once
for a whole matrix leg.

A flavor changes what the app *is*. Anything that would make it a different project — the crate
name, the `day` dependency, the manifest schema — stays in `Day.toml`, and a second project is
what `day new` is for.

## The file

```toml
# Day-custom.toml — activated by `day build --flavor custom`

# Overlay directories are top-level keys, so they go before the first table: TOML gives a bare
# key to the table above it, and `resources` written below `[env]` is an environment variable.
resources = "resource-custom"
store = "store-custom"

[app]
id = "dev.example.notes.custom"     # a different app id: installs beside the base app
title = "Notes Custom"
scheme = "notescustom"              # the deep-link scheme is published, so a separate app takes a separate one
build = 7
targets = ["macos-appkit", "ios-uikit", "android-mdc", "web-dom"]

[app.android]                       # the per-platform tables of Day.toml, same spelling
id = "dev.example.notes_custom"     # Android package names take no hyphens

[cargo]
features = ["custom-branding"]      # added to the backend feature and the pieces' features

[env]
NOTES_EDITION = "Custom"            # reaches build.rs and `option_env!()` in the app
NOTES_ACCENT = "#FF6B35"
```

| Key | Effect |
|---|---|
| `app.id` | The bundle id / application id / package name. A flavor that sets a different one installs beside the base app instead of replacing it |
| `app.title` | The display name, and the default the artifact name is slugged from |
| `app.artifact` | The artifact stem, when the default (`<base>-<flavor>`) is not what you want to publish |
| `app.scheme` | The deep-link scheme |
| `app.build` | The build number stores order releases by |
| `app.targets` | Replaces the base list: a flavor may ship on fewer targets, or on more |
| `[app.<platform\|toolkit\|target>]` | The same override tables `Day.toml` takes, merged field by field over the base app's |
| `[cargo] features` | Cargo features to compile with, added to the backend feature and the union the pieces need |
| `[env]` | Environment for every build tool the run spawns: what `build.rs` and `option_env!()` read |
| `resources` | A `resource/`-shaped directory merged over the app's, by relative path |
| `store` | A `store/`-shaped listing directory this flavor publishes with |

Unknown keys are an error. A misspelled key that still parsed would build the base app under the
flavor's name, and nothing in the output would say so.

## Precedence

The flavor is a layer above the whole manifest, and within each layer the usual specificity
applies:

```text
Day-<f>.toml [app.<target>] → [app.<platform>] → [app]
      Day.toml [app.<target>] → [app.<platform>] → [app]
```

A value the flavor states wins; one it omits is inherited. `day metadata` prints the merged
result, and `day metadata --json` carries `flavor` (the active one) and `flavors` (every one the
project declares).

## Where a flavored build writes

Compiled and staged output moves under the flavor's directory:

```text
build/day/                       # the base app
build/day/flavors/custom/        # `--flavor custom`
  artifacts/  cargo/  resource/  screenshots/  vectors/
```

Two flavors therefore build incrementally against each other instead of invalidating one
cargo directory back and forth, and both apps exist on disk at once. Two paths stay shared:
`build/day/dist/`, so one directory holds every flavor's packages, and `build/day/host/`, the
derived icon tree the checked-in host projects reference by a fixed path — it is regenerated from
the active flavor's resources on every build. Artifact names gain the
flavor as a suffix (`notes-custom.dmg`), unless `app.artifact` names something else, so one
`dist/` directory can hold every flavor of a release.

## Resources

`resources = "resource-custom"` merges that directory over the app's `resource/` by relative
path, into `build/day/flavors/<name>/resource/`. A file the overlay provides replaces the app's;
every other file is the app's, unchanged. So a flavor that ships one icon and one locale file
contains two files, not a copy of every asset the app has.

Locale files are replaced whole, since a `.ftl` file is one file: an overlay of
`locales/en/app.ftl` has to carry every key the app reads, not only the ones it changes.

The merged tree is what the whole build reads: the generated `res::` constants and compiled-in
strings, the icons `day prepare` derives the platform icon sets from, the images, fonts and data
assets each backend stages, and the files a desktop `day launch` loads at runtime.

## Reading a flavor from Rust

There are two mechanisms, and the choice depends on whether the difference is code or a value:

```rust
// A cargo feature compiles code in or out.
#[cfg(feature = "custom-branding")]
pub fn edition_badge() -> String { … }

// An [env] value is read at compile time, and `option_env!` keeps a plain build compiling.
const EDITION: &str = match option_env!("NOTES_EDITION") { Some(v) => v, None => "Standard" };
```

Strings that differ per flavor belong in the resource overlay instead, so they stay translatable.

## Dayscripts

A flavor changes what the app says, so a walkthrough that covers both says which step belongs to
which. `skip_on:` and `only_on:` match `flavor:<name>`, and `flavor:none` is the base app:

```yaml
- assert_text: { id: welcome-title, text: "Welcome to Notes", skip_on: [flavor:custom] }
- assert_text: { id: welcome-title, text: "Welcome to the Custom edition", only_on: [flavor:custom] }
- assert_visible: { id: edition-badge, only_on: [flavor:custom] }
```

One script then runs on every target and every flavor, and each run captures its screenshots
into the flavor's own directory (`build/day/flavors/<name>/screenshots/`).

## Checks

`day lint` reads every `Day-<name>.toml` in the project, whether or not this run builds one:

| Code | What it catches |
|---|---|
| `flavor-unparsed` | The file does not parse, or a top-level key landed inside `[env]` |
| `flavor-id-collision` | Two flavors (or a flavor and the base app) resolve to the same app id, which installs as one app that overwrites the other |
| `flavor-unknown-feature` | `[cargo] features` names a feature `Cargo.toml` does not declare, which would fail only when that flavor is built |

## CI

The `daybrite/actions` app workflow takes a `flavors:` input, a comma-separated list. Each flavor
becomes a matrix leg per target, with its artifacts named after it:

```yaml
jobs:
  app:
    uses: daybrite/actions/.github/workflows/dayapp.yml@main
    with:
      targets: macos-appkit,ios-uikit,android-mdc,web-dom
      flavors: custom
```

The base app always builds; `flavors:` adds legs rather than replacing them.

## How other tools spell this

Flutter's `--flavor` names "a custom Android product flavor or an Xcode scheme", so the
definition lives in the native build systems and the tool reads it back out — parsing an Xcode
configuration name to recover the flavor, guessing the filenames Gradle will write. Day
generates its host projects (the bundle id reaches Xcode through a generated xcconfig, the
application id through generated Gradle properties), so a flavor changes files Day already
writes and neither Xcode nor Gradle has to know the concept exists.
