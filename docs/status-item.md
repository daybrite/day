---
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
            .title(if paused.get() { "Paused" } else { "" })
            .tooltip(res::str::sync_tooltip())
            .menu(vec![
                menu_item(res::str::pause_sync())
                    .checked(paused.get())
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

`status_item(id, build)` shows the item and rebuilds it whenever a signal `build` reads changes, so
a check mark or a timer title follows the app's state. Showing a second item with the same `id`
replaces the first; `StatusItemHandle::remove()` takes it down.

- **Icon.** `.vector(res::vectors::…)` draws a bundled glyph as a template: the shape alone, in
  the menu bar's or panel's own color, so one glyph reads in light and dark. `.icon(Symbol::…)`
  uses a platform symbol the same way. `.image(res::images::…)` draws a picture as it is; add
  `.template(true)` for a monochrome image.
- **Title.** Text beside the icon where the platform shows it: the macOS menu bar.
- **Menu.** The same entries as `app_menu`, with the same dispatch. An app that keeps running with
  no window needs a way to quit in this menu: `menu_role(MenuRole::Quit)` or an item calling
  `day::quit()`.
- **Click.** With `.on_activate(f)`, a primary click runs `f` and the menu opens on a secondary
  click (right click, or the tray host's own menu gesture). Without it, any click opens the menu.

Probe `capability(Cap::StatusItem)` before relying on one. A phone and the web show nothing. On
Linux the item needs a running StatusNotifierItem host: KDE Plasma, Xfce, Cinnamon, MATE and
LXQt panels have one, and GNOME has one only with the AppIndicator extension (Ubuntu installs it).
Without a host the answer is `Unsupported`, so an app can keep a window instead of disappearing
into a tray nobody shows.

## Runtime status images on macOS

`StatusItem::raster(day::StatusImage)` displays pixels computed by the app, for a status grid,
meter or sparkline. `StatusImage::rgba(width, height, scale, pixels)` returns `Some` for a valid
image: top-to-bottom, tightly packed, **straight-alpha sRGB RGBA8**, with `scale` pixels per
logical point. For example, a 240×36 image at scale 2 occupies 120×18 points. Empty images,
mismatched byte counts, nonpositive/nonfinite scales, buffers over 16 MiB and logical dimensions
over 4096 points are rejected with `None`.

A raster is drawn in color by default; `.template(true)` uses only its alpha silhouette.
AppKit preserves its aspect ratio and logical width, scaling down proportionally only when it
is taller than 18 points. Unlike bundled glyphs, a wide raster is **not squeezed into 18×18**.
The status item owns the shared source bytes, and the native image owns a copy of the pixels;
it survives closing the producing window and is released on replacement or item removal.
Unchanged images are reused when only menu entries, title or tooltip change. The tooltip is
also the native button's accessibility label, so a grid needs a localized textual description.

This extension currently renders on **AppKit only**. Keep an `.icon(...)`, `.vector(...)` or
`.image(...)` fallback for other desktop backends, which ignore `raster`. Set the fallback
before `.raster(...)` so that the latter selects full-color rendering. `Cap::StatusItem` still
reports ordinary status-item support; it does not promise runtime-raster support. The mock
backend retains the raster in its probe for tests.

## Keeping the app running

An app with a status item keeps running when its last window closes, so its menu stays usable.
That is the default policy, `KeepRunning::Automatic`; `day::set_keep_running` changes it:

| `KeepRunning` | When the last window closes |
|---|---|
| `Automatic` | Keep running while a status item is shown; otherwise follow the platform's rule (macOS ends the app when its first window closes, the other desktops when the last one does). |
| `Always` | Keep running, status item or not: a background helper, an app reopened from the Dock or a global shortcut. |
| `Never` | End the app, on every platform. |

`day::quit()` ends the app at any time: every window is disposed and `Lifecycle::WillTerminate`
delivered first. On macOS a click on the Dock icon with no window open opens one through the
app's `register_new_window` builder.

## Menu-bar apps on macOS

A menu-bar app shows no Dock icon and lives in its status item. Declare it in Day.toml so the
icon never appears, not even for the first frame:

```toml
[app.macos]
dock = false
```

`day build` writes `LSUIElement` into the app's Info.plist, and a `day launch` run hears it too.
`day::set_dock_visible(bool)` shows or hides the Dock icon while the app runs, for an app with a
"Show in Dock" setting (`Cap::DockVisibility`).

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

| Backend | Status item | Progress | Dock visibility |
|---|---|---|---|
| AppKit | Native: `NSStatusItem` in the menu bar | Native: a bar drawn on the Dock tile | Native: the activation policy |
| Windows XAML | Native: `Shell_NotifyIcon` (compile-checked; runtime pending) | Native: `ITaskbarList3` (compile-checked) | Unsupported |
| GTK (Linux) | Native while a StatusNotifierItem host runs, through the `day-dbus` crate | Emulated: the Unity launcher protocol (KDE Plasma, Dash to Dock) | Unsupported |
| Qt (Linux) | Native where `QSystemTrayIcon` finds a tray | Emulated: the Unity launcher protocol | Unsupported |
| iOS, Android, HarmonyOS, web | Unsupported | Unsupported | Unsupported |

On a phone the nearest equivalent of a tray icon is an ongoing notification, which is a
notifications feature ([docs/notify.md](notify.md)), not a status item.
