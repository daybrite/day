# day-winui

Day's Windows backend: WinUI/XAML controls hosted through XAML Islands, over the
[`day-winui-sys`](https://crates.io/crates/day-winui-sys) C++/WinRT shim.

Pieces realize as real `Windows.UI.Xaml` controls (`TextBlock`, `Button`, `ToggleSwitch`,
`Slider`, …) inside a `DesktopWindowXamlSource`; Day owns layout, Windows owns rendering
and accessibility. This is the backend behind the `windows-winui` target.

Backends are picked by a cargo feature on [`day`](https://crates.io/crates/day) — one per
binary; apps never depend on this crate directly.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
