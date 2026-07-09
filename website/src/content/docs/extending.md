---
title: The extension model
description: "How new widgets and capabilities plug into Day as ordinary crates: composite pieces, native pieces, parts, and the registration machinery."
order: 40
section: Extend
---

Day's core widget vocabulary is small on purpose, and the framework expects to be extended. The
extension model has one organizing idea: **an extension is an ordinary Cargo crate** — you depend
on it, it registers itself, and the build tooling aggregates whatever native baggage it brings.
Nothing about extending Day involves forking it or editing generated projects.

There are three tiers, ordered by cost. Use the cheapest one that works.

## Tier 0 — composite pieces: pure composition

A composite piece is Rust code that arranges existing Pieces. No native code, no registration —
it works on every target automatically because it bottoms out in Pieces that already do.

```rust
pub fn rating(value: Signal<usize>) -> Rating { … }   // a row of tappable canvas stars

// consumers:
rating(stars).max(5).editable(true)
```

Most reusable UI in a Day app is this tier: cards, badges, form rows, charts drawn with
`canvas`. The shipped `day-piece-rating` and `day-piece-activity` crates are composite pieces,
and the [composite piece tutorial](/docs/tutorial-composite-piece) builds one end to end.

## Tier 1 — native pieces: a new leaf widget per toolkit

When the platform has a control Day doesn't wrap — a combo box, a web view, a map — you write a
**native piece**: one cross-platform front end plus a renderer per toolkit you support.

The front end defines the piece's identity and its props/patch protocol, and creates a leaf node:

```rust
const KIND: &str = "combo-box";

pub fn combo_box(items: Signal<Vec<String>>, selected: Signal<Option<usize>>) -> AnyPiece {
    piece_fn(move |cx| {
        let node = cx.leaf(KIND, &ComboProps { … }, Flex::default());
        bind_seeded(…, move |items| tree.patch(node, ComboPatch::Items(items), true));
        cx.on(node, move |ev| if let Event::SelectionChanged(i) = ev { … });
        node
    })
}
```

Each backend contributes `make` (create the native widget) and `update` (apply a patch),
registered at link time:

```rust
// inside #[cfg(feature = "appkit")] — creates an NSPopUpButton
day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: ComboProps, patch: ComboPatch,
    make: make, update: update);
```

The `renderer!` macro places an entry in the backend's link-time registry (a `linkme`
distributed slice), so the app that depends on your crate gets your renderer with zero
configuration — no plugin manifest, no runtime discovery, and an app that *doesn't* enable your
crate's feature for a given toolkit compiles none of it.

A piece that implements some toolkits and not others renders a labeled placeholder on the rest —
visible and honest rather than a crash — so coverage can grow toolkit by toolkit. The
[native piece tutorial](/docs/tutorial-native-piece) walks through all six desktop/mobile
backends for one control.

### Native dependencies without scaffold edits

Native pieces often wrap a platform *library* — Lottie's iOS framework, a Maps SDK's Gradle
artifact. A piece crate declares these in its Cargo metadata:

```toml
[package.metadata.day.ios]
swift-packages = [ … ]      # SwiftPM dependencies
frameworks = ["WebKit"]

[package.metadata.day.android]
java = ["java/"]            # Java sources shipped in the crate
gradle-dependencies = ["com.airbnb.android:lottie:6.4.0"]
permissions = ["android.permission.INTERNET"]
```

At build time, `day build` resolves every piece in your app's dependency graph via
`cargo metadata` and regenerates the glue the platform projects reference — a local SwiftPM
package for the Xcode side, a JSON manifest the Gradle build reads for Java sources,
dependencies, and merged permissions. Your checked-in platform scaffolds never change; only
generated, gitignored files do. (This is the same architecture Flutter uses for plugin
registration, adapted to Cargo.)

## Tier 2 — polyglot pieces: native-language implementations

The design reserves a third tier: pieces implemented in a platform's own language (Swift, Kotlin,
C++) behind **dayffi**, a small versioned C ABI — a vtable of `make`/`update`/`measure`/
`command`/`destroy` plus host callbacks for events and async completion. This is the tier that
would make "wrap an arbitrary CocoaPod/AAR behind one Rust function" routine without writing any
FFI by hand.

Plainly: dayffi is specified in depth but not yet shipped. Today, tier 1 covers the same ground
with hand-written Rust FFI (`objc2`, `jni`, C++ shims), which is more work per platform but fully
supported. If your plan depends on tier 2, check the repository's current state before counting
on it.

## Parts: capabilities without UI

Extensions that don't render — battery, clipboard, Bluetooth — are [parts](/docs/parts), which
skip all of the above machinery: no kind, no renderer, no registry, just `#[cfg]`-dispatched
functions (plus the same Cargo-metadata mechanism when Android needs Java or permissions). The
[part tutorial](/docs/tutorial-part) covers six platform implementations of one API.

## Choosing a tier

```text
does it render anything?
 ├─ no  → part
 └─ yes → can you build it from existing pieces (incl. canvas)?
           ├─ yes → composite piece            (works everywhere, free)
           └─ no  → native piece               (per-toolkit renderers, placeholder elsewhere)
                     └─ implementation must live in Swift/Kotlin itself?
                         → dayffi tier — designed, not yet available
```

Whichever tier, the packaging story is identical: publish a crate. Consumers add one dependency
line, and localization files and assets inside your crate aggregate into their app under your
package's namespace.
