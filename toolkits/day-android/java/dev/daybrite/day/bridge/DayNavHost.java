package dev.daybrite.day.bridge;

import android.content.Context;
import android.os.Bundle;
import android.view.LayoutInflater;
import android.view.View;
import android.view.ViewGroup;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.util.TypedValue;
import java.util.ArrayList;
import java.util.WeakHashMap;

import androidx.fragment.app.Fragment;
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
    /** page view → its host, for removePage routing even after the view is detached. */
    static final WeakHashMap<View, DayNavHost> pageHosts = new WeakHashMap<>();

    final MaterialToolbar toolbar;
    final FrameLayout pages;
    final long hostNode;
    final String rootTitle;
    private final FragmentManager fm;
    private final int containerId;
    /** This host's back-stack entry name prefix — several hosts share the activity's manager. */
    private final String prefix;
    private final ArrayList<PageFragment> frags = new ArrayList<>();
    private final ArrayList<String> titles = new ArrayList<>();
    /** Back-stack entries of ours the listener has already accounted for. */
    private int knownEntries;
    /** Pops the native side already performed — absorb the answering Popped patch. */
    private int nativePops;
    /** Pops we initiated via popBackStack — the listener must not re-report them. */
    private int pendingPops;

    public DayNavHost(Context ctx, long hostNode, String title) {
        super(ctx);
        setOrientation(VERTICAL);
        this.hostNode = hostNode;
        this.rootTitle = title;
        this.prefix = "day-nav-" + hostNode + "-";

        toolbar = new MaterialToolbar(ctx);
        toolbar.setTitle(title);
        toolbar.setNavigationOnClickListener(new OnClickListener() {
            @Override public void onClick(View v) {
                // Pop natively (animated); the back-stack listener reports it to Rust.
                if (myEntries() > 0) fm.popBackStack();
            }
        });
        AppBarLayout appBar = new AppBarLayout(ctx);
        appBar.addView(toolbar, new AppBarLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        addView(appBar, new LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT));

        pages = new FrameLayout(ctx);
        containerId = View.generateViewId();
        pages.setId(containerId);
        addView(pages, new LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f));

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
    private void resync() {
        int now = myEntries();
        while (knownEntries > now) {
            knownEntries--;
            if (!titles.isEmpty()) titles.remove(titles.size() - 1);
            if (pendingPops > 0) {
                pendingPops--;
            } else {
                pages.post(new Runnable() {
                    @Override public void run() {
                        nativePops++;
                        // kind 5 = NavBack; num 1.0 = the native container already popped.
                        DayBridge.nativeOnEvent(hostNode, 5, 1.0, null);
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

    private void syncChrome() {
        toolbar.setTitle(titles.isEmpty() ? rootTitle : titles.get(titles.size() - 1));
        showUpArrow(!titles.isEmpty());
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
            fm.beginTransaction().setReorderingAllowed(true)
                    .add(containerId, f).commitNowAllowingStateLoss();
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
    void push(String title) {
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
        fm.beginTransaction().setReorderingAllowed(true)
                .replace(containerId, top)
                .addToBackStack(prefix + titles.size())
                .commitAllowingStateLoss();
        // Execute NOW (commitNow can't take a back stack): the entry must be registered
        // before the next resync(), or the count mismatch reads as a phantom pop.
        fm.executePendingTransactions();
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
