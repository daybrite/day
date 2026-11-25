// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
package dev.daybrite.day.bridge;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Intent;
import android.graphics.Rect;
import android.os.Bundle;
import android.os.SystemClock;
import android.view.View;
import android.widget.TextView;
import java.util.ArrayList;
import java.util.concurrent.Callable;
import java.util.concurrent.FutureTask;

/** Native view assertions. Message and hint strings are synthetic fixture data. */
public final class FeedbackInstrumentation extends Instrumentation {
    private <T> T onUi(Callable<T> work) throws Exception {
        FutureTask<T> task = new FutureTask<>(work);
        runOnMainSync(task);
        return task.get();
    }

    private void awaitMessage(View root, String message, boolean visible) throws Exception {
        long deadline = SystemClock.uptimeMillis() + 10000;
        do {
            boolean found = onUi(() -> {
                ArrayList<View> matches = new ArrayList<>();
                root.findViewsWithText(matches, message, View.FIND_VIEWS_WITH_TEXT);
                for (View view : matches) {
                    if (view instanceof TextView && view.isShown() && view.getGlobalVisibleRect(new Rect()))
                        return true;
                }
                return false;
            });
            if (found == visible) return;
            SystemClock.sleep(30);
        } while (SystemClock.uptimeMillis() < deadline);
        throw new AssertionError("Expected native Snackbar visibility " + visible);
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
            String message = "Native snackbar fixture";
            View root = onUi(() -> {
                View target = new View(activity);
                int[] calls = {0};
                target.setOnLongClickListener(v -> { calls[0]++; return true; });
                DayBridge.setTooltip(target, "Fixture hint");
                target.performLongClick();
                DayBridge.setTooltip(target, "");
                target.performLongClick();
                if (calls[0] != 2) throw new AssertionError("Tooltip displaced the existing long-click listener");
                View content = activity.findViewById(android.R.id.content);
                DayBridge.showToast(content, message);
                return content;
            });
            awaitMessage(root, message, true);
            awaitMessage(root, message, false);
            result.putString("stream", "PASS Android native tooltip listener and Snackbar visibility\n");
            finish(Activity.RESULT_OK, result);
        } catch (Throwable error) {
            result.putString("stream", android.util.Log.getStackTraceString(error));
            finish(Activity.RESULT_CANCELED, result);
        }
    }
}
