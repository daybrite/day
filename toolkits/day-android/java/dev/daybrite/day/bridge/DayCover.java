// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

package dev.daybrite.day.bridge;

import android.view.MotionEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.animation.DecelerateInterpolator;
import android.widget.FrameLayout;

/** A fullscreen cover (docs/cover.md): an edge-to-edge shell attached to the activity's
 *  content root over everything else, holding a DayFixed content pane inset to the safe
 *  area (the same inset discipline as DayActivity's root). The shell is the Day handle:
 *  while unpresented it sits (zero-sized) wherever the Day tree put it; present() re-homes
 *  it to the content root with a slide-up, dismissCover() slides it out, reports
 *  K_COVER_HIDDEN, and detaches. System back is reported to Rust as NavBack (kind 5) while
 *  presented and not dismiss-disabled; Rust answers with the Dismiss patch.
 *
 *  A half sheet (`CoverProps::half`, docs/cover.md "Half sheet") is the same shell presented
 *  as a Material bottom sheet: a `BottomSheetBehavior` in a `CoordinatorLayout` over the
 *  content root sizes it to the bottom half of the window, animates it in and out, and lets
 *  the user pull it down; the window content above it stays on show and takes touches, since
 *  the host consumes nothing outside the sheet. */
public class DayCover extends FrameLayout {
    final DayFixed content;
    final long node;
    /** Presented as a half sheet (see the class comment). */
    final boolean half;
    boolean dismissDisabled;
    /** The half sheet's host and behavior, built on the first present. */
    private androidx.coordinatorlayout.widget.CoordinatorLayout sheetHost;
    private com.google.android.material.bottomsheet.BottomSheetBehavior<DayCover> sheet;
    /** The half sheet's slide in flight: what to run when it settles (null = nothing). */
    private Runnable sheetSettled;
    /** The handle area above the half sheet's content, in px. */
    private int handlePx;
    /** While true the shell belongs to the activity content root; day-tree re-parenting
     *  (z-order re-syncs) must leave it alone (see DayBridge.addChild/removeChild). */
    boolean presented;
    private androidx.activity.OnBackPressedCallback backCb;

    /** A modal surface must own touches its descendants do not handle (labels, spacers,
     *  padding and safe-area margins). An opaque FrameLayout alone is not a touch barrier:
     *  returning false on `ACTION_DOWN` lets the activity root try the sibling underneath the
     *  cover.
     *  Consume here, after normal child dispatch, so buttons, scrolling and canvas gestures
     *  keep their streams. Do not intercept children or make the shell a clickable a11y node. */
    @Override public boolean onTouchEvent(MotionEvent event) {
        return true;
    }

