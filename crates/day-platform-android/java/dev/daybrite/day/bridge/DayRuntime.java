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
    /** Attach process services when Android starts a Worker or Service without an Activity. */
    public static synchronized void startServices(android.content.Context context) {
        if (ctx == null) ctx = context.getApplicationContext();
        String library = "dayapp";
        try {
            android.content.pm.ApplicationInfo info = context.getPackageManager()
                    .getApplicationInfo(context.getPackageName(), android.content.pm.PackageManager.GET_META_DATA);
            if (info.metaData != null) library = info.metaData.getString("day.lib", library);
        } catch (android.content.pm.PackageManager.NameNotFoundException e) {
            throw new IllegalStateException(e);
        }
        System.loadLibrary(library);
        initializeServices();
    }
    private static native void initializeServices();
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
