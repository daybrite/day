---
title: "Sound effects"
description: "Short sound clips via day-part-sound: bundled WAV files played through each platform's own low-latency engine."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Sound effects

`day-part-sound` plays short clips bundled with the app (a tap, a chime, a card sliding onto a
pile) through each platform's own low-latency engine. It is a headless part in `parts/`, like
`day-part-haptics`, and the two are meant to be used together: a game pairs each haptic phrase
with a clip, and the player turns either off in its settings.

## Authoring

```rust
use day_part_sound::Play;

// Decode ahead, so the first play starts at once.
day_part_sound::preload(&[res::assets::sounds::tap_wav, res::assets::sounds::win_wav]);

day_part_sound::play(&res::assets::sounds::tap_wav);
day_part_sound::play_with(&res::assets::sounds::win_wav, Play { volume: 0.8, pan: -0.5, priority: 1 });
```

| Function | What it does |
|---|---|
| `play(&clip)` | play at full volume, centered |
| `play_with(&clip, Play { volume, pan, priority })` | play at a volume from 0 to 1, a pan from −1 (left) to 1 (right), and a priority |
| `preload(&[clip, …])` | load clips now, so their first play is immediate |
| `unload_all()` | release every loaded clip, for a screen that is closing |
| `set_volume(v)` / `volume()` | the master volume every play is scaled by |
| `set_enabled(on)` / `is_enabled()` | a switch for all sound, for an app-wide setting |
| `is_supported()` | whether this platform has a sound engine Day can reach |
| `recent()` | the clips most recently played, oldest first, for tests and walkthroughs |

Every call is fire-and-forget. None returns an error or panics, and none blocks on audio: a
missing or unreadable clip is logged once and stays silent, and so does a platform without an
engine. Call them from the UI thread, where the rest of an app's code runs.

A clip is named by its generated `res::assets::…` constant ([resources.md](resources.md)), or by
path with `AssetName::from_static("sounds/tap.wav")` where a `const` needs one. Day-Games keeps
its cues as statics that pair a clip with a haptic phrase; its `gamekit/src/chrome.rs` is a worked
example.

## Clips

Clips are ordinary data assets under the app's `resource/assets/`, bundled on every platform like
any other file there. The format is WAV. The canonical clip is **16-bit PCM, mono, 44.1 kHz**,
which every engine plays exactly as it is. Other PCM layouts (stereo; 8-, 24- or 32-bit integer;
32-bit float; another sample rate) also play: where an engine takes raw samples (Apple, Windows)
the part converts them to the canonical form as it loads, and elsewhere the platform's decoder
reads them. Compressed formats are not supported on every engine, so the part does not accept
them.

Keep clips short. Android's and HarmonyOS's SoundPool refuse a clip that decodes past about 1 MB,
which is about 11 seconds of the canonical format, and every clip stays in memory while loaded.
ffmpeg converts anything to the canonical form:

```sh
ffmpeg -i in.ogg -ac 1 -ar 44100 -c:a pcm_s16le -map_metadata -1 out.wav
```

## Mixing

- **Voices.** At most `VOICES` (8) clips sound at once. When all are busy, the oldest voice with the
  lowest priority stops for the new clip; a clip whose priority is below everything playing is
  skipped instead.
- **Repeats.** A clip asked for again within `MIN_REPEAT_MS` (40 ms) of its last start is skipped,
  so a burst of the same sound (a streak of bricks, a deal of cards) stays crisp rather than piling
  up into one loud smear.
- **Volume.** A play's volume is multiplied by the master volume; a result of 0 plays nothing.
- **Pan.** Apple platforms, Android, HarmonyOS and the web place a clip across the stereo field.
  Windows and Linux play every clip centered.
- **Other audio.** Sound effects mix with music and podcasts from other apps rather than
  interrupting them. On iOS they follow the Silent switch.

## Per-platform realization

| Platform | Engine | Notes |
|---|---|---|
| iOS | AVAudioEngine with a pool of AVAudioPlayerNodes | the `.ambient` audio session: mixes with other audio, silenced by the Silent switch |
| macOS | AVAudioEngine with a pool of AVAudioPlayerNodes | the engine starts on the first load |
| Android | `SoundPool` with `USAGE_GAME` audio attributes | loads in the background from the APK's assets |
| HarmonyOS | `SoundPool` with `STREAM_USAGE_GAME` | loads in the background from the app's raw files |
| web | Web Audio | fetches and decodes each clip; the audio context starts on the page's first tap or key press, as browsers require |
| Windows | XAudio2 | a mastering voice in the game-effects category and 8 source voices |
| Linux (GTK, Qt) | libcanberra, loaded at run time | the desktop's event-sound service; each clip is cached under `$XDG_CACHE_HOME/day-part-sound/` for it to read |

Where loading happens in the background (Android, HarmonyOS, the web), a clip played before it is
ready is skipped, because a late sound effect is worse than none; `preload` when a screen opens
avoids that. On Linux, `is_supported()` is false when libcanberra is not installed, and the part
has no build-time dependency on it.

The part has no cargo features: the engine is chosen by `#[cfg(target_os)]` and, for Android,
HarmonyOS and the web, by the daybridge arms in `src/lib.rs` ([bridge.md](bridge.md)). On Apple
platforms it links `AVFAudio` through `[package.metadata.day.ios]` and
`[package.metadata.day.macos]` ([extending.md](extending.md)). No platform asks the user for a
permission to play sound.

## Checking it

Run with `DAY_LOG=debug` and every play logs a `play sounds/…` line under the `day_part_sound`
target, whether or not the machine has speakers. In a test or a dayscript, `recent()` gives the
same record: Day-Showcase's walkthrough taps a sound button and asserts the path it shows.

## What it does not do

The part plays clips, not music. A music track needs streaming, looping and seeking, and would
exceed SoundPool's limit; that is a job for a streaming player rather than a sound-effects pool.
