# day-part-sound

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Short sound clips for Day apps, played through each platform's own low-latency engine: a tap, a
chime, a card sliding onto a pile.

```rust
use day_part_sound::{Play, AssetName};

day_part_sound::preload(&[res::assets::sounds::tap_wav]);
day_part_sound::play(&res::assets::sounds::tap_wav);
day_part_sound::play_with(&res::assets::sounds::tap_wav, Play::at(0.5));
```

Clips are ordinary data assets under the app's `resource/assets/`. WAV is the format: 16-bit PCM,
mono, 44.1 kHz plays everywhere without conversion.

| Platform | Engine |
|---|---|
| iOS, macOS | AVAudioEngine with a pool of player nodes; iOS uses the `.ambient` session |
| Android | SoundPool with game audio attributes |
| web | Web Audio |
| Windows | XAudio2 |
| Linux | libcanberra, loaded at run time |
| HarmonyOS | SoundPool with the game stream usage |

Every call is fire-and-forget: none returns an error or panics, and a platform without an engine
stays silent. See [docs/sound.md](../../docs/sound.md).
