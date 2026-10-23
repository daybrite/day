//! One Android bridge owns the combined device service's JVM declarations.
use crate::battery::{BatteryState, BatteryStatus};
use crate::info::DeviceInfo;

pub(crate) fn battery_status() -> Option<BatteryStatus> {
    // Both calls read the same sticky broadcast, so they cannot disagree in any way that matters;
    // a failure on either (no Context yet, no JVM) means no reading at all.
    let level = level_native().ok()?;
    let state = state_native().ok()?;
    Some(BatteryStatus {
        level: (0..=100).contains(&level).then(|| level as f32 / 100.0),
        state: match state {
            1 => BatteryState::Charging,
            2 => BatteryState::Discharging,
            3 => BatteryState::Full,
            4 => BatteryState::NotCharging,
            _ => BatteryState::Unknown,
        },
    })
}

pub(crate) fn device_info() -> DeviceInfo {
    DeviceInfo {
        model: model_native().unwrap_or_else(|_| "Unknown".into()),
        // Not asked of the platform: on Android the answer is Android.
        system_name: "Android".into(),
        system_version: system_version_native().unwrap_or_else(|_| "Unknown".into()),
        is_simulator: is_emulator_native().unwrap_or(false),
    }
}

day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {

        /// 0..=100, or -1 when the level is unknown.
        fn level_native() -> Result<i32, day_bridge::Error>;
        /// 0 unknown, 1 charging, 2 discharging, 3 full, 4 not-charging.
        fn state_native() -> Result<i32, day_bridge::Error>;
        /// `MODEL`, prefixed with `MANUFACTURER` when it does not already start with it.
        fn model_native() -> Result<String, day_bridge::Error>;
        /// `VERSION.RELEASE`, or `"Unknown"`.
        fn system_version_native() -> Result<String, day_bridge::Error>;
        /// Whether this build looks like the AOSP/Google emulator.
        fn is_emulator_native() -> Result<bool, day_bridge::Error>;
    }
    #[day_bridge::impl(java, platforms = [android])]
    java!(
        prelude = r#"
            import android.content.Context;
            import android.content.Intent;
            import android.content.IntentFilter;
            import android.os.BatteryManager;
            import dev.daybrite.day.bridge.DayBridge;
        
            import android.os.Build;
        "#,
        body = r#"
            private static Intent batteryIntent() {
                Context ctx = DayBridge.ctx;
                if (ctx == null) {
                    return null;
                }
                return ctx.registerReceiver(null, new IntentFilter(Intent.ACTION_BATTERY_CHANGED));
            }

            public static int level_native() {
                Intent intent = batteryIntent();
                if (intent == null) {
                    return -1;
                }
                int level = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1);
                int scale = intent.getIntExtra(BatteryManager.EXTRA_SCALE, -1);
                return (level >= 0 && scale > 0) ? Math.round(level * 100f / scale) : -1;
            }

            public static int state_native() {
                Intent intent = batteryIntent();
                if (intent == null) {
                    return 0;
                }
                switch (intent.getIntExtra(BatteryManager.EXTRA_STATUS,
                                           BatteryManager.BATTERY_STATUS_UNKNOWN)) {
                    case BatteryManager.BATTERY_STATUS_CHARGING: return 1;
                    case BatteryManager.BATTERY_STATUS_DISCHARGING: return 2;
                    case BatteryManager.BATTERY_STATUS_FULL: return 3;
                    case BatteryManager.BATTERY_STATUS_NOT_CHARGING: return 4;
                    default: return 0;
                }
            }
        
            public static String model_native() {
                String model = nonEmpty(Build.MODEL);
                String manufacturer = Build.MANUFACTURER;
                if (manufacturer != null && !manufacturer.isEmpty()
                        && !model.toLowerCase().startsWith(manufacturer.toLowerCase())
                        && !model.equals("Unknown")) {
                    return manufacturer + " " + model;
                }
                return model;
            }

            public static String system_version_native() {
                return nonEmpty(Build.VERSION.RELEASE);
            }

            // Heuristic emulator detection from the standard AOSP/Google build fingerprints.
            public static boolean is_emulator_native() {
                String fingerprint = lower(Build.FINGERPRINT);
                String product = lower(Build.PRODUCT);
                String model = lower(Build.MODEL);
                String hardware = lower(Build.HARDWARE);
                return fingerprint.contains("generic")
                        || fingerprint.contains("emulator")
                        || product.contains("sdk")
                        || product.contains("emulator")
                        || model.contains("emulator")
                        || model.contains("android sdk")
                        || hardware.contains("goldfish")
                        || hardware.contains("ranchu");
            }

            private static String nonEmpty(String s) {
                return (s == null || s.isEmpty()) ? "Unknown" : s;
            }

            private static String lower(String s) {
                return s == null ? "" : s.toLowerCase();
            }
        "#,
    );
    #[day_bridge::impl(rust, platforms = [other])]
    fn level_native() -> Result<i32, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn state_native() -> Result<i32, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }
    #[day_bridge::impl(rust, platforms = [other])]
    fn model_native() -> Result<String, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn system_version_native() -> Result<String, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn is_emulator_native() -> Result<bool, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }
}
