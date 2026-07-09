---
title: Resources, images & icons
description: "How assets/, images/, and icons/ travel from your project into each platform's native resource system — and how to read them back."
order: 24
section: Guides
---

A Day project has three conventional resource directories, each with a different destiny:

```text
myapp/
  assets/      # data files: JSON, fonts, databases — anything you open as bytes
  images/      # UI images, with @2x/@3x density variants
  icons/       # the app icon (and its per-platform renditions)
```

The principle behind all three: **resources ride each platform's native resource system**, not a
custom archive format. On Android your images become real `res/drawable-*` entries crunched by
aapt2; on iOS they join an asset catalog; on GTK they compile into a GResource bundle; on Qt, a
Qt resource file. `day build` does the staging automatically, per target, before the platform
build runs.

## Data files: `assets/`

Anything in `assets/` is packaged and readable at runtime through one call:

```rust
let bytes: day::Resource = day::resource("stations.json").expect("packaged asset");
let parsed: Stations = serde_json::from_slice(bytes.as_slice())?;
```

`Resource` is a zero-copy view: on Android it borrows straight from the `AAssetManager` buffer,
on GTK from the GResource, on desktop from an mmap — no copy into a Vec unless you make one.
`read_at(offset, buf)` gives random access for large files. During development the same call
resolves against your project directory, so editing an asset and relaunching picks it up without
a packaging step.

## Images: `images/`

Drop PNGs (with optional `@2x`/`@3x` density variants) into `images/` and reference them by
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
  runtime images are raster. Keep sources vector, export raster densities into `images/`.
- **Remote images** (URL-loaded, cached) are a separate piece —
  [`day-piece-remote-image`](/docs/internal/resources) — because they involve networking and
  cache policy the core deliberately doesn't own.

## The app icon: `icons/`

`icons/` holds the app icon renditions each platform wants (`icons/macos/`, `icons/windows/*.ico`,
`icons/linux/*.png`, plus mobile catalogs in the platform scaffolds). During development,
`day launch` wires the icon into the running window; at packaging time,
[`day pack`](/docs/packaging) builds the platform-specific artifacts — the `.icns` inside your
macOS bundle, hicolor icons inside the flatpak, MSIX logo assets — from these files.

Keeping a single SVG source in `icons/` and exporting the renditions is the current practice; a
generate-the-whole-matrix-from-one-SVG pipeline is designed but you still export by hand today.

## Localized strings are resources too

`locales/<lang>/app.ftl` files are compiled in via `include_str!` at the moment
([localization guide](/docs/localization)), and OS-facing strings (the app's display name) are
conveyed into platform manifests at build time. Piece packages can carry their own `locales/` and
resources, which aggregate into your app without name collisions.

## What happens at build, concretely

```text
images/wave@2x.png ──┐                       assets/stations.json ──┐
                     ▼ day build -p <target> ▼
   ┌──────────────────────────────────────────────────────────┐
   │ android  → res/drawable-xhdpi/wave.png   + assets/        │
   │ ios      → DayPieces asset catalog (actool → Assets.car)  │
   │ gtk      → app.gresource (images + data)                  │
   │ qt       → app.rcc                                        │
   │ arkui    → hap rawfile/                                   │
   │ desktop dev-launch → read from project dirs directly      │
   └──────────────────────────────────────────────────────────┘
```

Staging is best-effort in development — if a resource compiler is missing (say `rcc` on an
unusual Qt install), the build warns and the app falls back to loading loose files from the
project directory instead of failing. Packaged builds via `day pack` bundle everything properly.
