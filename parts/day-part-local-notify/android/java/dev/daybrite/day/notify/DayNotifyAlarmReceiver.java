// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Fires a scheduled notification. Declared in the app manifest through the crate's
// android/components.xml ([package.metadata.day.android].manifest-components) — without that
// declaration Android never instantiates this class, and the APK installs, runs, and silently
// never delivers.
//
// THIS RUNS WITH NO DAY TREE ALIVE. The alarm can wake a fresh process long after the app exited,
// so everything shown is read from the intent extras that were snapshotted at schedule time. There
// is no app state to consult here, which is exactly why a scheduled notification's content cannot
// be reactive (docs/notify.md).
package dev.daybrite.day.notify;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;

public final class DayNotifyAlarmReceiver extends BroadcastReceiver {
    @Override public void onReceive(Context ctx, Intent intent) {
        if (ctx == null || intent == null) return;
        int id = intent.getIntExtra(DayLocalNotify.EX_ID, 0);
        android.app.NotificationManager nm = (android.app.NotificationManager)
                ctx.getSystemService(Context.NOTIFICATION_SERVICE);
        if (nm == null) return;
        try {
            nm.notify(id, DayLocalNotify.build(
                    ctx,
                    intent.getStringExtra(DayLocalNotify.EX_CHANNEL),
                    intent.getStringExtra(DayLocalNotify.EX_TITLE),
                    intent.getStringExtra(DayLocalNotify.EX_BODY),
                    intent.getStringExtra(DayLocalNotify.EX_ROUTE),
                    intent.getStringExtra(DayLocalNotify.EX_ICON),
                    intent.getIntExtra(DayLocalNotify.EX_BADGE, 0)));
        } catch (Throwable t) {
            // A receiver that throws takes the process with it, and there is no UI to report to.
        }
        // Delivered once: drop the record so a later reboot does not re-arm a past alarm.
        DayLocalNotify.forget(ctx, id);
    }
}
