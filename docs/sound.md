---
navTitle: "Sound"
title: "Sound effects"
description: "Short sound clips via day-part-sound: bundled WAV files played once or looped through each platform's low-latency engine."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Sound effects

[`day-part-sound`](https://daybrite.dev/api/day_part_sound/index.html) plays short clips bundled with the app (a tap, a chime, a card sliding onto a
pile) through each platform's low-latency engine, once or as a [loop](#loops) that repeats until
it is stopped. It is a headless part in `parts/`, like
[`day-part-haptics`](/docs/reference/haptics), and the two are meant to be used together: a game pairs each haptic phrase
with a clip, and the player turns either off in its settings.

## Add to your app

Defined in [`day-part-sound`](https://github.com/daybrite/day/tree/main/crates/day-part-sound). Add this to your app’s `Cargo.toml`
`[dependencies]` table, using the same Day Git source as the rest of your app:

```toml
[dependencies]
day-part-sound = { git = "https://github.com/daybrite/day.git" }
```

The implementation is selected by the compilation target; no toolkit feature flags
or dependency entry in `Day.toml` are needed.

| Platforms / toolkits | Availability |
| --- | --- |
| macOS, iOS, Android, Windows, Linux (GTK / Qt), HarmonyOS | Sound effects and loops; Linux requires libpulse or PipeWire’s PulseAudio compatibility layer. |
| Web | Web Audio requires a user interaction before playback can start. |

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
| [`play(&clip)`](https://daybrite.dev/api/day_part_sound/fn.play.html) | play at full volume, centered |
| [`play_with(&clip, Play { volume, pan, priority })`](https://daybrite.dev/api/day_part_sound/fn.play_with.html) | play at a volume from 0 to 1, a pan from −1 (left) to 1 (right), and a priority |
| [`play_loop(&clip, Play { volume, pan, .. })`](https://daybrite.dev/api/day_part_sound/fn.play_loop.html) | repeat the clip until the [`Loop`](https://daybrite.dev/api/day_part_sound/struct.Loop.html) it returns is dropped |
| [`Loop::set_volume(v)`](https://daybrite.dev/api/day_part_sound/struct.Loop.html#method.set_volume) / [`Loop::set_pan(p)`](https://daybrite.dev/api/day_part_sound/struct.Loop.html#method.set_pan) | change a loop while it sounds |
| [`Loop::is_playing()`](https://daybrite.dev/api/day_part_sound/struct.Loop.html#method.is_playing) / [`Loop::stop()`](https://daybrite.dev/api/day_part_sound/struct.Loop.html#method.stop) | whether the loop is still held, and stopping it by name |
| [`preload(&[clip, …])`](https://daybrite.dev/api/day_part_sound/fn.preload.html) | load clips now, so their first play is immediate |
| [`unload_all()`](https://daybrite.dev/api/day_part_sound/fn.unload_all.html) | stop everything, loops included, and release every loaded clip, for a screen that is closing |
| [`set_volume(v)`](https://daybrite.dev/api/day_part_sound/fn.set_volume.html) / [`volume()`](https://daybrite.dev/api/day_part_sound/fn.volume.html) | the master volume every play and every loop is scaled by |
| [`set_enabled(on)`](https://daybrite.dev/api/day_part_sound/fn.set_enabled.html) / [`is_enabled()`](https://daybrite.dev/api/day_part_sound/fn.is_enabled.html) | a switch for all sound, for an app-wide setting |
| [`is_supported()`](https://daybrite.dev/api/day_part_sound/fn.is_supported.html) | whether this platform has a sound engine Day can reach |
| [`recent()`](https://daybrite.dev/api/day_part_sound/fn.recent.html) | the clips most recently played, oldest first, for tests and walkthroughs |
| [`loops()`](https://daybrite.dev/api/day_part_sound/fn.loops.html) | the clips looping now, oldest first, for tests and walkthroughs |

Every call is fire-and-forget. None returns an error or panics, and none blocks on audio: a
missing or unreadable clip is logged once and stays silent, and so does a platform without an
engine. Call them from the UI thread, where the rest of an app's code runs.

A bundled clip is named by its generated `res::assets::…` constant ([resources](resources.md)).
These constants can also be used in static cue tables; do not repeat the asset's path. Games Fair keeps
its cues as statics that pair a clip with a haptic phrase; its `gamekit/src/chrome.rs` is a worked
example.

## Loops

A clip that should keep sounding (an engine hum, a siren, rain) is a loop. [`play_loop`](https://daybrite.dev/api/day_part_sound/fn.play_loop.html) starts it
and returns a `Loop`; the clip repeats for as long as that value lives and stops when it is
dropped.

```rust
use day_part_sound::{Loop, Play};

struct Race {
    engine: Option<Loop>,
}

impl Race {
    fn start(&mut self) {
        self.engine = Some(day_part_sound::play_loop(&res::assets::sounds::engine_wav, Play::at(0.6)));
    }
    fn throttle(&self, level: f32) {
        if let Some(engine) = &self.engine {
            engine.set_volume(0.3 + 0.7 * level);
        }
    }
    fn pause(&mut self) {
        self.engine = None; // the engine stops
    }
}
```

Tying the sound to a value means it ends with whatever owns that value: a paused game sets an
`Option` to `None`, and a page that closes drops its state. Replacing one loop with another is an
assignment, which starts the new loop and then stops the old one. Bind the result, because
[`play_loop(…);`](https://daybrite.dev/api/day_part_sound/fn.play_loop.html) alone drops the handle and stops the loop at once; the compiler warns about it.

- **Authoring.** The clip repeats from its last sample straight to its first, so it must close
  on itself: a whole number of cycles of a tone, or a recording cut where the level and the
  slope match. A clip that fades in or out pulses once per repeat.
- **Volume and the switch.** A loop is scaled by the master volume and obeys [`set_enabled`](https://daybrite.dev/api/day_part_sound/fn.set_enabled.html), and
  follows both as they change. While its volume comes to zero it is silent but still held, and
  it starts again from the top of the clip when it has a volume.
- **Voices.** Loops have [`LOOPS`](https://daybrite.dev/api/day_part_sound/constant.LOOPS.html) (4) voices of their own. A loop never takes one of the eight
  clip voices and never loses its voice to a clip, which is why [`Play::priority`](https://daybrite.dev/api/day_part_sound/struct.Play.html#structfield.priority) means nothing to
  [`play_loop`](https://daybrite.dev/api/day_part_sound/fn.play_loop.html). A fifth loop stops the oldest; that loop's handle then reports [`is_playing()`](https://daybrite.dev/api/day_part_sound/struct.Loop.html#method.is_playing)
  false.
- **Loading.** Where clips load in the background (Android, HarmonyOS, the web), a loop asked for
  before its clip is ready starts when it is.
- **The background.** On Android a loop pauses while the app has no activity in front and
  resumes with it. On the web the audio context is suspended while a loop's tab is hidden. iOS
  silences the `.ambient` session in the background. A desktop app keeps sounding when another
  window is in front, as desktop apps do. A game should still stop its loops when it pauses.
- **The thread.** A `Loop` belongs to the UI thread and is neither `Send` nor `Sync`.

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

- **Voices.** At most [`VOICES`](https://daybrite.dev/api/day_part_sound/constant.VOICES.html) (8) clips sound at once. When all are busy, the oldest voice with the
  lowest priority stops for the new clip; a clip whose priority is below everything playing is
  skipped instead.
- **Repeats.** A clip asked for again within [`MIN_REPEAT_MS`](https://daybrite.dev/api/day_part_sound/constant.MIN_REPEAT_MS.html) (40 ms) of its last start is skipped,
  so a burst of the same sound (a streak of bricks, a deal of cards) stays crisp rather than piling
  up into one loud smear.
- **Volume.** A play's volume is multiplied by the master volume; a result of 0 plays nothing.
- **Pan.** Apple platforms, Android, HarmonyOS, Linux and the web place a clip or a loop across
  the stereo field. Windows plays every one centered.
- **Other audio.** Sound effects mix with music and podcasts from other apps rather than
  interrupting them. On iOS they follow the Silent switch.

## Per-platform realization

Platform guidance: [Apple: playing audio](https://developer.apple.com/design/human-interface-guidelines/playing-audio); [Android: audio focus](https://developer.android.com/media/optimize/audio-focus).

| Platform | Engine | Notes | Loops |
|---|---|---|---|
| iOS | [AVAudioEngine](https://developer.apple.com/documentation/avfaudio/avaudioengine) with a pool of [AVAudioPlayerNodes](https://developer.apple.com/documentation/avfaudio/avaudioplayernode) | the `.ambient` audio session: mixes with other audio, silenced by the Silent switch | native: the node is given the buffer with the `Loops` option |
| macOS | [AVAudioEngine](https://developer.apple.com/documentation/avfaudio/avaudioengine) with a pool of [AVAudioPlayerNodes](https://developer.apple.com/documentation/avfaudio/avaudioplayernode) | the engine starts on the first load | native, as on iOS |
| Android | [`SoundPool`](https://developer.android.com/reference/android/media/SoundPool) with `USAGE_GAME` audio attributes | loads in the background from the APK's assets | native: a stream played with `loop = -1` |
| HarmonyOS | [`SoundPool`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-media-kit/js-apis-inner-multimedia-soundPool.md) with `STREAM_USAGE_GAME` | loads in the background from the app's raw files | native: a stream played with `loop: -1` |
| web | [Web Audio](https://www.w3.org/TR/webaudio/) | fetches and decodes each clip; the audio context starts on the page's first tap or key press, as browsers require | native: a source node with `loop` set |
| Windows | [XAudio2](https://learn.microsoft.com/en-us/windows/win32/xaudio2/xaudio2-introduction) | a mastering voice in the game-effects category, 8 source voices for clips and 4 for loops | native: the buffer's `LoopCount` is `XAUDIO2_LOOP_INFINITE` |
| Linux (GTK, Qt) | [libpulse](https://www.freedesktop.org/software/pulseaudio/doxygen/), loaded at run time, with the part's own mixer | reaches PulseAudio and PipeWire's Pulse layer; the stream is open only while something sounds | emulated: the mixer wraps the clip from its last sample to its first |

Every native loop repeats sample-exactly inside the engine. [Linux](#linux) mixes in software.

On Apple platforms a route change (headphones, a new output device) stops the engine. The part
starts it again on the next call that makes sound and puts every loop back on its node, so a
loop that was the only sound stays silent until then.

Where loading happens in the background (Android, HarmonyOS, the web), a clip played before it is
ready is skipped, because a late sound effect is worse than none; [`preload`](https://daybrite.dev/api/day_part_sound/fn.preload.html) when a screen opens
avoids that.

The part has no cargo features: the engine is chosen by `#[cfg(target_os)]` and, for Android,
HarmonyOS and the web, by the daybridge arms in `src/lib.rs` ([bridge](bridge.md)). On Apple
platforms it links `AVFAudio` through `[package.metadata.day.ios]` and
`[package.metadata.day.macos]` ([extending](extending.md)). No platform asks the user for a
permission to play sound.

### Linux

The part plays through **[libpulse](https://www.freedesktop.org/software/pulseaudio/doxygen/)**, the client library of the PulseAudio sound server. PipeWire
speaks the same protocol through its Pulse layer (`pipewire-pulse`), so the one library reaches
the sound server on every current desktop. It is the only audio library the part uses on Linux.

[libpulse](https://www.freedesktop.org/software/pulseaudio/doxygen/)'s simple API offers a stream to write samples into, so the part mixes in software. One
thread owns one stereo stream at 44.1 kHz. Each clip and each loop sounding is a voice in the
mix: every 10 ms the thread sums them, each at its own volume and pan, and writes the result.

- **Latency.** The stream holds 50 ms, so a clip is heard, and a stopped loop goes silent, within
  about that.
- **Voices and priority.** The mixer keeps the same rules as the native engines: eight clip
  voices, the oldest of the lowest priority making way for a ninth, and four loop voices.
- **Loops.** A loop is a voice that wraps from its last sample to its first, so every repeat is
  sample-exact. This is the one platform where the part does the looping itself.
- **Changes.** A new loop volume or pan is eased in across one 10 ms write, and a stopped loop
  or a displaced clip fades out across one, because a step in level is a click.
- **The stream.** It opens when the first sound plays and closes two seconds after the last one
  ends. A quiet app holds no connection, and a run of taps shares one.
- **Without a server.** When no sound server answers, clips are dropped and a held loop is tried
  again every few seconds.

**What a machine needs.** `libpulse-simple.so.0` at run time. Desktop installs carry it: it is
in `libpulse0` on Debian and Ubuntu, `pulseaudio-libs` on Fedora, and [`libpulse`](https://www.freedesktop.org/software/pulseaudio/doxygen/) on Arch. The
part loads it with `dlopen`, so the app has no build-time dependency on it, starts without it,
and reports [`is_supported()`](https://daybrite.dev/api/day_part_sound/fn.is_supported.html) false there. A minimal container or CI image may need the package
installed:

```sh
sudo apt install libpulse0        # Debian / Ubuntu
```

**In a Flatpak.** The sandbox has no route to the sound server unless the manifest asks for one.
The part declares `finish-args = ["--socket=pulseaudio"]` in its
`[package.metadata.day.flatpak]` table, and `day pack` adds that to the manifest of any app that
depends on it ([extending](extending.md#flatpak-dependencies)). The GNOME and KDE runtimes
both ship libpulse. That socket carries recording as well as playback, which is how Flatpak
defines it.

**In an AppImage.** The AppImage uses the host's [libpulse](https://www.freedesktop.org/software/pulseaudio/doxygen/), like any other app on the machine.

## Checking it

Run with `DAY_LOG=debug` and every play logs a `play sounds/…` line, and every loop a
`loop sounds/…` line, under the `day_part_sound` target, whether or not the machine has speakers.
In a test or a dayscript, [`recent()`](https://daybrite.dev/api/day_part_sound/fn.recent.html) and [`loops()`](https://daybrite.dev/api/day_part_sound/fn.loops.html) give the same record: Day-Showcase's
walkthrough taps a sound button and asserts the path it shows, then turns its loop on and off
and asserts what is looping.

## What it does not do

The part plays clips, not music. A loop repeats a short clip held in memory; a music track needs
streaming and seeking, and would exceed SoundPool's limit. That is a job for a streaming player
rather than a sound-effects pool. A loop also keeps the pitch and speed its clip was recorded at.
