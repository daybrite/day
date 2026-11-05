// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
package dev.daybrite.day.bridge;

import android.app.Activity;
import android.app.Dialog;
import android.app.Instrumentation;
import android.content.Intent;
import android.os.Bundle;
import android.view.MotionEvent;
import android.view.View;
import android.widget.FrameLayout;
import android.widget.TextView;
import java.lang.reflect.Field;
import java.util.Map;

/** Native Android regression fixtures; labels below are synthetic test data. */
public final class ContextMenuInstrumentation extends Instrumentation {
    private int checks;
    private Dialog dialog;
    private FrameLayout row;
    private TextView target;
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
            Throwable[] failure = {null};
            runOnMainSync(() -> {
                try { exercise(); } catch (Throwable error) { failure[0] = error; }
            });
            if (failure[0] != null) throw failure[0];
            result.putString("stream", "PASS Android native context menus: " + checks + " assertions\n");
            finish(Activity.RESULT_OK, result);
        } catch (Throwable error) {
            result.putString("stream", android.util.Log.getStackTraceString(error));
            finish(Activity.RESULT_CANCELED, result);
        }
    }
    private android.widget.PopupMenu popup(View view) {
        try {
            Field field = DayBridge.class.getDeclaredField("contextMenus");
            field.setAccessible(true);
            Object binding = ((Map<?,?>)field.get(null)).get(view);
            if (binding == null) return null;
            Field popup = binding.getClass().getDeclaredField("popup");
            popup.setAccessible(true);
            return (android.widget.PopupMenu)popup.get(binding);
        } catch (ReflectiveOperationException error) { throw new AssertionError(error); }
    }
    private void setup(Activity activity) {
        dialog = new Dialog(activity);
        row = new FrameLayout(activity);
        target = new TextView(activity);
        target.setText("Synthetic menu fixture");
        row.addView(target, new FrameLayout.LayoutParams(400, 200));
        dialog.setContentView(row);
        dialog.show();
    }
    private void exercise() {
        try {
            check(target.isAttachedToWindow(), "fixture attached to native window");
            int[] calls = {0}, taps = {0};
            float[] at = {-1, -1};
            row.setOnClickListener(v -> taps[0]++);
            DayBridge.installContextMenu(target, (x,y) -> {
                at[0]=x; at[1]=y;
                return "A\t0\t1\t-1\tFixture " + (++calls[0]);
            });
            check(calls[0] == 0, "provider must be lazy");
            target.performClick();
            check(taps[0] == 1, "normal tap forwards to row");
            check(target.performLongClick(), "native long click consumed");
            check(calls[0] == 1, "one callback per summon");
            check(popup(target).getMenu().getItem(0).getTitle().equals("Fixture 1"), "first menu");
            check(at[0] == 0 && at[1] == 0, "accessibility summon uses origin");
            target.performLongClick();
            check(calls[0] == 2, "provider refreshes on repeated summon");
            check(popup(target).getMenu().getItem(0).getTitle().equals("Fixture 2"), "fresh menu state");
            check(taps[0] == 1, "long press does not click row");
            popup(target).dismiss();
            check(popup(target) == null, "dismiss releases popup");
            MotionEvent down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, 31, 47, 0);
            target.dispatchTouchEvent(down); down.recycle();
            target.performLongClick();
            check(at[0] == 31 && at[1] == 47, "touch coordinates reach provider");
            MotionEvent cancel = MotionEvent.obtain(0, 0, MotionEvent.ACTION_CANCEL, 31, 47, 0);
            target.dispatchTouchEvent(cancel); cancel.recycle();
            target.performLongClick();
            check(at[0] == 0 && at[1] == 0, "cancel clears stale touch coordinates");
            android.widget.PopupMenu previous = popup(target);
            boolean[] dismissed = {false};
            previous.setOnDismissListener(p -> dismissed[0]=true);
            DayBridge.setContextMenu(target, "A\t0\t0\t-1\tDisabled fixture\nA\t0\t1\t1\tChecked fixture");
            check(dismissed[0], "replacement dismisses open popup");
            target.performLongClick();
            check(calls[0] == 4, "static replacement stops dynamic provider");
            check(!popup(target).getMenu().getItem(0).isEnabled(), "disabled item preserved");
            check(popup(target).getMenu().getItem(1).isChecked(), "checked item preserved");
            DayBridge.installContextMenu(target, (x,y) -> "");
            target.performLongClick();
            check(popup(target) == null, "empty result suppresses popup");
            DayBridge.installContextMenu(target, (x,y) -> null);
            target.performLongClick();
            check(popup(target) == null, "failed callback suppresses popup");
            DayBridge.installContextMenu(target, (x,y) -> {
                DayBridge.setContextMenu(target, "");
                return "A\t0\t1\t-1\tStale fixture";
            });
            target.performLongClick();
            check(popup(target) == null, "reentrant removal cannot show stale popup");
            check(!target.isLongClickable(), "empty static menu removes long-click listener");
            check(!target.hasOnClickListeners(), "removal clears menu-owned forwarding click");
            target.setOnClickListener(v -> taps[0]+=10);
            DayBridge.installContextMenu(target, (x,y) -> "A\t0\t1\t-1\tFixture");
            DayBridge.setContextMenu(target, "");
            target.performClick();
            check(taps[0] == 11, "removal preserves independently owned click");
            for (int i=0;i<30;i++) {
                final int position=i;
                DayBridge.installContextMenu(target, (x,y) -> "A\t0\t1\t-1\tRow " + position);
                target.performLongClick();
                check(popup(target).getMenu().getItem(0).getTitle().equals("Row " + i), "rebound row uses current provider");
                DayBridge.setContextMenu(target, "");
            }
            DayBridge.enableGesture(target, 0, false);
            DayBridge.installContextMenu(target, (x,y) -> "A\t0\t1\t-1\tTap fixture");
            target.performLongClick();
            check(popup(target) != null, "menu coexists with an existing tap gesture");
            DayBridge.releaseContextMenu(target);
            check(popup(target) == null && !target.isLongClickable(), "release detaches popup and listener");
            check(target.hasOnClickListeners(), "release preserves declared tap gesture");
            DayBridge.installContextMenu(target, (x,y) -> "A\t0\t1\t-1\tTap fixture");
            DayBridge.enableGesture(target, 0, false);
            target.performLongClick();
            check(popup(target) != null, "later tap gesture keeps menu callback");
            DayBridge.setContextMenuFn(target, Long.MAX_VALUE);
            target.performLongClick();
            check(popup(target) == null, "released or unknown Rust provider fails closed");
        } finally {
            DayBridge.setContextMenu(target, "");
            dialog.dismiss();
        }
    }
}
