package dev.daybrite.day.bridge;

import android.content.Context;
import android.graphics.drawable.ShapeDrawable;
import android.graphics.drawable.shapes.OvalShape;
import android.view.MenuItem;
import android.view.View;
import android.view.ViewGroup;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import java.util.ArrayList;

import com.google.android.material.bottomnavigation.BottomNavigationView;
import com.google.android.material.navigation.NavigationBarView;

/**
 * Tabs host (docs/tabs.md): a page {@link FrameLayout} over an M3
 * {@link BottomNavigationView} — a bottom tab bar, so tabs look and act like the iOS
 * {@code UITabBarController} mapping. Day's tab API is title-only, so each item carries a small
 * dot glyph (the navigation bar reserves icon space; the theme tints it with the item state).
 * Tapping an item dispatches event kind 4 (SelectionChanged); the Rust route controller owns
 * day's state and drives selection back via {@link #select}, which is guarded against echoing a
 * synthetic SelectionChanged back to Rust. All tabs stay resident so each keeps its own state.
 */
public class DayTabs extends LinearLayout {
    final long hostNode;
    private final BottomNavigationView bar;
    private final FrameLayout pages;
    private final ArrayList<View> tabViews = new ArrayList<>();
    private int selected;
    /** True while select() applies a programmatic selection (suppresses the item listener). */
    private boolean syncing;

    public DayTabs(Context ctx, long hostNode, int initial) {
        super(ctx);
        setOrientation(VERTICAL);
        this.hostNode = hostNode;
        this.selected = Math.max(0, initial);

        // iOS ordering: content on top, tab bar at the bottom.
        pages = new FrameLayout(ctx);
        addView(pages, new LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f));

        bar = new BottomNavigationView(ctx);
        bar.setLabelVisibilityMode(NavigationBarView.LABEL_VISIBILITY_LABELED);
        bar.setItemIconSize(dp(10)); // the dot glyph (title-only API); see class doc
        bar.setOnItemSelectedListener(new NavigationBarView.OnItemSelectedListener() {
            @Override public boolean onNavigationItemSelected(MenuItem item) {
                int index = item.getItemId();
                showPage(index);
                if (!syncing) {
                    DayTabs.this.selected = index;
                    DayBridge.nativeOnEvent(hostNode, 4, (double) index, null); // 4 = SelectionChanged
                }
                return true;
            }
        });
        addView(bar, new LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT));
    }

    /** Append a tab (in insertion order) carrying `page` under the label `title`. */
    void addTab(View page, String title) {
        final int index = tabViews.size();
        // The bottom bar caps its item count (5, like the iOS tab bar). Extra pages stay resident
        // and reachable programmatically (routes/deep links) but get no bar item.
        if (index < bar.getMaxItemCount()) {
            MenuItem item = bar.getMenu().add(0, index, index, title == null ? "" : title);
            ShapeDrawable dot = new ShapeDrawable(new OvalShape()); // tinted by itemIconTintList
            dot.setIntrinsicWidth(dp(10));
            dot.setIntrinsicHeight(dp(10));
            item.setIcon(dot);
        } else {
            android.util.Log.w("Day", "tabs: item " + index + " (\"" + title + "\") exceeds the "
                    + "bottom bar's max of " + bar.getMaxItemCount() + "; no bar item added");
        }

        pages.addView(page, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        tabViews.add(page);
        page.setVisibility(index == selected ? View.VISIBLE : View.GONE);
        if (index == selected) select(selected); // sync the bar once the initial tab arrives
    }

    /** Show tab `index` (from a programmatic TabsPatch::Selected), without echoing to Rust. */
    void select(int index) {
        if (index < 0 || index >= tabViews.size()) return;
        selected = index;
        syncing = true;
        try {
            bar.setSelectedItemId(index); // fires the item listener; `syncing` mutes the echo
        } finally {
            syncing = false;
        }
        showPage(index);
    }

    private void showPage(int index) {
        for (int i = 0; i < tabViews.size(); i++) {
            tabViews.get(i).setVisibility(i == index ? View.VISIBLE : View.GONE);
        }
    }

    private int dp(int v) {
        return (int) (v * getResources().getDisplayMetrics().density);
    }
}
