# Standalone pieces (front-end **and** backend, zero core edits)

A **piece** is a reusable Day widget. Day ships built-in pieces (`button`, `slider`, `list`, …), but
anyone can publish a piece as an independent crate that adds **both** its cross-platform front-end
(Rust) **and** its per-toolkit native backend (Objective-C via objc2, C++ shims, Android Java, …) —
with **no edits to any core Day crate**. `day-piece-picker` is the reference implementation.

**Scaffold a new piece with `day new`.** Don't hand-assemble the crate — `day new piece <name>`
generates a ready-to-build project (remote Day deps by default; `--local <path>` for a local Day
checkout). With no `--toolkits` it emits a **composite** piece (front-end only); with
`--toolkits appkit,gtk,qt,uikit,widget,winui` (any subset) it emits a **native** piece with a renderer
per backend plus the C++/Java/Swift glue each one needs. The companion `day new part <name>` scaffolds
a headless part. For full walkthroughs see the tutorials:
[composite piece](https://daybrite.dev/docs/tutorial-composite-piece/),
[native piece](https://daybrite.dev/docs/tutorial-native-piece/), and
[part](https://daybrite.dev/docs/tutorial-part/).

The whole extensibility story rests on two mechanisms:

1. **Renderers register link-time** into each backend's `RENDERERS` slice (via `linkme`), so a backend
   dispatches an unknown `kind` to the piece's `make`/`update`/`measure` with zero registry edits.
2. **Native backend assets** (C++ shims, Android Java, Gradle deps) are declared in the crate's own
   `Cargo.toml` / `build.rs` and folded into the app's native build automatically.

## 1. The front-end (any backend)

```rust
use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_reactive::{Signal, bind_seeded};
use day_spec::Event;

pub const KIND: &str = "my.piece.gauge";

pub struct Gauge { /* … + a Signal for two-way binding */ }
impl Piece for Gauge {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let node = cx.leaf(KIND, &props, Flex::default());   // a native leaf of `KIND`
        bind_seeded(seed, move || value.get(), move |v| { with_tree(|t| t.patch(node, patch, false)); });
        cx.on(node, move |ev| { /* native events → write the Signal */ });
        node
    }
}
```

`impl Piece` gives you `.id()`/`.a11y()`/`.frame()` for free (blanket `Decorate`). Props are the full
realize payload; a sparse `Patch` enum carries changes.

## 2. Per-backend renderers — the `renderer!` macro

Each backend module registers its native renderer into that backend's `RENDERERS` slice — the same
slice the built-ins use, so **no Day edit is needed**. Write **typed** `make`/`update` (no `&dyn Any`
downcast) and one macro line; `day_pieces::renderer!` expands to the `linkme` registration + the props/
patch downcast:

```rust
#[cfg(all(feature = "appkit", target_os = "macos"))]
mod appkit_impl {
    use super::*;
    fn make(backend: &mut AppKit, props: &MyProps, id: NodeId) -> Retained<NSView> { … }
    fn update(backend: &mut AppKit, h: &Retained<NSView>, patch: &MyPatch) { … }

    day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
        kind: KIND, props: MyProps, patch: MyPatch, make: make, update: update);
}
```

Add `measure: f` for custom sizing — `measure: day_pieces::fill_measure` for a growing leaf (a web
view, a canvas), omit it for the backend's default. A **patchless** piece (configured once, e.g. Lottie)
drops `patch:`/`update:`: `renderer!(day_uikit::RENDERERS, Uikit, kind: KIND, props: MyProps, make: make)`.
Do the same for `day_gtk::RENDERERS`, `day_qt::RENDERERS`, `day_uikit::RENDERERS`,
`day_android::RENDERERS`, `day_winui::RENDERERS`. Each backend is behind a cargo feature that pulls in
that toolkit crate; the app enables `my-piece/<backend>` alongside `day/<backend>`.

**Reporting events back.** A renderer calls `day_<backend>::emit(node, event)`. Beyond the fixed
`Event` variants, a piece defines its own event with `Event::custom("my:tag", text)` (in-process) — its
`cx.on` reads it. Across a native boundary (JNI/C-ABI) the tag can't be a `&'static str`, so it's empty
and the payload rides in `num`/`text`: on Android the shim calls `DayBridge.nativeOnEvent(id, 12, num,
text)` (kind 12 = the open Custom channel). `day-piece-webview` reports its URL this way.

## 3. Native backend assets (the interesting part)

A piece often needs native code the Rust FFI alone can't express. Day gives each toolkit a **local**
extension path so it lives in the piece crate:

### C++ shims — Qt & WinUI (`build.rs`)

The piece carries its OWN `src/lib-qt-shim.cpp` / `src/lib-winui-shim.cpp` and compiles them in `build.rs`
(gated on the feature). Qt widgets are plain C++ objects and the handle is a raw `QWidget*`, so a Qt
shim is self-contained. WinUI handles are a private boxed type owned by `day-winui-sys`, so the piece
boxes its XAML element through the exported **`day_winui_box` / `day_winui_unbox`** seam (a stable
WinRT COM-ABI). Both reuse the sys crate's generic `measure` (`day_qt_size_hint` / `day_winui_measure`).
See `pieces/day-piece-picker/{build.rs,src/lib-qt-shim.cpp,src/lib-winui-shim.cpp}`.

### Android Java + Gradle deps (`[package.metadata.day.android]`)

The piece carries its own Java/Kotlin under a crate dir and declares it in `Cargo.toml`:

```toml
[package.metadata.day.android]
java = ["android/java"]                                        # → Gradle java srcDirs
gradle-dependencies = ["com.google.android.material:material:1.11.0"]   # → app dependencies { }
gradle-repositories = ["https://jitpack.io"]                  # → extra Maven repos (optional)
permissions = ["android.permission.INTERNET"]                 # → <uses-permission> in the manifest
```

`day build` (for `android-widget`) runs `cargo metadata`, walks the app's dependency closure, collects
every piece's contributions, and writes `build/day/android/day-pieces.json`. The app's checked-in
`platform/android/{app/build.gradle.kts,settings.gradle.kts}` read that file **generically** (a loop —
no per-piece edits, ever) and add the Java dirs, dependencies, and repos.

**Manifest permissions.** A piece that needs a permission (a web view needs `INTERNET`) can't reach the
app's `AndroidManifest.xml`, so `day build` also writes the collected permissions into a generated
**overlay manifest** (`build/day/android/day-pieces-manifest.xml`). The scaffold points its debug +
release source-set manifests at that overlay, and AGP's manifest merger folds the `<uses-permission>`
entries into the app manifest (deduping against any the app already declares). So a WebView-using app
needs no manual manifest edit — the piece declares the permission and it just appears. `day-piece-webview`
is the reference. (A piece can only add a permission; it never removes or narrows the app's own.)

The piece's Java uses day-android's **public** surface only — `DayBridge.ctx` (the `Context`) and
`DayBridge.nativeOnEvent(id, kind, num, str)` (the event trampoline, `kind` per §14.2, `4` = selection).
The Rust side calls its OWN Java class through the re-exported `jni` (`with_env` + `call_static_method`
+ `AHandle`); `day_android::make_view` is a convenience hardcoded to `DayBridge`, so a standalone piece
uses raw `call_static_method` on its class. See `pieces/day-piece-picker/android/java/dev/daybrite/day/piece/picker/DayPicker.java`.

> **Gradle configuration cache.** The scaffold reads `day-pieces.json` at *configuration* time, and
> `day build` rewrites it every build; the config cache can't track that read, so it would serve stale
> piece contributions. The scaffold ships with `org.gradle.configuration-cache=false` for this reason.
> Some pieces also pull libraries that require AndroidX (Lottie's view extends `AppCompatImageView`), so
> the scaffold sets `android.useAndroidX=true`.

### iOS Swift shims + SwiftPM packages (`[package.metadata.day.ios]`)

Many iOS libraries (and any Swift class with a non-`@objc` API) can't be driven from Rust directly, and
they ship as **SwiftPM packages**. A piece declares both — a Swift shim it carries, and the packages it
needs — in its `Cargo.toml`:

```toml
[package.metadata.day.ios]
swift = ["ios/swift"]                                         # dirs of Swift shim sources
swift-packages = [                                           # SwiftPM package dependencies
  { url = "https://github.com/airbnb/lottie-ios", from = "4.5.0", products = ["Lottie"] },
]
frameworks = ["WebKit"]                                      # system frameworks to link
```

`frameworks` links system frameworks via the generated package's `linkerSettings` — so a piece that
drives a class from an unlinked framework (e.g. a hand-rolled `WKWebView`) declares it here instead of
`dlopen`ing or hand-`#[link]`ing (which doesn't survive the cargo-staticlib → xcode link). `day-piece-webview`
uses `frameworks = ["WebKit"]`.

`{from, exact, branch, revision}` map to the matching SwiftPM version requirement; `products` are the
library products to link. Xcode is not script-driven like Gradle, so `day build` (ios-uikit) instead
generates a **local SwiftPM package** at `build/day/ios/DayPieces` — its `Package.swift` lists every
piece's `swift-packages` as dependencies and compiles every piece's staged Swift shims (each under a
per-crate subfolder). The app's checked-in `.xcodeproj` depends on that **one** local package (the iOS
analog of the checked-in Gradle scaffold — a `XCLocalSwiftPackageReference` + a product dependency in a
Frameworks phase). So adding an iOS piece is pure `Cargo.toml` data — **no `.xcodeproj` edits, ever**.

The Swift shim exposes a flat C ABI (`@_cdecl`) that the piece's Rust calls (mirroring the Android Java
shim); it `import`s the SwiftPM product and returns a native `UIView` that Rust wraps via
`Retained::from_raw`. See `pieces/day-piece-lottie/{ios/swift/DayLottie.swift,src/lib-uikit.rs}`.

## 4. Cargo wiring

```toml
[features]
appkit = ["dep:day-appkit", …]
gtk = ["dep:day-gtk", "dep:gtk4"]
qt = ["dep:day-qt"]                 # + a build.rs that compiles src/lib-qt-shim.cpp
uikit = ["dep:day-uikit", …]
widget = ["dep:day-android"]        # + [package.metadata.day.android]
winui = ["dep:day-winui", "dep:day-winui-sys"]   # + build.rs compiles src/lib-winui-shim.cpp
```

The app mirrors each: `my-piece/<backend>` in the matching feature. That's it — no changes to `day`,
the toolkit crates, the CLI, or the Gradle scaffold are needed to add a piece.

## Reference

`pieces/day-piece-picker` implements all of the above: three SwiftUI-style stylings, six backends, its
own Qt + WinUI C++ shims, and its own Android Java — verified on AppKit / GTK / Qt / iOS / Android, and
CI-built on WinUI. Use it as a template. Its layout keeps the shared front-end and each toolkit backend
in a separate file:

```
pieces/day-piece-picker/
├── Cargo.toml               # features + [package.metadata.day.android]
├── build.rs                 # compiles lib-qt-shim.cpp / lib-winui-shim.cpp per feature
├── android/java/…/DayPicker.java   # this piece's own Android backend
└── src/
    ├── lib.rs               # front-end (the `Piece`) + a `#[cfg]/#[path] mod` index of backends
    ├── lib-appkit.rs        # one file per toolkit renderer …
    ├── lib-gtk.rs
    ├── lib-qt.rs            (+ lib-qt-shim.cpp)
    ├── lib-uikit.rs
    ├── lib-android.rs      (+ android/java DayPicker.java)
    ├── lib-winui.rs        (+ lib-winui-shim.cpp)
    ├── lib-qt-shim.cpp
    └── lib-winui-shim.cpp
```

`lib.rs` declares each backend with `#[cfg(…)] #[path = "lib-<toolkit>.rs"] mod …_impl;`, so every file
is compiled only for its feature+target and the whole native surface for a toolkit lives in one place.

`pieces/day-piece-webview` (see [webview.md](webview.md)) is a second reference — a heavier native
backend (an embedded browser) that additionally contributes an Android permission, hand-rolls the iOS
`WKWebView` (`dlopen`-ing WebKit.framework so the piece stays self-contained), and returns the proposal
from `measure` so a growing leaf fills on Android.

`pieces/day-piece-lottie` (see [lottie.md](lottie.md)) is a third reference — an iOS/Android-only piece
that pulls an EXTERNAL native package on each platform: the **lottie-ios SwiftPM package** (via the
`[package.metadata.day.ios]` mechanism above) and **`com.airbnb.android:lottie`** (Gradle). Its Swift
and Java shims each wrap a `LottieAnimationView` behind a flat C ABI / static method.

`parts/day-part-battery` (see [battery.md](battery.md)) is a fourth reference — the first **part**:
a **headless** crate with no UI Piece at all. Where `pieces/` holds UI-library extensions (each
registers a renderer), `parts/` is its non-UI corollary — capability crates (`day-part-*`) that extend
Day apps with platform services. It shows the backend-contribution mechanism accommodates non-UI
capabilities: it contributes Android Java through `[package.metadata.day.android]` (for
`BatteryManager`) but registers nothing into any `RENDERERS` slice, and selects its per-OS impl by
`#[cfg(target_os)]` rather than a toolkit feature. Any Rust code can depend on it and call
`day_part_battery::status()`.
