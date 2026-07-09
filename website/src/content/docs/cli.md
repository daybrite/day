---
title: CLI & projects
description: The Day command-line tool, the conventional project layout, day.yaml, and dayscript.
order: 4
---

The `day` CLI (modeled on the architecture of `flutter_tools`) creates, builds, launches, packs,
lints, and scripts projects. It's built for humans, CI, IDEs, and AI agents alike.

## The commands

```bash
day new                      # interactive: scaffold an app, a piece, or a part
day new app my-app           # scaffold a new app non-interactively
day build   -p macos-appkit  # build one target
day launch  -p macos-gtk     # build + run on a target
day pack    -p macos-appkit  # build + sign + produce a distributable artifact (.dmg here)
day sign    --check          # report release-signing readiness without printing secrets
day lint                     # check ids, Fluent coverage, project shape
day doctor                   # check toolchains for every target
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
--appid com.example.myapp`. (`day create` remains as a thin alias for `day new app`.)

`day launch` streams the app's stdout/stderr back to your terminal and can drive it with a script:

```bash
# run a dayscript walkthrough after launch, capturing localized screenshots
day launch -p macos-gtk --script scripts/walkthrough.yaml --locale fr
```

## The conventional project

A Day project is a normal Cargo package plus a small `day.yaml`. The build tool generates the
per-platform scaffolds (an Xcode project, a Gradle module tree) on demand and links them to your
SwiftPM-style dependency graph. You never hand-maintain them.

```yaml
# day.yaml
day: 1
app:
  name: showcase
  id: dev.daybrite.showcase
  title: Day Showcase
  version: 0.1.0
targets:
  - macos-appkit
  - macos-gtk
  - macos-qt
  - ios-uikit
  - android-widget
window:
  width: 480
  height: 640
```

One backend feature is enabled per binary; `day launch -p <target>` selects it, so the AppKit build
contains only AppKit code and the Android build only its JNI bridge. The full directory anatomy,
the per-target build pipelines, and how resources are packaged are covered in
[Project structure & builds](/docs/project-structure).

## dayscript

**dayscript** is a YAML language that drives and asserts a *running* app over a socket, using the
same script on every platform. It powers the showcase walkthrough and the screenshots in the
[gallery](/gallery).

```yaml
name: walkthrough
flow:
  - wait_for:   { id: home-title }
  - navigate:   { route: controls }
  - assert_route: { route: controls }
  - input:      { id: name-field, text: "Ada" }
  - tap:        { id: increment-button }
  - assert_value: { id: counter-label, value: "1 click" }
  - tap:        { id: btn-alert }
  - assert_presented:
  - respond:    { button: 0 }
  - screenshot: controls
```

Because Pieces are addressed by the same stable `.id` you give them in Rust, and routes are the
same keys your `selector`/`stack` use, one script exercises the app identically on macOS, iOS,
Android, Linux, and Windows.

## Continuous integration

Every push builds the showcase on all ten targets and runs the walkthrough, uploading each target's
screenshots as an artifact. This site's [gallery](/gallery) is assembled from those artifacts, so
it always shows the latest captures from each platform that succeeded. See
[Why Day](/docs/benefits) for the bigger picture.
