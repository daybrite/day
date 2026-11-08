---
title: "Bluetooth"
description: "Connect to Bluetooth Low Energy peripherals and simulate their replies in automated tests."
navTitle: "Bluetooth"
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Bluetooth

Bluetooth Low Energy connects an app to nearby sensors and accessories. A peripheral exposes
services containing characteristics: values the app can read, write, or watch for changes.

`day-part-bluetooth` provides discovery, connections, GATT service inspection, and characteristic
notifications. Test simulations supply replies and events through the same API calls used on a device.

## Add to your app

The package is available from source ahead of its first crates.io release. With the package
checked out beside your app:

```toml
[dependencies]
day-part-bluetooth = { path = "../day-part-bluetooth" }
```

Build with the [Day CLI](/docs/reference/cli) to include native sources and permissions.
Apple and Android builds require Day's external bridge source loading in both `day-build` and
`day-cli`. Use a Day checkout containing that support until it is released.

The package's README and bundled guide contain the full usage instructions. Generate its API
reference locally with `cargo doc --no-deps --open` from the package directory.

## Find and connect to a peripheral

Request access, start a scan, and collect advertisement events. Let the user choose a device
before connecting. Device identifiers are opaque and platform-specific; an advertised name does
not establish a device's identity.

```rust
use day_part_bluetooth::{self as ble, Event, ScanOptions};

async fn scan() -> Result<ble::Watch, ble::Error> {
    let watch = ble::watch(|event| {
        if let Event::Advertisement { advertisement } = event {
            // Send this observation to the app's device list through a Day Setter.
            let _ = advertisement;
        }
    })?;
    ble::request_access().await?;
    ble::start_scan(&ScanOptions::default()).await?;
    Ok(watch)
}
```

Keep the returned watch alive while receiving events. Stop the scan explicitly when discovery
ends; dropping a watch only removes that event subscription. Service filters in `ScanOptions`
can narrow discovery to the accessories the app understands.

After connecting, discover services to obtain characteristic UUIDs and supported operations.
Read a value once, write a command, or enable notifications for continuing updates. Disconnect
when finished. Notification values arrive as `Event::Value` through the watch.

Callbacks may run off the UI thread. Send values through a [Setter](/docs/explanation/state) and
format [localized messages](/docs/reference/localization) on the UI thread. The app supplies its
device-specific protocol and divides large messages into values the peripheral accepts.

## Test application code without a radio

A simulation intercepts ordinary Bluetooth calls. Exact expectations verify the requests and
return synthetic replies. This helper tests an application read using the global API:

```rust
use day_part_bluetooth::{self as ble, simulation::{Request, Reply, Simulation}};

async fn simulated_read() -> Result<(), ble::Error> {
    let simulation = Simulation::new();
    simulation.expect(
        Request::Read {
            device: "test-battery".into(),
            service: "180f".into(),
            characteristic: "2a19".into(),
        },
        Ok(Reply::Value(vec![73])),
    );
    let _interception = simulation.install();
    assert_eq!(ble::read("test-battery", "180f", "2a19").await?, vec![73]);
    simulation.assert_finished()?;
    Ok(())
}
```

Keep the installation guard for the entire test. Unmatched requests fail instead of reaching
hardware. Handlers can calculate replies, wait on a test clock, and emit advertisements,
disconnects, and value notifications through the normal event callbacks.

For parallel tests, create an independent `Session` for each test and attach its simulation.
Independent sessions do not inherit global overrides. A watch stays attached to the provider
selected when it was created; drop watches and disconnect before changing providers.

From the package directory:

```sh
cargo run --example mock_peripheral
cargo test --test interception
```

These tests run on hosted CI without Bluetooth permission. A separate software peripheral and
native test example exercise radio traffic between physical devices; hosted walkthroughs check
the demo's controls.

## Platforms and permissions

| Platform | Requirements and limits |
| --- | --- |
| iOS / UIKit and macOS / AppKit | CoreBluetooth. Supply a localized `NSBluetoothAlwaysUsageDescription`; sandboxed macOS apps also need the Bluetooth entitlement. |
| Android / Material | Nearby devices permissions on Android 12 and later. Scanning also needs location access unless the app declares `neverForLocation`; older versions need location permission. |
| Linux / GTK / Qt | A running BlueZ service, system D-Bus access, and a powered LE adapter. Known dual-mode Classic/LE devices are rejected until explicit LE connection selection is available. |
| Windows / WinUI | A Bluetooth LE adapter and OS permission. Packaged apps need the `bluetooth` device capability. Scan before connecting so address information is available. |
| Web / DOM and HarmonyOS / ArkUI | Native operations are unsupported. Simulations are available. |

The Android demo declares `neverForLocation`. Use that declaration only when the app does not
derive location from scan results; Android may filter some beacons. See
[Android Bluetooth permissions](https://developer.android.com/develop/connectivity/bluetooth/bt-permissions)
and Day's [native project configuration](/docs/how-to/native-projects).

This part connects to peripherals as a central. It does not expose peripheral advertising,
Classic Bluetooth profiles, or background scanning and reconnection after a process restart.
Characteristics are identified by service and characteristic UUID; duplicate instances with the
same pair are not distinguished.

Disconnects, timeouts, and adapter shutdown fail affected operations. Cancelling an active
operation closes this client's connection; reconnect before continuing. A write may already
have reached the peripheral, and OS cleanup may finish after cancellation.

For services on a local network, use [local network discovery](/docs/reference/zeroconf).
For broader permission handling, see [permissions](/docs/reference/permissions).
