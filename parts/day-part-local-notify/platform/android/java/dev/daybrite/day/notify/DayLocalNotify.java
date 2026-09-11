// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-part-local-notify's OWN Android backend — a headless capability shim (no UI). Bundled with
// the crate and folded into the app's Gradle build via [package.metadata.day.android], with ZERO
// edits to day-android. The Android twin of parts/day-part-local-notify/src/apple.rs.
//
// NO GOOGLE DEPENDENCY. This is the platform NotificationManager and AlarmManager only — no Play
// services, no Firebase — so it runs unchanged on AOSP, GrapheneOS, or a Kindle. That is a design
// requirement of docs/notify.md, not an accident.
//
// WHY ALARMMANAGER FOR SCHEDULING. Android has no notification scheduler: unlike Apple's
// UNTimeIntervalNotificationTrigger, nothing in the OS will hold a notification for you. A delayed
// notification is therefore an alarm that wakes DayNotifyAlarmReceiver, which rebuilds and posts it
// from data persisted here — in a fresh process with no Day tree alive, which is why the payload is
// snapshotted at schedule time rather than read from app state at fire time.
//
// PERMISSIONS. This shim never requests one. POST_NOTIFICATIONS (API 33+) is asked for through
// day-part-permissions; a missing grant surfaces as ERR_DENIED rather than a silent no-op.
package dev.daybrite.day.notify;

import android.app.AlarmManager;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.net.Uri;
import android.os.Build;

import dev.daybrite.day.bridge.DayBridge;

public final class DayLocalNotify {
    private DayLocalNotify() {}

    /** Error codes shared with src/android.rs: 0 ok, 1 denied, 3 failed. */
    public static final int OK = 0;
    public static final int ERR_DENIED = 1;
    public static final int ERR_FAILED = 3;

    /** Where scheduled payloads live so DayNotifyBootReceiver can re-arm them after a reboot. */
    static final String PREFS = "day_local_notify";
    /** Intent extras, shared with the receivers. */
    static final String EX_ID = "day.notify.id";
    static final String EX_CHANNEL = "day.notify.channel";
    static final String EX_TITLE = "day.notify.title";
    static final String EX_BODY = "day.notify.body";
    static final String EX_ROUTE = "day.notify.route";
    static final String EX_ICON = "day.notify.icon";
    static final String EX_BADGE = "day.notify.badge";
    static final String EX_AT = "day.notify.at";

    private static NotificationManager manager() {
        Context ctx = DayBridge.ctx;
        return ctx == null ? null : (NotificationManager) ctx.getSystemService(Context.NOTIFICATION_SERVICE);
    }

    public static boolean isAvailable() {
        return manager() != null;
    }

    /** Whether the user has notifications switched on for this app (API 24+). */
    public static boolean areEnabled() {
        NotificationManager nm = manager();
        return nm != null && nm.areNotificationsEnabled();
    }

