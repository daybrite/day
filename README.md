<h1 align="center">
  <a href="https://daybrite.dev">
    <img width="120" alt="Day" src="website/public/favicon.svg" />
  </a>
  <br />
  Day
</h1>

<p align="center">

[![Crates.io](https://img.shields.io/crates/v/day-cli.svg)](https://crates.io/crates/day-cli)
[![Build Status](https://github.com/daybrite/day/actions/workflows/ci.yml/badge.svg)](https://github.com/daybrite/day/actions/workflows/ci.yml)
[![Rust Version](https://img.shields.io/badge/rust-1.89%2B-blue.svg)](https://www.rust-lang.org)
[![Edition](https://img.shields.io/badge/edition-2024-blue.svg)](https://doc.rust-lang.org/edition-guide/rust-2024/)
[![Examples](https://img.shields.io/badge/Day-Examples-purple.svg)](https://daybrite.dev/examples/)

</p>

## About Day

Day lets you build native desktop and mobile apps with shared Rust code. Describe the interface,
connect it to your app’s state, and use the controls provided by each target’s UI toolkit.
Translations, UI walkthroughs, and packaging live alongside the app in the same project.

For standard controls, Day creates toolkit widgets: an AppKit `NSButton` on macOS, a UIKit
`UIButton` on iOS, a Material button on Android, and GTK, Qt, or WinUI controls on desktop.
Composed controls combine other controls; custom drawing uses the canvas API. Appearance,
capabilities, and accessibility behavior vary by backend and need target-specific testing.

Day is useful when you want to bring a Rust library to people through an app, maintain desktop
and mobile interfaces together, or share a native integration with other creators. Test the
screen or device feature your app depends on before planning a full port. See
[Is Day a good fit?](https://daybrite.dev/features) and the
[examples](https://daybrite.dev/examples).

## A working example

The tutorial builds a small counter with generated Fluent messages and symbolic control IDs.
The website shows [the exact source and an AppKit capture](https://daybrite.dev/examples#tutorial-capture),
with a record tying the image to its source files. The tutorial continues through layout,
navigation, and an automated walkthrough.

[Build your first app](https://daybrite.dev/docs/tutorials/first-app).

## Platforms

Start with the supported desktop and mobile targets. These are declared maintenance tiers,
not a claim that every feature or release has passed on every device.

| Target | Toolkit | Declared tier |
|---|---|---|
| [macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit) | AppKit | 1 · Supported |
| [iOS and iPadOS](https://daybrite.dev/docs/reference/platforms/ios-uikit) | UIKit | 1 · Supported |
| [Android](https://daybrite.dev/docs/reference/platforms/android-mdc) | Material Components | 1 · Supported |
| [Linux](https://daybrite.dev/docs/reference/platforms/linux-gtk) | GTK 4 / libadwaita | 2 · Demi-supported |
| [Linux](https://daybrite.dev/docs/reference/platforms/linux-qt) | Qt 6 Widgets | 2 · Demi-supported |
| [Windows](https://daybrite.dev/docs/reference/platforms/windows-winui) | WinUI 3 | 2 · Demi-supported |

HarmonyOS and the browser target are experimental. Additional GTK/Qt combinations are for
framework development; system XAML is deprecated. All thirteen targets, package formats,
configured CI checks, and known gaps remain in the
[platform reference](https://daybrite.dev/docs/reference/platforms).

## Development and distribution

The Day CLI creates projects, checks installed tools, builds and launches apps, runs UI
walkthroughs, and produces desktop installers and mobile packages. Generated `res::str`
accessors connect UI text to Fluent catalogs; symbolic IDs let tests address controls without
matching a translated label.

Builds need the target’s SDKs. Distribution also needs the relevant signing credentials,
store accounts, and submission tools. Packaging support does not guarantee store acceptance;
see [packaging and distribution](https://daybrite.dev/docs/how-to/packaging).

Use the [system requirements](https://daybrite.dev/docs/reference/system-requirements) for your
host, then follow the [tutorial](https://daybrite.dev/docs/tutorials/first-app), which uses macOS/AppKit.

Build and launch [Day Showcase](https://github.com/daybrite/Day-Showcase) from source:

```sh
day launch --git https://github.com/daybrite/Day-Showcase
```

The [VS Code extension](https://daybrite.dev/docs/how-to/vscode) adds project and target commands
to your editor. Use the [GitHub Actions guide](https://daybrite.dev/docs/how-to/github-actions)
to automate builds and [store submission](https://daybrite.dev/docs/how-to/app-store-submissions).

## Contributing

Help another creator reach a platform or use a feature you know well. A small reproduction of a
bug, a correction to a guide, or a tested native integration can all help.
[Contribution instructions](CONTRIBUTING.md) explain setup and review expectations.

## License

Mozilla Public License 2.0
