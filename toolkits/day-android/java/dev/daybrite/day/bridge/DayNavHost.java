package dev.daybrite.day.bridge;

import android.content.Context;
import android.transition.Transition;
import android.transition.TransitionListenerAdapter;
import android.transition.TransitionManager;
import android.util.TypedValue;
import android.view.View;
import android.view.ViewGroup;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import java.util.ArrayList;

import com.google.android.material.appbar.AppBarLayout;
import com.google.android.material.appbar.MaterialToolbar;
import com.google.android.material.transition.platform.MaterialSharedAxis;

/**
 * Navigation host (docs/navigation.md): an M3 app bar ({@link AppBarLayout} hosting a
 * {@link MaterialToolbar} — title + up arrow) over a page {@link FrameLayout}. Pages arrive via
 * {@link #add}; presentation changes are driven by the Rust side's NavPatch calls and animated
 * with the Material motion system: {@link MaterialSharedAxis} X, forward on push and backward on
 * pop, via {@link TransitionManager} (the platform variant — no fragments involved). The system
 * back key and the toolbar up arrow both dispatch event kind 5 (NavBack) — Rust then pops the
 * route stack and calls {@link #pop} + removes the page.
 */
public class DayNavHost extends LinearLayout {
    /** v1: nav is app-root only, so a single active host suffices (back-key routing). */
    static DayNavHost active;

    final MaterialToolbar toolbar;
    final FrameLayout pages;
    final long hostNode;
    final String rootTitle;
    private final ArrayList<View> stack = new ArrayList<>();
    private final ArrayList<String> titles = new ArrayList<>();
    /** The page currently transitioning out under a pop — its FrameLayout detach is deferred to
     *  the transition's end so `removePage` (called by Rust right after the Popped patch) doesn't
     *  cut the exit animation short. */
    private View poppingView;

    public DayNavHost(Context ctx, long hostNode, String title) {
        super(ctx);
        setOrientation(VERTICAL);
        this.hostNode = hostNode;
        this.rootTitle = title;

        toolbar = new MaterialToolbar(ctx);
        toolbar.setTitle(title);
        final long node = hostNode;
        toolbar.setNavigationOnClickListener(new OnClickListener() {
            @Override public void onClick(View v) {
                DayBridge.nativeOnEvent(node, 5, 0.0, null);
            }
        });
        AppBarLayout appBar = new AppBarLayout(ctx);
        appBar.addView(toolbar, new AppBarLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        addView(appBar, new LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT,
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
        // pop()'s shared-axis exit transition owns the FrameLayout detach for the popped page.
        if (page == poppingView) return;
        pages.removeView(page);
    }

    /** Present the most recently added page (Pushed patch) with the Material forward motion:
     *  shared-axis X — the new page slides/fades in from the trailing edge while the predecessor
     *  recedes out the leading edge. */
    void push(String title) {
        int n = stack.size();
        if (n < 2) return;
        final View top = stack.get(n - 1);
        final View prev = stack.get(n - 2);
        titles.add(title);
        TransitionManager.beginDelayedTransition(pages, new MaterialSharedAxis(
                MaterialSharedAxis.X, /* forward= */ true));
        top.bringToFront();
        top.setVisibility(View.VISIBLE);
        prev.setVisibility(View.GONE);
        toolbar.setTitle(title);
        showUpArrow(true);
    }

    /** Dismiss the top page (Popped patch) with the Material backward motion: shared-axis X
     *  reversed — the page exits the trailing edge, the predecessor returns from the leading edge.
     *  Rust calls removePage on the popped page right after; its detach is deferred to the
     *  transition's end so the exit stays visible. */
    void pop() {
        int n = stack.size();
        if (n < 2) return;
        final View top = stack.get(n - 1);
        final View prev = stack.get(n - 2);
        poppingView = top;
        MaterialSharedAxis axis = new MaterialSharedAxis(MaterialSharedAxis.X, /* forward= */ false);
        axis.addListener(new TransitionListenerAdapter() {
            @Override public void onTransitionEnd(Transition t) {
                if (top.getParent() == pages) pages.removeView(top);
                if (poppingView == top) poppingView = null;
            }
        });
        TransitionManager.beginDelayedTransition(pages, axis);
        top.setVisibility(View.GONE);
        prev.setVisibility(View.VISIBLE);
        if (!titles.isEmpty()) titles.remove(titles.size() - 1);
        toolbar.setTitle(titles.isEmpty() ? rootTitle : titles.get(titles.size() - 1));
        showUpArrow(!titles.isEmpty());
    }

    private void showUpArrow(boolean show) {
        if (show) {
            // The M3 (AppCompat-based) theme sets the appcompat attr; fall back to the framework's.
            TypedValue tv = new TypedValue();
            if (!getContext().getTheme().resolveAttribute(
                    androidx.appcompat.R.attr.homeAsUpIndicator, tv, true)) {
                getContext().getTheme().resolveAttribute(
                        android.R.attr.homeAsUpIndicator, tv, true);
            }
            toolbar.setNavigationIcon(tv.resourceId);
        } else {
            toolbar.setNavigationIcon(null);
        }
    }
}
