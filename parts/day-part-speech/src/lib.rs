// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-part-speech — HEADLESS text to speech. One API; each platform's own engine underneath.
//!
//! ```no_run
//! if day_part_speech::available() != day_bridge::Support::Unsupported {
//!     let _ = day_part_speech::speak("Rain later");
//! }
//! ```
//!
//! This is [daybridge](https://daybrite.dev/docs/internal/bridge)'s reference sample. Every
//! platform has text to speech and every platform exposes it in a different language, so the arms
//! below are written in those languages and live in this file: Swift for `AVSpeechSynthesizer`,
//! Java for Android's `TextToSpeech`, ArkTS for HarmonyOS Core Speech Kit, JavaScript for
//! `speechSynthesis`, C++ for Windows SAPI, C for speech-dispatcher — plus the Rust arm that keeps
//! the crate compiling anywhere else, including under day-mock in `cargo test`.
//!
//! Speech is fire and forget: `speak` returns once the platform has accepted the utterance, not
//! when it finishes speaking (docs/bridge.md "Synchronous means dispatched").

pub use day_bridge::{Error, Support};

/// Speak `text` with the system voice, interrupting anything already speaking.
///
/// `Ok` means the platform accepted the request. Nothing reports completion in v1.
pub fn speak(text: &str) -> Result<(), Error> {
    speak_native(text)
}

/// Stop speaking immediately. Silent no-op where speech is unsupported.
pub fn stop() {
    stop_native();
}

