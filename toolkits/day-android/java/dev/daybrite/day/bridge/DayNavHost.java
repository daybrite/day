// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

package dev.daybrite.day.bridge;

import android.content.Context;
import android.os.Bundle;
import android.view.LayoutInflater;
import android.view.Menu;
import android.view.MenuItem;
import android.view.SubMenu;
import android.view.View;
import android.view.ViewGroup;
import android.widget.FrameLayout;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.text.Editable;
import android.text.TextWatcher;
import com.google.android.material.textfield.TextInputEditText;
import com.google.android.material.textfield.TextInputLayout;
import android.util.TypedValue;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.WeakHashMap;

import androidx.fragment.app.Fragment;
import androidx.activity.OnBackPressedCallback;
import androidx.fragment.app.FragmentActivity;
import androidx.fragment.app.FragmentManager;
import com.google.android.material.appbar.AppBarLayout;
import com.google.android.material.appbar.MaterialToolbar;
import com.google.android.material.transition.MaterialSharedAxis;

/**
 * Navigation host (docs/navigation.md): an M3 app bar ({@link AppBarLayout} hosting a
 * {@link MaterialToolbar} — title + up arrow) over a page container that is managed by the
 * activity's {@link FragmentManager}. Each Day page rides in a {@link PageFragment} that
 * retains its Rust-owned view (the react-native-screens pattern); a push is a back-stack
 * transaction with {@link MaterialSharedAxis} transitions. That buys the whole back story
 * from the system — androidx Fragment seeks the pop transition under the predictive back
 * gesture (progress, cancel, commit) on API 34+, dispatches the hardware/gesture back on
 * every API level via {@link androidx.activity.OnBackPressedDispatcher}, and keeps the
 * predictive back-to-home animation available at the root (its callback is enabled only
 * while the back stack is non-empty). No manual gesture math anywhere.
 *
 * Native pops (gesture, back button, toolbar up) happen first and are then REPORTED to Rust
 * as NavBack with already_popped=1, so the Popped patch Rust answers with is absorbed by
 * {@link #nativePops} instead of popping again. Rust-initiated pops (dayscript, signal
 * writes) run through {@link #pop} → popBackStack, tagged in {@link #pendingPops} so the
 * back-stack listener does not re-report them.
 */
public class DayNavHost extends LinearLayout {

    /** v1: nav is app-root only, so a single active host suffices (deep-link routing). */
    static DayNavHost active;

    /** Immersive mode: the extra top chrome the floating app bar adds (px; 0 when stacked). */
    static int immersiveTopExtraPx() {
        DayNavHost h = active;
        if (h == null || !DayActivity.edgeToEdge) return 0;
        // The app bar's height already includes its status-inset padding — report only the
        // chrome BELOW the status bar, which the activity adds separately.
        return Math.max(0, h.appBar.getHeight() - DayActivity.statusInsetPx);
    }

    /** Edge-to-edge inset pass: keep the floating app bar's title row below the status bar. */
    static void onStatusInset(int px) {
        DayNavHost h = active;
        if (h != null && DayActivity.edgeToEdge && h.appBar.getPaddingTop() != px) {
            h.appBar.setPadding(0, px, 0, 0);
        }
    }
    /** page view → its host, for removePage routing even after the view is detached. */
    static final WeakHashMap<View, DayNavHost> pageHosts = new WeakHashMap<>();

    final MaterialToolbar toolbar;
    final AppBarLayout appBar;
    /** The window toolbar's spec (docs/toolbars.md), retained so the app bar's menu can be
     *  repainted from it whenever the page's own bar actions change. Empty until an app sets one. */
    private String windowToolbarSpec = "";
    /** Live toolbar items by their day id, for targeted updates. */
    final HashMap<String, MenuItem> barItems = new HashMap<>();
    /** A segmented item's segments, in order, for `updateWindowToolbar` op 2. */
    final HashMap<String, List<MenuItem>> segmentItems = new HashMap<>();
    /** A segmented item's declared label, empty when it brought none — see `nameSegmentHead`. */
    final HashMap<String, String> segmentLabels = new HashMap<>();
    /** Each window-toolbar item's UNTINTED glyph. The items share the app bar with the page's own
     *  actions now, so they need the same re-tint against what is behind them ({@link
     *  #syncBarActions}) — a bundled glyph is authored dark and vanishes on a dark bar. */
    final HashMap<MenuItem, android.graphics.drawable.Drawable> barGlyphs = new HashMap<>();
    /** Inline search field (docs/search.md); null until `setSearch` runs. */
    TextInputLayout searchLayout;
    EditText searchEdit;
    /** Suppresses the echo while day writes the app's query back into the field. */
    boolean searchSyncing;
    final FrameLayout pages;
    /// The adaptive host (docs/size-classes.md). SlidingPaneLayout decides at MEASURE time
    /// whether both panes fit: side by side on a tablet, one-at-a-time on a phone, with no size
    /// class computed by Day. `isSlideable` is the answer, and Day OBSERVES it.
    ///
    /// NULL for a host whose presentation is a permanent Stack (a nested `stack()` under a
    /// split host): that host is a stack at every size, and nesting a SlidingPaneLayout inside
    /// a pane would re-run the whole tiling decision at pane width.
    final androidx.slidingpanelayout.widget.SlidingPaneLayout split;
    /// The list pane — the Pane::Sidebar page's permanent fragment container. Permanent because a
    /// Fragment cannot change containers without its view being destroyed and rebuilt, which is
    /// exactly what a re-presentation must not do. NULL alongside `split`.
    final FrameLayout listPane;
    /// The list pane's column: the inline search field over {@link #listPane}. It is the pane the
    /// SlidingPaneLayout sizes and slides, so the field travels WITH the list it filters instead
    /// of spanning the whole host — tiled, a full-width field above both panes reads as searching
    /// the detail page. NULL alongside `split`, where the list is the whole width anyway.
    final LinearLayout listColumn;
    private final int listContainerId;
    /// The last presentation reported to Rust, so only real changes are emitted.
    private boolean lastSlideable;
    final long hostNode;
    String rootTitle; // not final: NavPatch::Title retitles the root live
    private final FragmentManager fm;
    private final int containerId;
    /** This host's back-stack entry name prefix — several hosts share the activity's manager. */
    private final String prefix;
    private final ArrayList<PageFragment> frags = new ArrayList<>();
    private final ArrayList<String> titles = new ArrayList<>();
    /** Per-pushed-page immersive-chrome flags, parallel to `titles` (the root is standard). */
    private final ArrayList<Boolean> immersives = new ArrayList<>();
    /** Back-stack entries of ours the listener has already accounted for. */
    private int knownEntries;
    /** Pops the native side already performed — absorb the answering Popped patch. */
    private int nativePops;
    /** Back guard (docs/navigation.md): while armed, a native back must NOT pop — it emits
     *  NavBack{already_popped=0} so Rust's guard decides. */
    private boolean guarded;
    /** Added LAZILY on the first arm so it lands AFTER the FragmentManager's own back callback
     *  (OnBackPressedDispatcher is LIFO), giving ours priority while enabled. */
    private OnBackPressedCallback guardCallback;
    /** Pops we initiated via popBackStack — the listener must not re-report them. */
    private int pendingPops;

