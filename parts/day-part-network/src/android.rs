// Android: ConnectivityManager, read via this crate's OWN Java shim (android/java/…/DayNetwork.java) —
// staged into the app's Gradle build by `day build` through [package.metadata.day.android], exactly
// like the UI pieces, but registering NO renderer (and contributing the ACCESS_NETWORK_STATE manifest
// permission through the same overlay). The Java uses day-android's cached Context (DayBridge.ctx);
// Rust calls it through day-android's re-exported `jni`. So on Android this headless crate rides on
// the Day runtime (it needs the app's JVM + Context).

use super::{NetworkKind, NetworkStatus};
use day_android::DayEnv;
use day_android::with_env;

const NETWORK_CLASS: &str = "dev/daybrite/day/network/DayNetwork";

pub fn status() -> Option<NetworkStatus> {
    // `read()` packs the snapshot into a long: (online << 16) | (kind << 8) | expensiveByte,
    // or -1 when unavailable (no Context / no ConnectivityManager).
    let packed: i64 = with_env(|env| {
        env.dcall_static(NETWORK_CLASS, "read", "()J", &[])
            .ok()
            .and_then(|v| v.j().ok())
    })?;
    if packed < 0 {
        return None;
    }

    let online = (packed >> 16) & 0xFF != 0;
    let kind = match (packed >> 8) & 0xFF {
        1 => NetworkKind::Wifi,
        2 => NetworkKind::Cellular,
        3 => NetworkKind::Ethernet,
        4 => NetworkKind::Other,
        _ => NetworkKind::None,
    };
    let expensive = match packed & 0xFF {
        0 => Some(false),
        1 => Some(true),
        _ => None, // 255 = unknown
    };
    Some(NetworkStatus {
        online,
        kind,
        expensive,
    })
}
