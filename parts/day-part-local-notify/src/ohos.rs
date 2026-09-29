// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! HarmonyOS: Notification Kit (`notificationManager`), which exists only in ArkTS, through this
//! crate's daybridge arm (docs/bridge.md).
//!
//! An importance picks the notification's slot, HarmonyOS's fixed set of channels: the user
//! configures each slot, not each app channel, so `channels` stays false. A tap starts the app's
//! entry ability with the route as the `day.uri` want parameter, the same intake a deep link
//! uses (docs/deep-links.md), cold or warm. HarmonyOS asks the user once whether an app may
//! notify at all (day-part-permissions' `Permission::Notifications`); until they allow it, a post
//! answers `PermissionDenied`.
//!
//! A scheduled notification is an in-process timer, as on Linux: lost if the app exits, and held
//! back while it is in the background. The system's reminder agent could hold one instead, at the
//! cost of the `PUBLISH_AGENT_REMINDER` permission and its per-app reminder quota.

use day_bridge::Support;

use crate::{Capabilities, Channel, Importance, NotifId, Notification, NotifyError, channels};

pub(crate) fn capabilities() -> Capabilities {
    let available = post_native_support() != Support::Unsupported;
    Capabilities {
        post: available,
        schedule_while_dead: false,
        channels: false,
        badge: available,
        icon: false,
        tap_route: available,
        schedule_exact: false,
    }
}

pub(crate) fn register_channel(channel: &Channel) {
    channels::remember(channel);
}

/// `notificationManager.SlotType` for an importance.
fn slot(importance: Importance) -> i32 {
    match importance {
        // SOCIAL_COMMUNICATION: sound, vibration and a banner.
        Importance::High | Importance::Urgent => 1,
        // SERVICE_INFORMATION: sound, no banner.
        Importance::Default => 2,
        // CONTENT_INFORMATION: silent.
        Importance::Low => 3,
        // OTHER_TYPES: silent and folded away.
        Importance::Min => 0xFFFF,
    }
}

pub(crate) fn post(n: &Notification) -> Result<(), NotifyError> {
    let delay_ms = (n.delay_secs() * 1000.0).round() as i64;
    post_native(
        n.resolved_id().0 as i32,
        n.title_str(),
        n.body_str(),
        slot(channels::importance(n.channel_str())),
        n.route_str(),
        n.badge_count()
            .map_or(-1, |b| i32::try_from(b).unwrap_or(i32::MAX)),
        delay_ms,
    )
    .map_err(|e| match e {
        day_bridge::Error::Unsupported | day_bridge::Error::Runtime => NotifyError::Unsupported,
        // The arm's own marker for "the user has not allowed notifications".
        day_bridge::Error::Foreign(m) if m.contains("day-notify-denied") => {
            NotifyError::PermissionDenied
        }
        other => NotifyError::Failed(other.to_string()),
    })
}

pub(crate) fn cancel(id: NotifId) {
    cancel_native(id.0 as i32);
}

pub(crate) fn cancel_all() {
    cancel_all_native();
}

day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        /// Publish now, or after `delay_ms` (an in-process timer). `badge` < 0 leaves the icon's
        /// badge alone; `route` is empty for a tap that just opens the app. Throws
        /// `day-notify-denied` while the user has not allowed notifications.
        fn post_native(
            id: i32,
            title: &str,
            body: &str,
            slot: i32,
            route: &str,
            badge: i32,
            delay_ms: i64,
        ) -> Result<(), day_bridge::Error>;
        /// Remove one posted or pending notification.
        fn cancel_native(id: i32);
        /// Remove every notification this app posted, and every pending one.
        fn cancel_all_native();
    }

    #[day_bridge::impl(arkts, platforms = [ohos])]
    arkts!(
        prelude = r#"
            import { notificationManager } from '@kit.NotificationKit';
            import { wantAgent, WantAgent } from '@kit.AbilityKit';
            import { common } from '@kit.AbilityKit';
            import { BusinessError } from '@kit.BasicServicesKit';
        "#,
        body = r#"
            const dayPending: Map<number, number> = new Map<number, number>();

            function dayWarn(what: string, e: BusinessError): void {
              console.warn(`day-part-local-notify: ${what}: ${e.code} ${e.message}`);
            }

            async function dayPublish(id: number, title: string, body: string, slot: number,
                                      route: string, badge: number): Promise<void> {
              const ctx = getContext() as common.UIAbilityContext;
              const request: notificationManager.NotificationRequest = {
                id: id,
                notificationSlotType: slot,
                content: {
                  notificationContentType: notificationManager.ContentType.NOTIFICATION_CONTENT_BASIC_TEXT,
                  normal: { title: title, text: body }
                }
              };
              if (badge >= 0) {
                request.badgeNumber = badge;
              }
              // A tap starts the entry ability (the one `mainElement` names in every Day host),
              // which reads `day.uri` as a deep link: not the posting context's own ability,
              // which may be a secondary window's.
              const info: wantAgent.WantAgentInfo = {
                wants: [{
                  bundleName: ctx.abilityInfo.bundleName,
                  abilityName: 'EntryAbility',
                  parameters: route.length > 0 ? { 'day.uri': route } : {}
                }],
                actionType: wantAgent.OperationType.START_ABILITY,
                requestCode: id,
                wantAgentFlags: [wantAgent.WantAgentFlags.UPDATE_PRESENT_FLAG]
              };
              try {
                const agent: WantAgent = await wantAgent.getWantAgent(info);
                request.wantAgent = agent;
              } catch (e) {
                dayWarn('tap target', e as BusinessError);
              }
              await notificationManager.publish(request);
              if (badge >= 0) {
                await notificationManager.setBadgeNumber(badge);
              }
            }

            export function post_native(id: number, title: string, body: string, slot: number,
                                        route: string, badge: number, delay_ms: number): void {
              if (!notificationManager.isNotificationEnabledSync()) {
                throw new Error('day-notify-denied');
              }
              const pending = dayPending.get(id);
              if (pending !== undefined) {
                clearTimeout(pending);
                dayPending.delete(id);
              }
              const go = (): void => {
                dayPending.delete(id);
                dayPublish(id, title, body, slot, route, badge)
                  .catch((e: BusinessError) => dayWarn(`post ${id}`, e));
              };
              if (delay_ms > 0) {
                dayPending.set(id, setTimeout(go, delay_ms));
              } else {
                go();
              }
            }

            export function cancel_native(id: number): void {
              const pending = dayPending.get(id);
              if (pending !== undefined) {
                clearTimeout(pending);
                dayPending.delete(id);
              }
              notificationManager.cancel(id).catch((e: BusinessError) => dayWarn(`cancel ${id}`, e));
            }

            export function cancel_all_native(): void {
              dayPending.forEach((timer: number) => clearTimeout(timer));
              dayPending.clear();
              notificationManager.cancelAll().catch((e: BusinessError) => dayWarn('cancel all', e));
            }
        "#,
    );

    #[day_bridge::impl(rust, platforms = [other])]
    fn post_native(
        _id: i32,
        _title: &str,
        _body: &str,
        _slot: i32,
        _route: &str,
        _badge: i32,
        _delay_ms: i64,
    ) -> Result<(), day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn cancel_native(_id: i32) {}

    #[day_bridge::impl(rust, platforms = [other])]
    fn cancel_all_native() {}
}
