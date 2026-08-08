// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! iOS and macOS: `UNUserNotificationCenter`, one framework covering both OSes and both immediate
//! and scheduled delivery.
//!
//! Scheduling is OS-HELD: a `UNTimeIntervalNotificationTrigger` fires even if the app has exited,
//! which is what lets an alarm ring without a background process. That is also why the content is
//! snapshotted at post time — the notification is rendered by the system, not by the app.
//!
//! AUTHORIZATION is deliberately not requested here. day-part-permissions owns
//! `Permission::Notifications` on every platform, and duplicating the prompt would mean two crates
//! racing to ask. Note what that costs: `getNotificationSettings` is block-based, so this arm
//! cannot synchronously tell "denied" from "allowed" and never returns `PermissionDenied` — an
//! unauthorized post is accepted here and dropped by the system. An app that wants to explain the
//! silence asks day-part-permissions for the status.
//!
//! macOS caveat worth knowing when nothing appears: `UNUserNotificationCenter` requires a SIGNED,
//! BUNDLED app with a bundle identifier. `day pack` produces one; a bare `cargo run` binary does
//! not, and there the center itself is nil — reported here as `Unsupported` rather than a silent
//! no-op (docs/notify.md).

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AllocAnyThread, define_class, msg_send};
use objc2_foundation::{NSNumber, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNMutableNotificationContent, UNNotification, UNNotificationPresentationOptions,
    UNNotificationRequest, UNNotificationResponse, UNNotificationSound,
    UNTimeIntervalNotificationTrigger, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};

use crate::{Capabilities, Channel, Importance, NotifId, Notification, NotifyError};

pub(crate) fn capabilities() -> Capabilities {
    // No center (an unbundled macOS binary) means NOTHING works, so every capability must read
    // false — reporting `badge: true` beside `post: false` would have a UI offer a control that
    // cannot fire.
    if center().is_none() {
        return Capabilities::default();
    }
    Capabilities {
        post: true,
        // The system holds the trigger, so it fires with the app dead.
        schedule_while_dead: true,
        // Apple has no user-facing per-channel settings model; a channel still groups (thread
        // identifier) and carries importance, so this reports the honest `false`.
        channels: false,
        badge: true,
        // A custom small icon needs a UNNotificationAttachment, which is a file-URL image rather
        // than a named resource — out of scope for this phase.
        icon: false,
        tap_route: true,
        // UNTimeIntervalNotificationTrigger fires on time; the system does not defer it.
        schedule_exact: true,
    }
}

/// The notification center, or `None` when this process cannot have one (an unbundled macOS
/// binary). `currentNotificationCenter` traps rather than returning nil in that case on some OS
/// versions, so the bundle identifier is checked first.
fn center() -> Option<objc2::rc::Retained<UNUserNotificationCenter>> {
    #[cfg(target_os = "macos")]
    {
        // No bundle id ⇒ unbundled ⇒ the center is unusable. Checking here keeps the failure a
        // clean `Unsupported` instead of an abort inside the framework.
        objc2_foundation::NSBundle::mainBundle().bundleIdentifier()?;
    }
    Some(UNUserNotificationCenter::currentNotificationCenter())
}

/// Apple has no channel registry to populate — importance and grouping ride on each notification —
/// so registration only records the channel for later lookup.
pub(crate) fn register_channel(channel: &Channel) {
    super::channels::remember(channel);
}

define_class!(
    // The delegate decides what a notification does while the app is running. Without one, iOS
    // treats a foreground notification as already-seen and shows NOTHING — which is why tapping
    // "Post" in the open app appeared to do nothing at all.
    #[unsafe(super(NSObject))]
    // Creatable from any thread: the first `post` may run wherever the app called it.
    #[thread_kind = AllocAnyThread]
    #[name = "DayLocalNotifyDelegate"]
    #[ivars = ()]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            // Show it like any other notification even though we are frontmost: banner (or the
            // older Alert on pre-14 systems), the notification list, and the sound the channel asked
            // for. `List` alone would file it silently into Notification Center.
            let opts = UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound;
            (*handler).call((opts,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            handler: &block2::DynBlock<dyn Fn()>,
        ) {
            // The route rides in userInfo, put there at post time. `request_route` handles both a
            // warm tap and one that cold-started the process (docs/notify.md).
            let content = response.notification().request().content();
            let route = content
                .userInfo()
                .objectForKey(&*NSString::from_str(ROUTE_KEY));
            if let Some(route) = route {
                let text: Retained<NSString> = unsafe { Retained::cast_unchecked(route) };
                crate::deliver_tap(&text.to_string());
            }
            (*handler).call(());
        }
    }
);

