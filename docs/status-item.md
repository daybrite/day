---
navTitle: "Status items and Dock"
title: "Status items, progress and the Dock"
description: "An icon in the menu bar or tray with a menu, progress on the Dock or taskbar icon, menu-bar apps, and keeping an app running with no window open."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Status items, progress and the Dock

These features put an app somewhere other than its windows: an icon in the macOS menu bar, the
Windows notification area or a Linux tray; a progress bar on the Dock or taskbar icon; and, for an
app that lives in its status item, no Dock icon at all.

## A status item

```rust
use day::prelude::*;

fn tray() {
    let paused = Signal::global(false);
    let _item = status_item("sync", move || {
        StatusItem::new()
            .vector(res::vectors::sync_mark)
            .title(move || if paused.get() { "Paused" } else { "" }.to_string())
            .tooltip(res::str::sync_tooltip())
            .menu([
                menu_item(res::str::pause_sync())
                    .checked(paused)
                    .action(move || paused.set(!paused.get())),
                menu_item(res::str::open_window()).action(|| {
                    let _ = day::open_new_window();
                }),
                menu_separator(),
                menu_role(MenuRole::Quit),
            ])
            .on_activate(|| {
                day::open_preferences();
            })
    });
}
```

[`status_item(id, build)`](https://daybrite.dev/api/day_pieces/fn.status_item.html) shows the item and rebuilds it whenever a signal `build` reads changes, so
a check mark or a timer title follows the app's state. Showing a second item with the same `id`
replaces the first; [`StatusItemHandle::remove()`](https://daybrite.dev/api/day_pieces/struct.StatusItemHandle.html#method.remove) takes it down.

On macOS, an open status menu keeps the entries and action handlers the user saw when opening
it. Reactive updates to that status item are coalesced while the menu is open, including its
title and image. After the selection dispatches (or the menu is dismissed), the latest update
is installed. This keeps polling apps from moving entries or invalidating a selected run while
the user navigates a submenu. Removing the status item explicitly cancels its pending update.

- **Icon.** [`.vector(res::vectors::…)`](https://daybrite.dev/api/day_pieces/struct.StatusItem.html#method.vector) draws a bundled glyph as a template: the shape alone, in
  the menu bar's or panel's own color, so one glyph reads in light and dark. [`.icon(Symbol::…)`](https://daybrite.dev/api/day_pieces/struct.StatusItem.html#method.icon)
  uses a platform symbol the same way. [`.image(res::images::…)`](https://daybrite.dev/api/day_pieces/struct.StatusItem.html#method.image) draws a picture as it is; add
  [`.template()`](https://daybrite.dev/api/day_pieces/struct.StatusItem.html#method.template) for a monochrome image.
- **Title.** Text beside the icon where the platform shows it: the macOS menu bar.
- **Menu.** The same entries as [`app_menu`](https://daybrite.dev/api/day_pieces/fn.app_menu.html), with the same dispatch. An app that keeps running with
  no window needs a way to quit in this menu: [`menu_role(MenuRole::Quit)`](https://daybrite.dev/api/day_pieces/fn.menu_role.html) or an item calling
  [`day::quit()`](https://daybrite.dev/api/day/fn.quit.html).
- **Click.** With [`.on_activate(f)`](https://daybrite.dev/api/day_pieces/struct.StatusItem.html#method.on_activate), a primary click runs `f` and the menu opens on a secondary
  click (right click, or the tray host's own menu gesture). Without it, any click opens the menu.

Probe [`capability(Cap::StatusItem)`](https://daybrite.dev/api/day_core/fn.capability.html) before relying on one. A phone and the web show nothing. On
Linux the item needs a running StatusNotifierItem host: KDE Plasma, Xfce, Cinnamon, MATE and
LXQt panels have one, and GNOME has one only with the AppIndicator extension (Ubuntu installs it).
Without a host the answer is `Unsupported`, so an app can keep a window instead of disappearing
into a tray nobody shows.

## Runtime status images on macOS

[`StatusItem::raster(day::StatusImage)`](https://daybrite.dev/api/day_pieces/struct.StatusItem.html#method.raster) displays pixels computed by the app, for a status grid,
meter or sparkline. [`StatusImage::rgba(width, height, scale, pixels)`](https://daybrite.dev/api/day_spec/struct.StatusImage.html#method.rgba) returns `Some` for a valid
image: top-to-bottom, tightly packed, **straight-alpha sRGB RGBA8**, with `scale` pixels per
logical point. For example, a 240×36 image at scale 2 occupies 120×18 points. Empty images,
mismatched byte counts, nonpositive/nonfinite scales, buffers over 16 MiB and logical dimensions
over 4096 points are rejected with `None`.

A raster is drawn in color by default; [`.template()`](https://daybrite.dev/api/day_pieces/struct.StatusItem.html#method.template) uses only its alpha silhouette.
AppKit preserves its aspect ratio and logical width, scaling down proportionally only when it
is taller than 18 points. Unlike bundled glyphs, a wide raster is **not squeezed into 18×18**.
The status item owns the shared source bytes, and the native image owns a copy of the pixels;
it survives closing the producing window and is released on replacement or item removal.
Unchanged images are reused when only menu entries, title or tooltip change. The tooltip is
also the native button's accessibility label, so a grid needs a localized textual description.

This extension currently renders on **AppKit only**. Keep an [`.icon(...)`](https://daybrite.dev/api/day_pieces/struct.StatusItem.html#method.icon), [`.vector(...)`](https://daybrite.dev/api/day_pieces/struct.StatusItem.html#method.vector) or
[`.image(...)`](https://daybrite.dev/api/day_pieces/struct.StatusItem.html#method.image) fallback for other desktop toolkits, which ignore [`raster`](https://daybrite.dev/api/day_pieces/struct.StatusItem.html#method.raster). Set the fallback
before [`.raster(...)`](https://daybrite.dev/api/day_pieces/struct.StatusItem.html#method.raster) so that the latter selects full-color rendering. `Cap::StatusItem` still
reports ordinary status-item support; it does not promise runtime-raster support. The mock
toolkit retains the raster in its probe for tests.

## Keeping the app running

An app with a status item keeps running when its last window closes, so its menu stays usable.
That is the default policy, `KeepRunning::Automatic`; [`day::set_keep_running`](https://daybrite.dev/api/day/fn.set_keep_running.html) changes it:

| `KeepRunning` | When the last window closes |
|---|---|
| `Automatic` | Keep running while a status item is shown; otherwise follow the platform's rule (macOS ends the app when its first window closes, the other desktops when the last one does). |
| `Always` | Keep running, status item or not: a background helper, an app reopened from the Dock or a global shortcut. |
| `Never` | End the app, on every platform. |

[`day::quit()`](https://daybrite.dev/api/day/fn.quit.html) ends the app at any time: every window is disposed and `Lifecycle::WillTerminate`
delivered first. On macOS a click on the Dock icon with no window open opens one through the
app's [`register_new_window`](https://daybrite.dev/api/day_core/windows/fn.register_new_window.html) builder.

## Menu-bar apps on macOS

A menu-bar app shows no Dock icon and lives in its status item. Declare it in Day.toml so the
icon never appears, not even for the first frame:

```toml
[app.macos]
dock = false
```

`day build` writes `LSUIElement` into the app's Info.plist, and a `day launch` run hears it too.
[`day::set_dock_visible(bool)`](https://daybrite.dev/api/day/fn.set_dock_visible.html) shows or hides the Dock icon while the app runs, for an app with a
"Show in Dock" setting (`Cap::DockVisibility`).

To launch without showing a window, also pass [`res::window_options().start_hidden()`](https://daybrite.dev/api/day_spec/struct.WindowOptions.html#method.start_hidden) to
[`day::launch`](https://daybrite.dev/api/day/fn.launch.html) (or the `options:` argument of `day_start!`). On AppKit,
the initial window is created and its root mounted, but it is never ordered front and launch
does not activate the app. Timers and status-item updates run normally. Register Preferences
with separate, default-visible options so the status menu can open Settings on demand. There
is no need to show and then close the initial host; it remains available through
[`initial_window()`](https://daybrite.dev/api/day_core/windows/fn.initial_window.html) and can be shown explicitly with [`set_visible(true)`](https://daybrite.dev/api/day_core/windows/struct.WindowHandle.html#method.set_visible).

[`start_hidden`](https://daybrite.dev/api/day_spec/struct.WindowOptions.html#method.start_hidden) also applies to secondary AppKit windows. It defaults to `false`, is currently
ignored by other toolkits, and does not change `KeepRunning` or the window's lifetime.

## Progress on the Dock or taskbar icon

```rust
day::set_app_progress(day::AppProgress::Value(0.42));
day::set_app_progress(day::AppProgress::None);
```

`AppProgress` is `None`, `Indeterminate`, `Value(f)`, `Paused(f)` or `Error(f)`, with fractions
from 0.0 to 1.0. Platforms without a paused or error state show the plain bar
(`Cap::AppProgress`). The Dock tile is a still picture, so an indeterminate bar does not animate
there. Progress is app-wide; on Windows it shows on the first window's taskbar button.

## Platform support

Platform guidance: [Apple: the menu bar](https://developer.apple.com/design/human-interface-guidelines/the-menu-bar); [Windows: notification area](https://learn.microsoft.com/en-us/windows/win32/shell/notification-area).

| Toolkit | Status item | Progress | Dock visibility |
|---|---|---|---|
| AppKit | Native: [`NSStatusItem`](https://developer.apple.com/documentation/appkit/nsstatusitem) in the menu bar | Native: a bar drawn on the Dock tile | Native: the activation policy |
| Windows | Native: `Shell_NotifyIcon` (compile-checked; runtime pending) | Native: [`ITaskbarList3`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-itaskbarlist3) (compile-checked) | Unsupported |
| GTK (Linux) | Native while a StatusNotifierItem host runs, through the `day-platform-linux` crate | Emulated: the Unity launcher protocol (KDE Plasma, Dash to Dock) | Unsupported |
| Qt (Linux) | Native where [`QSystemTrayIcon`](https://doc.qt.io/qt-6/qsystemtrayicon.html) finds a tray | Emulated: the Unity launcher protocol | Unsupported |
| iOS, Android, HarmonyOS, web | Unsupported | Unsupported | Unsupported |

On a phone the nearest equivalent of a tray icon is an ongoing notification, which is a
notifications feature ([docs/notify](notify.md)), not a status item.
