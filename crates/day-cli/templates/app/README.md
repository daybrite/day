# {{title}}

A [Day](https://daybrite.dev) app: one Rust codebase, native widgets on every platform.

## Run it

Day compiles **one backend per binary**, so choose a target when you build or launch — a bare
`cargo build` enables no backend feature and will not link. The Day CLI supplies the right
feature for each target:

```sh
day doctor                  # check the toolchains for your targets
day launch -p {{first_target}}   # build + run
day build  -p {{first_target}}   # build only
```

Targets live in `Day.toml`. To use plain cargo, pass the backend feature yourself, e.g.
`cargo build --features appkit` (macOS) / `--features gtk` / `--features uikit` /
`--features mdc` (Android).

## What's inside

- `src/lib.rs` — the UI (`root()`), shared across every platform: a typed-route sidebar
  ([navigation](https://daybrite.dev/docs/navigation)) over four sample panels.
- `src/pages/home.rs` — signals in one glance: the reactive counter.
- `src/pages/controls.rs` — two-way bindings: toggle, slider, text field.
- `src/pages/canvas.rs` — a reactive display list drawn natively.
- `src/pages/items.rs` — a drill-down stack with data-carrying typed routes.
- `resource/locales/en/app.ftl` — every user-facing string ([localization](https://daybrite.dev/docs/localization)).
- `dayscript/smoke.yaml` — a [dayscript](https://daybrite.dev/docs/dayscript) UI test:
  `day launch -p {{first_target}} --script dayscript/smoke.yaml`.
- `platform/` — the thin native host projects (Xcode / Gradle / hvigor) the mobile targets
  build through; `day build` keeps their identity in sync with `Day.toml`.
- `Day.toml` — app metadata + the target list.

`day lint` checks routes, element ids, and locale coverage.
