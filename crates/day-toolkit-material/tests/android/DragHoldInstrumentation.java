// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
package dev.daybrite.day.bridge;

import android.app.Activity;
import android.app.Dialog;
import android.app.Instrumentation;
import android.content.Intent;
import android.os.Bundle;
import android.os.SystemClock;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewGroup;
import android.widget.ScrollView;
import dev.daybrite.day.piece.pullrefresh.DayPullRefresh;
import java.util.ArrayList;

/** A piece with a drag handler keeps its touch stream inside Day's scrolling containers: a
 *  vertical scroll view inside a pull-to-refresh. A drag declared along one axis keeps the
 *  streams that set off along it and leaves the others to a vertical and a horizontal scroll
 *  view. The touches are injected through the input pipeline (`sendPointerSync`), so each
 *  ancestor's own interception decides. */
public final class DragHoldInstrumentation extends Instrumentation {
    private int checks;
    private Dialog dialog;
    private View refresh;
    private ScrollView scroll;
    private Recorder drag, tap;
    private ScrollView columns;
    private android.widget.HorizontalScrollView rows;
    private Recorder sideways, upright;

    /** A view that notes the actions it is sent, cancels included. */
    private static final class Recorder extends View {
        final ArrayList<Integer> actions = new ArrayList<>();
        Recorder(Activity a, int color) { super(a); setBackgroundColor(color); }
        @Override public boolean dispatchTouchEvent(MotionEvent e) {
            actions.add(e.getActionMasked());
            return super.dispatchTouchEvent(e);
        }
        int count(int action) {
            int n = 0;
            for (int a : actions) if (a == action) n++;
            return n;
        }
    }

    private void check(boolean condition, String message) {
        if (!condition) throw new AssertionError(message);
        checks++;
    }
    @Override public void onCreate(Bundle args) { super.onCreate(args); start(); }
    @Override public void onStart() {
        Bundle result = new Bundle();
        try {
            Intent intent = getTargetContext().getPackageManager()
                    .getLaunchIntentForPackage(getTargetContext().getPackageName());
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            Activity activity = startActivitySync(intent);
            waitForIdleSync();
            runOnMainSync(() -> setup(activity));
            waitForIdleSync();
            SystemClock.sleep(500); // the dialog's window takes input once it has drawn
            try {
                exercise();
            } finally {
                runOnMainSync(() -> dialog.dismiss());
            }
            runOnMainSync(() -> axisSetup(activity));
            waitForIdleSync();
            SystemClock.sleep(500);
            try {
                axes();
            } finally {
                runOnMainSync(() -> dialog.dismiss());
            }
            result.putString("stream", "PASS Android native drag hold: " + checks + " assertions\n");
            finish(Activity.RESULT_OK, result);
        } catch (Throwable error) {
            result.putString("stream", android.util.Log.getStackTraceString(error));
            finish(Activity.RESULT_CANCELED, result);
        }
    }

    private void setup(Activity activity) {
        dialog = new Dialog(activity);
        // What Day builds for `pull_to_refresh(scroll(column(...)))`: its pull-to-refresh
        // around its scroll view around a DayFixed with the pieces placed in it.
        refresh = DayPullRefresh.makePullRefresh(0, false);
        scroll = (ScrollView) DayBridge.makeScroll(false);
        DayFixed content = (DayFixed) DayBridge.contentOf(scroll);
        drag = new Recorder(activity, 0xFF3060FF);
        tap = new Recorder(activity, 0xFF30C050);
        content.addView(drag);
        content.addView(tap);
        content.setChildFrame(drag, 0, 0, 600, 500);
        content.setChildFrame(tap, 0, 500, 600, 500);
        content.setContentSize(600, 4000);
        // Node 0 is no node: the reports go nowhere, and the listeners are what is under test.
        DayBridge.enableGesture(drag, 0, true);
        DayBridge.enableGesture(tap, 0, false);
        ((ViewGroup) refresh).addView(scroll, new ViewGroup.LayoutParams(600, 1000));
        dialog.setContentView(refresh, new ViewGroup.LayoutParams(600, 1000));
        dialog.show();
    }

