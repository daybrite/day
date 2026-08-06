//! Android: `NotificationManager` for display and `AlarmManager` for scheduling, through this
//! crate's OWN Java shim (`android/java/…/DayLocalNotify.java`), staged into the app's Gradle build
//! by `day build`.
//!
//! Deliberately NO Google dependency: no Play services, no Firebase, so an app linking this part
//! runs unchanged on AOSP, GrapheneOS, or a Kindle (docs/notify.md).
//!
//! Scheduling is the half that differs most from Apple. Android has no notification scheduler, so a
//! delayed notification is an `AlarmManager` alarm that wakes a `<receiver>` — declared through the
//! `manifest-components` key — which rebuilds the notification from data persisted at schedule
//! time. A reboot clears alarms, so a boot receiver re-arms them.

use day_android::DayEnv;
use day_android::jni::objects::JValue;
use day_android::with_env;

use crate::{Capabilities, Channel, Importance, NotifId, Notification, NotifyError};

const CLASS: &str = "dev/daybrite/day/notify/DayLocalNotify";

/// Mirrors the shim's OK / ERR_DENIED / ERR_FAILED.
fn to_result(code: i32) -> Result<(), NotifyError> {
    match code {
        0 => Ok(()),
        1 => Err(NotifyError::PermissionDenied),
        _ => Err(NotifyError::Failed("the platform refused the post".into())),
    }
}

fn call_bool(method: &str) -> bool {
    with_env(|env| {
        env.dcall_static(CLASS, method, "()Z", &[])
            .ok()
            .and_then(|v| v.z().ok())
            .unwrap_or(false)
    })
}

pub(crate) fn capabilities() -> Capabilities {
    let available = call_bool("isAvailable");
    Capabilities {
        post: available,
        // An alarm survives the app exiting — that is the whole reason scheduling goes through
        // AlarmManager rather than an in-process timer. Exactness is a separate question
        // (`canScheduleExact` below); the notification still arrives either way.
        schedule_while_dead: available,
        // The one platform with a real user-facing channel model.
        channels: available,
        // `setNumber` is honoured only by launchers that draw badges, so this is the platform
        // saying "supported", not a promise that every launcher shows it.
        badge: available,
        icon: available,
        tap_route: available,
        // Android 12+ may withhold the exact-alarm grant, in which case `schedule` downgrades to
        // an inexact alarm rather than dropping it.
        schedule_exact: available && call_bool("canScheduleExact"),
    }
}

/// The importance ints are Android's own `NotificationManager.IMPORTANCE_*` values, passed straight
/// through so the shim needs no translation table.
fn importance_value(i: Importance) -> i32 {
    match i {
        Importance::Min => 1,     // IMPORTANCE_MIN
        Importance::Low => 2,     // IMPORTANCE_LOW
        Importance::Default => 3, // IMPORTANCE_DEFAULT
        // There is no level above HIGH; Urgent differs from High only in interruption behaviour,
        // which on Android is a full-screen intent this phase does not use.
        Importance::High | Importance::Urgent => 4, // IMPORTANCE_HIGH
    }
}

pub(crate) fn register_channel(channel: &Channel) {
    // Recorded locally too, so `post` can read the channel's sound/importance without a JNI hop.
    super::channels::remember(channel);
    with_env(|env| {
        let (Ok(id), Ok(name)) = (
            env.new_string(channel.id()),
            env.new_string(channel.display_name()),
        ) else {
            return;
        };
        let _ = env.dcall_static(
            CLASS,
            "createChannel",
            "(Ljava/lang/String;Ljava/lang/String;IZ)V",
            &[
                JValue::Object(&id),
                JValue::Object(&name),
                JValue::Int(importance_value(channel.importance())),
                JValue::Bool(channel.plays_sound()),
            ],
        );
    });
}

pub(crate) fn post(n: &Notification) -> Result<(), NotifyError> {
    let delay = n.delay_secs();
    with_env(|env| {
        let (Ok(channel), Ok(title), Ok(body), Ok(route), Ok(icon)) = (
            env.new_string(n.channel_str()),
            env.new_string(n.title_str()),
            env.new_string(n.body_str()),
            env.new_string(n.route_str()),
            env.new_string(n.icon_str()),
        ) else {
            return Err(NotifyError::Failed("could not marshal strings".into()));
        };
        let id = JValue::Int(n.resolved_id().0 as i32);
        let badge = JValue::Int(n.badge_count().unwrap_or(0) as i32);

        let code = if delay > 0.0 {
            // Absolute wall-clock time, because AlarmManager.RTC_WAKEUP takes one — and because
            // the boot receiver has to know WHEN, not "how long from some forgotten start".
            let at_ms = now_ms() + (delay * 1000.0) as i64;
            env.dcall_static(
                CLASS,
                "schedule",
                "(IJLjava/lang/String;Ljava/lang/String;Ljava/lang/String;\
                 Ljava/lang/String;Ljava/lang/String;I)I",
                &[
                    id,
                    JValue::Long(at_ms),
                    JValue::Object(&channel),
                    JValue::Object(&title),
                    JValue::Object(&body),
                    JValue::Object(&route),
                    JValue::Object(&icon),
                    badge,
                ],
            )
        } else {
            env.dcall_static(
                CLASS,
                "notifyNow",
                "(ILjava/lang/String;Ljava/lang/String;Ljava/lang/String;\
                 Ljava/lang/String;Ljava/lang/String;I)I",
                &[
                    id,
                    JValue::Object(&channel),
                    JValue::Object(&title),
                    JValue::Object(&body),
                    JValue::Object(&route),
                    JValue::Object(&icon),
                    badge,
                ],
            )
        };
        match code.ok().and_then(|v| v.i().ok()) {
            Some(c) => to_result(c),
            None => Err(NotifyError::Failed("the notify shim did not answer".into())),
        }
    })
}

/// Wall-clock milliseconds. `SystemTime` can be before the epoch only on a badly-set clock, where
/// scheduling is meaningless anyway — treat it as "now".
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub(crate) fn cancel(id: NotifId) {
    with_env(|env| {
        let _ = env.dcall_static(CLASS, "cancel", "(I)V", &[JValue::Int(id.0 as i32)]);
    });
}

pub(crate) fn cancel_all() {
    with_env(|env| {
        let _ = env.dcall_static(CLASS, "cancelAll", "()V", &[]);
    });
}
