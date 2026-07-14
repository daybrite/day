// Android: BatteryManager, read via this crate's OWN Java shim (android/java/…/DayBattery.java) —
// staged into the app's Gradle build by `day build` through [package.metadata.day.android], exactly
// like the UI pieces, but registering NO renderer. The Java uses day-android's cached Context
// (DayBridge.ctx); Rust calls it through day-android's re-exported `jni`. So on Android this headless
// crate rides on the Day runtime (it needs the app's JVM + Context).

use super::{BatteryState, BatteryStatus};
use day_android::with_env;
use day_android::DayEnv;

const BATTERY_CLASS: &str = "dev/daybrite/day/battery/DayBattery";

pub fn status() -> Option<BatteryStatus> {
    // `read()` packs the reading into a long: (state << 8) | levelByte (255 = unknown level).
    let packed: i64 = with_env(|env| {
        env.dcall_static(BATTERY_CLASS, "read", "()J", &[])
            .ok()
            .and_then(|v| v.j().ok())
    })?;

    let level_byte = (packed & 0xFF) as u8;
    let level = if level_byte == 255 {
        None
    } else {
        Some(level_byte as f32 / 100.0)
    };
    let state = match (packed >> 8) & 0xFF {
        1 => BatteryState::Charging,
        2 => BatteryState::Discharging,
        3 => BatteryState::Full,
        4 => BatteryState::NotCharging,
        _ => BatteryState::Unknown,
    };
    Some(BatteryStatus { level, state })
}
