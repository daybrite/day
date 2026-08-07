# App icons (`day icon`)

One master, every platform's icon set, kept in sync. `day icon` renders the master into the
`resource/icons/` export tree and the committed `platform/` copies each build consumes; `day icon
--check` verifies nothing drifted (exit 5 — a CI gate beside the duty-matrix check).

## The master

Discovery order: an explicit path argument, then `resource/icons/icon.svg`,
`resource/icons/day-icon.svg`, `resource/icons/icon.png`.

An SVG master may mark **top-level** elements as semantic layers by id:

```xml
<rect id="day:background" …/>          <!-- the full-bleed backdrop -->
<g id="day:foreground">…</g>           <!-- the motif; day:foreground-2 … for more layers -->
<g id="day:monochrome">…</g>           <!-- reserved: themed/tinted modes (not yet consumed) -->
<g id="day:dark">…</g>                 <!-- reserved: dark-mode variants (not yet consumed) -->
```

The composite (background + foregrounds) feeds every full-bleed output; the split layers feed
Android's adaptive icon (foreground tightened to its content box and centred in the 66/108 dp
safe zone; background full-bleed). An **unlayered** SVG or a **PNG** master still produces the
complete legacy set — the adaptive foreground is then the whole art in the safe zone over a
derived background colour (the composite's corner pixel; white when transparent).

Text must be outlined: text shaping is deliberately not compiled into day (`<text>` is a hard
error naming the fix).

## Outputs

| Family | Files |
|---|---|
| `png/` | `day-icon-{16,32,64,128,256,512,1024}.png` — favicons, catalogs, general use |
| `macos/` | margin-composed squircle set (824 pt art on 1024, radius 184) `-{16,32,128,256,512,1024}.png` + `day-icon.icns` |
| `ios/` | `AppIcon-1024.png` (opaque — App Store validation) + sync into `platform/ios/…/AppIcon.appiconset/` |
| `android/` | adaptive `ic_launcher_{foreground,background}.png` (432), legacy 192, `play-store-512.png` + sync into `platform/android/…/mipmap-xxxhdpi/` |
| `linux/` | `day-icon-{48,128,256,512}.png` (appstream-compose-safe sizes) |
| `windows/` | multi-size `day.ico` (16/32/48/256, PNG-compressed) + `day-icon-256.png` |
| OHOS | `startIcon.png` sync into both `platform/ohos/{entry,AppScope}` media dirs |

`-p <target>` limits generation to that target's family. Everything renders in memory first, so
`--check` compares bytes without touching the tree.

## Modern formats

Beyond the legacy set, a **layered SVG master** also produces:

* **Android themed icon** (Android 13): a monochrome drawable the system tints —
  `day:monochrome` as a VectorDrawable when it fits the subset, else the adaptive foreground's
  alpha as a bitmap mask — plus an idempotent `<monochrome>` entry added to the committed
  `mipmap-anydpi-v26/ic_launcher.xml`.
* **HarmonyOS layered icon**: `layered_image.json` + `foreground.png`/`background.png` (216 px)
  in both media dirs, with `app.json5`/`module.json5` icon slots rewired to
  `$media:layered_image` (`startWindowIcon` keeps the flat `startIcon.png`).
* **Icon Composer package** (Xcode 26 Liquid Glass): `AppIcon.icon/` — `icon.json` + SVG layer
  assets split from the master's `day:` layers (`day:monochrome` ships as an asset for the
  Tinted appearance) — staged into `resource/icons/ios/` and `platform/ios/`. Open it in Icon
  Composer to tune materials, and point Xcode 26's app-icon build setting at it; the appiconset
  remains the pre-26 fallback.

## The lock

`resource/icons/icons.lock.json` records the generator (day-cli + engine version), the master's
digest, and a digest per output. Renders are byte-stable **within one generator version**;
`--check` under a different version reports "regenerate with this day version" instead of false
byte drift.

## Engine

[day-vector](../crates/day-vector) — resvg/usvg/tiny-skia with text shaping off, plus hand-rolled
`.ico`/`.icns` writers. The same crate powers `resource/vectors/` staging (docs/vectors.md).
