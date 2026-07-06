//! day-battery — a HEADLESS cross-platform battery-status API. No UI; any Rust code can depend on this
//! crate and call [`status`] to read the device battery through the platform's NATIVE API.
//!
//! ```no_run
//! if let Some(b) = day_battery::status() {
//!     println!("{:?} at {:?}%", b.state, b.percent());
//! }
//! ```
//!
//! Platform selection is purely `#[cfg(target_os)]`/`#[cfg(target_env)]` (a battery is an OS concern,
//! not a widget-toolkit one): macOS uses IOKit, iOS `UIDevice`, Windows `GetSystemPowerStatus`, Linux
//! `/sys/class/power_supply`, HarmonyOS the native `libohbattery_info.so`, and Android
//! `BatteryManager` (via a Java shim staged by `day build`). Platforms without a battery API — or
//! devices with no battery — return `None`.

/// A snapshot of the device battery.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BatteryStatus {
    /// Charge fraction in `0.0..=1.0`, or `None` if the level is unknown (e.g. a simulator).
    pub level: Option<f32>,
    /// Charging / discharging / …
    pub state: BatteryState,
}

/// The battery's charging state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BatteryState {
    /// Plugged in and charging.
    Charging,
    /// Running on battery.
    Discharging,
    /// Plugged in and fully charged.
    Full,
    /// Plugged in but not charging (e.g. charge-limited / paused).
    NotCharging,
    /// State could not be determined.
    #[default]
    Unknown,
}

impl BatteryStatus {
    /// The charge level as a whole percentage `0..=100`, if known.
    pub fn percent(&self) -> Option<u8> {
        self.level
            .map(|l| (l.clamp(0.0, 1.0) * 100.0).round() as u8)
    }
    /// Whether the battery is currently charging.
    pub fn is_charging(&self) -> bool {
        matches!(self.state, BatteryState::Charging)
    }
}

/// Read the current battery status via the platform's native API. Returns `None` when there is no
/// battery API for the platform, or the device has no battery.
pub fn status() -> Option<BatteryStatus> {
    imp::status()
}

// ---------------------------------------------------------------------------
// Per-OS implementations. Each exposes `fn status() -> Option<BatteryStatus>`.
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;

#[cfg(target_os = "ios")]
#[path = "ios.rs"]
mod imp;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;

// Desktop/embedded Linux reads sysfs; HarmonyOS (also `target_os = "linux"`) sandboxes that away,
// so it uses its own native battery API instead.
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
#[path = "linux.rs"]
mod imp;

#[cfg(all(target_os = "linux", target_env = "ohos"))]
#[path = "ohos.rs"]
mod imp;

#[cfg(target_os = "android")]
#[path = "android.rs"]
mod imp;

// Any other platform: no native battery API.
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "android"
)))]
mod imp {
    pub fn status() -> Option<super::BatteryStatus> {
        None
    }
}

#[cfg(test)]
mod tests {
    // Reading must never panic, whether or not the host has a battery (CI runners typically don't).
    #[test]
    fn status_does_not_panic() {
        let s = super::status();
        if let Some(b) = s {
            assert!(b.percent().is_none_or(|p| p <= 100));
        }
    }
}
