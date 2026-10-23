// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
package dev.daybrite.day.bridge;

/** Shared Android context and UI-thread dispatch, independent of the widget backend. */
public class DayRuntime {
    /** Current host context; the host updates it before initialization and on recreation. */
    public static volatile android.content.Context ctx;
    public static final android.os.Handler main =
            new android.os.Handler(android.os.Looper.getMainLooper());
    protected DayRuntime() {}
    /** The app cache dir (app-writable temp area for save staging). */
    public static String cacheDirPath() {
        try {
            return ctx.getCacheDir().getAbsolutePath();
        } catch (Exception e) {
            android.util.Log.w("Day", "cacheDirPath failed", e);
            return "";
        }
    }

    /** The app-private files dir (app-writable, persistent; for app data stores). */
    public static String filesDirPath() {
        try {
            return ctx.getFilesDir().getAbsolutePath();
        } catch (Exception e) {
            android.util.Log.w("Day", "filesDirPath failed", e);
            return "";
        }
    }

}
