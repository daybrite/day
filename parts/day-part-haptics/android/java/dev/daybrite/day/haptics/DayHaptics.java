// day-part-haptics's OWN Android backend — a headless capability shim (no UI). It is bundled with
// this crate and folded into the app's Gradle build via [package.metadata.day.android], with ZERO
// edits to day-android; it registers no view, and the same overlay merges android.permission.VIBRATE
// into the app manifest. It plays a haptic through Vibrator/VibrationEffect using day-android's
// public Context (DayBridge.ctx). It is the Android twin of parts/day-part-haptics/src/*.rs's other
// per-OS impls.
package dev.daybrite.day.haptics;

import android.content.Context;
import android.os.Build;
import android.os.VibrationEffect;
import android.os.Vibrator;
import android.os.VibratorManager;

import dev.daybrite.day.bridge.DayBridge;

public final class DayHaptics {
    private DayHaptics() {}

    // Wire codes from parts/day-part-haptics/src/android.rs::style_code.
    private static final int LIGHT = 0;
    private static final int MEDIUM = 1;
    private static final int HEAVY = 2;
    private static final int SUCCESS = 3;
    private static final int WARNING = 4;
    private static final int ERROR = 5;
    private static final int SELECTION = 6;

    /**
     * Play one haptic. `style` is a wire code (see the constants above). Fire-and-forget: silently
     * does nothing when there is no Context, no vibrator service, or no vibrator hardware.
     */
    public static void play(int style) {
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
            VibratorManager mgr =
                    (VibratorManager) ctx.getSystemService(Context.VIBRATOR_MANAGER_SERVICE);
            return mgr == null ? null : mgr.getDefaultVibrator();
        }
        return (Vibrator) ctx.getSystemService(Context.VIBRATOR_SERVICE);
    }

    // Map each style onto the closest predefined VibrationEffect (API 29+).
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
}
