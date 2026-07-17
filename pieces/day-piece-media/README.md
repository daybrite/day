# day-piece-media

Play video in a Day app with the platform's own player.

`AVPlayerView` on macOS, `AVPlayerViewController` on iOS, `QMediaPlayer` with a
`QVideoWidget` on Qt, and ExoPlayer-backed playback on Android — play, pause, and seek
from one Rust API, with the platform handling decoding and controls.

Pieces are Day's reusable UI components, shipped as ordinary crates: one Rust API in
front, a real native control per platform behind it. Enable the backends you build for
with cargo features, and `day build` wires up the native side automatically.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
