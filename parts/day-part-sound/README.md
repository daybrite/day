# day-part-sound

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Short sound clips for Day apps, played through each platform's low-latency engine: a tap, a
chime, a card sliding onto a pile, or an engine hum that repeats until it is stopped.

```rust
use day_part_sound::{Play, AssetName};

day_part_sound::preload(&[res::assets::sounds::tap_wav]);
day_part_sound::play(&res::assets::sounds::tap_wav);
day_part_sound::play_with(&res::assets::sounds::tap_wav, Play::at(0.5));

// A loop sounds for as long as its handle lives.
let hum = day_part_sound::play_loop(&res::assets::sounds::hum_wav, Play::at(0.4));
hum.set_volume(0.2);
drop(hum);
```

Clips are ordinary data assets under the app's `resource/assets/`. WAV is the format: 16-bit PCM,
mono, 44.1 kHz plays everywhere without conversion.

| Platform | Engine | Loops |
|---|---|---|
| iOS, macOS | AVAudioEngine with a pool of player nodes; iOS uses the `.ambient` session | native |
| Android | SoundPool with game audio attributes | native |
| web | Web Audio | native |
| Windows | XAudio2 | native |
| Linux | libcanberra, loaded at run time | emulated: a libpulse stream fed the clip round and round |
| HarmonyOS | SoundPool with the game stream usage | native |

Every call is fire-and-forget: none returns an error or panics, and a platform without an engine
stays silent. See [docs/sound.md](../../docs/sound.md).