    /**
     * Create (or leave alone) a channel. Importance is IMMUTABLE after the first registration —
     * Android hands the setting to the user at that point — so re-registering with a different
     * level deliberately does nothing rather than appearing to work.
     */
    public static void createChannel(String id, String name, int importance, boolean sound) {
        NotificationManager nm = manager();
        if (nm == null || Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return;
        NotificationChannel ch = new NotificationChannel(id, name, importance);
        if (!sound) ch.setSound(null, null);
        nm.createNotificationChannel(ch);
    }

    /** Post immediately. Returns one of the OK/ERR_* codes above. */
    public static int notifyNow(int id, String channelId, String title, String body,
                                String route, String icon, int badge) {
        Context ctx = DayBridge.ctx;
        NotificationManager nm = manager();
        if (ctx == null || nm == null) return ERR_FAILED;
        if (!nm.areNotificationsEnabled()) return ERR_DENIED;
        try {
            nm.notify(id, build(ctx, channelId, title, body, route, icon, badge));
            return OK;
        } catch (SecurityException e) {
            return ERR_DENIED;
        } catch (Throwable t) {
            return ERR_FAILED;
        }
    }

    /** Build the Notification a post or an alarm fire shows. Shared with DayNotifyAlarmReceiver. */
    static Notification build(Context ctx, String channelId, String title, String body,
                              String route, String icon, int badge) {
        Notification.Builder b = Build.VERSION.SDK_INT >= Build.VERSION_CODES.O
                ? new Notification.Builder(ctx, channelId)
                : new Notification.Builder(ctx);
        b.setContentTitle(title).setAutoCancel(true);
        if (body != null && !body.isEmpty()) b.setContentText(body);
        b.setSmallIcon(smallIcon(ctx, icon));
        if (badge > 0) b.setNumber(badge);
        PendingIntent tap = tapIntent(ctx, route);
        if (tap != null) b.setContentIntent(tap);
        return b.build();
    }

    /**
     * The small icon must be a MONOCHROME silhouette — a full-color drawable renders as a white
     * square. The crate ships ic_day_notify as the default; an app overrides it by name. Resolved
     * with getIdentifier because a piece cannot know the app's R class (docs/extending.md).
     */
    private static int smallIcon(Context ctx, String name) {
        if (name != null && !name.isEmpty()) {
            int id = ctx.getResources().getIdentifier(name, "drawable", ctx.getPackageName());
            if (id != 0) return id;
        }
        int fallback = ctx.getResources().getIdentifier(
                "ic_day_notify", "drawable", ctx.getPackageName());
        return fallback != 0 ? fallback : android.R.drawable.ic_dialog_info;
    }

    /**
     * Tapping opens the app at {@code route}. The intent carries the route as its DATA URI rather
     * than an extra, because that is the rail day-android already reads on both paths: a cold start
     * reads getIntent().getData() into DAY_DEEPLINK, and a warm tap arrives at onNewIntent, which
     * turns the same URI into a deep-link event. No day-android change is needed.
     */
    private static PendingIntent tapIntent(Context ctx, String route) {
        Intent open = ctx.getPackageManager().getLaunchIntentForPackage(ctx.getPackageName());
        if (open == null) return null;
        if (route != null && !route.isEmpty()) {
            open.setData(Uri.parse("dayroute://" + route));
        }
        open.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP);
        int flags = PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE;
        // The route is part of the identity: two notifications opening different routes must not
        // collapse onto one PendingIntent (which FLAG_UPDATE_CURRENT would otherwise do).
        return PendingIntent.getActivity(ctx, route == null ? 0 : route.hashCode(), open, flags);
    }

    /**
     * Schedule for {@code atMillis}. Persists the payload first, so a reboot (which clears every
     * alarm) can re-arm it, then sets the alarm. {@code alarmClock} = the notification rides an
     * Urgent channel: use the alarm-clock slot (status-bar icon, Doze-exempt, what the OS reserves
     * for "the user is expecting to wake up to this").
     */
    public static int schedule(int id, long atMillis, String channelId, String title, String body,
                               String route, String icon, int badge, boolean alarmClock) {
        Context ctx = DayBridge.ctx;
        if (ctx == null) return ERR_FAILED;
        AlarmManager am = (AlarmManager) ctx.getSystemService(Context.ALARM_SERVICE);
        if (am == null) return ERR_FAILED;
        try {
            persist(ctx, id, atMillis, channelId, title, body, route, icon, badge, alarmClock);
            PendingIntent pi = alarmIntent(ctx, id, atMillis, channelId, title, body, route, icon, badge);
            setAlarm(am, ctx, atMillis, route, alarmClock, pi);
            return OK;
        } catch (Throwable t) {
            return ERR_FAILED;
        }
    }

