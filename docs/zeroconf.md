---
title: "Local network discovery"
description: "Find, resolve, and advertise services on the local network with DNS-SD."
navTitle: "Local network discovery"
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Local network discovery

Local network discovery finds services on the same network without asking people to enter an
address. It helps apps locate resources such as printers or another app that is offering a service.

`day-part-zeroconf` uses DNS-based service discovery (DNS-SD) to report service names, addresses,
ports, and TXT attributes. It can also advertise a service the app hosts. Discovery supplies
connection information; the app chooses whether and how to connect.

## Add to your app

Add [`day-part-zeroconf`](https://crates.io/crates/day-part-zeroconf) to your app:

```sh
cargo add day-part-zeroconf
```

[Crate](https://crates.io/crates/day-part-zeroconf) ·
[API reference](https://docs.rs/day-part-zeroconf/latest/day_part_zeroconf/) ·
[Source](https://github.com/daybrite/day-part-zeroconf).
Use the [Day CLI](/docs/reference/cli) to include mobile bridge code and permission metadata.
No toolkit feature is required. Native service libraries and app permissions are described below.

## Browse a service type

Use an async event loop rather than blocking the UI thread. This integration helper waits for
one HTTP service; the caller decides whether to connect to it. `_http._tcp` is a protocol
identifier, not a translated label.

```rust
use day_part_zeroconf::{BrowseOptions, Event, Result, Service, browse};

pub async fn first_http_service() -> Result<Option<Service>> {
    let mut operation = browse(BrowseOptions::service("_http._tcp"))?;
    while let Some(event) = operation.next_event().await {
        match event {
            Event::Resolved(service) => return Ok(Some(service)),
            Event::Error(error) => return Err(error),
            _ => {}
        }
    }
    Ok(None)
}
```

[`browse()`](https://docs.rs/day-part-zeroconf/latest/day_part_zeroconf/fn.browse.html)
returns an [`Operation`](https://docs.rs/day-part-zeroconf/latest/day_part_zeroconf/struct.Operation.html).
Keep it alive for continuing updates. Dropping it requests cancellation; its
[`cancellation()`](https://docs.rs/day-part-zeroconf/latest/day_part_zeroconf/struct.Operation.html#method.cancellation)
handle lets a UI action cancel without blocking. The synchronous
[`recv_timeout()`](https://docs.rs/day-part-zeroconf/latest/day_part_zeroconf/struct.Operation.html#method.recv_timeout)
and joining [`stop()`](https://docs.rs/day-part-zeroconf/latest/day_part_zeroconf/struct.Operation.html#method.stop)
belong on a worker thread.

| Operation | Use |
| --- | --- |
| [`BrowseOptions::service(type)`](https://docs.rs/day-part-zeroconf/latest/day_part_zeroconf/struct.BrowseOptions.html#method.service) | Browse a known `_name._tcp` or `_name._udp` service in the local domain. |
| [`BrowseOptions::all()`](https://docs.rs/day-part-zeroconf/latest/day_part_zeroconf/struct.BrowseOptions.html#method.all) | Enumerate types and browse them. An empty scan does not establish that no services exist. |
| [`resolve(id)`](https://docs.rs/day-part-zeroconf/latest/day_part_zeroconf/fn.resolve.html) | Resolve an identity already known to the app. |
| [`advertise(service)`](https://docs.rs/day-part-zeroconf/latest/day_part_zeroconf/fn.advertise.html) | Advertise an existing listener's port and attributes. It does not start a server. |

## Identity, updates, and TXT data

A [`ServiceId`](https://docs.rs/day-part-zeroconf/latest/day_part_zeroconf/struct.ServiceId.html)
includes the name, type, domain, and interface. Keep the whole identity when updating a list:
the same instance can appear on several interfaces. Resolved events can repeat as addresses
arrive; a removed event invalidates that observation. Preserve scope IDs on IPv6 link-local
addresses. A service name or TXT record is not proof of a peer's identity.

[`TxtRecord`](https://docs.rs/day-part-zeroconf/latest/day_part_zeroconf/struct.TxtRecord.html)
values are bytes. A key without a value differs from a key with an empty value. Publication
rejects duplicate keys and limits each item to 255 bytes and the whole record to 1300 bytes.
Check [`capabilities()`](https://docs.rs/day-part-zeroconf/latest/day_part_zeroconf/fn.capabilities.html)
for binary values, key-only values, and explicit interface selection. On event-queue overflow,
stop the operation and restart with a narrower type.

Advertisement reports the actual registered name after native conflict handling. Linux may
report a collision instead of choosing another name. Keep its operation alive while the
listener is available, and drop it when the listener closes.

## Platforms and permissions

| Platform | Setup and limits |
| --- | --- |
| macOS / iOS | [Bonjour](https://developer.apple.com/bonjour/). iOS needs declared service types and a localized local-network usage description. Arbitrary type enumeration on a physical iOS device also requires Apple's multicast entitlement. |
| Android | [NsdManager](https://developer.android.com/reference/android/net/nsd/NsdManager). TXT publication uses UTF-8 strings; received values retain their bytes. The OS chooses the network. |
| Windows / WinUI | [DNS-SD APIs](https://learn.microsoft.com/en-us/windows/win32/api/windns/nf-windns-dnsservicebrowse). Resolution is one-shot; browse or resolve again to refresh metadata. |
| Linux / GTK / Qt | [Avahi](https://avahi.org/), its client/common shared libraries, a running daemon, and system D-Bus. |
| HarmonyOS | Native mDNS service. It chooses the interface, returns one address, and rejects key-only TXT publication. |
| Web | Unsupported; browsers do not expose a general local-network DNS-SD API. |

On Apple platforms, declare the intended service types and localized permission rationale in
the [native host configuration](/docs/how-to/native-projects). A sandboxed macOS app
also needs the network client/server entitlements appropriate to its operations. Declaring a
capability does not grant an entitlement or user permission.

Android and HarmonyOS contribute network permissions through the crate's metadata. Check
[`requires_explicit_permission()`](https://docs.rs/day-part-zeroconf/latest/day_part_zeroconf/fn.requires_explicit_permission.html)
and the [crate's host setup](https://github.com/daybrite/day-part-zeroconf/blob/main/docs/host-setup.md)
for the additional runtime permission requirements of the selected OS and target SDK.

Type enumeration on Android and HarmonyOS uses the default IPv4 multicast route. Targeted
browsing is preferable on IPv6-only networks or when a responder does not advertise its type.
Older Android, Windows, and HarmonyOS resolution needs explicit refresh for changing metadata.

## Related documentation

Use [HTTP](/docs/reference/http) or your app's transport to connect after discovery,
[connectivity](/docs/reference/network) to observe network changes, and
[permissions](/docs/reference/permissions) for app permission handling.
