---
title: "Screenshot bundle"
description: "The screenshots.tar.xz a release carries: a plain tar.xz of uncompressed PNG files that xz compresses across captures, the SHA256SUMS it opens with, the gallery.json fields that describe it, and how day screenshot pack and unpack handle it."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Screenshot bundle

A release carries its captures as `screenshots.tar.xz`, with one `screenshots-<target>[-<device>].tar.xz`
per capture tree beside it. Each is a plain tar.xz file: `tar -xJf` extracts it anywhere, and
`sha256sum -c SHA256SUMS` checks what came out. `day screenshot pack` writes them and
`day screenshot unpack` reads them back; the code is `crates/day-cli/src/screenshot.rs`.

```sh
day screenshot index --screenshot-paths shots-a shots-b --out merged/gallery.json
day screenshot pack --root shots-a --root shots-b --index merged/gallery.json \
  --index-out release/gallery.json --out release/screenshots.tar.xz --each release
day screenshot unpack release/screenshots.tar.xz --check        # verify, write nothing
day screenshot unpack release/screenshots.tar.xz --out restored # verify, write the tree
```

`pack` and `unpack` read files alone, so they run outside a Day project.

## Why it is small

A run's captures are near-copies of each other: one page in eight theme and locale variants,
forty pages around one sidebar. A PNG file compresses alone and sees none of that, and an
archive of compressed PNG files cannot see it either. The bundle stores each capture as a PNG
with stored (uncompressed) deflate blocks and no row filter, so its pixel rows lie open to the
compressor, and orders a tree's captures by shot, then variant, so a page's variants are next
to each other. xz's window, up to 256 MiB, then compresses one capture against the ones before
it.

Day Showcase v0.4.25's iPad captures, 392 files at 2752×2064:

| archive | size |
| --- | --- |
| the PNG files | 191 MB |
| `.tar.gz` of the same files | 120 MB |
| `.tar.bz2` | 73 MB |
| `.tar.zst` (level 19, 1 GiB window) | 24.0 MB |
| `.tar.xz` (preset 6, 256 MiB dictionary) | 21.6 MB |
| `.tar.xz` (1 GiB dictionary) | 20.6 MB, for four times the memory |

## What the round trip keeps

The PNG files in the bundle hold the same width, height and 8-bit RGB or RGBA samples as the
captures they were made from. `tar -xJf` gives you those files, large (an iPad capture is 17 MB).
`unpack` writes each capture back as a compact PNG of the same pixels, and the `gallery.json` it
writes describes those files.

Ancillary chunks (`eXIf`, `pHYs`, text) are not carried; the dayscript runner's captures have
none but `sRGB`, which every PNG in the bundle has. An embedded ICC profile other than sRGB is
not carried either, and changes what the samples mean, so `pack` warns with a count per tree.
16-bit captures are refused.

## The file

The tar's entries, in order:

1. `SHA256SUMS`: one line per later entry, `<sha-256 hex>  <path>`, the form `sha256sum -c` reads.
2. `gallery.json`, in the merged bundle: the index `day screenshot index` wrote, with each
   capture's `archived` size and sha-256 (below).
3. Each tree's other files (its own `gallery.json` indexes, a capture page), then its captures
   by group, shot and variant: `<target>/[<device>/]<variant>/<shot>.png`.

Entries are ustar headers with zero owner and time, so two bundles of the same files are the
same bytes.

The file is a concatenation of xz streams, which xz and tar read as one: a header stream
holding entries 1 and 2, one body stream per tree holding its entries and nothing else, and a
trailer stream holding the tar's end-of-archive marker. A per-tree bundle is that tree's body
between its own header (a `SHA256SUMS` for its entries) and the same trailer. The merged bundle
and the per-tree bundles are assembled from the same compressed bodies, so a tree is compressed
once. Each stream is LZMA2 at preset 6 with a CRC64 check and a dictionary of the smallest
power of two that holds the stream, between 1 MiB and 256 MiB; a reader allocates the
dictionary, an encoder about ten times it.

## The index fields

`pack` adds an `archive` block to the index it writes with `--index-out`, and an `archived`
object to each capture:

