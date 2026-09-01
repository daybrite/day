---
title: "Environment variables"
description: "Every environment variable day reads: toolchain and SDK discovery, capture and theming overrides, and CI switches."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Environment variables: toolchain & SDK discovery

Day locates host toolchains and SDKs through one shared implementation
(`crates/day-toolchain`), used by the `day` CLI, by every crate build script that compiles its
own native shim (day-xaml-sys, the `day-piece-*`/`day-tweak-*` crates, and the scaffolds
`day new` generates). Two rules apply everywhere:

1. **An environment variable always wins** over probing.
2. **Defaults derive from the platform's own environment** (`%ProgramFiles%`, `$HOME`,
   `%LOCALAPPDATA%`), never from a literal install path, so a relocated install needs only one
   variable set.

Build scripts emit `cargo:rerun-if-env-changed=` for their overrides, so changing one re-runs
the affected script instead of keeping stale results.

## Windows

| Variable | Meaning | Fallback when unset |
|---|---|---|
| `DAY_CPPWINRT` | Exact C++/WinRT header dir (`…\Include\<ver>\cppwinrt`). An override that fails validation (`winrt/base.h` missing) is an error, not silently ignored. | scan below |
| `DAY_WINDOWS_KITS_ROOT` | The `…\Windows Kits\10` root (headers **and** bin tools resolve under it) | `WindowsSdkDir`, then `%ProgramFiles(x86)%`/`%ProgramFiles%` + `Windows Kits\10` |
| `WindowsSdkDir` | MS-standard (set by Visual Studio developer shells); honored after the DAY_ vars | — |
| `DAY_WINDOWS_KIT` | A bin directory containing `signtool.exe`/`makeappx.exe` directly (`day pack` tool lookup) | PATH, then `bin\<ver>\<arch>` under the kits roots |
| `DAY_MAKENSIS` | The `makensis` executable for NSIS installers | PATH, then `%ProgramFiles(x86)%`/`%ProgramFiles%` + `NSIS`, then chocolatey (`%ChocolateyInstall%\bin` shim, else `…\lib\nsis\tools\<ver>`) |

## Android / JDK

| Variable | Meaning | Fallback when unset |
|---|---|---|
| `ANDROID_HOME` / `ANDROID_SDK_ROOT` | Android SDK root (standard) | `~/Library/Android/sdk` (macOS), `%LOCALAPPDATA%\Android\Sdk` (Windows), `~/Android/Sdk` (Linux) |
| `ANDROID_NDK_HOME` | NDK root | newest NDK under `<sdk>/ndk` |
| `JAVA_HOME` | JDK for Gradle (AGP 9 needs 17+; Gradle 9.7 runs on 17…26) | macOS: `/usr/libexec/java_home -v 17+`, then a Homebrew `openjdk` keg (either prefix) |
| `DAY_ANDROID_ABI` | Force the cargo-ndk ABI list for the build (comma/space-separated); **takes precedence over any connected device** (CI walkthrough: `x86_64`; dual-ABI pack: `arm64-v8a,x86_64`; each ABI needs its rustup target) | connected devices' ABIs, else `arm64-v8a` |

## OpenHarmony

| Variable | Meaning |
|---|---|
| `OHOS_NDK_HOME` | The SDK's `native` dir (cross-linker + shim compiles); set by CI's setup-ohos-sdk |
| `OHOS_BASE_SDK_HOME` / `OHOS_SDK_HOME` | SDK root(s); also probed for `hap-sign-tool.jar` |
| `DAY_OHOS_ARCH` | Force the build arch (`device` / `arm64` / `x86_64`). Takes precedence over any connected device, so a `day pack` produces the same hap whether or not an emulator is running; leave it unset to build for each attached target |

## Rust toolchain

