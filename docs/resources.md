---
title: "Resources"
description: "The typed res:: constants: images, data files, fonts, vectors, and hierarchical assets, staged natively per platform."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Resources

Bundle images, vectors, fonts, and data files with your app, and access them through generated
`res::images`, `res::vectors`, `res::fonts`, and `res::assets` constants. Day stages each resource
for the platform’s native packaging and loading tools.

| Project dir | Kind | API (typed) | Native store |
|---|---|---|---|
| `resource/images/` | processed images | [`image(res::images::logo)`](https://docs.rs/day-pieces/latest/day_pieces/fn.image.html) | SwiftPM `.process` → `Assets.car` (iOS) · bundle file (macOS) · `res/drawable` → `R` (Android) · GResource (GTK) · `.qrc` (Qt) · MRT / loose (WinUI) · rawfile (ArkUI) |
| `resource/assets/` | arbitrary data | [`resource(res::assets::stations_json)`](https://docs.rs/day-core/latest/day_core/fn.resource.html) | bundle file + mmap (Apple) · [`AAssetManager`](https://developer.android.com/ndk/reference/group/asset) (Android) · [`g_resources_lookup_data`](https://docs.gtk.org/gio/func.resources_lookup_data.html) (GTK) · [`QResource`](https://doc.qt.io/qt-6/qresource.html) (Qt) · loose file (WinUI) · rawfile fd (ArkUI) |
| `resource/fonts/` | custom fonts | [`Font::custom(res::fonts::family, pt)`](https://docs.rs/day-spec/latest/day_spec/enum.Font.html#method.custom) | CoreText registration (Apple) · `res/font` → `R.font` (Android) · fontconfig/CoreText (GTK) · [`QFontDatabase`](https://doc.qt.io/qt-6/qfontdatabase.html) (Qt) · WinUI `ms-appx:///fonts/<file>#family` (WinUI) · rawfile + ArkTS `registerFont` (ArkUI) |

## Referencing resources: typed constants

You don't reference bundled resources by bare string. An app's `build.rs` (one line, wired into the
`day new` scaffold) generates a typed constant for every file under `resource/`:

```rust
// build.rs
fn main() { day_build::prebuild_project().expect("day-build: prebuild"); }
```

```rust
// src/lib.rs — surface the generated constants once
day::resources!();
```

```rust
image(res::images::logo)                  // ImageName  ← resource/images/logo.png
resource(res::assets::stations_json)      // AssetName  ← resource/assets/stations.json
Font::custom(res::fonts::pacifico, 24.0)  // FontFamily ← resource/fonts/*.ttf (family "Pacifico")
```

Referencing a resource that isn't bundled is a **compile error**, the available names **autocomplete**,
and dropping a file into `resource/` makes its constant appear on the next build (`cargo:rerun-if-changed`).
Symbols are lowercase `[a-z0-9_]`: the file stem for images, the file name (sanitized, e.g.
`numbers.bin` → `numbers_bin`) for assets, and the parsed font family (`"Special Elite"` →
`special_elite`) for fonts.

`resource/assets/` is a **tree**: directories nest to any depth, ship with their structure
preserved, and generate a matching *module* tree. Each directory is also itself a constant, a
typed `AssetDir` sharing its module's name (consts and modules live in different Rust
namespaces):

```rust
// resource/assets/web/minisite/css/style.css
resource(res::assets::web::minisite::css::style_css)   // AssetName, value "web/minisite/css/style.css"
res::assets::web::minisite                              // AssetDir, value "web/minisite"
res::assets::web::minisite.join(relative_path)         // path selected at runtime
```

An `AssetName`'s value is the `/`-relative path, which is exactly what every native store
resolves (`file` paths, [`AAssetManager`](https://developer.android.com/ndk/reference/group/asset) paths, gresource/qrc aliases, rawfile paths). File and
directory symbols share one namespace per level, so `site.old` beside `site-old/` is a build
error. The inline web view builds on directory constants: `web_view_inline(res::assets::web::minisite)`
serves the subtree as a bundled site (day-piece-webview's
[docs/webview.md](/docs/reference/webview)).

For a name known only at runtime, opt in explicitly; this bypasses the presence guarantee:

```rust
image(ImageName::dynamic(user_choice))
resource(AssetName::dynamic(format!("page-{n}.json")))
```

`day-build` also fails the build with a fix hint if an image stem isn't **portable** across toolkits
(e.g. `Logo.png` resolves verbatim on Apple/GTK/Qt but as `logo` on Android/ArkUI; rename to
`logo.png`), or if two files **collide** on one symbol. The same crate is the single source of the
name→identifier rule the CLI stagers use, so a constant's string is exactly the name staged natively.

## Images: [`image(res::images::name)`](https://docs.rs/day-pieces/latest/day_pieces/fn.image.html)

Drop `resource/images/logo.png` (optionally `logo@2x.png`, `logo@3x.png`) in the project. `day build` stages
each image into the target's native image pipeline; [`image(res::images::logo)`](https://docs.rs/day-pieces/latest/day_pieces/fn.image.html) (the piece) then
resolves the name through the native by-name API. Nothing about the piece API changes, only how
the toolkit resolves the name.

- **iOS (UIKit):** a generated `Media.xcassets` is placed in the `DayResources` SwiftPM package with
  `resources: [.process(...)]`. xcodebuild runs `actool` → an optimized, deduplicated `Assets.car` in
  `DayResources_DayResources.bundle`; the toolkit loads via
  [`UIImage(named:in:compatibleWith:)`](https://developer.apple.com/documentation/uikit/uiimage).
- **macOS (AppKit):** the image is a file in the `.app` bundle, loaded with
  [`NSImage(contentsOfFile:)`](https://developer.apple.com/documentation/appkit/nsimage); the `platform/macos/` Xcode host stages images into
  `Contents/Resources` through its `day xcode-backend stage-resources` phase rather than through
  an asset catalog. (Optimization is whatever the source already is; no `actool` runs for
  images.)
- **Android:** staged into `res/drawable*/` (density buckets from `@Nx` variants); aapt2 crunches and
  assigns an `R.drawable` id. Runtime resolves the name with `Resources.getIdentifier(name,
  "drawable", pkg)` (cached) → `getDrawable`.
- **GTK / Qt:** compiled into the binary as a GResource / `.qrc` and loaded by a stable virtual path
  (`/dev/<appid>/logo.png`, `:/logo.png`).
- **WinUI:** staged as `logo.scale-{100,200,400}.png`; MRT auto-selects by DPI when packaged, a
  WIC/DPI resolver picks the file when unpackaged.
- **ArkUI:** staged into `resources/rawfile/day/`; the native NodeAPI image node is set to
  `resource://RAWFILE/day/logo.png` (rawfile is the only store the OpenHarmony NDK can reach).

## Data: [`resource(res::assets::name)`](https://docs.rs/day-core/latest/day_core/fn.resource.html)

[`day::resource(res::assets::stations_json)`](https://docs.rs/day-app/latest/day/fn.resource.html) returns a `Resource` with efficient random read-only
access, backed directly by the native store (zero-copy where the platform exposes a stable pointer):

```rust
let res = day::resource(res::assets::stations_json).expect("bundled");
let all: &[u8] = res.as_slice();          // zero-copy view
let n = res.len();                        // byte length
let mut hdr = [0u8; 16];
res.read_at(0, &mut hdr);                 // random access, no allocation
let owned: Vec<u8> = res.to_vec();        // copy out if you need ownership
```

Backing per platform: Apple = mmap of the bundle file (the "plain file handle"); Android = NDK
[`AAssetManager`](https://developer.android.com/ndk/reference/group/asset) (`AAsset_getBuffer` on an uncompressed asset, zero copy); GTK =
[`g_resources_lookup_data`](https://docs.gtk.org/gio/func.resources_lookup_data.html); Qt = [`QResource::data`](https://doc.qt.io/qt-6/qresource.html); ArkUI = `OH_ResourceManager_GetRawFileDescriptor`
+ mmap; desktop dev / host tests = mmap of `DAY_ASSET_ROOT/<name>`. The active toolkit registers its
opener once via [`day_core::set_resource_opener`](https://docs.rs/day-core/latest/day_core/fn.set_resource_opener.html); absent that, the default mmap-file opener is used
(which is exactly the Apple path). See `crates/day-core/src/resource.rs`.

## Fonts: [`Font::custom(res::fonts::family, pt)`](https://docs.rs/day-spec/latest/day_spec/enum.Font.html#method.custom)

Bundled families also appear in [`day::font_families()`](https://docs.rs/day-app/latest/day/fn.font_families.html) and draw on a canvas through
`CanvasFont` — see [docs/fonts](fonts.md).

`resource/fonts/*.{ttf,otf}` are referenced by the **family name** embedded in the file's sfnt `name`
table, never by file name. One invariant makes the name resolve everywhere without a side
table. `day build` parses the name table ([`day_spec::fonts::parse_font_names`](https://docs.rs/day-fonts/latest/day_fonts/fn.parse_font_names.html), a ~100-line
bounds-checked sfnt reader shared by the CLI and the runtimes) and derives every staged name from
the family via `font_ident` ("Special Elite" → `special_elite`), so a runtime can re-derive it
from the requested name. The size scales with the platform accessibility text scale exactly like
`Font::System` ([UIFontMetrics](https://developer.apple.com/documentation/uikit/uifontmetrics) on iOS, `sp` on Android, text-scaling-factor on GTK, …).

Per platform:

- **macOS (AppKit):** files register with [`CTFontManagerRegisterFontsForURL`](https://developer.apple.com/documentation/coretext/ctfontmanagerregisterfontsforurl(_:_:_:)) (process scope) in
  [`run()`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.run); [`NSFont(name:size:)`](https://developer.apple.com/documentation/appkit/nsfont) then resolves family/full/PostScript names. Dev launch reads
  `DAY_FONT_ROOT` (set by `day launch` to the project's `resource/fonts/`); packed apps read
  `Contents/Resources/fonts` (copied by `day pack`).
- **iOS (UIKit):** fonts ride the DayPieces SwiftPM bundle as a `.copy("fonts")` resource
  (`DayPieces_DayPieces.bundle/fonts/…`): `.copy`, not `.process`, so the bytes land verbatim.
  `day build` also syncs a [`UIAppFonts`](https://developer.apple.com/documentation/bundleresources/information-property-list/uiappfonts) array into `platform/ios/Runner/Info.plist` (managed key,
  rewritten each build), and day-toolkit-uikit registers the bundle dir with CoreText at launch; the
  registration covers dev loops and any path iOS declines to load from the plist.
- **Android:** staged as `res/font/<ident>.<ext>`; aapt2 assigns `R.font.<ident>`.
  `DayBridge.setLabelFont` takes the family string, re-derives `<ident>` with the same
  sanitization, resolves via `Resources.getIdentifier(…, "font", pkg)` → `getFont` (API 26+;
  older devices log and fall back), caches the Typeface, and builds
  `Typeface.create(base, weight, italic)` on API 28+.
- **GTK:** `FcConfigAppFontAddFile` on Linux; on macOS both CoreText and fontconfig (Homebrew
  Pango may sit on either fontmap); `AddFontResourceExW(FR_PRIVATE)` best-effort on Windows.
  The label carries a Pango `AttrString::new_family` attribute.
- **Qt:** [`QFontDatabase::addApplicationFont`](https://doc.qt.io/qt-6/qfontdatabase.html) per file at startup (shim `day_qt_register_font`);
  labels get [`QFont::setFamily`](https://doc.qt.io/qt-6/qfont.html) on top of the size/weight/italic font.
- **WinUI:** [`run()`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.run) stages every bundled font into `<exe>/fonts/` (a no-op when packed;
  `day pack` already ships them there). The shim sets
  `FontFamily("ms-appx:///fonts/<file>#<family>")`, resolving the font relative to the executable.
  The family→file mapping is resolved (and cached)
  through [`day_spec::fonts::resolve_font_file`](https://docs.rs/day-fonts/latest/day_fonts/fn.resolve_font_file.html) against `DAY_FONT_ROOT` / the exe-relative `fonts/` dir.
- **ArkUI:** staged into rawfile `day/fonts/` plus a `day/fonts.json` manifest
  (`[{family, file}]`); the platform/harmony scaffold's EntryAbility feeds it to ArkTS
  `font.registerFont` (building the rawfile `Resource` object by hand; `$rawfile()` only takes
  literals) before the native UI loads, and day-toolkit-arkui sets `NODE_FONT_FAMILY`.

Validation (`crates/day-cli/src/resources/mod.rs::scan_fonts`) is hard-error at build time: only
`.ttf`/`.otf` (Android's `res/font` accepts nothing else; the strictest platform sets the rule),
a parseable name table, and no two families colliding on the same sanitized ident. At runtime an
unknown family logs `day: unknown font family …` and falls back to the system font; a missing
font is a visual bug, never a crash. Weight overrides on custom fonts map to synthesized bold
(`>= Semibold`) where the family has no such face.

## Scaling: [`image`](https://docs.rs/day-pieces/latest/day_pieces/fn.image.html)`(res::images::logo)`[`.content_mode`](https://docs.rs/day-pieces/latest/day_pieces/trait.ImageBuilder.html#tymethod.content_mode)`(…)` / [`.aspect_ratio(…)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ImageBuilder.html#tymethod.aspect_ratio)

Images scale with `ContentMode::Fit` by default (preserve aspect, letterbox, never stretch). Tune
with [`.content_mode(ContentMode::Fill)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ImageBuilder.html#tymethod.content_mode) (preserve aspect, crop), [`.stretch()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ImageBuilder.html#tymethod.stretch), or the shorthands
[`.fit()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ImageBuilder.html#tymethod.fit)/[`.fill()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ImageBuilder.html#tymethod.fill); constrain the frame to a ratio with [`.aspect_ratio(16.0/9.0)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ImageBuilder.html#tymethod.aspect_ratio). Each maps to the
native scaler: NSImageView `imageScaling`, [UIImageView](https://developer.apple.com/documentation/uikit/uiimageview) `contentMode`, [GtkPicture](https://docs.gtk.org/gtk4/class.Picture.html) `content-fit`, a
Qt aspect-painting label, Android `ImageView.ScaleType`, WinUI `Image.Stretch`, ArkUI
`NODE_IMAGE_OBJECT_FIT`.

## Build-time staging

`crates/day-cli/src/resources/` scans `images/`, `assets/`, and `fonts/` and, before the platform
build, dispatches to a per-toolkit stager. GTK compiles a `.gresource` blob (`glib-compile-resources`) and
Qt a `.rcc` blob (`rcc -binary -no-compress`); both are registered at startup and loaded natively
([`g_resources_lookup_data`](https://docs.gtk.org/gio/func.resources_lookup_data.html) / [`gtk_picture_new_for_resource`](https://docs.gtk.org/gtk4/ctor.Picture.new_for_resource.html), [`QResource::data`](https://doc.qt.io/qt-6/qresource.html) / [`QPixmap(":/…")`](https://doc.qt.io/qt-6/qpixmap.html)).
Android copies images into `res/drawable*/` and iOS into a `.process` `Media.xcassets`; ArkUI copies
into `rawfile/`. Day performs no image/SVG processing of its own; the native build system optionally
optimizes.

## Limits

- ArkUI uses `rawfile` (native-reachable, uncompressed) with no per-density auto-selection yet; a
  `media/` + ArkTS-bridge path for density is a future enhancement.
- macOS AppKit does not run actool (cargo build), so its images are unoptimized bundle files.
- WinUI is built unpackaged, so images/data load as loose files via WIC/the file opener (the
  recommended unpackaged path); MRT `.pri` (`ms-appx:///`) applies only to MSIX-packaged apps.
- WinUI unpackaged uses loose scale-suffixed files + a DPI resolver (MRT/`.pri` needs MSIX).
