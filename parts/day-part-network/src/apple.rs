// macOS + iOS (one shared file): SystemConfiguration's SCNetworkReachability. A target created with
// CreateWithAddress on 0.0.0.0 asks "could traffic to the default route flow right now?"; GetFlags
// answers synchronously from the routing table (no packets are sent, so this cannot detect a captive
// portal or a dead upstream — "online" means routable, not verified internet). Plain-C FFI; the
// SystemConfiguration framework is force-linked below, no crates needed.
//
// What reachability can and cannot say about `kind`: the only transport bit is IsWWAN (iOS-only,
// cellular). A reachable non-WWAN iOS connection is reported as Wifi — the classic
// "ReachableViaWiFi" reading — though it could in fact be wired or a tether. macOS gets no
// transport information at all, so an online Mac reports Other.

use super::{NetworkKind, NetworkStatus};
use std::os::raw::c_void;

// SCNetworkReachabilityFlags (SCNetworkReachability.h).
const FLAG_REACHABLE: u32 = 1 << 1; // kSCNetworkReachabilityFlagsReachable
const FLAG_CONNECTION_REQUIRED: u32 = 1 << 2; // kSCNetworkReachabilityFlagsConnectionRequired
#[cfg(target_os = "ios")]
const FLAG_IS_WWAN: u32 = 1 << 18; // kSCNetworkReachabilityFlagsIsWWAN (API_UNAVAILABLE(macos))

/// BSD `sockaddr_in`, declared locally to avoid a libc dependency. Zeroed address = 0.0.0.0,
/// the conventional "default route" reachability target.
#[repr(C)]
struct SockaddrIn {
    sin_len: u8,
    sin_family: u8, // AF_INET = 2
    sin_port: u16,
    sin_addr: u32,
    sin_zero: [u8; 8],
}

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCNetworkReachabilityCreateWithAddress(
        allocator: *const c_void,
        address: *const SockaddrIn,
    ) -> *const c_void;
    fn SCNetworkReachabilityGetFlags(target: *const c_void, flags: *mut u32) -> u8; // Boolean
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

pub fn status() -> Option<NetworkStatus> {
    let zero = SockaddrIn {
        sin_len: size_of::<SockaddrIn>() as u8,
        sin_family: 2, // AF_INET
        sin_port: 0,
        sin_addr: 0,
        sin_zero: [0; 8],
    };
    unsafe {
        let target = SCNetworkReachabilityCreateWithAddress(std::ptr::null(), &zero);
        if target.is_null() {
            return None;
        }
        let mut flags: u32 = 0;
        let ok = SCNetworkReachabilityGetFlags(target, &mut flags);
        CFRelease(target);
        (ok != 0).then(|| interpret(flags))
    }
}

fn interpret(flags: u32) -> NetworkStatus {
    // Online = reachable without first bringing a connection up (PPP/VPN dial, user intervention).
    let online = flags & FLAG_REACHABLE != 0 && flags & FLAG_CONNECTION_REQUIRED == 0;
    if !online {
        return NetworkStatus {
            online: false,
            kind: NetworkKind::None,
            expensive: None,
        };
    }
    #[cfg(target_os = "ios")]
    if flags & FLAG_IS_WWAN != 0 {
        return NetworkStatus {
            online: true,
            kind: NetworkKind::Cellular,
            expensive: Some(true),
        };
    }
    // Non-cellular: Wi-Fi on iOS (best-effort — see the header comment), unknown on macOS.
    #[cfg(target_os = "ios")]
    let kind = NetworkKind::Wifi;
    #[cfg(target_os = "macos")]
    let kind = NetworkKind::Other;
    NetworkStatus {
        online: true,
        kind,
        expensive: None,
    }
}
