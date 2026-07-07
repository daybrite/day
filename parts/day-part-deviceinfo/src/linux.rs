// Linux: no single portable device-identity API is guaranteed, so we read the two files every desktop
// distro provides: /etc/os-release (the freedesktop standard — NAME/PRETTY_NAME + VERSION_ID) for the
// OS name/version, and the DMI node /sys/devices/virtual/dmi/id/product_name for the hardware model
// (e.g. "20XW..." on a ThinkPad, "VirtualBox" on a VM). Pure std — no dependencies. There is no
// simulator concept on desktop Linux.

use super::DeviceInfo;
use std::fs;

/// Parse a shell-style `KEY=value` file (os-release), returning the value for `key` with any
/// surrounding single/double quotes stripped. First match wins.
fn os_release_value(contents: &str, key: &str) -> Option<String> {
    for line in contents.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(key).and_then(|r| r.strip_prefix('=')) {
            let v = rest.trim().trim_matches(['"', '\'']).trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

pub fn get() -> DeviceInfo {
    let os_release = fs::read_to_string("/etc/os-release").unwrap_or_default();
    // Prefer the clean distro NAME ("Ubuntu"); fall back to PRETTY_NAME ("Ubuntu 22.04.3 LTS").
    let system_name = os_release_value(&os_release, "NAME")
        .or_else(|| os_release_value(&os_release, "PRETTY_NAME"))
        .unwrap_or_else(|| "Linux".to_string());
    let system_version =
        os_release_value(&os_release, "VERSION_ID").unwrap_or_else(|| "Unknown".to_string());

    let model = fs::read_to_string("/sys/devices/virtual/dmi/id/product_name")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Linux".to_string());

    DeviceInfo {
        model,
        system_name,
        system_version,
        is_simulator: false,
    }
}
