<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-cli

The `day` command line: create, build, run, test, and package Day apps.

```text
day new app hello         # a working project: Day.toml, src/, a starter UI
day launch -t macos-appkit -t android-material
day launch --git https://github.com/daybrite/Day-Rise.git   # clone a repo and run its app
day doctor                # what's installed, what's missing, how to fix it
day doctor verify        # doctor, then scaffold + build + pack a test app for every target
day pack -t macos-appkit  # a signed .dmg; .ipa, .apk, .flatpak, .msix, .hap per target
```

Building for seven toolkits normally means juggling seven build systems — cargo,
xcodebuild, Gradle, hvigor, resource compilers, code signing. `day` drives all of them,
in parallel when you ask for several targets at once, so a Day project keeps no platform
build files of its own beyond the small checked-in scaffolds.

It is also built for automation. Every command has a JSON output mode, `day drive` steps
through a running app one action at a time (the tool coding agents use), and
`day mcp-server` offers the same tools over the Model Context Protocol.

Install it with `cargo install --locked day-cli`, then run `day new`.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's own widgets — AppKit, UIKit, Android's Material widgets, GTK 4, Qt 6,
WinUI, and ArkUI — from one codebase. When you write `button("Save")`, macOS shows an
`NSButton` and Android shows a Material button. The framework also ships the tooling around
the app: the `day` CLI, a VS Code extension, GitHub CI workflows, localization,
accessibility, and dayscript automation.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).

## Android target naming

Use `android-material` for the Android target and `material` for its toolkit and Cargo
feature. The tooling temporarily accepts `android-mdc` and `mdc`, warns that they are
deprecated, and writes canonical target names. Mixed spellings in target lists select one
target; conflicting override keys are rejected. These aliases do not apply to Rust package
names, imports, or Cargo features. They will be removed in the next announced breaking
tooling release after one release of overlap.
