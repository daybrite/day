---
title: CLI & projects
description: The Day command-line tool, the conventional project layout, Day.toml, and dayscript.
order: 30
section: Build & ship
---

The `day` CLI (modeled on the architecture of `flutter_tools`) creates, builds, launches, packs,
lints, and scripts projects. It's built for humans, CI, IDEs, and AI agents alike.

## The commands

```bash
day new                      # interactive: scaffold an app, a piece, or a part
day new app my-app           # scaffold a new app non-interactively
day app add-toolkit android-widget   # add a target to an existing app
day build   -p macos-appkit  # build one target
day launch  -p macos-gtk     # build + run on a target
day pack    -p macos-appkit  # build + sign + produce a distributable artifact (.dmg here)
day sign    --check          # report release-signing readiness without printing secrets
day lint                     # check ids, Fluent coverage, project shape
day doctor                   # check toolchains for every target
day stop --all               # stop running launches (sessions in build/day/sessions.json)
day relaunch --all-running   # stop + rebuild + relaunch — "apply my changes"
day drive -p <t> --steps-json '…'   # drive a RUNNING app with dayscript steps
day mcp-server               # serve Day tools to AI agents (Model Context Protocol, stdio)
```

`day pack` produces a standalone, installable package per target — see
[Packaging & distribution](/docs/packaging) for formats, signing, and CI:

| target | artifact |
|---|---|
| `macos-appkit` | `.dmg` (codesign → notarize → staple) |
| `ios-uikit` | `.ipa` (App Store export; Simulator `.app.zip` without signing config) |
| `android-widget` | `.apk` + `.aab` (release-signed) |
| `linux-gtk` / `linux-qt` | single-file `.flatpak` bundle |
| `windows-winui` | `.msix` + NSIS `-setup.exe` |
| `ohos-arkui` | `.hap` |

Run `day new` with no arguments to be walked through choosing what to create (app / piece / part) and
which platforms and toolkits to support. Every question has an equivalent flag, so the same choices
can be made non-interactively, e.g. `day new app my-app --toolkit ios-uikit --toolkit macos-appkit
--appid com.example.myapp --title "My App"`. Scaffolds currently depend on `day` from its git
remote (the framework crates are not yet published to crates.io); once they are, `--registry`
pins them to your CLI's version from crates.io and will become the default.

`day new app` scaffolds a working starter — a typed-route sidebar over four sample panels (a
reactive counter, a controls tour, a canvas dial, and a drill-down stack), with locales, a
dayscript smoke test (`day launch -p <target> --script scripts/smoke.yaml`), and the thin native
host projects the mobile targets build through. The scaffold comes from a **template**: a plain
directory tree whose file contents *and paths* are rendered with mustache-style placeholders —
`{{name}}`, `{{ident}}`, `{{snake}}`, `{{pascal}}`, `{{title}}`, `{{id}}`, `{{scheme}}`,
`{{day_dep}}`, `{{targets_toml}}`, `{{first_target}}`. The built-in template is embedded in the
CLI (a fresh `cargo install day-cli` scaffolds offline); bring your own with:

```bash
day new app my-app --template ./my-template          # a local directory
day new app my-app --template https://github.com/you/tpl#v1   # a git repo (optional #ref)
```

Template conventions: a trailing `.hbs` on a filename is stripped after rendering (use
`Cargo.toml.hbs` so tooling doesn't mistake the template for a Rust package), `_gitignore`
becomes `.gitignore`, non-UTF-8 files (icons) copy verbatim, and an unknown `{{placeholder}}`
is an error rather than silent empty output. Files under `platform/<os>/` belong to that OS's
targets and are only scaffolded for targets that need them.

Add a platform later with **`day app add-toolkit <target>`** (repeatable / comma-separated):
it appends the target to `Day.toml`'s `[app] targets` array (via toml_edit, so your comments
and formatting survive) and materializes the target's native host project (`platform/android/`,
`platform/ios/`, `platform/ohos/`) from the same template, never overwriting existing files.
Pass the same `--template` the app was created with if it wasn't the built-in one.

`day launch` streams the app's stdout/stderr back to your terminal and can drive it with a script:

```bash
# run a dayscript walkthrough after launch, capturing localized screenshots
day launch -p macos-gtk --script scripts/walkthrough.yaml --locale fr

# capture VARIANTS of the same walkthrough: `--variant` names the screenshot subdirectory
# (build/day/screenshots/<target>/<variant>/) and DAY_THEME forces the theme on every backend
day launch -p macos-gtk --script scripts/walkthrough.yaml --variant dark --env DAY_THEME=dark
```

CI runs each showcase walkthrough three times — `light` and `dark` under a forced `DAY_THEME`,
and `fr` under `--locale fr` — and the [gallery](/gallery) lets you flip every screenshot
between those variants.

## The conventional project

A Day project is a normal Cargo package plus a small `Day.toml` — the project marker and the
home of everything Day-specific. Two rules keep it honest: `name` and `version` are **derived
from Cargo.toml's `[package]`** (never restated, so identity can't drift), and any `[app]`
property can be **overridden per platform, per toolkit, or per target** — `[app.ios]`,
`[app.qt]`, `[app.macos-appkit]` — with the most specific table winning. The build tool reads
the resolved values when it derives platform metadata (an Android build's label and
applicationId, for example).

```toml
# Day.toml
schema = 1

[app]
id = "dev.daybrite.showcase"
title = "Day Showcase"
build = 1
targets = [
  "macos-appkit",
  "macos-gtk",
  "macos-qt",
  "ios-uikit",
  "android-widget",
]

[window]
width = 480
height = 640

# Example: a different display title on iOS only.
[app.ios]
title = "Showcase Mobile"
```

`day metadata` prints the project's identity, targets, and per-target resolved values;
`--json` emits a versioned, machine-readable envelope (this is what the VS Code extension
consumes instead of parsing Day.toml itself, and it also carries the full target catalog).
`day lint` validates the manifest's structure — unknown targets and override tables that name
no known platform/toolkit/target are findings.

One backend feature is enabled per binary; `day launch -p <target>` selects it, so the AppKit build
contains only AppKit code and the Android build only its JNI bridge. The full directory anatomy,
the per-target build pipelines, and how resources are packaged are covered in
[Project structure & builds](/docs/project-structure).

## dayscript

**dayscript** is a YAML language that drives and asserts a *running* app over a socket, using the
same script on every platform. Pieces are addressed by the same stable `.id` you give them in
Rust, and routes are the same keys your `selector`/`stack` use, so one script exercises the app
identically everywhere. It has its own guide: [Testing with dayscript](/docs/dayscript).

## Continuous integration

Every push builds the showcase on every target and runs the walkthrough, uploading each target's
screenshots — and its installable packages — as artifacts. This site's [gallery](/gallery) is
assembled from those screenshot artifacts, so it always shows the latest captures from each
platform that succeeded. [Packaging & distribution](/docs/packaging) covers the artifact
pipeline, and [Platform support](/docs/platforms) reads the same CI honestly.
