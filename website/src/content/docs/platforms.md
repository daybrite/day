---
title: Platform support
description: "Where each target stands: what's solid, what's experimental, and the known per-platform caveats."
order: 33
section: Build & ship
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

The twelve [targets](/docs/glossary#target) differ in maturity, and this page records the
differences. It reflects what runs in CI on every push and what shipping applications have
exercised.

## Support tiers

Every target sits in one of four support tiers. The tier says how much testing and maintenance that
`(OS, toolkit)` pair gets, independent of how complete its [backend](/docs/glossary#backend) is. A
Tier 4 backend may be complete; the tier says nobody ships on that combination.

| Tier | Targets | What the tier means |
|---|---|---|
| [Tier 1 · Supported](/docs/platforms#support-tiers) | `ios-uikit`, `android-mdc`, `macos-appkit` | Fully supported and tested. Every screen of the walkthrough runs on them, applications ship on them, and a regression on one holds a release. |
| [Tier 2 · Demi-supported](/docs/platforms#support-tiers) | `linux-gtk`, `linux-qt`, `windows-xaml` | They build and run the walkthrough on every push, but get fewer hands-on passes and fewer hours in shipping applications. |
| [Tier 3 · Experimental](/docs/platforms#support-tiers) | `harmony-arkui`, `web-dom` | The walkthrough runs on them, but no application has shipped on them yet. Read the per-platform notes before you commit a product to one; the gaps listed below are those known today. |
| [Tier 4 · Development](/docs/platforms#support-tiers) | `macos-gtk`, `macos-qt`, `windows-gtk`, `windows-qt` | For compatibility testing, and to show one toolkit running on several operating systems. Real applications aren't expected to ship on them: `day pack` produces no bundle, and the caveats are development caveats. |

The badge appears wherever these docs name a target's support level, and links back to this
section.

> [!TIP] Tiers move up
> A tier records the maintenance a target has today. Any target moves up
> when there are people to keep it there: someone to own the
> backend, run the [walkthrough](/docs/glossary#walkthrough) on real hardware, triage that platform's bugs, and review patches
> against it. To take one on, start a discussion;
> [CONTRIBUTING](https://github.com/daybrite/day/blob/main/CONTRIBUTING.md#platform-support-tiers)
> explains how, and what maintaining a tier commits you to.

## Status at a glance

| Target | Tier | Builds in CI | Runs full UI walkthrough in CI | Packaging | Notes |
|---|---|---|---|---|---|
| `macos-appkit` | [Tier 1](/docs/platforms#support-tiers) | ✓ | ✓ | `.dmg` | Runs the full CI walkthrough and a shipping Matrix client |
| `linux-gtk` | [Tier 2](/docs/platforms#support-tiers) | ✓ | ✓ (headless X) | `.flatpak` + `.appimage` | |
| `linux-qt` | [Tier 2](/docs/platforms#support-tiers) | ✓ | ✓ (headless X) | `.flatpak` + `.appimage` | Strongest Linux accessibility bridge |
| `ios-uikit` | [Tier 1](/docs/platforms#support-tiers) | ✓ | ✓ (Simulator) | `.ipa` | Development is Simulator-first; `day pack` builds a device `.ipa`, signed with `signing.ios` config and otherwise unsigned (`-unsigned.ipa`, for sideloading or your own signing) |
| `android-mdc` | [Tier 1](/docs/platforms#support-tiers) | ✓ | ✓ (emulator) | `.apk` + `.aab` | Emulator leg tolerates flakes; the build itself gates hard |
| `macos-gtk` | [Tier 4](/docs/platforms#support-tiers) | ✓ | ✓ | — (dev only) | Development combo; no accessibility tree (GTK a11y is Linux-only) |
| `macos-qt` | [Tier 4](/docs/platforms#support-tiers) | ✓ | ✓ | — (dev only) | Development combo |
| `windows-xaml` | [Tier 2](/docs/platforms#support-tiers) | ✓ | ✓ | `.msix` + installer | XAML Islands (system XAML), not the WinAppSDK runtime |
| `windows-qt` | [Tier 4](/docs/platforms#support-tiers) | ✓ | best-effort | — (dev only) | MSYS2 toolchain ([setup](/docs/platforms/windows-xaml#qt-and-gtk-on-a-windows-host)); marked experimental in CI. Under CI's x86-64 MinGW `ld`, external piece renderers fail to register and draw placeholders; a clang/`lld` MSYS2 environment keeps them |
| `windows-gtk` | [Tier 4](/docs/platforms#support-tiers) | ✓ | best-effort | — (dev only) | Same, plus no accessibility tree and no WebKitGTK 6 for Windows |
| `harmony-arkui` | [Tier 3](/docs/platforms#support-tiers) | ✓ | best-effort (emulator) | `.hap` | Build and packaging gate hard; the QEMU emulator leg is tolerated-flaky |
| `web-dom` | [Tier 3](/docs/platforms#support-tiers) | ✓ | ✓ (headless Chromium) | static `dist/` | Experimental; the [live build](https://showcase.daybrite.dev/webapp/) is deployed by the showcase's own CI; see the [web notes](/docs/internal/web) |

"Runs full UI walkthrough" means the showcase app executes its complete
[dayscript](/docs/dayscript) walkthrough (navigation, inputs, dialogs, screenshots) on that
target on every push, with the captures feeding the [gallery](/gallery).

Beyond CI, a Matrix chat client (login, encrypted rooms, live timeline, media) built on Day runs its
full checklist on `macos-appkit`, `macos-gtk`, `macos-qt`, `ios-uikit` (Simulator), and
`android-mdc`.

[Tier 4 · Development](/docs/platforms#support-tiers) The GTK/Qt-on-macOS/Windows combos exist so
one development machine can run five desktop [toolkits](/docs/glossary#toolkit), and because some
teams standardize on Qt across Linux and Windows. Packaging for them is deferred, and
`macos-gtk`/`windows-gtk` have no accessibility tree.

## Per-platform notes

Each of the eight primary targets has its own page: how to get set up, the caveats that only apply
there, and a table of which native control every Day piece becomes, linked to the platform vendor's
own reference.

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
Apple credentials. Physical-device debugging gets less use than the Simulator.

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

### Windows (`windows-xaml`) — [full page](/docs/platforms/windows-xaml)
[Tier 2 · Demi-supported](/docs/platforms#support-tiers)
Day hosts XAML through XAML Islands, using the XAML stack that ships with Windows 10/11 itself
rather than the WinAppSDK runtime, so there's no runtime bootstrap to install. It builds with
MSVC, and its C++/WinRT shim follows the same pattern as Qt's. This target builds and walks
through in CI but fewer applications have shipped on it than on the Apple, Linux, and Android targets.

### HarmonyOS (`harmony-arkui`) — [full page](/docs/platforms/harmony-arkui)
[Tier 3 · Experimental](/docs/platforms#support-tiers)
The newest backend drives ArkUI via the NDK C API, packaged as a `.hap` by hvigor with
an ArkTS host project. The toolchain requires the OpenHarmony SDK and command-line tools, which
take the most setup steps of the supported platforms; `day doctor --toolkit harmonyos`
and the [HarmonyOS notes](/docs/internal/harmonyos) cover the install. Emulator behavior in
CI is tolerated-flaky.

### Web (`web-dom`) — [full page](/docs/platforms/web-dom)
[Tier 3 · Experimental](/docs/platforms#support-tiers)
The same Rust compiled to WebAssembly drives DOM elements (`<button>`, `<dialog>`,
`<input type="range">`) that the browser lays out and draws; the build runs through cargo and
the [`day` CLI](/docs/glossary#day-cli) alone. `day build -p web-dom` emits a self-contained static `dist/` you can host
anywhere; there is no `day pack` step because `dist/` is already the artifact. It is experimental: most external pieces (web view, map, Lottie, pickers, search field) render
[placeholders](/docs/glossary#placeholder), there are no file dialogs or context menus, the list is emulated rather than
recycled, and accessibility is thinner than on native because pieces that realize as `<div>`s
carry no compensating ARIA roles. The
[live build](https://showcase.daybrite.dev/webapp/) is deployed by the showcase's own CI.

## Cross-cutting gaps

Framework-level features that don't vary by platform but aren't done, kept here so there's one
list:

- **Animation**: partial. `with_animation(spec, || …)` ships, and four of the eight backends execute
  opacity, transform, and frame changes natively: AppKit, UIKit, Android, and web. On GTK, Qt, XAML,
  and ArkUI the changes apply at commit with no animation (`Cap::Animation` reports unsupported),
  because Day never ticks its own frames for native widgets. An animated background *color*
  interpolates on UIKit only, and the enter/exit `.transition` surface is not implemented.
- **Multi-window:** [secondary windows](/docs/internal/windows) work on every backend — native
  windows on AppKit, GTK, Qt, XAML, and Android, UIScenes on iPad, a multiton ability on
  HarmonyOS; iPhone and web present them as a fullscreen cover in the primary window. Probe
  `Cap::MultiWindow` to adapt chrome.
- **Semantic color tokens**: custom colors have no automatic dark-mode variant
  ([styling](/docs/styling#color-backgrounds-shape)).
- **Keyboard shortcuts** beyond native menu accelerators; no general key-event API.
- **Gestures**: tap and drag are wired; pinch, rotation, and long-press are not.
- **Forms**: no validation framework; roll your own with [signals](/docs/glossary#signal) and memos.
- **Hot reload**: not present; see [the tradeoffs page](/docs/benefits#what-you-give-up).

Check this list before you adopt the framework; if something you need is missing from it and
doesn't work, report it as a bug.
