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

Day's core widget vocabulary is small on purpose, and the framework expects to be extended. The
extension model has one organizing idea: **an extension is an ordinary Cargo crate**. You depend
on it, it registers itself, and the build tooling aggregates whatever native baggage it brings.
Nothing about extending Day involves forking it or editing generated projects.

There are three tiers, ordered by cost. Use the cheapest one that works, and note that when you
only need to *configure* an existing widget rather than build a new one, that's not an extension
tier at all but a [tweak](/docs/tweaks), which is cheaper than everything below.

## Tier 0: composite pieces, pure composition

A composite piece is Rust code that arranges existing Pieces. No native code, no registration;
it works on every target automatically because it bottoms out in Pieces that already do.

```rust
pub fn rating(value: Signal<usize>) -> Rating { … }   // a row of tappable canvas stars

// consumers:
rating(stars).max(5).editable(true)
```

Most reusable UI in a Day app is this tier: cards, badges, form rows, charts drawn with
`canvas`. The shipped `day-piece-rating` and `day-piece-settings` crates are composite pieces,
and the [composite piece tutorial](/docs/tutorial-composite-piece) builds one end to end.

## Tier 1: native pieces, a new leaf widget per toolkit

When the platform has a control Day doesn't wrap (a combo box, a web view, a map), you write a
**native piece**: one cross-platform front end plus a renderer per toolkit you support.

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

Each backend contributes `make` (create the native widget) and `update` (apply a patch),
registered at link time:

```rust
// inside #[cfg(feature = "appkit")] — creates an NSComboBox
day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: ComboProps, patch: ComboPatch,
    make: make, update: update);
```

The `renderer!` macro places an entry in the backend's link-time registry (a `linkme`
distributed slice), so the app that depends on your crate gets your renderer with zero
configuration: no plugin manifest, no runtime discovery, and an app that *doesn't* enable your
crate's feature for a given toolkit compiles none of it.

Two companion macros round this out. `day_pieces::glue_modules!(appkit, gtk, …)` declares the
feature-and-target-gated module index binding each `lib-<toolkit>.rs` — the one-liner every
shipped piece uses instead of a hand-written `#[cfg]`/`#[path]` block. And web is the one
exception to link-time registration: `linkme` has no wasm32 implementation, so a web-dom
renderer uses `dom_renderer!` and registers at runtime from the piece's own constructor.

A piece that implements some toolkits and not others renders a labeled placeholder on the rest
(visible rather than a crash), so coverage can grow toolkit by toolkit. The
[native piece tutorial](/docs/tutorial-native-piece) walks through all six desktop/mobile
backends for one control.

### Native dependencies without scaffold edits

Native pieces often wrap a platform *library*: Lottie's iOS framework, a Maps SDK's Gradle
artifact. A piece crate declares these in its Cargo metadata:

```toml
[package.metadata.day.ios]
swift = ["ios/swift"]       # Swift shim sources shipped in the crate
swift-packages = [ … ]      # SwiftPM dependencies; a local entry
                            # ({ path = "swiftui", products = ["MyViews"] }) is scanned for
                            # SwiftUI views and exported as typed Rust bindings
frameworks = ["WebKit"]
# platform = "16.0"         # minimum-OS floor (max across crates wins)

[package.metadata.day.macos]   # same shape as .ios, for the macos-appkit leg
swift-packages = [ … ]

[package.metadata.day.android]
java = ["android/java"]     # Java sources shipped in the crate
gradle-dependencies = ["com.airbnb.android:lottie:6.4.0"]
permissions = ["android.permission.INTERNET"]
# also: res, gradle-repositories, proguard, manifest-components

[package.metadata.day.ohos]
ets = ["ohos/ets"]          # ArkTS components (HarmonyOS)

[package.metadata.day.permissions]
uses = ["camera"]           # portable permission names, mapped per platform
```

The [extending reference](/docs/internal/extending) documents every key.

At build time, `day build` resolves every piece in your app's dependency graph via
`cargo metadata` and regenerates the glue the platform projects reference: a local SwiftPM
package for the Xcode side, a JSON manifest the Gradle build reads for Java sources,
dependencies, and merged permissions. On macos-appkit the same aggregation produces
`build/day/macos/DayPieces`, which its Xcode host project references — or which a `swift build`
prepass compiles and links into the cargo binary, on the bare-cargo path. Your checked-in platform scaffolds never change; only
generated, gitignored files do. (This is the same architecture Flutter uses for plugin
registration, adapted to Cargo.)

## Tier 2: native-language halves

The original design reserved a third tier for pieces implemented in a platform's own language
(Swift, Kotlin, C++) behind **dayffi**, a versioned C ABI. It was never built, and it is now
retired: none of it turned out to be needed (DESIGN.md §15.3). What shipped instead is the
ladder above — tweaks, then composition, then Rust renderers — plus **native halves**: the
crate ships its own Swift, Java, ArkTS, or C++ sources, declares them under
`[package.metadata.day.<platform>]`, and its tier-1 Rust renderer adopts the views those shims
create.

For code that must be *written* in Swift, that need is covered today by
[SwiftUI embedding](/docs/internal/swiftui): a SwiftPM package's public views become typed Rust
constructors (`crate::swiftui::MyView(…)`) on macos-appkit and ios-uikit. The matching
Kotlin/Compose leg is not built yet.

## Parts: capabilities without UI

Extensions that don't render (battery, clipboard, Bluetooth) are [parts](/docs/parts), which
skip all of the above machinery: a part is plain `#[cfg]`-dispatched functions, with no kind,
renderer, or registry (plus the same Cargo-metadata mechanism when Android needs Java or permissions). The
[part tutorial](/docs/tutorial-part) covers six platform implementations of one API.

## Choosing a tier

```text
does it render anything?
 ├─ no  → part
 └─ yes → is it an EXISTING widget that just needs configuring?
           ├─ yes → tweak                      (/docs/tweaks — cheapest of all)
           └─ no  → can you build it from existing pieces (incl. canvas)?
                     ├─ yes → composite piece  (works everywhere, free)
                     └─ no  → native piece     (per-toolkit renderers, placeholder elsewhere)
                               └─ implementation must live in Swift/Kotlin itself?
                                   ├─ Swift  → SwiftUI embedding (/docs/internal/swiftui)
                                   └─ Kotlin → Compose leg not built yet
```

Whichever tier, the packaging story is identical: publish a crate. Consumers add one dependency
line, and localization files and assets inside your crate aggregate into their app under your
package's namespace.
