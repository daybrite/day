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
 * retains its Rust-owned view (the react-native-screens pattern); a nested push is a back-stack
 * transaction with {@link MaterialSharedAxis} transitions. That buys the whole back story
 * from the system — androidx Fragment seeks the pop transition under the predictive back
 * gesture (progress, cancel, commit) on API 34+, dispatches the hardware/gesture back on
 * every API level via {@link androidx.activity.OnBackPressedDispatcher}, and keeps the
 * predictive back-to-home animation available at the root (its callback is enabled only
 * while the back stack is non-empty). No manual gesture math anywhere.
 *
 * Native pops (gesture, back button, toolbar up) happen first and are then REPORTED to Rust
 * as NavBack with already_popped=1. An answering Popped patch, when the owner sends one, is
 * absorbed by {@link #nativePoppedPages} instead of popping again; removing that same page
 * also consumes its acknowledgement. Adaptive hosts keep their selected section outside
 * fragment history: compact back clears the section and closes the pane, while wide back
 * leaves the current section selected. Rust-initiated pops (dayscript, signal writes) run
 * through {@link #pop} → popBackStack, tagged in {@link #pendingPops} so the back-stack
 * listener does not re-report them.
 */
public class DayNavHost extends LinearLayout {

    /** v1: nav is app-root only, so a single active host suffices (deep-link routing). */
    static DayNavHost active;

    /** The window root whose toolbar this host carries (DayBridge.navHostAttached), null
     *  while it carries none. */
    java.lang.ref.WeakReference<View> toolbarRoot;

    /** Immersive mode: the extra top chrome the floating app bar adds (px; 0 when stacked). */
    static int immersiveTopExtraPx() {
        DayNavHost h = active;
        if (h == null || !DayActivity.edgeToEdge) return 0;
        // The app bar's height already includes its status-inset padding — report only the
        // chrome below the status bar, which the activity adds separately.
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
    /** The window toolbar on this host's app bar (docs/toolbars.md) — the shared painter, bound
     *  to the bar this host already has. Assigned with `toolbar`, in the constructor. */
    private WindowBar windowBar;
    /** The pinned inline search field (docs/search.md): an outlined box over the list. Null
     *  until `setSearch` runs, and null for good when the chrome scrolls (see {@link #searchBar}). */
    TextInputLayout searchLayout;
    /** The field's editor: the box's, or the search view's. */
    EditText searchEdit;
    /** Whichever control sits in the app bar, the box or the search bar; null until `setSearch`. */
    View searchField;
    /** Scrolling chrome's search (docs/search.md): Material's search bar, the pill under the
     *  title that scrolls away with it, and the full-screen search view it expands into, which
     *  borrows the root list for its results while it is up. Null in pinned mode. */
    com.google.android.material.search.SearchBar searchBar;
    com.google.android.material.search.SearchView searchView;
    /** Where the root list lives while the search view is down: its parent, position and layout
     *  params, so the list goes back exactly where it came from. */
    private ViewGroup listHome;
    private int listHomeIndex;
    private ViewGroup.LayoutParams listHomeParams;
    /** System Back while the search view is up collapses it, as Material's own apps do. */
    private OnBackPressedCallback searchBackCallback;
    /** Suppresses the echo while day writes the app's query back into the field. */
    boolean searchSyncing;
    private androidx.coordinatorlayout.widget.CoordinatorLayout scrollingChrome;
    private View scrollingContent;
    private int chromeOffset;
    private boolean scrollingSearchShown;
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
    /// SlidingPaneLayout sizes and slides, so the field travels with the list it filters instead
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
    private final ArrayList<PageFragment> nativePoppedPages = new ArrayList<>();
    /** Adaptive hosts have a selected section, not a history entry for that section. */
    private OnBackPressedCallback sectionBackCallback;
    /** Back guard (docs/navigation.md): while armed, a native back must not pop — it emits
     *  NavBack{already_popped=0} so Rust's guard decides. */
    private boolean guarded;
    /** Added LAZILY on the first arm so it lands after the FragmentManager's own back callback
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
        windowBar = new WindowBar(toolbar);
        toolbar.setTitle(title);
        toolbar.setNavigationOnClickListener(new OnClickListener() {
            @Override public void onClick(View v) {
                if (titles.isEmpty()) return;
                if (guarded) {
                    // Route through Rust's guard instead of popping (docs/navigation.md).
                    DayBridge.nativeOnEvent(hostNode, DayBridge.K_NAV_BACK, 0.0, null);
                } else if (split != null && titles.size() == 1) {
                    DayBridge.nativeOnEvent(hostNode, DayBridge.K_NAV_BACK, 0.0, null);
                } else {
                    // Pop natively (animated); the back-stack listener reports it to Rust.
                    fm.popBackStack();
                }
            }
        });
        // Allocated before the bar, which names it as the view whose scrolling lifts it.
        containerId = View.generateViewId();
        appBar = new AppBarLayout(ctx);
        appBar.addView(toolbar, new AppBarLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        // LIFT ON SCROLL (docs/toolbars.md) — Material 3's answer to the same question iOS
        // answers with a bar that blends into the content until you scroll: at rest the bar is
        // `colorSurface`, the same tone as the panes below it, so there is no band across the
        // window; once a scrolling child passes beneath it, `AppBarLayout` raises it to
        // `colorSurfaceContainer` and the separation appears exactly when it means something.
        //
        // It matters most on a tablet. A full-width bar in a different color above two tiled
        // panes reads as a stripe between the content and the status bar rather than as chrome
        // belonging to either pane; toned with the panes, it reads as their shared header.
        appBar.setLiftOnScroll(true);
        appBar.setLiftOnScrollTargetViewId(containerId);

        pages = new FrameLayout(ctx);
        pages.setId(containerId);
        listContainerId = View.generateViewId();
        View content;
        if (adaptive) {
            listPane = new FrameLayout(ctx);
            listPane.setId(listContainerId);
            // The pane is a column so an inline search field can sit above the list inside it
            // (see setSearch); the list itself takes the remaining height.
            listColumn = new LinearLayout(ctx);
            listColumn.setOrientation(VERTICAL);
            listColumn.addView(listPane, new LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f));
            split = new androidx.slidingpanelayout.widget.SlidingPaneLayout(ctx) {
                @Override protected void onLayout(boolean changed, int l, int t, int r, int b) {
                    super.onLayout(changed, l, t, r, b);
                    // A child's remeasure can change isSlideable without changing this
                    // view's bounds. OnLayoutChangeListener misses that pass, leaving a
                    // compact destination with tiled chrome (and no Up button).
                    syncPresentation();
                    syncPane(depth());
                    syncSectionBack();
                }
            };
            // The navigation model owns which pane is visible. A free pane drag can
            // otherwise reveal the feed list without popping its logical destination.
            split.setLockMode(androidx.slidingpanelayout.widget.SlidingPaneLayout.LOCK_MODE_LOCKED);
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
            // Before weights are distributed, so a zero-width detail always "fits" — a portrait
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
            // hook and once more next tick — resync() is idempotent (knownEntries guard).
            @Override public void onBackStackChanged() {
                resync();
                pages.post(resyncRunnable);
            }
            @Override public void onBackStackChangeCommitted(
                    androidx.fragment.app.Fragment f, boolean pop) {
                // This hook precedes the settled history/view state. Reconcile only
                // after FragmentManager completes its transaction.
                pages.post(resyncRunnable);
            }
        });
        if (split != null) {
            sectionBackCallback = new OnBackPressedCallback(false) {
                @Override public void handleOnBackPressed() {
                    DayBridge.nativeOnEvent(hostNode, DayBridge.K_NAV_BACK, 0.0, null);
                }
            };
            ((FragmentActivity) ctx).getOnBackPressedDispatcher()
                    .addCallback(sectionBackCallback);
        }
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
     *  (gesture, back button, up arrow) are reported to Rust on the next main-loop tick:
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
        // Before the first measure, isSlideable is false even on a phone. Use the
        // same conservative stacked default as the model until onLayout reports
        // the actual presentation, then keep all chrome on that settled answer.
        return split == null || lastSlideable;
    }

    /** Navigation controls must not inherit the root search area's scroll offset.
     * Keep the same MaterialToolbar (and its actions) outside CoordinatorLayout on
     * destinations and tiled layouts. Only the compact root lends it to AppBarLayout.
     * This structural separation also protects Up from a late root fling/snap callback. */
    private void syncToolbarPlacement(int depth) {
        boolean pinned = depth > 0 || !stacked();
        ViewGroup parent = (ViewGroup) toolbar.getParent();
        ViewGroup target = pinned ? this : appBar;
        if (parent == target) return;
        parent.removeView(toolbar);
        if (pinned) {
            addView(toolbar, indexOfChild(scrollingChrome), new LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
            toolbar.setBackgroundColor(themeColor(
                    com.google.android.material.R.attr.colorSurface, 0xFFFFFFFF));
        } else {
            appBar.addView(toolbar, 0, new AppBarLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
            toolbar.setBackgroundColor(android.graphics.Color.TRANSPARENT);
        }
        appBar.setExpanded(true, false);
    }

    private void syncSearchVisibility(int depth) {
        if (searchField != null) {
            // Tiled, the list it filters never leaves the screen, so the field stays too.
            boolean show = depth == 0 || !stacked();
            searchField.setVisibility(show ? View.VISIBLE : View.GONE);
            if (scrollingChrome != null) {
                syncToolbarPlacement(depth);
                boolean scroll = show && !searchEditing();
                int flags = scroll ? AppBarLayout.LayoutParams.SCROLL_FLAG_SCROLL
                        | AppBarLayout.LayoutParams.SCROLL_FLAG_ENTER_ALWAYS
                        | AppBarLayout.LayoutParams.SCROLL_FLAG_SNAP : 0;
                for (View child : new View[] { toolbar, searchField }) {
                    if (child.getParent() != appBar) continue;
                    AppBarLayout.LayoutParams lp = (AppBarLayout.LayoutParams) child.getLayoutParams();
                    if (lp.getScrollFlags() != flags) { lp.setScrollFlags(flags); child.setLayoutParams(lp); }
                }
                AppBarLayout.LayoutParams searchParams = (AppBarLayout.LayoutParams) searchField.getLayoutParams();
                int width = !stacked() ? Math.round(NAV_SIDEBAR_DP * getResources().getDisplayMetrics().density)
                        : ViewGroup.LayoutParams.MATCH_PARENT;
                if (searchParams.width != width) { searchParams.width = width; searchField.setLayoutParams(searchParams); }
                if (!scroll || show != scrollingSearchShown) appBar.setExpanded(true, false);
                scrollingSearchShown = show;
            }
        }
    }

    /** Whether the user is in the middle of a search: the search view is up, or the pinned box's
     *  editor holds focus. The app bar must not scroll away under either. */
    private boolean searchEditing() {
        return searchView != null ? searchShowing() : searchEdit != null && searchEdit.hasFocus();
    }

    private boolean searchShowing() {
        if (searchView == null) return false;
        com.google.android.material.search.SearchView.TransitionState state =
                searchView.getCurrentTransitionState();
        return state == com.google.android.material.search.SearchView.TransitionState.SHOWING
                || state == com.google.android.material.search.SearchView.TransitionState.SHOWN;
    }

    /** Collapse the search view, for a navigation that leaves the list it searches. */
    private void dismissSearchView() {
        if (searchShowing()) searchView.hide();
    }

    /**
     * Slide back to the list once the stack is empty.
     *
     * `push` opens the detail pane. Every return to the root explicitly closes it,
     * including model-driven navigation and layout changes. Free pane dragging is locked:
     * revealing the sidebar must also update the application's selected section.
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
        while (knownEntries > now) {
            knownEntries--;
            int index = knownEntries + 1 + (split != null ? 1 : 0);
            final PageFragment popped = index < frags.size() ? frags.get(index) : null;
            if (!titles.isEmpty()) titles.remove(titles.size() - 1);
            if (!immersives.isEmpty()) immersives.remove(immersives.size() - 1);
            if (pendingPops > 0) {
                pendingPops--;
            } else {
                pages.post(new Runnable() {
                    @Override public void run() {
                        // A deferred callback for a released page must not pop a new one.
                        if (popped == null || !frags.contains(popped)) return;
                        nativePoppedPages.add(popped);
                        // kind 5 = NavBack; num 1.0 = the native container already popped.
                        DayBridge.nativeOnEvent(hostNode, DayBridge.K_NAV_BACK, 1.0, null);
                    }
                });
            }
        }
        knownEntries = now;
        syncPane(depth());
        syncChrome();
    }

    int depth() {
        return titles.size();
    }

    /** Whether a system back would do something here — pop a page, or ask an armed guard —
     *  rather than fall through to the activity (DayBridge.nativeBack). */
    boolean canBack() {
        return guarded || myEntries() > 0 || (stacked() && depth() > 0);
    }

    private void syncSectionBack() {
        if (sectionBackCallback != null) {
            sectionBackCallback.setEnabled(isShown() && stacked() && !guarded
                    && titles.size() == 1 && myEntries() == 0);
        }
    }

    @Override protected void onVisibilityChanged(View changed, int visibility) {
        super.onVisibilityChanged(changed, visibility);
        syncSectionBack();
    }

    @Override protected void onAttachedToWindow() {
        super.onAttachedToWindow();
        syncSectionBack();
        DayBridge.navHostAttached(this);
    }

    @Override protected void onDetachedFromWindow() {
        DayBridge.navHostDetached(this);
        super.onDetachedFromWindow();
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
        syncSectionBack();
    }

    /** Live retitle of the current top (`NavPatch::Title`): the root title when nothing is
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
     * Install the inline search over the navigation list (docs/search.md).
     *
     * Two forms. With scrolling chrome (`Search::reveal(Scrolls)` or `OnPull`), it is Material's
     * own search: a {@code SearchBar} under the title that scrolls away with it and returns on a
     * pull, and the full-screen {@code SearchView} it expands into when tapped
     * ({@link #installScrollingSearch}). Pinned, it is a Material `TextInputLayout` with a search
     * icon and a clear button, filtering the list in place under a bar that stays put: the list
     * underneath is the result set, and a pinned field reads as part of it.
     */
    void setSearch(final long id, String prompt, String text, boolean hidesOnScroll,
            boolean initiallyHidden) {
        if (searchField != null) {
            return; // already installed
        }
        if (hidesOnScroll && !DayActivity.edgeToEdge) {
            installScrollingSearch(id, prompt, text);
            syncSearchVisibility(depth());
            if (initiallyHidden) {
                // `SearchReveal::OnPull`: the bar starts scrolled away with the title row, and
                // the first scroll up brings it back, Material's enter-always behavior. After
                // the first layout, since a collapse before the app bar has a height is lost.
                appBar.post(() -> appBar.setExpanded(false, false));
            }
            return;
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
        // The keyboard's Search key (or a hardware Enter) submits the text (docs/search.md):
        // num 1 = submitted. Every change already went out from the watcher above, so this
        // follows the last of them. The keyboard goes away, as after any search.
        edit.setOnEditorActionListener((v, actionId, event) -> {
            boolean enter = event != null && event.getKeyCode() == android.view.KeyEvent.KEYCODE_ENTER;
            if (actionId != android.view.inputmethod.EditorInfo.IME_ACTION_SEARCH && !enter) {
                return false;
            }
            if (event != null && event.getAction() != android.view.KeyEvent.ACTION_DOWN) {
                return true; // the Enter key's release, after its press already submitted
            }
            DayBridge.nativeOnEvent(id, DayBridge.K_SEARCH_CHANGED, 1.0, v.getText().toString());
            android.view.inputmethod.InputMethodManager keyboard =
                    (android.view.inputmethod.InputMethodManager) getContext().getSystemService(
                            android.content.Context.INPUT_METHOD_SERVICE);
            if (keyboard != null) keyboard.hideSoftInputFromWindow(v.getWindowToken(), 0);
            return true;
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
        searchField = box;
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
        syncSearchVisibility(depth());
    }

    /**
     * Scrolling chrome: Material's search bar in the app bar, and its search view.
     *
     * The app bar (title, actions, and the search bar under them) rides a `CoordinatorLayout`
     * with scroll/enter-always/snap flags, so it scrolls away as the root list scrolls and comes
     * back on the first pull, the behavior Material gives a search bar in a top app bar. Only
     * nested scrolls that start in the root list move it (see the behavior below), never a
     * destination page's.
     *
     * The search bar is a launcher, not a field: tapping it expands the search view over the
     * whole host, keyboard up, and the query is typed there. Material's search view shows its own
     * results, and here the results ARE the navigation list the query filters, so the list is
     * lent to the search view while it is up (its view moves into the search view's content,
     * fragment and all) and returned to its pane as the view collapses. Choosing a row navigates,
     * which collapses the view as well. The query stays in the search bar's text when the view
     * collapses, so the pill reads as the active filter, and the view's own clear button empties
     * it.
     */
    private void installScrollingSearch(final long id, String prompt, String text) {
        Context ctx = getContext();
        int pad = (int) TypedValue.applyDimension(
                TypedValue.COMPLEX_UNIT_DIP, 12f, getResources().getDisplayMetrics());
        String hint = prompt == null ? "" : prompt;
        final com.google.android.material.search.SearchBar bar =
                new com.google.android.material.search.SearchBar(ctx);
        bar.setHint(hint);
        final com.google.android.material.search.SearchView view =
                new com.google.android.material.search.SearchView(ctx);
        view.setHint(hint);
        view.setAutoShowKeyboard(true);
        final EditText edit = view.getEditText();
        edit.setImeOptions(android.view.inputmethod.EditorInfo.IME_ACTION_SEARCH);
        if (text != null && !text.isEmpty()) {
            bar.setText(text);
            edit.setText(text);
        }
        edit.addTextChangedListener(new TextWatcher() {
            @Override public void beforeTextChanged(CharSequence s, int a, int b, int c) {}
            @Override public void onTextChanged(CharSequence s, int a, int b, int c) {}
            @Override public void afterTextChanged(Editable e) {
                bar.setText(e.toString());
                if (!searchSyncing) {
                    DayBridge.nativeOnEvent(id, DayBridge.K_SEARCH_CHANGED, 0.0, e.toString());
                }
            }
        });
        // The keyboard's Search key submits (num 1 = submitted; docs/search.md) and collapses the
        // view onto the filtered list, which is what the query was for.
        edit.setOnEditorActionListener((v, actionId, event) -> {
            boolean enter = event != null && event.getKeyCode() == android.view.KeyEvent.KEYCODE_ENTER;
            if (actionId != android.view.inputmethod.EditorInfo.IME_ACTION_SEARCH && !enter) {
                return false;
            }
            if (event != null && event.getAction() != android.view.KeyEvent.ACTION_DOWN) {
                return true;
            }
            DayBridge.nativeOnEvent(id, DayBridge.K_SEARCH_CHANGED, 1.0, v.getText().toString());
            view.hide();
            return true;
        });

        scrollingContent = split != null ? split : pages;
        removeView(appBar);
        removeView(scrollingContent);
        scrollingChrome = new androidx.coordinatorlayout.widget.CoordinatorLayout(ctx);
        androidx.coordinatorlayout.widget.CoordinatorLayout.LayoutParams contentParams =
                new androidx.coordinatorlayout.widget.CoordinatorLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT);
        contentParams.setBehavior(new AppBarLayout.ScrollingViewBehavior());
        scrollingChrome.addView(scrollingContent, contentParams);
        androidx.coordinatorlayout.widget.CoordinatorLayout.LayoutParams barParams =
                new androidx.coordinatorlayout.widget.CoordinatorLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        barParams.setBehavior(new AppBarLayout.Behavior() {
            @Override public boolean onStartNestedScroll(
                    androidx.coordinatorlayout.widget.CoordinatorLayout parent, AppBarLayout child,
                    View directTargetChild, View target, int axes, int type) {
                View feedPage = rootList();
                View current = target;
                while (current != feedPage && current.getParent() instanceof View) {
                    current = (View) current.getParent();
                }
                return current == feedPage && (depth() == 0 || !stacked())
                        && searchField.getVisibility() == View.VISIBLE
                        && !searchEditing()
                        && super.onStartNestedScroll(parent, child, directTargetChild, target, axes, type);
            }
        });
        scrollingChrome.addView(appBar, barParams);
        AppBarLayout.LayoutParams searchParams = new AppBarLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        searchParams.setMargins(pad, pad / 2, pad, pad / 2);
        appBar.addView(bar, searchParams);
        // After the app bar, so the expanded view draws over it.
        scrollingChrome.addView(view, new androidx.coordinatorlayout.widget.CoordinatorLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        view.setupWithSearchBar(bar);
        addView(scrollingChrome, new LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f));
        appBar.addOnOffsetChangedListener((b, offset) -> {
            chromeOffset = offset;
            sizeScrollingContent();
        });
        scrollingChrome.addOnLayoutChangeListener((v, l, t, r, b, ol, ot, or, ob) -> sizeScrollingContent());
        view.addTransitionListener((sv, previous, now) -> {
            switch (now) {
                case SHOWING:
                    lendListToSearchView();
                    break;
                case HIDING:
                    returnListFromSearchView();
                    break;
                default:
                    break;
            }
            if (searchBackCallback != null) {
                searchBackCallback.setEnabled(
                        now == com.google.android.material.search.SearchView.TransitionState.SHOWING
                        || now == com.google.android.material.search.SearchView.TransitionState.SHOWN);
            }
            syncSearchVisibility(depth());
        });
        searchBackCallback = new OnBackPressedCallback(false) {
            @Override public void handleOnBackPressed() {
                view.hide();
            }
        };
        ((FragmentActivity) ctx).getOnBackPressedDispatcher().addCallback(searchBackCallback);
        searchBar = bar;
        searchView = view;
        searchField = bar;
        searchEdit = edit;
    }

    /** The root list the search filters: the list pane of an adaptive host, else the pages. */
    private View rootList() {
        return listPane != null ? listPane : pages;
    }

    /** Move the root list into the search view's content, remembering where it came from. */
    private void lendListToSearchView() {
        View list = rootList();
        ViewGroup parent = (ViewGroup) list.getParent();
        if (parent == null || listHome != null) return;
        listHome = parent;
        listHomeIndex = parent.indexOfChild(list);
        listHomeParams = list.getLayoutParams();
        parent.removeView(list);
        searchView.addView(list, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
    }

    /** Put the root list back where {@link #lendListToSearchView} found it. */
    private void returnListFromSearchView() {
        View list = rootList();
        if (listHome == null) return;
        ViewGroup parent = (ViewGroup) list.getParent();
        if (parent != null) parent.removeView(list);
        listHome.addView(list, Math.min(listHomeIndex, listHome.getChildCount()), listHomeParams);
        listHome = null;
    }

    /** Keep pinned page footers inside the viewport as the app bar grows/shrinks. */
    private void sizeScrollingContent() {
        if (scrollingChrome.getHeight() <= 0) return;
        int height = Math.max(0, scrollingChrome.getHeight() - appBar.getHeight() - chromeOffset);
        ViewGroup.LayoutParams lp = scrollingContent.getLayoutParams();
        if (lp.height != height) { lp.height = height; scrollingContent.setLayoutParams(lp); }
    }

    void focusSearch() {
        if (searchEdit == null || searchField.getVisibility() != View.VISIBLE) return;
        if (searchView != null) {
            if (!searchShowing()) searchView.show();
            return;
        }
        searchEdit.requestFocus();
        searchEdit.selectAll();
        android.view.inputmethod.InputMethodManager keyboard =
            (android.view.inputmethod.InputMethodManager) getContext().getSystemService(
                android.content.Context.INPUT_METHOD_SERVICE);
        if (keyboard != null) keyboard.showSoftInput(searchEdit,
            android.view.inputmethod.InputMethodManager.SHOW_IMPLICIT);
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
        if (searchBar != null) searchBar.setText(next);
        searchSyncing = false;
    }

    /** Set the window toolbar (docs/toolbars.md) from day-mdc's `serialize_toolbar` spec and
     *  repaint the app bar. The spec is retained because the bar's menu is shared with the page's
     *  own bar actions, and either side changing repaints both. */
    void setWindowToolbar(String spec) {
        windowBar.setSpec(spec);
        paintBarMenu();
    }

    /** Paint the app bar's menu: the window toolbar's items, then the page's own bar actions.
     *
     *  One bar per window, the app bar, at every width — the same place day-uikit puts it
     *  (docs/toolbars.md). Day used to dock a second MaterialToolbar under the pages, which read
     *  as a phone's bottom bar and, tiled on a tablet, as a strip of icons stranded below both
     *  panes with the titled bar above them empty. An action belongs in the top app bar on
     *  Android, and what the bar cannot fit belongs in its overflow.
     *
     *  The toolbar's items lead and the bar actions trail, so a page's own action keeps the
     *  position it had before the window had a toolbar at all. {@link WindowBar#paint} lays the
     *  window's own items down and answers where they end. */
    private void paintBarMenu() {
        windowBar.paint();
        syncBarActions();
    }

    /** A targeted change to one live item: op 0 = enabled, 1 = toggle on, 2 = segment index,
     *  3 = focus the search field. */
    void updateWindowToolbar(String id, int op, double num) {
        windowBar.update(id, op, num);
    }

    /** The app wrote a page search field's text (docs/search.md). */
    void updateWindowToolbarText(String id, String text) {
        windowBar.updateText(id, text);
    }

    /** The window toolbar (docs/toolbars.md) painted onto one Material bar: the spec day-mdc
     *  serializes, the live items it produced, and the targeted updates that follow.
     *
     *  Written once and used twice. A window whose content is a navigation host paints it onto
     *  the host's own app bar, ahead of the page's `bar_action`s. A window with NO navigation
     *  host — one canvas or one form filling the window — gets a bar of its own across the top
     *  ({@link #dock}), because `Cap::Toolbar` answered `Native` before the app built that
     *  window and the app skipped its in-content strip on that answer; dropping the items would
     *  leave it with no commands at all. day-uikit gives such a window a navigation bar the same
     *  way. */
    static final class WindowBar {
        private final MaterialToolbar toolbar;
        /** The bar's own container when this is a docked bar; null on a nav host's app bar. */
        private AppBarLayout docked;
        /** The spec as the app last set it, retained so the menu can be repainted from it
         *  whenever the page's own bar actions change. Empty until an app sets one. */
        private String spec = "";
        /** Live toolbar items by their day id, for targeted updates. */
        final HashMap<String, MenuItem> items = new HashMap<>();
        /** Each segmented item's per-segment glyphs, in order, so a change of selection can
         *  lend the new segment's art to the control's own item. */
        final HashMap<String, List<android.graphics.drawable.Drawable>> segmentGlyphMap = new HashMap<>();
        /** The color the bar's glyphs were last tinted to; 0 before the first tint. */
        int lastTint = 0;
        /** A segmented item's segments, in order, for `updateWindowToolbar` op 2. */
        final HashMap<String, List<MenuItem>> segmentItems = new HashMap<>();
        /** A segmented item's declared label, empty when it brought none — see `nameSegmentHead`. */
        final HashMap<String, String> segmentLabels = new HashMap<>();
        /** Each item's UNTINTED glyph. The items share a bar with whatever else is on it, so they
         *  need the same re-tint against what is behind them — a bundled glyph is authored dark
         *  and vanishes on a dark bar. */
        final HashMap<MenuItem, android.graphics.drawable.Drawable> glyphs = new HashMap<>();

        // ── The page's search field (docs/search.md, "Page search") ─────────────────────
        //
        // A collapsible `SearchView` action: the magnifier in the bar, which expands into a
        // field across the whole app bar, with the toolbar's own collapse arrow and the system
        // back to close it. Collapsing clears the query, as a mail app's search does, so a
        // collapsed field never hides a filter still in force.
        //
        // The one item on this bar with state of its own (the user's typing, the caret, the
        // keyboard), so it is the one item a repaint does not rebuild: the MenuItem and its
        // SearchView stay while the spec carries a search record, and leave with it.

        /** Menu item id of the search action, the only item on the bar with one (the others
         *  are `Menu.NONE`), so a repaint can remove everything else around it. */
        private static final int SEARCH_ITEM = 0x5EA4C4;
        private MenuItem searchItem;
        private androidx.appcompat.widget.SearchView searchView;
        /** The search item's dispatch action, current as of the last paint. */
        private long searchAction;
        /** The text Day holds for the field: what the spec or a patch last said, or what the
         *  field last reported. A change equal to it is not sent again. */
        private String searchText = "";
        /** Day is writing the field (or collapsing it as its item leaves): its listeners stay
         *  quiet. */
        private boolean searchSyncing;
        /** Closes the expanded field on the system back, ahead of the navigation's own back,
         *  while the field is open. A toolbar that is not the activity's action bar does not
         *  collapse its action view on back by itself. */
        private androidx.activity.OnBackPressedCallback searchBack;

        WindowBar(MaterialToolbar toolbar) {
            this.toolbar = toolbar;
        }

        void setSpec(String spec) {
            this.spec = spec == null ? "" : spec;
        }

        /** Paint the spec into the bar's (cleared) menu.
         *
         *  The spec is `\u001e` between items, `\u001f` between fields (id, kind, label, icon,
         *  enabled, action, extra). Buttons and toggles become actions; a menu becomes a submenu
         *  built like the app menu; a segmented item becomes a submenu of radio choices; spaces
         *  and separators are the bar's own business and are skipped. */
        void paint() {
            Menu menu = toolbar.getMenu();
            if (searchItem == null) {
                menu.clear();
            } else {
                // Everything but the search action, which keeps its field (see above).
                for (int i = menu.size() - 1; i >= 0; i--) {
                    MenuItem it = menu.getItem(i);
                    if (it != searchItem) menu.removeItem(it.getItemId());
                }
            }
            boolean searched = false;
            items.clear();
            segmentItems.clear();
            segmentLabels.clear();
            segmentGlyphMap.clear();
            glyphs.clear();
            Context ctx = toolbar.getContext();
            // Order 0 is the search action's: the first action after the title, where Android
            // apps keep it.
            int order = 1;
            int groupSeq = 1;
            for (String rec : spec.split("\u001e", -1)) {
                if (rec.isEmpty()) continue;
                String[] f = rec.split("\u001f", -1);
                String id = f[0];
                String kind = f.length > 1 ? f[1] : "";
                final String label = f.length > 2 ? f[2] : "";
                String icon = f.length > 3 ? f[3] : "";
                boolean enabled = f.length > 4 && f[4].equals("1");
                final long action = f.length > 5 ? parseActionId(f[5]) : 0L;
                String extra = f.length > 6 ? f[6] : "";
                // The item's placement (docs/toolbars.md). `Primary` is what a Material app bar
                // shows as an action come what may; `Secondary` is what it folds into the
                // overflow first; `Automatic` keeps the old rule — an icon earns a slot IF ROOM,
                // and a wordless item would be a blank square, so it goes to the overflow.
                String placement = f.length > 7 ? f[7] : "auto";
                android.graphics.drawable.Drawable glyph = DayBridge.drawableByName(ctx, icon);
                if (kind.equals("button") || kind.equals("toggle")) {
                    MenuItem it = menu.add(Menu.NONE, Menu.NONE, order++, label);
                    it.setEnabled(enabled);
                    showAsAction(it, glyph, placement);
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
                    items.put(id, it);
                } else if (kind.equals("menu")) {
                    SubMenu sm = menu.addSubMenu(Menu.NONE, Menu.NONE, order++, label);
                    MenuItem it = sm.getItem();
                    it.setEnabled(enabled);
                    showAsAction(it, glyph, placement);
                    DayBridge.buildMenu(sm, extra);
                    items.put(id, it);
                } else if (kind.equals("segmented")) {
                    String[] seg = extra.split("\u001d", -1);
                    int selected = 0;
                    try {
                        selected = Integer.parseInt(seg[0]);
                    } catch (NumberFormatException e) {
                        // an unreadable index selects the first segment
                    }
                    SubMenu sm = menu.addSubMenu(Menu.NONE, Menu.NONE, order++, label);
                    final MenuItem head = sm.getItem();
                    head.setEnabled(enabled);
                    // Each segment is `title` 0x1C `icon`. The control itself carries no glyph;
                    // the segment in force lends it one, so the bar shows the setting as an icon
                    // button that opens the choices — the Material shape of a chooser.
                    final ArrayList<android.graphics.drawable.Drawable> segmentGlyphs = new ArrayList<>();
                    for (int i = 1; i < seg.length; i++) {
                        int cut = seg[i].indexOf('\u001c');
                        String segIcon = cut >= 0 ? seg[i].substring(cut + 1) : "";
                        seg[i] = cut >= 0 ? seg[i].substring(0, cut) : seg[i];
                        segmentGlyphs.add(DayBridge.drawableByName(ctx, segIcon));
                    }
                    if (glyph == null && selected >= 0 && selected < segmentGlyphs.size()) {
                        glyph = segmentGlyphs.get(selected);
                    }
                    showAsAction(head, glyph, placement);
                    segmentGlyphMap.put(id, segmentGlyphs);
                    final ArrayList<MenuItem> segments = new ArrayList<>();
                    // A segmented control carries no label of its own — it is a row of choices, and
                    // the platforms that draw one draw the choices (day-pieces `toolbar_segmented`).
                    // Folded into a menu there is no row to draw them on, and a submenu must be
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
                                lendSegmentGlyph(head, segmentGlyphs, idx);
                                DayBridge.nativeOnEvent(action, DayBridge.K_TOOLBAR_CHANGED,
                                        idx, "sel");
                                return true;
                            }
                        });
                        segments.add(s);
                    }
                    sm.setGroupCheckable(group, true, true);
                    items.put(id, head);
                    segmentItems.put(id, segments);
                    segmentLabels.put(id, label);
                    nameSegmentHead(head, segments, label);
                } else if (kind.equals("label")) {
                    MenuItem it = menu.add(Menu.NONE, Menu.NONE, order++, label);
                    it.setEnabled(false);
                    it.setShowAsAction(MenuItem.SHOW_AS_ACTION_NEVER);
                    items.put(id, it);
                } else if (kind.equals("search") && !searched) {
                    searched = true;
                    int cut = extra.indexOf('\u001d');
                    String text = cut >= 0 ? extra.substring(0, cut) : extra;
                    String hint = cut >= 0 ? extra.substring(cut + 1) : label;
                    paintSearch(menu, ctx, label, hint, text, glyph, enabled, action);
                    items.put(id, searchItem);
                }
                // "sep", "space", "flex": the app bar spaces its own actions.
            }
            if (!searched) dropSearch(menu);
        }

        /** Put the page's search action on the bar, or bring the one already there up to date. */
        private void paintSearch(Menu menu, Context ctx, String label, String hint, String text,
                android.graphics.drawable.Drawable glyph, boolean enabled, long action) {
            searchAction = action;
            if (searchItem == null) {
                final androidx.appcompat.widget.SearchView sv =
                        new androidx.appcompat.widget.SearchView(ctx);
                sv.setIconifiedByDefault(true);
                sv.setMaxWidth(Integer.MAX_VALUE);
                sv.setImeOptions(android.view.inputmethod.EditorInfo.IME_ACTION_SEARCH
                        | android.view.inputmethod.EditorInfo.IME_FLAG_NO_EXTRACT_UI);
                sv.setInputType(android.text.InputType.TYPE_CLASS_TEXT);
                sv.setOnQueryTextListener(
                        new androidx.appcompat.widget.SearchView.OnQueryTextListener() {
                    @Override public boolean onQueryTextChange(String q) {
                        if (!searchSyncing) sendSearchText(q);
                        return true;
                    }
                    @Override public boolean onQueryTextSubmit(String q) {
                        // The text first, if the field has not reported it, then the
                        // submission (docs/search.md); the keyboard then goes away, as after
                        // any search, and the field stays open with the query that ran.
                        sendSearchText(q);
                        DayBridge.nativeOnEvent(searchAction, DayBridge.K_TOOLBAR_CHANGED, 0.0,
                                "submit:" + q);
                        sv.clearFocus();
                        return true;
                    }
                });
                MenuItem it = menu.add(Menu.NONE, SEARCH_ITEM, 0, label);
                it.setActionView(sv);
                it.setShowAsAction(MenuItem.SHOW_AS_ACTION_IF_ROOM
                        | MenuItem.SHOW_AS_ACTION_COLLAPSE_ACTION_VIEW);
                it.setOnActionExpandListener(new MenuItem.OnActionExpandListener() {
                    @Override public boolean onMenuItemActionExpand(MenuItem mi) {
                        armSearchBack(true);
                        return true;
                    }
                    @Override public boolean onMenuItemActionCollapse(MenuItem mi) {
                        // The SearchView empties itself as it collapses, and that change goes
                        // out like any other: closing the field clears the search.
                        armSearchBack(false);
                        return true;
                    }
                });
                searchItem = it;
                searchView = sv;
                // A field arriving on the bar shows whatever Day already holds for it.
                searchText = "";
            }
            searchItem.setTitle(label);
            searchItem.setEnabled(enabled);
            searchView.setQueryHint(hint);
            if (glyph != null) {
                glyphs.put(searchItem, glyph);
                searchItem.setIcon(glyph);
            }
            showSearchText(text);
        }

        /** The field reported `q`: send it, unless Day holds that text already. */
        private void sendSearchText(String q) {
            if (q.equals(searchText)) return;
            searchText = q;
            DayBridge.nativeOnEvent(searchAction, DayBridge.K_TOOLBAR_CHANGED, 0.0, "text:" + q);
        }

        /** Show Day's text in the field without the field reporting it back. A non-empty query
         *  opens the field, so the query in force is always on screen (a page coming back with
         *  its search still applied, or the app setting one); an empty one closes a field the
         *  user is not typing in. Opening it this way does not raise the keyboard. */
        private void showSearchText(String text) {
            if (searchItem == null) return;
            String next = text == null ? "" : text;
            searchText = next;
            searchSyncing = true;
            try {
                if (!next.isEmpty() && !searchItem.isActionViewExpanded()) {
                    searchItem.expandActionView();
                    searchView.setQuery(next, false);
                    searchView.clearFocus();
                } else if (next.isEmpty() && searchItem.isActionViewExpanded()
                        && !searchView.hasFocus()) {
                    searchItem.collapseActionView();
                } else if (!next.contentEquals(searchView.getQuery())) {
                    searchView.setQuery(next, false);
                }
            } finally {
                searchSyncing = false;
            }
        }

        /** The spec no longer has a search record (its page left, or withdrew the field): take
         *  the action off the bar. Closing it here is not the user clearing the search, so it is
         *  not reported; the page's query stays the app's, and comes back with the page. */
        private void dropSearch(Menu menu) {
            if (searchItem == null) return;
            searchSyncing = true;
            try {
                searchItem.collapseActionView();
            } finally {
                searchSyncing = false;
            }
            armSearchBack(false);
            menu.removeItem(SEARCH_ITEM);
            glyphs.remove(searchItem);
            searchItem = null;
            searchView = null;
            searchText = "";
        }

        /** Hold the system back for the open field (true) or hand it back (false). Added at each
         *  opening, so it sits above every navigation callback registered before it. */
        private void armSearchBack(boolean on) {
            if (searchBack != null) {
                searchBack.remove();
                searchBack = null;
            }
            if (!on) return;
            Context c = toolbar.getContext();
            while (c instanceof android.content.ContextWrapper
                    && !(c instanceof androidx.activity.ComponentActivity)) {
                c = ((android.content.ContextWrapper) c).getBaseContext();
            }
            if (!(c instanceof androidx.activity.ComponentActivity)) return;
            searchBack = new androidx.activity.OnBackPressedCallback(true) {
                @Override public void handleOnBackPressed() {
                    if (searchItem != null) searchItem.collapseActionView();
                }
            };
            ((androidx.activity.ComponentActivity) c).getOnBackPressedDispatcher()
                    .addCallback(searchBack);
        }

        /** The app wrote the search field's text (ToolbarPatch::Text). */
        void updateText(String id, String text) {
            if (items.get(id) != null && items.get(id) == searchItem) showSearchText(text);
        }

        /** A targeted change to one live item: op 0 = enabled, 1 = toggle on, 2 = segment index. */
        void update(String id, int op, double num) {
            MenuItem it = items.get(id);
            if (it == null) return;
            if (op == 0) {
                it.setEnabled(num != 0.0);
            } else if (op == 3) {
                // Find (docs/search.md): open the field, keep its query, select it and raise
                // the keyboard.
                if (it != searchItem) return;
                searchSyncing = true;
                try {
                    if (!it.isActionViewExpanded()) {
                        it.expandActionView();
                        searchView.setQuery(searchText, false);
                    }
                } finally {
                    searchSyncing = false;
                }
                View field = searchView.findViewById(androidx.appcompat.R.id.search_src_text);
                if (field instanceof EditText) {
                    field.requestFocus();
                    ((EditText) field).selectAll();
                    android.view.inputmethod.InputMethodManager keyboard =
                            (android.view.inputmethod.InputMethodManager) field.getContext()
                                    .getSystemService(Context.INPUT_METHOD_SERVICE);
                    if (keyboard != null) keyboard.showSoftInput(field,
                            android.view.inputmethod.InputMethodManager.SHOW_IMPLICIT);
                }
            } else if (op == 1) {
                it.setChecked(num != 0.0);
                paintToggle(it);
            } else if (op == 2) {
                List<MenuItem> segments = segmentItems.get(id);
                if (segments != null) {
                    checkSegment(segments, (int) num);
                    // An unlabeled control is named by the segment in force, so a change the APP
                    // made has to rename it too — not only one the user tapped. Its glyph moves
                    // the same way.
                    nameSegmentHead(it, segments, segmentLabels.get(id));
                    List<android.graphics.drawable.Drawable> segGlyphs = segmentGlyphMap.get(id);
                    if (segGlyphs != null) {
                        lendSegmentGlyph(it, segGlyphs, (int) num);
                    }
                }
            }
        }

        /** Re-tint every glyph on this bar to `color`, the way {@link DayNavHost#syncBarActions}
         *  does for the app bar it shares with the page's own actions. */
        void tint(int color) {
            lastTint = color;
            for (java.util.Map.Entry<MenuItem, android.graphics.drawable.Drawable> e
                    : glyphs.entrySet()) {
                android.graphics.drawable.Drawable d = e.getValue().mutate();
                d.setTint(color);
                e.getKey().setIcon(d);
                // Tinting replaced the icon, so a toggle's on/off opacity has to be re-applied.
                if (e.getKey().isCheckable()) {
                    paintToggle(e.getKey());
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
        private void showAsAction(MenuItem it, android.graphics.drawable.Drawable glyph,
                String placement) {
            if ("secondary".equals(placement)) {
                // The app asked for this one to fold first (docs/toolbars.md). It reads as a
                // full menu row in the overflow, which is what a de-emphasized command wants.
                it.setShowAsAction(MenuItem.SHOW_AS_ACTION_NEVER);
                if (glyph != null) {
                    glyphs.put(it, glyph);
                    it.setIcon(glyph);
                }
                return;
            }
            if (glyph != null) {
                glyphs.put(it, glyph);
                it.setIcon(glyph);
                if ("primary".equals(placement)) {
                    // The one action this chrome is really offering: it keeps its slot however
                    // crowded the bar gets.
                    it.setShowAsAction(MenuItem.SHOW_AS_ACTION_ALWAYS);
                    return;
                }
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
        static void paintToggle(MenuItem it) {
            android.graphics.drawable.Drawable d = it.getIcon();
            if (d != null) d.setAlpha(it.isChecked() ? 255 : 110);
        }

        private static void checkSegment(List<MenuItem> segments, int idx) {
            for (int i = 0; i < segments.size(); i++) segments.get(i).setChecked(i == idx);
        }

        /** Put the segment in force's glyph on the control's own item, tinted like the rest of
         *  the bar, so the icon button says which choice is on. A segment without art leaves the
         *  head as it was. */
        private void lendSegmentGlyph(MenuItem head, List<android.graphics.drawable.Drawable> segGlyphs, int idx) {
            if (idx < 0 || idx >= segGlyphs.size()) return;
            android.graphics.drawable.Drawable g = segGlyphs.get(idx);
            if (g == null) return;
            android.graphics.drawable.Drawable prior = glyphs.get(head);
            glyphs.put(head, g);
            if (prior != null && lastTint != 0) {
                android.graphics.drawable.Drawable d = g.mutate();
                d.setTint(lastTint);
                head.setIcon(d);
            } else {
                head.setIcon(g);
            }
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

        /** The color a bar's glyphs take, derived from what is actually behind them: the bundled
         *  glyphs are authored dark, so a dark bar needs light ones. Relative luminance, Rec. 709. */
        static int glyphColorFor(int bg) {
            double lum = (0.2126 * android.graphics.Color.red(bg)
                    + 0.7152 * android.graphics.Color.green(bg)
                    + 0.0722 * android.graphics.Color.blue(bg)) / 255.0;
            return lum < 0.5 ? 0xFFFFFFFF : 0xFF1C1B1F;
        }

        // ── A window with no navigation host ────────────────────────────────────────────
        //
        // Keyed by the day root the bar belongs to, which is the handle `setWindowToolbar`
        // receives. Weak, because the activity owns both views and outlives nothing here.

        static final WeakHashMap<View, WindowBar> DOCKED = new WeakHashMap<>();

        /** Give this window a bar of its own across the top, or repaint the one it has. The day
         *  root gives up the strip: the activity's inset listener asks {@link #layoutDocked} for
         *  the height and starts the root below it, so the content the bar acts on is never
         *  underneath it. */
        static void dock(View root, String spec) {
            WindowBar bar = DOCKED.get(root);
            if (bar == null) {
                if (!(root.getParent() instanceof ViewGroup)) return;
                ViewGroup wrapper = (ViewGroup) root.getParent();
                Context ctx = root.getContext();
                MaterialToolbar tb = new MaterialToolbar(ctx);
                AppBarLayout appBar = new AppBarLayout(ctx);
                appBar.addView(tb, new AppBarLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
                wrapper.addView(appBar, new FrameLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT, barHeightPx(ctx)));
                bar = new WindowBar(tb);
                bar.docked = appBar;
                DOCKED.put(root, bar);
            }
            bar.setSpec(spec);
            bar.paint();
            bar.tint(glyphColorFor(themeColor(root.getContext(),
                    com.google.android.material.R.attr.colorSurface, 0xFFFFFFFF)));
            androidx.core.view.ViewCompat.requestApplyInsets((View) root.getParent());
        }

        /** Take this window's own bar away and give the strip back to the day root — when the app
         *  clears its toolbar, and when a navigation host appears to carry the items instead. */
        static void undock(View root) {
            WindowBar bar = DOCKED.remove(root);
            if (bar == null || bar.docked == null) return;
            ViewGroup parent = (ViewGroup) bar.docked.getParent();
            if (parent != null) {
                parent.removeView(bar.docked);
                androidx.core.view.ViewCompat.requestApplyInsets(parent);
            }
            bar.docked = null;
        }

        /** A targeted update for a window that has no navigation host. */
        static void updateDocked(View root, String id, int op, double num) {
            WindowBar bar = DOCKED.get(root);
            if (bar != null) bar.update(id, op, num);
        }

        /** A search field's text, for a window that has no navigation host. */
        static void updateDockedText(View root, String id, String text) {
            WindowBar bar = DOCKED.get(root);
            if (bar != null) bar.updateText(id, text);
        }

        /** Place the docked bar inside the current safe area and answer the height the day root
         *  owes it — 0 when this window has no bar of its own. Called from the activities' inset
         *  listeners, which own the root's margins, so the bar and the root are placed together. */
        static int layoutDocked(View root, int left, int top, int right) {
            WindowBar bar = DOCKED.get(root);
            if (bar == null || bar.docked == null) return 0;
            int h = barHeightPx(root.getContext());
            FrameLayout.LayoutParams lp =
                    (FrameLayout.LayoutParams) bar.docked.getLayoutParams();
            if (lp.leftMargin != left || lp.topMargin != top || lp.rightMargin != right
                    || lp.height != h) {
                lp.leftMargin = left;
                lp.topMargin = top;
                lp.rightMargin = right;
                lp.height = h;
                bar.docked.setLayoutParams(lp);
            }
            return h;
        }

        /** A Material top app bar's own height, from the theme (`actionBarSize`). */
        private static int barHeightPx(Context ctx) {
            TypedValue tv = new TypedValue();
            if (ctx.getTheme().resolveAttribute(
                    androidx.appcompat.R.attr.actionBarSize, tv, true)) {
                return TypedValue.complexToDimensionPixelSize(
                        tv.data, ctx.getResources().getDisplayMetrics());
            }
            return Math.round(56f * ctx.getResources().getDisplayMetrics().density);
        }

        private static int themeColor(Context ctx, int attr, int fallback) {
            TypedValue tv = new TypedValue();
            if (ctx.getTheme().resolveAttribute(attr, tv, true)) {
                if (tv.type >= TypedValue.TYPE_FIRST_COLOR_INT
                        && tv.type <= TypedValue.TYPE_LAST_COLOR_INT) {
                    return tv.data;
                }
                if (tv.resourceId != 0) {
                    return ctx.getColor(tv.resourceId);
                }
            }
            return fallback;
        }
    }

    /** Re-tint every glyph on the app bar — the window toolbar's and the page's own — to the bar's
     *  Current color, and hide the list-only actions off the list.
     *  Driven from {@link #syncChrome}, which already runs on every push, pop and re-present —
     *  the three moments that change what is behind these glyphs or which page they are on. */
    private void syncBarActions() {
        if (windowBar.glyphs.isEmpty()) {
            return;
        }
        windowBar.tint(barGlyphColor());
    }

    /** The color the bar's own glyphs take, derived from what is actually behind them.
     *
     *  Not a constant, and not "white when edge-to-edge": the app bar is `colorPrimary` under
     *  edge-to-edge, a dark scrim over an immersive page, and the theme's surface otherwise — and
     *  that surface follows the system light/dark setting. The bundled glyphs are authored dark,
     *  so leaving them untinted (which is what everything except the edge-to-edge case used to do)
     *  puts a black icon on a near-black bar in dark mode. */
    private int barGlyphColor() {
        return WindowBar.glyphColorFor(barBackgroundColor());
    }

    /** What the app bar is actually painted with right now — the same three cases
     *  {@link #syncChrome} paints, read back rather than re-derived. */
    private int barBackgroundColor() {
        if (DayActivity.edgeToEdge) {
            boolean topImmersive = !immersives.isEmpty() && immersives.get(immersives.size() - 1);
            // The immersive chrome is a black scrim gradient; the ordinary bar is the M3
            // surface tone, which is what lets it lift rather than stand apart.
            return topImmersive
                    ? 0xFF000000
                    : themeColor(com.google.android.material.R.attr.colorSurface, 0xFFFFFFFF);
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
        // num 1.0 = split (both panes), 0.0 = stacked.
        DayBridge.nativeOnEvent(hostNode, DayBridge.K_NAV_PRESENTATION, slideable ? 0.0 : 1.0, null);
    }

    private void syncChrome() {
        // Selection-only transactions intentionally do not enter FragmentManager history.
        // Reconcile search/scrolling here too, on EVERY navigation path, not just resync().
        syncSearchVisibility(depth());
        toolbar.setTitle(titles.isEmpty() ? rootTitle : titles.get(titles.size() - 1));
        // The tiled base section has its list alongside it. Nested pages still need Up
        // to return to that section; compact pages always need it to reach the root.
        showUpArrow(!titles.isEmpty() && (stacked() || titles.size() > 1));
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
                // Surface, not `colorPrimary`: an M3 top app bar shares the content's tone and
                // earns its separation by lifting (see the constructor). A primary-colored band
                // is the Material 2 look, and on a tiled tablet it reads as a stripe.
                appBar.setBackgroundColor(themeColor(
                        com.google.android.material.R.attr.colorSurface, 0xFFFFFFFF));
                appBar.setLiftOnScroll(true);
            }
        }
        // After the background above: the glyph color is derived from it, and the list-only items
        // depend on the depth this method just re-read.
        syncBarActions();
        syncSectionBack();
    }

    /** Register the Rust-owned page view. The root page becomes a fragment immediately; a
     *  pushed page parks in the container as a raw hidden child until push() presents it —
     *  the patch order is add-then-push, and keeping the view attached in between lets a nav
     *  host nested inside the page register its own container with the FragmentManager. */
    void add(View page) {
        page.setLayoutParams(new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        // The page moves as ONE view under the push/pop transition. Without this the
        // transition framework animates the page's leaf views one by one, and Day lays those
        // out (and a native row re-binds them) in the frames right after the push, which
        // clobbers the per-view animators: a page of native settings rows appeared with no
        // motion at all while a page of plain labels happened to fade.
        if (page instanceof ViewGroup) ((ViewGroup) page).setTransitionGroup(true);
        PageFragment f = new PageFragment(page);
        frags.add(f);
        pageHosts.put(page, this);
        if (frags.size() == 1) {
            // The first page. Adaptive host: the Pane::Sidebar page, the list pane's permanent
            // occupant in both presentations. Plain stack host: its root page, which lives in
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
        dismissSearchView();
        PageFragment top = frags.get(n - 1);
        PageFragment prev = frags.get(n - 2);
        View v = top.content;
        if (v.getParent() == pages) pages.removeView(v); // the fragment owns it from here
        v.setVisibility(View.VISIBLE);
        boolean skip = DayBridge.skipTransitions();
        top.setEnterTransition(skip ? null : new MaterialSharedAxis(MaterialSharedAxis.X, true));
        top.setReturnTransition(skip ? null : new MaterialSharedAxis(MaterialSharedAxis.X, false));
        prev.setExitTransition(skip ? null : new MaterialSharedAxis(MaterialSharedAxis.X, true));
        prev.setReenterTransition(skip ? null : new MaterialSharedAxis(MaterialSharedAxis.X, false));
        titles.add(title);
        immersives.add(immersive);
        androidx.fragment.app.FragmentTransaction transaction = fm.beginTransaction()
                .setReorderingAllowed(true).replace(containerId, top);
        // A selector's first destination is the currently selected section. Only
        // nested pages (article reader, editor, etc.) belong to FragmentManager history.
        if (split == null || titles.size() > 1) transaction.addToBackStack(prefix + titles.size());
        transaction.commitAllowingStateLoss();
        // Execute now (commitNow can't take a back stack): the entry must be registered
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
        if (!nativePoppedPages.isEmpty()) {
            nativePoppedPages.remove(0);
            return;
        }
        if (split != null && titles.size() == 1 && myEntries() == 0) {
            titles.clear();
            immersives.clear();
            syncPane(0);
            syncChrome();
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
                // Some Rust owners omit Popped after a native pop; others acknowledge
                // before removing the page. Either path must consume only THIS page.
                nativePoppedPages.remove(f);
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
            toolbar.setNavigationContentDescription(androidx.appcompat.R.string.abc_action_bar_up_description);
        } else {
            toolbar.setNavigationIcon(null);
            toolbar.setNavigationContentDescription(null);
        }
    }

    /** A fragment that retains and re-serves its Rust-owned page view (the
     *  react-native-screens pattern) — the FragmentManager owns when it shows, Day owns what
     *  it shows. Public with a no-arg constructor per the Fragment contract: the framework
     *  re-instantiates these on an activity recreation (the theme switch, docs/settings.md
     *  "Appearance on Android") with no content, and `DayBridge.beginMount` removes those restored shells before the
     *  new tree registers its own. */
    public static class PageFragment extends Fragment {
        View content;

        public PageFragment() {}

        PageFragment(View content) {
            this.content = content;
        }

        @Override public void onPause() {
            // replace/pop removes the outgoing fragment. An activity pause alone must
            // preserve editing (e.g. the user briefly switches apps or opens a permission UI).
            if (isRemoving()) DayKeyboard.endEditing(content);
            super.onPause();
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
