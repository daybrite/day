---
title: "Media playback"
description: "Play audio and video, observe playback, and retain an audio session across screens."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Media playback

`day-piece-media` plays audio and video through platform players. Use a media piece for playback
attached to a screen, or an audio session when playback needs to outlive that screen.

## Add to your app

Add [`day-piece-media`](https://crates.io/crates/day-piece-media) to your app:

```sh
cargo add day-piece-media
```

[Crate](https://crates.io/crates/day-piece-media) ·
[API reference](https://docs.rs/day-piece-media/latest/day_piece_media/) ·
[Source](https://github.com/daybrite/day-piece-media).
The Day CLI enables the matching toolkit feature and packages the native integration.
No dependency entry is needed in `Day.toml`.

## Play media in a view

This API excerpt accepts a URL supplied by the caller and leaves playback under the person's
control. The selected engine supplies the transport controls.

```rust
use day::prelude::*;
use day_piece_media::{media, PlaybackState};

pub fn player(url: Signal<String>, state: Signal<PlaybackState>) -> impl Piece {
    media(url).no_autoplay().state(state)
}
```

[`media()`](https://docs.rs/day-piece-media/latest/day_piece_media/fn.media.html)
autoplays by default. Use
[`no_autoplay()`](https://docs.rs/day-piece-media/latest/day_piece_media/struct.Media.html#method.no_autoplay)
to require a deliberate start. Browser autoplay policies can block playback even when requested.
For bundled media, use the generated asset constant; see [resources](/docs/reference/resources).

[`Media`](https://docs.rs/day-piece-media/latest/day_piece_media/struct.Media.html)
accepts play, pause, stop, and load triggers, optional looping, mute, and reactive volume.
Volume ranges from 0 to 1. Removing the piece releases its player; a newly constructed piece
starts a new playback lifetime.

## Observe playback and errors

Bind [`state()`](https://docs.rs/day-piece-media/latest/day_piece_media/struct.Media.html#method.state),
[`timeline()`](https://docs.rs/day-piece-media/latest/day_piece_media/struct.Media.html#method.timeline), and
[`error()`](https://docs.rs/day-piece-media/latest/day_piece_media/struct.Media.html#method.error)
to signals when building custom controls. Loading, buffering, playing, paused, ended, and failed
states are distinct. A requested action is not proof that playback started.

[`MediaTimeline`](https://docs.rs/day-piece-media/latest/day_piece_media/struct.MediaTimeline.html)
reports duration and seekable ranges. Live streams can have no known duration or only a moving
seek window. Enable seeking according to the reported ranges rather than assuming every source
can seek. Playback rate support depends on the engine; Linux GTK does not support changing it.

[`StreamMetadata`](https://docs.rs/day-piece-media/latest/day_piece_media/struct.StreamMetadata.html)
contains stream-provided information. Treat station names and titles as external content.
App-owned labels and empty states still use generated localized accessors.

## Keep audio playing across screens

Retain an [`AudioSession`](https://docs.rs/day-piece-media/latest/day_piece_media/struct.AudioSession.html)
in app-owned state. Its snapshots include playback, timeline, errors, and metadata. Clones share
the player; dropping the last handle releases it.

```rust
use day_piece_media::{AudioSession, MediaError};

pub fn start_audio(source: String) -> Result<AudioSession, MediaError> {
    let session = AudioSession::new();
    session.load(source, true)?;
    Ok(session)
}
```

This is an API excerpt; the caller must retain the returned handle. A successful
[`load()`](https://docs.rs/day-piece-media/latest/day_piece_media/struct.AudioSession.html#method.load)
returns a load generation, not a playback-completion result. Observe the snapshot for later
engine failures. Create and use the session on the UI thread.

For background playback, configure
[`BackgroundAudio`](https://docs.rs/day-piece-media/latest/day_piece_media/struct.BackgroundAudio.html)
with localized presentation text on the UI thread. Android needs the media foreground service;
iOS needs the audio background mode. The crate contributes these native requirements through
Day's build metadata. OS policy and permissions still govern playback.
[`NowPlayingInfo`](https://docs.rs/day-piece-media/latest/day_piece_media/struct.NowPlayingInfo.html)
and [`RemoteCommand`](https://docs.rs/day-piece-media/latest/day_piece_media/enum.RemoteCommand.html)
connect metadata and system transport actions to your session.

## Platform behavior

| Platform / toolkit | Player and constraints |
| --- | --- |
| AppKit / UIKit | AVFoundation. Background audio and lock-screen behavior require the appropriate app configuration. |
| Android | Media3/ExoPlayer. Background audio uses a foreground service and media session. |
| WinUI | MediaPlayer. Available formats depend on Windows codecs. |
| Linux GTK | GStreamer. Install the codecs needed for your sources; playback-rate changes are unsupported. |
| Qt | Qt Multimedia and its platform plugins. Include the runtime plugins in the app package. |
| HarmonyOS | Native media players. Video supports mute but not adjustable volume. |
| Web | HTML audio/video. Autoplay, codecs, CORS, and background behavior follow browser policy. |

Test the actual formats, streams, and packaged runtime on each target. Use
[sound effects](/docs/reference/sound) for short clips that do not need a media timeline.
See [Apple's audio guidance](https://developer.apple.com/design/human-interface-guidelines/playing-audio)
and [Android audio focus](https://developer.android.com/media/optimize/audio-focus).
