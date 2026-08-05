# Standalone pieces (front-end and backend, no core changes)

A **piece** is a reusable Day widget. Day ships built-in pieces (`button`, `slider`, `list`, …), but
anyone can publish a piece as an independent crate that adds both its cross-platform front-end
(Rust) and its per-toolkit native backend (Objective-C via objc2, C++ shims, Android Java, …),
with no edits to any core Day crate. `day-piece-searchfield` is the reference implementation.

**Scaffold a new piece with `day new`.** Don't hand-assemble the crate: `day new piece <name>`
generates a ready-to-build project (remote Day deps by default; `--local <path>` for a local Day
checkout). With no `--toolkits` it emits a **composite** piece (front-end only); with
`--toolkits appkit,gtk,qt,uikit,mdc,xaml` (any subset) it emits a **native** piece with a renderer
per backend plus the C++/Java/Swift glue each one needs. The companion `day new part <name>` scaffolds
a headless part. For full walkthroughs see the tutorials:
[composite piece](https://daybrite.dev/docs/tutorial-composite-piece/),
[native piece](https://daybrite.dev/docs/tutorial-native-piece/), and
[part](https://daybrite.dev/docs/tutorial-part/).

The whole extensibility story rests on two mechanisms:

1. **Renderers register link-time** into each backend's `RENDERERS` slice (via `linkme`), so a backend
   dispatches an unknown `kind` to the piece's `make`/`update`/`measure` with no registry edits.
   (web-dom is the one exception: `linkme` has no wasm32 implementation, so `day-dom` keeps a
   runtime registry and the piece calls `day_dom::register_renderer` from its own constructor. Same
   three functions, same dispatch, different moment; see [media.md](media.md).)
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

## 2. Per-backend renderers: the `renderer!` macro

Each backend module registers its native renderer into that backend's `RENDERERS` slice, the same
slice the built-ins use, so no Day edit is needed. Declare the per-toolkit glue modules with one
line: `day_pieces::glue_modules!(appkit, gtk, qt, uikit, mdc, xaml)` expands to the
feature-and-target-gated `mod` block binding each `lib-<toolkit>.rs` (list only the toolkits you
implement; a non-standard gate, like webview's Linux-only GTK, stays a hand-written block beside
it). Then write typed `make`/`update` (no `&dyn Any`
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

Add `measure: f` for custom sizing: `measure: day_pieces::fill_measure` for a growing leaf (a web
view, a canvas), or omit it for the backend's default. A **patchless** piece (configured once, e.g. Lottie)
drops `patch:`/`update:`: `renderer!(day_uikit::RENDERERS, Uikit, kind: KIND, props: MyProps, make: make)`.
Do the same for `day_gtk::RENDERERS`, `day_qt::RENDERERS`, `day_uikit::RENDERERS`,
`day_android::RENDERERS`, `day_xaml::RENDERERS`. Each backend is behind a cargo feature that pulls in
that toolkit crate; the app enables `my-piece/<backend>` alongside `day/<backend>`.

**Reporting events back.** A renderer calls `day_<backend>::emit(node, event)`. Beyond the fixed
`Event` variants, a piece defines its own event with `Event::custom("my:tag", text)` (in-process); its
`cx.on` reads it. Across a native boundary (JNI/C-ABI) the tag can't be a `&'static str`, so it's empty
and the payload rides in `num`/`text`: on Android the shim calls `DayBridge.nativeOnEvent(id, 12, num,
text)` (kind 12 = the open Custom channel). `day-piece-webview` reports its URL this way.

## 3. Native backend assets

A piece often needs native code the Rust FFI alone can't express. Day gives each toolkit a local
extension path so that code lives in the piece crate:

### C++ shims: Qt & XAML (`build.rs`)

The piece carries its own `src/lib-qt-shim.cpp` / `src/lib-xaml-shim.cpp` and compiles them in `build.rs`
(gated on the feature). Qt widgets are plain C++ objects and the handle is a raw `QWidget*`, so a Qt
shim is self-contained. XAML handles are a private boxed type owned by `day-xaml-sys`, so the piece
boxes its XAML element through the exported `day_xaml_box` / `day_xaml_unbox` seam (a stable
WinRT COM-ABI). Both reuse the sys crate's generic `measure` (`day_qt_size_hint` / `day_xaml_measure`).
See `pieces/day-piece-searchfield/{build.rs,src/lib-qt-shim.cpp,src/lib-xaml-shim.cpp}`.

### Android Java + Gradle deps (`[package.metadata.day.android]`)

The piece carries its own Java/Kotlin under a crate dir and declares it in `Cargo.toml`:

```toml
[package.metadata.day.android]
java = ["android/java"]                                        # → Gradle java srcDirs
res = ["android/res"]                                          # → Gradle res srcDirs (optional)
gradle-dependencies = ["com.google.android.material:material:1.11.0"]   # → app dependencies { }
gradle-repositories = ["https://jitpack.io"]                  # → extra Maven repos (optional)
permissions = ["android.permission.INTERNET"]                 # → <uses-permission> in the manifest
proguard = ["android/proguard-rules.pro"]                     # → R8 keep rules (see below)
manifest-components = ["android/components.xml"]              # → <receiver>/<service> (see below)
```

`day build` (for `android-mdc`) runs `cargo metadata`, walks the app's dependency closure, collects
every piece's contributions, and writes `build/day/android/day-pieces.json`. The app's checked-in
`platform/android/{app/build.gradle.kts,settings.gradle.kts}` read that file generically (a loop, so
per-piece edits are never needed) and add the Java dirs, res dirs, dependencies, and repos.

**Piece resources.** `res` dirs compile into the APP's resource table, so a piece can ship the styles
or drawables its Java needs (e.g. a theme overlay for a dialog). The app's `R` package differs per
app, so the piece's Java resolves its own resources by name at runtime:
`ctx.getResources().getIdentifier("SomeStyleName", "style", ctx.getPackageName())`. Prefix names with
the piece to avoid collisions (resource names are one flat namespace per app).
`day-piece-datetime/android/res` is the reference.

**Manifest components.** A part whose Java half is a `BroadcastReceiver` or `Service` needs it
declared in the manifest, or Android never instantiates it. `manifest-components` names files
holding ONLY the elements that belong inside `<application>` — no `<manifest>` or `<application>`
wrapper, which `day build` adds — and every class must be fully qualified, since the overlay merges
into an app whose package the part cannot know:

```xml
<!-- android/components.xml -->
<receiver android:name="dev.daybrite.day.notify.DayNotifyAlarmReceiver"
          android:exported="false" />
```

They merge into the same `day-pieces-manifest.xml` overlay the permissions use. Two notes. A
declared file that is missing is a hard build error rather than a skip-and-warn, because a dropped
receiver produces an APK that installs, runs, and silently never delivers. And a scaffold generated
before this key existed gates the overlay on the permission list being non-empty — `day build`
warns, with the one-line fix, when a part contributes components but no permissions.

**Manifest permissions.** A piece that needs a permission (a web view needs `INTERNET`) can't reach the
app's `AndroidManifest.xml`, so `day build` also writes the collected permissions into a generated
**overlay manifest** (`build/day/android/day-pieces-manifest.xml`). The scaffold points its debug +
release source-set manifests at that overlay, and AGP's manifest merger folds the `<uses-permission>`
entries into the app manifest (deduping against any the app already declares). So a WebView-using app
needs no manual manifest edit: the piece declares the permission and it shows up in the merged
manifest. `day-piece-webview` is the reference. (A piece can only add a permission; it never
removes or narrows the app's own.)

**Release minification (R8/ProGuard).** A `day build --profile release` (and `day pack`) minifies with
R8: it shrinks unused code and **renames** classes and methods. But Day reaches Java from native
(Rust) code *by name*: JNI `FindClass("dev/daybrite/day/piece/searchfield/DaySearch")`, `dcall_static` on a
method name, WorkManager instantiating a `Worker` from its class-name string, Room looking up a
`<Database>_Impl`. A renamed class breaks every one of those lookups, so an un-kept release APK
installs and then crashes at launch (`NoClassDefFound` / `ClassNotFoundException` / `UnsatisfiedLinkError`).

Two layers keep the right names:

- **The framework keeps its own namespace.** `day-android` ships a `proguard-rules.pro` (bundled by
  `day build` from the crate, like the Java shim) that keeps all of `dev.daybrite.day.**` (the render
  bridge and *every official Part/Piece shim*) plus every class with `native` methods. So a first-party
  piece needs no rules of its own. It also sets `-dontoptimize`: AGP forces the `proguard-android-optimize`
  base, whose aggressive optimizations break reflection-heavy libraries (WorkManager's Room database is
  the classic casualty), and Day would rather ship predictable release builds than squeeze the last few
  percent. R8 still shrinks and renames everything a keep rule doesn't protect.

- **Everything outside `dev.daybrite.day.**` keeps itself.** An **app**'s own JNI classes (its install
  bridge, a background `Worker`) live in the *app's* package, and a **third-party piece** lives in its
  own namespace. Neither is covered by the framework rule. Each ships a `proguard-rules.pro` and lists
  it in `proguard = [...]`. `day build` collects all of them (framework + every piece + the app) into
  the release build's proguard configuration, exactly like it collects Java dirs and Gradle deps. An app
  also keeps here anything its *dependencies* reach reflectively that their own consumer rules miss
  (e.g. `-keep class * extends androidx.room.RoomDatabase { *; }` for a WorkManager user).

```proguard
# android/proguard-rules.pro — keep the classes native code reaches by name.
-keep class com.example.mypiece.MyPieceView { *; }
```

The app's `platform/android/app/build.gradle.kts` reads `dayProguardFile` + `proguardFiles` from
`day-pieces.json` and applies them in the `release` build type. `pieces/day-piece-searchfield` (framework
side) and App Fair's `android/proguard-rules.pro` (app side) are the references.

The piece's Java uses only day-android's public surface: `DayBridge.ctx` (the `Context`) and
`DayBridge.nativeOnEvent(id, kind, num, str)` (the event trampoline, `kind` per §14.2, `4` = selection).
The Rust side calls its own Java class through the re-exported `jni` (`with_env` + `call_static_method`
+ `AHandle`); `day_android::make_view` is a convenience hardcoded to `DayBridge`, so a standalone piece
uses raw `call_static_method` on its class. See `pieces/day-piece-searchfield/android/java/dev/daybrite/day/piece/searchfield/DaySearch.java`.

> **Gradle configuration cache.** The scaffold reads `day-pieces.json` at *configuration* time, and
> `day build` rewrites it every build; the config cache can't track that read, so it would serve stale
> piece contributions. The scaffold ships with `org.gradle.configuration-cache=false` for this reason.
> Some pieces also pull libraries that require AndroidX (Lottie's view extends `AppCompatImageView`), so
> the scaffold sets `android.useAndroidX=true`.

### iOS Swift shims + SwiftPM packages (`[package.metadata.day.ios]`)

Many iOS libraries (and any Swift class with a non-`@objc` API) can't be driven from Rust directly, and
they ship as **SwiftPM packages**. A piece declares both in its `Cargo.toml`: the Swift shim it
carries, and the packages it needs.

```toml
[package.metadata.day.ios]
swift = ["ios/swift"]                                         # dirs of Swift shim sources
swift-packages = [                                           # SwiftPM package dependencies
  { url = "https://github.com/airbnb/lottie-ios", from = "4.5.0", products = ["Lottie"] },
]
frameworks = ["WebKit"]                                      # system frameworks to link
```

`frameworks` links system frameworks via the generated package's `linkerSettings`. A piece that
drives a class from an unlinked framework (e.g. a hand-rolled `WKWebView`) declares it here instead of
`dlopen`ing or hand-`#[link]`ing (which doesn't survive the cargo-staticlib → xcode link). `day-piece-webview`
uses `frameworks = ["WebKit"]`.

`{from, exact, branch, revision}` map to the matching SwiftPM version requirement; `products` are the
library products to link. Xcode is not script-driven like Gradle, so `day build` (ios-uikit) instead
generates a **local SwiftPM package** at `build/day/ios/DayPieces`. Its `Package.swift` lists every
piece's `swift-packages` as dependencies and compiles every piece's staged Swift shims (each under a
per-crate subfolder). The app's checked-in `.xcodeproj` depends on that one local package (the iOS
analog of the checked-in Gradle scaffold: a `XCLocalSwiftPackageReference` + a product dependency in a
Frameworks phase). So adding an iOS piece is pure `Cargo.toml` data; no `.xcodeproj` edits are needed.

### HarmonyOS ArkTS components (`[package.metadata.day.ohos]`)

Some HarmonyOS components exist **only in ArkTS**: the ArkUI **C** node API (`arkui/native_node.h`)
stops at the container kinds, so there is no way to construct a declarative `Web` (or `Map`) from
native code at all. A piece wrapping one carries its own ArkTS the way an Android piece carries Java.

```toml
[package.metadata.day.ohos]
ets = ["ohos/ets"]        # dirs of ArkTS sources; each needs an Index.ets exporting `dayPiece`
```

Each declared dir must contain an `Index.ets` exporting a `DayPieceModule`:

```ts
import { DayPieceModule } from '../DayPiece';           // generated next to the staged dirs

export const dayPiece: DayPieceModule = {
  kind: 'day.piece.webview',                            // matches the Rust KIND
  make: (ui, id, props) => frameNode | undefined,       // build it; undefined declines the kind
  update: (id, cmd, arg) => {},                         // the piece's own command vocabulary
  dispose: (id) => {}                                   // Day disposed the node — release it
};
```

`day build -p harmony-arkui` stages every piece's dirs under `entry/src/main/ets/daypieces/<crate>/`
(gitignored) and generates two files beside them: `DayPiece.ets` (the interface above) and
`DayPieces.ets`, whose `registerDayPieces(uiContext)` hands the native shim ONE factory, command sink,
and disposer for all pieces. The scaffold's host page calls it once, before `start()`, so adding an
ArkTS piece is pure `Cargo.toml` data, like the iOS leg, and the shim never grows a case per piece.

On the Rust side the renderer is the thinnest of all the backends, because there is no native widget
to build. `day_arkui::piece::make` returns the ArkTS component's FrameNode as an ordinary handle:

```rust
fn make(_b: &mut ArkUi, p: &WebProps, id: NodeId) -> AHandle { piece::make(KIND, id, &p.url) }
fn update(_b: &mut ArkUi, h: &AHandle, patch: &WebPatch) { piece::update(h, "load", url) }
```

Events come back through the shim's `pieceEvent(id, text)` as `Event::Custom`, the same open channel
(§8.2) the Android bridge uses, payload only. Two rules the bridge enforces: a declined `make` yields
Day's placeholder leaf rather than a null handle (a null would take the whole parent's layout down),
and `release` routes an ArkTS-owned node to `dispose` instead of disposing it natively.

**Sizing.** Day owns layout and sets each node's position + size through the C API, so the ArkTS
component must NOT size itself with percentages: a `BuilderNode` is built detached, where `'100%'`
resolves against the whole window and the component covers the page.

The Swift shim exposes a flat C ABI (`@_cdecl`) that the piece's Rust calls (mirroring the Android Java
shim); it `import`s the SwiftPM product and returns a native `UIView` that Rust wraps via
`Retained::from_raw`. See `pieces/day-piece-lottie/{ios/swift/DayLottie.swift,src/lib-uikit.rs}`.

### The Android bridging contract

Guarantees a part or piece can rely on when its Rust calls its Java sidecar (all provided by
`day-android`, all exercised in production by `day-part-http`):

- **Any thread may call.** `day_android::with_env(|env| …)` attaches the calling thread to the
  JVM (and detaches scoped attachments). Blocking Java work runs on the CALLER's thread; keep
  it off the UI thread, exactly like any other blocking Rust.
- **App classes resolve from any thread.** `env.dfind`/`dcall_static`/`dfield` fall back to the
  app `ClassLoader` cached at startup, so a Rust-spawned worker resolves your sidecar class even
  though a bare JNI `FindClass` there only sees system classes.
- **Post to the UI thread** with `DayBridge.main.post(...)` on the Java side; on the Rust side,
  capture a `day_reactive::Setter` (docs/focus.md, DESIGN §4.5) rather than touching UI state
  from a worker.
- **Bulk payloads cross as ONE `byte[]` envelope** — `[status i32 BE][meta-len i32 BE]
  ["k\nv\n…" meta][payload]`, negative status = your error sentinel with the message riding the
  meta block. Build it with `DayEnvelope.pack/error` in Java and parse it with
  `day_android::envelope::Envelope` in Rust; the two encode identically and Rust unit tests pin
  the format. One JNI copy each way, no per-field JNI chatter.
- **Piece-defined events** ride the `K_CUSTOM` kind (`DayBridge.nativeOnEvent(id, DayBridge.K_CUSTOM,
  num, text)` → `Event::Custom`): the tag can't cross JNI, so the piece reads the raw
  `num`/`text` payload. The full kind table is `day_spec::bridge::BridgeKind`; parity tests keep
  the Java constants in step.

## 4. Cargo wiring

```toml
[features]
appkit = ["dep:day-appkit", …]
gtk = ["dep:day-gtk", "dep:gtk4"]
qt = ["dep:day-qt"]                 # + a build.rs that compiles src/lib-qt-shim.cpp
uikit = ["dep:day-uikit", …]
mdc = ["dep:day-android"]        # + [package.metadata.day.android]
xaml = ["dep:day-xaml", "dep:day-xaml-sys"]   # + build.rs compiles src/lib-xaml-shim.cpp
```

The app mirrors each: `my-piece/<backend>` in the matching feature. That's it: no changes to `day`,
the toolkit crates, the CLI, or the Gradle scaffold are needed to add a piece.

**Dependency layering.** The extension graph stays acyclic by rule: **pieces may depend on
parts** (day-piece-remote-image → day-part-http is the shipped example) **and on core crates;
parts must not depend on `day-pieces` or any `day-piece-*`; tweaks may depend on `day-pieces`
(the built-ins they configure) but not on any satellite `day-piece-*` or `day-part-*`.** A
workspace test (`crates/day-cli/tests/layering.rs`) enforces this over `cargo metadata`, so a
violating dependency fails `cargo test` rather than knotting the graph quietly.

## 5. Container pieces (hosting a Day child)

A piece is not limited to leaves: it can be a **container** whose native view hosts a Day-built
subtree. day-core mounts children **by handle, not by kind** (the tree walks to the nearest native
ancestor and calls `Toolkit::insert(ancestor_handle, child_handle, index)` without consulting the
ancestor's kind), so a piece-realized node is a valid insertion parent on every backend. The recipe
(established by `pieces/day-piece-pullrefresh`, the reference container piece; see docs/pullrefresh.md):

```rust
let node = cx.native(
    KIND,
    &MyProps { /* … */ },
    Rc::new(day_core::FrameLayout { width: None, height: None }), // child fills the container
    Flex { grow_w: true, grow_h: true, ..Default::default() },
    day_core::Boundary::Yes,
);
cx.under(node, |cx| { let _ = child.build(cx); });               // mount the Day child inside
```

- Supply a **layout** (`FrameLayout`/`PassThrough`, or your own `day_core::Layout`). A container
  node measures/places through its layout, not through the renderer's `measure` fn.
- Your per-backend `make` must return a **container-capable native view**: any `NSView`/`UIView`,
  any `QWidget`, any ArkUI FrameNode, but on GTK a `gtk4::Fixed`-backed view, on Android a
  `ViewGroup`, and on XAML a `Panel`, or the generic `insert` silently drops the child.
  (Conveniently, native wrappers like Android's `SwipeRefreshLayout` ARE ViewGroups.)
- Events still flow through the single sink (`Event::Custom` for piece-defined ones) and commands
  through `with_tree(|t| t.patch(node, …))`, identical to leaf pieces.

## External toolkits: registering a platform-toolkit pair (experimental)

> **The toolkit SPI is unstable.** Everything a backend implements (day-spec's `Toolkit` and
> `Platform` traits, `Event`, `Cap`, the per-kind props structs) evolves with this repository and
> is not published to crates.io. An external toolkit pins the day crates to a **git revision** and
> expects breakage between revisions. This section describes the registration seam, which is the
> part that will stay; the contract behind it firms up at SPI stabilization.

A toolkit implemented in its own repository registers its platform-toolkit pair by declaring it in
the toolkit crate's `Cargo.toml`:

```toml
[package.metadata.day.toolkit]
target = "netbsd-wxwidgets"    # <os>-<toolkit>; the toolkit half names the app's cargo feature
host = "any"                   # optional: restrict to "macos" | "linux" | "windows"
label = "wxWidgets"            # optional: pickers and error listings
doctor = "wx-config --version" # optional: `day doctor` probe (command + space-separated args)
```

The CLI resolves `-p <name>` against the builtin catalog first, then against declarations found in
the app's dependency graph (via `cargo metadata --all-features`, since the toolkit crate sits
behind the very feature its declaration names). A declared target inherits the **desktop**
pipeline. That is the only `kind` accepted today, deliberately: a new pipeline kind (another
mobile OS) means new build/launch/pack code in the CLI, which cannot come from a crate.

The app wires the toolkit the same way it would a builtin, plus one entry call:

```toml
# the app's Cargo.toml
[features]
wxwidgets = ["dep:day-toolkit-wx"]          # feature = the toolkit half of the target name

[dependencies]
day-toolkit-wx = { git = "…", optional = true }
```

```rust
// the app's main.rs — the toolkit's entry wraps `day::launch_external`, the cfg-free
// launcher that starts the dayscript engine exactly as the builtin launchers do.
#[cfg(feature = "wxwidgets")]
day_toolkit_wx::launch(options, root);
```

With `targets = ["netbsd-wxwidgets"]` in Day.toml, the target behaves like a builtin desktop
target: `day launch` (build + run + log streaming), `--script` walkthroughs, `day drive`,
`day stop`/`relaunch`, the session registry, `day doctor` (running the declared probe), `day
metadata` (the catalog entry carries `external: true` and the declaring `crate`), and `day lint`.

What a declared target does **not** get:

- **`day pack`** — packaging formats are per-OS CLI code; the guard says so explicitly.
- **`day new` / `day app add`** — scaffolding stays builtin; the toolkit crate documents its own
  project shape.
- **The in-repo pieces' native renderers:** `day-piece-webview` has no `wxwidgets` feature arm, so
  extension-piece kinds render Day's visible `⟨kind⟩` placeholders unless the external ecosystem
  ships renderer crates for its backend (the `Registry`/`renderer!` seam is the same one in-repo
  pieces use). The BUILT-IN vocabulary is the backend's own `realize`: cover what you support and
  placeholder the rest; `assert_no_placeholders` allow-lists keep the gap ledger honest.

## Reference

`pieces/day-piece-searchfield` implements all of the above: a native search input on six backends,
its own Qt + XAML C++ shims, and its own Android Java. It's verified on AppKit / GTK / Qt / iOS /
Android and CI-built on XAML. Use it as a template. Its layout keeps the shared front-end and each
toolkit backend in a separate file:

```
pieces/day-piece-searchfield/
├── Cargo.toml               # features + [package.metadata.day.android]
├── build.rs                 # compiles lib-qt-shim.cpp / lib-xaml-shim.cpp per feature
├── android/java/…/DaySearch.java   # this piece's own Android backend
└── src/
    ├── lib.rs               # front-end (the `Piece`) + `day_pieces::glue_modules!(…)`
    ├── lib-appkit.rs        # one file per toolkit renderer …
    ├── lib-gtk.rs
    ├── lib-qt.rs            (+ lib-qt-shim.cpp)
    ├── lib-uikit.rs
    ├── lib-android.rs      (+ android/java DaySearch.java)
    ├── lib-xaml.rs        (+ lib-xaml-shim.cpp)
    ├── lib-qt-shim.cpp
    └── lib-xaml-shim.cpp
```

`lib.rs` declares the backends with one `day_pieces::glue_modules!(appkit, gtk, qt, uikit, mdc,
xaml)` line (§2), so every `lib-<toolkit>.rs` is compiled only for its feature+target and the whole
native surface for a toolkit lives in one place.

`pieces/day-piece-webview` (see [webview.md](webview.md)) is a second reference: a heavier native
backend (an embedded browser) that additionally contributes an Android permission, hand-rolls the iOS
`WKWebView` (`dlopen`-ing WebKit.framework so the piece stays self-contained), and returns the proposal
from `measure` so a growing leaf fills on Android.

`pieces/day-piece-lottie` (see [lottie.md](lottie.md)) is a third reference: an iOS/Android-only piece
that pulls an external native package on each platform, the lottie-ios SwiftPM package (via the
`[package.metadata.day.ios]` mechanism above) and `com.airbnb.android:lottie` (Gradle). Its Swift
and Java shims each wrap a `LottieAnimationView` behind a flat C ABI / static method.

`parts/day-part-battery` (see [battery.md](battery.md)) is a fourth reference, the first **part**:
a headless crate with no UI Piece at all. Where `pieces/` holds UI-library extensions (each
registers a renderer), `parts/` is the non-UI counterpart: capability crates (`day-part-*`) that extend
Day apps with platform services. It shows the backend-contribution mechanism accommodates non-UI
capabilities: it contributes Android Java through `[package.metadata.day.android]` (for
`BatteryManager`) but registers nothing into any `RENDERERS` slice, and selects its per-OS impl by
`#[cfg(target_os)]` rather than a toolkit feature. Any Rust code can depend on it and call
`day_part_battery::status()`.
