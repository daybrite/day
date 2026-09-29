// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Android and HarmonyOS, whole: the Java that drives Vibrator/VibrationEffect, the ArkTS that drives
// the Sensor Service Kit's vibrator, the declaration that binds them, and the mapping from `Haptic`
// onto the wire code. Nothing about these platforms appears anywhere else in the crate.
//
// Both are targets whose haptics API cannot be reached from Rust in a useful form (Android needs a
// `Context` and a service lookup with no C entry point; HarmonyOS's C vibrator takes only raw
// durations, while its ArkTS one plays the system's own haptic presets), so they are this crate's
// foreign arms (docs/bridge.md). The Android arm is Java rather than Kotlin so it compiles in any
// Android project.
//
// Before daybridge the Android half was a checked-in `DayHaptics.java`, a
// `[package.metadata.day.android] java = [...]` table, a class-name constant, and a hand-written
// JNI descriptor in Rust. The arm below is all four.

use super::Haptic;

/// The wire code the Java switches on. It stays a number rather than a string because an enum is
/// the one thing the v1 type table cannot carry (docs/bridge.md "Types").
fn style_code(h: Haptic) -> i32 {
    match h {
        Haptic::Light => 0,
        Haptic::Medium => 1,
        Haptic::Heavy => 2,
        Haptic::Success => 3,
        Haptic::Warning => 4,
        Haptic::Error => 5,
        Haptic::Selection => 6,
    }
}

pub fn play(h: Haptic) {
    // Fire and forget: a missing Context, vibrator service, or hardware all mean "no haptic", and
    // haptics are never worth reporting a failure for.
    play_native(style_code(h));
}

pub fn is_supported() -> bool {
    // Android always reaches its arm; HarmonyOS does where `day build` staged the ArkTS.
    cfg!(target_os = "android") || play_native_support() != day_bridge::Support::Unsupported
}

day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        /// `style` is a wire code; see `style_code` above, which is the only definition of it.
        fn play_native(style: i32);
    }

    #[day_bridge::impl(java, platforms = [android])]
    java!(
        prelude = r#"
            import android.content.Context;
            import android.os.Build;
            import android.os.VibrationEffect;
            import android.os.Vibrator;
            import android.os.VibratorManager;
            import dev.daybrite.day.bridge.DayBridge;
        "#,
        body = r#"
            private static final int LIGHT = 0;
            private static final int MEDIUM = 1;
            private static final int HEAVY = 2;
            private static final int SUCCESS = 3;
            private static final int WARNING = 4;
            private static final int ERROR = 5;
            private static final int SELECTION = 6;

            public static void play_native(int style) {
                Context ctx = DayBridge.ctx;
                if (ctx == null) {
                    return;
                }
                Vibrator vib = vibrator(ctx);
                if (vib == null || !vib.hasVibrator()) {
                    return;
                }
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    // API 29+: predefined system effects feel like the real UI haptics.
                    vib.vibrate(VibrationEffect.createPredefined(predefined(style)));
                } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    // API 26–28: no predefined effects; approximate with a short one-shot buzz.
                    vib.vibrate(VibrationEffect.createOneShot(durationMs(style),
                            VibrationEffect.DEFAULT_AMPLITUDE));
                } else {
                    // Pre-API 26: only the deprecated duration-based vibrate exists.
                    vib.vibrate(durationMs(style));
                }
            }

            private static Vibrator vibrator(Context ctx) {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                    VibratorManager mgr = (VibratorManager)
                            ctx.getSystemService(Context.VIBRATOR_MANAGER_SERVICE);
                    return mgr == null ? null : mgr.getDefaultVibrator();
                }
                return (Vibrator) ctx.getSystemService(Context.VIBRATOR_SERVICE);
            }

            // Each style onto the closest predefined VibrationEffect (API 29+).
            private static int predefined(int style) {
                switch (style) {
                    case LIGHT:
                    case SELECTION:
                        return VibrationEffect.EFFECT_TICK;
                    case HEAVY:
                    case WARNING:
                        return VibrationEffect.EFFECT_HEAVY_CLICK;
                    case SUCCESS:
                    case ERROR:
                        return VibrationEffect.EFFECT_DOUBLE_CLICK;
                    case MEDIUM:
                    default:
                        return VibrationEffect.EFFECT_CLICK;
                }
            }

            // Fallback intensities for pre-API-29 devices: length stands in for strength.
            private static long durationMs(int style) {
                switch (style) {
                    case LIGHT:
                    case SELECTION:
                        return 10L;
                    case HEAVY:
                    case WARNING:
                    case ERROR:
                        return 40L;
                    default:
                        return 20L;
                }
            }
        "#,
    );

    // HarmonyOS: the system's haptic presets where the device has them (what its own controls
    // play), else a short timed buzz whose length stands in for strength, like Android's
    // pre-API-29 fallback. `VIBRATE` is declared through the crate's ohos metadata.
    #[day_bridge::impl(arkts, platforms = [ohos])]
    arkts!(
        prelude = r#"
            import { vibrator } from '@kit.SensorServiceKit';
            import { BusinessError } from '@kit.BasicServicesKit';
        "#,
        body = r#"
            // Light, Medium, Heavy, Success, Warning, Error, Selection (the wire codes).
            const DAY_PRESETS: string[] = [
              'haptic.effect.soft', 'haptic.clock.timer', 'haptic.effect.hard',
              'haptic.effect.sharp', 'haptic.effect.hard', 'haptic.effect.hard', 'haptic.clock.timer'
            ];
            const DAY_MS: number[] = [10, 20, 40, 20, 40, 40, 10];

            function daySupports(preset: string): boolean {
              try {
                return vibrator.isSupportEffectSync(preset);
              } catch (e) {
                return false;
              }
            }

            export function play_native(style: number): void {
              const i: number = style >= 0 && style < DAY_MS.length ? style : 1;
              const effect: vibrator.VibrateEffect = daySupports(DAY_PRESETS[i])
                ? { type: 'preset', effectId: DAY_PRESETS[i], count: 1 } as vibrator.VibratePreset
                : { type: 'time', duration: DAY_MS[i] } as vibrator.VibrateTime;
              vibrator.startVibration(effect, { id: 0, usage: 'touch' })
                .catch((e: BusinessError) => {
                  console.info(`day-part-haptics: ${e.code} ${e.message}`);
                });
            }
        "#,
    );

    // The fallback every bridge declares. This file is compiled only on the two targets above, so
    // it never runs; it satisfies the rule that a bridge always has an answer for an unclaimed
    // target.
    #[day_bridge::impl(rust, platforms = [other])]
    fn play_native(_style: i32) {}
}
