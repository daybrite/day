// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-part-permissions' OWN Android backend — a headless capability shim (no UI). It is bundled with
// this crate and folded into the app's Gradle build via [package.metadata.day.android], with ZERO
// edits to day-android; it registers no view. It is the Android twin of
// parts/day-part-permissions/src/*.rs's other per-OS impls.
//
// WHY A FRAGMENT. Runtime permission results arrive at Activity.onRequestPermissionsResult, and
// day-android's DayActivity overrides only onActivityResult (hardcoded to the file picker). Rather
// than edit a core day crate — which every part's Cargo.toml promises not to do — this shim attaches
// its own headless Fragment and uses registerForActivityResult(RequestMultiplePermissions). Results
// route to that fragment alone, so there is no request-code space to partition, and this is the
// standard Android idiom (RxPermissions, Dexter and PermissionsDispatcher all do the same).
// androidx.fragment is already on the classpath because DayActivity extends FragmentActivity.
//
// NO REMEMBERED STATE. This shim deliberately persists nothing. Android cannot distinguish
// "never asked" from "permanently denied" without app-side state, and Day does not keep any — see
// classify_android() in src/lib.rs and docs/permissions.md for what that means and how an app that
// needs the distinction records it itself.
package dev.daybrite.day.permissions;

import android.app.Activity;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.Settings;

import androidx.activity.result.ActivityResultLauncher;
import androidx.activity.result.contract.ActivityResultContracts;
import androidx.fragment.app.Fragment;
import androidx.fragment.app.FragmentActivity;
import androidx.fragment.app.FragmentManager;

import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

import dev.daybrite.day.bridge.DayBridge;

public final class DayPermissions {
    private DayPermissions() {}

    /** The fragment's tag in the host FragmentManager; also how it is re-found after a rotation. */
    static final String TAG = "dev.daybrite.day.permissions";

    /**
     * Permission lists cross the JNI boundary as ONE string joined by U+001F, the same flattening
     * day_spec uses for the C ABI — it keeps this shim free of jobjectArray plumbing on both sides.
     */
    static final String SEP = "\u001f";

    /** token → the permissions that token asked for, so the answer can be reported positionally. */
    static final Map<Long, String[]> pending = new ConcurrentHashMap<>();

    /**
     * Implemented in this crate's Rust (src/android.rs). `grantedMask` bit i is set when perms[i]
     * was granted; Rust already knows the list it asked for, so only the bits have to travel.
     */
    private static native void nativeResult(long token, long grantedMask);

    // --- synchronous probes (safe from any thread) --------------------------

    /** 1 when the permission is currently granted, 0 otherwise. */
    public static int check(String perm) {
        Context ctx = DayBridge.ctx;
        if (ctx == null) {
            return 0;
        }
        return ctx.checkSelfPermission(perm) == PackageManager.PERMISSION_GRANTED ? 1 : 0;
    }

    /**
     * Whether the permission survived into the app's MERGED manifest. Without it a request is
     * denied in the same frame with no dialog, and Settings offers nothing — which is what
     * Status::Restricted means.
     */
    public static boolean isDeclared(String perm) {
        Context ctx = DayBridge.ctx;
        if (ctx == null) {
            return false;
        }
        try {
            PackageInfo info = ctx.getPackageManager()
                .getPackageInfo(ctx.getPackageName(), PackageManager.GET_PERMISSIONS);
            if (info.requestedPermissions == null) {
                return false;
            }
            for (String p : info.requestedPermissions) {
                if (p.equals(perm)) {
                    return true;
                }
            }
        } catch (Exception e) {
            // A package manager that cannot answer is not evidence of absence; say "declared" so
            // the app still gets a prompt rather than a spurious Restricted.
            return true;
        }
        return false;
    }

    /** The platform's "you should explain before asking again" signal. */
    public static boolean shouldShowRationale(String perm) {
        Context ctx = DayBridge.ctx;
        if (!(ctx instanceof Activity)) {
            return false;
        }
        return ((Activity) ctx).shouldShowRequestPermissionRationale(perm);
    }

    public static int sdkInt() {
        return Build.VERSION.SDK_INT;
    }

    /** Below API 33 there is no POST_NOTIFICATIONS permission — this is the whole answer. */
    public static boolean notificationsEnabled() {
        Context ctx = DayBridge.ctx;
        if (ctx == null) {
            return false;
        }
        android.app.NotificationManager nm =
            (android.app.NotificationManager) ctx.getSystemService(Context.NOTIFICATION_SERVICE);
        return nm != null && nm.areNotificationsEnabled();
    }

