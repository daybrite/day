//! day-part-haptics — a HEADLESS cross-platform haptic-feedback API. No UI; any Rust code can depend
//! on this crate and call [`play`] to fire a haptic through the platform's NATIVE API.
//!
//! ```no_run
//! use day_part_haptics::Haptic;
//! if day_part_haptics::is_supported() {
//!     day_part_haptics::play(Haptic::Success);
//! }
//! ```
//!
//! Platform selection is purely `#[cfg(target_os)]` (a haptic engine is an OS concern, not a
//! widget-toolkit one): iOS uses UIKit's feedback generators, macOS `NSHapticFeedbackManager`, and
//! Android `Vibrator`/`VibrationEffect` (via a Java shim staged by `day build`). Every other target
//! — Windows, desktop Linux (GTK/Qt), HarmonyOS — has no haptic engine wired here, so [`play`] is a
//! no-op and [`is_supported`] returns `false`.
//!
//! [`play`] is **fire-and-forget** and best-effort: it never blocks, never returns an error, and
//! never panics. On hardware without a Taptic engine (an iOS Simulator, a Mac without a Force Touch
//! trackpad, an Android device whose `Vibrator.hasVibrator()` is false) the call is silently ignored.

/// A haptic-feedback style, modeled on iOS's three feedback-generator families so the same call maps
/// to a sensible native pattern everywhere.
///
/// - [`Light`](Haptic::Light) / [`Medium`](Haptic::Medium) / [`Heavy`](Haptic::Heavy) are physical
///   *impact* intensities (iOS `UIImpactFeedbackGenerator`).
/// - [`Success`](Haptic::Success) / [`Warning`](Haptic::Warning) / [`Error`](Haptic::Error) are
///   *notification* outcomes (iOS `UINotificationFeedbackGenerator`).
/// - [`Selection`](Haptic::Selection) is the light tick played as a value scrolls past a detent (iOS
///   `UISelectionFeedbackGenerator`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Haptic {
    /// A light impact — the subtlest bump.
    Light,
    /// A medium impact.
    Medium,
    /// A heavy impact — the most forceful bump.
    Heavy,
    /// A "task succeeded" notification.
    Success,
    /// A "something needs attention" notification.
    Warning,
    /// A "task failed" notification.
    Error,
    /// A selection changed (a value ticked past a detent).
    Selection,
}

/// Play a haptic through the platform's native API. Fire-and-forget: no return value, never panics,
/// and a no-op on platforms/hardware without a haptic engine (see [`is_supported`]).
pub fn play(h: Haptic) {
    imp::play(h)
}

/// Whether this platform has a haptic engine wired up. `true` on iOS/macOS/Android (even on a
/// Simulator or a device that happens to lack the hardware — this reports API availability, not a
/// live hardware probe), `false` on every other target, where [`play`] is a no-op.
pub fn is_supported() -> bool {
    imp::is_supported()
}

// ---------------------------------------------------------------------------
// Per-OS implementations. Each exposes `fn play(Haptic)` + `fn is_supported() -> bool`.
// ---------------------------------------------------------------------------

#[cfg(target_os = "ios")]
#[path = "ios.rs"]
mod imp;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;

#[cfg(target_os = "android")]
#[path = "android.rs"]
mod imp;

// Any other platform — Windows, desktop Linux (GTK/Qt), HarmonyOS — has no haptic engine wired here.
// (HarmonyOS could in principle drive `libohvibrator`, but it needs an effect/attribute struct and
// the `ohos.permission.VIBRATE` grant, so it is left as a best-effort no-op for now.)
#[cfg(not(any(target_os = "ios", target_os = "macos", target_os = "android")))]
mod imp {
    pub fn play(_h: super::Haptic) {}
    pub fn is_supported() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::Haptic;

    // Firing every style must never panic, on any host (headless CI included). On a host without a
    // haptic engine these are no-ops; the point is that they return cleanly.
    #[test]
    fn play_never_panics() {
        for h in [
            Haptic::Light,
            Haptic::Medium,
            Haptic::Heavy,
            Haptic::Success,
            Haptic::Warning,
            Haptic::Error,
            Haptic::Selection,
        ] {
            super::play(h);
        }
    }

    // Support is a fixed per-platform fact: true only on the three haptic-capable OSes.
    #[test]
    fn is_supported_matches_platform() {
        let expected = cfg!(any(
            target_os = "ios",
            target_os = "macos",
            target_os = "android"
        ));
        assert_eq!(super::is_supported(), expected);
    }
}
