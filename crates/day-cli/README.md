# day-cli

The `day` command line: create, build, run, test, and package Day apps.

```text
day new app hello         # a working project: Day.toml, src/, a starter UI
day launch -p macos-appkit -p android-mdc
day doctor                # what's installed, what's missing, how to fix it
day pack -p macos-appkit  # a signed .dmg; .ipa, .apk, .flatpak, .msix, .hap per target
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
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
