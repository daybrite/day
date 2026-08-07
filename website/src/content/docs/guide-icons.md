---
title: One icon, every platform
description: "Render every platform's app-icon set from one master with `day icon`, and draw in-app glyphs from `resource/vectors/` SVGs as typed, tintable vector pieces."
order: 33
section: Guides
---

Icons come up twice in an app: the launcher icon the OS shows, and the glyphs your own UI
draws. Day covers both from files in `resource/`. One master image becomes every platform's
app-icon set with one command, and every SVG in `resource/vectors/` becomes a typed constant
you draw with one piece:

```sh
day icon    # resource/icons/icon.svg → .icns, .ico, adaptive + themed, layered, appiconset …
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
app` already ships an `icon.svg` — a generated placeholder seeded by your app id — so replacing
that one file is the whole setup.

An SVG master can mark top-level elements as semantic layers by id:

```xml
<rect id="day:background" …/>    <!-- the full-bleed backdrop -->
<g id="day:foreground">…</g>     <!-- the motif; day:foreground-2 … for more layers -->
<g id="day:monochrome">…</g>     <!-- themed/tinted appearances -->
```

The composite feeds every full-bleed output; the split layers feed Android's adaptive icon and
the other layered formats below. An unlayered SVG or a PNG master still produces the complete
legacy set — the whole art becomes the adaptive foreground over a derived background color.
Text in the master must be outlined first; `<text>` is a hard error that names the fix.

No art yet? `day icon --generate` writes a seeded pseudo-random layered master and renders
everything from it. `--seed <int|string>` reproduces a specific icon (the seed used is always
printed), and `--out preview.svg` writes a preview outside the project, no project required.

## 2. Run `day icon`

One run renders, per platform family:

- **macOS** — a margin-composed squircle PNG set and `day-icon.icns`.
- **iOS** — an opaque `AppIcon-1024.png`, synced into the committed `AppIcon.appiconset`, plus
  an Icon Composer package (`AppIcon.icon/`) for Xcode 26's Liquid Glass icons; the appiconset
  stays as the pre-26 fallback.
- **Android** — adaptive `ic_launcher_{foreground,background}.png`, the legacy 192 px icon,
  and `play-store-512.png`. A layered master also produces the Android 13 themed icon: a
  monochrome drawable the system tints, wired into `mipmap-anydpi-v26/ic_launcher.xml`.
- **HarmonyOS** — `startIcon.png` in both media dirs, plus a layered icon
  (`layered_image.json` with foreground and background) wired into `app.json5`/`module.json5`.
- **Windows** — a multi-size `day.ico` (16/32/48/256) and `day-icon-256.png`.
- **Linux** — PNGs at the sizes appstream tooling accepts (48/128/256/512).
- **`png/`** — `day-icon-{16…1024}.png` for favicons and general use.

The command writes both the `resource/icons/` export tree and the committed `platform/` copies
each build consumes, so the icon in version control is the icon that ships. `-p <target>`
limits a run to one target's family.

## 3. Gate drift in CI

`day icon --check` renders everything in memory, compares bytes against the tree, and writes
nothing. When the outputs match it exits 0; when they don't it lists the drifted files and
exits 5 — the same gate pattern the duty-matrix check uses:

```sh
day icon --check    # exit 5 = someone edited the master and forgot to run `day icon`
```

`resource/icons/icons.lock.json` records the generator version, the master's digest, and a
digest per output. Renders are byte-stable within one generator version; a `--check` under a
different day version reports "regenerate with this day version" instead of false byte drift.

## 4. Draw in-app glyphs from `resource/vectors/`

Drop SVGs into `resource/vectors/`. Three source forms work: a plain `.svg` (raw Material
Symbols downloads work as-is), an SF Symbols template export, and an Xcode `.symbolset` bundle
— the template forms also carry true Light and Bold weight art. The build generates a
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
name-based image channels unchanged — nav-item icons, tab icons, `toolbar_button(…).image(…)`,
and `bar_action` all accept a `res::vectors::` constant where they accept an image name.

## Pitfalls

- **Outline your text.** Text shaping is deliberately not compiled into day, so `<text>` in an
  icon master or a vector glyph is a hard build error in both pipelines. Convert text to
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
