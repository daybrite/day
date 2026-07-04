package dev.daybrite.day.bridge;

import android.app.Activity;
import android.content.pm.ActivityInfo;
import android.content.pm.PackageManager;
import android.os.Bundle;
import android.util.DisplayMetrics;

/** The host Activity (fragment/VC hosting arrives with navigation, §10.5): creates the root
 *  DayFixed and, after first layout (so size/density are known), hands it to Rust. The app's
 *  cdylib name comes from the manifest meta-data key "day.lib". */
public class DayActivity extends Activity {
    @Override protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        String lib = "app";
        try {
            ActivityInfo info = getPackageManager().getActivityInfo(
                    getComponentName(), PackageManager.GET_META_DATA);
            if (info.metaData != null && info.metaData.getString("day.lib") != null) {
                lib = info.metaData.getString("day.lib");
            }
        } catch (Exception ignored) {}
        System.loadLibrary(lib);

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
        root.post(new Runnable() {
            public void run() {
                DayBridge.nativeStart(root, dm.density, root.getWidth(), root.getHeight(),
                        autodrive, locale, envBlob);
            }
        });
    }

    static String uriRoute(android.net.Uri uri) {
        String host = uri.getHost() == null ? "" : uri.getHost();
        String path = uri.getPath() == null ? "" : uri.getPath();
        return host + path;
    }

    /** System back: pop the nav stack when there is one; otherwise leave the app. */
    @Override public void onBackPressed() {
        DayNavHost nav = DayNavHost.active;
        if (nav != null && nav.depth() > 0) {
            DayBridge.nativeOnEvent(nav.hostNode, 5, 0.0, null); // kind 5 = NavBack
        } else {
            super.onBackPressed();
        }
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
