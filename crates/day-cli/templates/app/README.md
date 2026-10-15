# {{title}}

A small starting point for a [Day](https://daybrite.dev) app: a welcome page, a list with a
detail editor, and settings, rendered with native widgets from one Rust codebase.

## Run it

Day compiles one toolkit backend per binary, so name a target when you build or launch:

```sh
day doctor                     # check the toolchains for your targets
day launch -t {{first_target}}  # build + run
day build -t {{first_target}}   # build only
```

Targets live in `Day.toml`. A bare `cargo build` uses the crate's default `mock` backend, which
lets rust-analyzer and `cargo check` work with no flags. To choose a toolkit from plain cargo,
turn the default off first:

```sh
cargo build --no-default-features --features appkit    # or gtk / qt / uikit / mdc / winui / dom
```

## What the scaffold gives you

Welcome and Items share adaptive navigation: the item list and editor appear side by side on a
wide window and stack on a phone. An item has just a name and a done flag. Start with an empty
list, add an item, and edit it through two-way bindings; changes are saved between launches.
Drag items to reorder them on platforms with native list reordering; the order is saved too.
The app also reopens on the section you last visited.

Creation, deletion, and reordering demonstrate `info!` console logging without logging item
contents. Failed saves produce a `warn!`; use `--env DAY_LOG=dayapp=debug` with `day launch`
to also see successful saves.

Settings provides theme and language preferences, and About shows the app's name and version
from `Day.toml`. Both come from `day-piece-settings`: on desktop they open from the app menu in
their own windows; on platforms without a menu bar, Settings is a navigation section with About
at its top. The app uses one main window.

`dayscript/demo.yaml` is a portable [dayscript](https://daybrite.dev/docs/dayscript) smoke test.
It opens Welcome, adds an item, edits its name and done flag, and deletes that same item. It
uses no platform-specific conditions, works with existing saved items, and captures Welcome,
the list, and the editor:

```sh
day launch -t {{first_target}} --script dayscript/demo.yaml
```

## Inside the code

- `src/lib.rs` configures the app and builds the navigation and menus.
- `src/pages/mod.rs` declares the welcome, items, and detail modules.
- `src/model.rs` holds the item model, selection, and JSON persistence, with unit tests at the end.
- `resource/locales/en/app.ftl` carries every user-facing string.
- `dayscript/demo.yaml` is the smoke test; `store/` and `website/` are the store-listing and
  site stubs.
- `platform/` holds app-owned Apple host sources and any native resource customizations.
- `Day.toml` holds app metadata and the target list.
- `.github/workflows/ci.yml` builds and tests the declared targets through the shared
  [Day workflow](https://daybrite.dev/docs/github-actions), and attaches packages to a
  GitHub release on a `v1.2.3` tag.

`day lint` checks routes, element ids, and locale coverage.

## Generated native projects

Apple builds require **Xcode 27 or newer**. Day generates Xcode projects, Android Gradle files,
and HarmonyOS hvigor files under `build/` when you run `day prepare`, `day build`, or
`day launch`. Open a project's IDE with `day open -t <target>`; edits to generated files are
disposable. App-owned native sources and configuration remain under `platform/`. For additional
native configuration, `day project add-target <target> --persist-native-project` creates an
editable override.
