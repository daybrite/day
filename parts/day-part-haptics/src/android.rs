// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Android, whole: the Java that drives Vibrator/VibrationEffect, the declaration that binds it, and
// the mapping from `Haptic` onto the wire code. Nothing about this platform appears anywhere else in
// the crate.
//
// Android is one of the targets whose haptics API cannot be reached from Rust — it needs a `Context`
// and a service lookup, with no C entry point — so it is this crate's only foreign arm
// (docs/bridge.md). Written in Java rather than Kotlin so it compiles in any Android project.
//
// Before daybridge this was a checked-in `DayHaptics.java`, a `[package.metadata.day.android]
// java = [...]` table, a class-name constant, and a hand-written JNI descriptor in Rust. The arm
// below is all four.

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
    // Fire and forget: no Context, no vibrator service, or no hardware all mean "no haptic", and
    // haptics are never worth reporting a failure for.
    play_native(style_code(h));
}

pub fn is_supported() -> bool {
    true
}

day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        /// `style` is a wire code — see `style_code` above, which is the only definition of it.
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
                    // API 26–28: no predefined effects — approximate with a short one-shot buzz.
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

    // The fallback every bridge declares. This file is `#[cfg(target_os = "android")]`, so it is
    // never compiled — it satisfies the rule that a bridge always has an answer for an unclaimed
    // target.
    #[day_bridge::impl(rust, platforms = [other])]
    fn play_native(_style: i32) {}
}
