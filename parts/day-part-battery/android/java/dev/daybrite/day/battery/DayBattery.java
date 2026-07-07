// day-part-battery's OWN Android backend — a headless capability shim (no UI). It is bundled with this
// crate and folded into the app's Gradle build via [package.metadata.day.android], with ZERO edits to
// day-android; it registers no view. It reads the sticky ACTION_BATTERY_CHANGED broadcast (no
// permission needed) using day-android's public Context (DayBridge.ctx). It is the Android twin of
// parts/day-part-battery/src/*.rs's other per-OS impls.
package dev.daybrite.day.battery;

import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.os.BatteryManager;

import dev.daybrite.day.bridge.DayBridge;

public final class DayBattery {
    private DayBattery() {}

    /**
     * Packs the reading into a long: (state << 8) | levelByte.
     * levelByte is 0..100, or 255 when unknown. state: 0=unknown, 1=charging, 2=discharging,
     * 3=full, 4=not-charging.
     */
    public static long read() {
        Context ctx = DayBridge.ctx;
        int level = -1;
        int state = 0;
        if (ctx != null) {
            IntentFilter filter = new IntentFilter(Intent.ACTION_BATTERY_CHANGED);
            Intent intent = ctx.registerReceiver(null, filter);
            if (intent != null) {
                int lvl = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1);
                int scale = intent.getIntExtra(BatteryManager.EXTRA_SCALE, -1);
                if (lvl >= 0 && scale > 0) {
                    level = Math.round(lvl * 100f / scale);
                }
                switch (intent.getIntExtra(BatteryManager.EXTRA_STATUS,
                        BatteryManager.BATTERY_STATUS_UNKNOWN)) {
                    case BatteryManager.BATTERY_STATUS_CHARGING:
                        state = 1;
                        break;
                    case BatteryManager.BATTERY_STATUS_DISCHARGING:
                        state = 2;
                        break;
                    case BatteryManager.BATTERY_STATUS_FULL:
                        state = 3;
                        break;
                    case BatteryManager.BATTERY_STATUS_NOT_CHARGING:
                        state = 4;
                        break;
                    default:
                        state = 0;
                }
            }
        }
        long levelByte = (level < 0) ? 255 : Math.min(100, level);
        return ((long) state << 8) | levelByte;
    }
}
