package dev.daybrite.day.bridge;

import android.view.View;
import android.view.ViewGroup;
import android.view.animation.DecelerateInterpolator;
import android.widget.FrameLayout;

/** A fullscreen cover (docs/cover.md): an edge-to-edge shell attached to the activity's
 *  content root over everything else, holding a DayFixed content pane inset to the safe
 *  area (the same inset discipline as DayActivity's root). The shell is the Day handle:
 *  while unpresented it sits (zero-sized) wherever the Day tree put it; present() re-homes
 *  it to the content root with a slide-up, dismissCover() slides it out, reports
 *  "cover-hidden", and detaches. System back is reported to Rust as NavBack (kind 5) while
 *  presented and not dismiss-disabled — Rust answers with the Dismiss patch. */
public class DayCover extends FrameLayout {
    final DayFixed content;
    final long node;
    boolean dismissDisabled;
    private androidx.activity.OnBackPressedCallback backCb;

    public DayCover(android.content.Context ctx, final long node) {
        super(ctx);
        this.node = node;
        content = new DayFixed(ctx);
        addView(content, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        // Keep the content pane inside the safe area (status/navigation bars + cutout),
        // exactly like the activity root. In immersive mode (deferred system gestures) the
        // bar insets collapse and the game becomes truly fullscreen.
        androidx.core.view.ViewCompat.setOnApplyWindowInsetsListener(this,
                new androidx.core.view.OnApplyWindowInsetsListener() {
            @Override public androidx.core.view.WindowInsetsCompat onApplyWindowInsets(
                    View v, androidx.core.view.WindowInsetsCompat insets) {
                androidx.core.graphics.Insets bars = insets.getInsets(
                        androidx.core.view.WindowInsetsCompat.Type.systemBars()
                        | androidx.core.view.WindowInsetsCompat.Type.displayCutout());
                FrameLayout.LayoutParams lp = (FrameLayout.LayoutParams) content.getLayoutParams();
                if (lp.leftMargin != bars.left || lp.topMargin != bars.top
                        || lp.rightMargin != bars.right || lp.bottomMargin != bars.bottom) {
                    lp.leftMargin = bars.left;
                    lp.topMargin = bars.top;
                    lp.rightMargin = bars.right;
                    lp.bottomMargin = bars.bottom;
                    content.setLayoutParams(lp);
                }
                return androidx.core.view.WindowInsetsCompat.CONSUMED;
            }
        });
        // The content pane's laid-out size is the cover's Day layout size (kind 6 =
        // FrameChanged, "w,h" px; Rust divides by density).
        content.addOnLayoutChangeListener(new View.OnLayoutChangeListener() {
            @Override public void onLayoutChange(View v, int l, int t, int r, int b,
                    int ol, int ot, int or2, int ob) {
                int w = r - l, h = b - t;
                if (w != or2 - ol || h != ob - ot) {
                    DayBridge.nativeOnEvent(node, DayBridge.K_FRAME_CHANGED, 0.0, w + "," + h);
                }
            }
        });
    }

    /** Attach over everything and slide up. Idempotent while already presented. */
    void present(boolean dismissDisabled) {
        this.dismissDisabled = dismissDisabled;
        ViewGroup root = ((android.app.Activity) DayBridge.ctx)
                .findViewById(android.R.id.content);
        if (getParent() == root) {
            return;
        }
        if (getParent() instanceof ViewGroup) {
            ((ViewGroup) getParent()).removeView(this);
        }
        root.addView(this, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        requestApplyInsets();
        int h = root.getHeight() > 0 ? root.getHeight() : 2000;
        setTranslationY(h);
        animate().translationY(0f).setDuration(300)
                .setInterpolator(new DecelerateInterpolator()).start();
        if (backCb == null) {
            backCb = new androidx.activity.OnBackPressedCallback(!dismissDisabled) {
                @Override public void handleOnBackPressed() {
                    // Not popped natively — Rust decides and answers with the Dismiss patch.
                    DayBridge.nativeOnEvent(node, DayBridge.K_NAV_BACK, 0.0, "");
                }
            };
            ((androidx.activity.ComponentActivity) DayBridge.ctx)
                    .getOnBackPressedDispatcher().addCallback(backCb);
        }
        backCb.setEnabled(!dismissDisabled);
    }

    void setDismissDisabled(boolean d) {
        dismissDisabled = d;
        if (backCb != null) backCb.setEnabled(!d);
    }

    /** Slide out, detach, and report "cover-hidden" so Rust can dispose the content. */
    void dismissCover() {
        if (backCb != null) {
            backCb.remove();
            backCb = null;
        }
        final ViewGroup p = (ViewGroup) getParent();
        float h = p != null ? p.getHeight() : Math.max(getHeight(), 2000);
        final DayCover self = this;
        animate().translationY(h).setDuration(250).withEndAction(new Runnable() {
            @Override public void run() {
                if (p != null) p.removeView(self);
                self.setTranslationY(0f);
                DayBridge.nativeOnEvent(node, DayBridge.K_CUSTOM, 0.0, "cover-hidden");
            }
        }).start();
    }
}
