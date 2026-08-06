---
title: CLI & projects
description: The Day command-line tool, the conventional project layout, Day.toml, and dayscript.
order: 30
section: Build & ship
---

The `day` CLI (modeled on the architecture of `flutter_tools`) creates, builds, launches, packs,
lints, and scripts projects. It works the same driven by hand, from CI, or from an IDE.

## The commands

```bash
day new                      # interactive: scaffold an app, a piece, or a part
day new app my-app           # scaffold a new app non-interactively (--no-website to skip the site config)
day app add-toolkit android-mdc   # add a target to an existing app
day localize list|add|remove # survey the project's locales, or add/remove one on every surface at once
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

`day pack` produces a standalone, installable package per target. See
[Packaging & distribution](/docs/packaging) for formats, signing, and CI:

| target | artifact |
|---|---|
| `macos-appkit` | `.dmg` (codesign → notarize → staple) |
| `ios-uikit` | `.ipa` (App Store export; Simulator `.app.zip` without signing config) |
| `android-mdc` | `.apk` + `.aab` (release-signed) |
| `linux-gtk` / `linux-qt` | single-file `.flatpak` bundle **and** a `.appimage` |
| `windows-xaml` | `.msix` + NSIS `-setup.exe` |
| `harmony-arkui` | `.hap` |
| `web-dom` | none — `day build` already emits a self-contained static `dist/` |

Run `day new` with no arguments to be walked through choosing what to create (app / piece / part) and
which platforms and toolkits to support. Every question has an equivalent flag, so the same choices
can be made non-interactively, e.g. `day new app my-app --toolkit ios-uikit --toolkit macos-appkit
--appid com.example.myapp --title "My App"`. Scaffolds currently depend on `day` from its git
remote (the framework crates are not yet published to crates.io); once they are, `--registry`
pins them to your CLI's version from crates.io and will become the default.

`day new app` scaffolds a working starter: a typed-route sidebar over four sample panels (a
reactive counter, a controls tour, a canvas dial, and a drill-down stack), with locales, a
dayscript smoke test (`day launch -p <target> --script dayscript/smoke.yaml`), and the thin native
host projects the mobile targets build through. The scaffold comes from a **template**: a plain
directory tree whose file contents *and paths* are rendered with mustache-style placeholders
(`{{name}}`, `{{ident}}`, `{{snake}}`, `{{pascal}}`, `{{title}}`, `{{id}}`, `{{scheme}}`,
`{{day_dep}}`, `{{targets_toml}}`, `{{first_target}}`). The built-in template is embedded in the
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
day launch -p macos-gtk --script dayscript/walkthrough.yaml --locale fr

# capture VARIANTS of the same walkthrough: `--variant` names the screenshot subdirectory
# (build/day/screenshots/<target>/<variant>/) and DAY_THEME forces the theme on every backend
day launch -p macos-gtk --script dayscript/walkthrough.yaml --variant dark --env DAY_THEME=dark

# variant loops share ONE binary (theme and locale are runtime inputs): build once, then
# `--skip-build` reuses the artifact — on iOS this pays xcodebuild once instead of per variant
day build -p ios-uikit
day launch -p ios-uikit --skip-build --script dayscript/walkthrough.yaml --variant dark --env DAY_THEME=dark

# --record captures what YOU do into a replayable dayscript: drive the app by hand, and the file
# is rewritten continuously (see the dayscript "Recording" guide)
day launch -p macos-appkit --record recording.yaml
```

CI runs each showcase walkthrough once per theme × locale (`light`/`dark` × en/fr/ar/zh-CN)
exactly this way, and the [gallery](/gallery) lets you flip every screenshot between those
variants.

### Simulators, emulators, and devices

Without a device flag, a launch goes to **every** runtime of that kind it can see: every booted
iOS simulator, every connected Android device and emulator. That is what a capture sweep wants.
When you mean one specific phone, name it. Selection is one flag per runtime, so a single command
can send each `-p` somewhere different:

| Flag | Selects | Find them with |
| --- | --- | --- |
| `--ios-device <name\|udid>` | a physical iPhone or iPad | `xcrun devicectl list devices` |
| `--ios-simulator <name\|udid>` | one booted simulator | `xcrun simctl list devices booted` |
| `--android-device <serial>` | one device or emulator | `adb devices` |

`--android-device` is the same selection `ANDROID_SERIAL` makes, so an exported serial keeps
working. `--device` is an accepted alias for `--ios-simulator`.