    /** What Day builds for a row with `.on_drag_along(Horizontal, ..)` in a vertical scroll
     *  view, and a column with `.on_drag_along(Vertical, ..)` in a horizontal one. */
    private void axisSetup(Activity activity) {
        dialog = new Dialog(activity);
        DayFixed root = new DayFixed(activity);
        columns = (ScrollView) DayBridge.makeScroll(false);
        DayFixed tall = (DayFixed) DayBridge.contentOf(columns);
        sideways = new Recorder(activity, 0xFF3060FF);
        tall.addView(sideways);
        tall.setChildFrame(sideways, 0, 0, 600, 400);
        tall.setContentSize(600, 4000);
        rows = (android.widget.HorizontalScrollView) DayBridge.makeScroll(true);
        DayFixed wide = (DayFixed) DayBridge.contentOf(rows);
        upright = new Recorder(activity, 0xFF30C050);
        wide.addView(upright);
        wide.setChildFrame(upright, 0, 0, 400, 400);
        wide.setContentSize(4000, 400);
        DayBridge.enableGesture(sideways, 0, true);
        DayBridge.setDragAxis(sideways, 1);
        DayBridge.enableGesture(upright, 0, true);
        DayBridge.setDragAxis(upright, 2);
        root.addView(columns);
        root.addView(rows);
        root.setChildFrame(columns, 0, 0, 600, 600);
        root.setChildFrame(rows, 0, 650, 600, 400);
        dialog.setContentView(root, new ViewGroup.LayoutParams(600, 1050));
        dialog.show();
    }

    /** One finger from the middle of `on`, moved by `dy` pixels in steps, then lifted.
     *  Returns an event of the stream, for {@link DayBridge#gestureHoldsTouch}. */
    private MotionEvent swipe(View on, float dy, boolean lift) {
        return swipe(on, 0, dy, lift);
    }

    /** One finger from the middle of `on`, moved by `(dx, dy)` pixels in steps. */
    private MotionEvent swipe(View on, float dx, float dy, boolean lift) {
        int[] at = new int[2];
        on.getLocationOnScreen(at);
        float x = at[0] + on.getWidth() / 2f, y = at[1] + on.getHeight() / 2f;
        long down = SystemClock.uptimeMillis();
        send(down, MotionEvent.ACTION_DOWN, x, y);
        int steps = 20;
        for (int i = 1; i <= steps; i++) {
            SystemClock.sleep(16);
            send(down, MotionEvent.ACTION_MOVE, x + dx * i / steps, y + dy * i / steps);
        }
        if (lift) send(down, MotionEvent.ACTION_UP, x + dx, y + dy);
        waitForIdleSync();
        return MotionEvent.obtain(down, SystemClock.uptimeMillis(), MotionEvent.ACTION_MOVE, x, y, 0);
    }

    /** Lift the finger of the stream `held` belongs to. */
    private void lift(MotionEvent held) {
        send(held.getDownTime(), MotionEvent.ACTION_UP, 0, 0);
        waitForIdleSync();
    }

