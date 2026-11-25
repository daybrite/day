---
navTitle: "Network"
title: "Network connectivity"
description: "Reachability and connection-type signals via day-part-connectivity."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Network connectivity

Connectivity information describes the network connection available to a device, such as Wi-Fi or
cellular. It helps an app explain connection changes and choose when to attempt network work; it
does not guarantee that a particular service is reachable.

`day-part-connectivity` returns a connectivity snapshot. For sending requests, use
[HTTP](/docs/reference/http).

## Add to your app

Add [`day-part-connectivity`](https://github.com/daybrite/day/tree/main/crates/day-part-connectivity) to your app:

```sh
cargo add 'day-part-connectivity@0.*'
```

The implementation is selected by the compilation target; no toolkit feature flags
or dependency entry in `Day.toml` are needed.

| Platforms / toolkits | Availability |
| --- | --- |
| [macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit"), [iOS](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit"), [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material"), [Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui"), [Linux](https://daybrite.dev/docs/reference/platforms "toolkit:linux-gtk,linux-qt") ([GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") / [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt")), [HarmonyOS](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | Connectivity information; available detail varies by OS. |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | Browser data-saving preference where available. [`status()`](https://docs.rs/day-part-connectivity/latest/day_part_connectivity/fn.status.html) returns `None`. |

## Authoring

```rust
if let Some(n) = day_part_connectivity::status() {
    println!("online: {}, kind: {:?}, expensive: {:?}", n.online, n.kind, n.expensive);
}
```

[`status() -> Option<NetworkStatus>`](https://docs.rs/day-part-connectivity/latest/day_part_connectivity/fn.status.html) returns `None` where there's no connectivity API (or the reading
failed), which is distinct from a successful reading that says offline (`Some` with `online: false`).
[`NetworkStatus { online: bool, kind: NetworkKind, expensive: Option<bool> }`](https://docs.rs/day-part-connectivity/latest/day_part_connectivity/struct.NetworkStatus.html); [`NetworkKind`](https://docs.rs/day-part-connectivity/latest/day_part_connectivity/enum.NetworkKind.html) is
`Wifi | Cellular | Ethernet | Other | None`.

There are no crate features; platform selection is purely `#[cfg(target_os)]`, because connectivity is
an OS concern rather than a widget-toolkit one. `crates/day-part-connectivity/examples/network.rs` is a plain
`main` that uses it with no Day framework at all.

## Platform implementations

Platform guidance: [Android: network operations](https://developer.android.com/develop/connectivity/network-ops); [Apple: URL loading system](https://developer.apple.com/documentation/foundation/url-loading-system).

| OS | API | dependency |
|---|---|---|
| [macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | [`SCNetworkReachability`](https://developer.apple.com/documentation/systemconfiguration/scnetworkreachability) (SystemConfiguration) | raw C FFI, shared `apple.rs` |
| [iOS](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | [`SCNetworkReachability`](https://developer.apple.com/documentation/systemconfiguration/scnetworkreachability) (`IsWWAN` ⇒ cellular) | raw C FFI, shared `apple.rs` |
| [Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | [`GetNetworkConnectivityHint`](https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-getnetworkconnectivityhint) (iphlpapi, resolved dynamically) | raw FFI (kernel32 + runtime lookup) |
| [Linux](https://daybrite.dev/docs/reference/platforms "toolkit:linux-gtk,linux-qt") | `/sys/class/net` interface scan | optional system D-Bus cost query |
| [HarmonyOS](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | [`OH_NetConn_HasDefaultNet`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-network-kit/capi-netconnection.md) / [`GetNetCapabilities`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-network-kit/capi-netconnection.md) (`libnet_connection.so`) | raw FFI (NetworkKit) |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | [`ConnectivityManager`](https://developer.android.com/reference/android/net/ConnectivityManager) + [`NetworkCapabilities`](https://developer.android.com/reference/android/net/NetworkCapabilities) via a Java shim | `day-toolkit-material` + `[package.metadata.day.android]` |

## Cost and data saving

[`network_conditions()`](https://docs.rs/day-part-connectivity/latest/day_part_connectivity/fn.network_conditions.html)
returns two independent optional booleans in [`NetworkConditions`](https://docs.rs/day-part-connectivity/latest/day_part_connectivity/struct.NetworkConditions.html).
`expensive` describes metering or cost. `constrained` describes a request to reduce data usage.
Either value can be unknown. A snapshot describes the default route at the time of observation.

| Platform | Expensive | Constrained |
| --- | --- | --- |
| macOS and iOS | [Network path cost](https://developer.apple.com/documentation/network/nwpath/isexpensive) | [Low Data Mode](https://developer.apple.com/documentation/network/nwpath/isconstrained). The first snapshot can be unknown while the path monitor starts. |
| Android | Active network capabilities, including temporary unmetering | [Data Saver](https://developer.android.com/develop/connectivity/network-ops/data-saver) applies to this app on a metered network. App exemptions are respected. |
| Windows | Fixed or variable connection cost | [Background data usage restriction](https://learn.microsoft.com/en-us/uwp/api/windows.networking.connectivity.connectioncost.backgrounddatausagerestricted) on the current internet connection profile. |
| Linux | [NetworkManager metering](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.html), including estimated cost, when libsystemd and the service are available | Unknown |
| HarmonyOS | NetworkKit metering capability | Unknown |
| Web | Unknown | [Browser data-saving preference](https://wicg.github.io/netinfo/), when exposed |

## What each platform can report

Every field is best-effort; the platforms report different amounts of detail:

- **[macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit")/[iOS](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit")**: reachability answers "could traffic to the default route flow right now?" from the
  routing table. It sends no packets, so it cannot detect a captive portal or a dead upstream:
  `online` means routable, not verified internet. The only transport bit is `IsWWAN` (iOS-only):
  cellular reports `Cellular`; any other reachable iOS connection reports
  `Wifi` (the classic "ReachableViaWiFi" reading; it could in fact be wired or a tether). macOS gets
  no transport info at all, so an online Mac reports `Other`.
- **[Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material")**: the richest reading. `online` is the system's `INTERNET` + `VALIDATED` verdict for the
  active network, `kind` comes from `TRANSPORT_WIFI/CELLULAR/ETHERNET`, and `expensive` is the
  inverse of the unmetered and temporarily unmetered capabilities. Requires `android.permission.ACCESS_NETWORK_STATE`, a
  normal install-time permission the crate contributes to the manifest itself (see below).
- **[HarmonyOS](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui")**: `online` = a default network is activated; `kind` from the bearer type, `expensive`
  from `NETCONN_NET_CAPABILITY_NOT_METERED`. The app must declare `ohos.permission.GET_NETWORK_INFO`
  in its `module.json5` (`requestPermissions`; normal permission, no prompt) or the calls fail with
  201 and [`status()`](https://docs.rs/day-part-connectivity/latest/day_part_connectivity/fn.status.html) returns `None`.
- **[Linux](https://daybrite.dev/docs/reference/platforms "toolkit:linux-gtk,linux-qt")**: no daemon-independent connectivity API is guaranteed (NetworkManager is optional), so
  the crate scans `/sys/class/net`: `online` = a non-loopback interface has operstate `up`
  (link-level, not validated internet); `kind` from the kernel's predictable name prefixes (`wl*`
  wireless, `en*`/`eth*` wired, `ww*` wwan), preferring wired > wifi > cellular when several are up;
  `expensive` comes from NetworkManager where available.
- **[Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui")**: [`GetNetworkConnectivityHint`](https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-getnetworkconnectivityhint) (Windows 10 2004+) gives a connectivity level (`online` = internet or constrained-internet access) and a cost
  (`expensive`) but no transport, so `kind` is `Other` when online. The symbol is resolved at runtime
  via `LoadLibrary`/`GetProcAddress`, so apps still start on older Windows; [`status()`](https://docs.rs/day-part-connectivity/latest/day_part_connectivity/fn.status.html)
  returns `None` there.

A snapshot is a point-in-time poll; a change-notification rail ([`SCNetworkReachability`](https://developer.apple.com/documentation/systemconfiguration/scnetworkreachability) callbacks,
[Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") `NetworkCallback`, [`OH_NetConn_RegisterNetConnCallback`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-network-kit/capi-netconnection.md), `NotifyNetworkConnectivityHintChange`)
is a v2 follow-up.

Connectivity says whether traffic *could* flow; to actually move bytes through the platform's HTTP
stack (proxies, VPN, platform TLS), see its sibling part [`day-part-http`](http.md).

## Extension integration

Like [`day-part-device`](/docs/reference/battery), this is a headless external crate: it has no UI Piece and registers nothing
into any toolkit's `RENDERERS` slice. It additionally exercises the **manifest-permission overlay**
(from the webview work, [docs/extending](extending.md)): `[package.metadata.day.android]` stages its own Java shim
and contributes `android.permission.ACCESS_NETWORK_STATE`, which `day build` merges into the app
manifest with no changes to any core Day crate. On Android the crate rides on the Day runtime
(day-toolkit-material's cached JVM + `DayBridge.ctx`); on every other platform it is fully day-independent.
