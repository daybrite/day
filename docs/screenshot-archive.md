---
title: "Screenshot frame archive"
description: "The file day screenshot pack writes: every capture's pixels in one Zstandard stream, the gallery.json fields that describe it, and how a tool extracts and verifies a capture."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Screenshot frame archive

`day screenshot pack` stores a run's captures as one file, `screenshots.frames.zst`, and records
in `gallery.json` where each capture sits and what its pixels hash to. `day screenshot unpack`
checks the file against the index and writes the PNG tree back. The code is
`crates/day-cli/src/screenshot.rs`.

```sh
day screenshot index --screenshot-paths shots --out shots/gallery.json
day screenshot pack shots/gallery.json --out release/screenshots.frames.zst --index-out release/gallery.json
day screenshot unpack release/gallery.json --check        # verify, write nothing
day screenshot unpack release/gallery.json --out restored # verify, write the tree and its index
```

`pack` and `unpack` read an index and files, so they run outside a Day project.

## What the round trip keeps

The archive keeps pixels. A capture unpacks to a PNG with the same width, height and 8-bit
RGB or RGBA samples as the file that was packed, and `unpack` refuses an archive in which any
capture's pixels hash differently from the index.

The PNG files are new encodings. Their bytes, and so their `bytes` and `sha256` in the index,
differ from the originals, and ancillary chunks (`sRGB`, `eXIf`, text) are not carried. The
`gallery.json` that `unpack` writes describes the files it wrote. 16-bit captures are refused
by `pack`.