/// The key the Day route travels under inside `UNNotificationContent.userInfo`.
const ROUTE_KEY: &str = "day.route";

/// Install the delegate once. It must be set before a tap can be delivered, and `UNUserNotification`
/// keeps only a weak reference, so the instance is leaked deliberately — it lives for the process.
fn install_delegate(center: &UNUserNotificationCenter) {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let this = Delegate::alloc().set_ivars(());
        // SAFETY: NSObject's designated initializer.
        let delegate: Retained<Delegate> = unsafe { msg_send![super(this), init] };
        let proto = ProtocolObject::from_ref(&*delegate);
        center.setDelegate(Some(proto));
        std::mem::forget(delegate);
    });
}

pub(crate) fn post(n: &Notification) -> Result<(), NotifyError> {
    let Some(center) = center() else {
        return Err(NotifyError::Unsupported);
    };
    install_delegate(&center);
    let content = UNMutableNotificationContent::new();
    {
        content.setTitle(&NSString::from_str(n.title_str()));
        if !n.body_str().is_empty() {
            content.setBody(&NSString::from_str(n.body_str()));
        }
        if !n.subtitle_str().is_empty() {
            content.setSubtitle(&NSString::from_str(n.subtitle_str()));
        }
        // Group related notifications; the channel is the natural thread key.
        content.setThreadIdentifier(&NSString::from_str(n.channel_str()));
        if !n.route_str().is_empty() {
            let typed = objc2_foundation::NSDictionary::from_slices::<NSString>(
                &[&*NSString::from_str(ROUTE_KEY)],
                &[&*NSString::from_str(n.route_str())],
            );
            // setUserInfo takes the type-erased dictionary; the concrete one is layout-identical.
            let erased: Retained<objc2_foundation::NSDictionary> =
                unsafe { Retained::cast_unchecked(typed) };
            unsafe { content.setUserInfo(&erased) };
        }
        if let Some(badge) = n.badge_count() {
            content.setBadge(Some(&NSNumber::new_u32(badge)));
        }
        if super::channels::plays_sound(n.channel_str()) {
            content.setSound(Some(&UNNotificationSound::defaultSound()));
        }
        set_interruption_level(&content, super::channels::importance(n.channel_str()));
    }

    // A zero interval is rejected by the framework, so immediate posts carry no trigger at all.
    let trigger = if n.delay_secs() > 0.0 {
        Some(
            UNTimeIntervalNotificationTrigger::triggerWithTimeInterval_repeats(
                n.delay_secs(),
                false,
            ),
        )
    } else {
        None
    };

    let request = {
        UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(&n.resolved_id().0.to_string()),
            &content,
            // The request takes the base class; the concrete trigger upcasts.
            trigger.as_deref().map(|t| &**t),
        )
    };
    // A nil completion handler is allowed; errors surface in the system log. Taking the callback
    // would mean threading a block back to the UI thread for a result the caller cannot act on.
    center.addNotificationRequest_withCompletionHandler(&request, None);
    Ok(())
}

/// `Urgent` asks for the time-sensitive level, which breaks through Focus. Without the matching
/// entitlement the system quietly downgrades it, which is why this is not an error.
fn set_interruption_level(content: &UNMutableNotificationContent, importance: Importance) {
    use objc2_user_notifications::UNNotificationInterruptionLevel as Level;
    let level = match importance {
        Importance::Min | Importance::Low => Level::Passive,
        Importance::Default | Importance::High => Level::Active,
        Importance::Urgent => Level::TimeSensitive,
    };
    content.setInterruptionLevel(level);
}

pub(crate) fn cancel(id: NotifId) {
    let Some(center) = center() else { return };
    let ids =
        objc2_foundation::NSArray::from_retained_slice(&[NSString::from_str(&id.0.to_string())]);
    center.removePendingNotificationRequestsWithIdentifiers(&ids);
    center.removeDeliveredNotificationsWithIdentifiers(&ids);
}

pub(crate) fn cancel_all() {
    let Some(center) = center() else { return };
    center.removeAllPendingNotificationRequests();
    center.removeAllDeliveredNotifications();
}
