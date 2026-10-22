---
title: "App icon badge"
description: "Set app icon counts, text, and dots, with per-platform behavior and permission requirements."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# App icon badge

Show an unread count or an attention marker on the app’s icon. Badge styles and permission
requirements vary by platform.

Use `day::set_app_badge(&AppBadge)` to set or clear the app icon badge. This is separate
from a navigation item's badge. The function delegates to the running toolkit and returns `()`.

## API boundary

`AppBadge` has `None`, `Count(u32)`, `Text(String)`, and `Dot` variants. Probe
`Cap::AppBadgeCount`, `Cap::AppBadgeText`, or `Cap::AppBadgeDot` through `capability` before
choosing a payload. Unsupported payloads are ignored rather than converted to another kind.
Localize app-owned text before constructing `AppBadge::Text`.

## Platform behavior

Platform guidance: [Apple: notifications](https://developer.apple.com/design/human-interface-guidelines/notifications); [Android: notifications](https://developer.android.com/develop/ui/views/notifications).

| Target | Count | Text | Dot | Behavior |
| --- | --- | --- | --- | --- |
| macOS / AppKit | Yes | Yes | Yes | Dock tile label; cleared when the process exits |
| iOS / UIKit | Yes | No | No | Requires notification permission; the count survives app termination |
| Windows / WinUI or XAML | Yes | No | Yes | Taskbar overlay; counts above 99 display as 99+ |
| Linux / GTK or Qt | Emulated | No | No | Unity launcher D-Bus signal; the desktop shell decides whether to display it |
| Android | No | No | No | Use a local notification's badge instead |
| HarmonyOS / ArkUI | Yes | No | No | Notification Kit badge number |
| Browser / DOM | Emulated | No | Emulated | Browser Badging API; requires browser and installed-app support |

Only AppKit accepts arbitrary text. Other targets ignore `AppBadge::Text`; choose a count or
omit the badge when `Cap::AppBadgeText` is unsupported.

On iOS, request [notification permission](permissions.md) before setting a count. Clear the
badge when its information is no longer relevant. A `WillTerminate` handler can clear it during
an orderly exit, but iOS can terminate a suspended app without that callback; update it on the
next launch as well. See [lifecycle events](lifecycle.md).

Android derives launcher badges from notifications. Use `Notification::badge(n)` from
[local notifications](notify.md); the launcher determines whether a count or dot appears.

GTK and Qt on Linux emit the same Unity launcher signal. Stock GNOME Shell ignores it;
shells or extensions that implement the protocol can display it. This is why the capability
reports `Emulated` rather than promising a visible badge.

See [app progress](status-item.md) for progress indicators.
