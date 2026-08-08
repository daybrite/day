---
title: Resources, images, fonts & icons
description: "How resource/assets, images, vectors, fonts, and icons travel from your project into each platform's native resource system — and how to read them back."
order: 24
section: Guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

A Day project keeps its resources under one conventional `resource/` directory, with five
subdirectories, each staged differently at build time:

```text
myapp/
  resource/
    assets/    # data files: JSON, databases — anything you open as bytes
    images/    # raster UI images, with @2x/@3x density variants
    vectors/   # SVG glyphs, staged natively per backend
    fonts/     # custom fonts (.ttf/.otf), referenced by family name
    icons/     # one app-icon master; day icon generates the renditions
```

The principle behind all five: **resources use each platform's native resource system**, not a
custom archive format. On Android your images become real `res/drawable-*` entries crunched by
aapt2; on iOS they join an asset catalog; on GTK they compile into a GResource bundle; on Qt, a
Qt resource file. `day build` does the staging automatically, per target, before the platform
build runs.

## Typed names, generated at build

You reference bundled resources through generated constants, not bare strings. The scaffold's
`build.rs` calls `day_build::generate_resources()`, which writes a typed constant per file:

```rust
image(res::images::wave)                  // ← resource/images/wave.png
day::resource(res::assets::stations_json) // ← resource/assets/stations.json
vector(res::vectors::home)                // ← resource/vectors/home.svg
```

A typo, or a file that was renamed or deleted, is a compile error, and the names autocomplete.
Dropping a file into `resource/` makes its constant appear on the next build. For a name known
only at runtime, `ImageName::dynamic(…)` / `AssetName::dynamic(…)` opt out of the presence
guarantee explicitly.

## Data files: `resource/assets/`

Anything in `resource/assets/` is packaged and readable at runtime through one call:

```rust
let bytes: day::Resource = day::resource(res::assets::stations_json).expect("packaged asset");
let parsed: Stations = serde_json::from_slice(bytes.as_slice())?;
```

`Resource` is a zero-copy view: on Android it borrows straight from the `AAssetManager` buffer,
on GTK from the GResource, on desktop from an mmap (no copy into a Vec unless you make one).
`read_at(offset, buf)` gives random access for large files. During development the same call
resolves against your project directory, so editing an asset and relaunching picks it up without
a packaging step.

## Images: `resource/images/`

Drop PNGs (with optional `@2x`/`@3x` density variants) into `resource/images/` and reference them by
name:

```rust
image(res::images::wave)   // finds wave.png / wave@2x.png / wave@3x.png
    .frame(240.0, 120.0)
```

At build time each toolkit gets the format it expects: density buckets on Android
(`drawable-xhdpi/…`), an asset catalog imageset on iOS, resource bundles on GTK/Qt. The
platform picks the right density at runtime the same way it does for any native app. The
[resources reference](/docs/internal/resources) documents the exact per-platform staging.

Two notes:

- **`resource/images/` is raster.** Photos and artwork belong here, with `@2x`/`@3x` density
  variants; SVG glyphs belong in `resource/vectors/` (next section), which ships them as
  vectors.
- **Remote images** (URL-loaded, cached) are a separate piece,
  [`day-piece-remote-image`](/docs/internal/resources), because they involve networking and
  cache policy the core deliberately doesn't own.

## Vector glyphs: `resource/vectors/`

SVG glyphs (nav and toolbar icons, symbols) go in `resource/vectors/` and render
resolution-independent through the `vector` piece:

```rust
vector(res::vectors::home).tint(accent).frame(24.0, 24.0)
```

Each backend loads the glyph natively where it can: a VectorDrawable on Android, the SVG itself
on macOS (`NSImage` renders it), a vector-preserving imageset on iOS, the SVG on the web and
HarmonyOS. A pre-rendered raster cache is the universal fallback (Qt uses it). Vector names
share the image namespace, so nav items, tab icons, and toolbar buttons accept them unchanged.
The [vectors reference](/docs/internal/vectors) documents the accepted source forms (plain SVG,
SF Symbol templates, `.symbolset` bundles) and the per-backend staging.

