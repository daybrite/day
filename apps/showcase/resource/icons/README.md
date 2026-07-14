# Day — Ember Dawn (4d) icon exports

Master art: `day-icon.svg` — rising-sun / rotated-D dome in a rust→amber gradient (#B7410E → #EFA94A), amber alternating long/short rays, ember horizon (#D95B29), charcoal ground (#201512).

## iOS (`ios/`)
- `AppIcon-1024.png` — full-bleed square, no transparency. Drop into Xcode's AppIcon asset (single 1024 size); iOS applies its own corner mask.

## Android (`android/`)
- `ic_launcher_foreground.svg/png` (432×432, 108dp adaptive canvas, motif inside the 66dp safe zone, transparent bg)
- `ic_launcher_background.svg/png` (solid #201512)
- `ic_launcher-legacy-192.png` — legacy launcher fallback
- `play-store-512.png` — Play listing icon (full-bleed, opaque)

## macOS (`macos/`)
- `day-icon-macos-*.png` — 1024/512/256/128/32/16 with Apple's standard transparent margin (824 pt art on 1024 canvas). Convert to `.icns` with `iconutil` or drop PNGs into an Xcode iconset.

## Windows (`windows/`)
- `day.ico` — multi-size (256/48/32/16, PNG-compressed)
- `day-icon-256.png`

## Linux (`linux/`)
- `day-icon-{512,256,128,48}.png` — install under `hicolor/<size>x<size>/apps/`. `../day-icon.svg` works for `hicolor/scalable/apps/`.

## Web / general (`png/`)
- `day-icon-{1024,512,256,128,64,32,16}.png` — favicons, PWA manifest, etc.
