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

        // DAY_THEME=light|dark (a `day launch --env` passthrough, delivered as an intent
        // extra) forces the app's night mode — themed screenshot runs and local theme checks;
        // unset ⇒ follow the system. API 31+; a recreation triggered by the mode change reads
        // the same extra again, so the set is idempotent.
        String dayTheme = getIntent().getStringExtra("day.env.DAY_THEME");
        if (dayTheme != null && android.os.Build.VERSION.SDK_INT >= 31) {
            android.app.UiModeManager ui =
                    (android.app.UiModeManager) getSystemService(UI_MODE_SERVICE);
            if (ui != null) {
                if ("dark".equals(dayTheme)) {
                    ui.setApplicationNightMode(android.app.UiModeManager.MODE_NIGHT_YES);
                } else if ("light".equals(dayTheme)) {
                    ui.setApplicationNightMode(android.app.UiModeManager.MODE_NIGHT_NO);
                }
            }
        }

        DayBridge.ctx = this;
        final DayFixed root = new DayFixed(this);
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