```bash
# every booted simulator — the default, and what a screenshot sweep wants
day launch -p ios-uikit

# one booted simulator, by name or UDID
day launch -p ios-uikit --ios-simulator "iPhone 16 Pro"

# a physical iPhone
day launch -p ios-uikit --ios-device "iPhone 13 mini"

# one Android device or emulator, by adb serial
day launch -p android-mdc --android-device 19091FDF600BAY

# both phones at once, from one command, with the logs interleaved
day launch -p ios-uikit    --ios-device "iPhone 13 mini" \
           -p android-mdc  --android-device 19091FDF600BAY

# start them and get the shell back rather than staying attached to the logs
day launch -p ios-uikit --ios-device "iPhone 13 mini" --detach

# drive a device run with a dayscript, the same as a simulator run
day launch -p ios-uikit --ios-device "iPhone 13 mini" --script dayscript/smoke.yaml

# a phone and a desktop together, to compare the same screen side by side
day launch -p ios-uikit --ios-device "iPhone 13 mini" -p macos-appkit
```

Every target narrates the same way. Day reports each step itself, and the tools underneath it
(`adb`, `devicectl`, `simctl`) stay quiet unless they fail, at which point their output is the
diagnostic. The two-phone command above prints:

```
     Signing Showcase.app (Day Showcase iOS Development)
  Installing ios-uikit on iPhone 13 mini
   Launching ios-uikit (dev.daybrite.showcase) on device iPhone 13 mini
  Installing android-mdc on 19091FDF600BAY
   Launching android-mdc (dev.daybrite.showcase) on 19091FDF600BAY (arm64-v8a)
```

and then streams both apps' stdout and stderr, each line prefixed with the target it came from —
`[ios-uikit]`, `[android-mdc]` — so two phones running at once read apart. Ctrl-C stops the run
and takes the log watchers with it.

### What a physical iOS device needs

Naming `--ios-device` changes the build, not just where it lands: the `iphoneos` SDK instead of
the simulator's, and code signing, which a simulator build does not do at all. Day signs the
bundle after the build against a **development provisioning profile** installed for the app's
bundle id — the profile supplies both the signing identity (matched by fingerprint, so a machine
holding several development certificates picks the right one) and the entitlements, which is what
keeps the signature from claiming something its profile does not grant.

So the prerequisites are a paired device and a profile that covers this app and lists that device.
Install one by double-clicking the `.mobileprovision`; without a match, the launch stops and says
so rather than falling back to a simulator. Push is the case where the two halves have to agree:
if `Day.toml` declares `notifications`, the build fails when the profile has no `aps-environment`,
instead of installing an app that cannot register.

One error is worth recognizing on sight, because Apple reports it as `RequestDenied`:

```
[ios-uikit] the device is locked — unlock it and run again (iOS will not launch an app onto a locked screen)
```

Installing works on a locked phone; launching does not.

## The conventional project

A Day project is a normal Cargo package plus a small `Day.toml`: the project marker and the
home of everything Day-specific. Two rules prevent drift: `name` and `version` are **derived
from Cargo.toml's `[package]`** (never restated, so identity can't drift), and any `[app]`
property can be **overridden per platform, per toolkit, or per target** (`[app.ios]`,
`[app.qt]`, `[app.macos-appkit]`), with the most specific table winning. The build tool reads
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
  "android-mdc",
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
`day lint` validates the manifest's structure. Unknown targets and override tables that name
no known platform/toolkit/target are findings.

## Store listings

An app that ships to the App Store or Google Play keeps its listing under `store/<locale>/`, as
plain text files named for what they are — `name.txt`, `subtitle.txt`, `short.txt`,
`description.txt`, `keywords.txt`, `release-notes.txt` — one directory per locale, keyed the same
way `resource/locales/` is. `day new app` scaffolds it for any app with a mobile target, and
`day store init` adds it to an existing one.

The two stores disagree about nearly everything: field names, length limits (release notes: 4000
characters on the App Store, 500 on Google Play), which fields exist, and how a locale is spelled
(`zh-CN` here is `zh-Hans` to Apple). `day store stage` resolves all of it, generating a ready-to-run
fastlane project per target under `build/day/store/<target>/`, with `validate` and `upload` lanes.
`day pack` runs the same generation, so a packaged build already has its listing beside it.

`day lint` holds the listing to the stores' rules before an upload can reject it — length limits per
store, required fields, URL format, leftover `TODO` placeholders, and locale parity with the app's
own translations, so translating the app into a new language asks for the listing to follow. See
[Store listings](/docs/internal/store) for the full field table and the credential variables.

In CI, `day lint --strict` turns any finding into a failure (exit 10). A fresh scaffold trips one
rule by design: the listing text it ships is still `TODO`. Pass `--allow store-placeholder` to let
that one code stand while every other rule still fails the run. An allowed code is still reported,
as one line carrying its count and a sample, so an `--allow` nobody has revisited stays visible.

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
screenshots (and its installable packages) as artifacts. This site's [gallery](/gallery) is
assembled from those screenshot artifacts, so it always shows the latest captures from each
platform that succeeded. [Packaging & distribution](/docs/packaging) covers the artifact
pipeline, and [Platform support](/docs/platforms) reports what that CI shows, per target.
