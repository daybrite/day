// Windows: RtlGetVersion (ntdll) fills an OSVERSIONINFOW with the REAL running version — unlike the
// Win32 GetVersionExW, which lies (reports 6.2 for Windows 8+) unless the app ships a compatibility
// manifest. RtlGetVersion is the documented driver-facing escape hatch and honours no manifest, so it
// is the reliable choice here. Raw FFI — no dependencies. Written blind (no Windows host); compiled
// only on the windows target. There is no simulator concept on desktop Windows.

use super::DeviceInfo;
use std::os::raw::c_ulong;

/// RTL_OSVERSIONINFOW (winnt.h). `szCSDVersion` is a fixed 128-wide-char service-pack string; we do
/// not read it, but the layout must match for RtlGetVersion to fill the numeric fields.
#[repr(C)]
struct OsVersionInfoW {
    dw_os_version_info_size: c_ulong,
    dw_major_version: c_ulong,
    dw_minor_version: c_ulong,
    dw_build_number: c_ulong,
    dw_platform_id: c_ulong,
    sz_csd_version: [u16; 128],
}

#[link(name = "ntdll")]
unsafe extern "system" {
    // NTSTATUS RtlGetVersion(PRTL_OSVERSIONINFOW) — returns STATUS_SUCCESS (0).
    fn RtlGetVersion(info: *mut OsVersionInfoW) -> i32;
}

pub fn get() -> DeviceInfo {
    let mut info = OsVersionInfoW {
        dw_os_version_info_size: std::mem::size_of::<OsVersionInfoW>() as c_ulong,
        dw_major_version: 0,
        dw_minor_version: 0,
        dw_build_number: 0,
        dw_platform_id: 0,
        sz_csd_version: [0; 128],
    };
    let system_version = if unsafe { RtlGetVersion(&mut info) } == 0 {
        format!(
            "{}.{}.{}",
            info.dw_major_version, info.dw_minor_version, info.dw_build_number
        )
    } else {
        "Unknown".to_string()
    };
    DeviceInfo {
        model: "PC".to_string(),
        system_name: "Windows".to_string(),
        system_version,
        is_simulator: false,
    }
}
