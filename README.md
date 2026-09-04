<h1 align="center">
  <a href="https://daybrite.dev">
    <img width="120" alt="Day" src="https://raw.githubusercontent.com/daybrite/day-vscode/main/media/day-icon.png" />
  </a>
  <br />
  Day
</h1>

<p align="center">

[![Crates.io](https://img.shields.io/crates/v/day-cli.svg)](https://crates.io/crates/day-cli)
[![Build Status](https://github.com/daybrite/day/actions/workflows/ci.yml/badge.svg)](https://github.com/daybrite/day/actions/workflows/ci.yml)
[![Rust Version](https://img.shields.io/badge/rust-1.89%2B-blue.svg)](https://www.rust-lang.org)
[![Edition](https://img.shields.io/badge/edition-2024-blue.svg)](https://doc.rust-lang.org/edition-guide/rust-2024/)
[![Gallery](https://img.shields.io/badge/Screenshots-Gallery-purple.svg)](https://daybrite.dev/gallery/)

</p>

## About Day

Day is a Rust framework for multi-platform application development. It provides a declarative interface over the platform's native underlying GUI toolkit. A single Rust codebase can be used to build mobile apps for iOS, Android, and HarmonyOS, as well as desktop apps for macOS, Windows, and Linux (Gnome/GTK and KDE/Qt).

Day takes a third path among cross-platform toolkits. Web-view shells (Tauri, Electron, Dioxus, Capacitor) render the interface in a browser engine, and custom renderers (Flutter, egui, Slint, Iced) draw every pixel themselves. Day's declarative API and reactive engine operate directly on the platform's own toolkit: they build the native widget tree and keep it in sync with your state. So a `button()` is a `UIButton` on iOS, an `NSButton` on macOS, a `MaterialButton` on Android, a `GtkButton` or `QPushButton` on Linux, and a XAML `Button` on Windows.

This enables you to use Day to create user interfaces that are indistinguishable from ones built directly with the first-party toolkit provided by the mobile or desktop vendor. And because it is Rust, it has bare-metal performance and unmatched efficiency _everywhere_ without sacrificing memory safety and without relying on any additional runtime or garbage collector. Using the first-party native toolkit widgets also gives Day excellent accessibility support out of the box, so screen readers and other assistive technologies can interoperate flawlessly with Day apps.

Applications created in Day are compact and follow the platform's native packaging idioms, which result in installation packages that are often just few megabytes. Day's built-in CI workflows and default project page template enable the automatic generation of landing pages with application information and install links.

See the Day Gallery at https://daybrite.dev/gallery/ for examples of applications built with Day and their individual landing pages.

## Platforms

Day targets twelve `(OS, toolkit)` pairs. Each one is built from that toolkit's own widgets, and
`day pack` produces the package that platform's users install. The
[support tier](https://daybrite.dev/docs/platforms/#support-tiers) records how much testing and
maintenance a target gets today, independent of how complete its backend is.

| Target | OS | Toolkit | Tier | Packaging |
|---|---|---|---|---|
| [`macos-appkit`](https://daybrite.dev/docs/platforms/macos-appkit/) | macOS | AppKit | 1 · Supported | `.dmg` |
| [`ios-uikit`](https://daybrite.dev/docs/platforms/ios-uikit/) | iOS and iPadOS | UIKit | 1 · Supported | `.ipa` |
| [`android-mdc`](https://daybrite.dev/docs/platforms/android-mdc/) | Android | Material Components | 1 · Supported | `.apk` and `.aab` |
| [`linux-gtk`](https://daybrite.dev/docs/platforms/linux-gtk/) | Linux | GTK 4 | 2 · Demi-supported | `.flatpak` and `.appimage` |
| [`linux-qt`](https://daybrite.dev/docs/platforms/linux-qt/) | Linux | Qt 6 Widgets | 2 · Demi-supported | `.flatpak` and `.appimage` |
| [`windows-xaml`](https://daybrite.dev/docs/platforms/windows-xaml/) | Windows | XAML | 2 · Demi-supported | `.msix` and installer |
| [`harmony-arkui`](https://daybrite.dev/docs/platforms/harmony-arkui/) | HarmonyOS | ArkUI | 3 · Experimental | `.hap` |
| [`web-dom`](https://daybrite.dev/docs/platforms/web-dom/) | Web | DOM, via WebAssembly | 3 · Experimental | static `dist/` |
| `macos-gtk` | macOS | GTK 4 | 4 · Development | none |
| `macos-qt` | macOS | Qt 6 | 4 · Development | none |
| `windows-gtk` | Windows | GTK 4 | 4 · Development | none |
| `windows-qt` | Windows | Qt 6 | 4 · Development | none |

All twelve build in CI on every push. Every target except the Windows development combos and
HarmonyOS also runs the Showcase app's full dayscript walkthrough there, and those captures are
what the [gallery](https://daybrite.dev/gallery/) shows. The Tier 4 combos exist for
compatibility testing and to show one toolkit running on several operating systems; real
applications aren't expected to ship on them. The
[Platforms page](https://daybrite.dev/docs/platforms/) carries the per-target notes and known
gaps.

## Getting Started

Top get a feel for how a Day application feels on your desktop, download the Day Showcase application for your platform from https://showcase.daybrite.dev or build and launch it from source with the commands (requires [`rustup`](https://rustup.rs)):

```bash
cargo install day-cli
day launch --git https://github.com/daybrite/Day-Showcase.git@main
```

See https://daybrite.dev/docs/getting-started/ and https://daybrite.dev/docs/system-requirements/ for more details about getting set up. 

> [!TIP]
> To explore using day for development, the [`day-vscode`](https://vscode.daybrite.dev) plugin is recommended but not required.


## About Daybrite

The goal of the Daybrite Project is to become the single go-to framework that Rust developers can use for most application-building needs across the spectrum of consumer-facing computing devices.

## License

Mozilla Public License 2.0
