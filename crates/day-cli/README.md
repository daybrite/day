# day-cli

The `day` command line: create, build, run, test, and package Day apps.

```text
day new app hello       # scaffold a project (day.yaml + src/lib.rs)
day launch -p macos-appkit -p android-widget
day doctor              # what's installed, what's missing, how to fix it
day pack -p macos-appkit  # .dmg — plus .ipa / .apk+.aab / .flatpak / .msix / .hap per target
```

One binary drives the whole per-platform build zoo — cargo, xcodebuild, Gradle, hvigor,
resource compilers, signing — so an app project needs no platform build files of its own
beyond the checked-in scaffolds. A dayscript engine can drive a launched app step by step
(tap, type, assert, screenshot), which is how Day's own showcase is tested on every target
in CI.

Install with `cargo install --locked day-cli`, then `day new`.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
