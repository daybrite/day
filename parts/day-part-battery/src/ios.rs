// iOS: UIDevice exposes batteryLevel (0.0–1.0, or -1 if unknown) and batteryState once
// batteryMonitoring is enabled. UIDevice is MainThreadOnly, so this reads on the main thread; called
// off it, it returns None. (The Simulator has no battery → level -1, state Unknown.)

use super::{BatteryState, BatteryStatus};
use objc2::MainThreadMarker;
use objc2_ui_kit::{UIDevice, UIDeviceBatteryState};

pub fn status() -> Option<BatteryStatus> {
    let mtm = MainThreadMarker::new()?;
    let device = UIDevice::currentDevice(mtm);
    device.setBatteryMonitoringEnabled(true);
    let raw = device.batteryLevel();
    let level = if raw < 0.0 { None } else { Some(raw) };
    let state = match device.batteryState() {
        UIDeviceBatteryState::Charging => BatteryState::Charging,
        UIDeviceBatteryState::Unplugged => BatteryState::Discharging,
        UIDeviceBatteryState::Full => BatteryState::Full,
        _ => BatteryState::Unknown,
    };
    Some(BatteryStatus { level, state })
}
