# Environment variables — toolchain & SDK discovery

Day locates host toolchains and SDKs through one shared implementation
(`crates/day-toolchain`), used by the `day` CLI, by every crate build script that compiles its
own native shim (day-winui-sys, the `day-piece-*`/`day-tweak-*` crates, and the scaffolds
`day new` generates). Two rules apply everywhere:

1. **An environment variable always wins** over probing.
2. **No literal install paths.** Defaults derive from the platform's own environment
   (`%ProgramFiles%`, `$HOME`, `%LOCALAPPDATA%`) — a relocated install needs one variable, not a
   patched source tree.

Build scripts emit `cargo:rerun-if-env-changed=` for their overrides, so changing one re-runs
the affected script instead of keeping stale results.

## Windows

| Variable | Meaning | Fallback when unset |
|---|---|---|
| `DAY_CPPWINRT` | Exact C++/WinRT header dir (`…\Include\<ver>\cppwinrt`). An override that fails validation (`winrt/base.h` missing) is an error, not silently ignored. | scan below |
| `DAY_WINDOWS_KITS_ROOT` | The `…\Windows Kits\10` root (headers **and** bin tools resolve under it) | `WindowsSdkDir`, then `%ProgramFiles(x86)%`/`%ProgramFiles%` + `Windows Kits\10` |
| `WindowsSdkDir` | MS-standard (set by Visual Studio developer shells) — honored after the DAY_ vars | — |
| `DAY_WINDOWS_KIT` | A bin directory containing `signtool.exe`/`makeappx.exe` directly (`day pack` tool lookup) | PATH, then `bin\<ver>\<arch>` under the kits roots |
| `DAY_MAKENSIS` | The `makensis` executable for NSIS installers | PATH, then `%ProgramFiles%\NSIS` |

## Android / JDK

| Variable | Meaning | Fallback when unset |
|---|---|---|
| `ANDROID_HOME` / `ANDROID_SDK_ROOT` | Android SDK root (standard) | `~/Library/Android/sdk` (macOS), `%LOCALAPPDATA%\Android\Sdk` (Windows), `~/Android/Sdk` (Linux) |
| `ANDROID_NDK_HOME` | NDK root | newest NDK under `<sdk>/ndk` |
| `JAVA_HOME` | JDK for Gradle (AGP needs 21 exactly) | macOS: `/usr/libexec/java_home -v 21`, then Homebrew `openjdk@21` (either prefix) |
| `DAY_ANDROID_ABI` | Force the cargo-ndk ABI list when no device is connected (CI: `x86_64`) | connected devices' ABIs, else `arm64-v8a` |

## OpenHarmony

| Variable | Meaning |
|---|---|
| `OHOS_NDK_HOME` | The SDK's `native` dir (cross-linker + shim compiles); set by CI's setup-ohos-sdk |
| `OHOS_BASE_SDK_HOME` / `OHOS_SDK_HOME` | SDK root(s) — also probed for `hap-sign-tool.jar` |
| `DAY_OHOS_ARCH` | Force the build arch (`device` / `arm64` / `x86_64`) when no device is connected |

## Rust toolchain

| Variable | Meaning | Fallback when unset |
|---|---|---|
| `RUSTUP_HOME` | rustup root for cross-std toolchains (mobile targets need rustup's per-target std; a Homebrew/system rustc has none) | `~/.rustup`; among installed toolchains a `stable-*` one is preferred |

## Linux packaging

| Variable | Meaning |
|---|---|
| `DAY_GNOME_RUNTIME` / `DAY_KDE_RUNTIME` | Pin the flatpak runtime branch `day pack` targets (GTK ⇒ org.gnome.Platform, Qt ⇒ org.kde.Platform) |

## Scaffolding & signing

| Variable | Meaning |
|---|---|
| `DAY_LOCAL` | Make `day new` scaffolds depend on a local day checkout instead of the git remote (CI) |
| `DAY_THEME` | `light` \| `dark` — forces the app's theme on every backend (AppKit appearance, libadwaita color scheme, Qt 6.8+ color scheme, UIKit interface style, Android night mode, WinUI element theme, OHOS color mode); unset = follow the system. CI's themed screenshot cycles pass it via `day launch --env` |
| `ANDROID_SERIAL` | adb's standard device selector — when set, `day build/launch` and dayscript sessions target ONLY that device instead of every connected one |
| `DAY_SIGN_*`, `DAY_NOTARY_*`, `DAY_ASC_*`, `DAY_KS_PASS`, … | Release-signing secrets referenced from `Day.toml`'s `[signing]` tables via `${VAR}` — resolved at pack time, degrade to the dev signing tier when unset (§20) |

Signing variables are listed exhaustively by `day sign --check`, which reports each platform's
readiness without printing a secret value.

## Update check

| Variable | Meaning |
|---|---|
| `DAY_NO_UPDATE_CHECK` | Set to any non-empty value to disable the background "a newer day-cli is on crates.io?" check. This is day's **only** outbound network call, so setting it keeps day fully offline. |
