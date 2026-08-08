---
title: Embed SwiftUI views
description: "Ship a SwiftPM package inside your app and call its SwiftUI views from Rust as generated, typed constructors — on macOS and iOS."
order: 27
section: Guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day renders native widgets from Rust, but sometimes the view you want already exists in SwiftUI,
or is easiest to build there: a custom chart, a control from an in-house Swift package, a screen
you're migrating incrementally. On `macos-appkit` and `ios-uikit`, Day hosts your own SwiftUI
views inside the Day tree. You write ordinary SwiftUI in an ordinary SwiftPM package; Day
generates a typed Rust constructor per view, so the call site is:

```rust
crate::swiftui::TemperatureDial(room_name, 21.5)
```

**Works on:** `macos-appkit`, `ios-uikit`. Every other target compiles the same code and you
gate the UI with a support probe (below). The full contract is in
[the swiftui reference](/docs/internal/swiftui).

## 1. Write the package

Create a normal SwiftPM package in your project, conventionally at `swiftui/`:

```text
my-app/
└── swiftui/
    ├── Package.swift             # name it after your module, e.g. MyAppViews
    └── Sources/MyAppViews/
        └── TemperatureDial.swift
```

The views you want to call from Rust are top-level `public struct`s conforming to `View`, with a
`public init` whose parameters are `String`, `Int`, `Double`, or `Bool`:

```swift
import SwiftUI

public struct TemperatureDial: View {
    let room: String
    let celsius: Double

    public init(room: String, celsius: Double) {
        self.room = room
        self.celsius = celsius
    }

    public var body: some View {
        VStack {
            Gauge(value: celsius, in: 0...40) { Text(room) }
            Text("\(celsius, specifier: "%.1f") °C")
        }
    }
}
```

It's a real package: it can depend on other SwiftPM packages, and `swift test` works in it.
Internal types stay internal — only public `View` structs are exported. Views whose init uses
other types (a model struct, a closure) are skipped with a build warning; the escape hatch below
covers them.

## 2. Declare it

Two additions to the app's `Cargo.toml`:

```toml
[dependencies]
day-piece-swiftui = { git = "https://github.com/daybrite/day.git" }

[package.metadata.day.ios]
swift-packages = [{ path = "swiftui", products = ["MyAppViews"] }]
platform = "16.0"        # optional: raise the floor for newer SwiftUI APIs

[package.metadata.day.macos]
swift-packages = [{ path = "swiftui", products = ["MyAppViews"] }]
platform = "13.0"
```

And one module in `src/lib.rs`, next to the `res` module the scaffold already has:

```rust
pub mod swiftui {
    include!(concat!(env!("OUT_DIR"), "/day_swiftui.rs"));
}
```

That's the whole setup. Your `build.rs` (the scaffold's `day_build::generate_resources()`) scans
the package and writes one constructor per exported view; `day build` compiles the package into
the app and generates the hosting glue. On iOS the package joins the generated `DayPieces`
SwiftPM package the Xcode scaffold already links; on macOS `day build` runs a `swift build`
prepass and statically links the result, with no Xcode project involved.

## 3. Call it

The generated constructor mirrors the Swift init exactly: same name, same parameters, in order.
A renamed view or a changed parameter is a Rust compile error, the same contract as the
generated `res::` resource constants:

```rust
use day::prelude::*;

fn climate_card(temp: Signal<f64>) -> AnyPiece {
    if day_piece_swiftui::support() != Support::Native {
        return label(crate::res::str::not_on_this_platform()).any();
    }
    crate::swiftui::TemperatureDial(String::from("Living room"), move || temp.get())
        .frame(220.0, 220.0)
        .id("climate-dial")
        .any()
}
```

Two things to notice:

- **Arguments are reactive.** Each parameter takes a constant, a `Signal`, or a closure. When a
  reactive argument changes, Day re-invokes the view's initializer with the new values, and
  SwiftUI reconciles it like any parent-driven update, so `@State` inside the view survives.
- **Gate with `support()`, not `cfg`.** `day_piece_swiftui::support()` is `Native` only on
  `macos-appkit` and `ios-uikit`. A `#[cfg(target_os = "macos")]` is the wrong gate: it is also
  true on `macos-gtk` and `macos-qt`, where there is no AppKit view tree to host into.

The hosted view fills the space it's offered, like `image` or `canvas`; constrain it with
`.frame(w, h)` when it shouldn't.

## Keeping state across navigation

Leaving the view's branch (a tab switch, a `when()` going false, a page navigation) disposes
the hosting view and the `@State` it owns. When the view should hold its state instead, give it
a key:

```rust
crate::swiftui::TemperatureDial(String::from("Living room"), move || temp.get())
    .state_key("climate-dial")
```

Day then retains the hosting view under that key and hands the same instance back on the next
mount: sliders, scroll positions, `@State`, and `@StateObject` all survive, and the mount's
current arguments are re-applied. Two rules: at most one mounted view per key, and a key pins
its view for the app's lifetime; use it for the handful of views that want persistence, not
per-row content.

## When the scan isn't enough

For views the generated path can't express (an init taking a model type, a delegate,
dynamic content), use the provider escape hatch. Subclass the provider in Swift, name
it `@objc(DayView_<name>)`, and call it by name from Rust:

```swift
@objc(DayView_history_chart)
final class HistoryChartProvider: DaySwiftUIProvider {
    override func body(_ params: String?) -> AnyView {
        AnyView(HistoryChart(model: decode(params)))
    }
}
```

```rust
day_piece_swiftui::swiftui("history_chart")
    .params(move || samples_as_json.get())
```

`params` is one JSON string, reactive like the typed arguments. The generated constructors are
this same mechanism with the ceremony generated for you.

## Pitfalls

- **Localized labels don't localize themselves.** Strings inside the Swift package don't go
  through Fluent. Pass them in as arguments (`res::str::…().format()` closures on the Rust
  side), so the hosted view follows the app's locale, including right-to-left layout, which the
  hosting view inherits.
- **Declare the floor you need.** SwiftUI APIs like `Grid` need iOS 16 / macOS 13; declare `platform`
  and `day build` raises the deployment target for you. Xcode ⌘R builds don't see the override;
  raise `platform/ios/DayApp.xcodeproj` by hand if you build from the IDE.
- **Provider not found renders `⟨name?⟩`.** A misspelled `@objc(DayView_…)` name or a package
  missing from the metadata shows a visible error view rather than crashing; the
  [reference](/docs/internal/swiftui) has the checklist.

## Reference

[swiftui](/docs/internal/swiftui) — the scanned subset, hosting and ownership details, the
macOS build leg, and every v1 limit. The extension model behind it is
[extending](/docs/extending).
