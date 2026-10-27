---
title: "Speech"
description: "Read text aloud with the system voice, observe completion, and stop an utterance."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Speech

Read a passage, announce an app event, or offer spoken playback of user-provided text.
`day-part-speech` uses the operating system's speech engine. For short interface feedback,
also consider [sound](/docs/reference/sound) and [accessibility](/docs/reference/accessibility).

## Add to your app

Add [`day-part-speech`](https://crates.io/crates/day-part-speech) to your app:

```sh
cargo add day-part-speech
```

Build with the [Day CLI](/docs/reference/cli) so its native bridge code is packaged. This
[part](/docs/reference/parts) needs no toolkit feature or extra `Day.toml` entry.
[API reference](https://docs.rs/day-part-speech/latest/day_part_speech/) ·
[Source](https://github.com/daybrite/day-part-speech).

## Speak and observe completion

This helper accepts text from the caller and returns the utterance's outcome. Call it from a
UI action under [`day::task`](https://docs.rs/day-app/latest/day/fn.task.html).
For app-authored announcements, format a generated `res::str` message before passing it in;
user-supplied text can be passed directly.

```rust
use day_part_speech::{Error, SpeechEnd, speak_future};

pub async fn read_aloud(text: String) -> Result<SpeechEnd, Error> {
    speak_future(&text).await
}
```

| Entry point | Behavior |
| --- | --- |
| [`available()`](https://docs.rs/day-part-speech/latest/day_part_speech/fn.available.html) | Reports `Native`, `Emulated`, or `Unsupported` for the host. |
| [`speak(text)`](https://docs.rs/day-part-speech/latest/day_part_speech/fn.speak.html) | Accepts an utterance and interrupts the previous one; success does not mean playback finished. |
| [`speak_future(text)`](https://docs.rs/day-part-speech/latest/day_part_speech/fn.speak_future.html) | Resolves when the engine reports completion, interruption, or failure. |
| [`speak_until_done(text, callback)`](https://docs.rs/day-part-speech/latest/day_part_speech/fn.speak_until_done.html) | Calls the callback once, including when the initial request fails. The callback may run on an engine thread. |
| [`stop()`](https://docs.rs/day-part-speech/latest/day_part_speech/fn.stop.html) | Stops the current utterance; does nothing when speech is unavailable. |

[`SpeechEnd`](https://docs.rs/day-part-speech/latest/day_part_speech/enum.SpeechEnd.html)
distinguishes finished playback, interruption, and an accepted utterance whose completion the
engine cannot observe. Starting another utterance replaces the current one; this API does not
provide an app-owned queue. Dropping the future stops observing it but does not stop the voice.
Stop playback explicitly when that is the intended behavior on navigation or cancellation.

Prefer the future form for UI code. If using the callback form, send data through a
[`Setter`](https://docs.rs/day-reactive/latest/day_reactive/struct.Setter.html) and format localized
messages on the UI thread. See [async ownership](/docs/explanation/async).

## Platform support

| Platform | Engine and limits |
| --- | --- |
| macOS / iOS | [AVSpeechSynthesizer](https://developer.apple.com/documentation/avfaudio/avspeechsynthesizer); uses the system voice. |
| Android | [TextToSpeech](https://developer.android.com/reference/android/speech/tts/TextToSpeech); engine initialization can delay the first utterance. |
| Windows | [Speech API](https://learn.microsoft.com/en-us/previous-versions/windows/desktop/ee125663(v=vs.85)); uses an installed SAPI voice. |
| Linux / GTK / Qt | speech-dispatcher must be installed and available. Completion returns `Unobserved`; it does not track the end of audible speech. |
| HarmonyOS | Core Speech Kit integration is reported as emulated and uses a Chinese voice configuration. |
| Web | [SpeechSynthesis](https://developer.mozilla.org/en-US/docs/Web/API/SpeechSynthesis); installed voices and playback policy depend on the browser. |

Handle unsupported and engine-error results even after checking availability. The API does not
expose voice selection, rate, pitch, or a per-utterance language. When language choice is
essential, verify that the system voice suits the text before offering this feature.

## Related documentation

- [Sound](/docs/reference/sound) for short system sounds and haptics.
- [Media playback](/docs/reference/media) for recorded audio and longer playback sessions.
- [Localization](/docs/reference/localization) for app-authored spoken messages.
