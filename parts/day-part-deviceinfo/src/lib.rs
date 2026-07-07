//! day-part-deviceinfo — a HEADLESS cross-platform device-identity API. No UI; any Rust code can
//! depend on this crate and call [`get`] for a snapshot of the device model, OS name/version, and
//! whether it is running on a simulator/emulator, through each platform's NATIVE API.
//!
//! ```no_run
//! let d = day_part_deviceinfo::get();
//! println!("{} — {} {}{}", d.model, d.system_name, d.system_version,
//!     if d.is_simulator { " (simulator)" } else { "" });
//! ```
//!
//! Platform selection is purely `#[cfg(target_os)]`/`#[cfg(target_env)]` (device identity is an OS
//! concern, not a widget-toolkit one): macOS uses `ProcessInfo` + `sysctl`, iOS `UIDevice`, Windows
//! `RtlGetVersion`, Linux `/etc/os-release` + DMI, HarmonyOS the native `libdeviceinfo_ndk.so`, and
//! Android `android.os.Build` (via a Java shim staged by `day build`).
//!
//! [`get`] **never panics** and never returns an error: fields that a platform cannot report are
//! filled with a sensible fallback (`"Unknown"`, or `system_name` set to the OS family name).

/// A snapshot of the device's identity. Every field is best-effort — each OS reports a different
/// slice of the truth, and unknown fields fall back to `"Unknown"` (see the per-platform notes in
/// docs/deviceinfo.md).
#[derive(Clone, Debug)]
pub struct DeviceInfo {
    /// The hardware model identifier — e.g. `"MacBookPro18,3"` (macOS `hw.model`), `"iPhone"`
    /// (iOS `UIDevice.model` reports the marketing class), `"Pixel 7"` (Android `Build.MODEL`), or
    /// the DMI `product_name` on Linux. `"Unknown"` when unreadable.
    pub model: String,
    /// The operating-system family name — `"macOS"`, `"iOS"` / `"iPadOS"`, `"Windows"`, the Linux
    /// distro `NAME`, `"OpenHarmony"` / `"HarmonyOS"`, or `"Android"`.
    pub system_name: String,
    /// The OS version string — a dotted `major.minor[.patch]` on Apple/Windows, `VERSION_ID` on
    /// Linux, `Build.VERSION.RELEASE` on Android, or the native display version on HarmonyOS.
    /// `"Unknown"` when unreadable.
    pub system_version: String,
    /// Whether the process is running on a simulator (iOS) or emulator (Android). Always `false`
    /// on desktop platforms, which have no such notion.
    pub is_simulator: bool,
}

/// Read the current device-identity snapshot via the platform's native API. Never panics; unknown
/// fields fall back to `"Unknown"` (or the OS family name for `system_name`).
pub fn get() -> DeviceInfo {
    imp::get()
}

// ---------------------------------------------------------------------------
// Per-OS implementations. Each exposes `fn get() -> DeviceInfo`.
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

// Desktop/embedded Linux parses /etc/os-release + DMI; HarmonyOS (also `target_os = "linux"`)
// sandboxes those away, so it uses its own native device-info API instead.
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
#[path = "linux.rs"]
mod imp;

#[cfg(all(target_os = "linux", target_env = "ohos"))]
#[path = "ohos.rs"]
mod imp;

#[cfg(target_os = "android")]
#[path = "android.rs"]
mod imp;

// Any other platform: no native device-identity API — report the compile-time OS family and leave
// the rest unknown.
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "android"
)))]
mod imp {
    pub fn get() -> super::DeviceInfo {
        super::DeviceInfo {
            model: "Unknown".to_string(),
            system_name: std::env::consts::OS.to_string(),
            system_version: "Unknown".to_string(),
            is_simulator: false,
        }
    }
}

#[cfg(test)]
mod tests {
    // Reading must never panic, and must never leave a field empty (the contract is a sensible
    // fallback, not "").
    #[test]
    fn get_never_panics_and_fills_fields() {
        let d = super::get();
        assert!(!d.model.is_empty(), "model must have a fallback");
        assert!(
            !d.system_name.is_empty(),
            "system_name must have a fallback"
        );
        assert!(
            !d.system_version.is_empty(),
            "system_version must have a fallback"
        );
    }

    // A dev/CI Mac reports "macOS", a non-empty version, and never claims to be a simulator.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_reports_macos() {
        let d = super::get();
        assert_eq!(d.system_name, "macOS");
        assert!(!d.is_simulator);
        // operatingSystemVersion always has at least a major version digit.
        assert!(
            d.system_version
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
        );
    }
}