/// What this build's target promises, from the arm the bridge selected: `Native` where the
/// platform's own engine is driven, `Unsupported` where no arm claims the target.
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

    // Apple: AVSpeechSynthesizer. `objc2` could reach it, but the synthesizer has to outlive the
    // call — it stops speaking if it deallocates — and a file-scope `let` in Swift says that in
    // one line.
    #[day_bridge::impl(swift, platforms = [ios, macos])]
    swift!(
        prelude = r#"
            import AVFoundation
        "#,
        body = r#"
            private let daySynthesizer = AVSpeechSynthesizer()

            func speak_native(text: String) throws {
                #if os(iOS)
                try AVAudioSession.sharedInstance().setCategory(.playback, mode: .spokenAudio)
                try AVAudioSession.sharedInstance().setActive(true)
                #endif
                daySynthesizer.stopSpeaking(at: .immediate)
                let utterance = AVSpeechUtterance(string: text)
                utterance.voice = AVSpeechSynthesisVoice(language: Locale.current.identifier)
                daySynthesizer.speak(utterance)
            }

            func stop_native() {
                daySynthesizer.stopSpeaking(at: .immediate)
            }
        "#,
    );

    // Linux: speech-dispatcher's C API. Rust could `#[link]` this directly — it is a C library —
    // so the arm is here to keep the connection handle in the language that owns it, and because
    // it is the smallest honest example of a C arm.
    #[day_bridge::impl(c, platforms = [linux], link = ["speechd"])]
    c!(
        prelude = r#"
            #include <stddef.h>
        "#,
        body = r#"
            /* speech-dispatcher's own header, when the dev package is installed. The declarations
               below keep the arm compiling on a machine that only has the runtime library. */
            typedef struct SPDConnection SPDConnection;
            typedef enum { SPD_MODE_SINGLE = 0, SPD_MODE_THREADED = 1 } SPDConnectionMode;
            typedef enum { SPD_IMPORTANT = 1, SPD_MESSAGE = 3 } SPDPriority;
            extern SPDConnection* spd_open(const char*, const char*, const char*,
                                           SPDConnectionMode);
            extern int spd_say(SPDConnection*, SPDPriority, const char*);
            extern int spd_stop(SPDConnection*);

            static SPDConnection* day_speech_conn = NULL;

            int32_t speak_native(const char* text) {
                if (!day_speech_conn) {
                    day_speech_conn = spd_open("day", "speech", NULL, SPD_MODE_SINGLE);
                    if (!day_speech_conn) return 1;
                }
                spd_stop(day_speech_conn);
                return spd_say(day_speech_conn, SPD_MESSAGE, text) < 0 ? 1 : 0;
            }

            void stop_native(void) {
                if (day_speech_conn) spd_stop(day_speech_conn);
            }
        "#,
    );

    // Windows: SAPI 5, the speech API every supported Windows has had since XP. It is COM —
    // `CoCreateInstance`, an `ISpVoice` interface pointer, `HRESULT`s — which Rust can reach but
    // only by declaring the vtable by hand; three lines of C++ get the same thing from the SDK
    // header. This is also the arm that needs UTF-16: SAPI speaks `WCHAR`, so the declaration's
    // `&str` is converted for this arm and left alone for every other one.
    #[day_bridge::impl(cpp, platforms = [windows], encoding = "utf16", link = ["ole32", "sapi"])]
    cpp!(
        prelude = r#"
            #include <windows.h>
            #include <sapi.h>
        "#,
        body = r#"
            static ISpVoice* day_speech_voice = nullptr;

            /* One voice for the process, created on first use. COM may already be
               initialized on this thread by the host app — S_FALSE and RPC_E_CHANGED_MODE
               both mean "already up", and SAPI is happy in either apartment. */
            static ISpVoice* day_speech_open() {
                if (day_speech_voice) {
                    return day_speech_voice;
                }
                HRESULT hr = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
                if (FAILED(hr) && hr != RPC_E_CHANGED_MODE) {
                    return nullptr;
                }
                if (FAILED(CoCreateInstance(CLSID_SpVoice, nullptr, CLSCTX_ALL, IID_ISpVoice,
                                            reinterpret_cast<void**>(&day_speech_voice)))) {
                    day_speech_voice = nullptr;
                }
                return day_speech_voice;
            }

            int32_t speak_native(const char16_t* text) {
                ISpVoice* voice = day_speech_open();
                if (!voice) {
                    return 1;
                }
                /* SPF_ASYNC returns as soon as the utterance is queued, which is what the contract
                   promises; PURGEBEFORESPEAK drops whatever is still speaking, matching every other
                   arm's "interrupt and say this". */
                HRESULT hr = voice->Speak(reinterpret_cast<const WCHAR*>(text),
                                          SPF_ASYNC | SPF_PURGEBEFORESPEAK, nullptr);
                return SUCCEEDED(hr) ? 0 : 1;
            }

            /* A null utterance with PURGEBEFORESPEAK is SAPI's documented stop. */
            void stop_native(void) {
                if (day_speech_voice) {
                    day_speech_voice->Speak(nullptr, SPF_PURGEBEFORESPEAK, nullptr);
                }
            }
        "#,
    );

    // Android: `TextToSpeech` needs a `Context` and is unusable until its `OnInitListener`
    // fires, so the first utterance is queued and flushed on init. Neither is reachable from Rust,
    // which is why this arm is required rather than a convenience.
    //
    // Written in Java rather than Kotlin so it compiles in any Android project: AGP compiles
    // `.java` from a source directory with no extra plugin, while a `.kt` needs the Kotlin plugin
    // (docs/bridge.md — `day lint` says so before a build gets that far).
    #[day_bridge::impl(java, platforms = [android])]
    java!(
        prelude = r#"
            import android.content.Context;
            import android.speech.tts.TextToSpeech;
            import dev.daybrite.day.bridge.DayBridge;
            import java.util.Locale;
        "#,
        body = r#"
            private static TextToSpeech engine = null;
            private static boolean ready = false;
            private static String pending = null;

            private static void say(String text) {
                if (engine != null) {
                    engine.speak(text, TextToSpeech.QUEUE_FLUSH, null, "day-speech");
                }
            }

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

            public static void stop_native() {
                if (engine != null) {
                    engine.stop();
                }
            }
        "#,
    );

    // HarmonyOS: Core Speech Kit is ArkTS-only — the ArkUI C NDK has no TTS at all — so this arm
    // is required. Its voices are zh-CN in API 13, which is why the arm reports Emulated rather
    // than Native: the platform answers, but not in every language a caller might ask for.
    #[day_bridge::impl(arkts, platforms = [ohos], support = "emulated")]
    arkts!(
        prelude = r#"
            import { textToSpeech } from '@kit.CoreSpeechKit';
        "#,
        body = r#"
            let dayEngine: textToSpeech.TextToSpeechEngine | undefined = undefined;
            let daySeq: number = 0;

            export async function speak_native(text: string): Promise<void> {
                if (!dayEngine) {
                    dayEngine = await textToSpeech.createEngine({
                        language: 'zh-CN', person: 0, online: 1,
                    });
                }
                dayEngine.stop();
                daySeq += 1;
                dayEngine.speak(text, { requestId: `day-speech-${daySeq}` });
            }

            export function stop_native(): void {
                dayEngine?.stop();
            }
        "#,
    );

    // Web: speechSynthesis. Nothing about it is reachable from wasm, so this arm is required
    // rather than a convenience.
    #[day_bridge::impl(js, platforms = [web])]
    js!(r#"
        export function speak_native(text) {
            const utterance = new SpeechSynthesisUtterance(text);
            utterance.lang = document.documentElement.lang || navigator.language;
            speechSynthesis.cancel();
            speechSynthesis.speak(utterance);
        }

        export function stop_native() {
            speechSynthesis.cancel();
        }
    "#);

    // Everywhere without a platform arm — and the arm `cargo test` and day-mock compile against.
    #[day_bridge::impl(rust, platforms = [other])]
    fn speak_native(_text: &str) -> Result<(), day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn stop_native() {}
}

#[cfg(test)]
mod tests {
    /// Speaking must never be fatal, whatever the host has: on a target with no arm this is the
    /// fallback's `Unsupported`, and on one with an arm it drives the real engine.
    #[test]
    fn speaking_is_never_fatal() {
        let _ = super::speak("");
        super::stop();
    }

    /// `available()` and `speak()` must agree: an `Unsupported` target cannot succeed.
    #[test]
    fn support_matches_behavior() {
        if super::available() == super::Support::Unsupported {
            assert_eq!(super::speak("x"), Err(super::Error::Unsupported));
        }
    }
}
