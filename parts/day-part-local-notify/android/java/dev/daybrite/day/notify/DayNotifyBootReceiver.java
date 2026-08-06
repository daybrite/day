// Re-arms scheduled notifications after a reboot.
//
// WHY THIS EXISTS. A restart clears every AlarmManager alarm, so without this a notification
// scheduled for tomorrow morning silently never fires if the phone is rebooted tonight — the
// failure this crate persists its payloads to avoid. Declared through the crate's
// android/components.xml, and the RECEIVE_BOOT_COMPLETED permission it needs is contributed by the
// crate's [package.metadata.day.android].permissions (it is structural: no prompt, no reason).
package dev.daybrite.day.notify;

import android.app.AlarmManager;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.os.Build;

import java.util.Map;

public final class DayNotifyBootReceiver extends BroadcastReceiver {
    @Override public void onReceive(Context ctx, Intent intent) {
        if (ctx == null || intent == null) return;
        String action = intent.getAction();
        if (action == null) return;
        // MY_PACKAGE_REPLACED matters as much as BOOT_COMPLETED: an app update also drops alarms.
        if (!Intent.ACTION_BOOT_COMPLETED.equals(action)
                && !Intent.ACTION_MY_PACKAGE_REPLACED.equals(action)
                && !"android.intent.action.QUICKBOOT_POWERON".equals(action)) {
            return;
        }
        SharedPreferences p = ctx.getSharedPreferences(DayLocalNotify.PREFS, Context.MODE_PRIVATE);
        AlarmManager am = (AlarmManager) ctx.getSystemService(Context.ALARM_SERVICE);
        if (am == null) return;
        long now = System.currentTimeMillis();
        for (Map.Entry<String, ?> e : p.getAll().entrySet()) {
            int id;
            try {
                id = Integer.parseInt(e.getKey());
            } catch (NumberFormatException bad) {
                continue;
            }
            Object v = e.getValue();
            if (!(v instanceof String)) continue;
            // atMillis \t channel \t title \t body \t route \t icon \t badge
            String[] f = ((String) v).split("\t", -1);
            if (f.length < 7) continue;
            long at;
            int badge;
            try {
                at = Long.parseLong(f[0]);
                badge = Integer.parseInt(f[6]);
            } catch (NumberFormatException bad) {
                continue;
            }
            if (at <= now) {
                // Its moment passed while the device was off. Firing it now would be a surprise
                // hours late, so drop it rather than deliver something stale.
                p.edit().remove(e.getKey()).apply();
                continue;
            }
            try {
                android.app.PendingIntent pi = DayLocalNotify.alarmIntent(
                        ctx, id, at, f[1], f[2], f[3], f[4], f[5], badge);
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && !am.canScheduleExactAlarms()) {
                    am.set(AlarmManager.RTC_WAKEUP, at, pi);
                } else {
                    am.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi);
                }
            } catch (Throwable t) {
                // One bad record must not stop the rest from being re-armed.
            }
        }
    }
}
