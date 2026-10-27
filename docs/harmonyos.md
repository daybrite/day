---
navTitle: "HarmonyOS"
title: "Build and run on HarmonyOS"
description: "Prepare the HarmonyOS toolchain, run an app, and customize its native host."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Build and run on HarmonyOS

The `harmony-arkui` target uses ArkUI controls on HarmonyOS and OpenHarmony. It is
[Tier 3 · Experimental](/docs/reference/platforms#support-tiers). Check the
[toolkit reference](/docs/reference/platforms/harmony-arkui) for component support.

## Prepare the toolchain

Start with:

```sh
day doctor --toolkit harmonyos
```

Building requires Rust's OHOS target, `cargo-ohos`, an SDK/NDK for your host operating system,
`hvigor`, `ohpm`, and Node.js. Launching also requires `hdc` and a reachable device or emulator.
Follow [System requirements](/docs/reference/system-requirements#harmonyos) for installation
commands, SDK layouts, and emulator images.

Use an SDK built for the development host. Linux SDK binaries cannot run on macOS.
`OHOS_BASE_SDK_HOME` must point to the versioned SDK layout expected by hvigor, while
`OHOS_SDK_NATIVE` names the native component. A wrong host binary can cause `spawn ENOEXEC`;
a flat SDK layout can produce “The SDK management mode has changed.”

## Build and launch

From the app directory:

```sh
day build -t harmony-arkui
day launch -t harmony-arkui
```

Day compiles Rust, prepares the native project, packages and signs the HAP, then installs and
starts it. Launch enumerates connected `hdc` targets and includes the required architectures
in the package. It launches on each connected target; disconnect devices you do not want to use.

For a configured emulator:

```sh
day devices boot -t harmony-arkui
day launch -t harmony-arkui
```

A tablet-sized emulator viewport tests the app's wide layout, but does not change the guest
OS's device type:

```sh
day devices boot -t harmony-arkui --device tablet --orientation landscape
```

On x86_64 Linux, KVM acceleration requires read/write access to `/dev/kvm`. Without it, QEMU
uses slower software emulation. ARM macOS hosts cannot accelerate the x86_64 guest; use CI or
a suitable device for repeated acceptance runs. See
[emulator troubleshooting](/docs/how-to/troubleshooting#harmonyos-builds-rust-but-cannot-package-or-launch).

## Customize the native host

Day stages the generated project under `build/day/harmony/project/`. Open it with:

```sh
day open -t harmony-arkui
```

Edits made only in that staged directory are replaced by preparation. To retain native edits:

```sh
day project add-target harmony-arkui --persist-native-project
```

Edit the source host under `platform/harmony/`, then refresh the staged project:

```sh
day prepare -t harmony-arkui
```

Keep native module and script paths relative to the host, or use absolute paths for external
tools. Paths that climb outside the source host resolve differently after staging. Use
`[app.harmony]` for platform identity overrides and `[harmony] vendor` for AppScope vendor metadata.
The old `platform/ohos/` layout is not accepted.

For physical devices, configure signing in the retained source host so it survives preparation.
See [Packaging](/docs/how-to/packaging) for distribution and signing guidance.

## Read launch logs

Attached launches stream `hilog` for the current app process. If PID lookup fails, Day reports
that it cannot attach the log stream. The filter excludes unrelated services and separate
web-view renderer processes; use `hdc shell hilog` when those diagnostics are needed.

Do not suspend the emulator guest with `power-shell suspend`: it also suspends `hdcd`, so
further device commands cannot wake it. Restart the emulator if that occurs.

## Check the app

Use [DayScript](/docs/how-to/dayscript) for repeatable interactions and test native input on
the devices and languages the app supports. Include narrow and wide layouts, permission
prompts, and keyboard visibility. Consult each component's platform notes for limitations;
a successful build does not establish that a device interaction is supported.
