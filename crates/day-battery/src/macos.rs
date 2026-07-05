// macOS: IOKit's IOPowerSources API. IOPSCopyPowerSourcesInfo() returns an opaque blob;
// IOPSCopyPowerSourcesList(blob) is an array of power sources; IOPSGetPowerSourceDescription(blob, ps)
// is a CFDictionary with the keys we read ("Current Capacity", "Max Capacity", "Is Charging",
// "Power Source State"). Raw core-foundation-sys for the dictionary access; the IOKit framework is
// force-linked below.

use super::{BatteryState, BatteryStatus};
use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation_sys::base::{CFRelease, CFTypeRef};
use core_foundation_sys::dictionary::{CFDictionaryGetValueIfPresent, CFDictionaryRef};
use core_foundation_sys::number::{CFNumberGetValue, CFNumberRef, kCFNumberSInt64Type};
use core_foundation_sys::string::{CFStringGetCString, CFStringRef, kCFStringEncodingUTF8};
use std::os::raw::{c_char, c_void};

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
    fn IOPSCopyPowerSourcesList(blob: CFTypeRef) -> CFArrayRef;
    fn IOPSGetPowerSourceDescription(blob: CFTypeRef, ps: CFTypeRef) -> CFDictionaryRef;
}

/// Look up `key` in `dict`, returning the borrowed value (Get-rule) if present.
unsafe fn value(dict: CFDictionaryRef, key: &str) -> Option<CFTypeRef> {
    let k = CFString::new(key);
    let mut out: *const c_void = std::ptr::null();
    let present = unsafe {
        CFDictionaryGetValueIfPresent(dict, k.as_concrete_TypeRef() as *const c_void, &mut out)
    };
    (present != 0 && !out.is_null()).then_some(out as CFTypeRef)
}

unsafe fn i64_value(dict: CFDictionaryRef, key: &str) -> Option<i64> {
    let v = unsafe { value(dict, key)? };
    let mut n: i64 = 0;
    let ok = unsafe {
        CFNumberGetValue(
            v as CFNumberRef,
            kCFNumberSInt64Type,
            &mut n as *mut i64 as *mut c_void,
        )
    };
    ok.then_some(n)
}

unsafe fn string_value(dict: CFDictionaryRef, key: &str) -> Option<String> {
    let v = unsafe { value(dict, key)? };
    let mut buf = [0i8; 128];
    let ok = unsafe {
        CFStringGetCString(
            v as CFStringRef,
            buf.as_mut_ptr() as *mut c_char,
            buf.len() as isize,
            kCFStringEncodingUTF8,
        )
    };
    if ok == 0 {
        return None;
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr() as *const c_char) };
    Some(cstr.to_string_lossy().into_owned())
}

pub fn status() -> Option<BatteryStatus> {
    unsafe {
        let blob = IOPSCopyPowerSourcesInfo();
        if blob.is_null() {
            return None;
        }
        let result = (|| {
            let list = IOPSCopyPowerSourcesList(blob);
            if list.is_null() {
                return None;
            }
            let count = CFArrayGetCount(list);
            let mut out = None;
            for i in 0..count {
                let ps = CFArrayGetValueAtIndex(list, i) as CFTypeRef;
                let desc = IOPSGetPowerSourceDescription(blob, ps);
                if desc.is_null() {
                    continue;
                }
                let cur = i64_value(desc, "Current Capacity");
                let max = i64_value(desc, "Max Capacity");
                let level = match (cur, max) {
                    (Some(c), Some(m)) if m > 0 => Some((c as f32 / m as f32).clamp(0.0, 1.0)),
                    _ => None,
                };
                let charging = i64_value(desc, "Is Charging").map(|b| b != 0);
                let ps_state = string_value(desc, "Power Source State").unwrap_or_default();
                let state = match charging {
                    Some(true) => BatteryState::Charging,
                    Some(false) if ps_state == "AC Power" => match (cur, max) {
                        (Some(c), Some(m)) if c >= m => BatteryState::Full,
                        _ => BatteryState::NotCharging,
                    },
                    Some(false) => BatteryState::Discharging,
                    None => BatteryState::Unknown,
                };
                out = Some(BatteryStatus { level, state });
                break;
            }
            CFRelease(list as CFTypeRef);
            out
        })();
        CFRelease(blob);
        result
    }
}
