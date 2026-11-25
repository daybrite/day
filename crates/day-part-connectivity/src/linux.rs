// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Linux: there is no daemon-independent connectivity API guaranteed to exist (NetworkManager /
// systemd-networkd are optional), so scan /sys/class/net: a non-loopback interface with operstate
// "up" means link-level connectivity. Kind is inferred from the kernel's predictable interface-name
// prefixes (wl* wireless, en*/eth* wired, ww* wwan), preferring wired > wifi > cellular when several
// are up. "online" here means a link is up, not that internet access was validated; meteredness is a
// NetworkManager property read separately from link state.

use super::{NetworkKind, NetworkStatus};
use std::fs;

pub fn status() -> Option<NetworkStatus> {
    let entries = fs::read_dir("/sys/class/net").ok()?;
    let mut best: Option<NetworkKind> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "lo" {
            continue;
        }
        let up = fs::read_to_string(entry.path().join("operstate")).is_ok_and(|s| s.trim() == "up");
        if !up {
            continue;
        }
        let kind = kind_for(&name);
        best = Some(match best {
            Some(b) if rank(b) >= rank(kind) => b,
            _ => kind,
        });
    }
    Some(match best {
        Some(kind) => NetworkStatus {
            online: true,
            kind,
            expensive: metered(),
        },
        None => NetworkStatus {
            online: false,
            kind: NetworkKind::None,
            expensive: None,
        },
    })
}

/// Classify an interface by the kernel's predictable naming (wlan0/wlp3s0, eth0/enp0s31f6, wwan0).
fn kind_for(name: &str) -> NetworkKind {
    if name.starts_with("wl") {
        NetworkKind::Wifi
    } else if name.starts_with("en") || name.starts_with("eth") {
        NetworkKind::Ethernet
    } else if name.starts_with("ww") {
        NetworkKind::Cellular
    } else {
        NetworkKind::Other
    }
}

/// Preference when several interfaces are up (report the "primary-est" one).
fn rank(kind: NetworkKind) -> u8 {
    match kind {
        NetworkKind::Ethernet => 3,
        NetworkKind::Wifi => 2,
        NetworkKind::Cellular => 1,
        NetworkKind::Other | NetworkKind::None => 0,
    }
}

/// `NetworkManager`'s Metered property describes the default route, including the daemon's
/// guessed-yes and guessed-no classifications. The system bus and libsystemd are optional.
pub fn network_conditions() -> super::NetworkConditions {
    super::NetworkConditions {
        expensive: metered(),
        constrained: None,
    }
}

fn metered() -> Option<bool> {
    use std::ffi::{c_char, c_int, c_void};
    use std::sync::OnceLock;

    type Bus = *mut c_void;
    struct Api {
        _library: libloading::Library,
        open: unsafe extern "C" fn(*mut Bus) -> c_int,
        unref: unsafe extern "C" fn(Bus) -> Bus,
        set_timeout: unsafe extern "C" fn(Bus, u64) -> c_int,
        property: unsafe extern "C" fn(
            Bus,
            *const c_char,
            *const c_char,
            *const c_char,
            *const c_char,
            *mut c_void,
            c_char,
            *mut c_void,
        ) -> c_int,
    }
    static API: OnceLock<Option<Api>> = OnceLock::new();
    // SAFETY: these symbols use the stable sd-bus.h ABI. The library outlives its symbols.
    let api = API
        .get_or_init(|| unsafe {
            let lib = libloading::Library::new("libsystemd.so.0").ok()?;
            Some(Api {
                open: *lib.get(b"sd_bus_open_system\0").ok()?,
                unref: *lib.get(b"sd_bus_unref\0").ok()?,
                set_timeout: *lib.get(b"sd_bus_set_method_call_timeout\0").ok()?,
                property: *lib.get(b"sd_bus_get_property_trivial\0").ok()?,
                _library: lib,
            })
        })
        .as_ref()?;
    let mut bus = std::ptr::null_mut();
    let mut value = 0u32;
    // SAFETY: the bus belongs to this call. The property has D-Bus type 'u' (u32)
    // and all strings and output storage remain valid until the synchronous call returns.
    unsafe {
        if (api.open)(&raw mut bus) < 0 {
            return None;
        }
        let ok = (api.set_timeout)(bus, 150_000) >= 0
            && (api.property)(
                bus,
                c"org.freedesktop.NetworkManager".as_ptr(),
                c"/org/freedesktop/NetworkManager".as_ptr(),
                c"org.freedesktop.NetworkManager".as_ptr(),
                c"Metered".as_ptr(),
                std::ptr::null_mut(),
                b'u' as c_char,
                (&raw mut value).cast(),
            ) >= 0;
        (api.unref)(bus);
        if !ok {
            return None;
        }
    }
    match value {
        1 | 3 => Some(true),
        2 | 4 => Some(false),
        _ => None,
    }
}
