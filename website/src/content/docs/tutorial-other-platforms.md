---
title: "Optional: Try another platform"
description: Run the completed app with Qt on macOS, then choose another desktop or mobile target.
order: 7
section: Tutorials
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Run [the completed Session counter](/docs/tutorials/complete-app) with another toolkit while
keeping its Rust code, translations, and walkthrough. Qt runs on the same Mac as the AppKit
lessons, so you can compare the controls before setting up another operating system.

## Run with Qt on your Mac

Install Qt 6 and pkg-config using the [macOS setup instructions](/docs/reference/system-requirements#macos).
With Homebrew:

```sh
brew install qt pkg-config
```

Add this line to the existing `[features]` table in your app's `Cargo.toml`. Keep the existing
`mock` and `appkit` entries:

```toml
qt = ["day/qt"]
```

From the app directory, create the target files and check the tools:

```sh
day project add-target macos-qt
day doctor --toolkit qt
day launch -t macos-qt
```

The window shows the same counter using Qt controls. Add two sessions, open the summary, return,
and reset the count. Quit, then run the same automated walkthrough against this target:

```sh
day launch -t macos-qt --locale en --script dayscript/session-counter.yaml
```

A successful walkthrough exits with code 0. Check the layout at the minimum window size and
try keyboard navigation as well. `macos-qt` is a development target; the supported Qt deployment
target is Linux. See [platform support](/docs/reference/platforms#support-tiers).

## Choose another operating system

Each target needs its matching Cargo feature, host tools, and platform configuration. Follow
its setup page before adding it with `day project add-target <target>`.

| Target | Cargo feature | Setup |
| --- | --- | --- |
| `ios-uikit` | `uikit = ["day/uikit"]` | [iOS](/docs/reference/platforms/ios-uikit) |
| `android-material` | `android = ["day/android"]` | [Android](/docs/reference/platforms/android-material) |
| `linux-gtk` | `gtk = ["day/gtk"]` | [Linux GTK](/docs/reference/platforms/linux-gtk) |
| `linux-qt` | `qt = ["day/qt"]` | [Linux Qt](/docs/reference/platforms/linux-qt) |
| `windows-winui` | `winui = ["day/winui"]` | [Windows](/docs/reference/platforms/windows-winui) |
| `harmony-arkui` | `arkui = ["day/arkui"]` | [HarmonyOS, experimental](/docs/reference/platforms/harmony-arkui) |
| `web-dom` | `dom = ["day/dom"]` | [Browser, experimental](/docs/reference/platforms/web-dom) |

## Troubleshooting

If Cargo reports that the app has no `qt` feature, confirm the new entry is inside `[features]`.
If doctor cannot find Qt, check that pkg-config can locate the installed Qt libraries. Enable
only one toolkit per binary; `day launch -t` selects the matching backend feature for you.

Return to [the complete app](/docs/tutorials/complete-app) or choose a [task guide](/docs/how-to).
