---
navTitle: "Platforms"
title: Platform support
description: "Compare platform support, CI coverage, packaging options, and known limitations."
order: 33
section: Reference
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day’s targets differ in testing coverage, packaging, and available features. The tables below
summarize support tiers and CI checks; the [platform notes](#per-platform-notes) describe known
limitations. Installation instructions are in [System requirements](/docs/reference/system-requirements).

## Support tiers

A support tier describes the testing and maintenance given to an operating-system/toolkit pair.
It does not measure API completeness. A development target can implement a feature fully while
still lacking release packaging or regular testing on physical hardware.

| Tier | Intended use | Testing and maintenance |
|---|---|---|
| [Tier 1 · Supported](/docs/reference/platforms#support-tiers) | Shipping applications | Highest declared maintenance priority; verify current CI and runtime evidence |
| [Tier 2 · Demi-supported](/docs/reference/platforms#support-tiers) | Shipping applications | Declared support with less testing than Tier 1 |
| [Tier 3 · Experimental](/docs/reference/platforms#support-tiers) | Evaluation and testing | Experimental; validate the required capabilities before choosing it |
| [Tier 4 · Development](/docs/reference/platforms#support-tiers) | Compatibility testing | Development combinations without release packaging |
| [Tier 5 · Deprecated](/docs/reference/platforms#support-tiers) | Existing projects only | Superseded by another target; use the replacement for new apps |

The target table below assigns each platform to a tier. Support badges throughout the
documentation link back to this section.

A target can move to a higher tier when maintainers can provide the testing and support it
requires. See [contributing to platform support](https://github.com/daybrite/day/blob/main/CONTRIBUTING.md#platform-support-tiers)
for the responsibilities, including hardware testing, bug triage, and patch review.

## Status at a glance

The table lists the checks configured in CI and the package formats the CLI produces.
Signing requires the platform's tools and credentials. Android walkthrough failures do not
fail CI; the HarmonyOS job also tolerates build and packaging failures.

| Target | Tier | UI walkthrough in CI | Package formats |
|---|---|---|---|
| `macos-appkit` | [Tier 1](/docs/reference/platforms#support-tiers) | Full | `.dmg` |
| `ios-uikit` | [Tier 1](/docs/reference/platforms#support-tiers) | Full, Simulator | `.ipa` |
| `android-material` | [Tier 1](/docs/reference/platforms#support-tiers) | Full, emulator; failures tolerated | `.apk`, `.aab` |
| `linux-gtk` | [Tier 2](/docs/reference/platforms#support-tiers) | Full, headless X | `.flatpak`, `.appimage` |
| `linux-qt` | [Tier 2](/docs/reference/platforms#support-tiers) | Full, headless X | `.flatpak`, `.appimage` |
| `windows-winui` | [Tier 2](/docs/reference/platforms#support-tiers) | Full | `.msix`, installer |
| `harmony-arkui` | [Tier 3](/docs/reference/platforms#support-tiers) | Best-effort, emulator | `.hap` |
| `web-dom` | [Tier 3](/docs/reference/platforms#support-tiers) | Full, headless Chromium | Static `dist/` |
| `macos-gtk` | [Tier 4](/docs/reference/platforms#support-tiers) | Full | None |
| `macos-qt` | [Tier 4](/docs/reference/platforms#support-tiers) | Full | None |

Check the [CI run](https://github.com/daybrite/day/actions/workflows/ci.yml) for the revision you
intend to ship. A tolerated failure does not block the remaining jobs.

Earlier project records describe additional Matrix-client usage. Those external apps and
shipping claims were not inspected in this repository-only review and are not validation evidence
for the current checkout.

### Development combinations

GTK and Qt on macOS support toolkit compatibility testing on a Mac. They do not have release
packaging, and `macos-gtk` also lacks an accessibility tree. On Windows, build `linux-gtk` and
`linux-qt` inside [WSL 2](/docs/reference/system-requirements#windows) instead.

## Per-platform notes

Each primary target has a setup guide, known limitations, and a mapping from Day pieces to native
controls, with links to the platform’s API documentation.

### macOS (`macos-appkit`) — [full page](/docs/reference/platforms/macos-appkit)
[Tier 1 · Supported](/docs/reference/platforms#support-tiers)
Day drives AppKit directly through `objc2`, with a native menu bar, dialogs, and window
management. Builds require full **Xcode 27 or newer** for the JSON5 host project.
Packaging produces a signed, notarized `.dmg` when credentials are configured
([packaging](/docs/how-to/packaging)).

### iOS (`ios-uikit`) — [full page](/docs/reference/platforms/ios-uikit)
[Tier 1 · Supported](/docs/reference/platforms#support-tiers)
Day derives a JSON5 Xcode project under `build/day/xcode/`, requiring **Xcode 27 or newer**.
A checked-in project remains an optional customization override.
Its build phase calls back into `day` for the
Rust static library, so Xcode, `day launch`, and CI all build the same way. Day-to-day
development targets the Simulator; App Store `.ipa` export exists in `day pack` and needs your
Apple credentials. Without `signing.ios` configuration, the output is an unsigned device archive
with an `-unsigned.ipa` filename. Physical-device debugging gets less use than the Simulator.

### Android (`android-material`) — [full page](/docs/reference/platforms/android-material)
[Tier 1 · Supported](/docs/reference/platforms#support-tiers)
Day renders Material Components widgets over JNI, with a generated Gradle project and the same
callback-build pattern. `day launch` installs on every connected device/emulator at once, each
with the right ABI. Accessibility annotations are partial
([details](/docs/how-to/accessibility#current-limits)), and process-death restoration is a cold
start unless your app persists its own state.

### Linux (`linux-gtk`, `linux-qt`) — full pages: [GTK](/docs/reference/platforms/linux-gtk), [Qt](/docs/reference/platforms/linux-qt)
[Tier 2 · Demi-supported](/docs/reference/platforms#support-tiers)
Day renders GTK 4 + libadwaita via `gtk4-rs`, and Qt 6 Widgets via a small compiled C++ shim.
The configured walkthrough uses a headless display. Flatpak packages both. The runtime supplies the
toolkit, so bundles stay app-sized. Pick GTK by default; pick Qt for its cross-OS accessibility bridge or for the Qt library set. The webview piece is
functional on GTK/Linux (WebKitGTK) and Qt (QtWebEngine).

### Windows (`windows-winui`) — [full page](/docs/reference/platforms/windows-winui)
[Tier 2 · Demi-supported](/docs/reference/platforms#support-tiers)
Day renders WinUI 3 controls from the Windows App SDK 2.5.1, hosted in a Win32 window through the
SDK's `DesktopWindowXamlSource`. It builds with MSVC, and its C++/WinRT shim follows the same
pattern as Qt's. Development builds need the Windows App Runtime installed; `day pack` ships the
runtime inside the `.msix` and installer, so users install nothing extra. Fewer applications have
shipped on it than on the Apple, Linux, and Android targets.

### HarmonyOS (`harmony-arkui`) — [full page](/docs/reference/platforms/harmony-arkui)
[Tier 3 · Experimental](/docs/reference/platforms#support-tiers)
The newest toolkit drives ArkUI via the NDK C API, packaged as a `.hap` by hvigor with
an ArkTS host project. The toolchain requires the OpenHarmony SDK and command-line tools, which
take the most setup steps of the supported platforms; `day doctor --toolkit harmonyos`
and the [HarmonyOS notes](/docs/how-to/harmonyos) cover the install. CI allows failures in the emulator checks.

### Web (`web-dom`) — [full page](/docs/reference/platforms/web-dom)
[Tier 3 · Experimental](/docs/reference/platforms#support-tiers)
The same Rust compiled to WebAssembly drives DOM elements (`<button>`, `<dialog>`,
`<input type="range">`) that the browser lays out and draws; the build runs through cargo and
the [`day` CLI](/docs/reference/glossary#day-cli) alone. `day build -t web-dom` emits a self-contained static `dist/` you can host
anywhere; there is no `day pack` step because `dist/` is already the artifact. It is experimental. The DOM toolkit now reports native file-dialog support and emulated list,
tree, toolbar, and app-menu capabilities. External-piece coverage varies; inspect the
[coverage matrix](/contributing/backend-contracts/coverage-matrix) and enable the piece's DOM feature where present.
Browser permissions, user-activation requirements, and accessibility behavior still need runtime
checks; neither API presence nor an emulated capability proves desktop-equivalent behavior. The
[live build](https://showcase.daybrite.dev/webapp/) is deployed by the showcase's CI.

## Cross-cutting gaps

Framework-level features that don't vary by platform but aren't done, kept here so there's one
list:

- **Animation**: partial. [`with_animation(spec, || …)`](https://daybrite.dev/api/day_core/fn.with_animation.html) ships, and five of the eight toolkits execute
  opacity, transform, and frame changes natively: AppKit, UIKit, Android, web, and ArkUI (where
  frames move instantly). On GTK, Qt, and WinUI the changes apply at commit with no animation
  (`Cap::Animation` reports unsupported), because Day never ticks its own frames for native
  widgets. An animated background *color* interpolates on UIKit and ArkUI only, and the enter/exit
  `.transition` surface is not implemented.
- **Multi-window:** [secondary windows](/docs/reference/windows) work on every toolkit — native
  windows on AppKit, GTK, Qt, WinUI, and Android, UIScenes on iPad, a multiton ability on
  HarmonyOS; iPhone and web present them as a fullscreen cover in the primary window. Probe
  `Cap::MultiWindow` to adapt chrome.
- **Semantic color tokens**: custom colors have no automatic dark-mode variant
  ([styling](/docs/explanation/styling#color-backgrounds-shape)).
- **Keyboard input:** [`Decorate::on_key`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_key) exists and has a focused-event conformance case. Menu
  accelerators remain a separate surface; delivery depends on focus and the toolkit.
- **Gestures:** [`Decorate::on_pinch`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_pinch) exists with a conformance case. Do not infer every gesture
  or target from it; inspect the decorator and toolkit implementation for the needed gesture.
- **Forms**: no validation framework; roll your own with [signals](/docs/reference/glossary#signal) and memos.
- **Hot reload**: not present; see [the tradeoffs page](/features#allow-for-the-development-workflow).

Check this list before you adopt the framework; if something you need is missing from it and
doesn't work, report it as a bug.

## Interpret compatibility evidence

`Support::Native`, `Emulated`, and `Unsupported` describe one runtime capability, not a support
tier. A source-generated coverage row records implementation branches; it cannot establish native
interaction, accessibility quality, screenshots, or release readiness. Query capabilities for the
operation you use and handle its return/error contract.

Evidence: [Toolkit and Cap](https://github.com/daybrite/day/blob/main/crates/day-spec/src/lib.rs),
[DOM capability implementation](https://github.com/daybrite/day/blob/main/crates/day-toolkit-dom/src/lib.rs),
[key and pinch decorators](https://github.com/daybrite/day/blob/main/crates/day-pieces/src/decorators.rs),
and [target registry](https://github.com/daybrite/day/blob/main/crates/day-cli/src/targets.rs).
HarmonyOS emulator acceptance belongs in CI; local validation must stop at compile/lint checks.