An embedded ICC profile (`iCCP`) other than sRGB is one of those chunks, and it changes what the
samples mean: the capture unpacks with the same samples tagged sRGB, so its colors shift. `pack`
warns with the count per platform. The dayscript runner's captures are sRGB (see
[Normalized captures](#normalized-captures)); macOS captures taken before 2026-10 embed the
capturing display's profile.

## Normalized captures

The dayscript runner rewrites each capture it saves, so every target's files have one shape
whatever tool took them (`normalize_capture` in `screenshot.rs`):

- 8-bit samples, RGB when every pixel is opaque and RGBA otherwise;
- an `sRGB` chunk and no other ancillary chunk;
- one encoder setting.

The pixels are kept exactly, including the translucent corners of a macOS window. `unpack`
writes the same shape, so a capture that is packed and unpacked comes back byte for byte.

A capture is left as saved, and the runner says so, when it is 16-bit or embeds an ICC profile
other than sRGB. `DAY_SCREENSHOT_RAW=1` keeps every file as its capture tool wrote it.

Before this, the same kind of image arrived differently per target (Day-Showcase v0.4.25):

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

## The file

The archive is a sequence of Zstandard frames, one per group, with nothing between them. A group
is the captures of one platform and device profile (`ios-uikit` on `ipad`, or `macos-appkit`).
Decompressed, a group is its captures' pixels back to back in the order the index lists them:
by shot, then by variant, so the theme and locale variants of one page are adjacent.

A capture's pixels are its rows from top to bottom, without padding or a header:

| `pixels` | bytes per pixel | layout |
| --- | --- | --- |
| `rgb24` | 3 | R, G, B. Used when every pixel of the capture is opaque. |
| `rgba` | 4 | R, G, B, A with straight alpha. |

Each frame is written at level 19 with long-distance matching, a content checksum, and a window
sized to its group, up to 2^30 bytes. A reader allocates that window: up to 1 GiB.

Because the file is plain concatenated Zstandard, stock tools read it. `zstd -d --long=30`
decompresses every group in order, and a reader that wants one group reads `bytes` from `offset`.

## The index fields

`pack` adds an `archive` block to the index and a `frame` to each entry of `screenshots`:

```json
{
  "archive": {
    "format": "day-frames-1",
    "file": "screenshots.frames.zst",
    "bytes": 23567694,
    "sha256": "017ee08b…",
    "compression": "zstd",
    "window_log": 30,
    "groups": [
      { "platform": "ios-uikit", "device": "ipad", "offset": 0, "bytes": 23567694,
        "raw_bytes": 6679830528, "frames": 392 }
    ]
  },
  "screenshots": [
    {
      "path": "gallery/ios-uikit/ipad/dark-ar/home.png",
      "width": 2752,
      "height": 2064,
      "frame": { "group": 0, "offset": 17040384, "bytes": 17040384, "pixels": "rgb24",
                 "sha256": "60371a36…" }
    }
  ]
}
```

| field | meaning |
| --- | --- |
| `archive.format` | `day-frames-1`. A reader refuses a format it does not know. |
| `archive.file` | the archive's file name, beside the index |
| `archive.bytes`, `archive.sha256` | the archive file's size and sha-256 |
| `archive.window_log` | the largest window any group uses, as a power of two |
| `groups[].offset`, `groups[].bytes` | where the group's Zstandard frame sits in the file |
| `groups[].raw_bytes`, `groups[].frames` | the group's decompressed size and capture count |
| `frame.group` | index into `archive.groups` |
| `frame.offset`, `frame.bytes` | where the capture's pixels sit in the group's decompressed bytes |
| `frame.pixels` | `rgb24` or `rgba`; `frame.bytes` is `width × height × 3` or `× 4` |
| `frame.sha256` | sha-256 of those pixel bytes |

The entry's own `bytes` and `sha256` stay those of the PNG file that was packed.

## Extracting a capture without the CLI

1. Check the file's size and sha-256 against `archive.bytes` and `archive.sha256`.
2. Read `groups[g].bytes` bytes at `groups[g].offset` and decompress them with a window limit of
   at least `archive.window_log`.
3. Take `frame.bytes` bytes at `frame.offset` of the result and compare their sha-256 with
   `frame.sha256`.
4. Interpret them as `width × height` pixels in `frame.pixels` layout.

With stock tools, for a file holding one group:

```sh
zstd -d --long=30 -c screenshots.frames.zst | tail -c +17040385 | head -c 17040384 > home.rgb
shasum -a 256 home.rgb
ffmpeg -f rawvideo -pixel_format rgb24 -video_size 2752x2064 -i home.rgb home.png
```

## Measurements

Day-Showcase v0.4.25's iPad captures: 392 files, 2752×2064, 8 variants of 49 shots, 191 MB as
PNG and 159 MB zipped.

| encoding | size |
| --- | --- |
| PNG files | 191 MB |
| lossless WebP, one file per capture | about 52 MB (sampled) |
| FFV1 | 119 MB |
| lossless H.265 (`libx265`, slow) | 118 MB |
| lossless H.264 RGB (`libx264rgb -qp 0`, veryslow) | 57 MB |
| previous-frame XOR, then Zstandard 19 | 53 MB |
| raw frames, Zstandard 19, 2^30 window, shot order | 23.7 MB |

Frame order and batching, all at Zstandard 19:

| layout | window | size |
| --- | --- | --- |
| one stream, shot order (variants adjacent) | 2^31 | 23.3 MB |
| one stream, capture order (variant by variant) | 2^31 | 31.5 MB |
| iPhone and iPad in one stream | 2^31 | 48.0 MB, the sum of the two apart |
| one stream, shot order | 2^28 | 25.6 MB |
| two streams, one per theme | 2^28 | 34.2 MB |
| eight streams, one per variant | 2^28 | 48.8 MB |

A shot's light and dark variants share its images and map tiles, and its locales share
everything but the text, so one stream per device profile is smallest. Two device profiles share
nothing a window finds, which is why each is its own Zstandard frame.

Subtracting or XOR-ing the previous frame before compressing made the file larger: it replaces
the repeats the long window would match with residue that no longer matches other frames.

On a 10-core Mac, packing those 392 captures takes 38 s and 2.4 GB of memory with four
compression threads. Checking takes 4 s and unpacking to PNG 5 s, both within 1.5 GB.
