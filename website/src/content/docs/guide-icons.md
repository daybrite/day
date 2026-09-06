---
title: One icon, every platform
description: "Render every platform's app-icon set from one master with `day icon`, and draw in-app glyphs from `resource/vectors/` SVGs as typed, tintable vector pieces."
order: 33
section: Guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Icons come up twice in an app: the launcher icon the OS shows, and the glyphs your own UI
draws. Day covers both from files in `resource/`. One master image becomes every platform's
app-icon set with one command, and every SVG in `resource/vectors/` becomes a typed constant
you draw with one piece:

```sh
day prepare # resource/icons/icon.svg → .icns, .ico, adaptive + themed, layered, catalogs, under build/day/host/
```

```rust
vector(res::vectors::home).tint(color).frame(24.0, 24.0)
```

**Works on:** every backend. The app-icon families cover macOS, iOS, Android, HarmonyOS,
Windows, and Linux, plus a plain PNG set for favicons and catalogs. In-app vectors draw
everywhere, but what ships differs per backend: Android gets a compiled VectorDrawable, the
Apple targets, web, and HarmonyOS render the SVG itself, and GTK, Qt, and WinUI draw a
build-time 256 px raster of the same glyph. Details in [the vectors
reference](/docs/internal/vectors).

## 1. Put one master in `resource/icons/`

`day icon` takes an explicit path argument, or finds the master at `resource/icons/icon.svg`,
then `resource/icons/day-icon.svg`, then `resource/icons/icon.png`. The scaffold from `day new
app` already ships an `icon.svg` (a generated placeholder seeded by your app id), so replacing
that one file completes the setup.

An SVG master can mark top-level elements as semantic layers by id:

```xml
<rect id="day:background" …/>    <!-- the full-bleed backdrop -->
<g id="day:foreground">…</g>     <!-- the motif; day:foreground-2 … for more layers -->
<g id="day:monochrome">…</g>     <!-- themed/tinted appearances -->
```

The composite feeds every full-bleed output; the split layers feed Android's adaptive icon and
the other layered formats below. An unlayered SVG or a PNG master still produces the complete
legacy set; the whole art becomes the adaptive foreground over a derived background color.
Text in the master must be outlined first; `<text>` is a hard error that names the fix.

Before you have art, `day icon --generate` writes a seeded pseudo-random layered master and
renders everything from it. `--seed <int|string>` reproduces a specific icon (the seed used is
always printed), and `--out preview.svg` writes a preview to a path of your choosing, which
works without a project.

## 2. Run `day prepare`

One run renders, per platform family, into `build/day/host/`:

- **macOS** — a margin-composed squircle PNG set, `day-icon.icns`, and the asset catalog the
  Xcode project compiles.
- **iOS** — the asset catalog with an opaque 1024 px image, plus an Icon Composer package
  (`AppIcon.icon/`) for Xcode 26's Liquid Glass icons.
- **Android** — the launcher resource tree: adaptive `ic_launcher_{foreground,background}.png`,
  the legacy icon, `mipmap-anydpi-v26/ic_launcher.xml`, and, from a layered master, the
  Android 13 themed icon; beside it `play-store-512.png` for the listing.
- **HarmonyOS** — `startIcon.png` and the layered icon (`layered_image.json` with foreground
  and background), linked into both module resource roots and wired into
  `app.json5`/`module.json5`.
- **Windows** — a multi-size `day.ico` (16/32/48/256) and `day-icon-256.png`.
- **Linux** — PNGs at the sizes appstream tooling accepts (48/128/256/512).
- **`png/`** — `day-icon-{16…1024}.png` for favicons and general use.

Nothing under `build/day/host/` is checked in. The Xcode and Gradle projects reference it by
path, `day pack` reads it, and every `day build`, `launch`, and `pack` runs `prepare` first
when the master or the day version changed, so the icon that ships is always the master's
current render. `-p <target>` limits a run to one target's family.

To change one platform's icon, add a source rather than editing an output:
`resource/icons/macos.svg` replaces the master for macOS alone, and a checked-in
`resource/icons/ios/AppIcon.icon/` bundle (tuned in Icon Composer) is copied through as-is.

## 3. Open the native project, and gate CI

`day open -p ios-uikit` prepares and opens `platform/ios/DayApp.xcodeproj`; `day open -p
android-mdc` does the same for Android Studio. The VS Code extension's "Open in Xcode" and
"Open in Android Studio" run `day prepare` first too. On a fresh clone, that is the one step
between checkout and the IDE.

`day prepare --check` renders everything in memory, compares bytes against `build/day/host`,
and writes nothing. When the outputs are present and current it exits 0; otherwise it lists
what is missing or stale and exits 5, the same gate pattern the duty-matrix check uses:

```sh
day prepare --check    # exit 5 = build/day/host is missing or older than the master
```

`build/day/host/host.lock.json` records the generator version and a digest per source and per
output. Renders are byte-stable within one generator version; a `--check` under a different
day version reports "run `day prepare`" instead of false byte drift.

An app that still commits its derived icons moves over with one command:

```sh
day prepare --migrate    # deletes the generated files, repoints the projects, gitignores the links
```

## 4. Draw in-app glyphs from `resource/vectors/`

Drop SVGs into `resource/vectors/`. The build accepts a plain `.svg` (raw Material Symbols
downloads work as-is), an SF Symbols template export, and an Xcode `.symbolset` bundle; the
template forms also carry separate Light and Bold weight art. The build generates a
`res::vectors::` constant per file, so a typo is a compile error and presence is guaranteed:

```rust
use day::prelude::*;

vector(res::vectors::home)
    .tint(Color::rgba(0.18, 0.50, 0.94, 1.0))
    .frame(24.0, 24.0)
```

The modifiers are the vector-appropriate ones: `.tint(color)` recolors a monochrome glyph
where the backend can, `.weight(VectorWeight::Light | Bold)` selects a weight variant, and
`.decorative()` hides the glyph from accessibility. Vector names also flow through the
name-based image channels unchanged: nav-item icons, tab icons, `toolbar_button(…).image(…)`,
and `bar_action` all accept a `res::vectors::` constant where they accept an image name.

## Pitfalls

- **Outline your text.** Day compiles no text shaper into either pipeline, so `<text>` in an
  icon master or a vector glyph is a hard build error in both. Convert text to
  outlines in your editor before exporting.
- **Android ships a subset.** VectorDrawable covers solid fills and strokes; art with
  gradients, clips, masks, or filters falls back to the 256 px raster, and `day lint` flags it
  as `day::lint::vector-raster-fallback` when `android-mdc` is a declared target. `day lint`
  also catches unreadable art, empty `.symbolset` bundles, and glyph-embedded text.
- **Tint has coverage limits.** AppKit, UIKit, Android, GTK, and ArkUI recolor; Qt, WinUI, and
  web draw the authored colors. Author glyphs in a single color if a tint must read the same
  everywhere.
- **Weights need template sources.** A plain SVG aliases Light and Bold to the same glyph, so
  `.weight(…)` degrades to Regular rather than to a missing asset. True weight variants come
  from SF template exports and `.symbolset` bundles.
- **Regenerate after editing the master.** The committed `platform/` copies only change when
  `day icon` runs; `day icon --check` in CI (exit 5) catches the forgotten run.

## Reference

[icons](/docs/internal/icons) — master layering, the generator, the full output table, and the
lock file. [vectors](/docs/internal/vectors) — source forms, the per-backend staging table,
weights, and tint coverage.
