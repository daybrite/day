---
title: Platform support
description: "Compare platform support, CI coverage, packaging options, and known limitations."
order: 33
section: Build & ship
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day’s targets differ in testing coverage, packaging, and available features. The tables below
summarize support tiers and CI checks; the [platform notes](#per-platform-notes) describe known
limitations. Installation instructions are in [System requirements](/docs/system-requirements).

## Support tiers

A support tier describes the testing and maintenance given to an operating-system/toolkit pair.
It does not measure API completeness. A development target can implement a feature fully while
still lacking release packaging or regular testing on physical hardware.

| Tier | Intended use | Testing and maintenance |
|---|---|---|
| [Tier 1 · Supported](/docs/platforms#support-tiers) | Shipping applications | Full walkthrough coverage; regressions block releases |
| [Tier 2 · Demi-supported](/docs/platforms#support-tiers) | Shipping applications | CI coverage, with less manual testing and production use |
| [Tier 3 · Experimental](/docs/platforms#support-tiers) | Evaluation and testing | Walkthrough coverage, but no shipping applications yet |
| [Tier 4 · Development](/docs/platforms#support-tiers) | Compatibility testing | Development combinations without release packaging |
| [Tier 5 · Deprecated](/docs/platforms#support-tiers) | Existing projects only | Superseded by another target; still builds, packs and runs, but not for new apps, and due for removal |

The target table below assigns each platform to a tier. Support badges throughout the
documentation link back to this section.

A target can move to a higher tier when maintainers can provide the testing and support it
requires. See [contributing to platform support](https://github.com/daybrite/day/blob/main/CONTRIBUTING.md#platform-support-tiers)
for the responsibilities, including hardware testing, bug triage, and patch review.

## Status at a glance

All targets below build in CI. The walkthrough column describes how CI runs the Showcase UI
tests, including navigation, input, dialogs, and screenshots. Captures appear in the
[gallery](/gallery).

| Target | Tier | UI walkthrough in CI | Package formats |
|---|---|---|---|
| `macos-appkit` | [Tier 1](/docs/platforms#support-tiers) | Full | `.dmg` |
| `ios-uikit` | [Tier 1](/docs/platforms#support-tiers) | Full, Simulator | `.ipa` |
| `android-mdc` | [Tier 1](/docs/platforms#support-tiers) | Full, emulator; failures tolerated | `.apk`, `.aab` |
| `linux-gtk` | [Tier 2](/docs/platforms#support-tiers) | Full, headless X | `.flatpak`, `.appimage` |
| `linux-qt` | [Tier 2](/docs/platforms#support-tiers) | Full, headless X | `.flatpak`, `.appimage` |
| `windows-winui` | [Tier 2](/docs/platforms#support-tiers) | Full | `.msix`, installer |
| `harmony-arkui` | [Tier 3](/docs/platforms#support-tiers) | Best-effort, emulator | `.hap` |
| `web-dom` | [Tier 3](/docs/platforms#support-tiers) | Full, headless Chromium | Static `dist/` |
| `macos-gtk` | [Tier 4](/docs/platforms#support-tiers) | Full | None |
| `macos-qt` | [Tier 4](/docs/platforms#support-tiers) | Full | None |
| `windows-gtk` | [Tier 4](/docs/platforms#support-tiers) | Best-effort | None |
| `windows-qt` | [Tier 4](/docs/platforms#support-tiers) | Best-effort | None |
| `windows-xaml` (deprecated) | [Tier 5](/docs/platforms#support-tiers) | Full | `.msix`, installer |

Android emulator failures do not block CI, but build failures do. HarmonyOS build and packaging
failures block CI; its QEMU emulator checks tolerate failures. Windows GTK and Qt jobs are marked
experimental in CI. These exceptions are worth considering alongside a target’s support tier.

Outside CI, a Day-based Matrix client exercises login, encrypted rooms, timelines, and media on
`macos-appkit`, `macos-gtk`, `macos-qt`, `ios-uikit` in the Simulator, and `android-mdc`.
The macOS AppKit target also runs a shipping Matrix client.

### Development combinations

GTK and Qt on macOS and Windows support toolkit compatibility testing on a development machine.
They do not have release packaging. The GTK combinations also lack an accessibility tree.

Windows GTK and Qt use MSYS2. Under CI’s x86-64 MinGW linker, external piece renderers fail to
register and appear as placeholders; an MSYS2 environment using Clang and `lld` retains them.
Windows GTK also lacks WebKitGTK 6. See the
[Windows toolkit setup](/docs/platforms/windows-winui#qt-and-gtk-on-a-windows-host) for details.

## Per-platform notes

Each primary target has a setup guide, known limitations, and a mapping from Day pieces to native
controls, with links to the platform’s API documentation.

### macOS (`macos-appkit`) — [full page](/docs/platforms/macos-appkit)
[Tier 1 · Supported](/docs/platforms#support-tiers)
Day drives AppKit directly through `objc2`, with a native menu bar, dialogs, and window
management. Packaging produces a signed, notarized `.dmg` when credentials are configured
([packaging](/docs/packaging)).

### iOS (`ios-uikit`) — [full page](/docs/platforms/ios-uikit)
[Tier 1 · Supported](/docs/platforms#support-tiers)
The scaffold is a checked-in Xcode project whose build phase calls back into `day` for the
Rust static library, so Xcode, `day launch`, and CI all build the same way. Day-to-day
development targets the Simulator; App Store `.ipa` export exists in `day pack` and needs your
Apple credentials. Without `signing.ios` configuration, the output is an unsigned device archive
with an `-unsigned.ipa` filename. Physical-device debugging gets less use than the Simulator.

### Android (`android-mdc`) — [full page](/docs/platforms/android-mdc)
[Tier 1 · Supported](/docs/platforms#support-tiers)
Day renders Material Components widgets over JNI, with a checked-in Gradle project and the same
callback-build pattern. `day launch` installs on every connected device/emulator at once, each
with the right ABI. Accessibility annotations are partial
([details](/docs/accessibility#current-limits)), and process-death restoration is a cold
start unless your app persists its own state.

### Linux (`linux-gtk`, `linux-qt`) — full pages: [GTK](/docs/platforms/linux-gtk), [Qt](/docs/platforms/linux-qt)
[Tier 2 · Demi-supported](/docs/platforms#support-tiers)
Day renders GTK 4 + libadwaita via `gtk4-rs`, and Qt 6 Widgets via a small compiled C++ shim.
Both run the full walkthrough headlessly in CI. Flatpak packages both. The runtime supplies the
toolkit, so bundles stay app-sized. Pick GTK by default; pick Qt for its cross-OS accessibility bridge or for the Qt library set. The webview piece is
functional on GTK/Linux (WebKitGTK) and Qt (QtWebEngine).

### Windows (`windows-winui`) — [full page](/docs/platforms/windows-winui)
[Tier 2 · Demi-supported](/docs/platforms#support-tiers)
Day renders WinUI 3 controls from the Windows App SDK 2.5.1, hosted in a Win32 window through the
SDK's `DesktopWindowXamlSource`. It builds with MSVC, and its C++/WinRT shim follows the same
pattern as Qt's. Development builds need the Windows App Runtime installed; `day pack` ships the
runtime inside the `.msix` and installer, so users install nothing extra. Fewer applications have
shipped on it than on the Apple, Linux, and Android targets.

### Windows, system XAML (`windows-xaml`) — [full page](/docs/platforms/windows-xaml)
[Tier 5 · Deprecated](/docs/platforms#support-tiers)
The same XAML backend built against the system XAML that ships with Windows, hosted through XAML
Islands. It still builds, packs and runs for existing projects, but new apps should use
`windows-winui`; move a project with `day project add-target windows-winui`.

### HarmonyOS (`harmony-arkui`) — [full page](/docs/platforms/harmony-arkui)
[Tier 3 · Experimental](/docs/platforms#support-tiers)
The newest backend drives ArkUI via the NDK C API, packaged as a `.hap` by hvigor with
an ArkTS host project. The toolchain requires the OpenHarmony SDK and command-line tools, which
take the most setup steps of the supported platforms; `day doctor --toolkit harmonyos`
and the [HarmonyOS notes](/docs/internal/harmonyos) cover the install. CI allows failures in the emulator checks.

### Web (`web-dom`) — [full page](/docs/platforms/web-dom)
[Tier 3 · Experimental](/docs/platforms#support-tiers)
The same Rust compiled to WebAssembly drives DOM elements (`<button>`, `<dialog>`,
`<input type="range">`) that the browser lays out and draws; the build runs through cargo and
the [`day` CLI](/docs/glossary#day-cli) alone. `day build -p web-dom` emits a self-contained static `dist/` you can host
anywhere; there is no `day pack` step because `dist/` is already the artifact. It is experimental: most external pieces (web view, map, Lottie, pickers, search field) render
[placeholders](/docs/glossary#placeholder), there are no file dialogs or context menus, the list is emulated rather than
recycled, and accessibility is thinner than on native because pieces that realize as `<div>`s
carry no compensating ARIA roles. The
[live build](https://showcase.daybrite.dev/webapp/) is deployed by the showcase's CI.

## Cross-cutting gaps

Framework-level features that don't vary by platform but aren't done, kept here so there's one
list:

- **Animation**: partial. `with_animation(spec, || …)` ships, and five of the eight backends execute
  opacity, transform, and frame changes natively: AppKit, UIKit, Android, web, and ArkUI (where
  frames move instantly). On GTK, Qt, and XAML the changes apply at commit with no animation
  (`Cap::Animation` reports unsupported), because Day never ticks its own frames for native
  widgets. An animated background *color* interpolates on UIKit and ArkUI only, and the enter/exit
  `.transition` surface is not implemented.
- **Multi-window:** [secondary windows](/docs/internal/windows) work on every backend — native
  windows on AppKit, GTK, Qt, XAML, and Android, UIScenes on iPad, a multiton ability on
  HarmonyOS; iPhone and web present them as a fullscreen cover in the primary window. Probe
  `Cap::MultiWindow` to adapt chrome.
- **Semantic color tokens**: custom colors have no automatic dark-mode variant
  ([styling](/docs/styling#color-backgrounds-shape)).
- **Keyboard shortcuts** beyond native menu accelerators; no general key-event API.
- **Gestures**: tap and drag are wired; pinch, rotation, and long-press are not.
- **Forms**: no validation framework; roll your own with [signals](/docs/glossary#signal) and memos.
- **Hot reload**: not present; see [the tradeoffs page](/docs/benefits#allow-for-the-development-workflow).

Check this list before you adopt the framework; if something you need is missing from it and
doesn't work, report it as a bug.
