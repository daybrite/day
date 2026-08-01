package dev.daybrite.day.bridge;

import android.os.Bundle;
import android.util.DisplayMetrics;

/** A secondary day window (docs/windows.md): a document-style Activity hosting one day
 *  window root. Launched by DayBridge.openWindow with the day node id and title as extras
 *  (NEW_DOCUMENT | MULTIPLE_TASK — its own recents entry; side-by-side in split-screen /
 *  freeform / desktop windowing). The cdylib is already loaded and native already running —
 *  this activity only builds a root, completes the pending open, and reports its lifecycle
 *  to the window's day root node. */
public class DayWindowActivity extends androidx.fragment.app.FragmentActivity {
    /** Live secondary windows by day node id (close/focus/title lookups). */
    static final java.util.Map<Long, DayWindowActivity> ACTIVE =
            new java.util.HashMap<Long, DayWindowActivity>();

    long node;
    private boolean started = false;

    @Override protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        node = getIntent().getLongExtra("day.node", 0);
        String title = getIntent().getStringExtra("day.title");
        if (node == 0 || !DayBridge.started) {
            // A recents-restored document from a previous process: nothing to mount.
            finish();
            return;
        }
        if (title != null && !title.isEmpty()) setTitle(title);
        ACTIVE.put(node, this);

        final DayFixed root = new DayFixed(this);
        root.setFocusableInTouchMode(true);
        // Same safe-area discipline as the primary (DayActivity): edge-to-edge window,
        // margins carry the system-bar insets, IME insets ride the resize rail.
        androidx.core.view.WindowCompat.setDecorFitsSystemWindows(getWindow(), false);
        final android.widget.FrameLayout wrapper = new android.widget.FrameLayout(this);
        wrapper.addView(root, new android.widget.FrameLayout.LayoutParams(
                android.view.ViewGroup.LayoutParams.MATCH_PARENT,
                android.view.ViewGroup.LayoutParams.MATCH_PARENT));
        setContentView(wrapper);
        androidx.core.view.ViewCompat.setOnApplyWindowInsetsListener(wrapper,
                new androidx.core.view.OnApplyWindowInsetsListener() {
            @Override public androidx.core.view.WindowInsetsCompat onApplyWindowInsets(
                    android.view.View v, androidx.core.view.WindowInsetsCompat insets) {
                androidx.core.graphics.Insets bars = insets.getInsets(
                        androidx.core.view.WindowInsetsCompat.Type.systemBars()
                        | androidx.core.view.WindowInsetsCompat.Type.displayCutout());
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
        final DisplayMetrics dm = getResources().getDisplayMetrics();
        root.sizeListener = new DayFixed.SizeListener() {
            @Override public void onSize(int w, int h) {
                if (!started) {
                    started = true;
                    root.post(new Runnable() { public void run() {
                        // Completes the pending day::open_window; false = closed before
                        // we connected — drop this activity again.
                        if (!DayBridge.nativeStartWindow(root, node, dm.density,
                                root.getWidth(), root.getHeight())) {
                            finish();
                        }
                    }});
                } else {
                    DayBridge.nativeOnEvent(node, DayBridge.K_WINDOW_RESIZED, 0, w + "," + h);
                }
            }
        };
    }

    // Per-window focus (docs/windows.md); app-level day lifecycle stays the primary's.
    @Override protected void onResume() {
        super.onResume();
        DayBridge.ctx = this;
        if (node != 0 && DayBridge.started) {
            DayBridge.nativeOnEvent(node, DayBridge.K_WINDOW_FOCUSED, 1, null);
        }
    }
    @Override protected void onPause() {
        if (node != 0 && DayBridge.started) {
            DayBridge.nativeOnEvent(node, DayBridge.K_WINDOW_FOCUSED, 0, null);
        }
        super.onPause();
    }

    @Override protected void onDestroy() {
        if (node != 0) {
            ACTIVE.remove(node);
            // A real close (back gesture, recents swipe, closeWindow): confirm to day,
            // which tears the subtree down. A config-change recreation is NOT a close.
            if (isFinishing() && DayBridge.started) {
                DayBridge.nativeOnEvent(node, DayBridge.K_WINDOW_CLOSED, 0, null);
            }
        }
        super.onDestroy();
    }
}