    public DayNavHost(Context ctx, long hostNode, String title, boolean adaptive,
            float tileMinDp) {
        super(ctx);
        setOrientation(VERTICAL);
        this.hostNode = hostNode;
        this.rootTitle = title;
        this.prefix = "day-nav-" + hostNode + "-";

        toolbar = new MaterialToolbar(ctx);
        toolbar.setTitle(title);
        toolbar.setNavigationOnClickListener(new OnClickListener() {
            @Override public void onClick(View v) {
                if (myEntries() == 0) return;
                if (guarded) {
                    // Route through Rust's guard instead of popping (docs/navigation.md).
                    DayBridge.nativeOnEvent(hostNode, DayBridge.K_NAV_BACK, 0.0, null);
                } else {
                    // Pop natively (animated); the back-stack listener reports it to Rust.
                    fm.popBackStack();
                }
            }
        });
        appBar = new AppBarLayout(ctx);
        appBar.addView(toolbar, new AppBarLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));

        pages = new FrameLayout(ctx);
        containerId = View.generateViewId();
        pages.setId(containerId);
        listContainerId = View.generateViewId();
        View content;
        if (adaptive) {
            listPane = new FrameLayout(ctx);
            listPane.setId(listContainerId);
            // The pane is a column so an inline search field can sit above the list INSIDE it
            // (see setSearch); the list itself takes the remaining height.
            listColumn = new LinearLayout(ctx);
            listColumn.setOrientation(VERTICAL);
            listColumn.addView(listPane, new LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f));
            split = new androidx.slidingpanelayout.widget.SlidingPaneLayout(ctx);
            // A fixed-width list beside a weighted detail. SlidingPaneLayout tiles them when
            // both minimum widths fit and overlaps them when they do not — the whole adaptive
            // decision, made by the platform at measure time (docs/size-classes.md).
            float density = ctx.getResources().getDisplayMetrics().density;
            androidx.slidingpanelayout.widget.SlidingPaneLayout.LayoutParams lp =
                    new androidx.slidingpanelayout.widget.SlidingPaneLayout.LayoutParams(
                            Math.round(NAV_SIDEBAR_DP * density),
                            ViewGroup.LayoutParams.MATCH_PARENT);
            split.addView(listColumn, lp);
            // The detail pane carries a REAL width, not `0dp + weight`. SlidingPaneLayout
            // decides whether it can tile by measuring children at their LayoutParams width
            // BEFORE weights are distributed, so a zero-width detail always "fits" — a portrait
            // handset tiled a 280dp list beside a sliver of detail instead of stacking.
            // (`setMinimumWidth` does not help either; the layout reads the params, not the
            // view's minimum.) The weight still does its job once tiling is chosen, expanding
            // the detail to fill the rest.
            //
            // The two widths sum to `tileMinDp` — Day's own Compact/Medium boundary, passed in
            // from the day-spec breakpoint table — so the platform's measure-time answer and
            // Day's table agree on where a phone stops being a phone by construction
            // (docs/size-classes.md).
            androidx.slidingpanelayout.widget.SlidingPaneLayout.LayoutParams dp =
                    new androidx.slidingpanelayout.widget.SlidingPaneLayout.LayoutParams(
                            Math.round((tileMinDp - NAV_SIDEBAR_DP) * density),
                            ViewGroup.LayoutParams.MATCH_PARENT);
            dp.weight = 1f;
            split.addView(pages, dp);
            // Report every change of the platform's own answer. Layout is when it is decided,
            // so this is where it is read.
            split.addOnLayoutChangeListener(new OnLayoutChangeListener() {
                @Override public void onLayoutChange(View v, int l, int t, int r, int b,
                        int ol, int ot, int or, int ob) {
                    syncPresentation();
                }
            });
            content = split;
        } else {
            // A permanent stack: pages only, no list pane, no tiling decision to observe.
            listPane = null;
            listColumn = null;
            split = null;
            content = pages;
        }
        lastSlideable = true;

        if (DayActivity.edgeToEdge) {
            // Immersive (docs/layout.md): the page container fills the host and the app bar
            // floats transparent above it — page content runs under the status bar and
            // toolbar, padding itself by day::safe_area(). The app bar carries the status-bar
            // inset as top padding so the title row sits below the system chrome.
            appBar.setElevation(0f);
            toolbar.setBackgroundColor(android.graphics.Color.TRANSPARENT);
            appBar.setPadding(0, DayActivity.statusInsetPx, 0, 0);
            appBar.addOnLayoutChangeListener(new OnLayoutChangeListener() {
                @Override public void onLayoutChange(View v, int l, int t, int r, int b,
                        int ol, int ot, int or, int ob) {
                    if ((b - t) != (ob - ot)) DayActivity.reportTopInset();
                }
            });
            FrameLayout overlay = new FrameLayout(ctx);
            overlay.addView(content, new FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
            overlay.addView(appBar, new FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
            addView(overlay, new LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f));
        } else {
            addView(appBar, new LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT));
            addView(content, new LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f));
        }
        fm = ((FragmentActivity) ctx).getSupportFragmentManager();
        fm.addOnBackStackChangedListener(new FragmentManager.OnBackStackChangedListener() {
            // Predictive-gesture pops commit on their own schedule, so reconcile from every
            // hook AND once more next tick — resync() is idempotent (knownEntries guard).
            @Override public void onBackStackChanged() {
                resync();
                pages.post(resyncRunnable);
            }
            @Override public void onBackStackChangeCommitted(
                    androidx.fragment.app.Fragment f, boolean pop) {
                resync();
                pages.post(resyncRunnable);
            }
        });
        active = this;
        syncChrome();
    }

    /** Our entries on the shared back stack (several hosts may nest in one activity). */
    private int myEntries() {
        int c = 0;
        for (int i = 0; i < fm.getBackStackEntryCount(); i++) {
            CharSequence n = fm.getBackStackEntryAt(i).getName();
            if (n != null && n.toString().startsWith(prefix)) c++;
        }
        return c;
    }

    private final Runnable resyncRunnable = new Runnable() {
        @Override public void run() {
            resync();
        }
    };

    /** Reconcile bookkeeping with the back stack. Pops the native container performed
     *  (gesture, back button, up arrow) are reported to Rust on the NEXT main-loop tick:
     *  this can run while the FragmentManager is still executing, and Rust's reaction
     *  (removing the page subtree) lands back in fragment transactions — re-entrant
     *  execution is an IllegalStateException. */
    /**
     * The inline search field belongs to the TOP-LEVEL list only (docs/search.md): it filters
     * that list, so on a pushed detail page there is nothing for it to filter. Driven from
     * `resync`, the one place the stack depth is reconciled, so every route in — push, pop,
     * predictive-back — obeys the same rule.
     */
    /** Stacked (one pane at a time)? A plain stack host always is; an adaptive host asks its
     *  SlidingPaneLayout, whose measure pass owns the answer (docs/size-classes.md). */
    private boolean stacked() {
        return split == null || split.isSlideable();
    }

    private void syncSearchVisibility(int depth) {
        if (searchLayout != null) {
            // Tiled, the list it filters never leaves the screen, so the field stays too.
            boolean show = depth == 0 || !stacked();
            searchLayout.setVisibility(show ? View.VISIBLE : View.GONE);
        }
    }

    /**
     * Slide back to the list once the stack is empty.
     *
     * `push` opens the detail pane; nothing but this closes it. A pop that leaves the pane open
     * shows the emptied detail container — a blank screen under the app bar, with the top-level
     * list sitting off to the side, present and laid out and not on screen. The system back
     * button hides the bug: SlidingPaneLayout installs its own back callback and closes the pane
     * itself, so only the routes that go through Rust (`navigate`, the toolbar up arrow, a
     * dayscript `nav_back`) ever showed it.
     *
     * Tiled, both panes are on screen and there is nothing to slide.
     */
    private void syncPane(int depth) {
        // Unconditional, not `if (isOpen())`: `push` opens the pane with an ANIMATION, and a pop
        // that lands while it is still sliding sees `isOpen() == false` and would skip — leaving
        // the slide to finish afterwards, ending open over an emptied detail container. `close`
        // on an already-closed pane does nothing, so asking every time is the cheap correct rule.
        if (depth == 0 && split != null && split.isSlideable()) {
            split.close();
        }
    }

    private void resync() {
        int now = myEntries();
        syncSearchVisibility(now);
        syncPane(now);
        while (knownEntries > now) {
            knownEntries--;
            if (!titles.isEmpty()) titles.remove(titles.size() - 1);
            if (!immersives.isEmpty()) immersives.remove(immersives.size() - 1);
            if (pendingPops > 0) {
                pendingPops--;
            } else {
                pages.post(new Runnable() {
                    @Override public void run() {
                        nativePops++;
                        // kind 5 = NavBack; num 1.0 = the native container already popped.
                        DayBridge.nativeOnEvent(hostNode, DayBridge.K_NAV_BACK, 1.0, null);
                    }
                });
            }
        }
        knownEntries = now;
        syncChrome();
    }

    int depth() {
        return titles.size();
    }

    /** Arm/disarm the back guard (NavPatch::GuardTop). While armed, the system/gesture back and
     *  the toolbar up-arrow route to Rust as NavBack{already_popped=0} so the app's guard
     *  decides; a Proceed then calls navPop (docs/navigation.md). The predictive-back preview is
     *  unavailable while armed (our callback owns the gesture). */
    void setGuard(boolean on) {
        this.guarded = on;
        if (guardCallback == null) {
            if (!on) return; // never armed yet — nothing to toggle
            guardCallback = new OnBackPressedCallback(false) {
                @Override public void handleOnBackPressed() {
                    DayBridge.nativeOnEvent(hostNode, DayBridge.K_NAV_BACK, 0.0, null);
                }
            };
            ((FragmentActivity) getContext()).getOnBackPressedDispatcher()
                    .addCallback(guardCallback);
        }
        guardCallback.setEnabled(on);
    }

    /** Live retitle of the CURRENT top (`NavPatch::Title`): the root title when nothing is
     *  pushed, else the top entry — then re-sync the toolbar. */
    void retitle(String title) {
        if (titles.isEmpty()) {
            rootTitle = title;
        } else {
            titles.set(titles.size() - 1, title);
        }
        syncChrome();
    }

    /**
     * Install the inline search field directly under the app bar, above the navigation list
     * (docs/search.md).
     *
     * A Material `TextInputLayout` with a search icon and a clear button, NOT the Material
     * `SearchBar`/`SearchView` pair: `SearchBar` is a launcher for a full-screen `SearchView`
     * overlay that shows its OWN results list, and on a searchable navigation surface the list
     * underneath already is the result set. An editable filter-in-place field is the control this
     * actually needs.
     *
     * No auto-hide: iOS reveals its field by over-scrolling past the top of the list, and Material
     * has no equivalent gesture, so the field stays put.
     */
    void setSearch(final long id, String prompt, String text) {
        if (searchLayout != null) {
            return; // already installed
        }
        TextInputLayout box = new TextInputLayout(getContext(), null,
                com.google.android.material.R.attr.textInputOutlinedStyle);
        box.setHint(prompt == null ? "" : prompt);
        box.setEndIconMode(TextInputLayout.END_ICON_CLEAR_TEXT);
        box.setBoxBackgroundMode(TextInputLayout.BOX_BACKGROUND_OUTLINE);
        TextInputEditText edit = new TextInputEditText(box.getContext());
        edit.setSingleLine(true);
        edit.setImeOptions(android.view.inputmethod.EditorInfo.IME_ACTION_SEARCH);
        edit.setInputType(android.text.InputType.TYPE_CLASS_TEXT);
        if (text != null && !text.isEmpty()) {
            edit.setText(text);
        }
        edit.addTextChangedListener(new TextWatcher() {
            @Override public void beforeTextChanged(CharSequence s, int a, int b, int c) {}
            @Override public void onTextChanged(CharSequence s, int a, int b, int c) {}
            @Override public void afterTextChanged(Editable e) {
                if (!searchSyncing) {
                    DayBridge.nativeOnEvent(id, DayBridge.K_SEARCH_CHANGED, 0.0, e.toString());
                }
            }
        });
        box.addView(edit, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT));
        int pad = (int) TypedValue.applyDimension(
                TypedValue.COMPLEX_UNIT_DIP, 12f, getResources().getDisplayMetrics());
        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT);
        lp.setMargins(pad, pad / 2, pad, pad / 2);
        box.setLayoutParams(lp);
        searchLayout = box;
        searchEdit = edit;
        if (listColumn != null) {
            // Inside the list pane, above the list: the field belongs to the surface it filters
            // (docs/search.md), and tiled that surface is one pane rather than the whole window.
            listColumn.addView(box, 0);
        } else {
            // No pane to belong to — directly under the app bar, part of the list's own chrome.
            int at = indexOfChild(appBar) + 1;
            addView(box, at < 0 ? 0 : at);
        }
        // A surface can be built with pages already stacked (a launch deep link), so start from
        // the current depth rather than assuming the root.
        syncSearchVisibility(myEntries());
    }

    /** Day writing the app's query back in — guarded so the watcher does not echo it. */
    void setSearchText(String text) {
        if (searchEdit == null) {
            return;
        }
        String next = text == null ? "" : text;
        if (next.contentEquals(searchEdit.getText())) {
            return;
        }
        searchSyncing = true;
        searchEdit.setText(next);
        searchSyncing = false;
    }

    /** One installed nav-bar action, kept as its DECLARATION rather than only as the live item:
     *  the window toolbar shares this menu, so setting one clears and repaints the whole thing and
     *  every action has to be re-addable. `glyph` is the UNTINTED master (re-tinted whenever the
     *  bar changes color, so it must survive); `rootOnly` marks the ones that belong to the list. */
    private static final class BarAction {
        final String iconName;
        final String label;
        final long actionId;
        final boolean rootOnly;
        MenuItem item;
        android.graphics.drawable.Drawable glyph;
        BarAction(String iconName, String label, long actionId, boolean rootOnly) {
            this.iconName = iconName;
            this.label = label == null ? "" : label;
            this.actionId = actionId;
            this.rootOnly = rootOnly;
        }
    }

    private final java.util.ArrayList<BarAction> barActions = new java.util.ArrayList<>();

    /** Set the window toolbar (docs/toolbars.md) from day-android's `serialize_toolbar` spec and
     *  repaint the app bar. The spec is retained because the bar's menu is shared with the page's
     *  own bar actions, and either side changing repaints both. */
    void setWindowToolbar(String spec) {
        windowToolbarSpec = spec == null ? "" : spec;
        paintBarMenu();
    }

    /** Paint the app bar's menu: the window toolbar's items, then the page's own bar actions.
     *
     *  ONE bar per window, the app bar, at every width — the same place day-uikit puts it
     *  (docs/toolbars.md). Day used to dock a second MaterialToolbar under the pages, which read
     *  as a phone's bottom bar and, tiled on a tablet, as a strip of icons stranded below both
     *  panes with the titled bar above them empty. An action belongs in the top app bar on
     *  Android, and what the bar cannot fit belongs in its overflow — which is why the items go in
     *  as `SHOW_AS_ACTION_IF_ROOM` and an app declares its least-used items last.
     *
     *  The toolbar's items lead and the bar actions trail, so a page's own action keeps the
     *  position it had before the window had a toolbar at all.
     *
     *  The spec is `\u001e` between items, `\u001f` between fields (id, kind, label, icon,
     *  enabled, action, extra). Buttons and toggles become actions; a menu becomes a submenu built
     *  like the app menu; a segmented item becomes a submenu of radio choices; spaces and
     *  separators are the bar's own business and are skipped. */
    private void paintBarMenu() {
        Menu menu = toolbar.getMenu();
        menu.clear();
        barItems.clear();
        segmentItems.clear();
        segmentLabels.clear();
        barGlyphs.clear();
        int order = 0;
        int groupSeq = 1;
        String spec = windowToolbarSpec;
        for (String rec : spec.split("\u001e", -1)) {
            if (rec.isEmpty()) continue;
            String[] f = rec.split("\u001f", -1);
            String id = f[0];
            String kind = f.length > 1 ? f[1] : "";
            String label = f.length > 2 ? f[2] : "";
            String icon = f.length > 3 ? f[3] : "";
            boolean enabled = f.length > 4 && f[4].equals("1");
            final long action = f.length > 5 ? parseActionId(f[5]) : 0L;
            String extra = f.length > 6 ? f[6] : "";
            android.graphics.drawable.Drawable glyph =
                    DayBridge.drawableByName(getContext(), icon);
            if (kind.equals("button") || kind.equals("toggle")) {
                MenuItem it = menu.add(Menu.NONE, Menu.NONE, order++, label);
                it.setEnabled(enabled);
                showAsAction(it, glyph);
                final boolean toggle = kind.equals("toggle");
                if (toggle) {
                    it.setCheckable(true);
                    it.setChecked(extra.equals("1"));
                    paintToggle(it);
                }
                it.setOnMenuItemClickListener(new MenuItem.OnMenuItemClickListener() {
                    @Override public boolean onMenuItemClick(MenuItem mi) {
                        if (toggle) {
                            boolean on = !mi.isChecked();
                            mi.setChecked(on);
                            paintToggle(mi);
                            DayBridge.nativeOnEvent(action, DayBridge.K_TOOLBAR_CHANGED,
                                    on ? 1.0 : 0.0, "on");
                        } else {
                            DayBridge.nativeOnEvent(action, DayBridge.K_MENU_ACTION, 0.0, "");
                        }
                        return true;
                    }
                });
                barItems.put(id, it);
            } else if (kind.equals("menu")) {
                SubMenu sm = menu.addSubMenu(Menu.NONE, Menu.NONE, order++, label);
                MenuItem it = sm.getItem();
                it.setEnabled(enabled);
                showAsAction(it, glyph);
                DayBridge.buildMenu(sm, extra);
                barItems.put(id, it);
            } else if (kind.equals("segmented")) {
                String[] seg = extra.split("\u001d", -1);
                int selected = 0;
                try {
                    selected = Integer.parseInt(seg[0]);
                } catch (NumberFormatException e) {
                    // an unreadable index selects the first segment
                }
                SubMenu sm = menu.addSubMenu(Menu.NONE, Menu.NONE, order++, label);
                MenuItem head = sm.getItem();
                head.setEnabled(enabled);
                showAsAction(head, glyph);
                final ArrayList<MenuItem> segments = new ArrayList<>();
                // A segmented control carries no label of its own — it is a row of choices, and
                // the platforms that draw one draw the choices (day-pieces `toolbar_segmented`).
                // Folded into a menu there is no row to draw them on, and a submenu MUST be
                // named or Android renders an empty line with an arrow. The choice in force is
                // the name: "Dark >" opening Light/System/Dark reads as the setting it is.
                int group = groupSeq++;
                for (int i = 1; i < seg.length; i++) {
                    final int idx = i - 1;
                    MenuItem s = sm.add(group, Menu.NONE, i, seg[i]);
                    s.setCheckable(true);
                    s.setChecked(idx == selected);
                    s.setOnMenuItemClickListener(new MenuItem.OnMenuItemClickListener() {
                        @Override public boolean onMenuItemClick(MenuItem mi) {
                            checkSegment(segments, idx);
                            nameSegmentHead(head, segments, label);
                            DayBridge.nativeOnEvent(action, DayBridge.K_TOOLBAR_CHANGED,
                                    idx, "sel");
                            return true;
                        }
                    });
                    segments.add(s);
                }
                sm.setGroupCheckable(group, true, true);
                barItems.put(id, head);
                segmentItems.put(id, segments);
                segmentLabels.put(id, label);
                nameSegmentHead(head, segments, label);
            } else if (kind.equals("label")) {
                MenuItem it = menu.add(Menu.NONE, Menu.NONE, order++, label);
                it.setEnabled(false);
                it.setShowAsAction(MenuItem.SHOW_AS_ACTION_NEVER);
                barItems.put(id, it);
            }
            // "sep", "space", "flex": the app bar spaces its own actions.
        }
        // The page's own actions trail the window's, at orders no toolbar item can reach.
        for (int i = 0; i < barActions.size(); i++) {
            BarAction ba = barActions.get(i);
            ba.item = menu.add(Menu.NONE, Menu.NONE, 1000 + i, ba.label);
            ba.item.setShowAsAction(MenuItem.SHOW_AS_ACTION_ALWAYS);
            ba.glyph = DayBridge.drawableByName(getContext(), ba.iconName);
            final long actionId = ba.actionId;
            ba.item.setOnMenuItemClickListener(new MenuItem.OnMenuItemClickListener() {
                @Override public boolean onMenuItemClick(MenuItem item) {
                    DayBridge.nativeOnEvent(actionId, DayBridge.K_MENU_ACTION, 0.0, "");
                    return true;
                }
            });
        }
        syncBarActions();
    }

    /** A targeted change to one live item: op 0 = enabled, 1 = toggle on, 2 = segment index. */
    void updateWindowToolbar(String id, int op, double num) {
        MenuItem it = barItems.get(id);
        if (it == null) return;
        if (op == 0) {
            it.setEnabled(num != 0.0);
        } else if (op == 1) {
            it.setChecked(num != 0.0);
            paintToggle(it);
        } else if (op == 2) {
            List<MenuItem> segments = segmentItems.get(id);
            if (segments != null) {
                checkSegment(segments, (int) num);
                // An unlabeled control is named by the segment in force, so a change the APP
                // made has to rename it too — not only one the user tapped.
                nameSegmentHead(it, segments, segmentLabels.get(id));
            }
        }
    }

    private static long parseActionId(String s) {
        try {
            return Long.parseLong(s);
        } catch (NumberFormatException e) {
            return 0L;
        }
    }

    /** An item with a glyph shows as an icon; without one, as its text — and only while the
     *  bar has room for it, since words are wide: the rest fold into the bar's overflow menu
     *  rather than running off its trailing edge. */
    private void showAsAction(MenuItem it, android.graphics.drawable.Drawable glyph) {
        if (glyph != null) {
            barGlyphs.put(it, glyph);
            it.setIcon(glyph);
            // IF_ROOM, not ALWAYS: the app bar carries the destination's title as well, and a
            // window toolbar is long enough (the Showcase declares eight items) that forcing
            // every one into the bar would leave no room to read where you are. What does not
            // fit folds into the overflow, in declaration order — which is why an app declares
            // its least-used items last.
            it.setShowAsAction(MenuItem.SHOW_AS_ACTION_IF_ROOM);
        } else {
            // No glyph, no place in the bar. A Material top app bar carries icon buttons and
            // sends the rest to its overflow; a titled text action sits there as wide as its
            // label, and two of them squeezed the Showcase's own title to "Day Showc…" on a
            // phone. In the overflow the same item reads as a full menu row.
            it.setShowAsAction(MenuItem.SHOW_AS_ACTION_NEVER);
        }
    }

    /** A toggle's state, drawn as the glyph's opacity: full when on, dimmed when off. */
    private static void paintToggle(MenuItem it) {
        android.graphics.drawable.Drawable d = it.getIcon();
        if (d != null) d.setAlpha(it.isChecked() ? 255 : 110);
    }

    private static void checkSegment(List<MenuItem> segments, int idx) {
        for (int i = 0; i < segments.size(); i++) segments.get(i).setChecked(i == idx);
    }

    /** Title the row a segmented control folds into, when the control brought no label of its
     *  own: the segment in force names it, so the row says what the setting IS and reads as a
     *  value to change rather than a blank line. A labeled control keeps its label. */
    private static void nameSegmentHead(MenuItem head, List<MenuItem> segments, String label) {
        if (label != null && !label.isEmpty()) return;
        for (MenuItem s : segments) {
            if (s.isChecked()) {
                head.setTitle(s.getTitle());
                return;
            }
        }
    }

    /** Add one trailing action to the app bar's menu (docs/navigation.md). Called once per action,
     *  in declaration order, by {@link DayBridge#setNavMenu} AFTER construction and inside a
     *  try/catch — never from the constructor — so a failure here can't blank the host.
     *  The MaterialToolbar keeps its menu across pushes and pops, so an item rides every page
     *  until {@link #syncBarActions} hides it. */
    void addBarAction(String iconName, String label, final long actionId, boolean rootOnly) {
        barActions.add(new BarAction(iconName, label, actionId, rootOnly));
        paintBarMenu();
    }

    /** Re-tint every glyph on the app bar — the window toolbar's and the page's own — to the bar's
     *  CURRENT color, and hide the list-only actions off the list.
     *  Driven from {@link #syncChrome}, which already runs on every push, pop and re-present —
     *  the three moments that change what is behind these glyphs or which page they are on. */
    private void syncBarActions() {
        if (barActions.isEmpty() && barGlyphs.isEmpty()) {
            return;
        }
        int tint = barGlyphColor();
        for (java.util.Map.Entry<MenuItem, android.graphics.drawable.Drawable> e
                : barGlyphs.entrySet()) {
            android.graphics.drawable.Drawable d = e.getValue().mutate();
            d.setTint(tint);
            e.getKey().setIcon(d);
            // Tinting replaced the icon, so a toggle's on/off opacity has to be re-applied to it.
            if (e.getKey().isCheckable()) {
                paintToggle(e.getKey());
            }
        }
        boolean atRoot = !stacked() || titles.isEmpty();
        for (BarAction ba : barActions) {
            ba.item.setVisible(!ba.rootOnly || atRoot);
            if (ba.glyph != null) {
                // mutate() per apply: a bundled drawable can be shared with other views through
                // the resource cache, and tinting the shared instance would recolor them too.
                android.graphics.drawable.Drawable d = ba.glyph.mutate();
                d.setTint(tint);
                ba.item.setIcon(d);
            }
        }
    }

    /** The color the bar's own glyphs take, derived from what is actually BEHIND them.
     *
     *  Not a constant, and not "white when edge-to-edge": the app bar is `colorPrimary` under
     *  edge-to-edge, a dark scrim over an immersive page, and the theme's surface otherwise — and
     *  that surface follows the system light/dark setting. The bundled glyphs are authored dark,
     *  so leaving them untinted (which is what everything except the edge-to-edge case used to do)
     *  puts a black icon on a near-black bar in dark mode. */
    private int barGlyphColor() {
        int bg = barBackgroundColor();
        // Relative luminance, Rec. 709. Below the midpoint the bar is dark and needs light glyphs.
        double lum = (0.2126 * android.graphics.Color.red(bg)
                + 0.7152 * android.graphics.Color.green(bg)
                + 0.0722 * android.graphics.Color.blue(bg)) / 255.0;
        return lum < 0.5 ? 0xFFFFFFFF : 0xFF1C1B1F;
    }

    /** What the app bar is actually painted with right now — the same three cases
     *  {@link #syncChrome} paints, read back rather than re-derived. */
    private int barBackgroundColor() {
        if (DayActivity.edgeToEdge) {
            boolean topImmersive = !immersives.isEmpty() && immersives.get(immersives.size() - 1);
            // The immersive chrome is a black scrim gradient; the ordinary bar is colorPrimary.
            return topImmersive
                    ? 0xFF000000
                    : themeColor(androidx.appcompat.R.attr.colorPrimary, 0xFF0B57D0);
        }
        android.graphics.drawable.Drawable d = appBar.getBackground();
        if (d instanceof android.graphics.drawable.ColorDrawable) {
            return ((android.graphics.drawable.ColorDrawable) d).getColor();
        }
        return themeColor(com.google.android.material.R.attr.colorSurface, 0xFFFFFFFF);
    }

    private int themeColor(int attr, int fallback) {
        android.util.TypedValue tv = new android.util.TypedValue();
        if (getContext().getTheme().resolveAttribute(attr, tv, true)) {
            if (tv.type >= android.util.TypedValue.TYPE_FIRST_COLOR_INT
                    && tv.type <= android.util.TypedValue.TYPE_LAST_COLOR_INT) {
                return tv.data;
            }
            if (tv.resourceId != 0) {
                return getContext().getColor(tv.resourceId);
            }
        }
        return fallback;
    }

    /** The list pane's width, dp (docs/size-classes.md). */
    private static final float NAV_SIDEBAR_DP = 280f;

    /**
     * Report the platform's own presentation decision to Rust (docs/size-classes.md).
     *
     * `isSlideable` is SlidingPaneLayout's answer to "did both panes fit?", settled during
     * measure. Day does not compute it and must not override it — it reconciles the model to what
     * the platform already did, which for a selector means the split-with-nothing-selected rule.
     * Only real changes are emitted; layout runs constantly.
     */
    private void syncPresentation() {
        if (split == null) return; // a permanent stack has nothing to observe or report
        boolean slideable = split.isSlideable();
        if (slideable == lastSlideable) return;
        lastSlideable = slideable;
        // Chrome differs: a visible list needs no up-arrow to get back to it, and the inline
        // search field filters a list that is now always on screen.
        syncChrome();
        syncSearchVisibility(depth());
        // num 1.0 = split (both panes), 0.0 = stacked.
        DayBridge.nativeOnEvent(hostNode, DayBridge.K_NAV_PRESENTATION, slideable ? 0.0 : 1.0, null);
    }

    private void syncChrome() {
        toolbar.setTitle(titles.isEmpty() ? rootTitle : titles.get(titles.size() - 1));
        // Only a stacked presentation needs an up-arrow: when both panes are tiled the list is
        // already on screen, so there is nowhere for "back" to go (docs/size-classes.md).
        showUpArrow(!titles.isEmpty() && stacked());
        // Edge-to-edge mode: per-page chrome (docs/navigation.md). An immersive page keeps the
        // floating scrim bar over full-bleed content; the root and unmarked pages get a solid
        // colorPrimary bar, which also backs the status-bar area (the app bar carries the
        // status inset as padding), keeping white chrome legible over light pages.
        if (DayActivity.edgeToEdge) {
            boolean topImmersive =
                    !immersives.isEmpty() && immersives.get(immersives.size() - 1);
            if (topImmersive) {
                appBar.setBackground(new android.graphics.drawable.GradientDrawable(
                        android.graphics.drawable.GradientDrawable.Orientation.TOP_BOTTOM,
                        new int[] { 0x66000000, 0x00000000 }));
            } else {
                android.util.TypedValue tv = new android.util.TypedValue();
                int color = 0xFF0B57D0;
                if (getContext().getTheme().resolveAttribute(
                        androidx.appcompat.R.attr.colorPrimary, tv, true)) {
                    color = tv.data;
                }
                appBar.setBackgroundColor(color);
            }
        }
        // AFTER the background above: the glyph color is derived from it, and the list-only items
        // depend on the depth this method just re-read.
        syncBarActions();
    }

    /** Register the Rust-owned page view. The root page becomes a fragment immediately; a
     *  pushed page parks in the container as a raw hidden child until push() presents it —
     *  the patch order is add-then-push, and keeping the view attached in between lets a nav
     *  host nested inside the page register its own container with the FragmentManager. */
    void add(View page) {
        page.setLayoutParams(new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        PageFragment f = new PageFragment(page);
        frags.add(f);
        pageHosts.put(page, this);
        if (frags.size() == 1) {
            // The first page. Adaptive host: the Pane::Sidebar page, the list pane's permanent
            // occupant in BOTH presentations. Plain stack host: its root page, which lives in
            // the pages container — there is no list pane at all.
            int container = split != null ? listContainerId : containerId;
            fm.beginTransaction().setReorderingAllowed(true)
                    .add(container, f).commitNowAllowingStateLoss();
        } else {
            page.setVisibility(View.GONE);
            pages.addView(page);
        }
    }

    /** Present the most recently added page (Pushed patch): a replace() back-stack transaction
     *  with the Material shared-axis X motion. The reversal of this transaction IS the pop —
     *  played by popBackStack and seeked live by the system under a predictive back gesture.
     *  (replace, not show/hide: fragment predictive back seeks lifecycle operations; the
     *  covered page detaches and its fragment re-serves the retained view on return.) */
    void push(String title, boolean immersive) {
        int n = frags.size();
        if (n < 2) return;
        PageFragment top = frags.get(n - 1);
        PageFragment prev = frags.get(n - 2);
        View v = top.content;
        if (v.getParent() == pages) pages.removeView(v); // the fragment owns it from here
        v.setVisibility(View.VISIBLE);
        top.setEnterTransition(new MaterialSharedAxis(MaterialSharedAxis.X, true));
        top.setReturnTransition(new MaterialSharedAxis(MaterialSharedAxis.X, false));
        prev.setExitTransition(new MaterialSharedAxis(MaterialSharedAxis.X, true));
        prev.setReenterTransition(new MaterialSharedAxis(MaterialSharedAxis.X, false));
        titles.add(title);
        immersives.add(immersive);
        fm.beginTransaction().setReorderingAllowed(true)
                .replace(containerId, top)
                .addToBackStack(prefix + titles.size())
                .commitAllowingStateLoss();
        // Execute NOW (commitNow can't take a back stack): the entry must be registered
        // before the next resync(), or the count mismatch reads as a phantom pop.
        fm.executePendingTransactions();
        // Bring the detail forward. Tiled, this is a no-op — both panes are already visible.
        // A plain stack host has no panes at all; its pages container is the only surface.
        if (split != null) split.open();
        syncChrome();
    }

    /** Rust-initiated pop (Popped patch). A pop the native container already performed
     *  (gesture / back button / toolbar up) was reported with already_popped and is absorbed
     *  here; anything else pops the back stack, which plays the push's reversal. Immediate so
     *  the fragment state is settled when Rust's removePage follows in the same patch batch
     *  (the exit transition still plays out visually). */
    void pop() {
        if (nativePops > 0) {
            nativePops--;
            return;
        }
        if (myEntries() == 0) return;
        // Pop OUR most recent entry (inclusive pops anything an inner host stacked above it,
        // whose own listener then reports those to Rust — correct unwinding).
        for (int i = fm.getBackStackEntryCount() - 1; i >= 0; i--) {
            CharSequence n = fm.getBackStackEntryAt(i).getName();
            if (n != null && n.toString().startsWith(prefix)) {
                pendingPops++;
                fm.popBackStackImmediate(n.toString(), FragmentManager.POP_BACK_STACK_INCLUSIVE);
                return;
            }
        }
    }

    /** Rust removed the page subtree. A popped page's fragment is already gone (the pop
     *  destroyed it); this covers the bookkeeping plus teardown of a still-presented page or
     *  a parked never-pushed one. */
    void removePage(View page) {
        pageHosts.remove(page);
        for (int i = frags.size() - 1; i >= 0; i--) {
            PageFragment f = frags.get(i);
            if (f.content == page) {
                frags.remove(i);
                if (f.isAdded()) {
                    fm.beginTransaction().setReorderingAllowed(true)
                            .remove(f).commitNowAllowingStateLoss();
                } else if (page.getParent() == pages) {
                    pages.removeView(page);
                }
                break;
            }
        }
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

    /** A fragment that retains and re-serves its Rust-owned page view (the
     *  react-native-screens pattern) — the FragmentManager owns WHEN it shows, Day owns WHAT
     *  it shows. Public with a no-arg constructor per the Fragment contract; DayActivity
     *  handles config changes itself (manifest configChanges), so framework re-instantiation
     *  does not happen in practice — if it ever does, the empty view is torn down and rebuilt
     *  by Rust. */
    public static class PageFragment extends Fragment {
        View content;

        public PageFragment() {}

        PageFragment(View content) {
            this.content = content;
        }

        @Override public View onCreateView(LayoutInflater inflater, ViewGroup container,
                Bundle savedInstanceState) {
            if (content == null) return new View(inflater.getContext());
            ViewGroup p = (ViewGroup) content.getParent();
            if (p != null) p.removeView(content);
            // The shared-axis transitions animate transforms on this RETAINED view; an
            // interrupted transition (pop mid-push, seek cut short) leaves its last values
            // behind, and the next transition builds on them — a compounding leftward drift.
            // Every appearance starts from identity.
            content.setTranslationX(0f);
            content.setTranslationY(0f);
            content.setScaleX(1f);
            content.setScaleY(1f);
            content.setAlpha(1f);
            return content;
        }
    }
}