| Variable | Meaning | Fallback when unset |
|---|---|---|
| `RUSTUP_HOME` | rustup root for cross-std toolchains (mobile targets need rustup's per-target std; a Homebrew/system rustc has none) | `~/.rustup`; among installed toolchains a `stable-*` one is preferred |

## Linux packaging

| Variable | Meaning |
|---|---|
| `DAY_GNOME_RUNTIME` / `DAY_KDE_RUNTIME` | Pin the flatpak runtime branch `day pack` targets (GTK ⇒ org.gnome.Platform, Qt ⇒ org.kde.Platform) |
| `DAY_LINUXDEPLOY` | The `linuxdeploy` executable that builds the `.appimage`. Checked before PATH, because linuxdeploy ships as a downloaded AppImage rather than a package |
| `DAY_LINUXDEPLOY_PLUGIN_GTK` / `DAY_LINUXDEPLOY_PLUGIN_QT` | Same, for the toolkit plugin. Absent, `day pack` still builds an AppImage and warns that it carries no GTK/Qt modules |

## Scaffolding & signing

| Variable | Meaning |
|---|---|
| `DAY_LOCAL` | Make `day new` scaffolds depend on a local day checkout instead of the git remote (CI) |
| `DAY_THEME` | `light` \| `dark` — forces the app's theme on every backend (AppKit appearance, libadwaita color scheme, Qt 6.8+ color scheme, UIKit interface style, Android night mode, XAML element theme, OHOS color mode); unset = follow the system. CI's themed screenshot cycles pass it via `day launch --env`. An app that applies its own persisted appearance at startup must honor the env-wins rule ([docs/prefs.md](prefs.md)): skip the boot application when `DAY_THEME` is set, or the app clears the forced theme right back; `day-piece-settings::apply_startup` does this for you, and Day Showcase's hand-rolled menu plumbing shows the guard (`src/commands.rs`) |
| `DAY_WINDOW` | `<width>x<height>` (e.g. `700x850`) — overrides the app's initial window size for responsive-layout testing on desktop backends; mobile/web size to the screen and ignore it |
| `DAY_APP_VERSION`, `DAY_SCRIPT` | The app's version and the driving script's file name, set by `day launch` on every run. A debug build appends them to every window title as `(version/toolkit[/script])` — [docs/windows.md](windows.md). Release builds ignore both |
| `ANDROID_SERIAL` | adb's standard device selector; when set, `day build/launch` and dayscript sessions target only that device instead of every connected one |
| `DAY_LOG_ACTIONS` | `1` narrates every user action to stdout in the dayscript vocabulary (`dayscript ▸ tap inc  "Add"`) without recording anything, the same lines a recording echoes (§14.6). An app can also call `day::record::log_actions(true)`; Day-Showcase does, and reads `DAY_LOG_ACTIONS=0` as the way to silence it |
| `DAY_VERBOSE` | `1` \| `true` — the global `--verbose` flag, from the environment: every `day` command forwards its sub-commands' raw output (cargo, gradle, xcodebuild, hvigor, adb, codesign, …) instead of capturing it. One `env:` line turns a whole CI job verbose, including invocations no flag can reach (the launches a generated dayscript runner performs). An explicit `--verbose` on the command line also turns it on; any other value (or unset) leaves the default quiet output |
| `DAY_SCRIPT_MAIN_TIMEOUT_SECS` | How long one dayscript step waits for the app's main thread before failing (default 30). This is not the step's implicit-wait budget; it covers a main thread that has not answered at all, which is a property of the machine (a shared CI vCPU compositing its first frame) rather than of the script. Raise it on a slow runner |
| `DAY_SELF_COMMAND` | How `day mcp-server` re-invokes the CLI for each tool call, as a JSON array of argv[0] plus any leading arguments (`["cargo","run","--manifest-path","…/Cargo.toml","-q","-p","day-cli","--"]`). Unset, a tool call runs the server's own executable. The VS Code extension sets it when `day.cliSource` points at a day checkout, so an edit to day-cli reaches the agent's tools the same way it already reaches the editor's Build and Run — see [docs/agent.md](agent.md). An unusable value falls back to the default rather than failing every tool |
| `DAY_SIGN_*`, `DAY_NOTARY_*`, `DAY_ASC_*`, `DAY_KS_PASS`, … | Release-signing secrets referenced from `Day.toml`'s `[signing]` tables via `${VAR}`; resolved at pack time, degrade to the dev signing tier when unset (§20) |

Signing variables are listed exhaustively by `day sign --check`, which reports each platform's
readiness without printing a secret value.

## Network

Day makes exactly two kinds of outbound call, both disableable:

| Variable | Meaning |
|---|---|
| `DAY_NO_UPDATE_CHECK` | Set to any non-empty value to disable the background "a newer day-cli is on crates.io?" check, the only outbound call day makes, so setting it keeps day fully offline. |
