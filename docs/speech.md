---
title: "Speech"
description: "Text-to-speech via day-part-speech, which doubles as daybridge's reference implementation across Swift, Kotlin, and ArkTS."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Speech (headless capability crate, and daybridge's reference)

> **Status: implemented** as `day-part-speech` (in `parts/`, the headless counterpart of `pieces/`).
> It has one Rust API, six foreign implementations, and one file. Verified by driving the Showcase's Platform
> services page: macOS, iOS simulator, Android emulator, and headless WebKit all reach a real
> engine. The Windows arm compiles on CI's `windows-xaml` leg; Linux needs speech-dispatcher
> installed to say anything, and says so through `available()` when it is missing.

> [!IMPORTANT]
> **The code below is copied from `parts/day-part-speech/src/lib.rs`, which is the authority.**
> This page exists because that crate is the worked example every bridge author reads first. When
> the crate changes (a new arm, a changed declaration, a different engine call), update the
> snippets here in the same change, the way [bridge.md](bridge.md) and
> [DESIGN.md](../DESIGN.md) §15.6 are kept in step with the generator.

## Authoring

```rust
if day_part_speech::available() != day_bridge::Support::Unsupported {
    day_part_speech::speak("A clear day, with a chance of rain later.")?;
}
day_part_speech::stop();
```

There are three functions, and calling code needs no platform conditionals:

| Function | Meaning |
|---|---|
| `speak(&str) -> Result<(), day_bridge::Error>` | say it, interrupting anything already speaking |
| `stop()` | stop immediately; a silent no-op where speech is unsupported |
| `available() -> Support` | `Native`, `Emulated`, or `Unsupported`, for this host rather than just this target |

**`Ok` means accepted, not finished.** A v1 bridge call is synchronous and one-shot, so nothing
reports completion; the boundary has no callback tier yet (bridge.md, "After v1"). Anything that
wants to know when the voice stops has to wait for that. The Showcase demo is built around this:
it has no progress readout, because a "Speaking…" label would never clear.

## The shape of the crate

The public API is three thin functions over a declaration, and every platform's engine is an arm
under it:

```rust
pub fn speak(text: &str) -> Result<(), Error> {
    speak_native(text)
}

pub fn stop() {
    stop_native();
}

pub fn available() -> Support {
    speak_native_support()
}

day_bridge::bridge! {
    // The contract. Every arm below implements exactly this; day-build checks that they agree.
    #[day_bridge::declare]
    extern "day" {
        fn speak_native(text: &str) -> Result<(), day_bridge::Error>;
        fn stop_native();
    }
    // … the arms …
}
```

`speak_native`, `stop_native`, and `speak_native_support` are all generated into `OUT_DIR` and
included by the `bridge!` macro; nothing in the crate declares them by hand.

## Per-platform native realization

| OS | Engine | Arm language | Why not Rust |
|---|---|---|---|
| macOS, iOS | `AVSpeechSynthesizer` | Swift | the synthesizer must outlive the call, so it is one file-scope `let` in Swift |
| Android | `TextToSpeech` | Java | needs a `Context` and an `OnInitListener`; unreachable from Rust |
| HarmonyOS | Core Speech Kit | ArkTS | ArkTS-only API with no C entry point (reports `Emulated`: zh-CN voices only) |
| Web | `speechSynthesis` | JavaScript | wasm cannot touch browser APIs |
| Windows | SAPI 5 `ISpVoice` | C++ | COM; the SDK header is shorter than a hand-declared vtable |
| Linux | speech-dispatcher (`dlopen`) | C | reachable from Rust, but the connection handle belongs with the API |
| everywhere else | — | Rust | the fallback that keeps `cargo test` and day-mock compiling |

Two arms show the two shapes every bridge takes.

**An arm that holds state.** Android's `TextToSpeech` is unusable until its listener fires, so the
first utterance is queued and flushed on init, state the Rust side never sees:

```java
public static void speak_native(String text) {
    Context ctx = DayBridge.ctx;
    if (ctx == null) {
        throw new IllegalStateException("no Context");
    }
    if (engine == null) {
        engine = new TextToSpeech(ctx, status -> {
            ready = status == TextToSpeech.SUCCESS;
            if (ready) {
                engine.setLanguage(Locale.getDefault());
                if (pending != null) {
                    say(pending);
                    pending = null;
                }
            }
        });
    }
    if (ready) {
        say(text);
    } else {
        pending = text;
    }
}
```

That `throw` is the error channel: the JVM has no status codes, so an exception is how an arm
fails, and the generated wrapper turns it into `Error::Foreign` (bridge.md, "Errors").

**An arm that needs a different string type.** SAPI speaks `WCHAR`, so the Windows arm opts into
UTF-16 and the *declaration stays the same*; the conversion is generated, and no other arm is
affected:

```rust
#[day_bridge::impl(cpp, platforms = [windows], encoding = "utf16", link = ["ole32", "sapi"])]
cpp!(
    prelude = r#"
        #include <windows.h>
        #include <sapi.h>
    "#,
    body = r#"
        int32_t speak_native(const char16_t* text) {
            ISpVoice* voice = day_speech_open();
            if (!voice) {
                return 1;
            }
            HRESULT hr = voice->Speak(reinterpret_cast<const WCHAR*>(text),
                                      SPF_ASYNC | SPF_PURGEBEFORESPEAK, nullptr);
            return SUCCEEDED(hr) ? 0 : 1;
        }
    "#,
);
```

`SPF_ASYNC` is required, because `speak` must return once the utterance is queued.

## What happens without an engine

Linux is the platform where the engine is a separate package, and the crate is built so that a host
without it degrades in the mildest way available:

- **The app still builds.** The arm declares no `link`, so no development package is needed on any
  build machine ([docs/bridge.md](bridge.md) "Linking"). This is what a `link = ["speechd"]` cost before: CI
  runners failed at the link step with `unable to find library -lspeechd`.
- **The app still launches.** A linked library is a `DT_NEEDED` entry and the loader enforces it
  before `main`; a `dlopen`ed one is looked up when speech is first used, and not before.
- **`available()` reports the engine.** It asks the arm at run time through `engine_ready_native`,
  so a desktop with no speech-dispatcher shows `Unsupported` rather than `Native` followed by
  silence. `speak()` there returns `Err`, and the Showcase demo ignores it as it ignores
  every other failure.

The same three properties are why the Windows arm *does* link `ole32` and `sapi`: both ship with
Windows, so there is nothing to be missing.

## What it shows about the extension system

`day-part-battery` ([battery.md](battery.md)) showed that a headless crate can contribute a platform
implementation. This crate is the case that motivated [daybridge](bridge.md) itself: **six
languages, one file, one declaration**. The generator writes the glue, so the whole crate is
`src/lib.rs` plus a manifest, and there is no wire format to keep in agreement.

It also shows the design's limits. Speech is fire-and-forget, which is the only
thing v1 bridges can express; the parts still waiting on the callback tier (location, sensors,
permissions, local-notify) are the ones whose platform half pushes events back.

## Trying it

The Showcase's **Platform services** page has the demo at the top: a text field (empty means the
localized sample the placeholder shows), Speak, and Stop.

```
day launch -p macos-appkit --script dayscript/speech.yaml
```

The walkthrough checks what a script can check: that the section renders, that it reports the
support the part claims for the target, and that both bridged calls run without wedging the UI.
Hearing the voice is the acceptance test, and it needs a person.