## Custom fonts: `resource/fonts/`

Drop `.ttf` or `.otf` files into `resource/fonts/` and reference them **by family name**, the name baked
into the font file itself (what Font Book or fontconfig report), not the file name:

```rust
label("Welcome aboard").font(Font::Custom("Pacifico", 24.0))
```

`day build` stages each font where the platform wants it: `res/font/` on Android (with the
resource-naming rules handled for you), the app bundle plus a `UIAppFonts` Info.plist entry on
iOS, a fonts directory registered with CoreText / fontconfig / the `QFontDatabase` on the
desktops, rawfile plus an ArkTS `registerFont` manifest on HarmonyOS. Each backend registers
everything at startup. The point size scales with the platform's accessibility text size, exactly
like `Font::System(pt)`.

The restrictions, all enforced as **hard errors at build time** (each would otherwise surface as
a confusing runtime-only failure on one platform):

- **`.ttf` and `.otf` only:** Android's `res/font/` accepts nothing else, so Day holds every
  platform to the same rule. Convert collections (`.ttc`) and variable fonts to single static
  faces before bundling.
- **One face per family:** Staged file names are derived from the family name (lowercased,
  `[a-z0-9_]`), so a second face of the same family would collide. Ship the regular face; bold
  and italic are synthesized where the platform can.
- **File names don't matter; family names do.** `resource/fonts/SpecialElite-Regular.ttf` whose embedded
  family is "Special Elite" is used as `Font::Custom("Special Elite", 20.0)`.

Beyond the rules: an unknown family never breaks the app. The label
renders in the system font and the log names the family that didn't resolve. And `.weight(...)` /
`.italic()` still apply, but a single-face family only gets what the platform can synthesize (a
heavier stroke, a slant), not true bold or italic cuts.

## The app icon: `resource/icons/`

`resource/icons/` holds one master (`icon.svg`, `day-icon.svg`, or `icon.png`); `day icon`
renders it into every platform's icon set — the macOS `.icns`, a multi-size Windows `.ico`,
Android's adaptive and themed icons, the HarmonyOS layered icon, an Xcode Icon Composer
package — writing the export tree under `resource/icons/` and the `platform/` copies each build
consumes. `icons.lock.json` records what was generated, and `day icon --check` fails CI when
the outputs drift from the master.

During development, `day launch` wires the icon into the running window; at packaging time,
[`day pack`](/docs/packaging) bundles the generated artifacts (the `.icns` inside your macOS
bundle, hicolor icons inside the flatpak, MSIX logo assets). The
[icons reference](/docs/internal/icons) covers layered SVG masters (separate background,
foreground, and monochrome layers) and the full output table.

## Localized strings are resources too

`resource/locales/<lang>/app.ftl` files are compiled in via `include_str!` at the moment
([localization guide](/docs/localization)), and OS-facing strings (the app's display name) are
conveyed into platform manifests at build time. Piece packages can carry their own `locales/` and
resources, which aggregate into your app without name collisions.

## What happens at build

```text
resource/images/wave@2x.png ─┐  resource/fonts/Pacifico-Regular.ttf ─┐  resource/assets/stations.json ─┐
                     ▼          day build -p <target>   ▼                            ▼
   ┌───────────────────────────────────────────────────────────────────────┐
   │ android  → res/drawable-xhdpi/wave.png · res/font/pacifico.ttf        │
   │ ios      → DayPieces asset catalog + fonts/ bundle dir + UIAppFonts   │
   │ gtk/qt   → app.gresource / app.rcc; fonts registered at startup       │
   │ arkui    → hap rawfile/ (+ day/fonts.json → registerFont)             │
   │ desktop dev-launch → read from project dirs directly                  │
   └───────────────────────────────────────────────────────────────────────┘
```

Staging is best-effort in development: if a resource compiler is missing (say `rcc` on an
unusual Qt install), the build warns and the app falls back to loading loose files from the
project directory instead of failing. Packaged builds via `day pack` bundle everything properly.
