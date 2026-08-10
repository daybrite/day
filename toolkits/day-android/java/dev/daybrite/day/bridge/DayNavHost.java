// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

package dev.daybrite.day.bridge;

import android.content.Context;
import android.os.Bundle;
import android.view.LayoutInflater;
import android.view.Menu;
import android.view.MenuItem;
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
            split = new androidx.slidingpanelayout.widget.SlidingPaneLayout(ctx);
            // A fixed-width list beside a weighted detail. SlidingPaneLayout tiles them when
            // both minimum widths fit and overlaps them when they do not — the whole adaptive
            // decision, made by the platform at measure time (docs/size-classes.md).
            float density = ctx.getResources().getDisplayMetrics().density;
            androidx.slidingpanelayout.widget.SlidingPaneLayout.LayoutParams lp =
                    new androidx.slidingpanelayout.widget.SlidingPaneLayout.LayoutParams(
                            Math.round(NAV_SIDEBAR_DP * density),
                            ViewGroup.LayoutParams.MATCH_PARENT);
            split.addView(listPane, lp);
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

    private void resync() {
        int now = myEntries();
        syncSearchVisibility(now);
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

    /** Add the trailing action item to the toolbar's menu: an always-visible icon button
     *  (docs/navigation.md). The MaterialToolbar keeps its menu across pushes/pops, so it rides
     *  every page's bar. Over the edge-to-edge blue app bar the glyph is tinted white; on the
     *  default light surface bar the bundled dark glyph reads as-is. Called by
     *  {@link DayBridge#setNavMenu} AFTER construction, inside a try/catch — never from the
     *  constructor — so a failure here can't blank the host. */
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
        // Directly under the app bar, so it reads as part of the list's own chrome.
        int at = indexOfChild(appBar) + 1;
        addView(box, at < 0 ? 0 : at);
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

    void setBarAction(String iconName, String label, final long actionId) {
        MenuItem it = toolbar.getMenu().add(Menu.NONE, 0, 0, label == null ? "" : label);
        it.setShowAsAction(MenuItem.SHOW_AS_ACTION_ALWAYS);
        android.graphics.drawable.Drawable icon = DayBridge.drawableByName(getContext(), iconName);
        if (icon != null) {
            if (DayActivity.edgeToEdge) {
                icon = icon.mutate();
                icon.setTint(0xFFFFFFFF);
            }
            it.setIcon(icon);
        }
        it.setOnMenuItemClickListener(new MenuItem.OnMenuItemClickListener() {
            @Override public boolean onMenuItemClick(MenuItem item) {
                DayBridge.nativeOnEvent(actionId, DayBridge.K_MENU_ACTION, 0.0, "");
                return true;
            }
        });
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
