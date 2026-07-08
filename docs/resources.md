# Resources (§18.3)

Day apps bundle two kinds of resource, both looked up by name, both routed through each platform's
native resource machinery so they get the platform's optimizations and load paths for free. Day
never rewrites your pixels or bytes itself. It hands the raw files to the native build system, which
optionally optimizes them (actool re-encodes/dedupes, aapt2 crunches, …). Data is stored
uncompressed wherever the platform allows, so the runtime can return a zero-copy view.

| Project dir | Kind | API | Native store |
|---|---|---|---|
| `images/` | processed images | `image("logo")` | SwiftPM `.process` → `Assets.car` (iOS) · bundle file (macOS) · `res/drawable` → `R` (Android) · GResource (GTK) · `.qrc` (Qt) · MRT / loose (WinUI) · rawfile (ArkUI) |
| `assets/` | arbitrary data | `resource("stations.json")` | bundle file + mmap (Apple) · `AAssetManager` (Android) · `g_resources_lookup_data` (GTK) · `QResource` (Qt) · loose file (WinUI) · rawfile fd (ArkUI) |

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

## Scaling — `image("logo").content_mode(…)` / `.aspect_ratio(…)`

Images scale with `ContentMode::Fit` by default (preserve aspect, letterbox, never stretch). Tune
with `.content_mode(ContentMode::Fill)` (preserve aspect, crop), `.stretch()`, or the shorthands
`.fit()`/`.fill()`; constrain the frame to a ratio with `.aspect_ratio(16.0/9.0)`. Each maps to the
native scaler: NSImageView `imageScaling`, UIImageView `contentMode`, GtkPicture `content-fit`, a
Qt aspect-painting label, Android `ImageView.ScaleType`, WinUI `Image.Stretch`, ArkUI
`NODE_IMAGE_OBJECT_FIT`.

## Build-time staging

`crates/day-cli/src/resources/` scans `images/` and `assets/` and, before the platform build,
dispatches to a per-toolkit stager. GTK compiles a `.gresource` blob (`glib-compile-resources`) and
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
