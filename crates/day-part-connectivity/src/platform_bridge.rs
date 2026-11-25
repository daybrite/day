// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(target_os = "android")]
use super::NetworkKind;
use super::{NetworkConditions, NetworkStatus};

#[cfg(target_arch = "wasm32")]
pub fn status() -> Option<NetworkStatus> {
    None
}

pub fn network_conditions() -> NetworkConditions {
    let flags = policy_native().unwrap_or(0);
    NetworkConditions {
        expensive: (flags & 1 != 0).then_some(flags & 2 != 0),
        constrained: (flags & 4 != 0).then_some(flags & 8 != 0),
    }
}

#[cfg(target_os = "android")]
pub fn status() -> Option<NetworkStatus> {
    // No Context or no ConnectivityManager means no reading at all, which is what `None` is for.
    // The three calls read the same live snapshot; a network that changes between them yields a
    // mixed reading no worse than the one a caller would get a millisecond later.
    let kind = match kind_native().ok()? {
        1 => NetworkKind::Wifi,
        2 => NetworkKind::Cellular,
        3 => NetworkKind::Ethernet,
        4 => NetworkKind::Other,
        _ => NetworkKind::None,
    };
    Some(NetworkStatus {
        online: online_native().unwrap_or(false),
        kind,
        expensive: network_conditions().expensive,
    })
}

day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        /// The system's INTERNET + VALIDATED verdict.
        fn online_native() -> Result<bool, day_bridge::Error>;
        /// 0 none, 1 wifi, 2 cellular, 3 ethernet, 4 other.
        fn kind_native() -> Result<i32, day_bridge::Error>;
        /// Independent known/value bits for metering (1/2) and data saving (4/8).
        fn policy_native() -> Result<i32, day_bridge::Error>;
    }

    #[day_bridge::impl(java, platforms = [android])]
    java!(
        prelude = r#"
            import android.content.Context;
            import android.net.ConnectivityManager;
            import android.net.Network;
            import android.net.NetworkCapabilities;
            import dev.daybrite.day.bridge.DayRuntime;
        "#,
        body = r#"
            /** The active network's capabilities, or null when there is nothing to read. */
            private static NetworkCapabilities caps() {
                Context ctx = DayRuntime.ctx;
                if (ctx == null) return null;
                ConnectivityManager cm =
                        (ConnectivityManager) ctx.getSystemService(Context.CONNECTIVITY_SERVICE);
                if (cm == null) return null;
                Network net = cm.getActiveNetwork();
                if (net == null) return null;
                return cm.getNetworkCapabilities(net);
            }

            public static boolean online_native() {
                NetworkCapabilities caps = caps();
                return caps != null
                        && caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                        && caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED);
            }

            public static int kind_native() {
                NetworkCapabilities caps = caps();
                if (caps == null) return 0;
                if (caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) return 1;
                if (caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)) return 2;
                if (caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)) return 3;
                return 4;
            }

            public static int policy_native() {
                Context ctx = DayRuntime.ctx;
                if (ctx == null) return 0;
                ConnectivityManager cm = (ConnectivityManager) ctx.getSystemService(Context.CONNECTIVITY_SERVICE);
                if (cm == null) return 0;
                Network net = cm.getActiveNetwork();
                if (net == null) return 0;
                NetworkCapabilities caps = cm.getNetworkCapabilities(net);
                if (caps == null) return 0;
                boolean metered = !caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
                    && !(android.os.Build.VERSION.SDK_INT >= 30
                        && caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_TEMPORARILY_NOT_METERED));
                int flags = metered ? 3 : 1;
                if (android.os.Build.VERSION.SDK_INT >= 24) {
                    int restriction = cm.getRestrictBackgroundStatus();
                    if (restriction == ConnectivityManager.RESTRICT_BACKGROUND_STATUS_ENABLED)
                        flags |= metered ? 12 : 4;
                    else if (restriction == ConnectivityManager.RESTRICT_BACKGROUND_STATUS_WHITELISTED
                            || restriction == ConnectivityManager.RESTRICT_BACKGROUND_STATUS_DISABLED)
                        flags |= 4;
                }
                return flags;
            }
        "#,
    );

    #[day_bridge::impl(js, platforms = [web])]
    js!(r#"
        export function policy_native() {
            const saving = globalThis.navigator?.connection?.saveData;
            return typeof saving === "boolean" ? (saving ? 12 : 4) : 0;
        }
        export function kind_native() { return 0; }
        export function online_native() { return false; }
    "#);

    #[day_bridge::impl(rust, platforms = [other])]
    fn online_native() -> Result<bool, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn kind_native() -> Result<i32, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn policy_native() -> Result<i32, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }
}
