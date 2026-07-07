// Linux: the kernel exposes power supplies under /sys/class/power_supply/<name>/ with `type`,
// `capacity` (0–100) and `status` files. Pure std — no dependencies.

use super::{BatteryState, BatteryStatus};
use std::fs;
use std::path::Path;

pub fn status() -> Option<BatteryStatus> {
    let base = Path::new("/sys/class/power_supply");
    for entry in fs::read_dir(base).ok()?.flatten() {
        let dir = entry.path();
        if fs::read_to_string(dir.join("type"))
            .unwrap_or_default()
            .trim()
            != "Battery"
        {
            continue;
        }
        let level = fs::read_to_string(dir.join("capacity"))
            .ok()
            .and_then(|s| s.trim().parse::<f32>().ok())
            .map(|c| (c / 100.0).clamp(0.0, 1.0));
        let state = match fs::read_to_string(dir.join("status"))
            .unwrap_or_default()
            .trim()
        {
            "Charging" => BatteryState::Charging,
            "Discharging" => BatteryState::Discharging,
            "Full" => BatteryState::Full,
            "Not charging" => BatteryState::NotCharging,
            _ => BatteryState::Unknown,
        };
        return Some(BatteryStatus { level, state });
    }
    None
}
