// Linux: there is no daemon-independent connectivity API guaranteed to exist (NetworkManager /
// systemd-networkd are optional), so scan /sys/class/net — a non-loopback interface with operstate
// "up" means link-level connectivity. Kind is inferred from the kernel's predictable interface-name
// prefixes (wl* wireless, en*/eth* wired, ww* wwan), preferring wired > wifi > cellular when several
// are up. "online" here means a link is up, NOT that internet access was validated; meteredness is a
// desktop-session concept the kernel doesn't know, so expensive is always None. Pure std.

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
        let up = fs::read_to_string(entry.path().join("operstate"))
            .map(|s| s.trim() == "up")
            .unwrap_or(false);
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
            expensive: None,
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
