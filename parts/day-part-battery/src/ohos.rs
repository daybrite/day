// HarmonyOS / OpenHarmony: the native BasicServicesKit battery C API (`libohbattery_info.so`,
// `ohbattery_info.h`, API 13+). Pure FFI, like macOS/iOS — no ArkTS bridge or Day runtime needed
// (unlike Android's BatteryManager, which rides day-android's JVM/Context). Reading battery needs
// no permission. The native API exposes capacity + plugged type but no explicit charge state, so
// the state is derived from the plugged type (charging when on external power).

use core::ffi::c_int;

use super::{BatteryState, BatteryStatus};

// BatteryInfo_BatteryPluggedType (ohbattery_info.h): 0 = none (on battery), 1 = AC, 2 = USB,
// 3 = wireless, 4 = unknown.
const PLUGGED_TYPE_NONE: c_int = 0;

#[link(name = "ohbattery_info")]
unsafe extern "C" {
    fn OH_BatteryInfo_GetCapacity() -> c_int;
    fn OH_BatteryInfo_GetPluggedType() -> c_int;
}

pub fn status() -> Option<BatteryStatus> {
    let capacity = unsafe { OH_BatteryInfo_GetCapacity() };
    let plugged = unsafe { OH_BatteryInfo_GetPluggedType() };

    let level = if (0..=100).contains(&capacity) {
        Some(capacity as f32 / 100.0)
    } else {
        None
    };
    // No native charge-state getter; infer it from whether a power source is plugged in.
    let state = if plugged == PLUGGED_TYPE_NONE {
        BatteryState::Discharging
    } else if capacity >= 100 {
        BatteryState::Full
    } else {
        BatteryState::Charging
    };
    Some(BatteryStatus { level, state })
}
