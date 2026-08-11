// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Android, whole: the Java that reads ConnectivityManager, the declaration that binds it, and the
// mapping into [`NetworkStatus`]. Nothing about this platform appears anywhere else in the crate.
//
// ConnectivityManager needs a `Context` and has no C entry point, so it is this crate's only
// foreign arm (docs/bridge.md). Written in Java rather than Kotlin so it compiles in any Android
// project. The ACCESS_NETWORK_STATE permission stays a build-graph fact in Cargo.toml.
//
// Before daybridge the snapshot crossed as ONE packed `long` — `(online << 16) | (kind << 8) |
// expensiveByte`, with -1 and 255 sentinels — written in Java and unpacked in Rust. Three
// declarations replace it, and every sentinel with it.

use super::{NetworkKind, NetworkStatus};

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
        expensive: match expensive_native() {
            Ok(0) => Some(false),
            Ok(1) => Some(true),
            _ => None, // unknown: no active network, or capabilities unreadable
        },
    })
}

day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        /// The system's INTERNET + VALIDATED verdict.
        fn online_native() -> Result<bool, day_bridge::Error>;
        /// 0 none, 1 wifi, 2 cellular, 3 ethernet, 4 other.
        fn kind_native() -> Result<i32, day_bridge::Error>;
        /// 0 not metered, 1 metered, -1 unknown.
        fn expensive_native() -> Result<i32, day_bridge::Error>;
    }

    #[day_bridge::impl(java, platforms = [android])]
    java!(
        prelude = r#"
            import android.content.Context;
            import android.net.ConnectivityManager;
            import android.net.Network;
            import android.net.NetworkCapabilities;
            import dev.daybrite.day.bridge.DayBridge;
        "#,
        body = r#"
            /** The active network's capabilities, or null when there is nothing to read. */
            private static NetworkCapabilities caps() {
                Context ctx = DayBridge.ctx;
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

            public static int expensive_native() {
                NetworkCapabilities caps = caps();
                if (caps == null) return -1;
                return caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED) ? 0 : 1;
            }
        "#,
    );

    // The fallback every bridge declares. This file is `#[cfg(target_os = "android")]`, so it is
    // never compiled — it satisfies the rule that a bridge always has an answer for an unclaimed
    // target.
    #[day_bridge::impl(rust, platforms = [other])]
    fn online_native() -> Result<bool, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn kind_native() -> Result<i32, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn expensive_native() -> Result<i32, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }
}
