// iOS: UIDevice.currentDevice exposes model ("iPhone"/"iPad" marketing class), systemName
// ("iOS"/"iPadOS") and systemVersion ("17.5"). UIDevice is MainThreadOnly, so this reads on the main
// thread; called off it, the OS fields fall back to "Unknown" (the simulator flag is still reported,
// as it does not need UIDevice). Simulator detection: the `sim` target ABI is definitive; a device
// build additionally honours the SIMULATOR_* environment the simulator injects.

use super::DeviceInfo;
use objc2::MainThreadMarker;
use objc2_ui_kit::UIDevice;

/// Whether this process is running in the iOS Simulator. The `aarch64-apple-ios-sim` /
/// `x86_64-apple-ios` targets set `target_abi = "sim"`; as a belt-and-braces fallback for device
/// builds launched by the simulator harness, the presence of the simulator's env vars also counts.
fn is_simulator() -> bool {
    cfg!(target_abi = "sim")
        || std::env::var_os("SIMULATOR_UDID").is_some()
        || std::env::var_os("SIMULATOR_DEVICE_NAME").is_some()
}

pub fn get() -> DeviceInfo {
    let simulator = is_simulator();
    match MainThreadMarker::new() {
        Some(mtm) => {
            let device = UIDevice::currentDevice(mtm);
            DeviceInfo {
                model: device.model().to_string(),
                system_name: device.systemName().to_string(),
                system_version: device.systemVersion().to_string(),
                is_simulator: simulator,
            }
        }
        // Off the main thread UIDevice is unavailable; report what we can without it.
        None => DeviceInfo {
            model: "Unknown".to_string(),
            system_name: "iOS".to_string(),
            system_version: "Unknown".to_string(),
            is_simulator: simulator,
        },
    }
}
