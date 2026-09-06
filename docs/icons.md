---
title: "App icons"
description: "day icon generates every platform's icon family from one master SVG, including the seeded generator for new apps."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# App icons (`day icon`)

`day icon` renders every platform's icon set from one master and keeps the copies in sync. It
renders the master into `build/day/host/`, the derived tree every host project references and no repository
build consumes; `day icon --check` verifies nothing drifted (exit 5; it is a CI gate beside the
duty-matrix check).

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
Android's adaptive icon (foreground tightened to its content box and centered in the 66/108 dp
safe zone; background full-bleed). An **unlayered** SVG or a **PNG** master still produces the
complete legacy set; the adaptive foreground is then the whole art in the safe zone over a
derived background color (the composite's corner pixel; white when transparent).

Text must be outlined: text shaping is not compiled into day, which keeps resvg's font stack
out of the build (`<text>` is a hard error naming the fix).

A reserved layer (`day:monochrome`, `day:dark`) may carry `display="none"` so plain SVG
viewers show the master as it ships; the layer-only documents re-enable it. Generated
masters do this.

## Generate

`day icon --generate` writes a seeded pseudo-random layered master (background gradient +
foreground motif + hidden monochrome silhouette) to `resource/icons/icon.svg` and renders
every output from it. It refuses to replace an existing master unless `--overwrite`.

* `--seed <int|string>` — reproduce a specific icon (a non-integer seed is hashed; the app-id
  convention below). Without it a fresh random seed is drawn and printed, so a liked icon can be
  regenerated.
* `--out <file.svg>` — preview mode: write the master (plus a 512 px PNG beside it) to a path
  instead of the project, touching nothing else. It works outside any project, so use it to
  browse seeds.

`day new app` uses the same generator for every fresh scaffold, seeded by the **app id**
(scaffolding the same id twice yields the same icon); `--icon-seed` overrides the seed.

The generator composes from a limited palette drawn from the classic color-harmony schemes
(analogous / complementary / split-complementary / triadic), with figure-ground contrast held by
construction, one or two focal points in simple geometry within the masks' safe zone, a subtle
vertical background gradient, and symmetric / rotational / golden-section-balanced arrangements
(`day-vector/src/icongen.rs` documents the sources).
Generated monochrome layers stay inside the VectorDrawable subset, so Android's themed icon
ships as a true vector.

## Outputs

Everything derived lives under `build/day/host/<family>/`, written by `day prepare` and never
checked in. The host projects reach it by path: the Xcode projects reference
`../../build/day/host/{ios,macos}/Assets.xcassets`, the Gradle module adds
`build/day/host/android/res` to its `res` source set, `day pack` reads the Linux and Windows
icons there, and hvigor — whose resource roots are fixed — gets gitignored symlinks from both
`resources/base/media` directories to `build/day/host/harmony/media`.

| Family | Files under `build/day/host/` |
|---|---|
| `png/` | `day-icon-{16,32,64,128,256,512,1024}.png` — favicons, catalogs, general use |
| `macos/` | margin-composed squircle set (824 pt art on 1024, radius 184) `-{16,32,128,256,512,1024}.png`, `day-icon.icns`, and `Assets.xcassets/` (the catalog the macOS Xcode project compiles) |
| `ios/` | `Assets.xcassets/` with the opaque 1024 px universal image (App Store validation rejects alpha), plus `AppIcon.icon/` (Icon Composer) |
| `android/` | `res/mipmap-xxxhdpi/ic_launcher{,_foreground,_background}.png`, `res/mipmap-anydpi-v26/ic_launcher.xml`, the themed-icon drawable; beside them `ic_launcher-legacy-192.png` and `play-store-512.png` for store listings |
| `harmony/` | `media/{startIcon.png, foreground.png, background.png, layered_image.json}` — the one set both module roots link to |
| `linux/` | `day-icon-{48,128,256,512}.png` (appstream-compose-safe sizes) |
| `windows/` | multi-size `day.ico` (16/32/48/256, PNG-compressed) + `day-icon-256.png` |

`-p <target>` limits a run to that target's family. Everything renders in memory first, so
`--check` compares bytes without touching the tree. Unchanged outputs are not rewritten, so
actool and aapt2 see no new mtimes.

## Overrides

A file in git is a source; a derived file is under `build/`. So customizing one platform's icon
means adding a source, never editing an output:

* `resource/icons/<family>.svg` (`ios.svg`, `macos.svg`, `android.svg`, `linux.svg`,
  `windows.svg`, `harmony.svg`, `png.svg`) replaces the master for that family alone.
* A checked-in `resource/icons/ios/AppIcon.icon/` bundle, tuned in Icon Composer, is copied
  through as-is instead of generated.

Both are digested into the lock, so editing one invalidates it like editing the master does.

## Modern formats

Beyond the legacy set, a **layered SVG master** also produces:

* **Android themed icon** (Android 13): a monochrome drawable the system tints
  (`day:monochrome` as a VectorDrawable when it fits the subset, else the adaptive foreground's
  alpha as a bitmap mask), wired into the generated `mipmap-anydpi-v26/ic_launcher.xml`.
* **HarmonyOS layered icon**: `layered_image.json` + `foreground.png`/`background.png` (216 px)
  in the linked media set, with `app.json5`/`module.json5` icon slots rewired to
  `$media:layered_image` (`startWindowIcon` keeps the flat `startIcon.png`). That rewrite of a
  source file is the one edit `prepare` makes under `platform/`, and it is idempotent.
* **Icon Composer package** (Xcode 26 Liquid Glass): `AppIcon.icon/` — `icon.json` + SVG layer
  assets split from the master's `day:` layers (`day:monochrome` ships as an asset for the
  Tinted appearance). Open it in Icon Composer to tune materials; the tuned bundle goes under
  `resource/icons/ios/` as an override.

## The lock and the build

`build/day/host/host.lock.json` records the generator (day-cli + engine version), a digest per
source (the master and every override), and a digest per output. Every `day build`, `launch`,
and `pack` calls `ensure` first: a no-op while the lock vouches for the sources and the
generator and every listed output exists, a full `prepare` otherwise. The Xcode target's
"Build Rust (day)" phase does the same, so a build started from the Xcode GUI never compiles a
stale or missing catalog. `day prepare --check` is the CI gate and what the VS Code extension
asks before "Open in Xcode" or "Open in Android Studio"; `day open -p <target>` prepares and
opens the project in one step.

## Migrating an app

`day prepare --migrate` moves a project that committed its derived files to this layout. It
deletes each legacy output the old `resource/icons/icons.lock.json` proves was generated (a
hand-edited file stays, and is named, since it has become an override or a mistake), repoints
the Xcode projects' catalog references and the Gradle source set, gitignores the HarmonyOS
links, and runs `prepare`. It touches nothing in git; review `git status` and commit the
deletions.

## Engine

[day-vector](../crates/day-vector) is resvg/usvg/tiny-skia with text shaping off, plus hand-rolled
`.ico`/`.icns` writers. The same crate powers `resource/vectors/` staging ([docs/vectors.md](vectors.md)).
