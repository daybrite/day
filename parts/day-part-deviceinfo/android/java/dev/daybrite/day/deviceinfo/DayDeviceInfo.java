// day-part-deviceinfo's OWN Android backend — a headless capability shim (no UI). It is bundled with
// this crate and folded into the app's Gradle build via [package.metadata.day.android], with ZERO
// edits to day-android; it registers no view and needs no permission. It reads the freely-accessible
// android.os.Build static fields (no Context required — unlike battery/network/clipboard, which need
// DayBridge.ctx). It is the Android twin of parts/day-part-deviceinfo/src/*.rs's other per-OS impls.
package dev.daybrite.day.deviceinfo;

import android.os.Build;

public final class DayDeviceInfo {
    private DayDeviceInfo() {}

    // ASCII unit separator (U+001F): a byte that cannot appear in a Build value, so it safely joins
    // the four fields into one string that the Rust side splits back apart.
    private static final char SEP = (char) 0x1F;

    /**
     * Returns the device identity as four fields joined by U+001F:
     * model, "Android", VERSION.RELEASE, and "1"/"0" for the emulator flag.
     * Never returns null; unknown parts are emitted as "Unknown".
     */
    public static String read() {
        String model = model();
        String release = nonEmpty(Build.VERSION.RELEASE);
        String emulator = isEmulator() ? "1" : "0";
        return model + SEP + "Android" + SEP + release + SEP + emulator;
    }

    /** MODEL, prefixed with MANUFACTURER when the model does not already start with it. */
    private static String model() {
        String model = nonEmpty(Build.MODEL);
        String manufacturer = Build.MANUFACTURER;
        if (manufacturer != null && !manufacturer.isEmpty()
                && !model.toLowerCase().startsWith(manufacturer.toLowerCase())
                && !model.equals("Unknown")) {
            return manufacturer + " " + model;
        }
        return model;
    }

    /** Heuristic emulator detection from the standard AOSP/Google emulator build fingerprints. */
    private static boolean isEmulator() {
        String fingerprint = lower(Build.FINGERPRINT);
        String product = lower(Build.PRODUCT);
        String model = lower(Build.MODEL);
        String hardware = lower(Build.HARDWARE);
        return fingerprint.contains("generic")
                || fingerprint.contains("emulator")
                || product.contains("sdk")
                || product.contains("emulator")
                || model.contains("emulator")
                || model.contains("android sdk")
                || hardware.contains("goldfish")
                || hardware.contains("ranchu");
    }

    private static String nonEmpty(String s) {
        return (s == null || s.isEmpty()) ? "Unknown" : s;
    }

    private static String lower(String s) {
        return s == null ? "" : s.toLowerCase();
    }
}
