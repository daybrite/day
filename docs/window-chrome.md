---
title: "Window chrome"
description: "Frameless and overlay title bars, transparent and material backgrounds, window placement, and drag regions for custom title bars."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Window chrome

A window opens with the platform's title bar and frame unless its `WindowOptions` say otherwise.
Four options shape the frame, and one decorator turns any piece into a title bar the user can drag:

```rust
use day::prelude::*;

day::open_window(
    Some("splash"),
    day::WindowOptions {
        size: Size::new(420.0, 240.0),
        chrome: day::WindowChrome::Frameless,           // Standard | Overlay | Frameless
        background: day::WindowBackground::Transparent, // Opaque | Transparent | Material(…)
        shadow: false,
        resizable: false,
        placement: day::WindowPlacement::Centered,      // Automatic | Centered | At(point)
        ..Default::default()
    },
    day::WindowKind::Normal,
    || {
        column((label("Loading…"), button("Close").action(close_splash)))
            .grow() // fill the window; only the rounded corners stay see-through
            .padding(24.0)
            .background(Color::rgba(0.15, 0.2, 0.35, 0.92))
            .corner_radius(24.0)
            .window_drag_region()
    },
);
```

The same fields apply to the first window through the `WindowOptions` handed to `day::launch`.
Phones and the web ignore them: a phone window has no frame to change, and a window the platform
cannot open presents as a cover inside the first one ([docs/windows.md](windows.md)), which is the
right splash on a phone.

## Chrome

| `WindowChrome` | What the window shows |
|---|---|
| `Standard` | The platform's title bar and frame. |
| `Overlay` | The content runs to the window's top edge, under the title bar; the platform's close, minimize and zoom controls stay in place. For a custom title bar. |
| `Frameless` | No title bar and no frame. The window still resizes from its edges when `resizable` is true, and still takes keyboard focus. |

An `Overlay` window reports the title bar's height as its safe-area top inset before its content
builds. Pad the content by it and let the background run underneath:

```rust
use day::prelude::*;

fn document_window() -> impl Piece {
    // The window being built; its inset is the height of the title bar over it.
    let top = day::current_window().map_or(0.0, |w| w.safe_area().top);
    column((
        row((spacer(), label("Untitled"), spacer()))
            .padding(Insets { top, ..Default::default() })
            .height(top + 40.0)
            .window_drag_region()
            .id("title-bar"),
        label("Document content"),
    ))
}
```

`WindowHandle::safe_area()` answers for that window when a binding re-runs later, for example
after a toolbar makes the title bar taller. The app-wide `day::safe_area()` answers for the window
being built, and otherwise for the first window.

## Background

| `WindowBackground` | What shows behind the content |
|---|---|
| `Opaque` | The platform's window background. |
| `Transparent` | The desktop, wherever the content leaves pixels unpainted: rounded and shaped splash screens, overlays. Turn `shadow` off for a shape that should not cast a rectangular shadow. |
| `Material(m)` | A translucent system material: `WindowMaterial::Window` (macOS window background, Windows 11 Mica), `Sidebar` (macOS sidebar, Windows 11 Mica Alt), `Transient` (macOS popover, Windows 11 Acrylic). |

## Drag regions

`.window_drag_region()` lets the user move the window by dragging the piece, as a title bar does.
The piece's own background starts a native move, with the platform's snapping and edge tiling.
Buttons, fields and other controls inside the region keep their own clicks. A double click does
what the user's title-bar setting says: zoom, minimize or nothing on macOS (System Settings ▸
Desktop & Dock), maximize on Windows and GNOME.

A region placed on a layout-only wrapper (`.padding(…)`, `.frame(…)`) applies to the view the
wrapper holds. On a phone the piece is an ordinary view.

## Placement

`WindowPlacement::Automatic` is the platform's choice: macOS cascades new windows, Windows and
Linux let the window manager decide. `Centered` centers the window on the active screen. `At(p)`
puts the frame's top-left corner at `p`, in points on the desktop (top-left origin at the primary
display's top-left corner). GTK 4 cannot place windows and Wayland does not let apps choose
positions, so there `At` behaves like `Automatic` (`Cap::WindowPosition`).

## Platform support

| Backend | Frameless | Overlay | Transparent | Material | Drag region |
|---|---|---|---|---|---|
| AppKit | Native | Native | Native | Native | Native |
| Windows XAML | Native | Native | Unsupported | Native on Windows 11 22H2+ | Native |
| GTK | Native | Native | Native on a compositing display | Unsupported | Native |
| Qt | Native | Unsupported (keeps the standard frame) | Native on a compositing display | Unsupported | Native |
| iOS, Android, HarmonyOS | Unsupported (ignored) | Unsupported | Unsupported | Unsupported | Unsupported |
| Web | Unsupported | Emulated in an installed Chromium app | Unsupported | Unsupported | Emulated in an installed Chromium app |

Probe the matching `Cap` before relying on an option: `FramelessWindow`, `OverlayTitleBar`,
`TransparentWindow`, `WindowMaterial`, `DragRegion`. An option the toolkit cannot honor falls
back to the standard frame or the opaque background.

### How each backend does it

- **AppKit.** `Frameless` is a borderless window that can still become key. `Overlay` is a full-size
  content view with a transparent, untitled title bar. Materials are an `NSVisualEffectView`
  behind the content. A drag region answers a press on its own background with
  `performWindowDragWithEvent:`, and reads the user's double-click setting
  (`AppleActionOnDoubleClick`).
- **Windows XAML.** `Frameless` keeps the caption and sizing styles and returns 0 from
  `WM_NCCALCSIZE`, so the shadow, rounded corners and Aero Snap stay. `Overlay` uses DWM's custom
  frame (`DwmExtendFrameIntoClientArea`, `DwmDefWindowProc`). Materials set
  `DWMWA_SYSTEMBACKDROP_TYPE`. Drag regions answer `HTCAPTION` from the window's hit test, which
  brings the system's own move, snap and double-click maximize; the XAML island's windows pass
  those hits through. These paths are compile-checked; the Windows runtime pass is pending.
- **GTK.** `Frameless` drops the header bar; with `shadow: false` the window is undecorated and
  Day resizes it from its edges with `gdk::Toplevel::begin_resize`. `Overlay` replaces the header
  bar with `GtkWindowControls` over the content's top band and reports the band's height as the
  inset. A drag region moves the window with `gdk::Toplevel::begin_move` once a drag passes the
  platform threshold; a double click follows `gtk-titlebar-double-click`, and a secondary click
  opens the window menu. GTK 4 cannot place windows, so `placement` is the window manager's.
- **Qt.** `Frameless` is `Qt::FramelessWindowHint`, resized from its edges with
  `QWindow::startSystemResize`. A drag region calls `QWindow::startSystemMove` once a drag passes
  `startDragDistance`, and a double click toggles maximize. Qt has no way to keep the window
  controls while removing the title bar, so `Overlay` keeps the standard frame and reports no
  inset. On Wayland the compositor places windows.
- **Web.** An installed Chromium app with a window-controls overlay honors `Overlay` (the inset
  comes from `navigator.windowControlsOverlay`) and drag regions (CSS `app-region: drag`, with
  `no-drag` on controls). Elsewhere both are inert.
