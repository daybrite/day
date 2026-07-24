---
title: Platform support
description: "Where each target actually stands: what's solid, what's experimental, and the known per-platform caveats."
order: 33
section: Build & ship
---

Not all eleven targets are equally mature, and this page exists so you don't have to infer the
differences from bug trackers. It reflects what runs in CI on every push and what has been
exercised by real applications, and it gets updated when reality changes.

## Status at a glance

| Target | Builds in CI | Runs full UI walkthrough in CI | Packaging | Notes |
|---|---|---|---|---|
| `macos-appkit` | ✓ | ✓ | `.dmg` | The most exercised target |
| `linux-gtk` | ✓ | ✓ (headless X) | `.flatpak` | |
| `linux-qt` | ✓ | ✓ (offscreen) | `.flatpak` | Strongest Linux accessibility bridge |
| `ios-uikit` | ✓ | ✓ (Simulator) | `.ipa` / sim-app | Development is Simulator-first; device builds go through `day pack` with signing |
| `android-mdc` | ✓ | ✓ (emulator) | `.apk` + `.aab` | Emulator leg tolerates flakes; the build itself gates hard |
| `macos-gtk` | ✓ | ✓ | — (dev only) | Development combo; no accessibility tree (GTK a11y is Linux-only) |
| `macos-qt` | ✓ | ✓ | — (dev only) | Development combo |
| `windows-winui` | ✓ | ✓ | `.msix` + installer | XAML Islands (system XAML), not the WinAppSDK runtime |
| `windows-qt` | ✓ | best-effort | — (dev only) | MSYS2/MinGW toolchain; marked experimental in CI |
| `windows-gtk` | ✓ | best-effort | — (dev only) | Same |
| `ohos-arkui` | ✓ | best-effort (emulator) | `.hap` | Build and packaging gate hard; the QEMU emulator leg is tolerated-flaky |

"Runs full UI walkthrough" means the showcase app executes its complete
[dayscript](/docs/dayscript) walkthrough — navigation, inputs, dialogs, screenshots — on that
target on every push, with the captures feeding the [gallery](/gallery).

Beyond CI, the strongest evidence for the first five rows is a real application: a Matrix chat
client (login, encrypted rooms, live timeline, media) built on Day runs its full checklist on
`macos-appkit`, `macos-gtk`, `macos-qt`, `ios-uikit` (Simulator), and `android-mdc`.

The GTK/Qt-on-macOS/Windows combos deserve a plain statement: they exist because having five
desktop toolkits runnable on one development machine is enormously useful, and because some teams
standardize on Qt across Linux and Windows. They are not first-class shipping targets —
packaging for them is deliberately deferred, and `macos-gtk`/`windows-gtk` have no accessibility
tree.

A `web-html` backend (DOM as the toolkit) is designed but not part of the current target set.

## Per-platform notes

### macOS (`macos-appkit`)
AppKit via `objc2`, no shim layer. Native menu bar, dialogs, and window management. Packaging
produces a signed, notarized `.dmg` when credentials are configured
([packaging](/docs/packaging)).

### iOS (`ios-uikit`)
The scaffold is a real, checked-in Xcode project whose build phase calls back into `day` for the
Rust static library — so Xcode, `day launch`, and CI all build the same way. Day-to-day
development targets the Simulator; App Store `.ipa` export exists in `day pack` and needs your
Apple credentials. Physical-device debugging workflows are still young compared to Simulator use.

### Android (`android-mdc`)
Material Components widgets over JNI, with a checked-in Gradle project and the same
callback-build pattern. `day launch` installs on every connected device/emulator at once, each
with the right ABI. Known rough edges: accessibility annotations are partial
([details](/docs/accessibility#current-limits-plainly)), and process-death restoration is a cold
start unless your app persists its own state.

### Linux (`linux-gtk`, `linux-qt`)
GTK 4 + libadwaita via `gtk4-rs`; Qt 6 Widgets via a small compiled C++ shim. Both run the full
walkthrough headlessly in CI. Flatpak is the packaging story for both — the runtime supplies the
toolkit, so bundles stay app-sized. GTK is the default recommendation; Qt matters when its
cross-OS accessibility bridge or ecosystem is the deciding factor. The webview piece is
functional on GTK/Linux (WebKitGTK) and Qt (QtWebEngine).

### Windows (`windows-winui`)
WinUI through XAML Islands — the XAML stack that ships with Windows 10/11 itself, not the
WinAppSDK runtime, so there's no runtime bootstrap to install. Built with MSVC. The C++/WinRT
shim pattern is the same as Qt's. This target builds and walks through in CI but has had less
real-application time than the Apple/Linux/Android targets; calibrate expectations accordingly.

### OpenHarmony (`ohos-arkui`)
The newest and least proven backend: ArkUI via the NDK C API, packaged as a `.hap` by hvigor with
an ArkTS host project. The toolchain requires the OpenHarmony SDK and command-line tools, which
are the least ergonomic of the supported platforms to install — `day doctor --toolkit harmonyos`
and the [HarmonyOS notes](/docs/internal/harmonyos) exist for exactly this. Emulator behavior in
CI is tolerated-flaky.

## Cross-cutting gaps

Framework-level features that don't vary by platform but aren't done, kept here so there's one
list:

- **Animation.** No animation scheduler or transition API yet; native implicit animations (e.g.
  navigation transitions) still happen, but you can't author your own beyond redrawing a canvas.
- **Multi-window.** One window per process today.
- **Semantic color tokens / automatic dark-mode for custom colors.**
  ([styling](/docs/styling#color-backgrounds-shape))
- **Keyboard shortcuts** beyond native menu accelerators; no general key-event API.
- **Gestures**: tap and drag are wired; pinch, rotation, and long-press are not.
- **Forms**: no validation framework — roll your own with signals and memos.
- **Hot reload**: not present; see [the tradeoffs page](/docs/benefits#what-you-give-up).

If something you need is on this list, that's useful information *before* you adopt the
framework — and if it's not on the list and doesn't work, that's a bug worth reporting.
