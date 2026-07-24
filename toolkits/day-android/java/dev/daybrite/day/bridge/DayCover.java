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
    /** While true the shell belongs to the activity content root; day-tree re-parenting
     *  (z-order re-syncs) must leave it alone (see DayBridge.addChild/removeChild). */
    boolean presented;
    private androidx.activity.OnBackPressedCallback backCb;

    public DayCover(android.content.Context ctx, final long node) {
        super(ctx);
        this.node = node;
        // An OPAQUE surface by default (the theme's window background): the shell overlays
        // the whole UI, and a transparent shell composites the presented app over the page
        // beneath it. An app-specified color (CoverPatch::Present) overrides this.
        android.util.TypedValue tv = new android.util.TypedValue();
        if (ctx.getTheme().resolveAttribute(android.R.attr.colorBackground, tv, true)) {
            setBackgroundColor(tv.data);
        } else {
            setBackgroundColor(0xFFFFFFFF);
        }
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
        if (getParent() != root) {
            if (getParent() instanceof ViewGroup) {
                ((ViewGroup) getParent()).removeView(this);
            }
            root.addView(this, new FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        }
        presented = true;
        setVisibility(View.VISIBLE);
        requestApplyInsets();
        int h = root.getHeight() > 0 ? root.getHeight() : 2000;
        slide(h, 0f, null);
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

    /** Slide out, hide, and report "cover-hidden" so Rust can dispose the content.
     *
     *  The shell is HIDDEN (View.GONE), never detached: the next present()'s content can
     *  include fragment hosts (a miniapp's nav stack), and a fragment commit resolves its
     *  container id against the ATTACHED hierarchy — a detached shell made every
     *  re-present throw "No view found for id" from FragmentStateManager. A GONE view
     *  neither lays out nor draws, so hiding costs nothing. */
    void dismissCover() {
        if (backCb != null) {
            backCb.remove();
            backCb = null;
        }
        ViewGroup p = (ViewGroup) getParent();
        float h = p != null ? p.getHeight() : Math.max(getHeight(), 2000);
        final DayCover self = this;
        slide(getTranslationY(), h, new Runnable() {
            @Override public void run() {
                self.presented = false;
                self.setVisibility(View.GONE);
                self.setTranslationY(0f);
                DayBridge.nativeOnEvent(node, DayBridge.K_CUSTOM, 0.0, "cover-hidden");
            }
        });
    }

    /** The one slide driver. A dedicated ValueAnimator (never the view's shared
     *  ViewPropertyAnimator, which any other animate() user can cancel) with the end
     *  callback in onAnimationEnd — invoked on BOTH natural end and cancellation, so the
     *  terminal state (hidden + "cover-hidden", or settled at 0) can never be lost. */
    private android.animation.ValueAnimator slideAnim;
    /** Cover slides in flight, all shells — dayscript's ui_idle gate (DayBridge.uiIdle). */
    static int slidesInFlight;

    private void slide(float from, float to, final Runnable done) {
        if (slideAnim != null) {
            slideAnim.removeAllListeners();
            slideAnim.cancel();
            slideAnim = null;
            slidesInFlight = Math.max(0, slidesInFlight - 1);
        }
        setTranslationY(from);
        final android.animation.ValueAnimator a =
                android.animation.ValueAnimator.ofFloat(from, to);
        a.setDuration(250);
        a.setInterpolator(new DecelerateInterpolator());
        a.addUpdateListener(new android.animation.ValueAnimator.AnimatorUpdateListener() {
            @Override public void onAnimationUpdate(android.animation.ValueAnimator v) {
                setTranslationY((Float) v.getAnimatedValue());
            }
        });
        final float end = to;
        a.addListener(new android.animation.AnimatorListenerAdapter() {
            @Override public void onAnimationEnd(android.animation.Animator anim) {
                slidesInFlight = Math.max(0, slidesInFlight - 1);
                setTranslationY(end);
                if (slideAnim == a) slideAnim = null;
                if (done != null) done.run();
            }
        });
        slideAnim = a;
        slidesInFlight++;
        a.start();
    }
}
