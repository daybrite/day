---
navTitle: "Badges"
title: "App icon badge"
description: "Set app icon counts, text, and dots, with per-platform behavior and permission requirements."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# App icon badge

An app-icon badge draws attention to something waiting in the app, such as unread messages. It
provides a brief signal when the app's windows are out of view.

Badge styles and permission requirements vary by platform.

Use [`day::set_app_badge(&AppBadge)`](https://docs.rs/day-app/latest/day/fn.set_app_badge.html) to set or clear the app icon badge. This is separate
from a navigation item's badge. The function delegates to the running toolkit and returns `()`.

## API boundary

`AppBadge` has `None`, `Count(u32)`, `Text(String)`, and `Dot` variants. Probe
`Cap::AppBadgeCount`, `Cap::AppBadgeText`, or `Cap::AppBadgeDot` through [`capability`](https://docs.rs/day-core/latest/day_core/fn.capability.html) before
choosing a payload. Unsupported payloads are ignored rather than converted to another kind.
Localize app-owned text before constructing `AppBadge::Text`.

## Platform behavior

Platform guidance: [Apple: notifications](https://developer.apple.com/design/human-interface-guidelines/notifications); [Android: notifications](https://developer.android.com/develop/ui/views/notifications).

| Target | Count | Text | Dot | Behavior |
| --- | --- | --- | --- | --- |
| [macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") / [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | Yes | Yes | Yes | Dock tile label; cleared when the process exits |
| [iOS](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") / [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | Yes | No | No | Requires notification permission; the count survives app termination |
| [Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | Yes | No | Yes | Taskbar overlay; counts above 99 display as 99+ |
| [Linux](https://daybrite.dev/docs/reference/platforms "toolkit:linux-gtk,linux-qt") / [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") or [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | Emulated | No | No | Unity launcher D-Bus signal; the desktop shell decides whether to display it |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | No | No | No | Use a local notification's badge instead |
| [HarmonyOS](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") / [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | Yes | No | No | Notification Kit badge number |
| [Browser](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") / [DOM](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | Emulated | No | Emulated | Browser Badging API; requires browser and installed-app support |

Only [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") accepts arbitrary text. Other targets ignore `AppBadge::Text`; choose a count or
omit the badge when `Cap::AppBadgeText` is unsupported.

On [iOS](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit"), request [notification permission](permissions.md) before setting a count. Clear the
badge when its information is no longer relevant. A `WillTerminate` handler can clear it during
an orderly exit, but iOS can terminate a suspended app without that callback; update it on the
next launch as well. See [lifecycle events](lifecycle.md).

[Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") derives launcher badges from notifications. Use [`Notification::badge(n)`](https://docs.rs/day-part-local-notify/latest/day_part_local_notify/struct.Notification.html#method.badge) from
[local notifications](notify.md); the launcher determines whether a count or dot appears.

[GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") and [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") on [Linux](https://daybrite.dev/docs/reference/platforms "toolkit:linux-gtk,linux-qt") emit the same Unity launcher signal. Stock GNOME Shell ignores it;
shells or extensions that implement the protocol can display it. This is why the capability
reports `Emulated` rather than promising a visible badge.

See [app progress](status-item.md) for progress indicators.
