// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

package dev.daybrite.day.bridge;

import android.content.Context;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.ShapeDrawable;
import android.graphics.drawable.shapes.OvalShape;
import android.view.Menu;
import android.view.MenuItem;
import android.view.View;
import android.view.ViewGroup;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import java.util.ArrayList;

import com.google.android.material.bottomnavigation.BottomNavigationView;
import com.google.android.material.navigation.NavigationBarView;
import com.google.android.material.navigation.NavigationView;
import com.google.android.material.navigationrail.NavigationRailView;

/**
 * The navigation suite (docs/navigation.md): resident pages in a {@link FrameLayout}, with the
 * destination chrome drawn in whichever form Material asks for at the current width — a
 * {@link BottomNavigationView} when compact, a {@link NavigationRailView} at medium, a permanent
 * {@link NavigationView} drawer when expanded. It is the View-system equivalent of Compose's
 * {@code NavigationSuiteScaffold}, and the counterpart of what UIKit's
 * {@code UITabBarController.Mode.tabSidebar} does on iOS.
 *
 * <p>Two things follow from being one container rather than three. The host reports {@code Tabs}
 * once and keeps it at every width — the chrome changes, the presentation does not — which is why
 * day-core keeps the pages resident and drives them with {@code NavPatch::Select} instead of
 * push/pop. And the CHROME is the row list: the bar's items come from the host's nav menu through
 * {@link #setRows}, and a tap reports against that menu's node, so a tab tap and a sidebar row
 * click are one event to everything above this backend.
 */
public class DayTabs extends LinearLayout {
    /** Chrome forms, in window-size-class order (M3: compact &lt; 600dp &lt;= medium &lt; 840dp). */
    private static final int FORM_BAR = 0, FORM_RAIL = 1, FORM_DRAWER = 2;
    /** M3 window size class breakpoints, in dp. */
    private static final int MEDIUM_MIN_DP = 600, EXPANDED_MIN_DP = 840;
    /** The permanent drawer's width — M3's standard navigation drawer. */
    private static final int DRAWER_WIDTH_DP = 280;

    final long hostNode;
    /** Who a tap reports against: the nav menu's node once {@link #setRows} has run. */
    private long menuNode;
    private final FrameLayout pages;
    private final ArrayList<View> pageViews = new ArrayList<>();
    private final ArrayList<String> titles = new ArrayList<>();
    private final ArrayList<String> iconNames = new ArrayList<>();
    private int selected;
    /** True while select() applies a programmatic selection (suppresses the item listener). */
    private boolean syncing;
    private int form = -1;
    private View chrome;

    public DayTabs(Context ctx, long hostNode, int initial) {
        super(ctx);
        this.hostNode = hostNode;
        this.menuNode = hostNode;
        this.selected = Math.max(0, initial);
        pages = new FrameLayout(ctx);
        // A form is applied on the first measure, when there is a width to judge. Starting at the
        // compact one means the first frame is never chrome-less on a phone, which is most of them.
        applyForm(FORM_BAR);
    }

    /**
     * The destination rows — titles and bundled icon NAMES, unit-separator joined and index
     * aligned, plus the node a tap reports against. This is the host's nav menu handing its rows
     * to the chrome that will draw them.
     */
    void setRows(String joinedTitles, String joinedIcons, long menuNode) {
        titles.clear();
        iconNames.clear();
        for (String t : split(joinedTitles)) {
            titles.add(t);
        }
        for (String i : split(joinedIcons)) {
            iconNames.add(i);
        }
        if (menuNode != 0) {
            this.menuNode = menuNode;
        }
        fillChrome();
    }

