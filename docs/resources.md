# Resources (§18.3, §18.4)

Day apps bundle three kinds of resource, all looked up by name, all routed through each platform's
native resource machinery so they get the platform's optimizations and load paths for free. Day
never rewrites your pixels or bytes itself. It hands the raw files to the native build system, which
optionally optimizes them (actool re-encodes/dedupes, aapt2 crunches, …). Data is stored
uncompressed wherever the platform allows, so the runtime can return a zero-copy view.

| Project dir | Kind | API | Native store |
|---|---|---|---|
| `images/` | processed images | `image("logo")` | SwiftPM `.process` → `Assets.car` (iOS) · bundle file (macOS) · `res/drawable` → `R` (Android) · GResource (GTK) · `.qrc` (Qt) · MRT / loose (WinUI) · rawfile (ArkUI) |
| `assets/` | arbitrary data | `resource("stations.json")` | bundle file + mmap (Apple) · `AAssetManager` (Android) · `g_resources_lookup_data` (GTK) · `QResource` (Qt) · loose file (WinUI) · rawfile fd (ArkUI) |
| `fonts/` | custom fonts | `Font::Custom("Family", pt)` | CoreText registration (Apple) · `res/font` → `R.font` (Android) · fontconfig/CoreText (GTK) · `QFontDatabase` (Qt) · XAML `path#family` (WinUI) · rawfile + ArkTS `registerFont` (ArkUI) |

## Images — `image("name")`

Drop `images/logo.png` (optionally `logo@2x.png`, `logo@3x.png`) in the project. `day build` stages
each image into the target's native image pipeline; `image("logo")` (the existing piece) then
resolves the name through the native by-name API. Nothing about the piece API changes, only how
the backend resolves the name.

- **iOS (UIKit):** a generated `Media.xcassets` is placed in the `DayResources` SwiftPM package with
  `resources: [.process(...)]`. xcodebuild runs `actool` → an optimized, deduplicated `Assets.car` in
  `DayResources_DayResources.bundle`; the backend loads via
  `UIImage(named:in:compatibleWith:)`.
- **macOS (AppKit):** the app is a plain cargo binary (no xcodebuild/actool), so the image is a file
  in the `.app` bundle, loaded with `NSImage(contentsOfFile:)`. (Optimization is whatever the source
  already is; there is no actool step off the Xcode build.)
- **Android:** staged into `res/drawable*/` (density buckets from `@Nx` variants); aapt2 crunches and
  assigns an `R.drawable` id. Runtime resolves the name with `Resources.getIdentifier(name,
  "drawable", pkg)` (cached) → `getDrawable`.
- **GTK / Qt:** compiled into the binary as a GResource / `.qrc` and loaded by a stable virtual path
  (`/dev/<appid>/logo.png`, `:/logo.png`).
- **WinUI:** staged as `logo.scale-{100,200,400}.png`; MRT auto-selects by DPI when packaged, a
  WIC/DPI resolver picks the file when unpackaged.
- **ArkUI:** staged into `resources/rawfile/day/`; the native NodeAPI image node is set to
  `resource://RAWFILE/day/logo.png` (rawfile is the only store the OpenHarmony NDK can reach).

## Data — `resource("name")`

`day::resource("stations.json")` returns a `Resource` with efficient random read-only access, backed
directly by the native store (zero-copy where the platform exposes a stable pointer):

```rust
let res = day::resource("stations.json").expect("bundled");
let all: &[u8] = res.as_slice();          // zero-copy view
let n = res.len();                        // byte length
let mut hdr = [0u8; 16];
res.read_at(0, &mut hdr);                 // random access, no allocation
let owned: Vec<u8> = res.to_vec();        // copy out if you need ownership
```

Backing per platform: Apple = mmap of the bundle file (the "plain file handle"); Android = NDK
`AAssetManager` (`AAsset_getBuffer` on an uncompressed asset, zero copy); GTK =
`g_resources_lookup_data`; Qt = `QResource::data`; ArkUI = `OH_ResourceManager_GetRawFileDescriptor`
+ mmap; desktop dev / host tests = mmap of `DAY_ASSET_ROOT/<name>`. The active backend registers its
opener once via `day_core::set_resource_opener`; absent that, the default mmap-file opener is used
(which is exactly the Apple path). See `crates/day-core/src/resource.rs`.

## Fonts — `Font::Custom("Family", pt)` (§18.4)

`fonts/*.{ttf,otf}` are referenced by the **family name** embedded in the file's sfnt `name`
table, never by file name. The single invariant that makes the name resolve everywhere with no
side table: `day build` parses the name table (`day_spec::fonts::parse_font_names` — a ~100-line
bounds-checked sfnt reader shared by the CLI and the runtimes) and derives every staged name from
the family via `font_ident` ("Special Elite" → `special_elite`), so a runtime can re-derive it
from the requested name. The size scales with the platform accessibility text scale exactly like
`Font::System` (UIFontMetrics on iOS, `sp` on Android, text-scaling-factor on GTK, …).

