//! day-part-network — a HEADLESS cross-platform network-connectivity API. No UI; any Rust code can
//! depend on this crate and call [`status`] for a snapshot of the device's connectivity through the
//! platform's NATIVE API.
//!
//! ```no_run
//! if let Some(n) = day_part_network::status() {
//!     println!("online: {}, kind: {:?}", n.online, n.kind);
//! }
//! ```
//!
//! Platform selection is purely `#[cfg(target_os)]`/`#[cfg(target_env)]` (connectivity is an OS
//! concern, not a widget-toolkit one): macOS and iOS share one `SCNetworkReachability` file, Windows
//! uses `GetNetworkConnectivityHint`, Linux scans `/sys/class/net`, HarmonyOS the native
//! `libnet_connection.so`, and Android `ConnectivityManager` (via a Java shim staged by `day build`).
//! Platforms without a connectivity API return `None`.
//!
//! Every field is **best-effort** — each OS reports a different slice of the truth. `kind` and
//! `expensive` in particular vary per platform (macOS reachability carries no transport info, Linux
//! infers the kind from interface names, only Android/HarmonyOS report meteredness); see the
//! per-field docs and docs/network.md for the honest per-platform matrix.

/// The transport class of the active network connection. Best-effort — not every platform can
/// distinguish these (see [`NetworkStatus::kind`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetworkKind {
    /// Wi-Fi (or, on iOS, any non-cellular transport — reachability can't tell Wi-Fi from wired).
    Wifi,
    /// A cellular / mobile-data connection.
    Cellular,
    /// Wired ethernet.
    Ethernet,
    /// Connected, but the transport is unknown or something else (VPN, bluetooth tether, …).
    Other,
    /// No active network connection.
    #[default]
    None,
}

/// A snapshot of the device's network connectivity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkStatus {
    /// Whether the device currently has a usable network connection. On Apple platforms this is
    /// *reachability* of the default route (traffic would flow without user intervention) — it does
    /// not probe the internet; Android reports the system's validated-connectivity verdict; Linux
    /// reports link-level "an interface is up".
    pub online: bool,
    /// The connection's transport class. Best-effort: exact on Android/HarmonyOS/Linux(-by-name);
    /// on iOS only cellular-vs-not is known (non-cellular reports [`NetworkKind::Wifi`]); macOS and
    /// Windows report [`NetworkKind::Other`] when online.
    pub kind: NetworkKind,
    /// Whether the connection is metered/expensive, where the platform reports it (Android's
    /// `NOT_METERED` capability, HarmonyOS likewise, Windows' cost hint, iOS cellular). `None` when
    /// unknown.
    pub expensive: Option<bool>,
}

/// Read a connectivity snapshot via the platform's native API. Returns `None` when there is no
/// connectivity API on the platform (or the reading failed) — distinct from a successful reading
/// that says offline (`Some` with `online: false`).
pub fn status() -> Option<NetworkStatus> {
    imp::status()
}

// ---------------------------------------------------------------------------
// Per-OS implementations. Each exposes `fn status() -> Option<NetworkStatus>`.
// ---------------------------------------------------------------------------

// macOS + iOS share one SCNetworkReachability impl (the IsWWAN flag exists only on iOS).
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[path = "apple.rs"]
mod imp;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;

// Desktop/embedded Linux scans sysfs; HarmonyOS (also `target_os = "linux"`) sandboxes that away,
// so it uses its own native connection-management API instead.
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
#[path = "linux.rs"]
mod imp;

#[cfg(all(target_os = "linux", target_env = "ohos"))]
#[path = "ohos.rs"]
mod imp;

#[cfg(target_os = "android")]
#[path = "android.rs"]
mod imp;

// Any other platform: no native connectivity API.
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "android"
)))]
mod imp {
    pub fn status() -> Option<super::NetworkStatus> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::NetworkKind;

    // Reading must never panic, connected or not.
    #[test]
    fn status_does_not_panic() {
        if let Some(n) = super::status() {
            // An offline snapshot must not claim a transport.
            if n.kind == NetworkKind::None {
                assert!(!n.online);
            }
        }
    }

    // macOS always has SCNetworkReachability, and dev/CI hosts running the test suite are
    // networked — so a real reading must come back and say online. (Tolerant of everything else:
    // kind/expensive are best-effort.)
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_host_reads_online() {
        let n = super::status().expect("macOS has SCNetworkReachability");
        assert!(n.online, "expected an online host, got {n:?}");
    }
}
