// HarmonyOS / OpenHarmony: the native device-info C API (`deviceinfo.h`, `libdeviceinfo_ndk.so`).
// Pure FFI, like macOS/iOS — no ArkTS bridge or Day runtime needed. The getters return borrowed,
// static `const char *` (never freed, so we only copy them). `OH_GetOSFullName()` is e.g.
// "OpenHarmony-5.0.0.0" — its head is the OS name; `OH_GetDisplayVersion()` is the user-facing version
// and `OH_GetProductModel()` the model. Reading device info needs no permission. There is no
// simulator concept exposed here, so is_simulator is false.

use super::DeviceInfo;
use std::ffi::{CStr, c_char};

#[link(name = "deviceinfo_ndk.z")]
unsafe extern "C" {
    fn OH_GetOSFullName() -> *const c_char;
    fn OH_GetDisplayVersion() -> *const c_char;
    fn OH_GetProductModel() -> *const c_char;
}

/// Copy a borrowed C string into an owned `String`, returning `None` for null or empty.
fn owned(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    (!s.is_empty()).then_some(s)
}

pub fn get() -> DeviceInfo {
    // "OpenHarmony-5.0.0.0" → name "OpenHarmony", version fallback "5.0.0.0".
    let full = owned(unsafe { OH_GetOSFullName() });
    let (system_name, version_from_full) = match full {
        Some(f) => match f.split_once('-') {
            Some((name, ver)) => (name.to_string(), Some(ver.to_string())),
            None => (f, None),
        },
        None => ("OpenHarmony".to_string(), None),
    };
    let system_version = owned(unsafe { OH_GetDisplayVersion() })
        .or(version_from_full)
        .unwrap_or_else(|| "Unknown".to_string());
    let model = owned(unsafe { OH_GetProductModel() }).unwrap_or_else(|| "Unknown".to_string());

    DeviceInfo {
        model,
        system_name,
        system_version,
        is_simulator: false,
    }
}
