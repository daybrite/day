---
title: "SwiftUI embedding"
description: "Host your own SwiftUI views inside the Day tree on macOS and iOS, with typed Rust constructors generated from your Swift package."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# SwiftUI embedding (macos-appkit + ios-uikit)

> **Status: implemented** as `day-piece-swiftui` plus build support in day-cli and day-build.
> An app (or a piece crate) ships ordinary SwiftUI code in an ordinary SwiftPM package; `day build`
> compiles it into the app on the two Apple-native backends and wraps each exported view in a
> hosting view, and day-build generates typed Rust constructors so the call-site is
> `crate::swiftui::MyView(param1, param2)`. Verified end to end in Day-Showcase's Benchmark page
> (the hosted SwiftUI twin of the Day-Bench Grids benchmark).

## The typed path

Write a normal SwiftPM package inside the app repo. It has its own module and its own transitive
SwiftPM dependencies, and `swift test` tests it:

```
swiftui/
  Package.swift
  Sources/MyViews/Hello.swift     public struct Hello: View { public init(name: String) { … } }
  Tests/…
```

Declare it once in the app's `Cargo.toml`, add the piece dependency, and surface the generated
module in lib.rs:

```toml
[dependencies]
day-piece-swiftui = { git = "https://github.com/daybrite/day.git" }

[package.metadata.day.ios]
swift-packages = [{ path = "swiftui", products = ["MyViews"] }]
platform = "16.0"          # optional: raise the deployment floor for newer SwiftUI APIs

[package.metadata.day.macos]
swift-packages = [{ path = "swiftui", products = ["MyViews"] }]
platform = "13.0"
```

```rust
pub mod swiftui {
    include!(concat!(env!("OUT_DIR"), "/day_swiftui.rs"));
}
```

Every top-level `public struct … : View` in the package whose first `public init` uses only
supported parameter types gets two generated halves:

- **Rust** (day-build, from the app's `build.rs`; it works on every host without a Swift
  toolchain): a
  constructor mirroring the Swift identity verbatim, `crate::swiftui::Hello(name)`. Each argument
  takes a constant, a `Signal`, or a closure (`IntoReactive`); reactive arguments re-invoke the
  view's initializer live, and SwiftUI diffing preserves the view's `@State` across those updates.
- **Swift glue** (`day build`, staged into the generated `DayPieces` module): an
  `@objc(DayView_MyViews_Hello)` provider that decodes the JSON params and calls the real
  initializer, so a mis-parsed signature fails the Swift compile with a readable error instead of
  shipping.

A renamed view or a changed parameter is then a Rust compile error at the call site, the same
contract the generated `res::str`/`res::images` constants give resources.

### The scanned subset

The scan is a text parse (day-build `swiftui::scan_package`), shared verbatim by the Rust-binding
and Swift-glue generators. A view is exported when all of these hold; anything else is skipped
with a build warning naming the reason:

- a **top-level**, **non-generic** `public struct` whose declaration's inheritance clause names
  `View` (conformance added in an `extension` is not seen);
- its **first `public init`** (the exported constructor, by contract, so a reordered overload
  cannot silently switch what the binding calls) has parameters typed only `String`, `Int`,
  `Double`, or `Bool`, each a plain parameter without a default value, an attribute
  (`@ViewBuilder`), `inout`, or a variadic marker.

Exported view names must be unique across the app's packages (`crate::swiftui` is flat).
Views the subset cannot express are still embeddable through the provider escape hatch below.

## The provider escape hatch

The generated glue is itself written against a public two-piece contract, usable directly for
views that need custom wiring (delegates, dynamic content, hosting configuration):

```swift
// anywhere in the app's staged Swift (`swift = ["dir"]`) or a scanned package
@objc(DayView_hello)
final class HelloProvider: DaySwiftUIProvider {
    override func body(_ params: String?) -> AnyView {
        AnyView(Hello(name: params ?? "world"))
    }
}
```

```rust
use day_piece_swiftui::swiftui;

swiftui("hello")                  // resolves @objc(DayView_hello) — dots become underscores
    .params(move || json)         // optional; reactive, re-invokes body(_:) live
    .frame(320.0, 240.0)          // it's a growing leaf, so constrain it (or let it fill)
```

`DaySwiftUIProvider` and the `DayView_<name>` lookup live in the shim `day-piece-swiftui` stages
into the generated `DayPieces` module (`apple/swift/DaySwiftUI.swift`). Resolution is one
`NSClassFromString` call at mount, so nothing registers at startup. The same string contract lets
a future Jetpack Compose leg resolve with `Class.forName`, which is why the naming carries no
Apple-specific structure. `day_piece_swiftui::support()` reports `Native` only on macos-appkit and
ios-uikit; gate UI on it (never on backend-feature `cfg`s: `target_os = "macos"` also covers
macos-gtk and macos-qt, which have no AppKit view tree).

## Params and `@State`

