// Windows: GetNetworkConnectivityHint (iphlpapi, Windows 10 2004+) fills an
// NL_NETWORK_CONNECTIVITY_HINT with a connectivity level and a cost. The symbol is resolved
// dynamically (LoadLibrary/GetProcAddress) so apps still start on older Windows — status() just
// returns None there. The hint reports level + cost but NOT the transport, so kind is Other when
// online. Raw FFI (shapes cross-checked against windows-sys 0.61's IpHelper/WinSock bindings).
// Written blind (no Windows host); compiled only on the windows target.

use super::{NetworkKind, NetworkStatus};
use std::os::raw::c_void;

// NL_NETWORK_CONNECTIVITY_LEVEL_HINT: 0 unknown, 1 none, 2 local access, 3 internet access,
// 4 constrained internet access, 5 hidden.
const LEVEL_INTERNET: i32 = 3;
const LEVEL_CONSTRAINED_INTERNET: i32 = 4;
// NL_NETWORK_CONNECTIVITY_COST_HINT: 0 unknown, 1 unrestricted, 2 fixed, 3 variable.
const COST_UNRESTRICTED: i32 = 1;
const COST_FIXED: i32 = 2;
const COST_VARIABLE: i32 = 3;

/// NL_NETWORK_CONNECTIVITY_HINT (netioapi.h). The three BOOLEANs are u8-sized.
#[repr(C)]
struct NlNetworkConnectivityHint {
    connectivity_level: i32,
    connectivity_cost: i32,
    approaching_data_limit: u8,
    over_data_limit: u8,
    roaming: u8,
}

type GetNetworkConnectivityHintFn =
    unsafe extern "system" fn(*mut NlNetworkConnectivityHint) -> u32;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

/// Resolve GetNetworkConnectivityHint at runtime (absent before Windows 10 2004 / build 19041).
fn lookup() -> Option<GetNetworkConnectivityHintFn> {
    let dll: Vec<u16> = "iphlpapi.dll\0".encode_utf16().collect();
    let module = unsafe { LoadLibraryW(dll.as_ptr()) };
    if module.is_null() {
        return None;
    }
    let proc = unsafe { GetProcAddress(module, c"GetNetworkConnectivityHint".as_ptr().cast()) };
    if proc.is_null() {
        return None;
    }
    Some(unsafe { std::mem::transmute::<*mut c_void, GetNetworkConnectivityHintFn>(proc) })
}

pub fn status() -> Option<NetworkStatus> {
    let get_hint = lookup()?;
    let mut hint = NlNetworkConnectivityHint {
        connectivity_level: 0,
        connectivity_cost: 0,
        approaching_data_limit: 0,
        over_data_limit: 0,
        roaming: 0,
    };
    if unsafe { get_hint(&mut hint) } != 0 {
        return None; // NETIO error status
    }
    let online = matches!(
        hint.connectivity_level,
        LEVEL_INTERNET | LEVEL_CONSTRAINED_INTERNET
    );
    let kind = if online {
        NetworkKind::Other // the hint carries no transport information
    } else {
        NetworkKind::None
    };
    let expensive = match hint.connectivity_cost {
        COST_UNRESTRICTED => Some(false),
        COST_FIXED | COST_VARIABLE => Some(true),
        _ => None,
    };
    Some(NetworkStatus {
        online,
        kind,
        expensive,
    })
}
