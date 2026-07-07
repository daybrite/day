// HarmonyOS / OpenHarmony: the native NetworkKit connection-management C API (`libnet_connection.so`,
// `net_connection.h`, API 11+). Pure FFI, like macOS/iOS — no ArkTS bridge or Day runtime needed
// (unlike Android's ConnectivityManager, which rides day-android's JVM/Context). The app DOES need
// the `ohos.permission.GET_NETWORK_INFO` permission declared in its module.json5 (a normal
// permission, no user prompt); without it the calls fail with 201 and status() returns None.

use core::ffi::c_int;

use super::{NetworkKind, NetworkStatus};

// net_connection_type.h array bounds.
const NETCONN_MAX_CAP_SIZE: usize = 32;
const NETCONN_MAX_BEARER_TYPE_SIZE: usize = 32;

// NetConn_NetBearerType: 0 = cellular, 1 = WIFI, 2 = bluetooth, 3 = ethernet, 4 = VPN.
const BEARER_CELLULAR: c_int = 0;
const BEARER_WIFI: c_int = 1;
const BEARER_ETHERNET: c_int = 3;
// NetConn_NetCap: 11 = NETCONN_NET_CAPABILITY_NOT_METERED.
const CAPABILITY_NOT_METERED: c_int = 11;

/// NetConn_NetHandle (net_connection_type.h).
#[repr(C)]
struct NetConnNetHandle {
    net_id: i32,
}

/// NetConn_NetCapabilities (net_connection_type.h).
#[repr(C)]
struct NetConnNetCapabilities {
    // Written by the C side; only the caps/bearer lists are read here.
    #[allow(dead_code)]
    link_up_bandwidth_kbps: u32,
    #[allow(dead_code)]
    link_down_bandwidth_kbps: u32,
    net_caps: [c_int; NETCONN_MAX_CAP_SIZE],
    net_caps_size: i32,
    bearer_types: [c_int; NETCONN_MAX_BEARER_TYPE_SIZE],
    bearer_types_size: i32,
}

#[link(name = "net_connection")]
unsafe extern "C" {
    fn OH_NetConn_HasDefaultNet(has_default_net: *mut i32) -> i32;
    fn OH_NetConn_GetDefaultNet(net_handle: *mut NetConnNetHandle) -> i32;
    fn OH_NetConn_GetNetCapabilities(
        net_handle: *mut NetConnNetHandle,
        net_capabilities: *mut NetConnNetCapabilities,
    ) -> i32;
}

pub fn status() -> Option<NetworkStatus> {
    let mut has_default: i32 = 0;
    // Non-zero return = permission missing (201) or service unreachable — no reading at all.
    if unsafe { OH_NetConn_HasDefaultNet(&mut has_default) } != 0 {
        return None;
    }
    if has_default == 0 {
        return Some(NetworkStatus {
            online: false,
            kind: NetworkKind::None,
            expensive: None,
        });
    }

    // A default network is activated; refine kind/expensive from its capabilities (best-effort —
    // if the detail calls fail we still report online).
    let mut handle = NetConnNetHandle { net_id: 0 };
    let mut caps: NetConnNetCapabilities = unsafe { std::mem::zeroed() };
    let detailed = unsafe {
        OH_NetConn_GetDefaultNet(&mut handle) == 0
            && OH_NetConn_GetNetCapabilities(&mut handle, &mut caps) == 0
    };
    if !detailed {
        return Some(NetworkStatus {
            online: true,
            kind: NetworkKind::Other,
            expensive: None,
        });
    }

    let bearers = caps
        .bearer_types_size
        .clamp(0, NETCONN_MAX_BEARER_TYPE_SIZE as i32) as usize;
    let kind = match caps.bearer_types[..bearers].first() {
        Some(&BEARER_WIFI) => NetworkKind::Wifi,
        Some(&BEARER_CELLULAR) => NetworkKind::Cellular,
        Some(&BEARER_ETHERNET) => NetworkKind::Ethernet,
        _ => NetworkKind::Other,
    };
    let ncaps = caps.net_caps_size.clamp(0, NETCONN_MAX_CAP_SIZE as i32) as usize;
    let expensive = Some(!caps.net_caps[..ncaps].contains(&CAPABILITY_NOT_METERED));
    Some(NetworkStatus {
        online: true,
        kind,
        expensive,
    })
}
