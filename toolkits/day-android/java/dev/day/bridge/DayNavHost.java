package dev.day.bridge;

import android.content.Context;
import android.util.TypedValue;
import android.view.View;
import android.view.ViewGroup;
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
        pages.removeView(page);
    }

    /** Present the most recently added page (Pushed patch). */
    void push(String title) {
        int n = stack.size();
        if (n < 2) return;
        final View top = stack.get(n - 1);
        final View prev = stack.get(n - 2);
        titles.add(title);
        top.setVisibility(View.VISIBLE);
        top.setTranslationX(pages.getWidth());
        top.animate().translationX(0f).setDuration(220)
                .withEndAction(new Runnable() {
                    @Override public void run() {
                        prev.setVisibility(View.GONE);
                    }
                }).start();
        toolbar.setTitle(title);
        showUpArrow(true);
    }

    /** Reveal the predecessor (Popped patch); Rust removes the top page right after. */
    void pop() {
        int n = stack.size();
        if (n < 2) return;
        View prev = stack.get(n - 2);
        prev.setVisibility(View.VISIBLE);
        prev.setTranslationX(0f);
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
