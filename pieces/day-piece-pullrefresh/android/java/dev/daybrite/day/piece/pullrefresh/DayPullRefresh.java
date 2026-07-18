// The day-piece-pullrefresh crate's OWN Android backend — bundled here and folded into the app's
// Gradle build via [package.metadata.day.android], which ALSO adds the AndroidX
// swiperefreshlayout dependency. Uses only day-android's PUBLIC Java surface: DayBridge.ctx and
// DayBridge.nativeOnEvent. SwipeRefreshLayout IS a ViewGroup, so DayBridge.addChild mounts the
// wrapped Day scrollable directly into it — the piece is a native CONTAINER (docs/extending.md).
package dev.daybrite.day.piece.pullrefresh;

import android.view.View;

import androidx.swiperefreshlayout.widget.SwipeRefreshLayout;

import dev.daybrite.day.bridge.DayBridge;

/** SwipeRefreshLayout wrapper reporting pull-begins via the open Custom-event kind (12). */
public final class DayPullRefresh {
    private DayPullRefresh() {}

    public static View makePullRefresh(long id, boolean refreshing) {
        SwipeRefreshLayout srl = new SwipeRefreshLayout(DayBridge.ctx);
        // kind 12 = a piece-defined Custom event (§8.2's open channel): a user pull began. The
        // front-end flips the bound `refreshing` signal; dismissal comes back as a command.
        srl.setOnRefreshListener(() -> DayBridge.nativeOnEvent(id, 12, 1.0, ""));
        if (refreshing) {
            srl.setRefreshing(true);
        }
        return srl;
    }

    /** Imperative command: show/dismiss the refresh indicator (idempotent). */
    public static void refreshCommand(View view, boolean on) {
        if (view instanceof SwipeRefreshLayout) {
            ((SwipeRefreshLayout) view).setRefreshing(on);
        }
    }
}