```json
{
  "archive": {
    "format": "tar.xz",
    "file": "screenshots.tar.xz",
    "bytes": 63012345,
    "sha256": "…",
    "dictionary": 268435456,
    "parts": [
      { "file": "screenshots-ios-uikit-ipad.tar.xz", "tree": "screenshots-ios-uikit-ipad",
        "bytes": 21601464, "sha256": "…" }
    ]
  },
  "screenshots": [
    { "path": "gallery/ios-uikit/ipad/light/home.png",
      "bytes": 322763, "sha256": "…",
      "archived": { "bytes": 17043816, "sha256": "…" } }
  ]
}
```

| field | meaning |
| --- | --- |
| `archive.format` | `tar.xz` |
| `archive.file`, `archive.bytes`, `archive.sha256` | the merged bundle's name, size and sha-256 |
| `archive.dictionary` | the largest xz dictionary any stream uses, in bytes |
| `archive.parts[]` | each per-tree bundle: its file name, the tree it holds, its size and sha-256 |
| `archived.bytes`, `archived.sha256` | the capture's size and sha-256 as it sits in the bundle, which is what `SHA256SUMS` lists |

The entry's own `bytes` and `sha256` stay those of the capture file that was packed. The
`gallery.json` inside the bundle carries `archived` and `archive.format`/`archive.file`, since
a file cannot hold its own hash.

## Extracting without the CLI

```sh
tar -xJf screenshots.tar.xz
sha256sum -c SHA256SUMS            # shasum -a 256 -c on macOS
```

A single capture from a bundle without extracting the rest: `tar -xJf screenshots.tar.xz
ios-uikit/ipad/light/home.png`. The files are valid PNG files as they are; to compact one,
re-encode it with any PNG tool.

## Normalized captures

The dayscript runner rewrites each capture it saves, so every target's files have one shape
whatever tool took them (`normalize_capture` in `screenshot.rs`):

- 8-bit samples, RGB when every pixel is opaque and RGBA otherwise;
- an `sRGB` chunk and no other ancillary chunk;
- one encoder setting.

The pixels are kept exactly, including the translucent corners of a macOS window. A capture is
left as saved, and the runner says so, when it is 16-bit or embeds an ICC profile other than
sRGB. `DAY_SCREENSHOT_RAW=1` keeps every file as its capture tool wrote it.

Before this, the same kind of image arrived differently per target (Day Showcase v0.4.25):

| target | color type | ancillary chunks |
| --- | --- | --- |
| ios-uikit | RGBA, opaque | `sRGB`, `eXIf` |
| android-mdc, harmony-arkui | RGBA, opaque | `sRGB`, `sBIT` |
| linux-gtk | RGBA, opaque | none |
| linux-qt | RGBA, opaque | `pHYs` |
| web-dom | RGB | none |
| windows-xaml | RGBA, opaque | `sRGB`, `gAMA`, `pHYs` |
| windows-winui | RGBA, a 1-pixel frame at alpha 102 | `sRGB`, `gAMA`, `pHYs` |
| macos-appkit | RGBA, translucent window corners | `iCCP` (the display's profile), `eXIf` |

The AppKit backend converts its capture to sRGB before encoding it, so a macOS capture's samples
are comparable with the other targets' and independent of the capturing display.

## Measurements behind the design

On the same 392 iPad captures, the alternatives tried first:

| encoding | size |
| --- | --- |
| lossless WebP, one file per capture | about 52 MB |
| FFV1 | 119 MB |
| lossless H.265 (`libx265`, slow) | 118 MB |
| lossless H.264 RGB (`libx264rgb -qp 0`, veryslow) | 57 MB |
| previous-frame XOR, then Zstandard 19 | 53 MB |
| raw pixel rows, Zstandard 19, 2^31 window | 23.3 MB |

Subtracting or XOR-ing the previous frame before compressing made the file larger: it replaces
the repeats the window would match with residue that matches nothing. Order matters: variants
adjacent (23.3 MB) against capture order (31.5 MB), one stream against one per theme (34 MB) or
one per variant (49 MB). Two targets share nothing a window finds: iPhone and iPad together came
to the sum of the two apart, which is why each tree is its own stream.
