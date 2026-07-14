---
title: Resources, images, fonts & icons
description: "How resource/assets, resource/images, resource/fonts, and resource/icons travel from your project into each platform's native resource system — and how to read them back."
order: 24
section: Guides
---

A Day project keeps its resources under one conventional `resource/` directory, with four
subdirectories, each with a different destiny:

```text
myapp/
  resource/
    assets/    # data files: JSON, databases — anything you open as bytes
    images/    # UI images, with @2x/@3x density variants
    fonts/     # custom fonts (.ttf/.otf), referenced by family name
    icons/     # the app icon (and its per-platform renditions)
```

The principle behind all four: **resources ride each platform's native resource system**, not a
custom archive format. On Android your images become real `res/drawable-*` entries crunched by
aapt2; on iOS they join an asset catalog; on GTK they compile into a GResource bundle; on Qt, a
Qt resource file. `day build` does the staging automatically, per target, before the platform
build runs.

## Data files: `resource/assets/`

Anything in `resource/assets/` is packaged and readable at runtime through one call:

```rust
let bytes: day::Resource = day::resource("stations.json").expect("packaged asset");
let parsed: Stations = serde_json::from_slice(bytes.as_slice())?;
```

`Resource` is a zero-copy view: on Android it borrows straight from the `AAssetManager` buffer,
on GTK from the GResource, on desktop from an mmap — no copy into a Vec unless you make one.
`read_at(offset, buf)` gives random access for large files. During development the same call
resolves against your project directory, so editing an asset and relaunching picks it up without
a packaging step.

## Images: `resource/images/`

Drop PNGs (with optional `@2x`/`@3x` density variants) into `resource/images/` and reference them by
name:

```rust
image("wave")          // finds wave.png / wave@2x.png / wave@3x.png
    .frame(240.0, 120.0)
```

At build time each toolkit gets the format it expects — density buckets on Android
(`drawable-xhdpi/…`), an asset catalog imageset on iOS, resource bundles on GTK/Qt — and the
platform picks the right density at runtime the same way it does for any native app. The
[resources reference](/docs/internal/resources) documents the exact per-platform staging.

Two notes worth knowing:

- **SVG is not a runtime format.** Android and Qt widgets can't render SVG at runtime, so
  runtime images are raster. Keep sources vector, export raster densities into `resource/images/`.
- **Remote images** (URL-loaded, cached) are a separate piece —
  [`day-piece-remote-image`](/docs/internal/resources) — because they involve networking and
  cache policy the core deliberately doesn't own.

## Custom fonts: `resource/fonts/`

Drop `.ttf` or `.otf` files into `resource/fonts/` and reference them **by family name** — the name baked
into the font file itself (what Font Book or fontconfig report), not the file name:

```rust
label("Welcome aboard").font(Font::Custom("Pacifico", 24.0))
```

`day build` stages each font where the platform wants it — `res/font/` on Android (with the
resource-naming rules handled for you), the app bundle plus a `UIAppFonts` Info.plist entry on
iOS, a fonts directory registered with CoreText / fontconfig / the `QFontDatabase` on the
desktops, rawfile plus an ArkTS `registerFont` manifest on HarmonyOS — and each backend registers
everything at startup. The point size scales with the platform's accessibility text size, exactly
like `Font::System(pt)`.

The restrictions, all enforced as **hard errors at build time** (each would otherwise surface as
a confusing runtime-only failure on one platform):

- **`.ttf` and `.otf` only.** Android's `res/font/` accepts nothing else, so Day holds every
  platform to the same rule. Convert collections (`.ttc`) and variable fonts to single static
  faces before bundling.
- **One face per family.** Staged file names are derived from the family name (lowercased,
  `[a-z0-9_]`), so a second face of the same family would collide. Ship the regular face; bold
  and italic are synthesized where the platform can.
- **File names don't matter; family names do.** `resource/fonts/SpecialElite-Regular.ttf` whose embedded
  family is "Special Elite" is used as `Font::Custom("Special Elite", 20.0)`.

Two things worth knowing beyond the rules: an unknown family never breaks the app — the label
renders in the system font and the log names the family that didn't resolve. And `.weight(...)` /
`.italic()` still apply, but a single-face family only gets what the platform can synthesize (a
heavier stroke, a slant), not true bold or italic cuts.

## The app icon: `resource/icons/`

`resource/icons/` holds the app icon renditions each platform wants (`resource/icons/macos/`,
`resource/icons/windows/*.ico`, `resource/icons/linux/*.png`, plus mobile catalogs in the
platform scaffolds). During development,
`day launch` wires the icon into the running window; at packaging time,
[`day pack`](/docs/packaging) builds the platform-specific artifacts — the `.icns` inside your
macOS bundle, hicolor icons inside the flatpak, MSIX logo assets — from these files.

Keeping a single SVG source in `resource/icons/` and exporting the renditions is the current practice; a
generate-the-whole-matrix-from-one-SVG pipeline is designed but you still export by hand today.

## Localized strings are resources too

`resource/locales/<lang>/app.ftl` files are compiled in via `include_str!` at the moment
([localization guide](/docs/localization)), and OS-facing strings (the app's display name) are
conveyed into platform manifests at build time. Piece packages can carry their own `locales/` and
resources, which aggregate into your app without name collisions.

## What happens at build, concretely

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

Staging is best-effort in development — if a resource compiler is missing (say `rcc` on an
unusual Qt install), the build warns and the app falls back to loading loose files from the
project directory instead of failing. Packaged builds via `day pack` bundle everything properly.
