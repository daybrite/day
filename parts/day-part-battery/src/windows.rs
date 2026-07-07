// Windows: GetSystemPowerStatus (kernel32) fills a SYSTEM_POWER_STATUS. Raw FFI — no dependencies.
// Written blind (no Windows host); compiled only on the windows target.

use super::{BatteryState, BatteryStatus};
use std::os::raw::c_int;

#[repr(C)]
struct SystemPowerStatus {
    ac_line_status: u8,       // 0 offline, 1 online, 255 unknown
    battery_flag: u8,         // bit 3 (8) = charging; 128 = no system battery
    battery_life_percent: u8, // 0..100, or 255 unknown
    system_status_flag: u8,
    battery_life_time: u32,
    battery_full_life_time: u32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetSystemPowerStatus(status: *mut SystemPowerStatus) -> c_int;
}

pub fn status() -> Option<BatteryStatus> {
    let mut sps = SystemPowerStatus {
        ac_line_status: 255,
        battery_flag: 255,
        battery_life_percent: 255,
        system_status_flag: 0,
        battery_life_time: 0,
        battery_full_life_time: 0,
    };
    if unsafe { GetSystemPowerStatus(&mut sps) } == 0 {
        return None;
    }
    if sps.battery_flag == 128 {
        return None; // no system battery
    }
    let level = if sps.battery_life_percent <= 100 {
        Some(sps.battery_life_percent as f32 / 100.0)
    } else {
        None
    };
    let charging = sps.battery_flag != 255 && sps.battery_flag & 8 != 0;
    let state = if charging {
        BatteryState::Charging
    } else if sps.ac_line_status == 1 {
        if sps.battery_life_percent >= 100 {
            BatteryState::Full
        } else {
            BatteryState::NotCharging
        }
    } else if sps.ac_line_status == 0 {
        BatteryState::Discharging
    } else {
        BatteryState::Unknown
    };
    Some(BatteryStatus { level, state })
}
