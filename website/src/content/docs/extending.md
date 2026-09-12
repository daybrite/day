---
title: The extension model
description: "How new widgets and capabilities plug into Day as ordinary crates: composite pieces, native pieces, parts, and the registration machinery."
order: 40
section: Extend
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day keeps its core widget vocabulary small and expects to be extended. Every extension is an
ordinary Cargo crate. Add it as a dependency; Day’s build tooling incorporates its declared
native sources and libraries, and native renderers register with the relevant backends.

The three approaches below require different amounts of platform-specific code. Start with
composition when existing pieces provide the behavior you need. To configure an existing
native widget, use a [tweak](/docs/tweaks).

## Tier 0: pieces composed from existing pieces

A composite [piece](/docs/glossary#piece) is Rust code that arranges existing Pieces. It needs no native code or
registration, and it works on every [target](/docs/glossary#target) automatically because it bottoms out in Pieces that
already do.

```rust
pub fn rating(value: Signal<usize>) -> Rating { … }   // a row of tappable canvas stars

// consumers:
rating(stars).max(5).editable(true)
```

Most reusable UI in a Day app is this tier: cards, badges, form rows, charts drawn with
`canvas`. The shipped `day-piece-rating` and `day-piece-settings` crates are composite pieces,
and the [composite piece tutorial](/docs/tutorial-composite-piece) provides a complete example.

## Tier 1: a native leaf widget per toolkit

When the platform has a control Day doesn't wrap (a combo box, a web view, a map), you write a
**native piece**: one cross-platform front end plus a renderer per [toolkit](/docs/glossary#toolkit) you support.

The front end defines the piece's identity and its props/patch protocol, and creates a leaf node
(abridged from `pieces/day-piece-combobox`):

```rust
pub const KIND: &str = "day.piece.combobox";

/// A config struct in the usual builder shape; `impl Piece` does the wiring.
pub fn combo_box(items: Signal<Vec<String>>, text: Signal<String>) -> ComboBox { … }

impl Piece for ComboBox {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let node = cx.leaf(KIND, &ComboProps { … }, Flex { grow_w: true, ..Default::default() });
        bind_seeded(initial_items, move || items.get(), move |v: &Vec<String>| {
            with_tree(|t| t.patch(node, Box::new(ComboPatch::Items(v.clone())), true));
        });
        cx.on(node, move |ev| if let Event::TextChanged(t) = ev { /* write the signal */ });
        node
    }
}
```

Each [backend](/docs/glossary#backend) contributes `make` (create the native widget) and `update` (apply a patch),
registered at link time:

```rust
// inside #[cfg(feature = "appkit")] — creates an NSComboBox
day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: ComboProps, patch: ComboPatch,
    make: make, update: update);
```

The `renderer!` macro places an entry in the backend's link-time registry (a `linkme`
distributed slice), so the app that depends on your crate gets your renderer through the linker
alone, with no manifest to write, and an app that doesn't enable your crate's feature for a given
toolkit compiles none of it.

Two companion macros round this out. `day_pieces::glue_modules!(appkit, gtk, …)` declares the
feature-and-target-gated module index binding each `lib-<toolkit>.rs`, the one-liner every
shipped piece uses in place of a hand-written `#[cfg]`/`#[path]` block. Web is the one
exception to link-time registration: `linkme` has no wasm32 implementation, so a web-dom
renderer uses `dom_renderer!` and registers at runtime from the piece's constructor.

A piece that implements some toolkits and not others renders a labeled placeholder on the rest,
so the gap is visible and coverage can grow toolkit by toolkit. The
[native piece tutorial](/docs/tutorial-native-piece) walks through all six desktop/mobile
backends for one control.

### Native dependencies without scaffold edits

Native pieces often wrap a platform *library*: Lottie's iOS framework, a Maps SDK's Gradle
artifact. A piece crate declares these in its Cargo metadata:

```toml
[package.metadata.day.ios]
swift = ["platform/ios/swift"]       # Swift shim sources shipped in the crate
swift-packages = [ … ]      # SwiftPM dependencies; a local entry
                            # ({ path = "swiftui", products = ["MyViews"] }) is scanned for
                            # SwiftUI views and exported as typed Rust bindings
frameworks = ["WebKit"]
# platform = "16.0"         # minimum-OS floor (max across crates wins)

[package.metadata.day.macos]   # same shape as .ios, for the macos-appkit leg
swift-packages = [ … ]

[package.metadata.day.android]
java = ["platform/android/java"]     # Java sources shipped in the crate
gradle-dependencies = ["com.airbnb.android:lottie:6.4.0"]
permissions = ["android.permission.INTERNET"]
# also: res, gradle-repositories, proguard, manifest-components

[package.metadata.day.ohos]
ets = ["platform/harmony/ets"]          # ArkTS components (HarmonyOS)

[package.metadata.day.permissions]
uses = ["camera"]           # portable permission names, mapped per platform
```

The [extending reference](/docs/internal/extending) documents every key.

At build time, `day build` resolves every piece in your app's dependency graph via
`cargo metadata` and regenerates the glue the platform projects reference: a local SwiftPM
package for the Xcode side, a JSON manifest the Gradle build reads for Java sources,
dependencies, and merged permissions. On macos-appkit the same aggregation produces
`build/day/macos/DayPieces`, which its Xcode host project references. Your checked-in platform scaffolds never change; only
generated, gitignored files do. (This is the same architecture Flutter uses for plugin
registration, adapted to Cargo.)

## Tier 2: native-language halves

Pieces implemented partly in a platform's language use **native halves**: the crate includes
Swift, Java, ArkTS, or C++ sources, declares them under `[package.metadata.day.<platform>]`, and
its tier-1 Rust renderer adopts the views those shims create.

For code that must be *written* in Swift, that need is covered today by
[SwiftUI embedding](/docs/internal/swiftui): a SwiftPM package's public views become typed Rust
constructors (`crate::swiftui::MyView(…)`) on macos-appkit and ios-uikit. The matching
Kotlin/Compose leg is not built yet.

## Parts: capabilities without UI

Extensions that don't render (battery, clipboard, Bluetooth) are [parts](/docs/parts), which
skip all of the above machinery: a [part](/docs/glossary#part) is plain `#[cfg]`-dispatched functions, with no [kind](/docs/glossary#kind),
renderer, or registry (plus the same Cargo-metadata mechanism when Android needs Java or permissions). The
[part tutorial](/docs/tutorial-part) covers six platform implementations of one API.

## Choosing a tier

```text
does it render anything?
 ├─ no  → part
 └─ yes → is it an existing widget that just needs configuring?
           ├─ yes → tweak                      (/docs/tweaks — configure a widget)
           └─ no  → can you build it from existing pieces (incl. canvas)?
                     ├─ yes → composite piece  (uses existing backends)
                     └─ no  → native piece     (per-toolkit renderers, placeholder elsewhere)
                               └─ implementation must live in Swift/Kotlin itself?
                                   ├─ Swift  → SwiftUI embedding (/docs/internal/swiftui)
                                   └─ Kotlin → Compose leg not built yet
```

Whichever tier you pick, you package it the same way, by publishing a crate. Consumers add one
dependency line, and localization files and assets inside your crate aggregate into their app
under your package's namespace.