Params cross the bridge as one JSON string (the typed path composes it from the constructor
arguments; `day_piece_swiftui::json` renders without a serde dependency). On every change the
native half re-invokes the provider's `body(_:)` and assigns the hosting view's `rootView`.
SwiftUI reconciles that like any parent-driven update: state owned by the view (`@State`,
`@StateObject`) survives as long as `body` keeps returning the same underlying view type. The
generated glue always does; hand-written providers must (return `AnyView(MyView(…))` every call,
not different types per branch).

## State retention across unmount (`.state_key`)

Leaving the piece's branch (a tab switch, a `when()` going false, a page navigation) disposes
the node, and with it the hosting view and every `@State` it owned; the next mount starts fresh.
When the view should hold its state instead, give it a key:

```rust
crate::swiftui::BenchGridsView(…)     // or swiftui("…")
    .state_key("bench-grids")
```

The shim retains the hosting view under the key and hands the same instance back on the next
mount (sliders, scroll positions, and `@State`/`@StateObject` all survive) after re-invoking the
provider's body with that mount's params, so data that changed while unmounted (a locale switch)
still lands. Keys have these constraints:

- **At most one live instance per key.** Two mounted pieces sharing a key would contend for one
  native view; give each usage its own key.
- **A key pins its hosting view for the app's lifetime.** Meant for the handful of views that
  want persistence (a settings pane, a benchmark tab), not per-row list content.

The showcase's Benchmark page pairs this with app-global signals on the Day-native tab, so both
tabs keep their parameters across tab switches and page revisits alike.

## Per-platform realization

| | macOS (AppKit) | iOS (UIKit) |
|---|---|---|
| host | `NSHostingView<AnyView>` | `UIHostingController<AnyView>`'s view |
| retention | provider via associated object | provider + controller via associated objects |
| ownership | shim returns +1-retained; Rust takes it as `Retained<NSView>` | same, `Retained<UIView>` |
| build | generated SwiftPM package at `build/day/macos/DayPieces`, referenced by the `platform/macos/` scaffold and built by its xcodebuild | the existing `build/day/ios/DayPieces` package, built by the scaffold's xcodebuild |

The hosting view is an ordinary native handle to Day: framed, measured (`fill_measure`: it fills
what it is offered; constrain with `.frame`), snapshotted, and disposed like a built-in.
`UIHostingController` is not parented to a view controller in v1, so UIKit appearance
callbacks do not fire inside the hosted view.

### The macOS leg

macos-appkit builds through the `platform/macos/DayApp.xcodeproj` host project (DESIGN §16.5),
whose pbxproj references the generated `DayPieces` package directly, the same shape as iOS.
`day build` regenerates `build/day/macos/DayPieces` (staged shims + generated glue +
`Package.swift`) before every xcodebuild, touching only files whose bytes changed so the Swift
incremental build stays warm, and writes it even when nothing contributes Swift: the pbxproj's
package reference must resolve, so an empty package still exists. xcodebuild compiles and links
the package with the Runner; the installed binary uses the OS Swift dylibs (macOS ≥ 10.14.4).
`day pack -p macos-appkit` needs nothing extra: the Swift code is inside the bundle's binary and
codesign/notarize are unchanged. (The retired bare-cargo build carried its own `swift build`
prepass and `cargo rustc` link; both went with that path in 2026-08.)

### Deployment floors

The generated packages default to iOS 16 / macOS 13; a contribution's `platform` key raises the
floor (the max across contributions wins). On iOS the raise must also reach the app target;
`day build` passes `IPHONEOS_DEPLOYMENT_TARGET=<floor>` to xcodebuild, which covers the app and
the SwiftPM package targets without editing the scaffold. Command-line settings do not apply to
⌘R builds inside Xcode, so for IDE work raise `IPHONEOS_DEPLOYMENT_TARGET` in
`platform/ios/DayApp.xcconfig` (a user-raised value is never lowered).

## Failure behavior

A missing provider class, or params the generated `Decodable` cannot decode, hosts a visible
`⟨name?⟩` error view (and logs the expected class name), matching Day's placeholder-leaf
convention, so the failure shows on screen instead of crashing or rendering blank. Views the scan
skips are reported as build warnings with the reason.

If a provider is not found, check three things: (1) the class name must be exactly
`@objc(DayView_<name>)`, with dots mapped to underscores; (2) the file's dir or package must be
declared under `[package.metadata.day.ios/macos]`; (3) stripping, where on macOS the
`-force_load` covers this, and on iOS you check `nm <app> | grep DayView_` and file a bug if the
class is absent.

## v1 limits

- Sizing is fill-only (`fill_measure`); intrinsic sizing (`sizeThatFits`) is future work.
- Params flow one way (Rust → Swift). Events out of the hosted view need the escape hatch plus an
  app-defined channel for now.
- The scan takes the first public init only, and no default arguments.
- The hosted view has no `UIViewController` parent on iOS.
- `.state_key` retention has no eviction: each key holds its hosting view until the app exits.

## Compose (planned, not built)

The `DayView_<name>` naming, the JSON params channel, and the piece front-end all transfer to an
android-mdc leg hosting Jetpack Compose views
(`Class.forName` + `AbstractComposeView`). That leg does not exist yet; this note records the
constraint that it must stay possible.
