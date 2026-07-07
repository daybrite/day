package dev.daybrite.day.bridge;

import android.content.Context;
import android.util.TypedValue;
import android.view.View;
import android.view.ViewGroup;
import android.view.animation.DecelerateInterpolator;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.Toolbar;
import java.util.ArrayList;

/**
 * Navigation host (docs/navigation.md): a framework {@link Toolbar} (title + up arrow)
 * over a page {@link FrameLayout}. Pages arrive via {@link #add}; presentation changes
 * (slide-in push, instant pop) are driven by the Rust side's NavPatch calls. The system
 * back key and the toolbar up arrow both dispatch event kind 5 (NavBack) — Rust then
 * pops the route stack and calls {@link #pop} + removes the page.
 */
public class DayNavHost extends LinearLayout {
    /** v1: nav is app-root only, so a single active host suffices (back-key routing). */
    static DayNavHost active;

    final Toolbar toolbar;
    final FrameLayout pages;
    final long hostNode;
    final String rootTitle;
    private final ArrayList<View> stack = new ArrayList<>();
    private final ArrayList<String> titles = new ArrayList<>();
    /** The page currently sliding out under a pop — its FrameLayout detach is deferred to the
     *  animation's end so `removePage` (called by Rust right after the Popped patch) doesn't cut it. */
    private View poppingView;
    private static final int NAV_ANIM_MS = 260;
    private static final float PARALLAX = 0.25f;

    public DayNavHost(Context ctx, long hostNode, String title) {
        super(ctx);
        setOrientation(VERTICAL);
        this.hostNode = hostNode;
        this.rootTitle = title;

        toolbar = new Toolbar(ctx);
        toolbar.setTitle(title);
        final long node = hostNode;
        toolbar.setNavigationOnClickListener(new OnClickListener() {
            @Override public void onClick(View v) {
                DayBridge.nativeOnEvent(node, 5, 0.0, null);
            }
        });
        addView(toolbar, new LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT));

        pages = new FrameLayout(ctx);
        addView(pages, new LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f));
        active = this;
    }

    int depth() {
        return titles.size();
    }

    void add(View page) {
        pages.addView(page, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        stack.add(page);
        // Later pages stay hidden until push() presents them (patch order: add, then push).
        if (stack.size() > 1) page.setVisibility(View.GONE);
    }

    void removePage(View page) {
        stack.remove(page);
        // pop()'s slide-out animation owns the FrameLayout detach for the page being popped.
        if (page == poppingView) return;
        pages.removeView(page);
    }

    /** Present the most recently added page (Pushed patch): it slides in from the right while the
     *  predecessor eases partially left (parallax), then hides once covered. */
    void push(String title) {
        int n = stack.size();
        if (n < 2) return;
        final View top = stack.get(n - 1);
        final View prev = stack.get(n - 2);
        titles.add(title);
        top.setVisibility(View.VISIBLE);
        top.bringToFront();
        top.setTranslationX(pages.getWidth());
        top.animate().translationX(0f).setDuration(NAV_ANIM_MS)
                .setInterpolator(new DecelerateInterpolator()).start();
        prev.animate().translationX(-pages.getWidth() * PARALLAX).setDuration(NAV_ANIM_MS)
                .setInterpolator(new DecelerateInterpolator())
                .withEndAction(new Runnable() {
                    @Override public void run() {
                        prev.setVisibility(View.GONE);
                        prev.setTranslationX(0f);
                    }
                }).start();
        toolbar.setTitle(title);
        showUpArrow(true);
    }

    /** Slide the top page out to the right, revealing the predecessor easing in from the left
     *  (Popped patch). Rust calls removePage on the popped page right after; its detach is deferred
     *  to this animation's end so the slide-out is visible. */
    void pop() {
        int n = stack.size();
        if (n < 2) return;
        final View top = stack.get(n - 1);
        final View prev = stack.get(n - 2);
        poppingView = top;
        prev.setVisibility(View.VISIBLE);
        prev.setTranslationX(-pages.getWidth() * PARALLAX);
        prev.animate().translationX(0f).setDuration(NAV_ANIM_MS)
                .setInterpolator(new DecelerateInterpolator()).start();
        top.bringToFront();
        top.animate().translationX(pages.getWidth()).setDuration(NAV_ANIM_MS)
                .setInterpolator(new DecelerateInterpolator())
                .withEndAction(new Runnable() {
                    @Override public void run() {
                        pages.removeView(top);
                        top.setTranslationX(0f);
                        if (poppingView == top) poppingView = null;
                    }
                }).start();
        if (!titles.isEmpty()) titles.remove(titles.size() - 1);
        toolbar.setTitle(titles.isEmpty() ? rootTitle : titles.get(titles.size() - 1));
        showUpArrow(!titles.isEmpty());
    }

    private void showUpArrow(boolean show) {
        if (show) {
            TypedValue tv = new TypedValue();
            getContext().getTheme().resolveAttribute(
                    android.R.attr.homeAsUpIndicator, tv, true);
            toolbar.setNavigationIcon(tv.resourceId);
        } else {
            toolbar.setNavigationIcon(null);
        }
    }
}
