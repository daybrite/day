---
navTitle: "Projects"
title: Project structure & builds
description: The anatomy of a conventional Day app, how each target is built, and how resources are packaged for runtime access.
order: 31
section: Reference
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

A Day project combines a Cargo package, a `Day.toml` app manifest, and platform host projects.
Rust code defines the app’s shared behavior; the host projects provide the files needed by
Xcode, Gradle, and other platform build tools. The CLI builds them from the layout below.

## The conventional project

```text
my-app/
├── Day.toml                  # the app manifest: id, title, targets, window (name/version come from Cargo.toml)
├── Cargo.toml                # a normal Cargo package (bin + rlib)
├── build.rs                  # day-build codegen: the typed res:: constants src/lib.rs includes
├── src/
│   ├── lib.rs                # the app: pieces, signals, routes; res::locales::install()
│   └── main.rs               # desktop entry point; mobile entries live in lib.rs macros
├── resource/
│   ├── assets/               # arbitrary data files   → res::assets::stations_json
│   ├── images/               # processed images       → res::images::logo, logo@2x.png variants
│   ├── vectors/              # SVG glyphs, staged natively per backend → res::vectors::…
│   ├── fonts/                # custom fonts (.ttf/.otf), referenced by family name
│   ├── icons/                # the app icon master; `day prepare` renders every platform's set under build/day/host
│   └── locales/
│       ├── en/app.ftl        # Fluent translations, embedded at compile time; a new
│       └── fr/app.ftl        #   directory here is a new language (res::locales::install())
├── dayscript/                # dayscript flows: walkthroughs, screenshots, assertions
├── store/                    # the canonical store listing `day store` consumes
├── website/                  # optional app-site scaffold (skip with --no-website)
├── .github/workflows/ci.yml  # the shared Day workflow on every target above (skip with --no-github, with .gitignore and the git repository)
├── platform/
│   ├── ios/                  # Apple host sources: thin Swift Runner + DayApp.xcconfig
│   └── macos/                # Apple host sources: thin Swift Runner + DayApp.xcconfig
├── build/android/            # generated Gradle host, caches and APK/AAB outputs
└── build/day/                # generated: cargo target dirs, staged resources, screenshots
```

