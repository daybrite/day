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

## Run with [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") on your Mac

Install [Qt 6](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") and pkg-config using the [macOS setup instructions](/docs/reference/system-requirements#macos).
With Homebrew:

```sh
brew install qt pkg-config
```

Add this line to the existing `[features]` table in your app's `Cargo.toml`. Keep the existing
`mock` and [`appkit`](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") entries:

```toml
qt = ["day/qt"]
```

From the app directory, create the target files and check the tools:

```sh
day project add-target macos-qt
day doctor --toolkit qt
day launch -t macos-qt
```

The window shows the same counter using [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") controls. Add two sessions, open the summary, return,
and reset the count. Quit, then run the same automated walkthrough against this target:

```sh
day launch -t macos-qt --locale en --script dayscript/session-counter.yaml
```

A successful walkthrough exits with code 0. Check the layout at the minimum window size and
try keyboard navigation as well. [`macos-qt`](https://daybrite.dev/docs/reference/platforms "toolkit:macos-qt") is a development target; the supported [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") deployment
target is [Linux](https://daybrite.dev/docs/reference/platforms "toolkit:linux-gtk,linux-qt"). See [platform support](/docs/reference/platforms#support-tiers).

## Choose another operating system

Each target needs its matching Cargo feature, host tools, and platform configuration. Follow
its setup page before adding it with `day project add-target <target>`.

| Target | Cargo feature | Setup |
| --- | --- | --- |
| [`ios-uikit`](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | `uikit = ["day/uikit"]` | [iOS](/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") |
| [`android-material`](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | `android = ["day/android"]` | [Android](/docs/reference/platforms/android-material "toolkit:android-material") |
| [`linux-gtk`](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | `gtk = ["day/gtk"]` | [Linux GTK](/docs/reference/platforms/linux-gtk "toolkit:linux-gtk,linux-qt") |
| [`linux-qt`](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | `qt = ["day/qt"]` | [Linux Qt](/docs/reference/platforms/linux-qt "toolkit:linux-gtk,linux-qt") |
| [`windows-winui`](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | `winui = ["day/winui"]` | [Windows](/docs/reference/platforms/windows-winui "toolkit:windows-winui") |
| [`harmony-arkui`](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | `arkui = ["day/arkui"]` | [HarmonyOS, experimental](/docs/reference/platforms/harmony-arkui) |
| [`web-dom`](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | `dom = ["day/dom"]` | [Browser, experimental](/docs/reference/platforms/web-dom) |

## Troubleshooting

If Cargo reports that the app has no `qt` feature, confirm the new entry is inside `[features]`.
If doctor cannot find Qt, check that pkg-config can locate the installed Qt libraries. Enable
only one toolkit per binary; `day launch -t` selects the matching backend feature for you.

Return to [the complete app](/docs/tutorials/complete-app) or choose a [task guide](/docs/how-to).
