package dev.daybrite.day.bridge;

import android.content.pm.ActivityInfo;
import android.content.pm.PackageManager;
import android.os.Bundle;
import android.util.DisplayMetrics;

/** The host Activity: creates the root DayFixed and, after first layout (so size/density are
 *  known), hands it to Rust. The app's cdylib name comes from the manifest meta-data key
 *  "day.lib". A FragmentActivity so DayNavHost pages ride the androidx FragmentManager back
 *  stack — which hands system back (all API levels), predictive back gesture seeking (34+),
 *  and root back-to-home to the platform (docs/navigation.md). */
public class DayActivity extends androidx.fragment.app.FragmentActivity {
    @Override protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        String lib = "app";
        try {
            ActivityInfo info = getPackageManager().getActivityInfo(
                    getComponentName(), PackageManager.GET_META_DATA);
            if (info.metaData != null && info.metaData.getString("day.lib") != null) {
                lib = info.metaData.getString("day.lib");
            }
        } catch (Exception e) {
            android.util.Log.w("Day", "day.lib metadata lookup failed; using \"" + lib + "\"", e);
        }
        System.loadLibrary(lib);

        // DAY_THEME forcing is the LAUNCHER's job (day-cli sets the device night mode over adb
        // before `am start`), not this activity's: UiModeManager.setApplicationNightMode
        // persists per-app across restarts, so a forced run would poison the NEXT run's window
        // inflation with the old scheme — and since the manifest handles the uiMode config
        // change itself (no recreation), the already-inflated window could never re-theme.
        // With no app-level override the theme simply follows the system, coherently, from the
        // first frame.

        DayBridge.ctx = this;
        final DayFixed root = new DayFixed(this);
        // A focusable root gives "focus nowhere" a home (docs/focus.md): resigning a field
        // hands focus here instead of snapping to the first focusable view.
        root.setFocusableInTouchMode(true);
        // RTL locales (docs/localization): mirror native widget internals (text alignment,
        // slider fill, back affordances) by flipping the view hierarchy's direction. Day's own
        // absolute frames are direction-independent — the Rust layout engine mirrors those.
        String dayLocale = getIntent().getStringExtra("day.locale");
        if (dayLocale == null && getIntent().getExtras() != null) {
            dayLocale = getIntent().getExtras().getString("day.env.DAY_LOCALE");
        }
        if (dayLocale != null && isRtlLanguage(dayLocale)) {
            getWindow().getDecorView().setLayoutDirection(android.view.View.LAYOUT_DIRECTION_RTL);
            root.setLayoutDirection(android.view.View.LAYOUT_DIRECTION_RTL);
        }
        setContentView(root);
        final DisplayMetrics dm = getResources().getDisplayMetrics();
        final String autodrive = getIntent().getStringExtra("day.autodrive");
        final String locale = getIntent().getStringExtra("day.locale");
        StringBuilder blob = new StringBuilder();
        android.os.Bundle extras = getIntent().getExtras();
        if (extras != null) {
            for (String key : extras.keySet()) {
                if (key.startsWith("day.env.")) {
                    blob.append(key.substring(8)).append('=')
                        .append(extras.getString(key, "")).append('\n');
                }
            }
        }
        // Cold-start deep link (docs/navigation.md): the launch URI's host+path is the route.
        android.net.Uri data = getIntent().getData();
        if (data != null) {
            blob.append("DAY_DEEPLINK=").append(uriRoute(data)).append('\n');
        }
        final String envBlob = blob.toString();
        final DayActivity self = this;
        root.post(new Runnable() {
            public void run() {
                DayBridge.nativeStart(root, dm.density, root.getWidth(), root.getHeight(),
                        autodrive, locale, envBlob);
                // Native is ready now (docs/lifecycle.md). onStart/onResume already ran before this
                // post, so their events were dropped — synthesize the current active state.
                DayBridge.started = true;
                if (self.resumed) DayBridge.lifecycle(2); // DidBecomeActive
            }
        });
    }

    /** Whether the Activity is currently resumed (foreground + interactive). */
    private boolean resumed = false;

    // Activity lifecycle → day lifecycle phases (docs/lifecycle.md). Codes match day_spec::Lifecycle.
    @Override protected void onStart() { super.onStart(); DayBridge.lifecycle(4); }   // WillEnterForeground
    @Override protected void onResume() { super.onResume(); resumed = true; DayBridge.lifecycle(2); } // DidBecomeActive
    @Override protected void onPause() { DayBridge.lifecycle(3); resumed = false; super.onPause(); }   // WillResignActive
    @Override protected void onStop() { DayBridge.lifecycle(5); super.onStop(); }     // DidEnterBackground

    @Override protected void onDestroy() {
        // Only a real finish is a termination; a config-change recreation is not.
        if (isFinishing()) DayBridge.lifecycle(7); // WillTerminate
        super.onDestroy();
    }

    @Override public void onTrimMemory(int level) {
        super.onTrimMemory(level);
        DayBridge.lifecycle(6); // DidReceiveMemoryWarning
    }

    static String uriRoute(android.net.Uri uri) {
        String host = uri.getHost() == null ? "" : uri.getHost();
        String path = uri.getPath() == null ? "" : uri.getPath();
        return host + path;
    }

    /** App menu (docs/menus.md): the global menu maps to the app-bar overflow (⋮). Rebuilt whenever
     *  DayBridge.setAppMenu invalidates it. No spec → no overflow item shown. */
    /** Whether the language subtag of {@code locale} writes right-to-left. */
    private static boolean isRtlLanguage(String locale) {
        String lang = locale.replace('_', '-').split("-")[0].toLowerCase(java.util.Locale.ROOT);
        switch (lang) {
            case "ar": case "he": case "iw": case "fa": case "ur":
            case "ps": case "sd": case "ug": case "yi": case "dv": case "ku":
                return true;
            default:
                return false;
        }
    }

    @Override public boolean onCreateOptionsMenu(android.view.Menu menu) {
        if (DayBridge.appMenuSpec != null && !DayBridge.appMenuSpec.isEmpty()) {
            DayBridge.buildMenu(menu, DayBridge.appMenuSpec);
            return true;
        }
        return false;
    }

    /** Storage Access Framework results (docs/files.md) route back to DayBridge. */
    @Override protected void onActivityResult(int requestCode, int resultCode,
            android.content.Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        DayBridge.onFileResult(requestCode, resultCode, data);
    }

    /** Warm deep link (launchMode=singleTask): route to the running nav host. */
    @Override protected void onNewIntent(android.content.Intent intent) {
        super.onNewIntent(intent);
        android.net.Uri data = intent.getData();
        DayNavHost nav = DayNavHost.active;
        if (data != null && nav != null) {
            // kind 7 = deep link; the nav host piece handles Custom("deeplink").
            DayBridge.nativeOnEvent(nav.hostNode, 7, 0.0, uriRoute(data));
        }
    }
}
