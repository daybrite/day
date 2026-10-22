# {{title}}

A small starting point for a [Day](https://daybrite.dev) app: a welcome page, a list with a
detail editor, and settings, rendered with native widgets from one Rust codebase.

## Run it

Day compiles one toolkit backend per binary, so name a target when you build or launch;
`Day.toml` lists the app's targets.

```sh
day doctor                     # check the toolchains for your targets
day launch -t {{first_target}}  # build + run
day launch -t {{first_target}} --script dayscript/demo.yaml   # walk through the app
```

A bare `cargo build` or `cargo test` uses the crate's default `mock` backend, which is also what
rust-analyzer checks.

## What it does

Welcome and Items share adaptive navigation: the item list and editor sit side by side on a wide
window and stack on a phone. Give an item a name, a type and a color, and mark it done; every
change is saved as you make it, and Undo and Redo step back through them. Search narrows the
list, a swipe or the context menu marks an item done, dragging reorders it, and the Items tab
counts what is left to do. Settings holds the appearance, the language, and the app's own
setting for completed items; About shows the app's name and version. On desktop both open from
the app menu; on a phone, Settings is a section. The app reopens on the section you last visited.

## The code

- `src/lib.rs` sets up the window, the routes, the navigation, and the menus.
- `src/model.rs` holds the items and the selection they share, with the unit tests.
- `src/pages.rs` builds the Welcome page, the item list, and the editor.
- `resource/locales/en/app.ftl` holds every string the app shows.
- `dayscript/demo.yaml` walks through the app on every target and takes the screenshots.
- `Day.toml` holds the app's identity, targets, and window size; `platform/` holds the
  app-owned native files, and Day generates the native projects under `build/`.
- `store/` and `website/` hold the store listing and the website settings.
- `.github/workflows/ci.yml` builds and tests every target through the shared
  [Day workflow](https://daybrite.dev/docs/how-to/github-actions).

`day lint` checks routes, element ids, and locale coverage. Apple builds need Xcode 27 or newer.
