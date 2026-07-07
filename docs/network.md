# Network connectivity (headless capability crate)

> **Status: implemented** as `day-part-network` (in `parts/`, the headless counterpart of `pieces/`) — a
> **headless** day-ecosystem crate (no UI Piece): a shared cross-platform API for reading a snapshot of
> the device's network connectivity through each platform's NATIVE API. Any Rust code can depend on it
> and call `day_part_network::status()`. Verified on macOS (real online reading); iOS-sim/Android
> clippy-clean and HarmonyOS cross-compiles + links against the native `libnet_connection.so`.

## Authoring

```rust
if let Some(n) = day_part_network::status() {
    println!("online: {}, kind: {:?}, expensive: {:?}", n.online, n.kind, n.expensive);
}
```

`status() -> Option<NetworkStatus>` returns `None` where there's no connectivity API (or the reading
failed) — distinct from a successful reading that says offline (`Some` with `online: false`).
`NetworkStatus { online: bool, kind: NetworkKind, expensive: Option<bool> }`; `NetworkKind` is
`Wifi | Cellular | Ethernet | Other | None`.

There are **no features** — platform selection is purely `#[cfg(target_os)]`, because connectivity is an
OS concern, not a widget-toolkit one. `parts/day-part-network/examples/network.rs` is a plain `main` that
uses it with no Day framework at all.

## Per-platform native realization

| OS | API | dependency |
|---|---|---|
| macOS | `SCNetworkReachability` (SystemConfiguration) | raw C FFI, shared `apple.rs` |
| iOS | `SCNetworkReachability` (`IsWWAN` ⇒ cellular) | raw C FFI, shared `apple.rs` |
| Windows | `GetNetworkConnectivityHint` (iphlpapi, resolved dynamically) | raw FFI (kernel32 + runtime lookup) |
| Linux | `/sys/class/net` interface scan | std only |
| HarmonyOS | `OH_NetConn_HasDefaultNet` / `GetNetCapabilities` (`libnet_connection.so`) | raw FFI (NetworkKit) |
| Android | `ConnectivityManager` + `NetworkCapabilities` via a Java shim | `day-android` + `[package.metadata.day.android]` |

## What each platform can honestly report

Every field is **best-effort**; the platforms report different slices of the truth:

- **macOS/iOS** — reachability answers "could traffic to the default route flow right now?" from the
  routing table. It sends no packets, so it cannot detect a captive portal or a dead upstream:
  `online` means *routable*, not *verified internet*. The only transport bit is `IsWWAN` (iOS-only):
  cellular reports `Cellular` + `expensive: Some(true)`; any other reachable iOS connection reports
  `Wifi` (the classic "ReachableViaWiFi" reading — it could in fact be wired or a tether). macOS gets
  no transport info at all, so an online Mac reports `Other`, `expensive: None`.
- **Android** — the richest reading: `online` is the system's `INTERNET` + `VALIDATED` verdict for the
  active network, `kind` comes from `TRANSPORT_WIFI/CELLULAR/ETHERNET`, and `expensive` is the
  inverse of `NET_CAPABILITY_NOT_METERED`. Requires `android.permission.ACCESS_NETWORK_STATE` — a
  normal install-time permission the crate contributes to the manifest itself (see below).
- **HarmonyOS** — `online` = a default network is activated; `kind` from the bearer type, `expensive`
  from `NETCONN_NET_CAPABILITY_NOT_METERED`. The app must declare `ohos.permission.GET_NETWORK_INFO`
  in its `module.json5` (`requestPermissions`; normal permission, no prompt) or the calls fail with
  201 and `status()` returns `None`.
- **Linux** — no daemon-independent connectivity API is guaranteed (NetworkManager is optional), so
  the crate scans `/sys/class/net`: `online` = a non-loopback interface has operstate `up`
  (link-level, not validated internet); `kind` from the kernel's predictable name prefixes (`wl*`
  wireless, `en*`/`eth*` wired, `ww*` wwan), preferring wired > wifi > cellular when several are up;
  `expensive` is always `None` (meteredness is a desktop-session concept).
- **Windows** — `GetNetworkConnectivityHint` (Windows 10 2004+, blind like the rest of the winui
  backend) gives a connectivity level (`online` = internet or constrained-internet access) and a cost
  (`expensive`) but no transport, so `kind` is `Other` when online. The symbol is resolved at runtime
  via `LoadLibrary`/`GetProcAddress`, so apps still *start* on older Windows — `status()` just
  returns `None` there.

A snapshot is a point-in-time poll; a change-notification rail (`SCNetworkReachability` callbacks,
Android `NetworkCallback`, `OH_NetConn_RegisterNetConnCallback`, `NotifyNetworkConnectivityHintChange`)
is a v2 follow-up.

## What it shows about the extension system

Like `day-part-battery`, this is a headless external crate — no UI Piece, nothing registered into any
backend's `RENDERERS` slice. It additionally exercises the **manifest-permission overlay** (from the
webview work, docs/extending.md): `[package.metadata.day.android]` stages its own Java shim *and*
contributes `android.permission.ACCESS_NETWORK_STATE`, which `day build` merges into the app manifest —
zero edits to any core day crate. On Android the crate rides on the Day runtime (day-android's cached
JVM + `DayBridge.ctx`); on every other platform it is fully day-independent.