    /** Open this app's page in Settings, the only remedy once a permission is permanently denied. */
    public static boolean openSettings() {
        Context ctx = DayBridge.ctx;
        if (ctx == null) {
            return false;
        }
        try {
            Intent intent = new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                Uri.parse("package:" + ctx.getPackageName()));
            // NEW_TASK is required because ctx may be the application context.
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            ctx.startActivity(intent);
            return true;
        } catch (Exception e) {
            return false;
        }
    }

    // --- the asynchronous request ------------------------------------------

    /**
     * Ask for {@code perms}. Callable from ANY thread: the work hops to the UI thread, attaches (or
     * reuses) the headless fragment, and launches. The answer comes back through nativeResult.
     *
     * <p>When there is no Activity to host the dialog — a headless process, a Service, a test — the
     * current grant state is reported immediately rather than hanging: a future must always resolve.
     */
    public static void request(long token, String permsJoined) {
        String[] perms = permsJoined.isEmpty() ? new String[0] : permsJoined.split(SEP, -1);
        pending.put(token, perms);
        Context ctx = DayBridge.ctx;
        if (!(ctx instanceof FragmentActivity)) {
            deliverCurrent(token, perms);
            return;
        }
        final FragmentActivity activity = (FragmentActivity) ctx;
        DayBridge.main.post(new Runnable() {
            @Override
            public void run() {
                try {
                    FragmentManager fm = activity.getSupportFragmentManager();
                    DayPermissionsFragment frag = (DayPermissionsFragment) fm.findFragmentByTag(TAG);
                    if (frag == null) {
                        frag = new DayPermissionsFragment();
                        // commitNow() runs onCreate synchronously, so the launcher exists below.
                        fm.beginTransaction().add(frag, TAG).commitNow();
                    }
                    frag.launch(token, perms);
                } catch (Exception e) {
                    deliverCurrent(token, perms);
                }
            }
        });
    }

    /** Report whatever the current grant state is — the no-Activity and failure path. */
    static void deliverCurrent(long token, String[] perms) {
        long mask = 0;
        for (int i = 0; i < perms.length && i < 64; i++) {
            if (check(perms[i]) == 1) {
                mask |= 1L << i;
            }
        }
        deliver(token, mask);
    }

    static void deliver(long token, long grantedMask) {
        if (pending.remove(token) == null) {
            return; // already answered (a duplicate callback after a rotation, say)
        }
        nativeResult(token, grantedMask);
    }

    /**
     * The token whose prompt is on screen, or 0. STATIC on purpose: a rotation destroys and
     * recreates this fragment (see the class note), so an instance field would lose the
     * correlation exactly when it is needed. Android serializes permission dialogs, so one slot is
     * enough.
     */
    static volatile long inFlight;

    /**
     * A headless fragment: no view, no UI, just the ActivityResult launcher that owns the permission
     * dialog's answer.
     *
     * <p>Deliberately NOT {@code setRetainInstance(true)}: a retained fragment skips {@code onCreate}
     * after a configuration change, which would leave {@code launcher} registered against the
     * destroyed activity's result registry and the answer would never arrive. Letting the fragment
     * be recreated re-registers the launcher with the new activity, and the ActivityResult API
     * delivers the pending result to it — which is precisely what that API exists to do. The token
     * survives in the static fields above.
     */
    public static final class DayPermissionsFragment extends Fragment {
        private ActivityResultLauncher<String[]> launcher;

        @Override
        public void onCreate(Bundle savedInstanceState) {
            super.onCreate(savedInstanceState);
            launcher = registerForActivityResult(
                new ActivityResultContracts.RequestMultiplePermissions(),
                result -> {
                    long token = inFlight;
                    inFlight = 0;
                    String[] perms = pending.get(token);
                    if (perms == null) {
                        return;
                    }
                    long mask = 0;
                    for (int i = 0; i < perms.length && i < 64; i++) {
                        Boolean granted = result.get(perms[i]);
                        // A permission the launcher didn't report is one the system refused to ask
                        // about; fall back to its current state rather than inventing a denial.
                        boolean ok = granted != null ? granted : check(perms[i]) == 1;
                        if (ok) {
                            mask |= 1L << i;
                        }
                    }
                    deliver(token, mask);
                });
        }

        void launch(long token, String[] perms) {
            if (launcher == null) {
                deliverCurrent(token, perms);
                return;
            }
            inFlight = token;
            try {
                launcher.launch(perms);
            } catch (Exception e) {
                inFlight = 0;
                deliverCurrent(token, perms);
            }
        }
    }
}
