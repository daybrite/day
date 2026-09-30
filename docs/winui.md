---
title: "WinUI 3 (windows-winui)"
description: "The windows-winui target: the XAML backend built against WinUI 3 and the Windows App SDK."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# WinUI 3 (`windows-winui`)

> **Status: Tier 2, the Windows target.** Day-Showcase's walkthrough passes on it, with every
> piece that has a native XAML half built for WinUI too (see [Pieces](#pieces)). `day new` gives a
> Windows project this target; `windows-xaml` (system XAML) is deprecated (Tier 5).

`windows-winui` is Day's XAML backend compiled against WinUI 3 (`Microsoft.UI.Xaml`, shipped in
the Windows App SDK) instead of system XAML (`Windows.UI.Xaml`, part of Windows), which the
deprecated `windows-xaml` target still builds against.
The two stacks expose nearly the same control set under two namespaces, so there is one
backend and one shim, not two:

- `day` feature `winui` implies `xaml` and turns on `day-xaml/winui` → `day-xaml-sys/winui`.
  Everything written for the XAML backend in Rust applies unchanged; `day::toolkit_name()`
  reports `"WinUI"`.
- `day-xaml-sys/src/shim.cpp` is written against namespace aliases (`WUX`, `WUXC`, …) that point
  at `Windows::UI::Xaml` or, under `DAY_WINUI`, `Microsoft::UI::Xaml`. What genuinely differs
  sits behind `#ifdef DAY_WINUI` where it happens:
  - **Bootstrapping.** An unpackaged app puts the Windows App SDK framework package on its
    package graph with the bootstrapper (`MddBootstrapInitialize2`, release 2.5: the one
    `Microsoft.WindowsAppRuntime.2` framework package, at 2.5.1 or newer) before any
    `Microsoft.UI` type activates.
  - **Hosting.** WinUI's own `DesktopWindowXamlSource` in the same Win32 host window:
    `Initialize(WindowId)` and a `DesktopChildSiteBridge` instead of the system island's
    `IDesktopWindowXamlSourceNative`. The bridge's child HWND is what the rest of the shim
    focuses, as it does the system island's.
  - **Dispatch.** A `Microsoft.UI.Dispatching.DispatcherQueue` on the UI thread (the system one
    is kept too), and `ContentPreTranslateMessage` in the message loop for keyboard routing.
  - **Resources.** The Application merges `XamlControlsResources` and answers for the controls'
    types through `XamlControlsXamlMetaDataProvider`: WinUI ships its templates as a dictionary,
    and there is no XAML compiler here to generate a provider.
  - **Capture.** Screenshots read the window back from DWM with Windows.Graphics.Capture
    (`IGraphicsCaptureItemInterop::CreateForWindow`, one frame from a free-threaded frame pool),
    so they show the window frame, the XAML content and what WinUI composes outside the tree
    (WebView2, video), and they work while the window is covered. GDI cannot do this:
    `PrintWindow` (even with `PW_RENDERFULLCONTENT`) and a screen read return the host's empty
    client area under WinUI's compositor. The cursor is left out, and the yellow "being
    captured" outline Windows may draw is drawn on screen, never into the frame. Where WGC is
    unavailable (before Windows 10 1903, or a session without DWM), the capture falls back to
    rendering the XAML tree with `RenderTargetBitmap`, which has no frame or WebView content.
    With the display off (the power plan's idle timeout; a script's steps are not user input)
    DWM stops composing and every capture reads a blank or stale window while the steps still
    pass, so a capture first holds the display on (`SetThreadExecutionState`) and, when the
    display's power notification says it is off, wakes it with a zero-distance mouse move and
    waits for it to come back before reading. Both XAML stacks share this.

## Building

Nothing from the Windows App SDK is linked. The shim loads the bootstrapper, and the one flat
export it needs (`ContentPreTranslateMessage`, in `Microsoft.UI.Windowing.Core.dll`) once the
runtime is on the package graph, at run time: from next to the exe, else from the SDK cache the
build used, whose path is compiled in. A static import of either would fail to load before the
bootstrapper had run.

The build inputs come from `day_toolchain::winappsdk`: the pinned NuGet packages (Foundation,
WinUI, InteractiveExperiences, and WebView2, whose metadata `Microsoft.UI.Xaml`'s references),
fetched with the in-box `curl`/`tar` into `%LOCALAPPDATA%\day\winappsdk` (or the NuGet global
cache's copy), and a C++/WinRT projection generated from their metadata by the `cppwinrt.exe`
of the same Windows SDK whose `base.h` the shim compiles with. `DAY_WINAPPSDK` points at an
already laid-out copy for an offline machine; `DAY_WINAPPSDK_CACHE` moves the cache.

The pins are the component versions the `Microsoft.WindowsAppSDK` 2.5.1 metapackage names
(Foundation 2.3.12, WinUI 2.3.9, InteractiveExperiences 2.1.9, WebView2 1.0.3719.77); from 2.0
on each component is versioned on its own, so the release is `RUNTIME_VERSION`, not any one
package's version. The projection is cached per Windows SDK and WinUI version.

At run time the Windows App SDK 2 runtime (2.5.1 or newer) must be installed; `day doctor`
checks for it, and
the bootstrapper offers its download page when it is missing.

## Pieces

A piece's XAML half is built for WinUI the same way the backend is: one shim, compiled against
whichever stack the build targets.

- **Cargo.** A `winui` feature that implies the piece's `xaml` feature and turns on
  `day-xaml-sys/winui` (or forwards to the piece it wraps), with `"winui"` in
  `[package.metadata.day.piece] backends`. The Rust half is the `xaml` half, unchanged, and
  `day build -p windows-winui` switches `<piece>/winui` on like any backend feature.
- **build.rs.** `.includes(day_toolchain::winappsdk::shim_includes(&cppwinrt))` where the shim used
  to add the SDK's cppwinrt directory, and `DAY_WINUI` defined when
  `day_toolchain::winappsdk::shim_is_winui()`.
- **The shim.** Its `winrt/Windows.UI.Xaml*.h` includes switch on `DAY_WINUI`, and its namespace
  aliases go through `DAY_XAML_NS`. What else differs is small: the text editor's document API is
  `Microsoft.UI.Text`, and the activity ring needs a size floor (WinUI's template measures to
  nothing before it loads).

The web view is the one piece with a different engine host. System XAML has no usable web view in
an island, so its half hosts WebView2 by hand (a composition controller rendering into a visual
spliced into the tree, with pointer input forwarded). WinUI 3 has a WebView2 control that does
all of that, so the WinUI half creates the control, then drives the same `ICoreWebView2` the
system-XAML half does, reached through `ICoreWebView2Interop2`: navigation, inline assets,
evaluation and link policy are shared code. The control needs `WebView2Loader.dll` and
`Microsoft.Web.WebView2.Core.dll`, which are the app's to supply rather than part of the Windows App
SDK: a packed app carries both, and a development build preloads them from the WebView2 package.

**dayscript.** A step gate naming the XAML backend (`only_on: [xaml]`, `skip_on: [windows-xaml]`)
applies to `windows-winui` too, since it is that backend; `winui` / `windows-winui` single the
WinUI build out.

## Packing

`day pack -p windows-winui` ships the app **self-contained**: the Windows App SDK runtime
travels inside the package, so neither the `.msix` nor the installer needs it installed. This is
what `WindowsAppSDKSelfContained=true` does in Microsoft's MSBuild targets
(`Microsoft.WindowsAppSDK.SelfContained.targets`, in the SDK's Base package), done for a cargo
build:

1. **The runtime beside the exe.** Each component package's
   `runtimes-framework\win-<arch>\native` payload (DLLs, `.pri` resource indexes, compiled XAML,
   localized `.mui` strings) and its metadata are staged into the payload root
   (`day_toolchain::winappsdk::self_contained_payload`): Foundation, WinUI and
   InteractiveExperiences.
2. **Registration-free activation.** The exe is linked with a second manifest input
   (`self_contained_manifest`) that registers every WinRT class those DLLs implement
   (`<winrtv1:activatableClass>`) and their proxy stubs, generated from each package's
   `package.appxfragment` the way the SDK's `GenerateAppManifestFromAppxFragments` task does.
3. **Start-up.** The shim finds `Microsoft.WindowsAppRuntime.dll` beside the exe, loads it (the
   SDK's own self-contained initializer), and skips the bootstrapper.

Only pack links this way. An exe carrying those registrations activates the classes from its own
folder and cannot fall back to an installed runtime, so `day build` and `day launch` stay
framework-dependent (bootstrapped), which is also what keeps a development build from copying the
runtime on every link.

## Follow-ups

- `day patch --local` finds only workspace members, so a crate outside the checkout's workspace
  (day-piece-lottie's `gallery`) has to be added to the patch table by hand.
- A framework-dependent `.msix` option (a `PackageDependency` on the 2.x runtime, which the Store
  installs), for a much smaller Store package than the self-contained one.
- Trimming the self-contained payload: the `.mui` string tables for every Windows display
  language ship today, whether the app is localized into them or not.
- CI: the `windows-winui` legs (the scaffold check and toolkit row in ci.yml, the showcase
  gallery through daybrite/actions, whose `setup-day-deps` installs the Windows App Runtime and
  caches the SDK packages) are written but have not run on GitHub yet.
