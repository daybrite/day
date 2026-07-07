// day-part-network's OWN Android backend — a headless capability shim (no UI). It is bundled with this
// crate and folded into the app's Gradle build via [package.metadata.day.android], with ZERO edits to
// day-android; it registers no view, and the same overlay merges android.permission.ACCESS_NETWORK_STATE
// into the app manifest. It reads ConnectivityManager's active network + NetworkCapabilities using
// day-android's public Context (DayBridge.ctx). It is the Android twin of
// parts/day-part-network/src/*.rs's other per-OS impls.
package dev.daybrite.day.network;

import android.content.Context;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;

import dev.daybrite.day.bridge.DayBridge;

public final class DayNetwork {
    private DayNetwork() {}

    /**
     * Packs the connectivity snapshot into a long: (online << 16) | (kind << 8) | expensiveByte.
     * online is 0/1 (the system's INTERNET + VALIDATED verdict). kind: 0=none, 1=wifi, 2=cellular,
     * 3=ethernet, 4=other. expensiveByte: 0=not metered, 1=metered, 255=unknown.
     * Returns -1 when the snapshot cannot be read at all (no Context / no ConnectivityManager).
     */
    public static long read() {
        Context ctx = DayBridge.ctx;
        if (ctx == null) {
            return -1L;
        }
        ConnectivityManager cm =
                (ConnectivityManager) ctx.getSystemService(Context.CONNECTIVITY_SERVICE);
        if (cm == null) {
            return -1L;
        }
        Network net = cm.getActiveNetwork();
        if (net == null) {
            return 255L; // offline: online=0, kind=none, expensive unknown
        }
        NetworkCapabilities caps = cm.getNetworkCapabilities(net);
        if (caps == null) {
            // An active network exists but its capabilities are (transiently) unreadable.
            return (1L << 16) | (4L << 8) | 255L;
        }
        long online = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                && caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED) ? 1L : 0L;
        long kind;
        if (caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) {
            kind = 1;
        } else if (caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)) {
            kind = 2;
        } else if (caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)) {
            kind = 3;
        } else {
            kind = 4;
        }
        long expensive =
                caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED) ? 0L : 1L;
        return (online << 16) | (kind << 8) | expensive;
    }
}
