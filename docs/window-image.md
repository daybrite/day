<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Window image: capturing the app's own window

An app can capture its own window as a PNG:

```rust
let png: Vec<u8> = day::window_image().capture()?;
```

The call is **synchronous** and returns PNG bytes. Nothing is written to disk and no permission is
requested — an app photographing its own window is not a screen recording, and none of the nine
backends treats it as one.

Pair it with a save picker ([files](./files.md)) to let the user keep the result:

```rust
button(tr("screenshot")).action(|| day::task(async move {
    let png = match day::window_image().capture() {
        Ok(bytes) => bytes,
        Err(e) => { eprintln!("capture failed: {e}"); return; }
    };
    save_file(png)
        .suggested_name("shot.png")
        .filter("PNG", &["png"])
        .await;
}));
```

## What lands in the image

By default the capture is the window's **content** — what the app itself drew. Ask for `chrome()`
to include the frame the platform draws around it:

```rust
day::window_image().chrome().capture()?
```

| | content (default) | `.chrome()` |
|---|---|---|
| macOS | the content view | plus the titlebar and window toolbar |
| iOS | the app's root view | plus the status bar |
| Linux | the content area | plus the GTK HeaderBar (client-side decorations) |
| Android | the activity's decor content | plus the status bar (same pixel size where the app draws edge-to-edge — the bar's own pixels appear, the frame does not grow) |
| Windows, Qt | the window, already including in-window chrome — see below | same image |
| HarmonyOS | the window root node | same image |

Two backends cannot separate the two. On Windows the capture is a `PrintWindow` of the top-level
`HWND`, and on Qt a `QWidget::grab` of the top-level widget: both already contain everything drawn
*inside* the window, and neither can reach the frame the window manager draws *outside* it. They
answer `chrome()` with the same image rather than pretending to a distinction they do not have.

Nothing composited on top of the window by the system — a menu that has torn off into its own
window, an IME candidate popup, a screen-recording indicator — is part of any capture.

## Capability

`day::window_image_support()` reports whether the running backend can capture at all:

```rust
if day::window_image_support() == Support::Native { /* offer the command */ }
```

It answers `Support::Native` on eight backends and `Support::Unsupported` on **web-dom**, where a
DOM genuinely cannot rasterize itself. Gate the UI on it — the Showcase's Screenshot toolbar
button and View-menu item both do — rather than offering a command that can only fail.

`capture()` still returns `Err` for the ordinary runtime reasons even where support is `Native`:
no window on screen yet, a zero-size window, a compositor that declined.

## How each backend captures

| backend | API |
|---|---|
| macOS (AppKit) | `CGWindowListCreateImage`, cropped to the content view; `cacheDisplayInRect` into an `NSBitmapImageRep` when the window server declines |
| iOS (UIKit) | `UIGraphicsImageRenderer` + `drawViewHierarchyInRect:afterScreenUpdates:` |
| Linux (GTK) | `GtkWidgetPaintable` rendered through the window's own `GskRenderer` |
| Qt | `QWidget::grab()` |
| Windows (XAML) | `PrintWindow` with `PW_RENDERFULLCONTENT`, `BitBlt` from the screen as a fallback |
| Android | `View.draw(Canvas)` into a `Bitmap`, `Bitmap.compress(PNG)` |
| HarmonyOS (ArkUI) | `OH_ArkUI_GetNodeSnapshot` + the native image packer |
| web-dom | unsupported |

Two of these are worth knowing about, because both are the second thing tried:

**AppKit prefers the window server.** `cacheDisplayInRect` renders the view hierarchy the app
drew and nothing else, so macOS's own composited materials — a Liquid Glass sidebar, vibrancy —
come back blank. `CGWindowListCreateImage` asks the window server for the pixels the user is
actually looking at. It has the opposite limitation: it has no image for a window that is not on
screen, so the offscreen render remains the fallback.

**ArkUI encodes natively, not through ArkTS.** The obvious route is the ArkTS host — that is how
day-arkui reaches the file picker and the browser — but `@ohos.multimedia.image` has no
synchronous packer at all (`packToData` and `packing` are Promise/callback only), so bridging
through the host would have forced `window_image()` to be async on **every** backend to satisfy
this one. `OH_ArkUI_GetNodeSnapshot` and `OH_ImagePackerNative_PackToDataFromPixelmap` do the same
work synchronously in-process, so the API stays sync everywhere. It costs two extra linked
libraries (`libpixelmap.so`, `libimage_packer.so`); see day-arkui-sys's `build.rs`.

## Relationship to dayscript screenshots

A dayscript `screenshot:` step ([agent](./agent.md), DESIGN.md §14) is a separate path with a
different goal, and it does **not** simply call this API.

- **Desktop** — the in-process capture is the real capture, and it is what a walkthrough writes.
  The Linux CI legs keep a fallback: when the engine declines, `day` reads the xvfb root window
  with ImageMagick's `import`.
- **Device and simulator** — the platform's own screen capture stays the source of truth
  (`simctl io screenshot`, `adb exec-out screencap`, `hdc uitest screenCap`). It photographs the
  whole screen, status bar and system chrome included, which is what the published mobile
  galleries show; an in-process capture frames the app's view tree alone and would silently
  re-crop all of them. Where a mobile backend has an in-process capture it now serves as the
  **fallback** — a refusing device tool used to abandon the shot outright.
- **web-dom** — the `DAY_WEB_DRIVER` browser captures the page.

`day drive` follows the same precedence, so the same screen frames the same way whichever entry
point took the picture.
