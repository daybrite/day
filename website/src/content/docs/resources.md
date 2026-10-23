---
navTitle: "Resources"
title: Resources, images, fonts & icons
description: "How resource/assets, images, vectors, fonts, and icons travel from your project into each platform's native resource system, and how to read them back."
order: 24
section: How-to guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Images, fonts, icons, and data files are bundled with a Day app from its `resource/` directory.
The build prepares each file for the selected platform and generates typed names for use in Rust.
The subdirectories distinguish how each resource is used:

```text
myapp/
  resource/
    assets/    # data files: JSON, databases — anything you open as bytes
    images/    # raster UI images, with @2x/@3x density variants
    vectors/   # SVG glyphs, staged natively per backend
    fonts/     # custom fonts (.ttf/.otf), referenced by family name
    icons/     # one app-icon master; day icon build generates the renditions
```

Staging depends on both the resource type and the target. Android images become
`res/drawable-*` entries; iOS images join an asset catalog; GTK and Qt can bundle data and
images in their resource systems. Desktop fonts can remain loose files registered at startup.
`day build` runs the target's staging path before its platform build.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/examples/catalog) and [catalog setup](/docs/how-to/localization).

## Typed names, generated at build

> [!TIP]
> **Use the generated names for known resources.** `res::images::logo` and
> `res::assets::stations_json` let the compiler catch missing files and misspellings.
> Reserve dynamic names for assets selected at runtime. The same rule applies to
> human-readable text: use generated `res::str` accessors from the
> [Fluent catalog](/docs/how-to/localization).

