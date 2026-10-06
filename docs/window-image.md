---
title: "Window capture"
description: "day::window_image(): the app capturing its own window as a PNG on every platform."
---

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
requested; an app photographing its own window is not a screen recording, and none of the nine
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

By default the capture is the window's **content**, what the app itself drew. Ask for `chrome()`
to include the frame the platform draws around it:

```rust
day::window_image().chrome().capture()?
```

| | content (default) | `.chrome()` |
|---|---|---|
| macOS | the content view | plus the titlebar and window toolbar |
| iOS | the app's root view | plus the status bar |
| Linux | the content area | plus the GTK HeaderBar (client-side decorations) |
| Android | the activity's decor content | plus the status bar (same pixel size where the app draws edge-to-edge; the bar's own pixels appear, the frame does not grow) |
| Windows, Qt | the window, already including in-window chrome; see below | same image |
| HarmonyOS | the window root node | same image |

Two backends cannot separate the two. On Windows the capture is a `PrintWindow` of the top-level
`HWND`, and on Qt a `QWidget::grab` of the top-level widget: both already contain everything drawn
*inside* the window, and neither can reach the frame the window manager draws *outside* it. They
answer `chrome()` with the same image.

Nothing composited on top of the window by the system (a menu that has torn off into its own
window, an IME candidate popup, a screen-recording indicator) is part of any capture.

## Capability

`day::window_image_support()` reports whether the running backend can capture at all:

```rust
if day::window_image_support() == Support::Native { /* offer the command */ }
```

It answers `Support::Native` on eight backends and `Support::Unsupported` on **web-dom**, where a
DOM cannot rasterize itself. Gate the UI on it (the Showcase's Screenshot menu item
does) rather than offering a command that can only fail.

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

Two of these were the second thing tried:

**AppKit prefers the window server.** `cacheDisplayInRect` renders the view hierarchy the app
drew and nothing else, so macOS's composited materials (a Liquid Glass sidebar, vibrancy)
come back blank. `CGWindowListCreateImage` asks the window server for the pixels the user is
looking at. It has the opposite limitation: it has no image for a window that is not on
screen, so the offscreen render remains the fallback.

**ArkUI encodes natively.** The obvious route is the ArkTS host (that is how
day-arkui reaches the file picker and the browser), but `@ohos.multimedia.image` has no
synchronous packer at all (`packToData` and `packing` are Promise/callback only), so bridging
through the host would have forced `window_image()` to be async on **every** backend to satisfy
this one. `OH_ArkUI_GetNodeSnapshot` and `OH_ImagePackerNative_PackToDataFromPixelmap` do the same
work synchronously in-process, so the API stays sync everywhere. It costs two extra linked
libraries (`libpixelmap.so`, `libimage_packer.so`), linked by the image kit's `ohos-sys` bindings (day-arkui's `images` module).

## Relationship to dayscript screenshots

A dayscript `screenshot:` step ([agent](./agent.md), DESIGN.md §14) is a separate path with a
different goal, and it does **not** call this API directly.

- **Desktop** — the in-process capture is the real capture, and it is what a walkthrough writes.
  The Linux CI legs keep a fallback: when the engine declines, `day` reads the xvfb root window
  with ImageMagick's `import`.
- **Device and simulator** — the platform's screen capture remains the primary path
  (`simctl io screenshot`, `adb exec-out screencap`, `hdc uitest screenCap`). It photographs the
  whole screen, status bar and system chrome included, which is what the published mobile
  galleries show; an in-process capture frames the app's view tree alone and would silently
  re-crop all of them. On an Android emulator the whole screen stays free of system dialogs:
  `day devices boot --wait` and `day launch` set `hide_error_dialogs=1` and
  `immersive_mode_confirmations=confirmed`, and each capture first closes an ANR dialog, a crash
  dialog, or the "Viewing full screen" hint left on screen before those settings landed
  (`clear_system_dialogs` in `crates/day-cli/src/mobile.rs`). Android checks again after
  `screencap`, before saving any bytes. A failed/empty window probe, surviving error dialog,
  unsuccessful capture command, or non-PNG output refuses the device capture. Physical devices
  get the same checks but no automatic dismissal or settings changes. If the app-only fallback
  also fails, the screenshot step fails and no previous image at its path is retained.
  Where a mobile backend has an in-process capture it now serves as the
  **fallback**; a refusing device tool used to abandon the shot outright. Because that image is
  wanted only when the device tool refuses, the runner tells the engine not to render one
  (`in_process: false` on the step) and re-asks on the failure path: rendering and encoding a
  capture per shot only to discard it cost 819ms each on the iOS simulator, 33.6s across one
  walkthrough variant. The idle wait that makes a capture land on a settled frame happens either
  way, because the wait is the step's purpose rather than a side effect of the encoding.
- **web-dom** — the `DAY_WEB_DRIVER` browser captures the page.

`day drive` follows the same precedence, so the same screen frames the same way whichever entry
point took the picture.

### Screenshot render checkpoints

The dayscript engine flushes reactive work and waits for native transitions before asking
`Toolkit::prepare_snapshot(host, revision)` for a capture checkpoint. The revision belongs to
one ordered screenshot request and stays the same across bounded retries. It is not an image
hash or a global “everything is idle” flag. Each asynchronous window fence ignores callbacks
from superseded requests. Missing windows, render errors and timed-out checkpoints fail the
step; they do not authorize a stale capture.

| Toolkit / platform targets | Freshness boundary |
|---|---|
| ArkUI / HarmonyOS | Every ArkTS piece module's `settle` (docs/extending.md; the web view waits for four animation frames and any CSS transition in its page, capped), then an ArkTS component snapshot with `waitUntilRenderFinished: true`; a tiny disposable PixelMap confirms rendering before the full device capture |
| GTK / Linux, macOS, Windows | Selected window's frame clock `after-paint`, following a requested draw |
| Android / MDC | `registerFrameCommitCallback`; after-draw fallback for software rendering and Android before API 29 |
| DOM / web | Two animation-frame turns; the browser screenshot operation performs final capture synchronization |
| Qt / Linux, macOS, Windows | Synchronous `QWidget::render` / `grab` in the capture itself |
| AppKit / macOS | Layout, display and Core Animation flush in the window capture |
| UIKit / iOS | In-process capture uses `afterScreenUpdates: true`; external `simctl` capture keeps its existing transition check and capture semantics |
| XAML / Windows (XAML and WinUI) | Shared CompositionTarget/DwmFlush checkpoint; WinUI also selects a fresh WGC frame |

`Ready` acknowledges a platform render checkpoint; it does not claim that a vsync callback
proves physical display presentation. `OnCapture` delegates freshness to a synchronous native
capture and does not authorize skipping waits for an unrelated external capture. Pending
native checkpoints are polled between main-loop turns, with a 16 ms retry interval and the
step's normal deadline. This does not wait for perpetual animations, video, or arbitrary
network requests: scripts still assert the application state they need and retain deliberate
`pause` steps.

The reply's optional `capture_revision` lets a current runner omit Harmony's fixed screenshot
delay. Older apps retain `DAY_OHOS_SHOT_SETTLE_MS` (4 seconds by default), with a compatibility
message. Equal consecutive screenshots are valid and are never used as a stale-frame test.
Both app and CLI must be rebuilt to use the new protocol. Capture framing and the existing
app-only fallback remain unchanged. Runs report script elapsed time and split screenshot time
between engine/checkpoints (including in-process encoding) and external capture/save work.
