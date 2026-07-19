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
        // Safe-area insets (DESIGN §7.7). Edge-to-edge is forced on for apps targeting SDK 35+
        // (Android 15+): the window draws under the status and navigation bars, and on Android 16
        // there is no longer a way to opt back out (setDecorFitsSystemWindows(true) no longer
        // reinstates the fit). Day positions every piece by absolute frame from its root's top-left
        // and has no safe-area model, so its top row would draw beneath the status bar. Rather than
        // depend on the platform's version-dependent auto-fit, make Day the sole inset authority:
        // keep the window edge-to-edge on every version, then consume the system-bar insets
        // ourselves — hold the root in a wrapper and set the root's margins to the status/navigation
        // -bar (and display-cutout) insets, keeping all Day content inside the safe area.
        androidx.core.view.WindowCompat.setDecorFitsSystemWindows(getWindow(), false);
        final android.widget.FrameLayout wrapper = new android.widget.FrameLayout(this);
        wrapper.addView(root, new android.widget.FrameLayout.LayoutParams(
                android.view.ViewGroup.LayoutParams.MATCH_PARENT,
                android.view.ViewGroup.LayoutParams.MATCH_PARENT));
        setContentView(wrapper);
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
        // The insets listener does exactly one job: keep the root's margins equal to the
        // system-bar (+ cutout) insets. Everything downstream is size-driven — no launch or
        // relayout choreography lives here, so a late or repeated inset pass (second pass on
        // some devices, rotation, bar changes) is handled the same as the first.
        androidx.core.view.ViewCompat.setOnApplyWindowInsetsListener(wrapper,
                new androidx.core.view.OnApplyWindowInsetsListener() {
            @Override public androidx.core.view.WindowInsetsCompat onApplyWindowInsets(
                    android.view.View v, androidx.core.view.WindowInsetsCompat insets) {
                androidx.core.graphics.Insets bars = insets.getInsets(
                        androidx.core.view.WindowInsetsCompat.Type.systemBars()
                        | androidx.core.view.WindowInsetsCompat.Type.displayCutout());
                // Keyboard avoidance (docs/focus.md): consume the IME inset too, so a raised
                // keyboard shrinks the root exactly like a taller navigation bar would — the
                // resize rail relayouts Day, and the platform ScrollView then scrolls the
                // focused field back into view (its stock resized-with-focus behavior).
                androidx.core.graphics.Insets ime = insets.getInsets(
                        androidx.core.view.WindowInsetsCompat.Type.ime());
                int bottom = Math.max(bars.bottom, ime.bottom);
                android.widget.FrameLayout.LayoutParams lp =
                        (android.widget.FrameLayout.LayoutParams) root.getLayoutParams();
                if (lp.leftMargin != bars.left || lp.topMargin != bars.top
                        || lp.rightMargin != bars.right || lp.bottomMargin != bottom) {
                    lp.leftMargin = bars.left;
                    lp.topMargin = bars.top;
                    lp.rightMargin = bars.right;
                    lp.bottomMargin = bottom;
                    root.setLayoutParams(lp);
                }
                return androidx.core.view.WindowInsetsCompat.CONSUMED;
            }
        });
        // Size-driven start + resize: the root's FIRST laid-out size starts native (posted, so
        // the traversal has finished and getWidth/Height are settled); every later size change —
        // a second inset pass shrinking the root into the safe area, rotation, bar changes —
        // flows to native as a window-resize event and Day relayouts. Native never needs to know
        // where the size came from, which is what makes edge-to-edge handling automatic instead
        // of a launch-time snapshot.
        final Runnable start = new Runnable() {
            public void run() {
                DayBridge.nativeStart(root, dm.density, root.getWidth(), root.getHeight(),
                        autodrive, locale, envBlob);
                // Native is ready now (docs/lifecycle.md). onStart/onResume already ran before this
                // post, so their events were dropped — synthesize the current active state.
                DayBridge.started = true;
                if (self.resumed) DayBridge.lifecycle(2); // DidBecomeActive
            }
        };
        final boolean[] launched = { false };
        root.sizeListener = new DayFixed.SizeListener() {
            @Override public void onSize(int w, int h) {
                if (!launched[0]) {
                    launched[0] = true;
                    root.post(start);
                } else {
                    DayBridge.resized(w, h);
                }
            }
        };
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
