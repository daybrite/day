---
navTitle: "Media"
title: "Media playback"
description: "The external day-piece-media crate for native audio and video playback."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Media playback

`day-piece-media` now lives in its [own repository](https://github.com/daybrite/day-piece-media).
It provides audio and video playback, transport controls, volume, playback-state signals, and
stream metadata through native platform players.

## Add to your app

Defined in [`day-piece-media`](https://github.com/daybrite/day-piece-media). Add this to your app’s `Cargo.toml`
`[dependencies]` table:

```toml
[dependencies]
day-piece-media = "0.4"
```

`day build -t <target>` enables this direct dependency’s matching toolkit feature
automatically. No dependency entry is needed in `Day.toml`.

| Platforms / toolkits | Availability |
| --- | --- |
| macOS, iOS, Android, Windows, Linux (GTK / Qt), HarmonyOS, Web | Platform players; codecs, streaming formats, and required runtime libraries vary by toolkit. HarmonyOS video supports mute but not adjustable volume. |

See its [README](https://github.com/daybrite/day-piece-media/blob/main/README.md) for installation
and platform support, and the [implementation guide](https://github.com/daybrite/day-piece-media/blob/main/docs/media.md)
for toolkit behavior and dependencies. The repository includes a demo with a bundled clip and
cross-platform dayscript tests.

Day discovers the crate's native contributions from Cargo metadata. Browser events and volume
handling also ship with the piece, through the shared JavaScript bridge; the framework has no
media-specific browser listener.

## Platform guidance

- [Apple: playing audio](https://developer.apple.com/design/human-interface-guidelines/playing-audio)
- [Android: audio focus](https://developer.android.com/media/optimize/audio-focus)
