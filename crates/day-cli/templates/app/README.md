# {{title}}

A [Day](https://daybrite.dev) app: one Rust codebase, native widgets on every platform.

## Run it

Day compiles **exactly one backend per binary**, so choose a target when you build or launch. The
Day CLI supplies the right feature for each target:

```sh
day doctor                  # check the toolchains for your targets
day launch -t {{first_target}}   # build + run
day build  -t {{first_target}}   # build only
```

Targets live in `Day.toml`. A bare `cargo build` uses this crate's default `mock` backend, which is
what lets rust-analyzer and `cargo check` work with no flags. To pick a real one from plain cargo,
turn the default off as well — otherwise `mock` and your choice are both on, which is two backends
and a compile error:

```sh
cargo build --no-default-features --features appkit    # or gtk / qt / uikit / mdc / winui / dom
```

## What's inside

- `src/lib.rs` — the UI (`root()`), shared across every platform: a typed-route sidebar
  ([navigation](https://daybrite.dev/docs/navigation)) over four sample panels.
- `src/pages/home.rs` — signals in one glance: the reactive counter.
- `src/pages/controls.rs` — two-way bindings: toggle, slider, text field.
- `src/pages/canvas.rs` — a reactive display list drawn natively.
- `src/pages/items.rs` — a drill-down stack with data-carrying typed routes.
- `resource/locales/en/app.ftl` — every user-facing string ([localization](https://daybrite.dev/docs/localization)).
- `dayscript/demo.yaml` — a [dayscript](https://daybrite.dev/docs/dayscript) walkthrough that
  drives every feature this app ships, and doubles as its UI test:
  `day launch -t {{first_target}} --script dayscript/demo.yaml`.
- `platform/` — native host sources and configuration (Swift / Gradle / hvigor);
  `day build` keeps their identity in sync with `Day.toml`. Apple `.xcodeproj` folders
  are generated under `build/day/xcode/` by default.
- `Day.toml` — app metadata + the target list.
- `.github/workflows/ci.yml` — builds and tests every target above on each push through the
  shared [Day workflow](https://daybrite.dev/docs/github-actions), and attaches packages to a
  GitHub release on a `v1.2.3` tag.

`day lint` checks routes, element ids, and locale coverage.

macOS and iOS builds require full **Xcode 27 or newer**. Their host projects use
`build/day/xcode/<platform>/DayApp.xcodeproj/project.xcproj` (generated JSON5); `day doctor` checks the selected Xcode
and `day lint` validates the project wiring. Open a prepared Apple project with
`day open -t macos-appkit` or `day open -t ios-uikit`. For manual project customization,
`day project add-target <target> --persist-xcode-project` creates a persistent override.
