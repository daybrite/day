---
title: Project structure & builds
description: The anatomy of a conventional Day app, how each target is built, and how resources are packaged for runtime access.
order: 31
section: Build & ship
---

A Day app is a normal Cargo package plus a small `Day.toml` manifest and a few conventional
directories. The `day` CLI reads that layout to build every target: the same Rust code becomes a
plain desktop binary, a static library inside an Xcode app, a JNI `.so` inside a Gradle APK, or a
NAPI `.so` inside a HarmonyOS `.hap`. This page walks the layout, then each build pipeline, then
how resources travel from your project into each platform's native store.

## The conventional project

```text
my-app/
├── Day.toml                  # the app manifest: id, title, targets, window (name/version come from Cargo.toml)
├── Cargo.toml                # a normal Cargo package (bin + rlib)
├── src/
│   ├── lib.rs                # the app: pieces, signals, routes; install_locales(…)
│   └── main.rs               # desktop entry point; mobile entries live in lib.rs macros
├── resource/
│   ├── assets/               # arbitrary data files   → resource("stations.json")
│   ├── images/               # processed images       → image("logo"), logo@2x.png variants
│   ├── fonts/                # custom fonts (.ttf/.otf), referenced by family name
│   ├── icons/                # app icon sources, staged per platform (dock, taskbar, launcher)
│   └── locales/
│       ├── en/app.ftl        # Fluent translations, embedded at compile time (include_str!)
│       └── fr/app.ftl
├── dayscript/                # dayscript flows: walkthroughs, screenshots, assertions
├── platform/
│   ├── ios/                  # Xcode scaffold: DayApp.xcodeproj + a thin Swift Runner
│   └── android/              # Gradle scaffold: settings/app modules, AndroidManifest, theme
├── platform/ohos/            # HarmonyOS scaffold: hvigor ArkTS host + sign-hap.mjs
└── build/day/                # generated: cargo target dirs, staged resources, screenshots
```

Three rules keep this layout predictable:

- **`Day.toml` is the single manifest.** The app's Day-specific identity (`id`, `title`,
  `build`), its declared `targets`, and the default window geometry live here — while `name`
  and `version` are derived from Cargo.toml's `[package]`, so they can never drift. Any `[app]`
  property can be overridden per platform (`[app.ios]`), per toolkit (`[app.qt]`), or per
  target (`[app.macos-appkit]`); the platform scaffolds read the resolved values at build time.
- **The scaffolds are hosts, not apps.** `platform/ios`, `platform/android`, and `platform/ohos` contain
  no app logic. Each is a minimal native shell that loads the Rust library and hands it the root
  view. They change so rarely that diffs to them are meaningful.
- **Everything generated lands in `build/day/`.** Cargo target directories (one per target and
  profile, so parallel builds never contend), staged resources, packed artifacts, and dayscript
  screenshots all live under one ignorable directory.

## How a build works

Every target follows the same shape. `day build -p <target>` (or `launch`, which builds first)
stages resources, selects the toolkit feature, and runs the platform's own build system for
anything native:

```text
day build -p <target>
│
├── 1. stage resources          resource/images + resource/assets → the target's native store
│                               (actool / aapt2 / GResource / .qrc / rawfile — see below)
│
├── 2. select features          --features <toolkit> + every standalone piece's
│                               <piece>/<toolkit> renderer feature (from cargo metadata)
│
└── 3. platform build
    ├── desktop   cargo build            → the app binary IS the artifact
    ├── ios       xcodebuild             → Runner.app  (links the cargo staticlib)
    ├── android   cargo-ndk + gradle     → app.apk     (bundles the cargo cdylib)
    └── harmony   cargo + hvigor + sign  → app.hap     (bundles the cargo cdylib)
```

One backend is compiled per binary. The AppKit build contains no GTK code, the Android build only
its JNI bridge. Standalone pieces (say, a Lottie or map piece) contribute their own native code and
dependencies through Cargo metadata, so the app never re-declares per-piece build wiring.

### Desktop: `macos-appkit`, `linux-gtk`, `linux-qt`, `windows-winui`, and the GTK/Qt combinations

Desktop targets are the simplest: the artifact is the Cargo binary itself.

```text
src/*.rs ──► cargo build -p my-app --features appkit     (per-target CARGO_TARGET_DIR)
                 │
                 ├── GTK: links system GTK 4 / libadwaita
                 ├── Qt / WinUI: cc-compiled C++ shim (built by the toolkit crate's build.rs)
                 └── WinUI: embeds a side-by-side manifest (XAML Islands requires it)
                 ▼
         build/day/cargo/<target>/<profile>/my-app      ◄── day launch runs this directly
                 ▼
         day pack: macOS .app + ad-hoc codesign + .dmg
```

Because GTK and Qt are portable, `macos-gtk`, `macos-qt`, `windows-gtk`, and `windows-qt` build the
same way on their respective hosts. Resources that need a native compiler (GResource, `.qrc`) are
compiled if the tool is on `PATH` and otherwise fall back to filesystem loading, so a missing
`glib-compile-resources` never fails the build.

### iOS: `ios-uikit`

The Xcode project owns the bundle; the Rust code arrives as a static library through a build-phase
callback into the `day` CLI:

```text
day build -p ios-uikit
│
├── generate DayPieces           a local SwiftPM package assembled from every piece's
│                                [package.metadata.day.ios] (Swift shims, SwiftPM deps)
│
└── xcodebuild  platform/ios/DayApp.xcodeproj  (Runner target, iphonesimulator arm64)
        │
        ├── script phase: "day xcode-backend build"
        │       └── cargo rustc --crate-type staticlib --target aarch64-apple-ios-sim
        │           → libmy_app.a, linked into Runner
        ├── actool: resource/images → Media.xcassets → optimized Assets.car
        └── Swift Runner: loads the Day root view, hands control to Rust
        ▼
build/day/ios-uikit/Debug-iphonesimulator/MyApp.app
        ▼
xcrun simctl install booted … && simctl launch          (day launch)
```

The callback design means opening `platform/ios` in Xcode and pressing Run also works: Xcode calls
back into `day` for the Rust half, exactly as `day` calls into `xcodebuild` for the native half.

### Android: `android-mdc`

Android inverts iOS: `day` runs Cargo first, then hands Gradle a project whose source sets already
point at everything Day staged:

```text
day build -p android-mdc
│
├── cargo-ndk (arm64-v8a) ────────► build/day/jniLibs/arm64-v8a/libmy_app.so
│
├── piece discovery ──────────────► build/day/android/day-pieces.json
│                                   (each piece's Java dirs, Gradle deps, Maven repos,
│                                    manifest permissions — read generically by the scaffold)
│
└── gradle assembleDebug   platform/android/
        │
        ├── sourceSets: the day-android Java shim + piece Java + jniLibs + assets/
        ├── aapt2: staged resource/images → res/drawable* → R.drawable ids
        └── Material 3 theme + DayActivity host (loads the .so, calls nativeStart)
        ▼
platform/android/app/build/outputs/apk/debug/app-debug.apk
        ▼
adb install … && am start DayActivity                   (day launch)
```

The Gradle scaffold also calls back (`day gradle-backend build`) so a build started from Android
Studio rebuilds the Rust `.so` the same way.

### HarmonyOS: `ohos-arkui`

The newest pipeline follows the Android shape with HarmonyOS tooling: an ArkTS host project in
`platform/ohos/`, a cross-compiled NAPI library, and a post-build signing step that needs no vendor
account:

```text
day build -p ohos-arkui
│
├── cargo rustc --crate-type cdylib --target x86_64-unknown-linux-ohos   (emulator; arm64 device)
│       linker = $OHOS_NDK_HOME/llvm/bin/<triple>-clang
│       ────────► platform/ohos/entry/libs/<abi>/libentry.so
│
├── hvigor assembleHap   platform/ohos/   (ohpm install first)
│       ├── compiles the ArkTS host (Index.ets mounts Day via a NodeContent slot)
│       ├── packs libentry.so + resources/rawfile/day/ (staged images & assets)
│       └── → entry-default-unsigned.hap
│
└── sign-hap.mjs         patch compileSdkType → "OpenHarmony", sign with the SDK's
        │                public release material (no developer account required)
        ▼
platform/ohos/entry/build/…/my-app-signed.hap
        ▼
hdc install … && aa start EntryAbility                  (day launch)
```

## How resources are packaged

`resource/images/` and `resource/assets/` are looked up by name at runtime through `image("logo")` and
`resource("stations.json")`. Day never rewrites your bytes. Before each platform build it stages
the files into that target's **native resource store**, so the platform's own machinery does the
optimizing, and the runtime read is native (and zero-copy wherever the store exposes a stable
pointer):

```text
                        day build -p <target>
                                 │  stage
      ┌──────────────────────────┼──────────────────────────────┐
      ▼                          ▼                              ▼
   resource/images/logo.png   resource/assets/stations.json resource/icons/
      │                          │                              │
      │ per-target store         │ per-target store             │ dock / taskbar /
      │                          │                              │ launcher icon
┌─────┴──────────────────┐ ┌─────┴─────────────────────┐        │
│ iOS      Assets.car    │ │ Apple    bundle file+mmap │        ▼
│ macOS    bundle file   │ │ Android  AAssetManager    │   .icns / mipmap /
│ Android  res/drawable* │ │ GTK      GResource        │   .ico / xcassets
│ GTK      GResource     │ │ Qt       QResource        │
│ Qt       .qrc          │ │ WinUI    loose file       │
│ WinUI    scale-*.png   │ │ ArkUI    rawfile fd+mmap  │
│ ArkUI    rawfile       │ └───────────┬───────────────┘
└─────┬──────────────────┘             │
      ▼                                ▼
 image("logo")                 resource("stations.json")
 native by-name lookup         zero-copy &[u8] view, random access
```

At runtime, `resource()` returns a `Resource` backed directly by that store:

```rust
let res = day::resource("stations.json").expect("bundled");
let bytes: &[u8] = res.as_slice();   // zero-copy view into the native store
let mut header = [0u8; 16];
res.read_at(0, &mut header);         // random access, no allocation
```

On Apple platforms that view is an `mmap` of the bundle file; on Android it is the NDK
`AAssetManager` buffer of an uncompressed asset; GTK and Qt read out of resource blobs compiled
into the binary; ArkUI maps the `rawfile` descriptor. Images resolve through each platform's
by-name API (`UIImage(named:)`, `R.drawable`, `gtk_picture_new_for_resource`, `QPixmap(":/…")`,
`resource://RAWFILE/…`), so density variants like `logo@2x.png` map onto the platform's own
scale-selection mechanism.

Fluent translations under `resource/locales/` take a different, simpler path: they are embedded into the
binary at compile time with `include_str!`, so locale switching never touches the filesystem.

The full per-platform details, including the limits (what gets optimized where, and which stores
allow zero-copy), are in the [resources reference](/docs/internal/resources); the HarmonyOS
pipeline has its own [deep dive](/docs/internal/harmonyos).