    /**
     * Arm {@code pi} at {@code atMillis} as exactly as this device allows. Shared with
     * DayNotifyBootReceiver so a re-armed alarm keeps the exactness it was scheduled with.
     *
     * Exact alarms are increasingly restricted: SCHEDULE_EXACT_ALARM is auto-granted but revocable
     * on 12–13 and withheld by default on 14+; a clock app gets an install-time grant by declaring
     * USE_EXACT_ALARM itself (docs/notify.md). All three exact paths — including setAlarmClock —
     * need the grant, so a missing one falls back to an inexact alarm rather than dropping it; the
     * caller is told which it got by the canScheduleExact capability flag.
     */
    static void setAlarm(AlarmManager am, Context ctx, long atMillis, String route,
                         boolean alarmClock, PendingIntent pi) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && !am.canScheduleExactAlarms()) {
            am.set(AlarmManager.RTC_WAKEUP, atMillis, pi);
        } else if (alarmClock) {
            am.setAlarmClock(new AlarmManager.AlarmClockInfo(atMillis, tapIntent(ctx, route)), pi);
        } else {
            am.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, atMillis, pi);
        }
    }

    static PendingIntent alarmIntent(Context ctx, int id, long atMillis, String channelId,
                                     String title, String body, String route, String icon, int badge) {
        Intent i = new Intent(ctx, DayNotifyAlarmReceiver.class);
        i.putExtra(EX_ID, id).putExtra(EX_CHANNEL, channelId).putExtra(EX_TITLE, title)
         .putExtra(EX_BODY, body).putExtra(EX_ROUTE, route).putExtra(EX_ICON, icon)
         .putExtra(EX_BADGE, badge).putExtra(EX_AT, atMillis);
        return PendingIntent.getBroadcast(ctx, id, i,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
    }

    /** One record per scheduled id, tab-separated. A tab cannot appear in the fields we store. */
    private static void persist(Context ctx, int id, long atMillis, String channelId, String title,
                                String body, String route, String icon, int badge, boolean alarmClock) {
        SharedPreferences p = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        String rec = atMillis + "\t" + s(channelId) + "\t" + s(title) + "\t" + s(body) + "\t"
                + s(route) + "\t" + s(icon) + "\t" + badge + "\t" + (alarmClock ? "1" : "0");
        p.edit().putString(String.valueOf(id), rec).apply();
    }

    static void forget(Context ctx, int id) {
        ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
           .edit().remove(String.valueOf(id)).apply();
    }

    private static String s(String v) {
        return v == null ? "" : v.replace('\t', ' ');
    }

    public static void cancel(int id) {
        Context ctx = DayBridge.ctx;
        NotificationManager nm = manager();
        if (nm != null) nm.cancel(id);
        if (ctx == null) return;
        AlarmManager am = (AlarmManager) ctx.getSystemService(Context.ALARM_SERVICE);
        if (am != null) {
            am.cancel(alarmIntent(ctx, id, 0L, "", "", "", "", "", 0));
        }
        forget(ctx, id);
    }

    public static void cancelAll() {
        Context ctx = DayBridge.ctx;
        NotificationManager nm = manager();
        if (nm != null) nm.cancelAll();
        if (ctx == null) return;
        SharedPreferences p = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        AlarmManager am = (AlarmManager) ctx.getSystemService(Context.ALARM_SERVICE);
        if (am != null) {
            for (String key : p.getAll().keySet()) {
                try {
                    am.cancel(alarmIntent(ctx, Integer.parseInt(key), 0L, "", "", "", "", "", 0));
                } catch (NumberFormatException ignored) {
                    // A malformed key cannot name an alarm; clearing the store below drops it.
                }
            }
        }
        p.edit().clear().apply();
    }

    /** Whether an exact alarm would actually be exact — the honest input to the capability flag. */
    public static boolean canScheduleExact() {
        Context ctx = DayBridge.ctx;
        if (ctx == null) return false;
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return true;
        AlarmManager am = (AlarmManager) ctx.getSystemService(Context.ALARM_SERVICE);
        return am != null && am.canScheduleExactAlarms();
    }
}