    /** A resident destination page, in row order: page i is row i. */
    void addPage(View page) {
        pages.addView(page, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        pageViews.add(page);
        page.setVisibility(pageViews.size() - 1 == selected ? View.VISIBLE : View.GONE);
    }

    /**
     * The page whose rows became the chrome. It is kept in the hierarchy but never shown: the
     * chrome draws its rows, so drawing them a second time as a list would be the same navigation
     * twice. Staying attached is what lets its nav menu find this suite when it arrives.
     */
    void addChromePage(View page) {
        pages.addView(page, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        page.setVisibility(View.GONE);
    }

    /** Show destination `index` (from a programmatic NavPatch::Select), without echoing to Rust. */
    void select(int index) {
        if (index < 0) return;
        selected = index;
        syncing = true;
        try {
            if (chrome instanceof NavigationBarView) {
                ((NavigationBarView) chrome).setSelectedItemId(index);
            } else if (chrome instanceof NavigationView) {
                Menu m = ((NavigationView) chrome).getMenu();
                MenuItem item = m.findItem(index);
                if (item != null) ((NavigationView) chrome).setCheckedItem(item);
            }
        } finally {
            syncing = false;
        }
        showPage(index);
    }

    // --- chrome ---------------------------------------------------------------------------

    @Override protected void onSizeChanged(int w, int h, int ow, int oh) {
        super.onSizeChanged(w, h, ow, oh);
        int dp = (int) (w / getResources().getDisplayMetrics().density);
        applyForm(dp >= EXPANDED_MIN_DP ? FORM_DRAWER : dp >= MEDIUM_MIN_DP ? FORM_RAIL : FORM_BAR);
    }

    /** Swap the chrome for the form this width calls for, carrying the rows and selection over. */
    private void applyForm(int next) {
        if (next == form) return;
        form = next;
        removeAllViews();
        Context ctx = getContext();
        switch (next) {
            case FORM_RAIL: chrome = new NavigationRailView(ctx); break;
            case FORM_DRAWER: chrome = new NavigationView(ctx); break;
            default: {
                BottomNavigationView bar = new BottomNavigationView(ctx);
                bar.setLabelVisibilityMode(NavigationBarView.LABEL_VISIBILITY_LABELED);
                chrome = bar;
                break;
            }
        }
        if (chrome instanceof NavigationBarView) {
            ((NavigationBarView) chrome).setOnItemSelectedListener(
                    new NavigationBarView.OnItemSelectedListener() {
                        @Override public boolean onNavigationItemSelected(MenuItem item) {
                            pick(item.getItemId());
                            return true;
                        }
                    });
        } else {
            ((NavigationView) chrome).setNavigationItemSelectedListener(
                    new NavigationView.OnNavigationItemSelectedListener() {
                        @Override public boolean onNavigationItemSelected(MenuItem item) {
                            pick(item.getItemId());
                            return true;
                        }
                    });
        }
        // The bar sits under the content; the rail and the drawer sit beside it, on the leading
        // edge — the arrangement each form is drawn for.
        setOrientation(next == FORM_BAR ? VERTICAL : HORIZONTAL);
        if (next == FORM_BAR) {
            addView(pages, new LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f));
            addView(chrome, new LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        } else {
            int w = next == FORM_DRAWER ? dp(DRAWER_WIDTH_DP) : ViewGroup.LayoutParams.WRAP_CONTENT;
            addView(chrome, new LayoutParams(w, ViewGroup.LayoutParams.MATCH_PARENT));
            addView(pages, new LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 1f));
        }
        fillChrome();
    }

    /** Build the current chrome's menu from the rows, and restore the selection into it. */
    private void fillChrome() {
        if (chrome == null) return;
        Menu menu = chrome instanceof NavigationBarView
                ? ((NavigationBarView) chrome).getMenu()
                : ((NavigationView) chrome).getMenu();
        menu.clear();
        // Only the bottom bar caps its item count (5, like the iOS tab bar). Extra destinations
        // stay resident and reachable by route or deep link; they simply get no bar item, and the
        // rail and drawer forms show them all.
        int max = chrome instanceof NavigationBarView
                ? ((NavigationBarView) chrome).getMaxItemCount()
                : titles.size();
        for (int i = 0; i < titles.size() && i < max; i++) {
            MenuItem item = menu.add(0, i, i, titles.get(i));
            Drawable icon = i < iconNames.size()
                    ? DayBridge.drawableByName(getContext(), iconNames.get(i))
                    : null;
            if (icon != null) {
                item.setIcon(icon);
            } else if (chrome instanceof NavigationBarView) {
                // The navigation bar reserves icon space whether or not there is a glyph, so a
                // title-only destination gets a small dot rather than a ragged gap.
                ShapeDrawable dot = new ShapeDrawable(new OvalShape());
                dot.setIntrinsicWidth(dp(10));
                dot.setIntrinsicHeight(dp(10));
                item.setIcon(dot);
            }
            if (chrome instanceof NavigationView) item.setCheckable(true);
        }
        if (titles.size() > max) {
            android.util.Log.w("Day", "nav suite: " + (titles.size() - max) + " destination(s) "
                    + "past the bottom bar's max of " + max + "; resident but with no bar item");
        }
        if (!titles.isEmpty()) select(Math.min(selected, titles.size() - 1));
    }

    /** A chrome tap: show the page, and report it unless we are the ones who moved the selection. */
    private void pick(int index) {
        showPage(index);
        if (!syncing) {
            selected = index;
            DayBridge.nativeOnEvent(menuNode, DayBridge.K_SELECTION_CHANGED, (double) index, null);
        }
    }

    private void showPage(int index) {
        for (int i = 0; i < pageViews.size(); i++) {
            pageViews.get(i).setVisibility(i == index ? View.VISIBLE : View.GONE);
        }
    }

    private static String[] split(String joined) {
        return joined == null || joined.isEmpty() ? new String[0] : joined.split("\u001f");
    }

    private int dp(int v) {
        return (int) (v * getResources().getDisplayMetrics().density);
    }
}
