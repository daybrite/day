// macOS: the OS version comes from Foundation's ProcessInfo.operatingSystemVersion (an
// NSOperatingSystemVersion struct of major/minor/patch — the honest running version, unlike the
// deprecated Gestalt/sw_vers paths), read through objc2-foundation. The hardware model identifier
// (e.g. "MacBookPro18,3", "Macmini9,1") comes from the BSD `sysctl` node "hw.model" via libc.
// There is no simulator concept on macOS, so is_simulator is always false.

use super::DeviceInfo;
use objc2_foundation::NSProcessInfo;
use std::ffi::{CString, c_void};

/// Read a NUL-terminated string `sysctl` node by name (e.g. "hw.model"). Two-call idiom: size, then
/// value. Returns `None` when the node is missing or empty.
fn sysctl_string(name: &str) -> Option<String> {
    let cname = CString::new(name).ok()?;
    let mut size: libc::size_t = 0;
    // First call: query the required buffer size.
    let rc = unsafe {
        libc::sysctlbyname(
            cname.as_ptr(),
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size];
    // Second call: fill the buffer.
    let rc = unsafe {
        libc::sysctlbyname(
            cname.as_ptr(),
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    // The value is NUL-terminated; trim at the first NUL before decoding.
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let s = String::from_utf8_lossy(&buf[..end]).into_owned();
    (!s.is_empty()).then_some(s)
}

pub fn get() -> DeviceInfo {
    let v = NSProcessInfo::processInfo().operatingSystemVersion();
    let system_version = if v.patchVersion != 0 {
        format!("{}.{}.{}", v.majorVersion, v.minorVersion, v.patchVersion)
    } else {
        format!("{}.{}", v.majorVersion, v.minorVersion)
    };
    DeviceInfo {
        model: sysctl_string("hw.model").unwrap_or_else(|| "Unknown".to_string()),
        system_name: "macOS".to_string(),
        system_version,
        is_simulator: false,
    }
}
