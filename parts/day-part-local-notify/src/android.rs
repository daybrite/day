// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Android: `NotificationManager` for display and `AlarmManager` for scheduling, through this
//! crate's Java shim (`src/DayLocalNotify.java`, with its two receivers beside it), staged into the app's Gradle build
//! by `day build`.
//!
//! No Google dependency (neither Play services nor Firebase), so an app linking this part runs
//! unchanged on AOSP, GrapheneOS, or a Kindle (docs/notify.md).
//!
//! Scheduling is the half that differs most from Apple. Android has no notification scheduler, so a
//! delayed notification is an `AlarmManager` alarm that wakes a `<receiver>` (declared through the
//! `manifest-components` key), which rebuilds the notification from data persisted at schedule
//! time. A reboot clears alarms, so a boot receiver re-arms them.

use day_android::java;
use day_android::jni::objects::JString;

use crate::{Capabilities, Channel, Importance, NotifId, Notification, NotifyError};

// The shim (DayLocalNotify.java, staged into the app's Gradle build by `day build`).
day_android::java_class! {
    DayLocalNotify => dev.daybrite.day.notify.DayLocalNotify,
    methods {
        static fn isAvailable() -> jboolean,
        static fn canScheduleExact() -> jboolean,
        static fn createChannel(id: JString, name: JString, importance: jint, sound: jboolean),
        static fn schedule(id: jint, at_ms: jlong, channel: JString, title: JString,
            body: JString, route: JString, icon: JString, badge: jint, alarm_clock: jboolean)
            -> jint,
        static fn notifyNow(id: jint, channel: JString, title: JString, body: JString,
            route: JString, icon: JString, badge: jint) -> jint,
        static fn cancel(id: jint),
        static fn cancelAll(),
    },
}

/// Mirrors the shim's OK / ERR_DENIED / ERR_FAILED.
fn to_result(code: i32) -> Result<(), NotifyError> {
    match code {
        0 => Ok(()),
        1 => Err(NotifyError::PermissionDenied),
        _ => Err(NotifyError::Failed("the platform refused the post".into())),
    }
}

pub(crate) fn capabilities() -> Capabilities {
    let available = java(|env| DayLocalNotify::isAvailable(env)).unwrap_or(false);
    Capabilities {
        post: available,
        // An alarm survives the app exiting, which is why scheduling goes through
        // AlarmManager rather than an in-process timer. Exactness is a separate question
        // (`canScheduleExact` below); the notification still arrives either way.
        schedule_while_dead: available,
        // The one platform with a user-facing channel model.
        channels: available,
        // `setNumber` is honored only by launchers that draw badges, so this is the platform
        // saying "supported", not a promise that every launcher shows it.
        badge: available,
        icon: available,
        tap_route: available,
        // Android 12+ may withhold the exact-alarm grant, in which case `schedule` downgrades to
        // an inexact alarm rather than dropping it.
        schedule_exact: available
            && java(|env| DayLocalNotify::canScheduleExact(env)).unwrap_or(false),
    }
}

/// The importance ints are Android's own `NotificationManager.IMPORTANCE_*` values, passed straight
/// through so the shim needs no translation table.
fn importance_value(i: Importance) -> i32 {
    match i {
        Importance::Min => 1,     // IMPORTANCE_MIN
        Importance::Low => 2,     // IMPORTANCE_LOW
        Importance::Default => 3, // IMPORTANCE_DEFAULT
        // There is no level above `IMPORTANCE_HIGH`; Urgent differs from High only in interruption
        // behavior, which on Android is a full-screen intent this phase does not use.
        Importance::High | Importance::Urgent => 4, // IMPORTANCE_HIGH
    }
}

pub(crate) fn register_channel(channel: &Channel) {
    // Recorded locally too, so `post` can read the channel's sound/importance without a JNI hop.
    super::channels::remember(channel);
    java(|env| {
        let id = JString::from_str(env, channel.id())?;
        let name = JString::from_str(env, channel.display_name())?;
        DayLocalNotify::createChannel(
            env,
            &id,
            &name,
            importance_value(channel.importance()),
            channel.plays_sound(),
        )
    });
}

pub(crate) fn post(n: &Notification) -> Result<(), NotifyError> {
    let fire_at = n.fire_at_ms(now_ms());
    // An Urgent channel schedules through `setAlarmClock`, the user-visible alarm-clock slot
    // (status-bar icon, Doze-exempt). The channel's importance lives on the Rust side; Android's
    // channel can't answer this because Urgent and High both map to IMPORTANCE_HIGH there.
    let alarm_clock = super::channels::importance(n.channel_str()) == Importance::Urgent;
    let code = java(|env| {
        let channel = JString::from_str(env, n.channel_str())?;
        let title = JString::from_str(env, n.title_str())?;
        let body = JString::from_str(env, n.body_str())?;
        let route = JString::from_str(env, n.route_str())?;
        let icon = JString::from_str(env, n.icon_str())?;
        let id = n.resolved_id().0 as i32;
        let badge = n.badge_count().unwrap_or(0) as i32;
        match fire_at {
            // Absolute wall-clock time, because AlarmManager.RTC_WAKEUP takes one, and because
            // the boot receiver has to know when, not "how long from some forgotten start".
            Some(at_ms) => DayLocalNotify::schedule(
                env,
                id,
                at_ms,
                &channel,
                &title,
                &body,
                &route,
                &icon,
                badge,
                alarm_clock,
            ),
            None => {
                DayLocalNotify::notifyNow(env, id, &channel, &title, &body, &route, &icon, badge)
            }
        }
    });
    match code {
        Some(c) => to_result(c),
        None => Err(NotifyError::Failed("the notify shim did not answer".into())),
    }
}

/// Wall-clock milliseconds. `SystemTime` can be before the epoch only on a badly-set clock, where
/// scheduling is meaningless anyway; treat it as "now".
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub(crate) fn cancel(id: NotifId) {
    java(|env| DayLocalNotify::cancel(env, id.0 as i32));
}

pub(crate) fn cancel_all() {
    java(|env| DayLocalNotify::cancelAll(env));
}
