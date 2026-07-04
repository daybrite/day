# Standalone pieces (front-end **and** backend, zero core edits)

A **piece** is a reusable day widget. day ships built-in pieces (`button`, `slider`, `list`, …), but
anyone can publish a piece as an independent crate that adds **both** its cross-platform front-end
(Rust) **and** its per-toolkit native backend (Objective-C via objc2, C++ shims, Android Java, …) —
with **no edits to any core day crate**. `day-piece-picker` is the reference implementation.

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

## 2. Per-backend renderers

Each backend module registers a `Renderer<B>` into that backend's slice — the exact same slice the
built-ins use, so **no day edit is needed**:

```rust
#[cfg(all(feature = "appkit", target_os = "macos"))]
mod appkit_impl {
    #[linkme::distributed_slice(day_appkit::RENDERERS)]
    static R: fn() -> Renderer<AppKit> = || Renderer { kind: KIND, make, update, measure: Some(measure) };
    fn make(backend: &mut AppKit, props: &dyn Any, id: NodeId) -> Retained<NSView> { … }
    // update / measure …
}
```

Do the same for `day_gtk::RENDERERS`, `day_qt::RENDERERS`, `day_uikit::RENDERERS`,
`day_android::RENDERERS`, `day_winui::RENDERERS`. Each backend is behind a cargo feature that pulls in
that toolkit crate; the app enables `my-piece/<backend>` alongside `day/<backend>`.

## 3. Native backend assets (the interesting part)

A piece often needs native code the Rust FFI alone can't express. day gives each toolkit a **local**
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
```

`day build` (for `android-widget`) runs `cargo metadata`, walks the app's dependency closure, collects
every piece's contributions, and writes `build/day/android/day-pieces.json`. The app's checked-in
`platform/android/{app/build.gradle.kts,settings.gradle.kts}` read that file **generically** (a loop —
no per-piece edits, ever) and add the Java dirs, dependencies, and repos.

The piece's Java uses day-android's **public** surface only — `DayBridge.ctx` (the `Context`) and
`DayBridge.nativeOnEvent(id, kind, num, str)` (the event trampoline, `kind` per §14.2, `4` = selection).
The Rust side calls its OWN Java class through the re-exported `jni` (`with_env` + `call_static_method`
+ `AHandle`); `day_android::make_view` is a convenience hardcoded to `DayBridge`, so a standalone piece
uses raw `call_static_method` on its class. See `pieces/day-piece-picker/android/java/dev/daybrite/day/piece/picker/DayPicker.java`.

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