- `Day.toml` is the single manifest. The app's Day-specific identity (`id`, `title`, `artifact`,
  `build`), its declared `targets`, the default window geometry, and the `[permissions]`,
  `[signing]`, and `[sbom]` sections live here, while `name` and `version` are derived from
  Cargo.toml's `[package]`, so they can never drift. The identity properties (`id`, `title`,
  `artifact`, `build`) can be overridden per platform (`[app.ios]`), per
  [toolkit](/docs/reference/glossary#toolkit) (`[app.qt]`), or per target (`[app.macos-appkit]`); the platform
  scaffolds read the resolved values at build time.
- Native hosts load the Rust library and provide its root view. Apple keeps its Swift sources
  under `platform/`. Apple project documents, Android Gradle scaffolds and HarmonyOS hvigor
  scaffolds are derived by default. `day project add-target <target> --persist-native-project`
  creates an app-owned override under `platform/` when manual customization is needed.
- Generated files are written to `build/` (mostly `build/day/`): Cargo target directories (one per target and profile,
  so parallel builds never contend), staged resources, packed artifacts, and
  [dayscript](/docs/reference/glossary#dayscript) screenshots all live under one ignorable directory.
- `website/site.toml` enables a localized app website in the shared Actions workflow.
  `website/theme.css` supplies optional style overrides. Add `daysite.config.mjs` for reusable
  theme/component customization, or an Astro configuration and source tree for your own pages.
  See [App websites](/docs/how-to/websites) for setup, customization, Pages, and custom domains.

## How a build works

Every target follows the same shape. `day build -t <target>` (or `launch`, which builds first)
stages resources, selects the toolkit feature, and runs the platform's build system for
anything native:

```text
day build -t <target>
│
├── 1. stage resources          resource/images + resource/assets → the target's native store
│                               (actool / aapt2 / GResource / .qrc / rawfile — see below)
│
├── 2. select features          --features <toolkit> + every standalone piece's
│                               <piece>/<toolkit> renderer feature (from cargo metadata)
│
└── 3. platform build
    ├── desktop   cargo build            → the app binary is the artifact
    ├── ios       xcodebuild             → Runner.app  (links the cargo staticlib)
    ├── android   cargo-ndk + gradle     → app.apk     (bundles the cargo cdylib)
    └── harmony   cargo + hvigor + sign  → app.hap     (bundles the cargo cdylib)
```

One [toolkit](/docs/reference/glossary#toolkit) is compiled per binary. The AppKit build contains no GTK code, the Android build only
its JNI bridge. Standalone [pieces](/docs/reference/glossary#piece) (say, a Lottie or map piece) contribute their own native code and
dependencies through Cargo metadata, so the app never re-declares per-piece build wiring.

### Desktop: `macos-appkit`, `linux-gtk`, `linux-qt`, `windows-winui`, and the GTK/Qt combinations

Desktop targets are the simplest: the artifact is the Cargo binary itself.

```text
src/*.rs ──► cargo build -p my-app --features appkit     (per-target CARGO_TARGET_DIR)
                 │
                 ├── GTK: links system GTK 4 / libadwaita
                 ├── Qt / WinUI: cc-compiled C++ shim (built by the toolkit crate's build.rs)
                 ├── WinUI: embeds a side-by-side manifest (`maxversiontested`);
                 │   day pack -t windows-winui adds the runtime's WinRT registrations
                 └── macos-appkit: swift build prepass when Swift is contributed
                     (build/day/macos/DayPieces, statically linked — docs/swiftui)
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
day build -t ios-uikit
│
├── generate DayPieces           a local SwiftPM package assembled from every piece's
│                                [package.metadata.day.ios]: Swift shims, SwiftPM deps
│                                (remote or local packages), system frameworks, and the
│                                platform floor. A local package's public SwiftUI views
│                                are scanned and exported as crate::swiftui::… bindings;
│                                [package.metadata.day.macos] is the appkit twin
│
└── xcodebuild  build/day/xcode/ios/DayApp.xcodeproj  (Runner target, iphonesimulator arm64)
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

The callback design means opening the project with `day open -t ios-uikit` and pressing Run also works, because the
build phase calls back into `day` for the Rust half.

### Xcode project format

macOS and iOS hosts use JSON5 `project.xcproj` files requiring **Xcode 27 or newer**.
By default, no `.xcodeproj` folder is checked in. `day build`, `day launch`, `day pack`,
`day prepare`, and `day open` derive the project under
`build/day/xcode/<macos|ios>/DayApp.xcodeproj/`. This folder is disposable and recreated
when needed; edits to its project document are replaced on the next preparation.
The Swift sources, `Info.plist`, and editable `DayApp.xcconfig` remain under
`platform/<macos|ios>/`. The generic `day-cli.sh` launcher is generated beside the derived
project under `build/day/xcode/<macos|ios>/`; it does not need to be checked in. An existing
`platform/<macos|ios>/day-cli.sh` is treated as a custom override and preserved. For a persisted
Xcode project, preparation creates a missing launcher in `platform/<macos|ios>/` instead.
Generated projects derive their locales from `resource/locales/`.

Open a prepared project with:

```sh
day open -t macos-appkit
day open -t ios-uikit
# Generate/prepare it and print the path without opening Xcode:
day open -t ios-uikit --print-path
```

VS Code's **Open in Xcode** asks the CLI for this path, so it also works before the first
build. `--format json` returns the path in an `open` result object for editor integrations.

For targets, build phases, or other project customizations beyond Day's generated layout,
opt into a persistent project (also works for an already-declared target):

```sh
day project add-target macos-appkit,ios-uikit --persist-xcode-project
```

This writes `platform/<macos|ios>/DayApp.xcodeproj/project.xcproj` without overwriting an
existing project. A checked-in project always takes precedence over generated output;
Day does not regenerate it. Use `--template` as well for a custom project template.
To return to automatic generation, first preserve any customizations you need, then
remove the persistent `.xcodeproj` folder. Ordinary build settings can stay in
`DayApp.xcconfig` without requiring a persistent project.

Day requires `project.xcproj`; PBX-only projects fail lint and native builds. Remove the
persistent folder to use automatic generation, or convert a custom project with Xcode 27:

```sh
xcodebuild -project platform/macos/DayApp.xcodeproj -convert-project "Xcode Project"
xcodebuild -project platform/ios/DayApp.xcodeproj -convert-project "Xcode Project"
```

Review each conversion and remove any obsolete legacy file. If conversion creates a
recursive `../..` file reference, replace it with an explicit group of app sources:
otherwise Xcode opens generated DayPieces folders before SwiftPM can load them.
`day lint` validates persistent projects or derives the default document in memory,
without requiring generated packages or writing build output.

The templates omit optional product IDs, upgrade history, and duplicated target
configurations. Xcode still requires a stable target ID; Day uses the readable
`"id": "Runner"`. Keep it stable when renaming a target, because schemes and other
projects may reference it. Existing converted hexadecimal IDs remain supported.

### Xcode build settings

Both Xcode scaffolds keep their editable build settings in a committed
`platform/<ios|macos>/DayApp.xcconfig` rather than inside the `.xcodeproj`, so they read and diff as
plain text. Each project configuration uses it as its base configuration; Runner inherits
those settings without repeating the configuration entries. The file ends with two
optional includes:

```text
#include? "../../build/day/xcconfig/ios.xcconfig"   # generated from Day.toml by `day build`
#include? "DayApp.local.xcconfig"                   # per-checkout, gitignored
```

The generated file carries the bundle id, version, and build number, so `Day.toml` stays
authoritative once a build has run. `DayApp.local.xcconfig` is included after it and overrides both,
which is where a signing team belongs:

```text
DEVELOPMENT_TEAM = ABCDE12345
CODE_SIGN_STYLE = Automatic
```

The scaffold sets no signing keys of its own, so simulator and local desktop builds sign ad-hoc and
need no Apple account. A device build takes the team from the local file, which every `.gitignore`
covers, so a fork signs with its own team and its own bundle id without editing a tracked file.
Settings that `day build` and `day pack` pass on the command line override all three.

The generated macOS and iOS Runner projects use Swift 6 language mode (`SWIFT_VERSION = 6.0`).
Xcode still supplies its current Swift compiler; the language mode controls source compatibility
and concurrency checking. Xcode 27.2 rejects an omitted language version, so Day sets it
explicitly. SwiftPM packages use the language modes selected by their own manifests.

### Android: `android-mdc`

On Android, `day` runs Cargo first, then hands Gradle a project whose source sets already point at
everything Day staged:

```text
day build -t android-mdc
│
├── cargo-ndk (arm64-v8a) ────────► build/day/jniLibs/arm64-v8a/libmy_app.so
│
├── piece discovery ──────────────► build/day/android/day-pieces.json
│                                   (each piece's Java dirs, Gradle deps, Maven repos,
│                                    manifest permissions — read generically by Day's Gradle plugin)
│
└── gradle assembleDebug   build/android/
        │
        ├── sourceSets: the day-mdc Java shim + piece Java + jniLibs + assets/
        ├── aapt2: staged resource/images → res/drawable* → R.drawable ids
        └── Material 3 theme + DayActivity host (loads the .so, calls nativeStart)
        ▼
build/android/app/build/outputs/apk/debug/app-debug.apk
        ▼
adb install … && am start DayActivity                   (day launch)
```

An optional app-owned `platform/android/AndroidManifest.xml` supplies manifest customizations
without persisting the Gradle project; see [manifest customization](/docs/reference/platforms/android-mdc#manifest-customization).
An existing `platform/android/` Gradle project takes precedence over the generated default.
`day open -t android-mdc` prepares and opens the selected project in Android Studio.
Day's Gradle plugin configures that project from `build/day/android/`
([The Gradle project](/docs/reference/platforms/android-mdc#the-gradle-project)). A build started from Android
Studio packages the Rust `.so` from the last `day build`.

### HarmonyOS: `harmony-arkui`

The newest pipeline follows the Android shape with HarmonyOS tooling: an ArkTS host project
derived under `build/day/harmony/project/`, a cross-compiled NAPI library, and a post-build signing step with the public
OpenHarmony development certificate:

```text
day build -t harmony-arkui
│
├── cargo ohos rustc --crate-type cdylib --target x86_64-unknown-linux-ohos   (emulator; arm64 device)
│       cross environment from the OpenHarmony SDK cargo-ohos resolves (OHOS_SDK_NATIVE)
│       ────────► build/day/harmony/project/entry/libs/<abi>/libentry.so
│
├── hvigor assembleHap   build/day/harmony/project/   (ohpm install first)
│       ├── compiles the ArkTS host (Index.ets mounts Day via a NodeContent slot)
│       ├── packs libentry.so + resources/rawfile/day/ (staged images & assets)
│       └── → entry-default-unsigned.hap
│
└── sign-hap.mjs         patch compileSdkType → "OpenHarmony", sign with the SDK's
        │                public release material (no developer account required)
        ▼
build/day/harmony/project/entry/build/…/my-app-signed.hap
        ▼
hdc install … && aa start EntryAbility                  (day launch)
```

### Web: `web-dom`

This is the shortest of the five pipelines, and the only one with no host project to check in. The
app's lib crate is compiled straight to wasm and dropped next to the host page:

```text
day build -t web-dom
│
├── cargo rustc --crate-type cdylib --target wasm32-unknown-unknown
│       exports day_dom_main (via day::day_start_web!)
│       ────────► dist/app.wasm
│
├── the host trio, embedded in the CLI and written out verbatim
│       ────────► dist/index.html · dist/shim.js · dist/day.css
│
└── bundled images and fonts + a fonts.json manifest the shim preloads
        ────────► dist/assets/…
                        ▼
              dist/ is the deployable — copy it to any static host
                        ▼
day launch -t web-dom   serves dist/ over loopback and opens your browser
```

There is no `day pack` for this target: `dist/` is already the artifact. Browsers refuse to
instantiate WebAssembly from `file:`, so use `day launch` (or any static server) rather than
opening `index.html` directly.

## How resources are packaged

`resource/images/` and `resource/assets/` are looked up by name at runtime through the generated
typed constants ([`image(res::images::logo)`](https://daybrite.dev/api/day_pieces/fn.image.html), [`resource(res::assets::stations_json)`](https://daybrite.dev/api/day_core/fn.resource.html); a typo is a
compile error). Before each platform build Day stages the files, unchanged, into that target's
native resource store, so the platform's machinery does the optimizing, and the runtime read is
native (and zero-copy wherever the store exposes a stable pointer):

```text
                        day build -t <target>
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
│ Qt       .qrc          │ │ XAML    loose file       │
│ XAML    scale-*.png   │ │ ArkUI    rawfile fd+mmap  │
│ ArkUI    rawfile       │ └───────────┬───────────────┘
└─────┬──────────────────┘             │
      ▼                                ▼
 image(res::images::logo)      resource(res::assets::stations_json)
 native by-name lookup         zero-copy &[u8] view, random access
```

At runtime, [`resource()`](https://daybrite.dev/api/day_core/fn.resource.html) returns a `Resource` backed directly by that store:

```rust
let res = day::resource(res::assets::stations_json).expect("bundled");
let bytes: &[u8] = res.as_slice();   // zero-copy view into the native store
let mut header = [0u8; 16];
res.read_at(0, &mut header);         // random access, no allocation
```

On Apple platforms that view is an `mmap` of the bundle file; on Android it is the NDK
[`AAssetManager`](https://developer.android.com/ndk/reference/group/asset) buffer of an uncompressed asset; GTK and Qt read out of resource blobs compiled
into the binary; ArkUI maps the `rawfile` descriptor. Images resolve through each platform's
by-name API ([`UIImage(named:)`](https://developer.apple.com/documentation/uikit/uiimage), `R.drawable`, [`gtk_picture_new_for_resource`](https://docs.gtk.org/gtk4/ctor.Picture.new_for_resource.html), [`QPixmap(":/…")`](https://doc.qt.io/qt-6/qpixmap.html),
`resource://RAWFILE/…`), so density variants like `logo@2x.png` map onto the platform's own
scale-selection mechanism.

[Fluent](/docs/reference/glossary#fluent) translations under `resource/locales/` take a different path: they
are embedded into the binary at compile time with `include_str!`, so [locale](/docs/reference/glossary#locale)
switching never touches the filesystem.

The full per-platform details, including the limits (what gets optimized where, and which stores
allow zero-copy), are in the [resources reference](/docs/reference/resources); the HarmonyOS
pipeline has its own [reference page](/docs/how-to/harmonyos).
