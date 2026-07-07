// day-part-prefs's OWN Android backend — a headless capability shim (no UI). It is bundled with this
// crate and folded into the app's Gradle build via [package.metadata.day.android], with ZERO edits to
// day-android; it registers no view. It reads/writes an app-private SharedPreferences file using
// day-android's public Context (DayBridge.ctx); no manifest permission is needed. It is the Android
// twin of parts/day-part-prefs/src/file.rs / apple.rs — the same String key/value store, backed here
// by SharedPreferences ("day_part_prefs", MODE_PRIVATE), which persists across launches.
package dev.daybrite.day.prefs;

import android.content.Context;
import android.content.SharedPreferences;

import dev.daybrite.day.bridge.DayBridge;

public final class DayPrefs {
    private DayPrefs() {}

    /** App-private store name; separate from the app's own preferences to avoid key collisions. */
    private static final String STORE = "day_part_prefs";

    private static SharedPreferences prefs() {
        Context ctx = DayBridge.ctx;
        if (ctx == null) return null;
        return ctx.getSharedPreferences(STORE, Context.MODE_PRIVATE);
    }

    /** Persists {@code value} under {@code key}. Returns whether the commit succeeded. */
    public static boolean set(String key, String value) {
        SharedPreferences p = prefs();
        if (p == null || key == null || value == null) return false;
        return p.edit().putString(key, value).commit();
    }

    /** The string stored under {@code key}, or null if absent (or no Context). */
    public static String get(String key) {
        SharedPreferences p = prefs();
        if (p == null || key == null) return null;
        return p.getString(key, null);
    }

    /** Removes {@code key}; returns true only if it existed and the delete committed. */
    public static boolean remove(String key) {
        SharedPreferences p = prefs();
        if (p == null || key == null || !p.contains(key)) return false;
        return p.edit().remove(key).commit();
    }

    /** Whether a value is currently stored under {@code key}. */
    public static boolean contains(String key) {
        SharedPreferences p = prefs();
        if (p == null || key == null) return false;
        return p.contains(key);
    }
}