You reference bundled resources through generated constants, not bare strings. The scaffold's
`build.rs` calls [`day_build::prebuild_project()`](https://daybrite.dev/api/day_build/fn.prebuild_project.html), which writes a typed constant per file:

```rust
image(res::images::wave);                 // ← resource/images/wave.png
day::resource(res::assets::stations_json); // ← resource/assets/stations.json
vector(res::vectors::home);               // ← resource/vectors/home.svg
```

A typo, or a file that was renamed or deleted, is a compile error, and the names autocomplete.
Dropping a file into `resource/` makes its constant appear on the next build. For a name known
only at runtime, [`ImageName::dynamic(…)`](https://daybrite.dev/api/day_core/struct.ImageName.html#method.dynamic) / [`AssetName::dynamic(…)`](https://daybrite.dev/api/day_core/struct.AssetName.html#method.dynamic) opt out of the presence
guarantee explicitly.

## Data files: `resource/assets/`

Anything in `resource/assets/` is packaged and readable at runtime through one call:

```rust
let bytes: day::Resource = day::resource(res::assets::stations_json)
    .ok_or_else(|| res::str::resource_missing().format())?;
let parsed: Stations = serde_json::from_slice(bytes.as_slice())?;
```

This integration fragment requires an app-defined `Stations` type and `serde_json`. The
`resource_missing` message is defined in the [complete resource example](#handle-a-missing-bundled-file).
Handle parsing errors at the caller and map them to localized UI messages too.

[`Resource::as_slice()`](https://daybrite.dev/api/day_core/struct.Resource.html#method.as_slice) borrows the opened bytes without another copy. The opener may use native
storage, a memory map, or owned bytes; zero-copy opening is not guaranteed on every backend.
[`read_at(offset, buf)`](https://daybrite.dev/api/day_core/struct.Resource.html#method.read_at) gives random access for large files. During development the same call
resolves against your project directory, so editing an asset and relaunching picks it up without
a packaging step.

## Images: `resource/images/`

Drop PNGs (with optional `@2x`/`@3x` density variants) into `resource/images/` and reference them by
name:

```rust
image(res::images::wave)   // finds wave.png / wave@2x.png / wave@3x.png
    .frame(240.0, 120.0)
```

At build time each [toolkit](/docs/reference/glossary#toolkit) gets the format it expects: density buckets on Android
(`drawable-xhdpi/…`), an asset catalog imageset on iOS, resource bundles on GTK/Qt. The
platform picks the right density at runtime the same way it does for any native app. The
[resources reference](/docs/reference/resources) documents the exact per-platform staging.

- **`resource/images/` is raster.** Photos and artwork belong here, with `@2x`/`@3x` density
  variants; SVG glyphs belong in `resource/vectors/` (next section), which ships them as
  vectors.
- **Remote images** (URL-loaded, cached) are a separate [piece](/docs/reference/glossary#piece), [`day-piece-remote-image`](/docs/reference/images),
  because they involve networking and cache policy that the core leaves to the piece.

## Vector glyphs: `resource/vectors/`

SVG glyphs (nav and toolbar icons, symbols) go in `resource/vectors/` and render
resolution-independent through the [`vector`](https://daybrite.dev/api/day_pieces/fn.vector.html) piece:

```rust
vector(res::vectors::home).tint(accent).frame(24.0, 24.0)
```

Vector names share the image namespace, so nav items, tab icons, and toolbar buttons accept them
unchanged. Source art can be a plain `.svg`, an SF Symbols template export, or a `.symbolset`
bundle; text has to be outlined, since Day compiles in no text shaping.

### What each target ships

`day build` stages every glyph into the form its toolkit loads natively, and prints the split as
it goes; `Vectors xaml: 81/81 glyph(s) vector` means every glyph converted and none fell back.

| Target | Ships as | Tint |
|---|---|---|
| [ios-uikit](/docs/reference/platforms/ios-uikit) | the SVG in an asset catalog with `preserves-vector-representation`, so `UIImage` renders at display size | `tintColor` on a template image |
| [macos-appkit](/docs/reference/platforms/macos-appkit) | the SVG itself — `NSImage` renders SVG files at display size (macOS 11+) | `contentTintColor` on a template image |
| [android-mdc](/docs/reference/platforms/android-mdc) | a **VectorDrawable** in `res/drawable/` | `setImageTintList` |
| [windows-winui](/docs/reference/platforms/windows-winui) | **XAML geometry** — a `Path` in a scaling `Viewbox`, a `PathIcon` in the nav pane, redrawn at every size | a brush on the shapes |
| [web-dom](/docs/reference/platforms/web-dom) | the SVG, rendered by the browser | a CSS mask painted with the tint |
| [harmony-arkui](/docs/reference/platforms/harmony-arkui) | the SVG in `rawfile`, rendered by ArkUI's `Image` | SVG fill color |
| [linux-gtk](/docs/reference/platforms/linux-gtk), macos-gtk, windows-gtk | retained SVG rendered by librsvg 2.52+ and Cairo at the requested size, including the [`vector`](https://daybrite.dev/api/day_pieces/fn.vector.html) piece | an alpha-preserving render-node color matrix |
| [linux-qt](/docs/reference/platforms/linux-qt), macos-qt, windows-qt | retained SVG rendered by `QSvgRenderer` at the requested size, including the [`vector`](https://daybrite.dev/api/day_pieces/fn.vector.html) piece | alpha compositing at the current paint resolution |

Qt SVG and librsvg development libraries are required at build time. On Debian/Ubuntu,
install `qt6-svg-dev` for Qt or `librsvg2-dev` (2.52+) for GTK. Homebrew supplies them through
`qt` and `librsvg`; MSYS2 supplies `qt6-svg` and `librsvg` with the matching environment prefix.
See [system requirements](/docs/reference/system-requirements) for complete commands. `day doctor`
checks both dependencies and enforces librsvg's minimum version. GTK controls that accept
only icon-theme names, including settings group icons and `AdwViewSwitcher`, still use the
symbolic PNG cache; ordinary vector pieces and image widgets retain their SVG geometry.

> [!NOTE]
> Every [backend](/docs/reference/glossary#backend) recolors a template glyph, and on six of them (AppKit, UIKit, Android, GTK, Qt,
> and web) a tint bound to a [signal](/docs/reference/glossary#signal) repaints the realized view instead of rebuilding it. XAML and
> ArkUI take the tint when the glyph is realized, so a [reactive](/docs/reference/glossary#reactive) tint there lands on the next
> rebuild.

### Where a vector degrades to a raster

Two toolkits draw the art themselves rather than handing an SVG to the platform, and both accept
the same subset: **solid fills and strokes, in either fill rule**. Art outside it (gradients,
clipping paths, masks, filters) stages no geometry, and that glyph alone ships as a raster
instead:

- **Android** falls back at xxxhdpi. `day lint` reports it as `day::lint::vector-raster-fallback`
  whenever `android-mdc` is a declared target, so you hear about a gradient in an icon at lint
  time, before it reaches a device.
- **Windows** falls back the same way, and the tint degrades with it: a monochrome `BitmapIcon`
  over the raster rather than a brush on geometry.

The fallback is per glyph, not per app. Day ships the raster only for art that could not convert,
because bundling one for every glyph would double the payload and let a broken vector path hide
behind a raster that still looks right.

> [!NOTE]
> It is usually better to remove a gradient from an icon. Two flat glyphs read
> better at 24 pt than one gradient glyph, and they stay crisp on the two targets that would
> otherwise rasterize them.

### Weights

[`.weight(VectorWeight::Light | Bold)`](https://daybrite.dev/api/day_pieces/trait.VectorBuilder.html#tymethod.weight) selects per-weight art on every backend. Template sources
(SF Symbols exports, `.symbolset` bundles) carry real Light and Bold variants; a plain SVG aliases
all three to the same glyph, so the call degrades to Regular rather than to a missing asset.

The [vectors reference](/docs/reference/vectors) covers the staging in full, including how packed
apps carry these forms without the launch environment.

## Custom fonts: `resource/fonts/`

Drop `.ttf` or `.otf` files into `resource/fonts/` and reference them through the generated
constant, which carries the **family name** baked into the font file itself (what Font Book or
fontconfig report), not the file name:

```rust
label(res::str::welcome_aboard()).font(Font::custom(res::fonts::pacifico, 24.0))
```

`day build` stages each font where the platform wants it: `res/font/` on Android (with the
resource-naming rules handled for you), the app bundle plus a `UIAppFonts` Info.plist entry on
iOS, a fonts directory registered with CoreText / fontconfig / the `QFontDatabase` on the
desktops, rawfile plus an ArkTS `registerFont` manifest on HarmonyOS. Each backend registers
everything at startup. The point size scales with the platform's accessibility text size, exactly
like `Font::System(pt)`.

The following restrictions are enforced as **hard errors at build time**, because each would
otherwise surface as a confusing runtime-only failure on one platform:

- **`.ttf` and `.otf` only:** the staging scanner rejects other extensions, including font
  collections (`.ttc`), and requires a readable family name. It does not reject a font merely
  for having variation tables. The portable font API does not expose arbitrary variation axes;
  check the resulting face and synthesized styles on your targets.
- **One face per family:** Staged file names are derived from the family name (lowercased,
  `[a-z0-9_]`), so a second face of the same family would collide. Ship the regular face; bold
  and italic are synthesized where the platform can.
- **File names don't matter; family names do.** `resource/fonts/SpecialElite-Regular.ttf` whose embedded
  family is "Special Elite" generates `res::fonts::special_elite`, used as
  [`Font::custom(res::fonts::special_elite, 20.0)`](https://daybrite.dev/api/day_spec/enum.Font.html#method.custom). (`Font::Custom("Special Elite", 20.0)` is the
  unchecked alternative for a family name only known at runtime.)

Outside those rules, an unknown family never breaks the app. The label
renders in the system font and the log names the family that didn't resolve. [`.weight(...)`](https://daybrite.dev/api/day_pieces/trait.LabelBuilder.html#tymethod.weight) and
[`.italic()`](https://daybrite.dev/api/day_pieces/trait.LabelBuilder.html#tymethod.italic) still apply, but a single-face family only gets what the platform can synthesize (a
heavier stroke, a slant), not true bold or italic cuts.

## The app icon: `resource/icons/`

`resource/icons/` holds one master (`icon.svg`, `day-icon.svg`, or `icon.png`), and that is
the only icon file in the repository. `day prepare` renders every platform's set from it
(`.icns`, `.ico`, Android's adaptive and themed icons, the HarmonyOS layered icon, the Xcode
asset catalogs, an Icon Composer package) under `build/day/host/`, which the host projects
reference by path and every build refreshes when the master changes. A per-platform override
beside the master (`macos.svg`, `android.svg`, …) replaces it for that platform alone. The
[icons guide](/docs/how-to/guide-icons) covers the layered master and the outputs.

## Localized strings are resources too

`resource/locales/<lang>/app.ftl` files are compiled in via `include_str!` at the moment
([localization guide](/docs/how-to/localization)), and OS-facing strings (the app's display name) are
conveyed into platform manifests at build time. Piece packages can carry their own `locales/` and
resources, which aggregate into your app without name collisions.

## What happens at build

```text
resource/images/wave@2x.png ─┐  resource/fonts/Pacifico-Regular.ttf ─┐  resource/assets/stations.json ─┐
                     ▼          day build -t <target>   ▼                            ▼
   ┌───────────────────────────────────────────────────────────────────────┐
   │ android  → res/drawable-xhdpi/wave.png · res/font/pacifico.ttf        │
   │ ios      → DayPieces asset catalog + fonts/ bundle dir + UIAppFonts   │
   │ gtk/qt   → app.gresource / app.rcc; fonts registered at startup       │
   │ arkui    → hap rawfile/ (+ day/fonts.json → registerFont)             │
   │ desktop dev-launch → read from project dirs directly                  │
   └───────────────────────────────────────────────────────────────────────┘
```

Development launch can use loose resource files, so a successful development run does not
prove that a package contains them. Check staging diagnostics and test the packaged app without
access to the source directory. Missing-tool behavior depends on the target's staging path;
do not rely on a development fallback for distribution.

## Validate generated access and runtime loading

Start with the scaffold's [`day_build::prebuild_project()`](https://daybrite.dev/api/day_build/fn.prebuild_project.html) in `build.rs` and `day::resources!()`
in the app crate. Add the resource, rebuild, then use its generated `res::assets`, `res::images`,
`res::vectors`, or `res::fonts` constant. A generated identifier proves the name was discovered
at build time; it does not prove runtime packaging, decoding, or backend support. Handle the
resource API's missing/error result where applicable.

The snippets are integration fragments requiring the files named in their comments. Do not use
`include_bytes!`, `include_str!`, or dynamic-name constructors for known app bundles. The generator
may use inclusion internally; that is not an app-level bypass. Open a bundled web document through
its generated AssetDir constant and leave its internal links document-relative. Supply embedded
web text from generated localized accessors with no English fallback.

Source: [resource generation](https://github.com/daybrite/day/blob/main/crates/day-build/src/lib.rs)
and [platform staging](https://github.com/daybrite/day/tree/main/crates/day-cli/src/resources).
Check that bundled images and fonts load in each packaged target, including paths used only
by a secondary screen.

## Handle a missing bundled file

Create `resource/assets/recipe_data.json` with the JSON below. Rebuild to generate its
`res::assets` constant. This module reads the byte length and gives a localized error when the
runtime resource cannot be opened. A generated name checks discovery, not packaging success.

Complete module for `src/resource_recipe.rs` in the
[technical recipe harness](/docs/reference/examples/catalog#technical-recipes).
Add the matching Fluent messages below. This module is compiled as part of the harness;
it is not a standalone app.

<!-- technical-example: resource_recipe -->
```rust
use crate::res;
use day::prelude::*;

pub fn bundled_data() -> impl Piece {
    match day::resource(res::assets::recipe_data_json) {
        Some(data) => label(res::str::resource_bytes(data.len() as i64)).id(id!(resource_status)),
        None => label(res::str::resource_missing()).id(id!(resource_status)),
    }
}
```

<!-- technical-catalog: resource_recipe -->
```ftl
resource_bytes = Bundled data: { $bytes } bytes
resource_missing = The bundled data could not be opened.
```

Synthetic data for the example, not a published data set:

<!-- technical-resource: resource/assets/recipe_data.json -->
```json
{"stations": []}
```

The harness checks the generated accessor and exercises both successful loading and a missing
file. It does not establish image decoding, font rendering, or packaged resource staging on
other platforms.