    private void axes() throws Exception {
        check(sideways.isAttachedToWindow() && upright.isAttachedToWindow(), "axis fixture laid out");

        // A row that drags sideways in a vertical list: a vertical swipe is the list's.
        MotionEvent held = swipe(sideways, 0, -250, false);
        check(!DayBridge.gestureHoldsTouch(held), "a cross-axis swipe is not held");
        lift(held);
        check(columns.getScrollY() > 0, "a vertical swipe on a sideways row scrolls the list");
        check(sideways.count(MotionEvent.ACTION_CANCEL) == 1, "the list took that stream");
        runOnMainSync(() -> columns.scrollTo(0, 0));
        waitForIdleSync();
        // A horizontal swipe is the row's: held, every move, the list still.
        sideways.actions.clear();
        held = swipe(sideways, 250, -30, false);
        check(DayBridge.gestureHoldsTouch(held), "an along-axis swipe is held");
        lift(held);
        check(!DayBridge.gestureHoldsTouch(held), "and given back at the lift");
        check(sideways.count(MotionEvent.ACTION_CANCEL) == 0, "the sideways drag is never cancelled");
        check(sideways.count(MotionEvent.ACTION_MOVE) == 20, "the sideways drag sees every move");
        check(columns.getScrollY() == 0, "the list did not scroll under the sideways drag");

        // A column that drags vertically in a horizontal list: a horizontal swipe is the list's.
        held = swipe(upright, -250, 0, false);
        check(!DayBridge.gestureHoldsTouch(held), "a cross-axis swipe is not held");
        lift(held);
        check(rows.getScrollX() > 0, "a horizontal swipe on an upright column scrolls the list");
        check(upright.count(MotionEvent.ACTION_CANCEL) == 1, "the horizontal list took that stream");
        runOnMainSync(() -> rows.scrollTo(0, 0));
        waitForIdleSync();
        upright.actions.clear();
        held = swipe(upright, -30, 250, false);
        check(DayBridge.gestureHoldsTouch(held), "an along-axis vertical swipe is held");
        lift(held);
        check(upright.count(MotionEvent.ACTION_CANCEL) == 0, "the upright drag is never cancelled");
        check(upright.count(MotionEvent.ACTION_MOVE) == 20, "the upright drag sees every move");
        check(rows.getScrollX() == 0, "the horizontal list did not scroll under the upright drag");
    }

    private void send(long down, int action, float x, float y) {
        MotionEvent e = MotionEvent.obtain(down, SystemClock.uptimeMillis(), action, x, y, 0);
        e.setSource(android.view.InputDevice.SOURCE_TOUCHSCREEN);
        sendPointerSync(e);
        e.recycle();
    }

    private boolean refreshing() throws ReflectiveOperationException {
        // SwipeRefreshLayout is not on this test's compile classpath.
        return (Boolean) refresh.getClass().getMethod("isRefreshing").invoke(refresh);
    }

    private void exercise() throws Exception {
        check(drag.isAttachedToWindow() && drag.getHeight() == 500, "fixture laid out");

        // Up, on the piece with the drag handler: the scroll view would take this.
        MotionEvent held = swipe(drag, -300, false);
        check(DayBridge.gestureHoldsTouch(held), "the drag holds its stream while the finger is down");
        send(held.getDownTime(), MotionEvent.ACTION_UP, 0, 0);
        waitForIdleSync();
        check(!DayBridge.gestureHoldsTouch(held), "the stream is given back when the finger lifts");
        check(drag.count(MotionEvent.ACTION_CANCEL) == 0, "the drag is never cancelled");
        check(drag.count(MotionEvent.ACTION_MOVE) == 20, "the drag sees every move");
        check(drag.count(MotionEvent.ACTION_UP) == 1, "the drag sees the lift");
        check(scroll.getScrollY() == 0, "the page did not scroll under the drag");

        // Down, at the top of the page: the pull-to-refresh would take this.
        drag.actions.clear();
        swipe(drag, 900, true);
        check(drag.count(MotionEvent.ACTION_CANCEL) == 0, "a downward drag is not cancelled");
        check(drag.count(MotionEvent.ACTION_MOVE) == 20, "a downward drag sees every move");
        check(!refreshing(), "a drag does not pull to refresh");
        check(scroll.getScrollY() == 0, "the page did not scroll under the downward drag");

        // The same two swipes on a piece with only a tap handler belong to the containers.
        swipe(tap, -300, true);
        SystemClock.sleep(400);
        final int[] y = new int[1];
        runOnMainSync(() -> { y[0] = scroll.getScrollY(); scroll.scrollTo(0, 0); });
        check(y[0] > 0, "a swipe on a tap-only piece scrolls the page");
        check(tap.count(MotionEvent.ACTION_CANCEL) == 1, "the scroll view took that stream");
        waitForIdleSync();
        swipe(tap, 900, true);
        SystemClock.sleep(400);
        check(refreshing(), "a pull on a tap-only piece still refreshes");
        check(!DayBridge.gestureHoldsTouch(held), "no stream is held afterwards");
    }
}