    public DayCover(android.content.Context ctx, final long node, boolean half) {
        super(ctx);
        this.node = node;
        this.half = half;
        // An opaque surface by default (the theme's window background): the shell overlays
        // the whole UI, and a transparent shell composites the presented app over the page
        // beneath it. An app-specified color (CoverPatch::Present) overrides this.
        android.util.TypedValue tv = new android.util.TypedValue();
        int bg = ctx.getTheme().resolveAttribute(android.R.attr.colorBackground, tv, true)
                ? tv.data : 0xFFFFFFFF;
        content = new DayFixed(ctx);
        if (half) {
            // The Material bottom sheet look: rounded top corners, a drag handle, a shadow.
            float density = ctx.getResources().getDisplayMetrics().density;
            float r = 28f * density;
            android.graphics.drawable.GradientDrawable shape =
                    new android.graphics.drawable.GradientDrawable();
            shape.setColor(bg);
            shape.setCornerRadii(new float[] { r, r, r, r, 0f, 0f, 0f, 0f });
            setBackground(shape);
            setClipToOutline(true);
            setElevation(8f * density);
            handlePx = Math.round(24f * density);
            View handle = new View(ctx);
            android.graphics.drawable.GradientDrawable pill =
                    new android.graphics.drawable.GradientDrawable();
            int onSurface = ctx.getTheme().resolveAttribute(
                    android.R.attr.textColorPrimary, tv, true) && tv.resourceId != 0
                    ? androidx.core.content.ContextCompat.getColor(ctx, tv.resourceId)
                    : 0xFF000000;
            pill.setColor((onSurface & 0x00FFFFFF) | 0x66000000);
            pill.setCornerRadius(2f * density);
            handle.setBackground(pill);
            FrameLayout.LayoutParams hlp = new FrameLayout.LayoutParams(
                    Math.round(32f * density), Math.round(4f * density));
            hlp.gravity = android.view.Gravity.CENTER_HORIZONTAL | android.view.Gravity.TOP;
            hlp.topMargin = Math.round(10f * density);
            addView(handle, hlp);
        } else {
            setBackgroundColor(bg);
        }
        FrameLayout.LayoutParams clp = new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT);
        clp.topMargin = handlePx;
        addView(content, clp);
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
                // A half sheet never reaches the top edge: its content starts under the handle.
                int top = half ? handlePx : bars.top;
                if (lp.leftMargin != bars.left || lp.topMargin != top
                        || lp.rightMargin != bars.right || lp.bottomMargin != bars.bottom) {
                    lp.leftMargin = bars.left;
                    lp.topMargin = top;
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

    /** Attach over everything and slide up. Idempotent while already presented.
     *
     *  Re-presenting a shell whose dismissal is still sliding reverses it: Rust reopens a
     *  keyed window that has not finished closing as the same window (windows.rs), and the
     *  slide picks up from where the view currently is rather than dropping it back to the
     *  bottom edge first. The out-slide's completion callback goes with it (`slide` drops
     *  the old animator's listeners before cancelling), so no K_COVER_HIDDEN follows. */
    void present(boolean dismissDisabled) {
        this.dismissDisabled = dismissDisabled;
        ViewGroup root = ((android.app.Activity) DayBridge.ctx)
                .findViewById(android.R.id.content);
        if (!presented) DayKeyboard.endEditing(root);
        if (half) {
            presentSheet(root);
            return;
        }
        boolean onScreen = getParent() == root && getVisibility() == View.VISIBLE;
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
        slide(onScreen ? getTranslationY() : h, 0f, null);
        if (backCb == null) {
            backCb = new androidx.activity.OnBackPressedCallback(!dismissDisabled) {
                @Override public void handleOnBackPressed() {
                    // Not popped natively: Rust decides and answers with the Dismiss patch.
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
        if (sheet != null) sheet.setHideable(!d);
    }

    /** The half sheet's present: a `BottomSheetBehavior` host over the content root (built
     *  once), the shell sized to the bottom half of the window, and the behavior's own slide
     *  from hidden to expanded. The host consumes no touch outside the sheet, so the window
     *  content above stays interactive. */
    private void presentSheet(ViewGroup root) {
        boolean fresh = sheetHost == null;
        if (fresh) {
            sheetHost = new androidx.coordinatorlayout.widget.CoordinatorLayout(getContext());
            sheet = new com.google.android.material.bottomsheet.BottomSheetBehavior<DayCover>();
            sheet.setFitToContents(true);
            sheet.setSkipCollapsed(true);
            sheet.setHideable(!dismissDisabled);
            sheet.setState(com.google.android.material.bottomsheet.BottomSheetBehavior.STATE_HIDDEN);
            final DayCover self = this;
            sheet.addBottomSheetCallback(
                    new com.google.android.material.bottomsheet.BottomSheetBehavior.BottomSheetCallback() {
                @Override public void onStateChanged(View v, int state) {
                    if (state == com.google.android.material.bottomsheet.BottomSheetBehavior.STATE_HIDDEN) {
                        if (sheetSettled != null) {
                            // Our own dismissal landed.
                            Runnable r = sheetSettled;
                            sheetSettled = null;
                            slidesInFlight = Math.max(0, slidesInFlight - 1);
                            r.run();
                        } else if (self.presented) {
                            // The user pulled it down: the back request every cover answers.
                            // Rust writes `None` and the Dismiss patch confirms the hide.
                            DayBridge.nativeOnEvent(node, DayBridge.K_NAV_BACK, 0.0, "");
                        }
                    } else if (state == com.google.android.material.bottomsheet.BottomSheetBehavior.STATE_EXPANDED
                            || state == com.google.android.material.bottomsheet.BottomSheetBehavior.STATE_COLLAPSED
                            || state == com.google.android.material.bottomsheet.BottomSheetBehavior.STATE_HALF_EXPANDED) {
                        if (sheetSettled != null) {
                            // A re-present reversed the dismissal: the settle is the present's.
                            sheetSettled = null;
                            slidesInFlight = Math.max(0, slidesInFlight - 1);
                        } else if (sheetPresenting) {
                            sheetPresenting = false;
                            slidesInFlight = Math.max(0, slidesInFlight - 1);
                        }
                    }
                }
                @Override public void onSlide(View v, float offset) {}
            });
            root.addView(sheetHost, new FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        }
        if (getParent() != sheetHost) {
            if (getParent() instanceof ViewGroup) {
                ((ViewGroup) getParent()).removeView(this);
            }
            androidx.coordinatorlayout.widget.CoordinatorLayout.LayoutParams lp =
                    new androidx.coordinatorlayout.widget.CoordinatorLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT, sheetHeight(root));
            lp.setBehavior(sheet);
            sheetHost.addView(this, lp);
        } else {
            ViewGroup.LayoutParams lp = getLayoutParams();
            int h = sheetHeight(root);
            if (lp.height != h) {
                lp.height = h;
                setLayoutParams(lp);
            }
        }
        presented = true;
        setVisibility(View.VISIBLE);
        requestApplyInsets();
        if (backCb == null) {
            backCb = new androidx.activity.OnBackPressedCallback(!dismissDisabled) {
                @Override public void handleOnBackPressed() {
                    DayBridge.nativeOnEvent(node, DayBridge.K_NAV_BACK, 0.0, "");
                }
            };
            ((androidx.activity.ComponentActivity) DayBridge.ctx)
                    .getOnBackPressedDispatcher().addCallback(backCb);
        }
        backCb.setEnabled(!dismissDisabled);
        sheet.setHideable(!dismissDisabled);
        final int expanded = com.google.android.material.bottomsheet.BottomSheetBehavior.STATE_EXPANDED;
        if (DayBridge.skipTransitions()) {
            sheetSettled = null;
            sheet.setState(expanded);
            return;
        }
        if (sheet.getState() != expanded && !sheetPresenting) {
            sheetPresenting = true;
            slidesInFlight++;
        }
        // Expanding before the first layout jumps there without a slide; after it, the
        // behavior animates from the hidden offset.
        if (fresh || !isLaidOut()) {
            post(new Runnable() {
                @Override public void run() { sheet.setState(expanded); }
            });
        } else {
            sheet.setState(expanded);
        }
    }

    /** A present's slide in flight (the behavior's own animation). */
    private boolean sheetPresenting;

    /** Half the window content height: the half sheet's own height. */
    private static int sheetHeight(ViewGroup root) {
        int h = root.getHeight() > 0 ? root.getHeight() : 2000;
        return h / 2;
    }

    /** Slide out, hide, and report K_COVER_HIDDEN so Rust can dispose the content.
     *
     *  The shell is hidden (View.GONE), never detached: the next present()'s content can
     *  include fragment hosts (a miniapp's nav stack), and a fragment commit resolves its
     *  container id against the attached hierarchy; a detached shell made every
     *  re-present throw "No view found for id" from FragmentStateManager. A `GONE` view
     *  neither lays out nor draws, so hiding costs nothing. */
    void dismissCover() {
        DayKeyboard.endEditing(this);
        if (backCb != null) {
            backCb.remove();
            backCb = null;
        }
        if (half) {
            dismissSheet();
            return;
        }
        ViewGroup p = (ViewGroup) getParent();
        float h = p != null ? p.getHeight() : Math.max(getHeight(), 2000);
        final DayCover self = this;
        slide(getTranslationY(), h, new Runnable() {
            @Override public void run() {
                self.presented = false;
                self.setVisibility(View.GONE);
                self.setTranslationY(0f);
                DayBridge.nativeOnEvent(node, DayBridge.K_COVER_HIDDEN, 0.0, "");
            }
        });
    }

    /** The half sheet's dismissal: the behavior slides it out (or it is already out, pulled
     *  down by the user), then the shell hides and reports K_COVER_HIDDEN like the fullscreen
     *  slide does. */
    private void dismissSheet() {
        final DayCover self = this;
        final Runnable done = new Runnable() {
            @Override public void run() {
                self.presented = false;
                self.setVisibility(View.GONE);
                DayBridge.nativeOnEvent(node, DayBridge.K_COVER_HIDDEN, 0.0, "");
            }
        };
        final int hidden = com.google.android.material.bottomsheet.BottomSheetBehavior.STATE_HIDDEN;
        if (sheet == null || sheet.getState() == hidden || DayBridge.skipTransitions()) {
            if (sheetPresenting) {
                sheetPresenting = false;
                slidesInFlight = Math.max(0, slidesInFlight - 1);
            }
            sheetSettled = null;
            if (sheet != null) {
                sheet.setHideable(true);
                sheet.setState(hidden);
            }
            done.run();
            return;
        }
        if (sheetPresenting) {
            sheetPresenting = false;
            slidesInFlight = Math.max(0, slidesInFlight - 1);
        }
        if (sheetSettled == null) slidesInFlight++;
        sheetSettled = done;
        sheet.setHideable(true);
        sheet.setState(hidden);
    }

    /** The one slide driver. A dedicated ValueAnimator (never the view's shared
     *  ViewPropertyAnimator, which any other animate() user can cancel) with the end
     *  callback in onAnimationEnd, invoked on both natural end and cancellation, so the
     *  terminal state (hidden + K_COVER_HIDDEN, or settled at 0) can never be lost. */
    private android.animation.ValueAnimator slideAnim;
    /** Cover slides in flight, all shells: dayscript's ui_idle gate (DayBridge.uiIdle). */
    static int slidesInFlight;

    private void slide(float from, float to, final Runnable done) {
        if (slideAnim != null) {
            slideAnim.removeAllListeners();
            slideAnim.cancel();
            slideAnim = null;
            slidesInFlight = Math.max(0, slidesInFlight - 1);
        }
        if (DayBridge.skipTransitions()) {
            setTranslationY(to);
            if (done != null) done.run();
            return;
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
