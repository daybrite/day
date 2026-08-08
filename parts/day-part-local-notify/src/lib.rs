// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-part-local-notify — a HEADLESS cross-platform local-notification API. No UI; any Rust code
//! can depend on this crate and post or schedule a notification through the platform's NATIVE API.
//!
//! ```no_run
//! use day_part_local_notify::{Channel, Importance, Notification, Trigger};
//! use std::time::Duration;
//!
//! Channel::new("timers", Importance::High).sound(true).register();
//!
//! Notification::new("Timer done")
//!     .body("Your 5 minute timer finished.")
//!     .channel("timers")
//!     .route("services")            // tapping navigates here (docs/navigation.md)
//!     .trigger(Trigger::In(Duration::from_secs(5)))
//!     .post();
//! ```
//!
//! LOCAL only: this crate never talks to a server. Server-sent notifications are
//! `day-part-push-notify`, which layers on this one for display (docs/notify.md).
//!
//! Platform selection is purely `#[cfg(target_os)]`. What each backend can actually do differs, so
//! an app asks [`capabilities`] rather than branching on a target name — most importantly
//! [`Capabilities::schedule_while_dead`], which is false on Linux and the web, where a scheduled
//! notification is lost if the process or tab goes away.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// How loudly a channel's notifications announce themselves. Fixed when the channel is registered:
/// Android's `NotificationChannel` importance is immutable after first registration (the user owns
/// it thereafter), so this is a required argument at [`Channel::new`] rather than a setter, to force
/// the decision up front on every platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Importance {
    /// No sound, no badge, minimized in the shade.
    Min,
    /// No sound.
    Low,
    /// The platform default.
    Default,
    /// Sound, and a heads-up banner where the platform has one.
    High,
    /// The most attention the platform allows short of a full-screen alarm. On Apple this is the
    /// time-sensitive interruption level, which needs the matching entitlement to break through
    /// Focus — without it the system quietly downgrades it to `High`.
    Urgent,
}

impl Importance {
    /// A stable ASCII id — locale-independent, so it is safe to assert on in dayscript and to log.
    pub fn as_str(self) -> &'static str {
        match self {
            Importance::Min => "min",
            Importance::Low => "low",
            Importance::Default => "default",
            Importance::High => "high",
            Importance::Urgent => "urgent",
        }
    }
}

/// When a notification should fire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// Immediately.
    Now,
    /// After a delay. Where [`Capabilities::schedule_while_dead`] is true the OS holds it, so it
    /// fires even if the app exits; elsewhere it is an in-process timer and dies with the process.
    In(Duration),
}

/// A notification's identity. Posting again with the same id UPDATES the existing notification
/// rather than stacking a second one, and [`cancel`] takes one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NotifId(pub u32);

/// What this platform can actually do. Query it rather than branching on the target name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Notifications can be posted at all. False on the targets with no implementation wired yet
    /// (Windows, HarmonyOS), where every call is a no-op returning [`NotifyError::Unsupported`].
    pub post: bool,
    /// A scheduled notification is held by the OS and fires even if the app is not running. False
    /// on Linux and the web, where scheduling is an in-process timer.
    pub schedule_while_dead: bool,
    /// The platform has a native channel model the user can configure per channel (Android).
    /// Elsewhere channels still group and carry importance, but the OS exposes no per-channel UI.
    pub channels: bool,
    /// A per-notification badge count on the app icon.
    pub badge: bool,
    /// A custom small icon per notification.
    pub icon: bool,
    /// Tapping routes into the app (docs/navigation.md).
    pub tap_route: bool,
    /// A scheduled notification fires at its exact moment. False where the platform may delay it —
    /// Android 12+ can withhold the exact-alarm grant, so the notification still arrives but may
    /// run late in Doze. Worth surfacing: a clock app that silently drifts looks broken.
    pub schedule_exact: bool,
}

