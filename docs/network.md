---
title: "Network connectivity"
description: "Reachability and connection-type signals via day-part-network."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Network connectivity

Show whether the device has a network connection and what kind it is, such as Wi-Fi or cellular.
`day-part-network` returns a connectivity snapshot. For sending requests, use
[HTTP](/docs/reference/http).

## Add to your app

Defined in [`day-part-network`](https://github.com/daybrite/day/tree/main/parts/day-part-network). Add this to your app’s `Cargo.toml`
`[dependencies]` table, using the same Day Git source as the rest of your app:

```toml
[dependencies]
day-part-network = { git = "https://github.com/daybrite/day.git" }
```

The implementation is selected by the compilation target; no toolkit feature flags
or dependency entry in `Day.toml` are needed.

| Platforms / toolkits | Availability |
| --- | --- |
| macOS, iOS, Android, Windows, Linux (GTK / Qt), HarmonyOS | Connectivity information; available detail varies by OS. |
| Web | Unavailable; `status()` returns `None`. |

## Authoring

```rust
if let Some(n) = day_part_network::status() {
    println!("online: {}, kind: {:?}, expensive: {:?}", n.online, n.kind, n.expensive);
}
```

[`status() -> Option<NetworkStatus>`](/api/day_part_network/fn.status.html) returns `None` where there's no connectivity API (or the reading
failed), which is distinct from a successful reading that says offline (`Some` with `online: false`).
[`NetworkStatus { online: bool, kind: NetworkKind, expensive: Option<bool> }`](/api/day_part_network/struct.NetworkStatus.html); [`NetworkKind`](/api/day_part_network/enum.NetworkKind.html) is
`Wifi | Cellular | Ethernet | Other | None`.

There are no crate features; platform selection is purely `#[cfg(target_os)]`, because connectivity is
an OS concern rather than a widget-toolkit one. `parts/day-part-network/examples/network.rs` is a plain
`main` that uses it with no Day framework at all.

## Per-platform native realization

Platform guidance: [Android: network operations](https://developer.android.com/develop/connectivity/network-ops); [Apple: URL loading system](https://developer.apple.com/documentation/foundation/url-loading-system).

| OS | API | dependency |
|---|---|---|
| macOS | [`SCNetworkReachability`](https://developer.apple.com/documentation/systemconfiguration/scnetworkreachability) (SystemConfiguration) | raw C FFI, shared `apple.rs` |
| iOS | [`SCNetworkReachability`](https://developer.apple.com/documentation/systemconfiguration/scnetworkreachability) (`IsWWAN` ⇒ cellular) | raw C FFI, shared `apple.rs` |
| Windows | [`GetNetworkConnectivityHint`](https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-getnetworkconnectivityhint) (iphlpapi, resolved dynamically) | raw FFI (kernel32 + runtime lookup) |
| Linux | `/sys/class/net` interface scan | std only |
| HarmonyOS | [`OH_NetConn_HasDefaultNet`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-network-kit/capi-netconnection.md) / [`GetNetCapabilities`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-network-kit/capi-netconnection.md) (`libnet_connection.so`) | raw FFI (NetworkKit) |
| Android | [`ConnectivityManager`](https://developer.android.com/reference/android/net/ConnectivityManager) + [`NetworkCapabilities`](https://developer.android.com/reference/android/net/NetworkCapabilities) via a Java shim | `day-android` + `[package.metadata.day.android]` |

## What each platform can report

Every field is best-effort; the platforms report different amounts of detail:

- **macOS/iOS**: reachability answers "could traffic to the default route flow right now?" from the
  routing table. It sends no packets, so it cannot detect a captive portal or a dead upstream:
  `online` means routable, not verified internet. The only transport bit is `IsWWAN` (iOS-only):
  cellular reports `Cellular` + `expensive: Some(true)`; any other reachable iOS connection reports
  `Wifi` (the classic "ReachableViaWiFi" reading; it could in fact be wired or a tether). macOS gets
  no transport info at all, so an online Mac reports `Other`, `expensive: None`.
- **Android**: the richest reading. `online` is the system's `INTERNET` + `VALIDATED` verdict for the
  active network, `kind` comes from `TRANSPORT_WIFI/CELLULAR/ETHERNET`, and `expensive` is the
  inverse of `NET_CAPABILITY_NOT_METERED`. Requires `android.permission.ACCESS_NETWORK_STATE`, a
  normal install-time permission the crate contributes to the manifest itself (see below).
- **HarmonyOS**: `online` = a default network is activated; `kind` from the bearer type, `expensive`
  from `NETCONN_NET_CAPABILITY_NOT_METERED`. The app must declare `ohos.permission.GET_NETWORK_INFO`
  in its `module.json5` (`requestPermissions`; normal permission, no prompt) or the calls fail with
  201 and `status()` returns `None`.
- **Linux**: no daemon-independent connectivity API is guaranteed (NetworkManager is optional), so
  the crate scans `/sys/class/net`: `online` = a non-loopback interface has operstate `up`
  (link-level, not validated internet); `kind` from the kernel's predictable name prefixes (`wl*`
  wireless, `en*`/`eth*` wired, `ww*` wwan), preferring wired > wifi > cellular when several are up;
  `expensive` is always `None` (meteredness is a desktop-session concept).
- **Windows**: [`GetNetworkConnectivityHint`](https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-getnetworkconnectivityhint) (Windows 10 2004+, blind like the rest of the xaml
  toolkit) gives a connectivity level (`online` = internet or constrained-internet access) and a cost
  (`expensive`) but no transport, so `kind` is `Other` when online. The symbol is resolved at runtime
  via `LoadLibrary`/`GetProcAddress`, so apps still start on older Windows; `status()`
  returns `None` there.

A snapshot is a point-in-time poll; a change-notification rail ([`SCNetworkReachability`](https://developer.apple.com/documentation/systemconfiguration/scnetworkreachability) callbacks,
Android `NetworkCallback`, [`OH_NetConn_RegisterNetConnCallback`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-network-kit/capi-netconnection.md), `NotifyNetworkConnectivityHintChange`)
is a v2 follow-up.

Connectivity says whether traffic *could* flow; to actually move bytes through the platform's HTTP
stack (proxies, VPN, platform TLS), see its sibling part [`day-part-http`](http.md).

## What it shows about the extension system

Like [`day-part-battery`](/docs/reference/battery), this is a headless external crate: it has no UI Piece and registers nothing
into any toolkit's `RENDERERS` slice. It additionally exercises the **manifest-permission overlay**
(from the webview work, [docs/extending](extending.md)): `[package.metadata.day.android]` stages its own Java shim
and contributes `android.permission.ACCESS_NETWORK_STATE`, which `day build` merges into the app
manifest with no changes to any core Day crate. On Android the crate rides on the Day runtime
(day-android's cached JVM + `DayBridge.ctx`); on every other platform it is fully day-independent.
