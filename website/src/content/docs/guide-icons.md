---
navTitle: "Icons"
title: App icons and interface icons
description: "App-icon generation, interface SVGs, and platform rendering limits."
order: 33
section: How-to guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

An app has two kinds of icons: the app icon shown by the operating system, and the icons used
inside its interface. Day generates platform-specific app icons from a source image and makes
interface SVGs available as typed Rust resources. This guide covers both, from source files to
build outputs.

<span id="1-put-one-master-in-resourceicons"></span>

## App icon source

The scaffold includes `resource/icons/icon.svg`. Replacing that file sets the app icon.
`day icon build` can also take an explicit source path; otherwise it searches for `icon.svg`,
`day-icon.svg`, then `icon.png` in `resource/icons/`.

An SVG master can mark top-level elements as semantic layers by id:

```xml
<rect id="day:background" …/>    <!-- the full-bleed backdrop -->
<g id="day:foreground">…</g>     <!-- the motif; day:foreground-2 … for more layers -->
<g id="day:monochrome">…</g>     <!-- themed/tinted appearances -->
```

The combined image supplies full-bleed outputs; separate layers supply adaptive and layered
formats. An unlayered SVG or PNG supplies the legacy formats and becomes the adaptive foreground
over a derived background color. Text must be converted to outlines; see
[rendering limits](#rendering-limits).

For temporary artwork, `day icon new` creates a layered icon. `--seed <int|string>` repeats
a design, and `--out preview.svg` saves a preview without requiring a project.

<span id="2-run-day-prepare"></span>

## Generated platform assets

`day prepare` writes platform assets to `build/day/host/`:

- **macOS** — PNG icons with the required shape and margins, `day-icon.icns`, and the asset catalog the
  Xcode project compiles.
- **iOS** — the asset catalog with an opaque 1024 px image, plus an Icon Composer package
  (`AppIcon.icon/`) for Xcode 26's Liquid Glass icons.
- **Android** — the launcher resource tree: adaptive `ic_launcher_{foreground,background}.png`,
  the legacy icon, `mipmap-anydpi-v26/ic_launcher.xml`, and, from a layered master, the
  Android 13 themed icon; beside it `play-store-512.png` for the listing.
- **HarmonyOS** — `startIcon.png` and the layered icon (`layered_image.json` with foreground
  and background), linked into both module resource roots and referenced by
  `app.json5`/`module.json5`.
- **Windows** — a multi-size `day.ico` (16/32/48/256) and `day-icon-256.png`.
- **Linux** — PNGs at the sizes appstream tooling accepts (48/128/256/512).
- **`png/`** — `day-icon-{16…1024}.png` for favicons and general use.

The files in `build/day/host/` are generated and should not be committed. Native projects reference
them, and build, launch, and packaging commands regenerate them when the source icon or Day
version changes. `day prepare -t <target>` limits generation to one platform family.

Platform-specific sources override the shared icon: `resource/icons/macos.svg` supplies the macOS
icon, and `resource/icons/ios/AppIcon.icon/` supplies an Icon Composer bundle for iOS.

<span id="3-open-the-native-project-and-gate-ci"></span>

## Native projects and CI

`day open -t ios-uikit` prepares assets before opening Xcode; `day open -t android-mdc` does the
same for Android Studio. The corresponding VS Code commands also prepare assets first.

CI can check whether generated files are current without writing them:

```sh
day prepare --check
```

The command exits 0 when outputs match and 5 when files are missing or stale.
`build/day/host/host.lock.json` records the generator version and source and output digests.
A version mismatch requests regeneration rather than comparing outputs from different generators.

Keep master artwork under `resource/icons/` and remove obsolete generated copies and their
references from customized native projects before running `day prepare`. Generated native
projects use the current paths automatically. See [native project migration](/contributing/backend-contracts/native-project-migration).

<span id="4-draw-in-app-glyphs-from-resourcevectors"></span>

## Interface icons

Interface icons belong in `resource/vectors/`. Supported sources are SVG files, SF Symbols
template exports, and Xcode `.symbolset` bundles. Template sources can include Light and Bold
variants. The build generates a `res::vectors::` constant for each resource:

```rust
use day::prelude::*;

vector(res::vectors::home)
    .tint(Color::rgba(0.18, 0.50, 0.94, 1.0))
    .frame(24.0, 24.0)
```

Vector modifiers control appearance: [`.tint(color)`](https://daybrite.dev/api/day_pieces/trait.VectorBuilder.html#tymethod.tint) recolors a monochrome glyph
where the backend can, [`.weight(VectorWeight::Light | Bold)`](https://daybrite.dev/api/day_pieces/trait.VectorBuilder.html#tymethod.weight) selects a weight variant, and
[`.decorative()`](https://daybrite.dev/api/day_pieces/trait.VectorBuilder.html#tymethod.decorative) hides the glyph from accessibility. Navigation items, tabs,
[`toolbar_button`](https://daybrite.dev/api/day_pieces/fn.toolbar_button.html)`(…)`[`.image`](https://daybrite.dev/api/day_pieces/struct.ToolbarEntry.html#method.image)`(…)`, and `bar_action` also accept `res::vectors::` constants as image names.

<span id="pitfalls"></span>

## Rendering limits

Interface icons are supported on every backend. Android uses VectorDrawable where possible;
Apple targets, HarmonyOS, and web render SVG. GTK, Qt, and WinUI use a 256 px raster generated
at build time.

- SVG text must be converted to outlines. Both app icons and interface icons reject `<text>` elements.
- Android VectorDrawable supports solid fills and strokes; art with
  gradients, clips, masks, or filters falls back to the 256 px raster, and `day lint` flags it
  as `day::lint::vector-raster-fallback` when `android-mdc` is a declared target. `day lint`
  also catches unreadable art, empty `.symbolset` bundles, and glyph-embedded text.
- Tint has coverage limits. AppKit, UIKit, Android, GTK, and ArkUI recolor; Qt, WinUI, and
  web draw the authored colors. Use authored colors when the color must match across backends.
- Weights need template sources. A plain SVG aliases Light and Bold to the same glyph, so
  [`.weight(…)`](https://daybrite.dev/api/day_pieces/trait.VectorBuilder.html#tymethod.weight) degrades to Regular rather than to a missing asset. True weight variants come
  from SF template exports and `.symbolset` bundles.

## Reference

[icons](/docs/how-to/icons) — master layering, the generator, the full output table, and the
lock file. [vectors](/docs/reference/vectors) — source forms, the per-backend staging table,
weights, and tint coverage.