/// Why a notification could not be posted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotifyError {
    /// This target has no local-notification implementation wired.
    Unsupported,
    /// The user has not granted notification permission (request it with day-part-permissions).
    PermissionDenied,
    /// The platform refused the post; the string is its own message, for logs.
    Failed(String),
}

impl std::fmt::Display for NotifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotifyError::Unsupported => write!(f, "local notifications are not supported here"),
            NotifyError::PermissionDenied => write!(f, "notification permission not granted"),
            NotifyError::Failed(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for NotifyError {}

/// A notification channel: a named group carrying an [`Importance`] and a sound preference.
///
/// Register every channel before posting to it. Registration is idempotent, and on Android the
/// importance is fixed the first time — see [`Importance`].
#[derive(Clone, Debug)]
pub struct Channel {
    id: String,
    name: String,
    importance: Importance,
    sound: bool,
}

impl Channel {
    /// A channel with `id` as both its key and its initial user-visible name.
    pub fn new(id: impl Into<String>, importance: Importance) -> Channel {
        let id = id.into();
        Channel {
            name: id.clone(),
            id,
            importance,
            sound: importance >= Importance::High,
        }
    }

    /// The user-visible channel name (Android shows it in the app's notification settings).
    pub fn name(mut self, name: impl Into<String>) -> Channel {
        self.name = name.into();
        self
    }

    /// Whether notifications on this channel play a sound.
    pub fn sound(mut self, on: bool) -> Channel {
        self.sound = on;
        self
    }

    /// Register the channel. Idempotent; call before posting to it.
    pub fn register(self) {
        imp::register_channel(&self);
    }

    /// The channel key.
    pub fn id(&self) -> &str {
        &self.id
    }
    /// The user-visible name.
    pub fn display_name(&self) -> &str {
        &self.name
    }
    /// The channel's importance.
    pub fn importance(&self) -> Importance {
        self.importance
    }
    /// Whether this channel plays a sound.
    pub fn plays_sound(&self) -> bool {
        self.sound
    }
}

/// A notification to post.
///
/// Everything a scheduled notification renders is captured HERE, at post time — a `Trigger::In`
/// notification may fire in a process that has no Day tree alive (the Android alarm receiver runs
/// in a fresh process), so the content cannot be a signal or a closure. This is the one place Day's
/// reactivity deliberately does not reach (docs/notify.md).
#[derive(Clone, Debug)]
pub struct Notification {
    id: Option<NotifId>,
    title: String,
    body: String,
    subtitle: String,
    channel: String,
    route: String,
    icon: String,
    badge: Option<u32>,
    trigger: Trigger,
}

impl Notification {
    /// A notification with a title. Everything else is optional.
    pub fn new(title: impl Into<String>) -> Notification {
        Notification {
            id: None,
            title: title.into(),
            body: String::new(),
            subtitle: String::new(),
            channel: "default".into(),
            route: String::new(),
            icon: String::new(),
            badge: None,
            trigger: Trigger::Now,
        }
    }

    /// The body text.
    pub fn body(mut self, body: impl Into<String>) -> Notification {
        self.body = body.into();
        self
    }

    /// A subtitle, where the platform has one (Apple). Ignored elsewhere.
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Notification {
        self.subtitle = subtitle.into();
        self
    }

    /// The channel to post on (default: `"default"`). Register it first.
    pub fn channel(mut self, channel: impl Into<String>) -> Notification {
        self.channel = channel.into();
        self
    }

    /// The Day route to navigate to when the notification is tapped (docs/navigation.md). Delivered
    /// through `day_core::request_route`, so it works whether the tap wakes a running app or cold
    /// starts the process.
    pub fn route(mut self, route: impl Into<String>) -> Notification {
        self.route = route.into();
        self
    }

    /// A custom small-icon name, where the platform supports one ([`Capabilities::icon`]). On
    /// Android this resolves as a drawable resource name; elsewhere it is ignored.
    pub fn icon(mut self, icon: impl Into<String>) -> Notification {
        self.icon = icon.into();
        self
    }

    /// The app-icon badge count to show ([`Capabilities::badge`]).
    pub fn badge(mut self, count: u32) -> Notification {
        self.badge = Some(count);
        self
    }

    /// A stable id, so a later post with the same id replaces this notification.
    pub fn id(mut self, id: NotifId) -> Notification {
        self.id = Some(id);
        self
    }

    /// When to fire (default [`Trigger::Now`]).
    pub fn trigger(mut self, trigger: Trigger) -> Notification {
        self.trigger = trigger;
        self
    }

    /// Post (or schedule) the notification. Returns the id it was posted under, so it can be
    /// cancelled or replaced.
    pub fn post(self) -> Result<NotifId, NotifyError> {
        let id = self.id.unwrap_or_else(next_id);
        let n = Notification {
            id: Some(id),
            ..self
        };
        imp::post(&n)?;
        Ok(id)
    }

    // --- accessors the platform arms read ---
    // Which of these are live depends on the target: Apple has no named small icon and no tap
    // delegate yet, so `icon_str`/`route_str`/`trigger_kind` are dead on a macOS host build while
    // the Android, Linux, and web arms all use them. The allow is on the accessor group only.
    #[allow(dead_code)]
    pub(crate) fn resolved_id(&self) -> NotifId {
        self.id.unwrap_or(NotifId(0))
    }
    #[allow(dead_code)]
    pub(crate) fn title_str(&self) -> &str {
        &self.title
    }
    #[allow(dead_code)]
    pub(crate) fn body_str(&self) -> &str {
        &self.body
    }
    #[allow(dead_code)]
    pub(crate) fn subtitle_str(&self) -> &str {
        &self.subtitle
    }
    #[allow(dead_code)]
    pub(crate) fn channel_str(&self) -> &str {
        &self.channel
    }
    #[allow(dead_code)]
    pub(crate) fn route_str(&self) -> &str {
        &self.route
    }
    #[allow(dead_code)]
    pub(crate) fn icon_str(&self) -> &str {
        &self.icon
    }
    #[allow(dead_code)]
    pub(crate) fn badge_count(&self) -> Option<u32> {
        self.badge
    }
    #[allow(dead_code)]
    pub(crate) fn trigger_kind(&self) -> Trigger {
        self.trigger
    }
    /// The delay in whole seconds, or 0 for immediate — the form every platform arm wants.
    #[allow(dead_code)]
    pub(crate) fn delay_secs(&self) -> f64 {
        match self.trigger {
            Trigger::Now => 0.0,
            Trigger::In(d) => d.as_secs_f64(),
        }
    }
}

/// Ids handed out by [`Notification::post`] when the caller supplies none. Starts above zero so a
/// generated id is never confused with the `NotifId(0)` placeholder.
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

fn next_id() -> NotifId {
    NotifId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// What this platform can do (see [`Capabilities`]).
pub fn capabilities() -> Capabilities {
    imp::capabilities()
}

/// Whether local notifications work at all here — shorthand for `capabilities().post`.
pub fn is_supported() -> bool {
    imp::capabilities().post
}

/// Remove a posted or scheduled notification.
pub fn cancel(id: NotifId) {
    imp::cancel(id);
}

/// Remove every notification this app posted or scheduled.
pub fn cancel_all() {
    imp::cancel_all();
}

/// Deliver a tap into the app's navigation. The platform arms call this; it is public so a host
/// that receives the tap itself (an Android activity intent extra) can hand it over.
pub fn deliver_tap(route: &str) {
    if !route.is_empty() {
        day_core::request_route(route);
    }
}

/// The registered channels, shared by every platform arm.
///
/// Android has a real `NotificationChannel` registry and reads this only to build it; the other
/// platforms have none, so this IS their channel model — the place a notification's importance and
/// sound are looked up at post time. Process-global rather than thread-local because a post can
/// come from any thread.
pub(crate) mod channels {
    use super::{Channel, Importance};
    use std::collections::HashMap;
    use std::sync::Mutex;

    static CHANNELS: Mutex<Option<HashMap<String, (Importance, bool)>>> = Mutex::new(None);

    fn with<T>(f: impl FnOnce(&mut HashMap<String, (Importance, bool)>) -> T) -> T {
        // Poison recovery: the map is plain data, so a panic elsewhere must not wedge every later
        // post.
        let mut guard = CHANNELS.lock().unwrap_or_else(|e| e.into_inner());
        f(guard.get_or_insert_with(HashMap::new))
    }

    #[allow(dead_code)] // unused on targets that fall through to the Unsupported stub (wasm32).
    pub(crate) fn remember(channel: &Channel) {
        with(|m| {
            m.insert(
                channel.id().to_string(),
                (channel.importance(), channel.plays_sound()),
            )
        });
    }

    /// A channel's importance, defaulting to [`Importance::Default`] for one never registered —
    /// posting to an unknown channel should still notify, not silently vanish.
    #[allow(dead_code)] // read by the Apple arm; Android asks the platform instead.
    pub(crate) fn importance(id: &str) -> Importance {
        with(|m| m.get(id).map(|(i, _)| *i)).unwrap_or(Importance::Default)
    }

    /// Whether a channel plays a sound (unregistered channels stay quiet).
    #[allow(dead_code)] // read by the Apple arm; Android sets sound on the channel itself.
    pub(crate) fn plays_sound(id: &str) -> bool {
        with(|m| m.get(id).map(|(_, s)| *s)).unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// Per-OS implementations. Each exposes: register_channel, post, cancel, cancel_all, capabilities.
// ---------------------------------------------------------------------------

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[path = "apple.rs"]
mod imp;

#[cfg(target_os = "android")]
#[path = "android.rs"]
mod imp;

// Linux and web-dom fall through to the honest stub for now; both are designed in docs/notify.md.
// Their modules are added with their implementations rather than ahead of them, because declaring
// a `mod` whose file does not exist breaks `cargo fmt --all` for the whole workspace.
#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "android")))]
#[path = "unsupported.rs"]
mod imp;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_are_unique_and_nonzero() {
        let a = next_id();
        let b = next_id();
        assert_ne!(a, b);
        assert_ne!(a.0, 0);
    }

    #[test]
    fn channel_sound_defaults_by_importance() {
        // A Low channel that beeps would be wrong on every platform; High and up should announce.
        assert!(!Channel::new("c", Importance::Low).plays_sound());
        assert!(Channel::new("c", Importance::High).plays_sound());
        // …and stays overridable either way.
        assert!(Channel::new("c", Importance::Low).sound(true).plays_sound());
    }

    #[test]
    fn delay_is_zero_for_immediate() {
        assert_eq!(Notification::new("t").delay_secs(), 0.0);
        assert_eq!(
            Notification::new("t")
                .trigger(Trigger::In(Duration::from_secs(5)))
                .delay_secs(),
            5.0
        );
    }

    #[test]
    fn builder_round_trips_every_field() {
        let n = Notification::new("Title")
            .body("Body")
            .subtitle("Sub")
            .channel("chan")
            .route("services")
            .icon("bell")
            .badge(3)
            .id(NotifId(42));
        assert_eq!(n.title_str(), "Title");
        assert_eq!(n.body_str(), "Body");
        assert_eq!(n.subtitle_str(), "Sub");
        assert_eq!(n.channel_str(), "chan");
        assert_eq!(n.route_str(), "services");
        assert_eq!(n.icon_str(), "bell");
        assert_eq!(n.badge_count(), Some(3));
        assert_eq!(n.resolved_id(), NotifId(42));
    }

    #[test]
    fn importance_ids_are_stable() {
        // dayscript and logs assert on these; they must not drift with the display name.
        assert_eq!(Importance::Urgent.as_str(), "urgent");
        assert_eq!(Importance::Default.as_str(), "default");
    }
}
