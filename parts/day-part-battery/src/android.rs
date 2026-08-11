// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Android, whole: the Java that reads BatteryManager, the declaration that binds it, and the
// mapping into `BatteryStatus`. Nothing about this platform appears anywhere else in the crate.
//
// Android is the one target whose battery API cannot be reached from Rust — `BatteryManager` needs
// a `Context` and the sticky ACTION_BATTERY_CHANGED broadcast, with no C entry point — so it is
// this crate's only foreign arm (docs/bridge.md). The other five arms stay Rust: IOKit,
// GetSystemPowerStatus, sysfs and libohbattery_info are C APIs, and a bridge there would add a
// toolchain and buy nothing.
//
// Before daybridge this file did the JNI call by hand against a checked-in `DayBattery.java`, and
// the two halves agreed on a packed `i64` — `(state << 8) | levelByte`, 255 meaning unknown —
// written twice and kept in agreement by comment. Both halves now come from one declaration, so
// the packing is gone. (A `#[day_bridge::data]` struct would collapse the two calls back into one;
// POD struct returns are after v1.)

use super::{BatteryState, BatteryStatus};

pub fn status() -> Option<BatteryStatus> {
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

day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        /// 0..=100, or -1 when the level is unknown.
        fn level_native() -> Result<i32, day_bridge::Error>;
        /// 0 unknown, 1 charging, 2 discharging, 3 full, 4 not-charging.
        fn state_native() -> Result<i32, day_bridge::Error>;
    }

    // Java, not Kotlin: a `.java` arm compiles in any Android project, while a `.kt` arm needs the
    // project to have the Kotlin plugin (docs/bridge.md). A part published for other people's apps
    // has no say in that, so it uses the language every app can already build.
    #[day_bridge::impl(java, platforms = [android])]
    java!(
        prelude = r#"
            import android.content.Context;
            import android.content.Intent;
            import android.content.IntentFilter;
            import android.os.BatteryManager;
            import dev.daybrite.day.bridge.DayBridge;
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
        "#,
    );

    // The fallback every bridge declares. This file is `#[cfg(target_os = "android")]`, so these
    // are never actually compiled — they satisfy the rule that a bridge always has an answer for
    // an unclaimed target, which matters when the block sits in a file every target compiles.
    #[day_bridge::impl(rust, platforms = [other])]
    fn level_native() -> Result<i32, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn state_native() -> Result<i32, day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }
}
