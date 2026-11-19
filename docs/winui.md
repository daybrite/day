---
navTitle: "WinUI"
title: "Build and package for Windows"
description: "Run a Day app with WinUI, prepare offline build dependencies, and create a Windows package."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Build and package for [Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui")

Use [`windows-winui`](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") for [Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") apps. It uses [WinUI 3](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") from the Windows App SDK and is a
[Tier 2](/docs/reference/platforms#support-tiers) target. The older [`windows-xaml`](https://daybrite.dev/docs/reference/platforms "toolkit:windows-xaml") target is
deprecated. See the [Windows toolkit reference](/docs/reference/platforms/windows-winui) for
control support and platform behavior.

## Prepare and run

On a [Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") development host, install Rust and the C++ build tools listed in
[System requirements](/docs/reference/system-requirements#windows), then run:

```sh
day doctor --toolkit winui
day launch -t windows-winui
```

Development builds use the installed [Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") App SDK runtime. `day doctor` checks it and
reports missing requirements. Day fetches the pinned SDK packages and generates the C++/WinRT
projection needed by its native shims; applications do not need to generate this manually.

`DAY_WINAPPSDK` selects a prepared SDK directory for offline builds. `DAY_WINAPPSDK_CACHE`
changes the download cache. Keep the complete pinned package layout, including Win2D and
WebView2 dependencies when the app uses them. See [Environment](/docs/reference/environment#windows).

## Optional components

`day build -t windows-winui` enables matching toolkit features for declared optional pieces.
Consult each piece's documentation for its dependency and [Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") requirements.

[Web views](/docs/reference/webview) use WebView2. Packaged apps carry the loader libraries;
development builds can use the SDK cache. The WebView2 runtime is a separate requirement.

## Package the app

```sh
day pack -t windows-winui
```

[Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") packages include the Windows App SDK runtime beside the application. A packaged
installation therefore does not require the development machine's runtime installation.
Development builds remain framework-dependent to avoid copying that runtime on every build.

Follow [Packaging](/docs/how-to/packaging) for installer formats, signing, and store distribution.
Test the resulting package on a clean machine as well as on the development host.

## Canvas and capture

[WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") draws canvases through Win2D, supplied by the pinned SDK packages. Bitmap and PNG export
are supported; PDF export is unavailable. See [Canvas](/docs/reference/canvas).

Window capture normally uses [Windows](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") Graphics Capture, including composed content such as
WebView2 and video. Where that API is unavailable, the XAML-tree fallback omits the window
frame and separately composed content. See [Window images](/docs/reference/window-image).