Per platform:

- **macOS (AppKit):** files register with `CTFontManagerRegisterFontsForURL` (process scope) in
  `run()`; `NSFont(name:size:)` then resolves family/full/PostScript names. Dev launch reads
  `DAY_FONT_ROOT` (set by `day launch` to the project's `fonts/`); packed apps read
  `Contents/Resources/fonts` (copied by `day pack`).
- **iOS (UIKit):** fonts ride the DayPieces SwiftPM bundle as a `.copy("fonts")` resource
  (`DayPieces_DayPieces.bundle/fonts/…`) — `.copy`, not `.process`, so the bytes land verbatim.
  `day build` ALSO syncs a `UIAppFonts` array into `platform/ios/Runner/Info.plist` (managed key,
  rewritten each build), and day-uikit registers the bundle dir with CoreText at launch — the
  registration covers dev loops and any path iOS declines to load from the plist.
- **Android:** staged as `res/font/<ident>.<ext>`; aapt2 assigns `R.font.<ident>`.
  `DayBridge.setLabelFont` takes the family string, re-derives `<ident>` with the same
  sanitization, resolves via `Resources.getIdentifier(…, "font", pkg)` → `getFont` (API 26+;
  older devices log and fall back), caches the Typeface, and builds
  `Typeface.create(base, weight, italic)` on API 28+.
- **GTK:** `FcConfigAppFontAddFile` on Linux; on macOS BOTH CoreText and fontconfig (Homebrew
  Pango may sit on either fontmap); `AddFontResourceExW(FR_PRIVATE)` best-effort on Windows.
  The label carries a Pango `AttrString::new_family` attribute.
- **Qt:** `QFontDatabase::addApplicationFont` per file at startup (shim `day_qt_register_font`);
  labels get `QFont::setFamily` on top of the size/weight/italic font.
- **WinUI:** unpackaged Win32 XAML has no registration API — the shim sets
  `FontFamily("<absolute path>#<family>")`, with the family→file mapping resolved (and cached)
  through `day_spec::fonts::resolve_font_file` against `DAY_FONT_ROOT` / the exe-relative
  `fonts/` dir.
- **ArkUI:** staged into rawfile `day/fonts/` plus a `day/fonts.json` manifest
  (`[{family, file}]`); the platform/ohos scaffold's EntryAbility feeds it to ArkTS
  `font.registerFont` (building the rawfile `Resource` object by hand — `$rawfile()` only takes
  literals) before the native UI loads, and day-arkui sets `NODE_FONT_FAMILY`.

Validation (`crates/day-cli/src/resources/mod.rs::scan_fonts`) is hard-error at build time: only
`.ttf`/`.otf` (Android's `res/font` accepts nothing else — the strictest platform sets the rule),
a parseable name table, and no two families colliding on the same sanitized ident. At runtime an
unknown family logs `day: unknown font family …` and falls back to the system font — a missing
font is a visual bug, never a crash. Weight overrides on custom fonts map to synthesized bold
(`>= Semibold`) where the family has no such face.

## Scaling — `image("logo").content_mode(…)` / `.aspect_ratio(…)`

Images scale with `ContentMode::Fit` by default (preserve aspect, letterbox, never stretch). Tune
with `.content_mode(ContentMode::Fill)` (preserve aspect, crop), `.stretch()`, or the shorthands
`.fit()`/`.fill()`; constrain the frame to a ratio with `.aspect_ratio(16.0/9.0)`. Each maps to the
native scaler: NSImageView `imageScaling`, UIImageView `contentMode`, GtkPicture `content-fit`, a
Qt aspect-painting label, Android `ImageView.ScaleType`, WinUI `Image.Stretch`, ArkUI
`NODE_IMAGE_OBJECT_FIT`.

## Build-time staging

`crates/day-cli/src/resources/` scans `images/`, `assets/`, and `fonts/` and, before the platform
build, dispatches to a per-toolkit stager. GTK compiles a `.gresource` blob (`glib-compile-resources`) and
Qt a `.rcc` blob (`rcc -binary -no-compress`); both are registered at startup and loaded natively
(`g_resources_lookup_data` / `gtk_picture_new_for_resource`, `QResource::data` / `QPixmap(":/…")`).
Android copies images into `res/drawable*/` and iOS into a `.process` `Media.xcassets`; ArkUI copies
into `rawfile/`. Day performs no image/SVG processing of its own; the native build system optionally
optimizes.

## Notes / limits

- ArkUI uses `rawfile` (native-reachable, uncompressed) with no per-density auto-selection yet; a
  `media/` + ArkTS-bridge path for density is a future enhancement.
- macOS AppKit does not run actool (cargo build), so its images are unoptimized bundle files.
- WinUI is built unpackaged, so images/data load as loose files via WIC/the file opener (the
  recommended unpackaged path); MRT `.pri` (`ms-appx:///`) applies only to MSIX-packaged apps.
- WinUI unpackaged uses loose scale-suffixed files + a DPI resolver (MRT/`.pri` needs MSIX).
