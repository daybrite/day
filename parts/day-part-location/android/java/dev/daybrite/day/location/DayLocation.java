// day-part-location's OWN Android backend — a headless capability shim (no UI). Bundled with the
// crate and folded into the app's Gradle build via [package.metadata.day.android], with ZERO edits
// to day-android; it registers no view. The Android twin of parts/day-part-location/src/*.rs's
// other per-OS impls.
//
// WHY LocationManager AND NOT FusedLocationProviderClient. The fused provider is the usual Android
// recommendation, and it is the wrong dependency here: it ships in Google Play services, which AOSP
// images and many emulators do not have, and it would add a Gradle coordinate to every Day app that
// links this part. The platform LocationManager is always present.
//
// PERMISSIONS. This shim never requests one. A missing permission surfaces as a SecurityException,
// which is reported as PermissionDenied; the app asks through day-part-permissions.
package dev.daybrite.day.location;

import android.content.Context;
import android.location.Location;
import android.location.LocationListener;
import android.location.LocationManager;
import android.os.Bundle;
import android.os.Looper;

import dev.daybrite.day.bridge.DayBridge;

public final class DayLocation {
    private DayLocation() {}

    /** Error codes shared with src/android.rs: 1 denied, 2 disabled, 4 unavailable, 5 other. */
    private static final int ERR_DENIED = 1;
    private static final int ERR_DISABLED = 2;
    private static final int ERR_UNAVAILABLE = 4;
    private static final int ERR_OTHER = 5;

    private static final Object lock = new Object();
    private static LocationListener listener;

    /** Implemented in this crate's Rust (src/android.rs). */
    private static native void nativeFix(
        double latitude, double longitude, double altitude, double accuracy,
        double verticalAccuracy, double speed, double course, long timestampMs,
        int hasAltitude, int hasAccuracy, int hasVerticalAccuracy, int hasSpeed, int hasCourse);

    private static native void nativeError(int code);

    private static LocationManager manager() {
        Context ctx = DayBridge.ctx;
        return ctx == null ? null : (LocationManager) ctx.getSystemService(Context.LOCATION_SERVICE);
    }

    public static boolean isAvailable() {
        return manager() != null;
    }

    /** `best` selects the GPS provider where it exists; otherwise the network provider is used. */
    public static void start(boolean best) {
        LocationManager lm = manager();
        if (lm == null) {
            nativeError(ERR_UNAVAILABLE);
            return;
        }
        synchronized (lock) {
            if (listener != null) {
                return;
            }
            listener = new LocationListener() {
                @Override
                public void onLocationChanged(Location l) {
                    report(l);
                }

                // Required on API < 30; harmless above it.
                @Override
                public void onStatusChanged(String provider, int status, Bundle extras) {}

                @Override
                public void onProviderEnabled(String provider) {}

                @Override
                public void onProviderDisabled(String provider) {
                    nativeError(ERR_DISABLED);
                }
            };
            String provider = null;
            if (best && lm.isProviderEnabled(LocationManager.GPS_PROVIDER)) {
                provider = LocationManager.GPS_PROVIDER;
            } else if (lm.isProviderEnabled(LocationManager.NETWORK_PROVIDER)) {
                provider = LocationManager.NETWORK_PROVIDER;
            } else if (lm.isProviderEnabled(LocationManager.GPS_PROVIDER)) {
                provider = LocationManager.GPS_PROVIDER;
            }
            if (provider == null) {
                listener = null;
                nativeError(ERR_DISABLED);
                return;
            }
            try {
                // The listener needs a Looper; DayBridge.main is the UI thread's, which always has
                // one — a Rust-spawned caller thread would not.
                lm.requestLocationUpdates(provider, 1000L, 0f, listener, Looper.getMainLooper());
                // A cached fix makes the first update immediate instead of waiting for the radio.
                Location last = lm.getLastKnownLocation(provider);
                if (last != null) {
                    report(last);
                }
            } catch (SecurityException e) {
                listener = null;
                nativeError(ERR_DENIED);
            } catch (Exception e) {
                listener = null;
                nativeError(ERR_OTHER);
            }
        }
    }

    public static void stop() {
        LocationManager lm = manager();
        synchronized (lock) {
            if (listener != null && lm != null) {
                try {
                    lm.removeUpdates(listener);
                } catch (Exception e) {
                    // Already gone; nothing to undo.
                }
            }
            listener = null;
        }
    }

    /** Android reports "not measured" with hasXxx() rather than a sentinel, so pass both across. */
    private static void report(Location l) {
        nativeFix(
            l.getLatitude(),
            l.getLongitude(),
            l.getAltitude(),
            l.getAccuracy(),
            android.os.Build.VERSION.SDK_INT >= 26 ? l.getVerticalAccuracyMeters() : 0f,
            l.getSpeed(),
            l.getBearing(),
            l.getTime(),
            l.hasAltitude() ? 1 : 0,
            l.hasAccuracy() ? 1 : 0,
            (android.os.Build.VERSION.SDK_INT >= 26 && l.hasVerticalAccuracy()) ? 1 : 0,
            l.hasSpeed() ? 1 : 0,
            l.hasBearing() ? 1 : 0);
    }
}
